//! Literal mapping, settings and analysis of an index (ADR 0023, ticket 37).
use archguard_core::{SearchArtifact, ServiceTopology};
use archguard_engine::{analyze_services, is_elasticsearch_gap};
use std::path::Path;

fn fixture() -> ServiceTopology {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/es-index-detail/literal/terraform");
    analyze_services(&root).unwrap()
}

fn index<'a>(r: &'a ServiceTopology, name: &str) -> &'a SearchArtifact {
    r.elasticsearch
        .artifacts
        .iter()
        .find(|a| a.kind == "index" && a.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("index {name} missing"))
}

fn fields(a: &SearchArtifact) -> Vec<String> {
    a.detail
        .as_ref()
        .unwrap()
        .fields
        .iter()
        .map(|f| {
            let extra = f
                .analyzer
                .as_deref()
                .map(|x| format!("(analyzer {x})"))
                .or_else(|| f.normalizer.as_deref().map(|x| format!("(normalizer {x})")))
                .unwrap_or_default();
            format!("{}:{}{extra}", f.path, f.kind)
        })
        .collect()
}

#[test]
fn literal_attributes_become_fields_settings_and_analysis() {
    let r = fixture();
    let d = index(&r, "orders").detail.as_ref().unwrap();
    assert_eq!(
        fields(index(&r, "orders")),
        [
            "id:keyword",
            "title:text(analyzer german_custom)",
            "title.raw:keyword(normalizer lower)",
            "customer:object",
            "customer.name:text",
            "lines:nested",
            "lines.sku:keyword"
        ]
    );
    let settings: Vec<_> = d.settings.iter().map(|(k, v)| format!("{k}={v}")).collect();
    assert_eq!(
        settings,
        [
            "default_pipeline=p1",
            "number_of_replicas=2",
            "number_of_shards=1",
            "refresh_interval=30s"
        ]
    );
    let analysis: Vec<_> = d
        .analysis
        .iter()
        .map(|a| format!("{}/{}/{}", a.kind, a.name, a.definition))
        .collect();
    assert_eq!(
        analysis,
        [
            r#"analyzer/german_custom/{"type":"custom","tokenizer":"standard"}"#,
            r#"normalizer/lower/{"type":"custom","filter":["lowercase"]}"#
        ]
    );
}

#[test]
fn heredoc_json_is_read() {
    assert_eq!(fields(index(&fixture(), "heredoc")), ["ts:date"]);
}

#[test]
fn non_literal_attributes_are_one_gap_and_the_rest_is_read() {
    let r = fixture();
    let c = index(&r, "computed");
    let d = c.detail.as_ref().unwrap();
    assert!(d.fields.is_empty());
    assert_eq!(d.settings.len(), 1);
    assert_eq!(d.settings["refresh_interval"], "1s");
    let gaps: Vec<_> = r
        .diagnostics
        .iter()
        .filter(|d| {
            d.message
                .contains("elasticstack_elasticsearch_index.computed")
        })
        .filter(|d| is_elasticsearch_gap(d))
        .filter(|d| d.message.contains("mappings, number_of_shards"))
        .collect();
    assert_eq!(gaps.len(), 1, "{:#?}", r.diagnostics);
    assert!(!r.complete);
}

#[test]
fn plain_index_has_no_detail_and_lookalikes_are_ignored() {
    let r = fixture();
    assert!(index(&r, "plain").detail.is_none());
    assert!(index(&r, "aliased").detail.is_none());
    let detail_gaps = r
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("not read:"))
        .count();
    assert_eq!(detail_gaps, 1, "{:#?}", r.diagnostics);
    assert!(
        r.elasticsearch
            .artifacts
            .iter()
            .filter(|a| a.kind != "index")
            .all(|a| a.detail.is_none())
    );
}

#[test]
fn detail_gap_is_recognized_as_elasticsearch_gap() {
    let r = fixture();
    assert!(
        r.diagnostics
            .iter()
            .any(|d| d.message.contains("not read:") && is_elasticsearch_gap(d))
    );
}
