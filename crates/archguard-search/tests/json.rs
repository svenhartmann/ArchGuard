//! Raw JSON/YAML definition files become Elasticsearch artifacts by their key shape (ticket 21).
use archguard_core::{SearchArtifact, ServiceTopology};
use archguard_flow_common::support::Discovery;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/es-classdiagram/21")
}

fn analyze() -> ServiceTopology {
    let root = root();
    let mut report = ServiceTopology::default();
    let discovery = Discovery::new(&root, &mut report).expect("discovery");
    let read = |_: &Path, p: &Path| std::fs::read_to_string(p).ok();
    archguard_search::analyze(&root, &discovery, &read, &mut report);
    report
}

fn artifact<'a>(report: &'a ServiceTopology, name: &str) -> &'a SearchArtifact {
    report
        .elasticsearch
        .artifacts
        .iter()
        .find(|a| a.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("no artifact {name}"))
}

#[test]
fn shapes_become_artifacts() {
    let report = analyze();
    let mut got: Vec<String> = report
        .elasticsearch
        .artifacts
        .iter()
        .map(|a| {
            format!(
                "{} {} {} {}",
                a.kind,
                a.name.as_deref().unwrap_or("?"),
                a.scope,
                a.flavor
            )
        })
        .collect();
    got.sort();
    let want = [
        "component_template base-component json:elasticsearch ",
        "ilm_policy hot-ilm json:elasticsearch ",
        "index analysis-only json:elasticsearch ",
        "index customers-index json:elasticsearch ",
        "index product-mapping json:mappings ",
        "index_template legacy-logs json:elasticsearch ",
        "index_template orders-template json:elasticsearch ",
        "ingest_pipeline clean-pipeline json:pipelines ",
        "ingest_pipeline geo-pipeline json:elasticsearch ",
        "ism_policy rollover-ism json:opensearch opensearch",
    ];
    assert_eq!(got, want, "lookalikes and search templates must not match");
}

#[test]
fn template_detail_references_and_evidence() {
    let report = analyze();
    let t = artifact(&report, "orders-template");
    assert_eq!(t.patterns, ["orders-*"]);
    let refs: Vec<_> = t
        .references
        .iter()
        .map(|r| format!("{}:{}", r.kind, r.name))
        .collect();
    assert_eq!(
        refs,
        [
            "component_template:base-component",
            "ingest_pipeline:geo-pipeline",
            "ilm_policy:hot-ilm",
            "alias:orders-current"
        ]
    );
    let d = t.detail.as_ref().expect("detail");
    let fields: Vec<_> = d.fields.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(fields, ["placed", "title", "title.raw"]);
    assert_eq!(d.settings["number_of_shards"], "3");
    assert_eq!(d.analysis[0].name, "folded");
    let e = &t.evidence[0];
    assert_eq!(e.file, "elasticsearch/orders-template.json");
    assert_eq!(e.line, Some(2));
}

#[test]
fn unreadable_definition_under_an_elasticsearch_path_is_a_gap_elsewhere_it_is_not() {
    let report = analyze();
    let gaps: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| archguard_search::is_elasticsearch_gap(d))
        .map(|d| d.file.as_str())
        .collect();
    assert_eq!(gaps, ["elasticsearch/broken.json"]);
    assert!(!report.complete);
}
