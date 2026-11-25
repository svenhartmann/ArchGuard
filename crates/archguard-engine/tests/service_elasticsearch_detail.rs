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

fn templates() -> ServiceTopology {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/es-index-detail/templates/terraform");
    analyze_services(&root).unwrap()
}

fn art<'a>(r: &'a ServiceTopology, kind: &str, name: &str) -> &'a SearchArtifact {
    r.elasticsearch
        .artifacts
        .iter()
        .find(|a| a.kind == kind && a.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("{kind} {name} missing"))
}

fn refs(a: &SearchArtifact) -> Vec<String> {
    a.references
        .iter()
        .map(|x| format!("{}/{}", x.kind, x.name))
        .collect()
}

#[test]
fn index_template_reads_mappings_settings_analysis_and_ilm() {
    let r = templates();
    let a = art(&r, "index_template", "logs");
    assert_eq!(fields(a), ["msg:text"]);
    let d = a.detail.as_ref().unwrap();
    let settings: Vec<_> = d.settings.iter().map(|(k, v)| format!("{k}:{v}")).collect();
    assert_eq!(
        settings,
        [
            "lifecycle.name:hot-warm",
            "number_of_shards:2",
            "refresh_interval:5s"
        ]
    );
    let analysis: Vec<_> = d
        .analysis
        .iter()
        .map(|x| format!("{}/{}", x.kind, x.name))
        .collect();
    assert_eq!(analysis, ["analyzer/a1"]);
    assert!(refs(a).contains(&"ilm_policy/hot-warm".to_string()));
}

#[test]
fn component_template_reads_literal_json_string() {
    let r = templates();
    assert_eq!(fields(art(&r, "component_template", "base")), ["ts:date"]);
}

#[test]
fn legacy_settings_block_gives_setting_and_ilm() {
    let r = templates();
    let a = art(&r, "index", "legacy");
    assert_eq!(a.detail.as_ref().unwrap().settings["lifecycle.name"], "p9");
    assert!(refs(a).contains(&"ilm_policy/p9".to_string()));
}

#[test]
fn watch_indices_are_references_and_patterns() {
    let r = templates();
    let w = art(&r, "watch", "w1");
    assert_eq!(refs(w), ["index/orders"]);
    assert_eq!(w.patterns, ["logs-*"]);
    assert!(refs(art(&r, "watch", "w3")).is_empty());
    assert!(refs(art(&r, "watch", "w2")).is_empty());
}

#[test]
fn non_literal_template_and_watch_are_one_gap_each() {
    let r = templates();
    let gaps: Vec<_> = r
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("not read:"))
        .map(|d| d.message.as_str())
        .collect();
    assert_eq!(gaps.len(), 2, "{gaps:#?}");
    assert!(
        gaps.iter()
            .any(|m| m.contains("index_template.dyn") && m.contains("mappings"))
    );
    assert!(
        gaps.iter()
            .any(|m| m.contains("watch.dynamic") && m.contains("input"))
    );
}

#[test]
fn ilm_reference_is_an_edge_to_a_ghost_without_finding() {
    let r = templates();
    let g = archguard_engine::search_graph(&r);
    let ghost = g
        .nodes
        .iter()
        .position(|n| n.kind == "ilm_policy" && n.label == "hot-warm")
        .expect("ghost");
    assert!(!g.nodes[ghost].declared && !g.nodes[ghost].warn);
    assert!(
        g.edges
            .iter()
            .any(|e| e.to == ghost && e.relation == "reference")
    );
}

fn locals() -> ServiceTopology {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/es-index-detail/locals/terraform");
    analyze_services(&root).unwrap()
}

fn setting(a: &SearchArtifact, key: &str) -> Option<String> {
    a.detail.as_ref()?.settings.get(key).cloned()
}

#[test]
fn locals_and_sibling_resources_of_one_module_instance_are_read() {
    let r = locals();
    let a = art(&r, "index", "via-body");
    assert_eq!(fields(a), ["a:keyword"]);
    assert_eq!(setting(a, "number_of_shards").as_deref(), Some("3"));
    assert_eq!(fields(art(&r, "index", "text-local")), ["b:text"]);
    assert_eq!(fields(art(&r, "index", "decoded")), ["b:text"]);
    let b = art(&r, "index", "second");
    assert_eq!(setting(b, "refresh_interval").as_deref(), Some("7s"));
    assert_eq!(b.detail.as_ref().unwrap().analysis[0].name, "a1");
    // the child module reads its own local
    assert_eq!(fields(art(&r, "index", "inner")), ["c:date"]);
}

#[test]
fn unresolvable_references_are_one_gap_with_the_attribute_names() {
    let r = locals();
    let gaps: Vec<_> = r
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("not read:"))
        .map(|d| d.message.as_str())
        .collect();
    for (address, attr) in [
        ("second_level", "mappings"),
        ("each", "mappings"),
        ("other_module", "refresh_interval"),
        ("lookalike", "mappings"),
    ] {
        let hits: Vec<_> = gaps.iter().filter(|m| m.contains(address)).collect();
        assert_eq!(hits.len(), 1, "{address}: {gaps:#?}");
        assert!(hits[0].contains(attr), "{address}: {}", hits[0]);
    }
    assert_eq!(gaps.len(), 4, "{gaps:#?}");
}

#[test]
fn names_still_do_not_read_locals() {
    let r = locals();
    assert!(
        r.elasticsearch
            .artifacts
            .iter()
            .all(|a| a.name.as_deref() != Some("named"))
    );
    assert!(
        r.diagnostics
            .iter()
            .any(|d| d.message.contains("name_local"))
    );
}
