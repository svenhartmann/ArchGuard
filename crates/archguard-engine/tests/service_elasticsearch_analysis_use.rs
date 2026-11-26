//! Who uses an analysis part, a field's role and index/search differences (es-classdiagram 02).
use archguard_core::{SearchIndexDetail, ServiceTopology};
use archguard_engine::{
    SearchFieldRole, analyze_services, search_analysis_users, search_analyzer_difference,
    search_field_depth, search_field_role,
};
use std::path::Path;

fn detail() -> SearchIndexDetail {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/es-classdiagram/02/terraform");
    let r: ServiceTopology = analyze_services(&root).unwrap();
    r.elasticsearch
        .artifacts
        .iter()
        .find(|a| a.kind == "index" && a.name.as_deref() == Some("docs"))
        .and_then(|a| a.detail.clone())
        .expect("docs detail")
}

fn users(d: &SearchIndexDetail, kind: &str, name: &str) -> Vec<(String, String)> {
    search_analysis_users(d)
        .into_iter()
        .find(|u| u.kind == kind && u.name == name)
        .unwrap_or_else(|| panic!("{kind} {name} missing"))
        .users
        .into_iter()
        .map(|u| (u.subject, u.role.to_string()))
        .collect()
}

fn s(a: &str, b: &str) -> (String, String) {
    (a.into(), b.into())
}

#[test]
fn users_roles_and_differences() {
    let d = detail();
    let role = |p: &str| {
        let i = d.fields.iter().position(|f| f.path == p).unwrap();
        search_field_role(&d.fields, i)
    };
    assert_eq!(role("title"), SearchFieldRole::Field);
    assert_eq!(role("title.raw"), SearchFieldRole::MultiField);
    assert_eq!(role("author.name"), SearchFieldRole::Property);
    assert_eq!(role("lines.sku"), SearchFieldRole::Property);
    let i = d
        .fields
        .iter()
        .position(|f| f.path == "author.name")
        .unwrap();
    assert_eq!(search_field_depth(&d.fields, i), 1);

    assert_eq!(
        users(&d, "analyzer", "idx_a"),
        [
            s("title", "index"),
            s("same", "index"),
            s("author.name", "index")
        ]
    );
    assert_eq!(users(&d, "analyzer", "qry_a"), [s("title", "search")]);
    assert_eq!(
        users(&d, "analyzer", "shingle_a"),
        [s("title.shingle", "index")]
    );
    assert_eq!(
        users(&d, "normalizer", "fold_n"),
        [s("title.raw", "normalizer")]
    );
    assert_eq!(users(&d, "analyzer", "default"), [s("body", "default")]);
    assert_eq!(
        users(&d, "analyzer", "default_search"),
        [
            s("title.shingle", "default_search"),
            s("body", "default_search"),
            s("author.name", "default_search")
        ]
    );
    assert!(users(&d, "analyzer", "unused_a").is_empty());
    assert_eq!(
        users(&d, "filter", "my_stop"),
        [s("analyzer:idx_a", "chain"), s("analyzer:twin_a", "chain")]
    );

    let diff = search_analyzer_difference(&d, "title").unwrap();
    assert_eq!(
        (diff.index.as_str(), diff.search.as_str()),
        ("idx_a", "qry_a")
    );
    assert!(diff.tokenizer_differs);
    assert_eq!(diff.only_index, ["dash_map", "my_stop"]);
    assert_eq!(diff.only_search, ["syn"]);
    assert!(
        search_analyzer_difference(&d, "same").is_none(),
        "lookalike"
    );
    // `default_search` beats the field's own `analyzer` in ES 8.19.
    let body = search_analyzer_difference(&d, "body").unwrap();
    assert_eq!(
        (body.index.as_str(), body.search.as_str()),
        ("default", "default_search")
    );
    assert!(!body.tokenizer_differs);
    assert_eq!(
        (body.only_index.len(), body.only_search.as_slice()),
        (0, ["trim".to_string()].as_slice())
    );
    let sh = search_analyzer_difference(&d, "title.shingle").unwrap();
    assert_eq!(
        (sh.index.as_str(), sh.search.as_str()),
        ("shingle_a", "default_search")
    );
}
