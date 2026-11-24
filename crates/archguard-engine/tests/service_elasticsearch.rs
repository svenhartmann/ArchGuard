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
    assert_eq!(in_kibana("not a literal"), 3, "{:#?}", r.diagnostics);
    assert_eq!(
        in_kibana("alerting_rule.dynamic_type: Elasticsearch name is not a literal; the rule type"),
        1
    );
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

fn named<'a>(r: &'a ServiceTopology, address: &str) -> &'a SearchArtifact {
    r.elasticsearch
        .artifacts
        .iter()
        .find(|a| a.address == address)
        .unwrap_or_else(|| panic!("{address} missing: {:#?}", r.elasticsearch.artifacts))
}

#[test]
fn root_variable_default_names_the_artifact_with_its_source() {
    let r = fixture();
    let a = find(&r, "index", "orders-v3");
    assert_eq!(a.address, "elasticstack_elasticsearch_index.from_default");
    let d = &a.evidence[0].description;
    assert!(
        d.contains("default of variable root_index (vars.tf:"),
        "{d}"
    );
    assert_eq!(
        find(&r, "index_template", "vars-template").patterns,
        ["vars-a-*", "vars-b-*"]
    );
}

#[test]
fn literal_module_argument_beats_the_variable_default() {
    let r = fixture();
    let a = find(&r, "index", "orders-v2");
    assert_eq!(
        a.address,
        "module.literal_arg.elasticstack_elasticsearch_index.named"
    );
    let d = &a.evidence[0].description;
    assert!(
        d.contains("argument index_name of module.literal_arg (vars.tf:"),
        "{d}"
    );
    let t = named(
        &r,
        "module.literal_arg.elasticstack_elasticsearch_index_template.named",
    );
    assert_eq!(t.patterns, ["vars-a-*", "vars-b-*"]);
}

#[test]
fn module_argument_var_follows_one_hop_to_the_root_default() {
    let r = fixture();
    let a = find(&r, "index", "orders-v4");
    assert_eq!(
        a.address,
        "module.chained_arg.elasticstack_elasticsearch_index.named"
    );
    assert!(
        a.evidence[0]
            .description
            .contains("default of variable module_index")
    );
    let t = named(
        &r,
        "module.chained_arg.elasticstack_elasticsearch_index_template.named",
    );
    assert_eq!(t.patterns, ["chained-*"]);
}

#[test]
fn unset_module_argument_falls_back_to_the_variable_default() {
    let r = fixture();
    let a = find(&r, "index", "named-default");
    assert_eq!(
        a.address,
        "module.defaulted.elasticstack_elasticsearch_index.named"
    );
    assert!(
        a.evidence[0]
            .description
            .contains("default of variable index_name (modules/named/main.tf:")
    );
}

#[test]
fn unresolvable_variable_values_stay_gaps() {
    let r = fixture();
    assert!(!r.complete);
    let gaps = |file: &str| {
        r.diagnostics
            .iter()
            .filter(|d| d.file == file && d.message.contains("not a literal"))
            .map(|d| d.message.clone())
            .collect::<Vec<_>>()
    };
    let root = gaps("vars.tf");
    assert_eq!(root.len(), 3, "{root:#?}");
    for needle in ["index.secret", "index.unset", "index.from_local"] {
        assert!(
            root.iter().any(|m| m.contains(needle)),
            "{needle}: {root:#?}"
        );
    }
    let module = gaps("modules/named/main.tf");
    assert_eq!(module.len(), 3, "{module:#?}");
    assert!(
        module
            .iter()
            .any(|m| m.contains("module.number_arg") && m.contains("index.named"))
    );
    assert!(
        module
            .iter()
            .any(|m| m.contains("module.defaulted") && m.contains("index_template"))
    );
    assert!(
        module
            .iter()
            .any(|m| m.contains("module.dynamic_arg") && m.contains("index.named"))
    );
    assert!(
        named(
            &r,
            "module.dynamic_arg.elasticstack_elasticsearch_index.named"
        )
        .name
        .is_none()
    );
}

#[test]
fn sensitive_defaults_locals_and_tfvars_are_never_read() {
    let r = fixture();
    assert!(
        named(&r, "elasticstack_elasticsearch_index.secret")
            .name
            .is_none()
    );
    let all = format!("{:#?}{:#?}", r.elasticsearch, r.diagnostics);
    for forbidden in [
        "secret-must-not-appear",
        "local-must-not-appear",
        "tfvars-must-not-appear",
    ] {
        assert!(!all.contains(forbidden), "{forbidden} leaked");
    }
}

#[test]
fn numbers_bools_and_non_string_lists_are_gaps_not_names() {
    let r = fixture();
    let gaps: Vec<_> = r
        .diagnostics
        .iter()
        .filter(|d| d.file == "types.tf" && d.message.contains("not a literal"))
        .collect();
    assert_eq!(gaps.len(), 3, "{gaps:#?}");
    for needle in ["index.number", "index.bool", "index_template.numbers"] {
        assert!(gaps.iter().any(|d| d.message.contains(needle)), "{needle}");
    }
    let all = format!("{:?}", r.elasticsearch.artifacts);
    for bad in ["\"43\"", "\"true\"", "\"1\""] {
        assert!(!all.contains(bad), "{bad} read as a name");
    }
}

fn templates() -> ServiceTopology {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/elasticsearch-services/searchtemplates");
    analyze_services(&root).unwrap()
}

#[test]
fn json_files_with_lang_mustache_become_search_templates_named_by_file() {
    let r = templates();
    let found: Vec<_> = r
        .elasticsearch
        .artifacts
        .iter()
        .filter(|a| a.kind == "search_template")
        .map(|a| {
            (
                a.name.clone().unwrap(),
                a.evidence[0].file.clone(),
                a.evidence[0].line,
            )
        })
        .collect();
    // strict.json, and the mustache-templated (not strictly parseable) templated.json;
    // not painless.json, note.json, .terraform/cached.json or docs/outside.json (no .tf above).
    assert_eq!(
        found,
        vec![
            ("strict".to_owned(), "infra/strict.json".to_owned(), Some(3)),
            (
                "templated".to_owned(),
                "infra/sub/templated.json".to_owned(),
                Some(3)
            ),
        ],
        "{:#?}",
        r.elasticsearch.artifacts
    );
    let a = find(&r, "search_template", "strict");
    assert!(
        a.evidence[0]
            .description
            .contains("name assumed from file name")
    );
    assert!(r.diagnostics.is_empty(), "{:#?}", r.diagnostics);
}

fn big_json(head: &str) -> ServiceTopology {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.tf"), "").unwrap();
    let text = format!("{head}{}", " ".repeat(1024 * 1024 + 1));
    std::fs::write(dir.path().join("big.json"), text).unwrap();
    analyze_services(dir.path()).unwrap()
}

#[test]
fn large_json_without_the_pattern_is_no_diagnostic() {
    let r = big_json("{}");
    assert!(
        r.complete && r.diagnostics.is_empty(),
        "{:#?}",
        r.diagnostics
    );
}

#[test]
fn large_json_with_the_pattern_is_a_diagnostic_not_silence() {
    let r = big_json(r#"{"lang": "mustache"}"#);
    assert!(!r.complete);
    assert!(
        r.diagnostics.iter().any(|d| d.file == "big.json"),
        "{:#?}",
        r.diagnostics
    );
}
