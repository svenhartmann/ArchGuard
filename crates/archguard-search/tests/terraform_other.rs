//! Terraform providers opensearch-project/opensearch and phillbaker/elasticsearch (ticket 22).
use archguard_core::{SearchArtifact, ServiceTopology};
use archguard_flow_common::support::Discovery;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/es-classdiagram/22")
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

fn gaps(report: &ServiceTopology, dir: &str) -> Vec<String> {
    let mut out: Vec<String> = report
        .diagnostics
        .iter()
        .filter(|d| d.file.starts_with(dir) && archguard_search::is_elasticsearch_gap(d))
        .map(|d| d.message.clone())
        .collect();
    out.sort();
    out
}

#[test]
fn resources_become_artifacts_and_lookalikes_do_not() {
    let report = analyze();
    let mut got: Vec<String> = report
        .elasticsearch
        .artifacts
        .iter()
        .map(|a| {
            let name = a.name.as_deref().unwrap_or("?");
            format!("{} {name} [{}] {}", a.kind, a.flavor, a.scope)
        })
        .collect();
    got.sort();
    let want = [
        "alias orders-read [opensearch] terraform:os",
        "component_template logs-base [] terraform:es",
        "component_template orders-base [opensearch] terraform:os",
        "data_stream events-stream [opensearch] terraform:os",
        "data_stream logs-stream [] terraform:es",
        "ilm_policy logs-ilm [] terraform:es",
        "index ? [opensearch] terraform:os",
        "index dynamic-v1 [opensearch] terraform:os",
        "index logs-v1 [] terraform:es",
        "index orders-v1 [opensearch] terraform:os",
        "index_template legacy-logs [opensearch] terraform:os",
        "index_template logs-legacy [] terraform:es",
        "index_template logs-template [] terraform:es",
        "index_template orders-template [opensearch] terraform:os",
        "ingest_pipeline file-ingest [opensearch] terraform:os",
        "ingest_pipeline logs-ingest [] terraform:es",
        "ingest_pipeline orders-ingest [opensearch] terraform:os",
        "ism_policy hot-delete [opensearch] terraform:os",
        "ism_policy logs-ism [opensearch] terraform:es",
        "search_template orders-search [opensearch] terraform:os",
        "snapshot_lifecycle logs-snap [] terraform:es",
        "snapshot_lifecycle nightly-snap [opensearch] terraform:os",
        "stored_script calc-score [opensearch] terraform:os",
        "stored_script logs-score [] terraform:es",
        "watch logs-alarm [] terraform:es",
    ];
    assert_eq!(got, want);
}

#[test]
fn index_detail_aliases_and_references() {
    let report = analyze();
    let orders = artifact(&report, "orders-v1");
    let d = orders.detail.as_ref().expect("detail");
    assert_eq!(d.fields[0].path, "title");
    assert_eq!(d.fields[0].analyzer.as_deref(), Some("folding"));
    assert_eq!(d.analysis[0].name, "folding");
    for (key, value) in [
        ("number_of_shards", "2"),
        ("index_knn", "true"),
        ("rollover_alias", "orders-write"),
    ] {
        assert_eq!(
            d.settings.get(key).map(String::as_str),
            Some(value),
            "{key}"
        );
    }
    assert!(d.settings["index_similarity_default"].contains("BM25"));
    assert!(
        orders
            .references
            .iter()
            .any(|r| r.kind == "ingest_pipeline" && r.name == "orders-ingest")
    );
    let alias = artifact(&report, "orders-read");
    assert_eq!(alias.references[0].name, "orders-v1");
    let tpl = artifact(&report, "orders-template");
    assert_eq!(tpl.patterns, ["orders-*"]);
    assert_eq!(tpl.references[0].name, "orders-base");
    assert_eq!(tpl.detail.as_ref().expect("detail").fields[0].path, "sku");
    assert!(tpl.evidence[0].file.ends_with("os/main.tf"));
    assert!(tpl.evidence[0].line.is_some());
}

#[test]
fn unreadable_parts_are_gaps_and_unknown_providers_are_never_guessed() {
    let report = analyze();
    let os = gaps(&report, "os/");
    assert_eq!(os.len(), 3, "{os:?}");
    assert!(
        os.iter()
            .any(|g| g.contains("opensearch_index.dynamic") && g.ends_with("mappings."))
    );
    assert!(
        os.iter()
            .any(|g| g.contains("opensearch_ingest_pipeline.from_file") && g.ends_with("body."))
    );
    assert!(
        os.iter()
            .any(|g| g.contains("opensearch_index.named_by_local"))
    );
    let unknown = gaps(&report, "unknown/");
    assert_eq!(unknown.len(), 2, "{unknown:?}");
    assert!(unknown.iter().all(|g| g.contains("unknown provider")));
    assert!(gaps(&report, "es/").is_empty());
    assert!(gaps(&report, "lookalike/").is_empty());
    assert!(
        !report
            .elasticsearch
            .artifacts
            .iter()
            .any(|a| a.name.as_deref() == Some("no-provider-declared"))
    );
}
