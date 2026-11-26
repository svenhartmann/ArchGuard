//! Python elasticsearch-dsl, opensearch-dsl and django-elasticsearch-dsl documents become index
//! artifacts (ticket 24).
use archguard_core::{SearchArtifact, ServiceTopology};
use archguard_flow_common::support::Discovery;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/es-classdiagram/24")
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

fn gaps(report: &ServiceTopology) -> Vec<String> {
    report
        .diagnostics
        .iter()
        .filter(|d| archguard_search::is_elasticsearch_gap(d))
        .map(|d| format!("{}: {}", d.file, d.message))
        .collect()
}

#[test]
fn documents_of_every_import_path_become_artifacts_and_lookalikes_do_not() {
    let report = analyze();
    let got: Vec<String> = report
        .elasticsearch
        .artifacts
        .iter()
        .map(|a| {
            format!(
                "{} {} {} [{}] {}",
                a.kind,
                a.name.as_deref().unwrap_or("?"),
                a.scope,
                a.flavor,
                a.address
            )
        })
        .collect();
    let want = [
        "index ? python:shop [] shop/dynamic.py:Nameless",
        "index partial python:shop [] shop/dynamic.py:Partial",
        "index orders-v1 python:shop [] shop/orders.py:Order",
        "index events python:shop [opensearch] shop/others.py:Event",
        "index notes python:shop [] shop/others.py:Note",
        "index products python:shop [] shop/products.py:ProductDocument",
    ];
    assert_eq!(got, want, "own `Document` classes must not match");
    // The unqualified `Text` of another library is no field either.
    assert!(
        report
            .elasticsearch
            .artifacts
            .iter()
            .all(|a| a.name.as_deref() != Some("fake-index"))
    );
}

#[test]
fn fields_multi_fields_analysis_settings_aliases_and_evidence() {
    let report = analyze();
    let a = artifact(&report, "orders-v1");
    let d = a.detail.as_ref().expect("detail");
    let fields: Vec<_> = d
        .fields
        .iter()
        .map(|f| {
            format!(
                "{}:{}:{}:{}:{}",
                f.path,
                f.kind,
                f.analyzer.as_deref().unwrap_or(""),
                f.search_analyzer.as_deref().unwrap_or(""),
                f.normalizer.as_deref().unwrap_or("")
            )
        })
        .collect();
    assert_eq!(
        fields,
        [
            "customer:object:::",
            "customer.name:text:::",
            "lines:nested:::",
            "lines.qty:integer:::",
            "lines.sku:keyword:::lower_norm",
            "placed:date:::",
            "title:text:title_an:standard:",
            "title.raw:keyword:::",
        ]
    );
    assert_eq!(d.settings["number_of_shards"], "2");
    let analysis: Vec<_> = d
        .analysis
        .iter()
        .map(|x| format!("{} {} {}", x.kind, x.name, x.definition))
        .collect();
    assert_eq!(
        analysis,
        [
            r#"analyzer title_an {"char_filter":["strip_html"],"filter":["lowercase","fold_ascii"],"tokenizer":"tri","type":"custom"}"#,
            r#"char_filter strip_html {"type":"html_strip"}"#,
            r#"filter fold_ascii {"preserve_original":true,"type":"asciifolding"}"#,
            r#"normalizer lower_norm {"filter":["lowercase"],"type":"custom"}"#,
            r#"tokenizer tri {"max_gram":3,"min_gram":3,"type":"ngram"}"#,
        ]
    );
    assert_eq!(a.references.len(), 1);
    assert_eq!(
        (a.references[0].kind.as_str(), a.references[0].name.as_str()),
        ("alias", "orders")
    );
    assert_eq!(a.evidence[0].file, "shop/orders.py");
    assert_eq!(a.evidence[0].line, Some(33));
}

#[test]
fn django_fields_and_multi_fields() {
    let report = analyze();
    let d = artifact(&report, "products")
        .detail
        .as_ref()
        .expect("detail");
    let fields: Vec<_> = d
        .fields
        .iter()
        .map(|f| format!("{}:{}", f.path, f.kind))
        .collect();
    assert_eq!(fields, ["name:text", "name.raw:keyword", "price:float"]);
    assert_eq!(d.settings["number_of_replicas"], "0");
}

#[test]
fn non_literal_parts_are_gaps_that_name_the_attribute() {
    let report = analyze();
    let gaps = gaps(&report);
    assert_eq!(gaps.len(), 2, "{gaps:#?}");
    assert!(gaps.iter().any(
        |g| g.starts_with("shop/dynamic.py: shop/dynamic.py:Partial")
            && g.ends_with("not read: mappings, settings.")
    ));
    assert!(
        gaps.iter()
            .any(|g| g.starts_with("shop/dynamic.py: Index name is not a literal"))
    );
    assert!(!report.complete);
    let partial = artifact(&report, "partial");
    assert!(partial.detail.is_none(), "all or nothing per attribute");
}
