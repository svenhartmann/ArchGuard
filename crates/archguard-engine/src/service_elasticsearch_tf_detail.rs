//! Literal mapping, settings and analysis of an `elasticstack_elasticsearch_index` (ADR 0023).
//! All or nothing per attribute: one non-literal part and the attribute is not read.
use super::service_terraform_hcl::{attr, literal};
use archguard_core::{SearchAnalysis, SearchField, SearchIndexDetail};
use hcl_edit::expr::{Expression, ObjectKey};
use hcl_edit::parser;
use hcl_edit::structure::Body;

/// Scalar and set attributes of provider elasticstack v0.11.13 that are index settings:
/// the documented ones minus name, alias, mappings, deletion_protection, master_timeout,
/// timeout, wait_for_active_shards, settings_raw, elasticsearch_connection and the list
/// `sort_order`; the deprecated `settings` block is no attribute.
const SETTINGS: &[&str] = &[
    "analyze_max_token_count",
    "auto_expand_replicas",
    "blocks_metadata",
    "blocks_read",
    "blocks_read_only",
    "blocks_read_only_allow_delete",
    "blocks_write",
    "codec",
    "default_pipeline",
    "final_pipeline",
    "gc_deletes",
    "highlight_max_analyzed_offset",
    "indexing_slowlog_level",
    "indexing_slowlog_source",
    "indexing_slowlog_threshold_index_debug",
    "indexing_slowlog_threshold_index_info",
    "indexing_slowlog_threshold_index_trace",
    "indexing_slowlog_threshold_index_warn",
    "load_fixed_bitset_filters_eagerly",
    "mapping_coerce",
    "max_docvalue_fields_search",
    "max_inner_result_window",
    "max_ngram_diff",
    "max_refresh_listeners",
    "max_regex_length",
    "max_rescore_window",
    "max_result_window",
    "max_script_fields",
    "max_shingle_diff",
    "max_terms_count",
    "number_of_replicas",
    "number_of_routing_shards",
    "number_of_shards",
    "query_default_field",
    "refresh_interval",
    "routing_allocation_enable",
    "routing_partition_size",
    "routing_rebalance_enable",
    "search_idle_after",
    "search_slowlog_level",
    "search_slowlog_threshold_fetch_debug",
    "search_slowlog_threshold_fetch_info",
    "search_slowlog_threshold_fetch_trace",
    "search_slowlog_threshold_fetch_warn",
    "search_slowlog_threshold_query_debug",
    "search_slowlog_threshold_query_info",
    "search_slowlog_threshold_query_trace",
    "search_slowlog_threshold_query_warn",
    "shard_check_on_startup",
    "sort_field",
    "unassigned_node_left_delayed_timeout",
];

/// Attribute and the analysis kind it defines.
const ANALYSIS: &[(&str, &str)] = &[
    ("analysis_analyzer", "analyzer"),
    ("analysis_char_filter", "char_filter"),
    ("analysis_filter", "filter"),
    ("analysis_normalizer", "normalizer"),
    ("analysis_tokenizer", "tokenizer"),
];

/// A fully literal value; objects keep the written order.
enum Json {
    Null,
    Bool(bool),
    Num(String),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    fn compact(&self) -> String {
        let quote = |s: &str| serde_json::to_string(s).unwrap_or_default();
        match self {
            Json::Null => "null".into(),
            Json::Bool(b) => b.to_string(),
            Json::Num(n) => n.clone(),
            Json::Str(s) => quote(s),
            Json::Arr(a) => format!(
                "[{}]",
                a.iter().map(Json::compact).collect::<Vec<_>>().join(",")
            ),
            Json::Obj(o) => format!(
                "{{{}}}",
                o.iter()
                    .map(|(k, v)| format!("{}:{}", quote(k), v.compact()))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        }
    }

    fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(o) => o.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    fn str(&self, key: &str) -> Option<String> {
        match self.get(key)? {
            Json::Str(s) => Some(s.clone()),
            _ => None,
        }
    }
}

fn tree(e: &Expression) -> Option<Json> {
    Some(match e {
        Expression::Null(_) => Json::Null,
        Expression::Bool(b) => Json::Bool(*b.value()),
        Expression::Number(n) => Json::Num(n.value().to_string()),
        Expression::String(_) | Expression::StringTemplate(_) | Expression::HeredocTemplate(_) => {
            Json::Str(literal(e)?)
        }
        Expression::Array(a) => Json::Arr(a.iter().map(tree).collect::<Option<_>>()?),
        Expression::Object(o) => Json::Obj(
            o.iter()
                .map(|(k, v)| {
                    let key = match k {
                        ObjectKey::Ident(i) => i.as_str().to_owned(),
                        ObjectKey::Expression(k) => literal(k)?,
                    };
                    Some((key, tree(v.expr())?))
                })
                .collect::<Option<_>>()?,
        ),
        _ => return None,
    })
}

/// `jsonencode(<literal>)` or a literal JSON text (string or heredoc). JSON is read as HCL,
/// whose object syntax accepts it; `file()`, `local.*`, `var.*` and references are not literal.
fn json_attr(e: &Expression) -> Option<Json> {
    if let Expression::FuncCall(c) = e {
        let mut args = c.args.iter();
        return match (c.name.name.as_str(), args.next(), args.next()) {
            ("jsonencode", Some(a), None) => tree(a),
            _ => None,
        };
    }
    let text = literal(e).filter(|_| !matches!(e, Expression::Number(_) | Expression::Bool(_)))?;
    let body = parser::parse_body(&format!("x = {text}")).ok()?;
    tree(attr(&body, "x")?)
}

fn setting(e: &Expression) -> Option<String> {
    match tree(e)? {
        Json::Str(s) => Some(s),
        Json::Null | Json::Obj(_) => None,
        other => Some(other.compact()),
    }
}

fn fields(props: &Json, prefix: &str, out: &mut Vec<SearchField>) {
    let Json::Obj(props) = props else { return };
    for (name, def) in props {
        let path = format!("{prefix}{name}");
        out.push(SearchField {
            path: path.clone(),
            kind: def.str("type").unwrap_or_else(|| "object".into()),
            analyzer: def.str("analyzer"),
            search_analyzer: def.str("search_analyzer"),
            normalizer: def.str("normalizer"),
        });
        for sub in ["fields", "properties"] {
            if let Some(children) = def.get(sub) {
                fields(children, &format!("{path}."), out);
            }
        }
    }
}

/// The detail of an index body and the attributes that were present but not read.
pub(super) fn read(body: &Body) -> (Option<SearchIndexDetail>, Vec<&'static str>) {
    let (mut detail, mut missed, mut any) = (SearchIndexDetail::default(), vec![], false);
    if let Some(e) = attr(body, "mappings") {
        match json_attr(e) {
            Some(m) => {
                any = true;
                m.get("properties")
                    .iter()
                    .for_each(|p| fields(p, "", &mut detail.fields));
            }
            None => missed.push("mappings"),
        }
    }
    for (key, kind) in ANALYSIS {
        let Some(e) = attr(body, key) else { continue };
        match json_attr(e) {
            Some(Json::Obj(items)) => {
                any = true;
                detail
                    .analysis
                    .extend(items.iter().map(|(name, def)| SearchAnalysis {
                        kind: (*kind).into(),
                        name: name.clone(),
                        definition: def.compact(),
                    }));
            }
            _ => missed.push(key),
        }
    }
    for key in SETTINGS {
        let Some(e) = attr(body, key) else { continue };
        match setting(e) {
            Some(v) => {
                any = true;
                detail.settings.insert((*key).into(), v);
            }
            None => missed.push(key),
        }
    }
    (any.then_some(detail), missed)
}
