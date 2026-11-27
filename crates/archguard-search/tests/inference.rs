//! Inference endpoints as artifacts and their references (ticket 32).
use archguard_core::{SearchArtifact, ServiceTopology};
use archguard_flow_common::support::Discovery;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/es-classdiagram/32")
}

fn analyze() -> ServiceTopology {
    let root = root();
    let mut report = ServiceTopology::default();
    let discovery = Discovery::new(&root, &mut report).expect("discovery");
    let read = |_: &Path, p: &Path| std::fs::read_to_string(p).ok();
    archguard_search::analyze(&root, &discovery, &read, &mut report);
    report
}

fn of<'a>(r: &'a ServiceTopology, kind: &str, name: &str) -> &'a SearchArtifact {
    r.elasticsearch
        .artifacts
        .iter()
        .find(|a| a.kind == kind && a.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("no {kind} {name}"))
}

fn refs(a: &SearchArtifact) -> Vec<String> {
    a.references
        .iter()
        .map(|r| format!("{} {}", r.kind, r.name))
        .collect()
}

#[test]
fn endpoints_come_from_console_http_and_json_with_settings() {
    let r = analyze();
    let e5 = of(&r, "inference_endpoint", "my-e5");
    let s = &e5.detail.as_ref().expect("detail").settings;
    assert_eq!(s["task"], "text_embedding");
    assert_eq!(s["service"], "elasticsearch");
    assert_eq!(s["model_id"], ".my-e5-small");
    assert!(
        !of(&r, "inference_endpoint", "my-elser")
            .detail
            .as_ref()
            .expect("d")
            .settings
            .contains_key("task")
    );
    of(&r, "inference_endpoint", "http-rerank");
    of(&r, "inference_endpoint", "my-sparse");
}

#[test]
fn semantic_text_and_inference_processor_reference_their_endpoint() {
    let r = analyze();
    assert_eq!(
        refs(of(&r, "index", "docs-index")),
        [
            "inference_endpoint my-e5",
            "inference_endpoint my-e5",
            "inference_endpoint ghost-endpoint"
        ]
    );
    assert_eq!(
        refs(of(&r, "ingest_pipeline", "embed-pipe")),
        ["inference_endpoint my-elser"]
    );
}

#[test]
fn an_inference_key_outside_processors_is_no_reference() {
    let r = analyze();
    for (kind, name) in [("ingest_pipeline", "meta"), ("index", "meta-index")] {
        assert!(of(&r, kind, name).references.is_empty(), "{name}");
    }
}

#[test]
fn every_unread_form_yields_a_gap_naming_its_file() {
    let r = analyze();
    for f in ["unread/var-id.console", "unread/bad-body.console"] {
        assert!(
            r.diagnostics
                .iter()
                .any(|d| d.file == f && archguard_search::is_elasticsearch_gap(d)),
            "no gap for {f}"
        );
    }
    assert!(
        r.elasticsearch
            .artifacts
            .iter()
            .all(|a| !a.evidence[0].file.starts_with("unread/"))
    );
}
