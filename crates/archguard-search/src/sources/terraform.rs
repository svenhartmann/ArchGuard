//! Elasticsearch artifacts declared as `elasticstack_elasticsearch_*` Terraform resources
//! (ADR 0023). Names are read from literals, `jsonencode({...})` and, for a plain `var.x`, from a
//! non-sensitive `string`/`list(string)` literal module argument or `default`; anything else the
//! provider would evaluate (locals, `each`, module outputs, functions) is a gap.
use crate::context::SearchContext;
use crate::context::SourceOutput;
use crate::findings::{EsGap, es_gap};
use archguard_core::{SearchArtifact, SearchIndexDetail, SearchReference, ServiceTopology};
use archguard_terraform::hcl::{attr, block, get, literal, object};
use archguard_terraform::module::{ModuleCtx, Resource, TfFile};
use hcl_edit::Span;
use hcl_edit::expr::{Expression, TraversalOperator};
use hcl_edit::parser;
use hcl_edit::structure::Body;

const PREFIX: &str = "elasticstack_elasticsearch_";
const KIBANA: &str = "elasticstack_kibana_";
/// Rule types that keep their index in `params.index`; others have no index to read.
const INDEX_RULES: [&str; 2] = [".es-query", ".index-threshold"];
const NOTE: &str =
    "Declared Elasticsearch artifact read statically; no terraform run, state or cluster access.";
/// Resource type suffix, artifact kind and the attribute holding its name.
const KINDS: [(&str, &str, &str); 10] = [
    ("index", "index", "name"),
    ("index_template", "index_template", "name"),
    ("component_template", "component_template", "name"),
    ("index_lifecycle", "ilm_policy", "name"),
    ("ingest_pipeline", "ingest_pipeline", "name"),
    ("transform", "transform", "name"),
    ("watch", "watch", "watch_id"),
    ("enrich_policy", "enrich_policy", "name"),
    ("snapshot_lifecycle", "snapshot_lifecycle", "name"),
    ("script", "stored_script", "script_id"),
];

/// Walks every Terraform module instance (a second parse next to the engine's deployment walk;
/// duplicate parse diagnostics are removed by the final sort and dedup of the report).
pub(crate) fn extract_with(ctx: &SearchContext) -> SourceOutput {
    let mut scratch = ServiceTopology::default();
    archguard_terraform::module::walk(
        ctx.root,
        ctx.discovery,
        &mut scratch,
        ctx.read,
        &mut augment,
    );
    SourceOutput::from_scratch(scratch)
}

/// Appends the artifacts of one module instance to `report.elasticsearch`.
fn augment(report: &mut ServiceTopology, scope: &str, resources: &[Resource]) {
    for r in resources {
        if let Some(suffix) = r.kind.strip_prefix(KIBANA) {
            kibana(report, scope, r, suffix);
            continue;
        }
        let Some(suffix) = r.kind.strip_prefix(PREFIX) else {
            continue;
        };
        let Some(&(_, kind, name_key)) = KINDS.iter().find(|(s, _, _)| *s == suffix) else {
            continue;
        };
        let mut read = Read {
            report,
            r,
            scope,
            out: Vec::new(),
            sources: Vec::new(),
            detail: None,
        };
        read.resource(kind, name_key);
        let out = read.finish();
        report.elasticsearch.artifacts.extend(out);
    }
}

/// Kibana data views and alerting rules; saved-object imports are noted, not read.
fn kibana(report: &mut ServiceTopology, scope: &str, r: &Resource, suffix: &str) {
    if suffix == "import_saved_objects" {
        // Not a gap: the import is out of scope, so `complete` stays untouched.
        es_gap(
            report,
            EsGap::SavedObjects,
            &r.file.path,
            None,
            &[&r.address],
        );
        return;
    }
    if !matches!(suffix, "data_view" | "alerting_rule") {
        return;
    }
    let mut read = Read {
        report,
        r,
        scope,
        out: Vec::new(),
        sources: Vec::new(),
        detail: None,
    };
    if suffix == "data_view" {
        read.data_view();
    } else {
        read.alerting_rule();
    }
    let out = read.finish();
    report.elasticsearch.artifacts.extend(out);
}

struct Read<'x, 'a> {
    report: &'x mut ServiceTopology,
    r: &'x Resource<'a>,
    scope: &'x str,
    out: Vec<SearchArtifact>,
    /// Where names came from when read through `var.x`; attached to the evidence.
    sources: Vec<String>,
    /// Detail of the index being read, taken over by the next `push`.
    detail: Option<SearchIndexDetail>,
}

/// Values read through a variable, and where they came from.
struct Found {
    values: Vec<String>,
    list: bool,
    source: String,
}

fn reference(kind: &str, name: Option<String>) -> Option<SearchReference> {
    Some(SearchReference {
        kind: kind.into(),
        name: name?,
    })
}

impl Read<'_, '_> {
    fn finish(self) -> Vec<SearchArtifact> {
        let mut out = self.out;
        if !self.sources.is_empty() {
            let note = format!(" Name read from {}.", self.sources.join("; "));
            out.iter_mut()
                .for_each(|a| a.evidence[0].description.push_str(&note));
        }
        out
    }

    /// The value of a plain `var.x`, remembering its source.
    fn variable(&mut self, e: &Expression) -> Option<Found> {
        let found = resolve(&self.r.module, variable_name(e)?)?;
        if !self.sources.contains(&found.source) {
            self.sources.push(found.source.clone());
        }
        Some(found)
    }

    /// A literal name, or a gap diagnostic for `what` and `None`.
    fn text(&mut self, what: &str, e: Option<&Expression>) -> Option<String> {
        let text = e.and_then(literal).or_else(|| {
            let found = self.variable(e?)?;
            (!found.list).then(|| found.values.into_iter().next())?
        });
        if text.is_none() {
            let path = &self.r.file.path;
            es_gap(
                self.report,
                EsGap::NotLiteral,
                path,
                None,
                &[&self.r.address, what],
            );
        }
        text
    }

    /// Literal strings of a string or list attribute; non-literal items are gaps.
    fn texts(&mut self, what: &str, e: Option<&Expression>) -> Vec<String> {
        match e {
            None => vec![],
            Some(Expression::Array(items)) => items
                .iter()
                .filter_map(|i| self.text(what, Some(i)))
                .collect(),
            Some(e) => match self.variable(e) {
                Some(found) => found.values,
                None => self.text(what, Some(e)).into_iter().collect(),
            },
        }
    }

    fn push(
        &mut self,
        kind: &str,
        name: Option<String>,
        suffix: &str,
        patterns: Vec<String>,
        references: Vec<SearchReference>,
    ) {
        let r = self.r;
        self.out.push(SearchArtifact {
            kind: kind.into(),
            name,
            address: format!("{}{suffix}", r.address),
            scope: self.scope.into(),
            patterns,
            references,
            evidence: vec![r.evidence(r.line(), "", NOTE)],
            origins: vec![],
            detail: self.detail.take(),
        });
    }

    /// Aliases declared in `alias` blocks of `body`, each pointing at `target`.
    fn aliases(&mut self, body: &Body, target: Option<SearchReference>) {
        for b in body.get_blocks("alias") {
            let name = self.text("an alias name", attr(&b.body, "name"));
            let suffix = name
                .as_ref()
                .map_or(".alias".into(), |n| format!(".alias.{n}"));
            self.push(
                "alias",
                name,
                &suffix,
                vec![],
                target.clone().into_iter().collect(),
            );
        }
    }

    fn data_view(&mut self) {
        let body = &self.r.block.body;
        let name = self.text("the data view name", field(body, "data_view", "name"));
        let title = field(body, "data_view", "title");
        let patterns = self
            .texts("an index pattern", title)
            .iter()
            .flat_map(|t| t.split(','))
            .map(str::trim)
            .filter(|p| !p.is_empty() && !p.starts_with('-'))
            .map(str::to_owned)
            .collect();
        self.push("data_view", name, "", patterns, vec![]);
    }

    fn alerting_rule(&mut self) {
        let body = &self.r.block.body;
        let name = self.text("the alerting rule name", attr(body, "name"));
        let rule_type = self.text("the rule type", attr(body, "rule_type_id"));
        let (mut found, mut gap) = (Vec::new(), false);
        if let Some(p) = attr(body, "params") {
            params_index(p, &mut found, &mut gap);
        }
        if gap {
            self.text("the rule index", None);
        } else if found.is_empty()
            && rule_type
                .as_deref()
                .is_some_and(|t| INDEX_RULES.contains(&t))
        {
            let path = &self.r.file.path;
            es_gap(
                self.report,
                EsGap::NoParamsIndex,
                path,
                None,
                &[&self.r.address],
            );
        }
        let mut patterns = Vec::new();
        for e in found {
            patterns.extend(self.texts("an index pattern", Some(&e)));
        }
        self.push("alerting_rule", name, "", patterns, vec![]);
        if let (Some(t), Some(a)) = (rule_type, self.out.last_mut()) {
            a.evidence[0]
                .description
                .push_str(&format!(" rule_type_id: {t}."));
        }
    }

    fn resource(&mut self, kind: &str, name_key: &str) {
        let body = &self.r.block.body;
        let name = self.text(&format!("the {kind} name"), attr(body, name_key));
        let mut patterns = Vec::new();
        let mut script_kind = kind;
        let refs = match kind {
            "index" => self.index(body, &name),
            "index_template" | "component_template" => {
                self.template(body, kind, &name, &mut patterns)
            }
            "ingest_pipeline" => self.pipeline(body),
            "transform" => self.transform(body),
            "watch" => self.watch(body, &mut patterns),
            "enrich_policy" | "snapshot_lifecycle" => self.index_refs(body),
            _ => {
                if kind == "stored_script"
                    && attr(body, "lang").and_then(literal).as_deref() == Some("mustache")
                {
                    script_kind = "search_template";
                }
                vec![]
            }
        };
        self.push(script_kind, name, "", patterns, refs);
    }

    fn index(&mut self, body: &Body, name: &Option<String>) -> Vec<SearchReference> {
        let mut refs = Vec::new();
        for key in ["default_pipeline", "final_pipeline"] {
            let n = attr(body, key).map(|e| self.text(key, Some(e)));
            refs.extend(reference("ingest_pipeline", n.flatten()));
        }
        // An alias of an index whose own name is unknown is still declared, with no target.
        let target = name.as_ref().map(|n| SearchReference {
            kind: "index".into(),
            name: n.clone(),
        });
        self.aliases(body, target);
        let (mut detail, mut missed) = super::terraform_detail::read(body, &self.r.module);
        let (legacy, bad) = super::terraform_links::legacy_settings(body);
        if !legacy.is_empty() {
            detail.get_or_insert_default().settings.extend(legacy);
        }
        if bad {
            missed.push("settings");
        }
        refs.extend(self.detail(detail, &missed));
        refs
    }

    /// Keeps `detail` for the next `push`, names what was not read in one gap and returns the ILM
    /// reference.
    fn detail(
        &mut self,
        detail: Option<SearchIndexDetail>,
        missed: &[&str],
    ) -> Option<SearchReference> {
        if !missed.is_empty() {
            let path = &self.r.file.path;
            let parts = [self.r.address.as_str(), &missed.join(", ")];
            es_gap(self.report, EsGap::DetailNotLiteral, path, None, &parts);
        }
        let ilm = super::terraform_links::ilm(detail.as_ref());
        self.detail = detail;
        ilm
    }

    fn template(
        &mut self,
        body: &Body,
        kind: &str,
        name: &Option<String>,
        patterns: &mut Vec<String>,
    ) -> Vec<SearchReference> {
        let mut refs = Vec::new();
        if kind == "index_template" {
            *patterns = self.texts("an index pattern", attr(body, "index_patterns"));
            for n in self.texts("a composed_of name", attr(body, "composed_of")) {
                refs.extend(reference("component_template", Some(n)));
            }
        }
        if let Some(template) = block(body, "template") {
            let read = super::terraform_detail::read_template(template);
            refs.extend(self.detail(read.0, &read.1));
        }
        if let (Some(template), Some(n)) = (block(body, "template"), name) {
            self.aliases(
                template,
                Some(SearchReference {
                    kind: kind.into(),
                    name: n.clone(),
                }),
            );
        }
        refs
    }

    fn pipeline(&mut self, body: &Body) -> Vec<SearchReference> {
        let (mut found, mut gap) = (Vec::new(), false);
        for key in ["processors", "on_failure"] {
            if let Some(p) = attr(body, key) {
                pipelines(p, true, &mut found, &mut gap);
            }
        }
        if gap {
            self.text("a pipeline processor", None);
        }
        let mut refs = Vec::new();
        for n in found {
            let n = self.text("a pipeline processor name", Some(&n));
            refs.extend(reference("ingest_pipeline", n));
        }
        refs
    }

    fn transform(&mut self, body: &Body) -> Vec<SearchReference> {
        let mut refs = Vec::new();
        for n in self.texts("a source index", field(body, "source", "indices")) {
            refs.extend(reference("index", Some(n)));
        }
        let dest = field(body, "destination", "index");
        let n = dest.map(|e| self.text("the destination index", Some(e)));
        refs.extend(reference("index", n.flatten()));
        let pipe = field(body, "destination", "pipeline");
        let n = pipe.map(|e| self.text("the destination pipeline", Some(e)));
        refs.extend(reference("ingest_pipeline", n.flatten()));
        refs
    }

    fn watch(&mut self, body: &Body, patterns: &mut Vec<String>) -> Vec<SearchReference> {
        let (names, found, bad) = super::terraform_links::watch_indices(body);
        if bad {
            self.detail(None, &["input"]);
        }
        *patterns = found;
        names
            .into_iter()
            .flat_map(|n| reference("index", Some(n)))
            .collect()
    }

    fn index_refs(&mut self, body: &Body) -> Vec<SearchReference> {
        let names = self.texts("an index", attr(body, "indices"));
        names
            .into_iter()
            .flat_map(|n| reference("index", Some(n)))
            .collect()
    }
}

/// An attribute of a nested `section { key = ... }` block or `section = { key = ... }` object.
fn field<'b>(body: &'b Body, section: &'b str, key: &str) -> Option<&'b Expression> {
    match block(body, section) {
        Some(b) => attr(b, key),
        None => attr(body, section).and_then(|e| get(e, key)),
    }
}

/// Names of `{ pipeline = { name = ... } }` processors anywhere in `jsonencode(...)`, literal
/// JSON or plain objects; the name expressions are returned for the caller to read.
/// `strict` marks positions that must hold processors: an unreadable expression there sets `gap`.
fn pipelines(e: &Expression, strict: bool, out: &mut Vec<Expression>, gap: &mut bool) {
    match e {
        Expression::FuncCall(call) if call.name.name.as_str() == "jsonencode" => {
            call.args.iter().for_each(|a| pipelines(a, true, out, gap));
        }
        Expression::Array(items) => items.iter().for_each(|i| pipelines(i, strict, out, gap)),
        Expression::Object(_) => {
            for (key, value) in object(e) {
                match (key.as_str(), get(value, "name")) {
                    ("pipeline", Some(n)) => out.push(n.clone()),
                    _ => pipelines(value, false, out, gap),
                }
            }
        }
        other => match literal(other) {
            Some(s) if s.trim_start().starts_with(['{', '[']) => {
                if let Ok(parsed) = parser::parse_expr(s.trim()) {
                    pipelines(&parsed, strict, out, gap);
                }
            }
            Some(_) => {}
            None => *gap |= strict,
        },
    }
}

/// The top-level `index` value of `params` given as `jsonencode(...)`, literal JSON or a plain
/// object. Nested `index` keys (e.g. `searchConfiguration.index`) are data view ids, not patterns.
/// Unreadable params (variable, function, malformed JSON) set `gap`.
fn params_index(e: &Expression, out: &mut Vec<Expression>, gap: &mut bool) {
    match e {
        Expression::FuncCall(call) if call.name.name.as_str() == "jsonencode" => {
            call.args.iter().for_each(|a| params_index(a, out, gap));
        }
        Expression::Object(_) => out.extend(get(e, "index").cloned()),
        other => match literal(other).filter(|s| s.trim_start().starts_with('{')) {
            Some(s) => match parser::parse_expr(s.trim()) {
                Ok(parsed) => params_index(&parsed, out, gap),
                Err(_) => *gap = true,
            },
            None => *gap = true,
        },
    }
}

/// `x` of a plain `var.x`.
fn variable_name(e: &Expression) -> Option<&str> {
    let Expression::Traversal(t) = e else {
        return None;
    };
    match (&t.expr, t.operators.as_slice()) {
        (Expression::Variable(v), [op]) if v.as_str() == "var" => match op.value() {
            TraversalOperator::GetAttr(n) => Some(n.as_str()),
            _ => None,
        },
        _ => None,
    }
}

/// A string literal, or a list of string literals, and whether it was a list. Numbers and
/// bools are no names.
fn values(e: &Expression) -> Option<(Vec<String>, bool)> {
    let string = |e: &Expression| match e {
        Expression::String(_) => literal(e),
        _ => None,
    };
    match e {
        Expression::Array(items) => Some((items.iter().map(string).collect::<Option<_>>()?, true)),
        other => Some((vec![string(other)?], false)),
    }
}

/// `Some(Some(list))` for a declared `string`/`list(string)`, `Some(None)` when untyped,
/// `None` for anything else or a sensitive variable: those are never read.
fn readable(var: &Body) -> Option<Option<bool>> {
    if attr(var, "sensitive").is_some_and(|s| literal(s).as_deref() != Some("false")) {
        return None;
    }
    match attr(var, "type") {
        None => Some(None),
        Some(Expression::Variable(v)) if v.as_str() == "string" => Some(Some(false)),
        Some(Expression::FuncCall(c))
            if c.name.name.as_str() == "list"
                && matches!(c.args.iter().collect::<Vec<_>>().as_slice(),
                    [Expression::Variable(v)] if v.as_str() == "string") =>
        {
            Some(Some(true))
        }
        Some(_) => None,
    }
}

fn at(f: &TfFile, e: &Expression) -> String {
    let line = f.line(e.span()).map_or(String::new(), |l| format!(":{l}"));
    format!("{}{line}", f.path)
}

/// The literal module argument `name` of the call, else the `default` of the variable in the
/// module. A `var.y` argument continues in the caller's module, one hop per module level.
fn resolve(ctx: &ModuleCtx, name: &str) -> Option<Found> {
    let (var, var_file) = ctx.variable(name)?;
    let list = readable(&var.body)?;
    let found = match ctx
        .call()
        .and_then(|(c, f)| Some((c, f, attr(&c.body, name)?)))
    {
        Some((call, file, arg)) => {
            let module = call.labels.first()?.as_str();
            match values(arg) {
                Some((values, list)) => Found {
                    values,
                    list,
                    source: format!("argument {name} of module.{module} ({})", at(file, arg)),
                },
                None => resolve(ctx.parent()?, variable_name(arg)?)?,
            }
        }
        None => {
            let d = attr(&var.body, "default")?;
            let (values, list) = values(d)?;
            let source = format!("default of variable {name} ({})", at(var_file, d));
            Found {
                values,
                list,
                source,
            }
        }
    };
    list.is_none_or(|l| l == found.list).then_some(found)
}
