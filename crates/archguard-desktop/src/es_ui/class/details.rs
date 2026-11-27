//! Detail model (index, building block, other), ticket 07. Reuses `index_page` for the overview,
//! links, usage and the "not read" note; the analysis parts come from the engine.

use archguard_engine::core::{SearchArtifact, SearchField, SearchIndexDetail, ServiceTopology};
use archguard_engine::{
    ChainRef, SearchAnalysisPart, SearchFieldRole, SearchGraph, SearchNode, search_analysis_parts,
    search_analysis_users, search_analyzer_difference, search_field_analyzers, search_field_depth,
    search_field_role,
};
use serde_json::Value;

use super::boxes::{mapping_unread, product};
use super::types::Key;
use crate::es_ui::index_page::{IndexPage, index_page};
use crate::services_ui::{search, view::evidence};
use crate::ui::SvcSearchDetail;

/// Kinds with an index page; the others show the search detail.
const PAGE_KINDS: [&str; 3] = ["index", "index_template", "component_template"];
/// The roles that make an analyzer or normalizer a chain table of an index.
const FIELD_ROLES: [&str; 3] = ["index", "search", "normalizer"];

#[derive(Debug, PartialEq)]
pub enum Details {
    Index(IndexDetails),
    Part(PartDetails),
    Other(SvcSearchDetail),
}

#[derive(Debug, PartialEq)]
pub struct IndexDetails {
    pub page: IndexPage,
    pub fields: Vec<FieldRow>,
    /// The mapping was not read (a gap): an empty field list is not "no fields".
    pub mapping_unread: bool,
    pub chains: Vec<ChainTable>,
    pub hints: Vec<Hint>,
    pub writes: Vec<String>,
    pub reads: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct FieldRow {
    pub path: String,
    pub depth: usize,
    pub role: SearchFieldRole,
    pub kind: String,
    pub analyzer: Option<String>,
    pub search_analyzer: Option<String>,
    pub normalizer: Option<String>,
}

/// The analysis chain a field uses in one role: the effective analyzer or normalizer `name`;
/// `stages` is empty when it is built in or not declared.
#[derive(Debug, PartialEq, Eq)]
pub struct FieldChain {
    pub name: String,
    pub declared: bool,
    pub stages: Vec<StageRow>,
}

/// One field of an index: its role, the index and search chain (`None` without an analyzer: not
/// a text field and none named), the normalizer chain and the neutral hint when index and search
/// analysis differ.
#[derive(Debug, PartialEq, Eq)]
pub struct FieldDetails {
    pub path: String,
    pub kind: String,
    pub role: SearchFieldRole,
    pub index: Option<FieldChain>,
    pub search: Option<FieldChain>,
    pub normalizer: Option<FieldChain>,
    /// Vector parameters in key order; `semantic_text` without `inference_id` shows the default
    /// endpoint (the default depends on the Elasticsearch version, so no id is invented).
    pub params: Vec<(String, String)>,
    pub hint: Option<Hint>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ChainTable {
    pub analyzer: String,
    /// `index`, `search`, `normalizer`: how fields use it.
    pub roles: Vec<String>,
    pub stages: Vec<StageRow>,
}

/// `stage` is `char_filter`, `tokenizer` or `filter`; `n` counts filters from 1, else 0; `class` is
/// `declared`, `builtin`, `undeclared` or `inline`.
#[derive(Debug, PartialEq, Eq)]
pub struct StageRow {
    pub stage: &'static str,
    pub n: usize,
    pub name: String,
    pub class: &'static str,
    pub params: Vec<(String, String)>,
}

/// Neutral note: a field's index and search analyzer differ.
#[derive(Debug, PartialEq, Eq)]
pub struct Hint {
    pub field: String,
    pub index: String,
    pub search: String,
    pub tokenizer_differs: bool,
    pub only_index: Vec<String>,
    pub only_search: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct PartDetails {
    pub kind: String,
    pub kind_type: Option<String>,
    pub name: String,
    pub owner: String,
    pub declaration: Option<(String, Option<u32>)>,
    pub stages: Vec<StageRow>,
    pub params: Vec<(String, String)>,
    pub used_by: Vec<String>,
    pub definition: Vec<(String, String)>,
}

pub fn details(t: &ServiceTopology, g: &SearchGraph, key: &Key) -> Details {
    let found = match key {
        Key::Node(i) => g.nodes.get(*i).and_then(|n| node_details(t, n)),
        Key::Part { owner, kind, name } => {
            part_details(t, g, *owner, kind, name).map(Details::Part)
        }
    };
    found.unwrap_or_else(|| Details::Other(SvcSearchDetail::default()))
}

/// An empty value means "default endpoint"; the view translates it.
fn field_params(f: &SearchField) -> Vec<(String, String)> {
    let mut p: Vec<_> = f
        .params
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if f.kind == "semantic_text" && !f.params.contains_key("inference_id") {
        p.insert(0, ("inference_id".into(), String::new()));
    }
    p
}

/// The details of field `path` of the index (or template) node `owner`.
pub fn field_details(
    t: &ServiceTopology,
    g: &SearchGraph,
    owner: usize,
    path: &str,
) -> Option<FieldDetails> {
    let node = g.nodes.get(owner)?;
    let d = node
        .artifact
        .and_then(|i| t.elasticsearch.artifacts.get(i))?
        .detail
        .as_ref()?;
    let i = d.fields.iter().position(|f| f.path == path)?;
    let f = &d.fields[i];
    let parts = search_analysis_parts(d);
    let chain = |kind: &str, name: &str| {
        let part = parts.iter().find(|p| p.kind == kind && p.name == name);
        FieldChain {
            name: name.to_string(),
            declared: part.is_some(),
            stages: part.map(|p| stage_rows(p, &parts)).unwrap_or_default(),
        }
    };
    let (index, search) = search_field_analyzers(d, path).map_or((None, None), |(i, s)| {
        (Some(chain("analyzer", &i)), Some(chain("analyzer", &s)))
    });
    Some(FieldDetails {
        path: f.path.clone(),
        kind: f.kind.clone(),
        role: search_field_role(&d.fields, i),
        index,
        search,
        normalizer: f.normalizer.as_deref().map(|n| chain("normalizer", n)),
        params: field_params(f),
        hint: hints_of(d, path),
    })
}

fn node_details(t: &ServiceTopology, n: &SearchNode) -> Option<Details> {
    let tag = |scope: &mut String| {
        if let Some(p) = product(t, n) {
            *scope = format!("{p} \u{b7} {scope}");
        }
    };
    if PAGE_KINDS.contains(&n.kind.as_str()) {
        let mut d = index_details(t, n);
        tag(&mut d.page.overview.scope);
        return Some(Details::Index(d));
    }
    if n.kind == "service" {
        return None;
    }
    // A name without declaration is an artifact with nothing but its name.
    let ghost: Option<SearchArtifact> = serde_json::from_value(serde_json::json!({
        "kind": n.kind, "name": n.title, "address": n.title, "scope": "",
        "patterns": [], "references": [], "evidence": [],
    }))
    .ok();
    let a = match n.artifact {
        Some(i) => t.elasticsearch.artifacts.get(i)?,
        None => ghost.as_ref()?,
    };
    let mut o = search::detail(a, t, evidence);
    let mut scope = o.scope.to_string();
    tag(&mut scope);
    o.scope = scope.into();
    Some(Details::Other(o))
}

fn index_details(t: &ServiceTopology, n: &SearchNode) -> IndexDetails {
    let page = index_page(t, n);
    let services = |access: &str| {
        let set: std::collections::BTreeSet<&str> = page
            .usages
            .iter()
            .filter(|u| u.access == access)
            .map(|u| u.service.as_str())
            .collect();
        set.into_iter().map(String::from).collect::<Vec<_>>()
    };
    let (writes, reads) = (services("write"), services("read"));
    let detail = n
        .artifact
        .and_then(|i| t.elasticsearch.artifacts.get(i))
        .and_then(|a| a.detail.as_ref());
    let mut out = IndexDetails {
        page,
        fields: Vec::new(),
        mapping_unread: mapping_unread(t, n),
        chains: Vec::new(),
        hints: Vec::new(),
        writes,
        reads,
    };
    if let Some(d) = detail {
        out.fields = field_rows(d);
        out.chains = chains(d);
        out.hints = hints(d);
    }
    out
}

fn field_rows(d: &SearchIndexDetail) -> Vec<FieldRow> {
    d.fields
        .iter()
        .enumerate()
        .map(|(i, f)| FieldRow {
            path: f.path.clone(),
            depth: search_field_depth(&d.fields, i),
            role: search_field_role(&d.fields, i),
            kind: f.kind.clone(),
            analyzer: f.analyzer.clone(),
            search_analyzer: f.search_analyzer.clone(),
            normalizer: f.normalizer.clone(),
        })
        .collect()
}

/// One table per declared analyzer or normalizer that fields use with a role of `FIELD_ROLES`.
fn chains(d: &SearchIndexDetail) -> Vec<ChainTable> {
    let parts = search_analysis_parts(d);
    let mut out = Vec::new();
    for u in search_analysis_users(d) {
        let mut roles: Vec<String> = Vec::new();
        for r in u
            .users
            .iter()
            .map(|x| x.role)
            .filter(|r| FIELD_ROLES.contains(r))
        {
            if !roles.iter().any(|x| x == r) {
                roles.push(r.to_string());
            }
        }
        let part = parts.iter().find(|p| p.kind == u.kind && p.name == u.name);
        if let (false, Some(p)) = (roles.is_empty(), part) {
            out.push(ChainTable {
                analyzer: u.name,
                roles,
                stages: stage_rows(p, &parts),
            });
        }
    }
    out
}

fn stage_rows(p: &SearchAnalysisPart, parts: &[SearchAnalysisPart]) -> Vec<StageRow> {
    let row = |stage: &'static str, n: usize, c: &ChainRef| StageRow {
        stage,
        n,
        name: c.name.clone(),
        class: c.class,
        params: stage_params(c, stage, p, parts),
    };
    let filters = p.filters.iter().enumerate();
    p.char_filters
        .iter()
        .map(|c| row("char_filter", 0, c))
        .chain(p.tokenizer.iter().map(|c| row("tokenizer", 0, c)))
        .chain(filters.map(|(i, c)| row("filter", i + 1, c)))
        .collect()
}

/// Built in and undeclared members have no parameters; an inline member's are `<type>.<key>` of
/// the chain owner.
fn stage_params(
    c: &ChainRef,
    stage: &str,
    owner: &SearchAnalysisPart,
    parts: &[SearchAnalysisPart],
) -> Vec<(String, String)> {
    match c.class {
        "declared" => parts
            .iter()
            .find(|x| x.kind == stage && x.name == c.name)
            .map(|x| x.params.clone())
            .unwrap_or_default(),
        "inline" => {
            let prefix = format!("{}.", c.name);
            owner
                .params
                .iter()
                .filter_map(|(k, v)| Some((k.strip_prefix(&prefix)?.to_string(), v.clone())))
                .collect()
        }
        _ => Vec::new(),
    }
}

fn hints_of(d: &SearchIndexDetail, path: &str) -> Option<Hint> {
    let x = search_analyzer_difference(d, path)?;
    Some(Hint {
        field: path.to_string(),
        index: x.index,
        search: x.search,
        tokenizer_differs: x.tokenizer_differs,
        only_index: x.only_index,
        only_search: x.only_search,
    })
}

fn hints(d: &SearchIndexDetail) -> Vec<Hint> {
    d.fields
        .iter()
        .filter_map(|f| hints_of(d, &f.path))
        .collect()
}

fn part_details(
    t: &ServiceTopology,
    g: &SearchGraph,
    owner: usize,
    kind: &str,
    name: &str,
) -> Option<PartDetails> {
    let node = g.nodes.get(owner)?;
    let a = t.elasticsearch.artifacts.get(node.artifact?)?;
    let d = a.detail.as_ref()?;
    let parts = search_analysis_parts(d);
    let p = parts.iter().find(|p| p.kind == kind && p.name == name)?;
    let used_by = search_analysis_users(d)
        .into_iter()
        .find(|u| u.kind == kind && u.name == name)
        .map(|u| {
            u.users
                .into_iter()
                .map(|x| match x.role {
                    "chain" => x.subject,
                    role => format!("{} \u{b7} {role}", x.subject),
                })
                .collect()
        })
        .unwrap_or_default();
    Some(PartDetails {
        kind: p.kind.clone(),
        kind_type: p.kind_type.clone(),
        name: p.name.clone(),
        owner: node.title.clone(),
        declaration: a.evidence.first().map(|e| (e.file.clone(), e.line)),
        stages: stage_rows(p, &parts),
        params: p.params.clone(),
        used_by,
        definition: definition(&p.definition),
    })
}

fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(text).collect::<Vec<_>>().join(", "),
        other => other.to_string(),
    }
}

/// Every top-level key of the compact JSON definition with its value as text.
fn definition(json: &str) -> Vec<(String, String)> {
    match serde_json::from_str::<Value>(json) {
        Ok(Value::Object(o)) => o.iter().map(|(k, v)| (k.clone(), text(v))).collect(),
        _ => Vec::new(),
    }
}
