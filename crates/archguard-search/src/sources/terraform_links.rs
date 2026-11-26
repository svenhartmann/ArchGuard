//! ILM policy, legacy `settings {}` blocks and watch indices of Elasticsearch resources (ADR 0023).
use super::terraform_detail::{Json, json_attr};
use archguard_core::{SearchIndexDetail, SearchReference};
use archguard_terraform::hcl::{attr, literal};
use hcl_edit::structure::Body;
use std::collections::BTreeMap;

/// The ILM policy named by the setting `lifecycle.name`.
pub(super) fn ilm(detail: Option<&SearchIndexDetail>) -> Option<SearchReference> {
    Some(SearchReference {
        kind: "ilm_policy".into(),
        name: detail?.settings.get("lifecycle.name")?.clone(),
    })
}

/// Settings of the deprecated `settings { setting { name, value } }` blocks without `index.`;
/// true when one of them is not literal.
pub(super) fn legacy_settings(body: &Body) -> (BTreeMap<String, String>, bool) {
    let (mut out, mut bad) = (BTreeMap::new(), false);
    for setting in body
        .get_blocks("settings")
        .flat_map(|b| b.body.get_blocks("setting"))
    {
        let read = |k| attr(&setting.body, k).and_then(literal);
        match (read("name"), read("value")) {
            (Some(n), Some(v)) => {
                out.insert(n.strip_prefix("index.").unwrap_or(&n).to_owned(), v);
            }
            _ => bad = true,
        }
    }
    (out, bad)
}

/// Literal `input.search.request.indices` of a watch: literal names, patterns (with `*`) and
/// whether `input` is not literal. Other inputs (`http`, ...) have no index.
pub(super) fn watch_indices(body: &Body) -> (Vec<String>, Vec<String>, bool) {
    let Some(e) = attr(body, "input") else {
        return Default::default();
    };
    let Some(input) = json_attr(e, None) else {
        return (vec![], vec![], true);
    };
    let indices = input
        .get("search")
        .and_then(|s| s.get("request"))
        .and_then(|r| r.get("indices"));
    let names: Vec<String> = match indices {
        Some(Json::Str(s)) => vec![s.clone()],
        Some(Json::Arr(a)) => a
            .iter()
            .filter_map(|i| match i {
                Json::Str(s) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        _ => vec![],
    };
    let (patterns, names) = names.into_iter().partition(|n| n.contains('*'));
    (names, patterns, false)
}
