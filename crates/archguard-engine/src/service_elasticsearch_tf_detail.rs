//! Literal mapping, settings and analysis of an `elasticstack_elasticsearch_index` (ADR 0023).
//! All or nothing per attribute: one non-literal part and the attribute is not read.
use super::service_elasticsearch_tf_eval::value;
pub(super) use super::service_elasticsearch_tf_eval::{Json, json_attr};
use archguard_core::{SearchAnalysis, SearchField, SearchIndexDetail};
use archguard_terraform::hcl::attr;
use archguard_terraform::module::ModuleCtx;
use hcl_edit::expr::Expression;
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

fn setting(e: &Expression, ctx: Option<&ModuleCtx>) -> Option<String> {
    match value(e, ctx)? {
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

/// Reads `mappings` into `detail.fields`; true when read.
fn mapping(
    body: &Body,
    ctx: Option<&ModuleCtx>,
    detail: &mut SearchIndexDetail,
    missed: &mut Vec<&'static str>,
) -> bool {
    let Some(e) = attr(body, "mappings") else {
        return false;
    };
    let Some(m) = json_attr(e, ctx) else {
        missed.push("mappings");
        return false;
    };
    m.get("properties")
        .iter()
        .for_each(|p| fields(p, "", &mut detail.fields));
    true
}

/// Settings object of a template flattened to dotted keys without `index.`; `analysis` becomes
/// analysis entries. Arrays and numbers keep their compact JSON, nulls are dropped.
fn flatten(prefix: &str, o: &[(String, Json)], d: &mut SearchIndexDetail) {
    for (k, v) in o {
        let path = if prefix.is_empty() {
            k.clone()
        } else {
            format!("{prefix}.{k}")
        };
        let key = path.strip_prefix("index.").unwrap_or(&path);
        match (v, key) {
            (Json::Obj(kinds), "analysis") => {
                for (kind, names) in kinds {
                    let Json::Obj(names) = names else { continue };
                    d.analysis
                        .extend(names.iter().map(|(name, def)| SearchAnalysis {
                            kind: kind.clone(),
                            name: name.clone(),
                            definition: def.compact(),
                        }));
                }
            }
            (Json::Obj(inner), _) => flatten(&path, inner, d),
            (Json::Null, _) => {}
            (Json::Str(s), _) => {
                d.settings.insert(key.into(), s.clone());
            }
            (other, _) => {
                d.settings.insert(key.into(), other.compact());
            }
        }
    }
}

/// The detail of a `template { mappings, settings }` block and the attributes not read.
pub(super) fn read_template(body: &Body) -> (Option<SearchIndexDetail>, Vec<&'static str>) {
    let (mut detail, mut missed) = (SearchIndexDetail::default(), vec![]);
    let mut any = mapping(body, None, &mut detail, &mut missed);
    if let Some(e) = attr(body, "settings") {
        match json_attr(e, None) {
            Some(Json::Obj(o)) => {
                any = true;
                flatten("", &o, &mut detail);
            }
            _ => missed.push("settings"),
        }
    }
    (any.then_some(detail), missed)
}

/// The detail of an index body and the attributes that were present but not read.
pub(super) fn read(body: &Body, ctx: &ModuleCtx) -> (Option<SearchIndexDetail>, Vec<&'static str>) {
    let (mut detail, mut missed, mut any) = (SearchIndexDetail::default(), vec![], false);
    any |= mapping(body, Some(ctx), &mut detail, &mut missed);
    for (key, kind) in ANALYSIS {
        let Some(e) = attr(body, key) else { continue };
        match json_attr(e, Some(ctx)) {
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
        match setting(e, Some(ctx)) {
            Some(v) => {
                any = true;
                detail.settings.insert((*key).into(), v);
            }
            None => missed.push(key),
        }
    }
    (any.then_some(detail), missed)
}
