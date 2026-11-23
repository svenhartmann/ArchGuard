//! Elasticsearch artifacts declared as `elasticstack_elasticsearch_*` Terraform resources
//! (ADR 0023). Names are read only from literals and `jsonencode({...})`; anything the
//! provider would evaluate (variables, locals, `each`, module outputs, functions) is a gap.
use super::service_terraform::Resource;
use super::service_terraform_hcl::{attr, block, get, literal, object};
use archguard_core::{SearchArtifact, SearchReference, ServiceTopology};
use archguard_flow_common::support::diagnostic;
use hcl_edit::expr::Expression;
use hcl_edit::parser;
use hcl_edit::structure::Body;

const PREFIX: &str = "elasticstack_elasticsearch_";
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

/// Appends the artifacts of one module instance to `report.elasticsearch`.
pub(super) fn augment(report: &mut ServiceTopology, scope: &str, resources: &[Resource]) {
    for r in resources {
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
        };
        read.resource(kind, name_key);
        let out = read.out;
        report.elasticsearch.artifacts.extend(out);
    }
}

struct Read<'x, 'a> {
    report: &'x mut ServiceTopology,
    r: &'x Resource<'a>,
    scope: &'x str,
    out: Vec<SearchArtifact>,
}

fn reference(kind: &str, name: Option<String>) -> Option<SearchReference> {
    Some(SearchReference {
        kind: kind.into(),
        name: name?,
    })
}

impl Read<'_, '_> {
    /// A literal name, or a gap diagnostic for `what` and `None`.
    fn text(&mut self, what: &str, e: Option<&Expression>) -> Option<String> {
        let text = e.and_then(literal);
        if text.is_none() {
            let message = format!(
                "{}: Elasticsearch name is not a literal; {what} is unknown (variables, locals, each, module outputs and function calls are not evaluated).",
                self.r.address
            );
            diagnostic(self.report, &self.r.file.path, None, message);
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
            Some(e) => self.text(what, Some(e)).into_iter().collect(),
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
        });
    }

    /// Aliases declared in `alias` blocks of `body`, each pointing at `target`.
    fn aliases(&mut self, body: &Body, target: &SearchReference) {
        for b in body.get_blocks("alias") {
            let name = self.text("an alias name", attr(&b.body, "name"));
            let suffix = name
                .as_ref()
                .map_or(".alias".into(), |n| format!(".alias.{n}"));
            self.push("alias", name, &suffix, vec![], vec![target.clone()]);
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
        if let Some(n) = name {
            self.aliases(
                body,
                &SearchReference {
                    kind: "index".into(),
                    name: n.clone(),
                },
            );
        }
        refs
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
        if let (Some(template), Some(n)) = (block(body, "template"), name) {
            self.aliases(
                template,
                &SearchReference {
                    kind: kind.into(),
                    name: n.clone(),
                },
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
