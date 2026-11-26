//! Python documents of elasticsearch-dsl, opensearch-dsl and django-elasticsearch-dsl (ADR 0023,
//! ticket 24). Rules of this format:
//! - A class is an index when a base resolves through an import to `Document` below
//!   `elasticsearch_dsl`, `elasticsearch.dsl`, `django_elasticsearch_dsl` (flavor empty) or
//!   `opensearchpy`, `opensearch_dsl` (flavor `opensearch`). An own `Document` class matches nothing.
//! - Its inner `class Index` gives the name (`name = "x"` or a module constant), `settings` (a
//!   literal dict), `aliases` (keys become alias references) and `analyzers`. A document without
//!   `class Index` declares no index and is skipped; a name that is not a literal is a gap.
//! - Fields are the class attributes that call a field class (`Text(...)`, `fields.TextField(...)`):
//!   ES type is the snake-case class name, `fields={'raw': Keyword()}` are multi-fields,
//!   `Object`/`Nested` take `properties={...}` or an inner document class.
//! - `analyzer`, `tokenizer`, `token_filter`, `char_filter`, `normalizer` calls resolve through
//!   module constants into the analysis, stored as Elasticsearch writes it (`type: custom`).
//! - All or nothing per attribute: `mappings` or `settings` that are not literal are not read and
//!   named in one gap. Name = literal, `scope` = `python:<folder>`, address = `<file>:<class>`.
//!
//! Trust boundary: files below the root through the safe reader, at most 5 000 files, no test
//! paths; no network, nothing is executed. Constants resolve within the file only.
// shortcut: no inheritance from a local Document subclass and no constants from other modules;
// both end as a gap or a skipped class. Lift when a project needs them.
use super::json::search_definition;
use super::pydsl_eval::{DEPTH, Eval};
use super::pydsl_module::Module;
use crate::context::{SearchContext, SourceOutput};
use crate::findings::{EsGap, es_gap};
use archguard_core::{SearchArtifact, ServiceEvidence, ServiceTopology};
use archguard_flow_common::lang_text::line;
use archguard_flow_common::support::{owners, relative, test_path};
use serde_json::{Map, Value};

const MAX_FILES: usize = 5_000;
const SKIPPED_DIRS: &[&str] = &[".terraform", ".git", "node_modules", "venv", ".venv"];

pub(crate) fn extract_with(ctx: &SearchContext) -> SourceOutput {
    let mut out = ServiceTopology::default();
    let mut files = candidates(ctx);
    if files.len() > MAX_FILES {
        files.truncate(MAX_FILES);
        es_gap(&mut out, EsGap::DefinitionScanStop, ".", None, &[]);
    }
    for rel in files {
        match (ctx.read)(ctx.root, &ctx.root.join(&rel)) {
            Some(text) => read_file(&rel, &text, &mut out),
            // code.rs reports unread files a service owns; the others are reported here.
            None if owners(ctx.topology, &rel).is_empty() => {
                es_gap(&mut out, EsGap::Unread, &rel, None, &[]);
            }
            None => {}
        }
    }
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

fn read_file(rel: &str, text: &str, out: &mut ServiceTopology) {
    if !(text.contains("elasticsearch") || text.contains("opensearch")) {
        return;
    }
    let m = Module::parse(text);
    for (i, class) in m.defs.iter().enumerate() {
        let Some(flavor) = m.document_flavor(class) else {
            continue;
        };
        if let Some(a) = document(&m, i, flavor, rel, out) {
            out.elasticsearch.artifacts.push(a);
        }
    }
}

/// The artifact of document class `i`, or `None` without a `class Index`.
fn document(
    m: &Module,
    i: usize,
    flavor: &str,
    rel: &str,
    out: &mut ServiceTopology,
) -> Option<SearchArtifact> {
    let class = &m.defs[i];
    let index = m.nested(i, "Index")?;
    let address = format!("{rel}:{}", class.name);
    let mut ev = Eval::new(m);
    let mut missed = vec![];
    let mut body = Map::new();
    match ev.properties(i, DEPTH) {
        Some(props) => {
            body.insert(
                "mappings".into(),
                serde_json::json!({ "properties": props }),
            );
        }
        None => {
            // The analysis the fields defined is part of the attribute that was not read.
            ev.analysis.clear();
            missed.push("mappings");
        }
    }
    let mut attrs = IndexAttrs::default();
    for (target, a, e) in m.body_assigns(index) {
        let value = ev.value((a, e), DEPTH);
        if target == "name" && !matches!(value, Some(Value::String(_))) {
            let at = line(m.src, a).to_string();
            es_gap(out, EsGap::IndexNotLiteral, rel, None, &[&at]);
        }
        attrs.take(&target, value, &mut missed);
    }
    let IndexAttrs {
        name,
        mut settings,
        aliases,
    } = attrs;
    body.extend(aliases.map(|v| ("aliases".to_owned(), v)));
    if !missed.contains(&"settings") {
        add_analysis(&mut settings, ev.analysis);
        body.insert("settings".into(), Value::Object(settings));
    }
    if !missed.is_empty() {
        let parts = [address.as_str(), &missed.join(", ")];
        es_gap(out, EsGap::DetailNotLiteral, rel, None, &parts);
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
fn add_analysis(
    settings: &mut Map<String, Value>,
    analysis: std::collections::BTreeMap<&str, Map<String, Value>>,
) {
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
