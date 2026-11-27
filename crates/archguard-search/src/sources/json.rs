//! Shape classifier for Elasticsearch/OpenSearch definition bodies (ADR 0023, history 2026-11-25).
//! Pure over a parsed value: no file access. Rules of this format, by top-level keys only:
//! `index_patterns` (array of strings) -> index_template, composable when `template` is an object,
//! else legacy; `template` object holding `mappings`/`settings`/`aliases`, no patterns ->
//! component_template; `mappings` with a mapping key or `settings` with a setting key -> index;
//! `policy.phases` object -> ilm_policy; `policy.states` array -> ism_policy (flavor opensearch);
//! `processors` array of known ingest processors -> ingest_pipeline; `properties` of ES-only field
//! types -> index (mapping fragment); `analysis` of analysis kinds -> index settings. A body with
//! `$schema`, `openapi`, `apiVersion`, `swagger` or a mustache script is none of them.
// shortcut: serde_json without preserve_order sorts keys, so fields and analysis come out in key
// order, as Terraform's jsonencode does; enable the feature when file order is wanted.
use super::terraform_detail::{fields, flatten};
use super::terraform_eval::Json;
use archguard_core::{SearchIndexDetail, SearchReference};
use serde_json::{Map, Value};

/// An artifact kind of `SearchArtifact.kind`.
pub(crate) type Kind = &'static str;

pub(crate) struct Definition {
    pub(crate) kind: Kind,
    pub(crate) patterns: Vec<String>,
    /// composed_of, default/final pipeline, lifecycle.name, aliases
    pub(crate) references: Vec<SearchReference>,
    pub(crate) detail: Option<SearchIndexDetail>,
    /// Empty = Elasticsearch.
    pub(crate) flavor: &'static str,
    /// The key that decided the kind; its first line is the evidence line.
    pub(crate) key: &'static str,
}

const MAPPING_KEYS: &[&str] = &[
    "properties",
    "dynamic",
    "dynamic_templates",
    "_source",
    "_meta",
    "_routing",
    "runtime",
    "date_detection",
    "numeric_detection",
];
const SETTING_KEYS: &[&str] = &[
    "index",
    "analysis",
    "number_of_shards",
    "number_of_replicas",
    "refresh_interval",
];
const ANALYSIS_KINDS: &[&str] = &[
    "analyzer",
    "tokenizer",
    "filter",
    "char_filter",
    "normalizer",
];
/// Field types no JSON Schema uses (`integer`, `object`, `boolean` are shared and prove nothing).
const ES_ONLY_TYPES: &[&str] = &[
    "text",
    "keyword",
    "date",
    "long",
    "short",
    "byte",
    "float",
    "double",
    "half_float",
    "scaled_float",
    "nested",
    "geo_point",
    "geo_shape",
    "ip",
    "flattened",
    "dense_vector",
    "completion",
    "date_nanos",
    "binary",
    "constant_keyword",
    "wildcard",
    "search_as_you_type",
];
const PROCESSORS: &[&str] = &[
    "append",
    "attachment",
    "bytes",
    "circle",
    "community_id",
    "convert",
    "csv",
    "date",
    "date_index_name",
    "dissect",
    "dot_expander",
    "drop",
    "enrich",
    "fail",
    "fingerprint",
    "foreach",
    "geoip",
    "grok",
    "gsub",
    "html_strip",
    "inference",
    "join",
    "json",
    "kv",
    "lowercase",
    "network_direction",
    "pipeline",
    "registered_domain",
    "remove",
    "rename",
    "reroute",
    "script",
    "set",
    "set_security_user",
    "sort",
    "split",
    "terminate",
    "trim",
    "uppercase",
    "uri_parts",
    "urldecode",
    "user_agent",
];
const FOREIGN: &[&str] = &["$schema", "openapi", "swagger", "apiVersion"];

/// The ES object a body describes, or `None` (not ES-shaped, or a search template). `hint` is a
/// kind known from the API path of the request that carries the body; it replaces the shape rules.
pub(crate) fn search_definition(v: &Value, hint: Option<Kind>) -> Option<Definition> {
    let o = v.as_object()?;
    if o.get("script")
        .and_then(|s| s.get("lang"))
        .and_then(Value::as_str)
        == Some("mustache")
    {
        return None;
    }
    let (kind, key) = match hint {
        Some(k) => (k, ""),
        None => classify(o)?,
    };
    Some(build(o, kind, key))
}

fn classify(o: &Map<String, Value>) -> Option<(Kind, &'static str)> {
    if FOREIGN.iter().any(|k| o.contains_key(*k)) {
        return None;
    }
    template(o)
        .or_else(|| index(o))
        .or_else(|| policy(o))
        .or_else(|| pipeline(o))
        .or_else(|| fragment(o))
}

fn has_any(o: &Map<String, Value>, keys: &[&str]) -> bool {
    keys.iter().any(|k| o.contains_key(*k))
}

fn object<'a>(o: &'a Map<String, Value>, key: &str) -> Option<&'a Map<String, Value>> {
    o.get(key)?.as_object()
}

fn strings(v: Option<&Value>) -> Option<Vec<String>> {
    let items = v?.as_array()?;
    let out: Vec<String> = items
        .iter()
        .filter_map(|i| i.as_str().map(str::to_owned))
        .collect();
    (out.len() == items.len() && !out.is_empty()).then_some(out)
}

fn template(o: &Map<String, Value>) -> Option<(Kind, &'static str)> {
    if strings(o.get("index_patterns")).is_some() {
        return Some(("index_template", "index_patterns"));
    }
    let t = object(o, "template")?;
    has_any(t, &["mappings", "settings", "aliases"]).then_some(("component_template", "template"))
}

fn index(o: &Map<String, Value>) -> Option<(Kind, &'static str)> {
    if object(o, "mappings").is_some_and(|m| has_any(m, MAPPING_KEYS)) {
        return Some(("index", "mappings"));
    }
    let s = object(o, "settings")?;
    let es = has_any(s, SETTING_KEYS) || s.keys().any(|k| k.starts_with("index."));
    es.then_some(("index", "settings"))
}

fn policy(o: &Map<String, Value>) -> Option<(Kind, &'static str)> {
    let p = object(o, "policy")?;
    if p.get("phases").is_some_and(Value::is_object) {
        return Some(("ilm_policy", "phases"));
    }
    p.get("states")
        .is_some_and(Value::is_array)
        .then_some(("ism_policy", "states"))
}

fn pipeline(o: &Map<String, Value>) -> Option<(Kind, &'static str)> {
    let items = o.get("processors")?.as_array()?;
    let known = |p: &Value| {
        p.as_object()
            .is_some_and(|m| m.len() == 1 && m.keys().all(|k| PROCESSORS.contains(&k.as_str())))
    };
    (!items.is_empty() && items.iter().all(known)).then_some(("ingest_pipeline", "processors"))
}

/// A mapping fragment (`properties` that is no JSON Schema) or settings with only `analysis`.
fn fragment(o: &Map<String, Value>) -> Option<(Kind, &'static str)> {
    if is_mapping_fragment(o) {
        return Some(("index", "properties"));
    }
    let a = object(o, "analysis")?;
    (!a.is_empty() && a.keys().all(|k| ANALYSIS_KINDS.contains(&k.as_str())))
        .then_some(("index", "analysis"))
}

fn is_mapping_fragment(o: &Map<String, Value>) -> bool {
    if has_any(o, &["type", "required", "$id", "definitions", "$defs"]) {
        return false;
    }
    let Some(props) = object(o, "properties") else {
        return false;
    };
    let es = |f: &Map<String, Value>| {
        f.get("type")
            .and_then(Value::as_str)
            .is_some_and(|t| ES_ONLY_TYPES.contains(&t))
            || has_any(f, &["analyzer", "fields"])
    };
    let fields: Vec<_> = props.values().filter_map(Value::as_object).collect();
    fields.len() == props.len() && fields.iter().any(|f| es(f))
}

fn build(o: &Map<String, Value>, kind: Kind, key: &'static str) -> Definition {
    let flavor = if kind == "ism_policy" {
        "opensearch"
    } else {
        ""
    };
    let patterns = strings(o.get("index_patterns")).unwrap_or_default();
    let mut def = Definition {
        kind,
        patterns,
        references: vec![],
        detail: None,
        flavor,
        key,
    };
    if !matches!(kind, "index" | "index_template" | "component_template") {
        return def;
    }
    // Composable and component templates keep mappings and settings below `template`.
    let body = object(o, "template").unwrap_or(o);
    let whole = Value::Object(o.clone());
    let mappings = if key == "properties" {
        Some(&whole)
    } else {
        body.get("mappings")
    };
    let settings = if key == "analysis" {
        Some(&whole)
    } else {
        body.get("settings")
    };
    let detail = detail_from_json(mappings, settings);
    def.references = references(o, body, &detail);
    def.detail = (detail != SearchIndexDetail::default()).then_some(detail);
    def
}

/// Fields from `mappings.properties`, flattened settings and analysis; the same rules as the
/// Terraform reader (`terraform_detail`), the value converted to its `Json`.
pub(crate) fn detail_from_json(
    mappings: Option<&Value>,
    settings: Option<&Value>,
) -> SearchIndexDetail {
    let mut d = SearchIndexDetail::default();
    if let Some(p) = mappings.and_then(|m| m.get("properties")) {
        fields(&json(p), "", &mut d.fields);
    }
    if let Json::Obj(o) = settings.map(json).unwrap_or(Json::Null) {
        flatten("", &o, &mut d);
    }
    d
}

fn json(v: &Value) -> Json {
    match v {
        Value::Null => Json::Null,
        Value::Bool(b) => Json::Bool(*b),
        Value::Number(n) => Json::Num(n.to_string()),
        Value::String(s) => Json::Str(s.clone()),
        Value::Array(a) => Json::Arr(a.iter().map(json).collect()),
        Value::Object(o) => Json::Obj(o.iter().map(|(k, v)| (k.clone(), json(v))).collect()),
    }
}

fn reference(kind: &str, name: &str) -> SearchReference {
    SearchReference {
        kind: kind.into(),
        name: name.into(),
    }
}

fn references(
    top: &Map<String, Value>,
    body: &Map<String, Value>,
    detail: &SearchIndexDetail,
) -> Vec<SearchReference> {
    let mut out: Vec<_> = strings(top.get("composed_of"))
        .unwrap_or_default()
        .iter()
        .map(|n| reference("component_template", n))
        .collect();
    for (setting, kind) in [
        ("default_pipeline", "ingest_pipeline"),
        ("final_pipeline", "ingest_pipeline"),
        ("lifecycle.name", "ilm_policy"),
    ] {
        out.extend(detail.settings.get(setting).map(|n| reference(kind, n)));
    }
    out.extend(
        object(body, "aliases")
            .into_iter()
            .flat_map(Map::keys)
            .map(|n| reference("alias", n)),
    );
    out
}
