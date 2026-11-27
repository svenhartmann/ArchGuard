//! Kibana Dev Tools text (`PUT idx` + JSON body) becomes artifacts (ticket 33).
use archguard_core::{SearchArtifact, ServiceDiagnostic, ServiceTopology};
use archguard_flow_common::support::Discovery;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/es-classdiagram/33")
}

fn analyze() -> ServiceTopology {
    let root = root();
    let mut report = ServiceTopology::default();
    let discovery = Discovery::new(&root, &mut report).expect("discovery");
    let read = |_: &Path, p: &Path| {
        (!p.ends_with("unreadable.console"))
            .then(|| std::fs::read_to_string(p).ok())
            .flatten()
    };
    archguard_search::analyze(&root, &discovery, &read, &mut report);
    report
}

fn devtools(report: &ServiceTopology) -> Vec<&SearchArtifact> {
    let mut all: Vec<_> = report
        .elasticsearch
        .artifacts
        .iter()
        .filter(|a| a.scope.starts_with("devtools:"))
        .collect();
    all.sort_by_key(|a| (a.scope.clone(), a.name.clone()));
    all
}

fn gaps<'a>(report: &'a ServiceTopology, file: &str) -> Vec<&'a ServiceDiagnostic> {
    report
        .diagnostics
        .iter()
        .filter(|d| d.file == file && archguard_search::is_elasticsearch_gap(d))
        .collect()
}

#[test]
fn console_requests_yield_artifacts_and_search_calls_and_lookalikes_yield_nothing() {
    let report = analyze();
    let got: Vec<_> = devtools(&report)
        .iter()
        .map(|a| {
            format!(
                "{} {} {}",
                a.scope,
                a.kind,
                a.name.as_deref().unwrap_or("?")
            )
        })
        .collect();
    let want = [
        "devtools:demo index books",
        "devtools:demo ingest_pipeline books-pipe",
        "devtools:demo index_template books-template",
        "devtools:notes index notes-idx",
        "devtools:notes ilm_policy notes-ilm",
    ];
    assert_eq!(
        got, want,
        "GET and POST _search, prose and host URLs create nothing"
    );
    for file in [
        "lookalike/prose.txt",
        "lookalike/host.txt",
        "lookalike/plain.json",
    ] {
        assert!(gaps(&report, file).is_empty(), "{file}");
    }
}

#[test]
fn evidence_is_the_request_line_and_the_body_goes_through_the_classifier() {
    let report = analyze();
    let by = |n: &str| {
        devtools(&report)
            .into_iter()
            .find(|a| a.name.as_deref() == Some(n))
            .unwrap_or_else(|| panic!("no artifact {n}"))
    };
    let books = by("books");
    assert_eq!(books.evidence[0].file, "demo/setup.json");
    assert_eq!(books.evidence[0].line, Some(2), "after the # comment");
    assert_eq!(books.detail.as_ref().expect("detail").fields.len(), 2);
    assert_eq!(
        by("books-template").evidence[0].line,
        Some(16),
        "after the // comment"
    );
    assert_eq!(by("books-template").patterns, ["books-*"]);
    assert_eq!(by("books-pipe").evidence[0].line, Some(24));
    assert_eq!(by("notes-idx").evidence[0].file, "notes/mapping.txt");
    assert_eq!(by("notes-ilm").evidence[0].file, "notes/policy.console");
}

#[test]
fn a_recognized_console_file_has_no_json_gap_of_its_own() {
    let report = analyze();
    assert!(gaps(&report, "demo/setup.json").is_empty());
}

#[test]
fn every_unread_form_yields_a_gap_naming_its_file() {
    let report = analyze();
    let dir = root().join("unread");
    let mut seen = 0;
    for entry in std::fs::read_dir(dir).expect("unread dir") {
        let name = entry.expect("entry").file_name();
        let rel = format!("unread/{}", name.to_string_lossy());
        assert!(!gaps(&report, &rel).is_empty(), "no gap for {rel}");
        seen += 1;
    }
    assert!(seen >= 5);
    assert!(
        devtools(&report)
            .iter()
            .all(|a| !a.evidence[0].file.starts_with("unread/"))
    );
}
