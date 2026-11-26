//! Python documents of elasticsearch-dsl, opensearch-dsl and django-elasticsearch-dsl (ADR 0023,
//! ticket 24). Rules of this format:
//! - A class is an index when its bases lead to `Document` or `AsyncDocument` below
//!   `elasticsearch_dsl`, `elasticsearch.dsl`, `django_elasticsearch_dsl` (flavor empty) or
//!   `opensearchpy`, `opensearch_dsl` (flavor `opensearch`): directly through an import, or
//!   through base classes of the same file or of files below the root (`pydsl_project`). An
//!   own `Document` class matches nothing. A base that leads nowhere in a file that imports the
//!   DSL is a gap when the class has fields or a `class Index`.
//! - The Index and the fields come from the class and its bases (the nearest `class Index`
//!   wins). Its `class Index` gives the name (`name = "x"`, a constant of the file or of the
//!   service), `settings` (a literal dict), `aliases` (keys become alias references) and
//!   `analyzers`. A document without a name (no `class Index`, none inherited, no `name`) is an
//!   artifact without name and a gap.
//! - Fields are the class attributes that call a field class (`Text(...)`, `fields.TextField(...)`),
//!   `mapped_field(Text())` or carry an annotation: ES type is the snake-case class name,
//!   `fields={'raw': Keyword()}` are multi-fields, `Object`/`Nested` take `properties={...}` or
//!   an inner document class. Annotations follow the DSL table (`str` text, `int` integer, `float`,
//!   `bool` boolean, `bytes` binary, `datetime`/`date` date); `M`, `Optional`, `| None` unwrap,
//!   `list[...]` makes an `InnerDoc` nested, `ClassVar` is skipped. What the table lacks
//!   (`M[Keyword]`, `M[Any]`) is a gap; the real DSL would refuse it too. Inference from
//!   `mapped_field()` options is not done.
//! - `analyzer`, `tokenizer`, `token_filter`, `char_filter`, `normalizer` calls resolve through
//!   module constants into the analysis, stored as Elasticsearch writes it (`type: custom`).
//! - All or nothing per attribute: `mappings` or `settings` that are not literal are not read and
//!   named in one gap. A django `class Django` with `fields` names model fields: a gap.
//!   Name = literal, `scope` = `python:<folder>`, address = `<file>:<class>`.
//!
//! Trust boundary: files below the root through the safe reader, at most 5 000 files, no test
//! paths; no network, nothing is executed. Values resolve through constants of their own file.
// shortcut: fields of a base class are read with the constants of the base's file; the Index name
// alone also resolves through the service's constants. Lift when a project needs more.
use super::constants::{Constants, Name, reference};
use super::json::search_definition;
use super::pydsl_eval::{DEPTH, Eval};
use super::pydsl_module::Module;
use super::pydsl_project::{Class, Kind, Project};
use crate::context::{SearchContext, SourceOutput};
use crate::findings::{EsGap, es_gap};
use archguard_core::{SearchArtifact, ServiceEvidence, ServiceTopology};
use archguard_flow_common::lang_python::DefKind;
use archguard_flow_common::lang_text::line;
use archguard_flow_common::support::{owners, relative, test_path};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

const MAX_FILES: usize = 5_000;
const SKIPPED_DIRS: &[&str] = &[".terraform", ".git", "node_modules", "venv", ".venv"];

pub(crate) fn extract_with(ctx: &SearchContext) -> SourceOutput {
    let mut out = ServiceTopology::default();
    let mut files = candidates(ctx);
    if files.len() > MAX_FILES {
        files.truncate(MAX_FILES);
        es_gap(&mut out, EsGap::DefinitionScanStop, ".", None, &[]);
    }
    let mut texts = vec![];
    for rel in files {
        match (ctx.read)(ctx.root, &ctx.root.join(&rel)) {
            Some(text) => texts.push((rel, text)),
            // code.rs reports unread files a service owns; the others are reported here.
            None if owners(ctx.topology, &rel).is_empty() => {
                es_gap(&mut out, EsGap::Unread, &rel, None, &[]);
            }
            None => {}
        }
    }
    let mut consts = Constants::default();
    let mut services = vec![];
    for (rel, text) in &texts {
        let service = owners(ctx.topology, rel)
            .into_iter()
            .next()
            .unwrap_or_default();
        consts.collect(&service, "py", rel, text);
        services.push(service);
    }
    let files = (texts.iter()).map(|(rel, text)| (rel.clone(), Module::parse(text)));
    let reader = Reader {
        project: Project::new(files.collect()),
        consts,
        services,
    };
    reader.read_all(&mut out);
    SourceOutput::from_scratch(out)
}

fn candidates(ctx: &SearchContext) -> Vec<String> {
    let mut files: Vec<String> = ctx
        .discovery
        .walk(ctx.root)
        .flatten()
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .map(|e| relative(ctx.root, e.path()))
        .filter(|rel| rel.ends_with(".py"))
        .filter(|rel| !test_path(rel) && !rel.split('/').any(|p| SKIPPED_DIRS.contains(&p)))
        .collect();
    files.sort();
    files
}

type Analysis = BTreeMap<&'static str, Map<String, Value>>;

struct Reader<'a> {
    project: Project<'a>,
    consts: Constants,
    /// Owning service of each file, `""` without one.
    services: Vec<String>,
}

impl Reader<'_> {
    fn read_all(&self, out: &mut ServiceTopology) {
        for (f, (rel, m)) in self.project.files.iter().enumerate() {
            for (c, class) in m.defs.iter().enumerate() {
                if class.kind != DefKind::Class {
                    continue;
                }
                match self.project.kind((f, c)) {
                    Kind::Document(flavor, chain) => {
                        if let Some(a) = self.artifact(&chain, flavor, out) {
                            out.elasticsearch.artifacts.push(a);
                        }
                    }
                    Kind::Unresolved(base) if self.reads_dsl(f, c) => {
                        let address = format!("{rel}:{}", class.name);
                        let what =
                            format!("base class {base} is not a DSL Document below the root");
                        es_gap(out, EsGap::DetailNotLiteral, rel, None, &[&address, &what]);
                    }
                    _ => {}
                }
            }
        }
    }

    /// Whether class `c` of file `f` looks like a document: the file imports the DSL and the class
    /// has fields or a `class Index`.
    fn reads_dsl(&self, f: usize, c: usize) -> bool {
        let m = &self.project.files[f].1;
        m.imports_dsl() && (m.nested(c, "Index").is_some() || !m.members(&m.defs[c]).is_empty())
    }

    /// The artifact of the document `chain[0]`; `chain` runs up to the DSL base.
    fn artifact(
        &self,
        chain: &[Class],
        flavor: &str,
        out: &mut ServiceTopology,
    ) -> Option<SearchArtifact> {
        let (f, i) = chain[0];
        let (rel, m) = &self.project.files[f];
        let class = &m.defs[i];
        let address = format!("{rel}:{}", class.name);
        let mut missed = vec![];
        let mut body = Map::new();
        let mut analysis = Analysis::new();
        match self.mappings(chain, &mut analysis) {
            Some(props) => {
                body.insert(
                    "mappings".into(),
                    serde_json::json!({ "properties": props }),
                );
            }
            None => {
                // The analysis the fields defined is part of the attribute that was not read.
                analysis.clear();
                missed.push("mappings");
            }
        }
        let IndexAttrs {
            name,
            mut settings,
            aliases,
        } = self.index(chain, out, &mut missed, &mut analysis);
        body.extend(aliases.map(|v| ("aliases".to_owned(), v)));
        if !missed.contains(&"settings") {
            add_analysis(&mut settings, analysis);
            body.insert("settings".into(), Value::Object(settings));
        }
        if !missed.is_empty() {
            let parts = [address.as_str(), &missed.join(", ")];
            es_gap(out, EsGap::DetailNotLiteral, rel, None, &parts);
        }
        if (m.nested(i, "Django").into_iter())
            .any(|d| m.body_assigns(d).iter().any(|(t, ..)| t == "fields"))
        {
            let what = "Django.fields (model fields, not index fields)";
            es_gap(out, EsGap::DetailNotLiteral, rel, None, &[&address, what]);
        }
        let def = search_definition(&Value::Object(body), Some("index"))?;
        Some(SearchArtifact {
            flavor: flavor.into(),
            kind: "index".into(),
            name,
            address,
            scope: format!("python:{}", rel.rsplit_once('/').map_or(".", |(d, _)| d)),
            patterns: vec![],
            references: def.references,
            evidence: vec![ServiceEvidence {
                file: rel.into(),
                pointer: format!("/{}", class.name),
                line: Some(class.line),
                description: format!(
                    "Document class {} declared in {rel}; read as data, nothing executed.",
                    class.name
                ),
            }],
            origins: vec![],
            detail: def.detail,
        })
    }

    /// The properties of the chain, base classes first so that a class overrides its bases;
    /// `None` when one class has a field that is not literal.
    fn mappings(&self, chain: &[Class], analysis: &mut Analysis) -> Option<Map<String, Value>> {
        let mut props = Map::new();
        for &(f, c) in chain.iter().rev() {
            let mut ev = Eval::new(&self.project.files[f].1);
            props.extend(ev.properties(c, DEPTH)?);
            merge(analysis, ev.analysis);
        }
        Some(props)
    }

    /// What the nearest `class Index` of the chain says. A name that stays unknown is a gap.
    fn index(
        &self,
        chain: &[Class],
        out: &mut ServiceTopology,
        missed: &mut Vec<&'static str>,
        analysis: &mut Analysis,
    ) -> IndexAttrs {
        let found = chain.iter().find_map(|&(f, c)| {
            let index = self.project.files[f].1.nested(c, "Index")?;
            Some((f, index))
        });
        let mut attrs = IndexAttrs::default();
        let Some((f, index)) = found else {
            let (f, c) = chain[0];
            let (rel, m) = &self.project.files[f];
            let at = m.defs[c].line.to_string();
            es_gap(out, EsGap::IndexNotLiteral, rel, None, &[&at]);
            return attrs;
        };
        let (rel, m) = &self.project.files[f];
        let mut ev = Eval::new(m);
        let mut named = false;
        for (target, a, e) in m.body_assigns(index) {
            let mut value = ev.value((a, e), DEPTH);
            if target == "name" {
                named = true;
                value = value.or_else(|| self.constant(f, &m.src[a..e]));
                if !matches!(value, Some(Value::String(_))) {
                    let at = line(m.src, a).to_string();
                    es_gap(out, EsGap::IndexNotLiteral, rel, None, &[&at]);
                }
            }
            attrs.take(&target, value, missed);
        }
        if !named {
            es_gap(
                out,
                EsGap::IndexNotLiteral,
                rel,
                None,
                &[&index.line.to_string()],
            );
        }
        merge(analysis, ev.analysis);
        attrs
    }

    /// The literal of a name the service declares as a Python string constant.
    fn constant(&self, f: usize, text: &str) -> Option<Value> {
        let Name::Ref(name) = reference(text) else {
            return None;
        };
        self.consts
            .python(&self.services[f], &name)
            .map(Value::String)
    }
}

fn merge(into: &mut Analysis, from: Analysis) {
    for (kind, defs) in from {
        into.entry(kind).or_default().extend(defs);
    }
}

/// What `class Index` says.
#[derive(Default)]
struct IndexAttrs {
    name: Option<String>,
    settings: Map<String, Value>,
    aliases: Option<Value>,
}

impl IndexAttrs {
    /// Keeps a literal attribute; one that is not literal is named in `missed`.
    fn take(&mut self, target: &str, value: Option<Value>, missed: &mut Vec<&'static str>) {
        match (target, value) {
            ("name", Some(Value::String(n))) => self.name = Some(n),
            ("settings", Some(Value::Object(s))) => self.settings = s,
            ("aliases", Some(v @ Value::Object(_))) => self.aliases = Some(v),
            ("analyzers", Some(Value::Array(_))) | ("name", _) => {}
            ("settings", _) => missed.push("settings"),
            ("aliases", _) => missed.push("aliases"),
            ("analyzers", _) => missed.push("analysis"),
            _ => {}
        }
    }
}

/// The analysis objects the calls defined go under `settings.analysis`, next to literal ones.
fn add_analysis(settings: &mut Map<String, Value>, analysis: Analysis) {
    if analysis.is_empty() {
        return;
    }
    let Value::Object(all) = settings
        .entry("analysis")
        .or_insert_with(|| Value::Object(Map::new()))
    else {
        return;
    };
    for (kind, defs) in analysis {
        if let Value::Object(into) = all.entry(kind).or_insert_with(|| Value::Object(Map::new())) {
            into.extend(defs);
        }
    }
}
