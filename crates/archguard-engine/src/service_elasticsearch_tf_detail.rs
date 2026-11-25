//! Literal mapping, settings and analysis of an `elasticstack_elasticsearch_index` (ADR 0023).
//! All or nothing per attribute: one non-literal part and the attribute is not read.
use super::service_elasticsearch_tf_locals::lookup;
use super::service_terraform::ModuleCtx;
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
pub(super) enum Json {
    Null,
    Bool(bool),
    Num(String),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub(super) fn compact(&self) -> String {
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

    pub(super) fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(o) => o.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub(super) fn str(&self, key: &str) -> Option<String> {
        match self.get(key)? {
            Json::Str(s) => Some(s.clone()),
            _ => None,
        }
    }
}

/// Reference hops allowed from an attribute: index -> local -> local is depth 3 (ADR 0023).
const MAX_HOPS: usize = 2;

/// Addresses of the expressions followed so far; its length is the depth, a repeat is a cycle.
type Path = Vec<usize>;

fn object(
    o: &hcl_edit::expr::Object,
    ctx: Option<&ModuleCtx>,
    path: &mut Path,
) -> Option<Vec<(String, Json)>> {
    o.iter()
        .map(|(k, v)| {
            let key = match k {
                ObjectKey::Ident(i) => i.as_str().to_owned(),
                ObjectKey::Expression(k) => literal(k)?,
            };
            Some((key, tree(v.expr(), ctx, path)?))
        })
        .collect()
}

/// Follows a `local.*` or sibling-index reference and reads what it points to with `read`.
fn follow<T>(
    ctx: Option<&ModuleCtx>,
    e: &Expression,
    path: &mut Path,
    read: impl FnOnce(&Expression, &mut Path) -> Option<T>,
) -> Option<T> {
    let found = lookup(ctx?, e)?;
    let id = std::ptr::from_ref(found) as usize;
    if path.len() >= MAX_HOPS || path.contains(&id) {
        return None;
    }
    path.push(id);
    let out = read(found, path);
    path.pop();
    out
}

/// `merge()` of objects (shallow, the right value wins) or `concat()` of lists, all arguments
/// literal.
fn call(c: &hcl_edit::expr::FuncCall, ctx: Option<&ModuleCtx>, path: &mut Path) -> Option<Json> {
    let name = c.name.name.as_str();
    if c.args.expand_final() || !matches!(name, "merge" | "concat") {
        return None;
    }
    let mut objects: Vec<(String, Json)> = vec![];
    let mut items = vec![];
    for a in c.args.iter() {
        match (name, tree(a, ctx, path)?) {
            ("merge", Json::Obj(o)) => {
                for (k, v) in o {
                    match objects.iter_mut().find(|(x, _)| *x == k) {
                        Some(slot) => slot.1 = v,
                        None => objects.push((k, v)),
                    }
                }
            }
            ("concat", Json::Arr(a)) => items.extend(a),
            _ => return None,
        }
    }
    Some(if name == "merge" {
        Json::Obj(objects)
    } else {
        Json::Arr(items)
    })
}

fn tree(e: &Expression, ctx: Option<&ModuleCtx>, path: &mut Path) -> Option<Json> {
    Some(match e {
        Expression::Null(_) => Json::Null,
        Expression::Bool(b) => Json::Bool(*b.value()),
        Expression::Number(n) => Json::Num(n.value().to_string()),
        Expression::String(_) | Expression::StringTemplate(_) | Expression::HeredocTemplate(_) => {
            Json::Str(literal(e)?)
        }
        Expression::Array(a) => Json::Arr(
            a.iter()
                .map(|x| tree(x, ctx, path))
                .collect::<Option<_>>()?,
        ),
        Expression::Object(o) => Json::Obj(object(o, ctx, path)?),
        Expression::FuncCall(c) => call(c, ctx, path)?,
        Expression::Traversal(_) => return follow(ctx, e, path, |f, p| tree(f, ctx, p)),
        _ => return None,
    })
}

/// `jsonencode(<literal>)` or a literal JSON text (string or heredoc). JSON is read as HCL,
/// whose object syntax accepts it; `file()`, `var.*` and unresolved references are not literal.
pub(super) fn json_attr(e: &Expression, ctx: Option<&ModuleCtx>) -> Option<Json> {
    json_at(e, ctx, &mut vec![])
}

fn json_at(e: &Expression, ctx: Option<&ModuleCtx>, path: &mut Path) -> Option<Json> {
    if let Expression::Traversal(_) = e {
        return follow(ctx, e, path, |f, p| json_at(f, ctx, p));
    }
    if let Expression::FuncCall(c) = e {
        let mut args = c.args.iter();
        return match (c.name.name.as_str(), args.next(), args.next()) {
            ("jsonencode", Some(a), None) => tree(a, ctx, path),
            ("jsondecode", Some(a), None) => json_at(a, ctx, path),
            _ => None,
        };
    }
    let text = literal(e).filter(|_| !matches!(e, Expression::Number(_) | Expression::Bool(_)))?;
    let body = parser::parse_body(&format!("x = {text}")).ok()?;
    tree(attr(&body, "x")?, None, path)
}

fn setting(e: &Expression, ctx: Option<&ModuleCtx>) -> Option<String> {
    match tree(e, ctx, &mut vec![])? {
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
