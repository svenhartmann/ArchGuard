//! Vector fields keep their parameters (ticket 31).
use archguard_core::{SearchArtifact, SearchField, ServiceTopology};
use archguard_flow_common::support::Discovery;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/es-classdiagram/31")
}

fn analyze() -> ServiceTopology {
    let root = root();
    let mut report = ServiceTopology::default();
    let discovery = Discovery::new(&root, &mut report).expect("discovery");
    let read = |_: &Path, p: &Path| std::fs::read_to_string(p).ok();
    archguard_search::analyze(&root, &discovery, &read, &mut report);
    report
}

fn fields<'a>(report: &'a ServiceTopology, name: &str) -> &'a [SearchField] {
    let a: &SearchArtifact = report
        .elasticsearch
        .artifacts
        .iter()
        .find(|a| a.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("no artifact {name}"));
    &a.detail.as_ref().expect("detail").fields
}

fn params(fs: &[SearchField], path: &str) -> Vec<(String, String)> {
    let f = fs.iter().find(|f| f.path == path).expect(path);
    // Through serde: `params` is skipped when empty.
    let v = serde_json::to_value(f).expect("json");
    v.get("params")
        .and_then(|p| p.as_object())
        .map(|o| {
            o.iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
                .collect()
        })
        .unwrap_or_default()
}

fn check(fs: &[SearchField]) {
    assert_eq!(
        params(fs, "embedding"),
        [
            ("dims", "384"),
            ("element_type", "float"),
            (
                "index_options",
                r#"{"ef_construction":100,"m":16,"type":"bbq_hnsw"}"#
            ),
            ("similarity", "cosine"),
        ]
        .map(|(k, v)| (k.to_string(), v.to_string()))
    );
    assert!(
        params(fs, "body")
            .iter()
            .any(|(k, v)| k == "inference_id" && v == "my-e5-endpoint")
    );
    // No invented default endpoint.
    assert!(params(fs, "summary").is_empty());
    assert!(params(fs, "tokens").is_empty());
    assert!(
        fs.iter()
            .any(|f| f.path == "tokens" && f.kind == "sparse_vector")
    );
    assert!(params(fs, "title").is_empty());
}

#[test]
fn json_mapping_keeps_vector_parameters() {
    let r = analyze();
    check(fields(&r, "vec-docs"));
    let body = params(fields(&r, "vec-docs"), "body");
    assert!(body.contains(&("search_inference_id".into(), "my-e5-search".into())));
}

#[test]
fn terraform_mapping_keeps_vector_parameters() {
    check(fields(&analyze(), "tf-vectors"));
}

#[test]
fn semantic_text_alone_makes_a_mapping_fragment() {
    let r = analyze();
    assert_eq!(fields(&r, "semantic-only")[0].kind, "semantic_text");
}

#[test]
fn a_string_schema_with_dims_is_no_index() {
    let r = analyze();
    assert!(
        !r.elasticsearch
            .artifacts
            .iter()
            .any(|a| a.name.as_deref() == Some("lookalike-schema"))
    );
}
