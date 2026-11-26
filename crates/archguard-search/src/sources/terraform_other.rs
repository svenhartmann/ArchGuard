//! Elasticsearch artifacts of the Terraform providers `opensearch-project/opensearch` (opensearch_*)
//! and `phillbaker/elasticsearch` (elasticsearch_*), read like `elasticstack_*` (ADR 0023).
//! Rules of this format:
//! - a resource counts only when its type is in `ROWS` and `required_providers` (this module or a
//!   caller) names the matching source for the type prefix; otherwise it is a gap, never guessed;
//! - the name is a literal (no variables, locals or functions): else a gap and the artifact has no
//!   name; `file()`, `var.*` or a reference in a body attribute is a gap naming the attribute;
//! - `*_index`: `mappings`, `analysis_*` and the settings of `terraform_detail` plus `index_knn`,
//!   `index_similarity_default`, `rollover_alias`; `aliases` (JSON object) makes alias artifacts;
//!   `default_pipeline` makes an ingest_pipeline reference;
//! - every other resource with a `body` is read by `search_definition` with the kind as hint;
//! - `flavor` is `opensearch` for the opensearch-project provider and the ISM policy of phillbaker.
// shortcut: names read no `var.x` (the elasticstack reader does); add when a user needs it.
use super::json::search_definition;
use super::terraform_detail::{Json, json_attr, read as read_index};
use super::terraform_eval::value;
use crate::context::{SearchContext, SourceOutput};
use crate::findings::{EsGap, es_gap};
use archguard_core::{SearchArtifact, SearchIndexDetail, SearchReference, ServiceTopology};
use archguard_terraform::hcl::{attr, block, get, literal};
use archguard_terraform::module::{ModuleCtx, Resource};
use hcl_edit::structure::Body;

const NOTE: &str =
    "Declared Elasticsearch artifact read statically; no terraform run, state or cluster access.";
const OPENSEARCH: &str = "opensearch-project/opensearch";
const PHILLBAKER: &str = "phillbaker/elasticsearch";
/// Index attributes of both providers beyond `terraform_detail::SETTINGS`.
const EXTRA_SETTINGS: [&str; 2] = ["index_knn", "rollover_alias"];

/// Resource type, provider source, artifact kind, attribute holding the name, flavor, and whether
/// a `body` is read.
type Row = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    bool,
);
const ROWS: &[Row] = &[
    (
        "opensearch_index",
        OPENSEARCH,
        "index",
        "name",
        "opensearch",
        false,
    ),
    (
        "opensearch_index_template",
        OPENSEARCH,
        "index_template",
        "name",
        "opensearch",
        true,
    ),
    (
        "opensearch_composable_index_template",
        OPENSEARCH,
        "index_template",
        "name",
        "opensearch",
        true,
    ),
    (
        "opensearch_component_template",
        OPENSEARCH,
        "component_template",
        "name",
        "opensearch",
        true,
    ),
    (
        "opensearch_ingest_pipeline",
        OPENSEARCH,
        "ingest_pipeline",
        "name",
        "opensearch",
        true,
    ),
    (
        "opensearch_ism_policy",
        OPENSEARCH,
        "ism_policy",
        "policy_id",
        "opensearch",
        true,
    ),
    (
        "opensearch_data_stream",
        OPENSEARCH,
        "data_stream",
        "name",
        "opensearch",
        false,
    ),
    (
        "opensearch_script",
        OPENSEARCH,
        "stored_script",
        "script_id",
        "opensearch",
        false,
    ),
    (
        "opensearch_sm_policy",
        OPENSEARCH,
        "snapshot_lifecycle",
        "policy_name",
        "opensearch",
        true,
    ),
    (
        "elasticsearch_index",
        PHILLBAKER,
        "index",
        "name",
        "",
        false,
    ),
    (
        "elasticsearch_index_template",
        PHILLBAKER,
        "index_template",
        "name",
        "",
        true,
    ),
    (
        "elasticsearch_composable_index_template",
        PHILLBAKER,
        "index_template",
        "name",
        "",
        true,
    ),
    (
        "elasticsearch_component_template",
        PHILLBAKER,
        "component_template",
        "name",
        "",
        true,
    ),
    (
        "elasticsearch_ingest_pipeline",
        PHILLBAKER,
        "ingest_pipeline",
        "name",
        "",
        true,
    ),
    (
        "elasticsearch_data_stream",
        PHILLBAKER,
        "data_stream",
        "name",
        "",
        false,
    ),
    (
        "elasticsearch_script",
        PHILLBAKER,
        "stored_script",
        "script_id",
        "",
        false,
    ),
    (
        "elasticsearch_xpack_index_lifecycle_policy",
        PHILLBAKER,
        "ilm_policy",
        "name",
        "",
        true,
    ),
    (
        "elasticsearch_xpack_snapshot_lifecycle_policy",
        PHILLBAKER,
        "snapshot_lifecycle",
        "name",
        "",
        true,
    ),
    (
        "elasticsearch_xpack_watch",
        PHILLBAKER,
        "watch",
        "watch_id",
        "",
        true,
    ),
    (
        "elasticsearch_opensearch_ism_policy",
        PHILLBAKER,
        "ism_policy",
        "policy_id",
        "opensearch",
        true,
    ),
];

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

fn augment(report: &mut ServiceTopology, scope: &str, resources: &[Resource]) {
    for r in resources {
        let Some(row) = ROWS.iter().find(|row| row.0 == r.kind) else {
            continue;
        };
        if declares(&r.module, r.kind, row.1) {
            let out = read(report, scope, r, row);
            report.elasticsearch.artifacts.extend(out);
        } else {
            let path = &r.file.path;
            es_gap(report, EsGap::UnknownProvider, path, None, &[&r.address]);
        }
    }
}

/// Whether `required_providers` of the module or a caller gives the local provider name of the
/// resource type (its prefix) the source `want`, with or without a registry host.
fn declares(ctx: &ModuleCtx, kind: &str, want: &str) -> bool {
    let local = kind.split('_').next().unwrap_or_default();
    let source = ctx
        .terraform
        .iter()
        .filter_map(|t| block(&t.body, "required_providers"))
        .find_map(|rp| literal(get(attr(rp, local)?, "source")?));
    match source {
        Some(s) => s == want || s.ends_with(&format!("/{want}")),
        None => ctx.parent().is_some_and(|p| declares(p, kind, want)),
    }
}

fn read(report: &mut ServiceTopology, scope: &str, r: &Resource, row: &Row) -> Vec<SearchArtifact> {
    let (_, _, kind, name_key, flavor, has_body) = *row;
    let body = &r.block.body;
    let path = &r.file.path;
    let name = attr(body, name_key).and_then(literal);
    if name.is_none() {
        let what = format!("the {kind} name");
        es_gap(report, EsGap::NotLiteral, path, None, &[&r.address, &what]);
    }
    let mut missed: Vec<&'static str> = Vec::new();
    let mut a = make(r, scope, flavor, kind, name.clone(), "");
    let mut out = Vec::new();
    if kind == "index" {
        let (detail, aliases) = index(r, &mut missed, &mut a);
        a.detail = detail;
        out = aliases
            .into_iter()
            .map(|x| make_alias(r, scope, flavor, &name, x))
            .collect();
    } else if has_body {
        body_definition(r, &mut a, kind, &mut missed);
    } else if kind == "stored_script"
        && attr(body, "lang").and_then(literal).as_deref() == Some("mustache")
    {
        a.kind = "search_template".into();
    }
    if !missed.is_empty() {
        let parts = [r.address.as_str(), &missed.join(", ")];
        es_gap(report, EsGap::DetailNotLiteral, path, None, &parts);
    }
    out.insert(0, a);
    out
}

fn make(
    r: &Resource,
    scope: &str,
    flavor: &str,
    kind: &str,
    name: Option<String>,
    suffix: &str,
) -> SearchArtifact {
    SearchArtifact {
        flavor: flavor.into(),
        kind: kind.into(),
        name,
        address: format!("{}{suffix}", r.address),
        scope: scope.into(),
        patterns: vec![],
        references: vec![],
        evidence: vec![r.evidence(r.line(), "", NOTE)],
        origins: vec![],
        detail: None,
    }
}

fn make_alias(
    r: &Resource,
    scope: &str,
    flavor: &str,
    target: &Option<String>,
    alias: String,
) -> SearchArtifact {
    let mut a = make(
        r,
        scope,
        flavor,
        "alias",
        Some(alias.clone()),
        &format!(".alias.{alias}"),
    );
    a.references = target
        .iter()
        .map(|n| SearchReference {
            kind: "index".into(),
            name: n.clone(),
        })
        .collect();
    a
}

/// `patterns`, `references` and `detail` from the `body` JSON, with the kind as hint.
fn body_definition(
    r: &Resource,
    a: &mut SearchArtifact,
    kind: &'static str,
    missed: &mut Vec<&'static str>,
) {
    let Some(e) = attr(&r.block.body, "body") else {
        return;
    };
    let def = json_attr(e, Some(&r.module))
        .and_then(|j| serde_json::from_str(&j.compact()).ok())
        .and_then(|v| search_definition(&v, Some(kind)));
    match def {
        Some(d) => {
            a.patterns = d.patterns;
            a.references = d.references;
            a.detail = d.detail;
        }
        None => missed.push("body"),
    }
}

/// Detail of an index resource and the names of its aliases; what was not literal goes to `missed`.
fn index(
    r: &Resource,
    missed: &mut Vec<&'static str>,
    a: &mut SearchArtifact,
) -> (Option<SearchIndexDetail>, Vec<String>) {
    let body = &r.block.body;
    let ctx = Some(&*r.module);
    let (mut detail, miss) = read_index(body, &r.module);
    missed.extend(miss);
    for key in EXTRA_SETTINGS {
        let Some(e) = attr(body, key) else { continue };
        match value(e, ctx) {
            Some(j) => {
                detail
                    .get_or_insert_default()
                    .settings
                    .insert(key.into(), text(j));
            }
            None => missed.push(key),
        }
    }
    if let Some(e) = attr(body, "index_similarity_default") {
        match json_attr(e, ctx) {
            Some(j) => {
                let d = detail.get_or_insert_default();
                d.settings
                    .insert("index_similarity_default".into(), j.compact());
            }
            None => missed.push("index_similarity_default"),
        }
    }
    if let Some(p) = detail
        .as_ref()
        .and_then(|d| d.settings.get("default_pipeline"))
    {
        a.references.push(SearchReference {
            kind: "ingest_pipeline".into(),
            name: p.clone(),
        });
    }
    (detail, aliases(body, missed))
}

/// The alias names of the `aliases` JSON object attribute (no locals).
fn aliases(body: &Body, missed: &mut Vec<&'static str>) -> Vec<String> {
    let Some(e) = attr(body, "aliases") else {
        return vec![];
    };
    match json_attr(e, None) {
        Some(Json::Obj(items)) => items.into_iter().map(|(k, _)| k).collect(),
        _ => {
            missed.push("aliases");
            vec![]
        }
    }
}

fn text(j: Json) -> String {
    match j {
        Json::Str(s) => s,
        other => other.compact(),
    }
}
