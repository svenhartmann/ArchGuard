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
        "index items python:djangodoc [] djangodoc/items.py:ItemDocument",
        "index base-docs python:inherit [] inherit/base.py:BaseDoc",
        "index base-docs python:inherit [] inherit/documents.py:Child",
        "index ? python:inherit [] inherit/local.py:Bare",
        "index mid python:inherit [] inherit/local.py:Leaf",
        "index mid python:inherit [] inherit/local.py:Mid",
        "index ? python:inherit [] inherit/local.py:NoName",
        "index imported-index python:inherit [] inherit/named.py:Imported",
        "index ? python:shop [] shop/dynamic.py:Nameless",
        "index partial python:shop [] shop/dynamic.py:Partial",
        "index orders-v1 python:shop [] shop/orders.py:Order",
        "index events python:shop [opensearch] shop/others.py:Event",
        "index notes python:shop [] shop/others.py:Note",
        "index products python:shop [] shop/products.py:ProductDocument",
        "index opaque python:typed [] typed/rhs.py:Opaque",
        "index positional python:typed [] typed/rhs.py:Positional",
        "index rhs python:typed [] typed/rhs.py:Rhs",
        "index subclassed python:typed [] typed/rhs.py:Subclassed",
        "index typed python:typed [] typed/typed.py:Typed",
        "index invalid python:typed [] typed/untyped.py:Invalid",
        "index untyped python:typed [] typed/untyped.py:Untyped",
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

fn by_address<'a>(report: &'a ServiceTopology, address: &str) -> &'a SearchArtifact {
    (report.elasticsearch.artifacts.iter())
        .find(|a| a.address == address)
        .unwrap_or_else(|| panic!("no artifact {address}"))
}

fn fields_of(a: &SearchArtifact) -> Vec<String> {
    let d = a.detail.as_ref().expect("detail");
    (d.fields.iter())
        .map(|f| format!("{}:{}", f.path, f.kind))
        .collect()
}

#[test]
fn local_base_documents_pass_index_and_fields_down_and_unresolved_bases_are_gaps() {
    let report = analyze();
    let child = by_address(&report, "inherit/documents.py:Child");
    assert_eq!(fields_of(child), ["code:keyword", "title:text"]);
    assert_eq!(
        child.detail.as_ref().unwrap().settings["number_of_shards"],
        "1"
    );
    let leaf = by_address(&report, "inherit/local.py:Leaf");
    assert_eq!(fields_of(leaf), ["kind:keyword"]);
    let gaps = gaps(&report);
    assert!(
        gaps.iter().any(|g| g.starts_with("inherit/documents.py: ")
            && g.contains("Lost")
            && g.contains("Thing")),
        "{gaps:#?}"
    );
}

#[test]
fn documents_without_a_name_are_artifacts_without_name_and_a_gap() {
    let report = analyze();
    let gaps = gaps(&report);
    for class in ["Bare", "NoName"] {
        let a = by_address(&report, &format!("inherit/local.py:{class}"));
        assert_eq!(a.name, None);
    }
    let n = gaps
        .iter()
        .filter(|g| g.starts_with("inherit/local.py: Index name is not a literal"))
        .count();
    assert_eq!(n, 2, "{gaps:#?}");
}

#[test]
fn index_name_constants_resolve_across_files() {
    let report = analyze();
    assert_eq!(
        by_address(&report, "inherit/named.py:Imported")
            .name
            .as_deref(),
        Some("imported-index")
    );
}

#[test]
fn typed_fields_async_documents_and_numpy_vectors() {
    let report = analyze();
    let t = by_address(&report, "typed/typed.py:Typed");
    assert_eq!(
        fields_of(t),
        [
            "home:object",
            "home.city:text",
            "homes:nested",
            "homes.city:text",
            "maybe:text",
            "note:text",
            "seen:date",
            "title:text",
            "vec:dense_vector"
        ]
    );
    let u = by_address(&report, "typed/untyped.py:Untyped");
    assert_eq!(fields_of(u), ["count:integer"]);
    // A Field class is no annotation type: the DSL raises TypeError, nothing is guessed.
    let bad = by_address(&report, "typed/untyped.py:Invalid");
    assert!(bad.detail.is_none());
    assert!(gaps(&report).iter().any(
        |g| g.starts_with("typed/untyped.py: typed/untyped.py:Invalid")
            && g.ends_with("not read: mappings.")
    ));
}

#[test]
fn a_right_hand_side_wins_over_the_annotation() {
    let report = analyze();
    // keyword from `field=`, `exclude=True` skipped, keyword from the local subclass, the
    // annotation only without an assignment.
    let r = by_address(&report, "typed/rhs.py:Rhs");
    assert_eq!(
        fields_of(r),
        ["a:keyword", "c:keyword", "plain:integer", "renamed:keyword"]
    );
    // A subclass call with arguments would lose them: a gap, not a bare keyword.
    assert!(
        by_address(&report, "typed/rhs.py:Subclassed")
            .detail
            .is_none()
    );
    assert!(
        gaps(&report)
            .iter()
            .any(|g| g.starts_with("typed/rhs.py: typed/rhs.py:Subclassed"))
    );
    // A positional string is no field and no name (`es_name` is keyword-only): a gap.
    assert!(
        by_address(&report, "typed/rhs.py:Positional")
            .detail
            .is_none()
    );
    assert!(
        gaps(&report)
            .iter()
            .any(|g| g.starts_with("typed/rhs.py: typed/rhs.py:Positional"))
    );
    // An assigned call that is no field class is a gap, never the annotation type.
    let o = by_address(&report, "typed/rhs.py:Opaque");
    assert!(o.detail.is_none());
    assert!(
        gaps(&report)
            .iter()
            .any(|g| g.starts_with("typed/rhs.py: typed/rhs.py:Opaque")
                && g.ends_with("not read: mappings."))
    );
}

#[test]
fn django_model_fields_are_a_gap() {
    let report = analyze();
    assert!(
        gaps(&report)
            .iter()
            .any(|g| g.starts_with("djangodoc/items.py: ")
                && g.contains("djangodoc/items.py:ItemDocument")
                && g.contains("Django.fields")),
    );
}

#[test]
fn non_literal_parts_are_gaps_that_name_the_attribute() {
    let report = analyze();
    let gaps = gaps(&report);
    assert_eq!(gaps.len(), 10, "{gaps:#?}");
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
