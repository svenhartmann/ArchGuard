//! Elasticsearch artifacts declared in Terraform (ADR 0023).
use archguard_core::{SearchArtifact, ServiceTopology};
use archguard_engine::analyze_services;
use std::path::Path;

fn fixture() -> ServiceTopology {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/elasticsearch-services/terraform");
    analyze_services(&root).unwrap()
}

fn find<'a>(r: &'a ServiceTopology, kind: &str, name: &str) -> &'a SearchArtifact {
    r.elasticsearch
        .artifacts
        .iter()
        .find(|a| a.kind == kind && a.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("{kind} {name} missing: {:#?}", r.elasticsearch.artifacts))
}

fn refs(a: &SearchArtifact) -> Vec<String> {
    a.references
        .iter()
        .map(|r| format!("{}:{}", r.kind, r.name))
        .collect()
}

#[test]
fn terraform_resources_become_artifacts_with_references_and_evidence() {
    let r = fixture();
    let index = find(&r, "index", "orders-v1");
    assert_eq!(index.address, "elasticstack_elasticsearch_index.orders");
    assert_eq!(index.scope, "terraform:.");
    assert_eq!(
        refs(index),
        [
            "ingest_pipeline:orders-ingest",
            "ingest_pipeline:orders-final"
        ]
    );
    let line = index.evidence[0].line.unwrap();
    assert!(index.evidence[0].description.contains(&format!(
        "Declared in Terraform resource elasticstack_elasticsearch_index.orders (main.tf:{line})"
    )));
    assert_eq!(refs(find(&r, "alias", "orders")), ["index:orders-v1"]);
    assert_eq!(
        refs(find(&r, "alias", "orders-all")),
        ["component_template:orders-base"]
    );
    let template = find(&r, "index_template", "orders-template");
    assert_eq!(template.patterns, ["orders-*", "orders-archive-*"]);
    assert_eq!(refs(template), ["component_template:orders-base"]);
    assert_eq!(
        refs(find(&r, "ingest_pipeline", "orders-ingest")),
        ["ingest_pipeline:orders-enrich"]
    );
    assert_eq!(
        refs(find(&r, "transform", "orders-rollup")),
        [
            "index:orders-v1",
            "index:orders-summary",
            "ingest_pipeline:orders-ingest"
        ]
    );
    assert_eq!(
        refs(find(&r, "enrich_policy", "orders-enrich")),
        ["index:orders-v1"]
    );
    assert_eq!(
        refs(find(&r, "snapshot_lifecycle", "orders-nightly")),
        ["index:orders-*"]
    );
    find(&r, "watch", "orders-watch");
    find(&r, "ilm_policy", "orders-ilm");
    find(&r, "component_template", "orders-base");
    find(&r, "stored_script", "orders-score");
    find(&r, "search_template", "orders-search");
}

#[test]
fn module_resources_carry_the_module_address() {
    let r = fixture();
    let inner = find(&r, "ilm_policy", "inner-ilm");
    assert_eq!(
        inner.address,
        "module.search.elasticstack_elasticsearch_index_lifecycle.inner"
    );
    assert_eq!(inner.scope, "terraform:.");
}

#[test]
fn non_literal_names_are_gaps_and_terraform_cache_is_never_read() {
    let r = fixture();
    assert!(!r.complete);
    let gaps = r
        .diagnostics
        .iter()
        .filter(|d| {
            d.file == "main.tf" && d.message.contains("Elasticsearch name is not a literal")
        })
        .count();
    assert_eq!(gaps, 2, "{:#?}", r.diagnostics);
    assert!(r.elasticsearch.artifacts.iter().any(|a| a.name.is_none()));
    assert!(
        r.elasticsearch
            .artifacts
            .iter()
            .all(|a| a.name.as_deref() != Some("must-not-appear"))
    );
}

#[test]
fn on_failure_processors_are_read_besides_processors() {
    let r = fixture();
    assert_eq!(
        refs(find(&r, "ingest_pipeline", "orders-failing")),
        ["ingest_pipeline:orders-fallback"]
    );
}

#[test]
fn unreadable_processor_lists_are_gaps() {
    let r = fixture();
    let gaps = r
        .diagnostics
        .iter()
        .filter(|d| d.file == "pipelines.tf" && d.message.contains("not a literal"))
        .count();
    assert_eq!(gaps, 2, "{:#?}", r.diagnostics);
}

#[test]
fn kibana_data_views_read_title_patterns() {
    let r = fixture();
    let v = find(&r, "data_view", "orders-view");
    assert_eq!(v.address, "elasticstack_kibana_data_view.orders");
    assert_eq!(v.patterns, ["orders-*", "orders-archive-*"]);
    assert!(find(&r, "data_view", "logs-view").patterns.is_empty());
}

#[test]
fn kibana_alerting_rules_read_params_index() {
    let r = fixture();
    let q = find(&r, "alerting_rule", "orders-alert");
    assert_eq!(q.patterns, ["orders-*", "orders-archive-*"]);
    assert!(q.evidence[0].description.contains(".es-query"));
    let j = find(&r, "alerting_rule", "orders-json-alert");
    assert_eq!(j.patterns, ["orders-logs-*"]);
    assert!(
        j.evidence[0]
            .description
            .contains("logs.alert.document.count")
    );
    assert!(
        find(&r, "alerting_rule", "orders-dynamic-alert")
            .patterns
            .is_empty()
    );
}

#[test]
fn kibana_gaps_and_saved_objects_note() {
    let r = fixture();
    let in_kibana = |needle: &str| {
        r.diagnostics
            .iter()
            .filter(|d| d.file == "kibana.tf" && d.message.contains(needle))
            .count()
    };
    assert_eq!(in_kibana("not a literal"), 2, "{:#?}", r.diagnostics);
    assert_eq!(in_kibana("saved objects are not analyzed"), 1);
    assert!(
        r.diagnostics
            .iter()
            .filter(|d| d.message.contains("saved objects are not analyzed"))
            .all(|d| d
                .message
                .contains("elasticstack_kibana_import_saved_objects.dashboards"))
    );
    assert!(
        r.elasticsearch
            .artifacts
            .iter()
            .all(|a| !a.address.contains("import_saved_objects"))
    );
}

#[test]
fn nested_search_configuration_index_is_a_data_view_id_not_a_pattern() {
    let r = fixture();
    assert!(
        find(&r, "alerting_rule", "orders-source-alert")
            .patterns
            .is_empty()
    );
    assert!(r.diagnostics.iter().any(|d| {
        d.file == "kibana.tf"
            && d.message
                .contains("elasticstack_kibana_alerting_rule.search_source")
            && d.message.contains("data view")
    }));
}

#[test]
fn rule_types_without_a_params_index_are_no_gap() {
    let r = fixture();
    assert!(
        find(&r, "alerting_rule", "orders-anomaly-alert")
            .patterns
            .is_empty()
    );
    assert!(!r.diagnostics.iter().any(|d| {
        d.message
            .contains("elasticstack_kibana_alerting_rule.anomaly")
    }));
}
