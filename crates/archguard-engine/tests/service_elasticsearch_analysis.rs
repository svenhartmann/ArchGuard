//! Analysis parts of an index and its templates (ADR 0023, es-classdiagram ticket 01).
use archguard_core::{SearchArtifact, ServiceTopology};
use archguard_engine::{SearchAnalysisPart, analyze_services, search_analysis_parts};
use std::path::Path;

fn fixture() -> ServiceTopology {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/es-classdiagram/01/terraform");
    analyze_services(&root).unwrap()
}

fn artifact<'a>(r: &'a ServiceTopology, kind: &str, name: &str) -> &'a SearchArtifact {
    r.elasticsearch
        .artifacts
        .iter()
        .find(|a| a.kind == kind && a.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("{kind} {name} missing"))
}

fn parts(r: &ServiceTopology, kind: &str, name: &str) -> Vec<SearchAnalysisPart> {
    search_analysis_parts(artifact(r, kind, name).detail.as_ref().unwrap())
}

fn part<'a>(p: &'a [SearchAnalysisPart], kind: &str, name: &str) -> &'a SearchAnalysisPart {
    p.iter()
        .find(|x| x.kind == kind && x.name == name)
        .unwrap_or_else(|| panic!("{kind} {name} missing"))
}

fn chain(refs: &[archguard_engine::ChainRef]) -> Vec<String> {
    refs.iter()
        .map(|c| format!("{}:{}", c.name, c.class))
        .collect()
}

#[test]
fn all_five_kinds_are_read_for_index_and_templates() {
    let r = fixture();
    let kinds = |k: &str, n: &str| {
        let mut v: Vec<_> = artifact(&r, k, n)
            .detail
            .as_ref()
            .unwrap()
            .analysis
            .iter()
            .map(|a| a.kind.clone())
            .collect();
        v.sort();
        v.dedup();
        v
    };
    let all = [
        "analyzer",
        "char_filter",
        "filter",
        "normalizer",
        "tokenizer",
    ];
    assert_eq!(kinds("index", "products"), all);
    assert_eq!(kinds("index_template", "logs"), all);
    assert_eq!(kinds("component_template", "base"), all);
}

#[test]
fn parts_of_the_fixture_index() {
    let r = fixture();
    let p = parts(&r, "index", "products");
    assert_eq!(p.len(), 8);
    let a = part(&p, "analyzer", "title_fold");
    assert_eq!(a.kind_type.as_deref(), Some("custom"));
    assert_eq!(
        chain(&a.char_filters),
        ["html_strip:builtin", "dash_map:declared"]
    );
    assert_eq!(
        a.tokenizer
            .as_ref()
            .map(|t| format!("{}:{}", t.name, t.class))
            .as_deref(),
        Some("standard:builtin")
    );
    assert_eq!(
        chain(&a.filters),
        [
            "lowercase:builtin",
            "my_stop:declared",
            "icu_folding:undeclared",
            "syn:declared"
        ]
    );
    assert!(a.params.is_empty());
    let t = part(&p, "tokenizer", "prefix_tok");
    assert_eq!(t.kind_type.as_deref(), Some("edge_ngram"));
    assert_eq!(
        t.params,
        [
            ("max_gram".to_string(), "10".to_string()),
            ("min_gram".to_string(), "2".to_string()),
            ("token_chars".to_string(), "letter, digit".to_string())
        ]
    );
    let pre = part(&p, "analyzer", "prefix_search");
    assert_eq!(pre.kind_type.as_deref(), Some("custom"));
    assert_eq!(pre.tokenizer.as_ref().unwrap().class, "declared");
    assert_eq!(chain(&pre.filters), ["length:inline"]);
    let n = part(&p, "normalizer", "lower_fold");
    assert_eq!(
        chain(&n.filters),
        ["lowercase:builtin", "asciifolding:builtin"]
    );
    assert_eq!(
        part(&p, "filter", "syn").params[0],
        ("synonyms_path".into(), "analysis/synonyms.txt".into())
    );
    assert_eq!(
        part(&p, "char_filter", "dash_map").kind_type.as_deref(),
        Some("mapping")
    );
}

#[test]
fn chains_of_templates_resolve_in_their_own_detail() {
    let r = fixture();
    let p = parts(&r, "component_template", "base");
    assert_eq!(
        chain(&part(&p, "normalizer", "base_n").filters),
        ["base_f:declared"]
    );
    let p = parts(&r, "index_template", "logs");
    assert_eq!(
        part(&p, "analyzer", "log_a")
            .tokenizer
            .as_ref()
            .unwrap()
            .class,
        "declared"
    );
}
