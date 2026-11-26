//! Spring Data Elasticsearch / OpenSearch annotations become index artifacts (ticket 23).
use archguard_core::{SearchArtifact, ServiceTopology};
use archguard_flow_common::support::Discovery;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/es-classdiagram/23")
}

fn analyze() -> ServiceTopology {
    let root = root();
    let mut report = ServiceTopology::default();
    let discovery = Discovery::new(&root, &mut report).expect("discovery");
    let read = |_: &Path, p: &Path| std::fs::read_to_string(p).ok();
    archguard_search::analyze(&root, &discovery, &read, &mut report);
    report
}

fn spring(report: &ServiceTopology) -> Vec<&SearchArtifact> {
    let mut all: Vec<_> = report
        .elasticsearch
        .artifacts
        .iter()
        .filter(|a| a.scope.starts_with("spring:"))
        .collect();
    all.sort_by_key(|a| a.name.clone());
    all
}

fn named<'a>(report: &'a ServiceTopology, name: &str) -> &'a SearchArtifact {
    spring(report)
        .into_iter()
        .find(|a| a.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("no artifact {name}"))
}

#[test]
fn only_documents_of_the_spring_packages_are_artifacts() {
    let report = analyze();
    let got: Vec<_> = spring(&report)
        .iter()
        .map(|a| {
            format!(
                "{} {} {}",
                a.kind,
                a.name.as_deref().unwrap_or("?"),
                a.flavor
            )
        })
        .collect();
    let want = [
        "index customers-v2 opensearch",
        "index escape-idx ",
        "index mapped-idx ",
        "index missing-idx ",
        "index orders-v1 ",
    ];
    assert_eq!(
        got, want,
        "comments, other packages, test code and SpEL names are none"
    );
}

#[test]
fn fields_settings_and_evidence_of_an_annotated_class() {
    let report = analyze();
    let a = named(&report, "orders-v1");
    assert_eq!(a.scope, "spring:shop");
    let d = a.detail.as_ref().expect("detail");
    let fields: Vec<_> = d
        .fields
        .iter()
        .map(|f| {
            format!(
                "{} {} {:?} {:?} {:?}",
                f.path, f.kind, f.analyzer, f.search_analyzer, f.normalizer
            )
        })
        .collect();
    assert_eq!(
        fields,
        [
            "title text Some(\"folded\") Some(\"folded_search\") None",
            "sku_code keyword None None Some(\"lowercase\")",
            "customerName text Some(\"folded\") None None",
            "customerName.raw keyword None None None",
            "placed date None None None",
        ]
    );
    assert_eq!(d.settings["number_of_shards"], "3");
    assert_eq!(d.settings["number_of_replicas"], "2");
    assert_eq!(d.settings["refresh_interval"], "5s");
    let e = &a.evidence[0];
    assert_eq!(e.file, "shop/src/main/java/demo/shop/Order.java");
    assert_eq!(e.line, Some(15));
}

#[test]
fn constant_names_resolve_and_the_flavor_follows_the_import() {
    let report = analyze();
    let a = named(&report, "customers-v2");
    assert_eq!(a.flavor, "opensearch");
    assert_eq!(a.detail.as_ref().expect("detail").fields[0].path, "email");
}

#[test]
fn resource_files_replace_the_field_annotations() {
    let report = analyze();
    let d = named(&report, "mapped-idx")
        .detail
        .as_ref()
        .expect("detail");
    let fields: Vec<_> = d.fields.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(fields, ["headline", "tags"]);
    assert_eq!(d.settings["number_of_shards"], "2");
    assert_eq!(d.analysis[0].name, "std2");
}

#[test]
fn unresolved_names_and_paths_are_gaps_never_silent() {
    let report = analyze();
    let mut gaps: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| archguard_search::is_elasticsearch_gap(d))
        .map(|d| d.file.as_str())
        .collect();
    gaps.sort_unstable();
    gaps.dedup();
    let java = "shop/src/main/java/demo/shop";
    let want = [
        format!("{java}/Escape.java"),
        format!("{java}/Event.java"),
        format!("{java}/Missing.java"),
    ];
    assert_eq!(gaps, want);
    assert!(!report.complete);
    assert!(named(&report, "escape-idx").detail.is_none());
}
