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

/// Corpus probe (ticket 13): counts only, no names. See spec.md `## Abnahme-Probe`.
#[test]
#[ignore = "reads the corpus named by ARCHGUARD_ES_PROBE"]
fn probe() {
    use archguard_engine::{
        SearchFieldRole, search_analysis_users, search_analyzer_difference, search_field_analyzers,
        search_field_role,
    };
    use std::collections::{BTreeMap, BTreeSet};
    let Ok(root) = std::env::var("ARCHGUARD_ES_PROBE") else {
        return;
    };
    let r = analyze_services(Path::new(&root)).unwrap();
    let arts = &r.elasticsearch.artifacts;
    println!(
        "artifacts: {}; usages: {}; index blocks: {}",
        arts.len(),
        r.elasticsearch.usages.len(),
        arts.iter().filter(|a| a.kind == "index").count()
    );
    // One entry per resource block (file and line), not per module instance.
    let mut seen = BTreeSet::new();
    let blocks: Vec<_> = arts
        .iter()
        .filter(|a| {
            let e = a.evidence.first();
            seen.insert((
                e.map(|e| e.file.clone()),
                e.and_then(|e| e.line),
                a.kind.clone(),
            ))
        })
        .collect();
    println!(
        "index blocks (deduplicated): {}",
        blocks.iter().filter(|a| a.kind == "index").count()
    );
    let mut declared: BTreeMap<String, usize> = BTreeMap::new();
    let mut classes: BTreeMap<&str, usize> = BTreeMap::new();
    let (mut analyzer, mut search, mut normalizer, mut multi, mut object, mut diff) =
        (0, 0, 0, 0, 0, 0);
    let (mut no_user, mut total, mut distinct) = (0, 0, BTreeSet::new());
    for a in blocks {
        let Some(d) = a.detail.as_ref() else { continue };
        let ps = search_analysis_parts(d);
        for p in &ps {
            *declared.entry(p.kind.clone()).or_default() += 1;
            total += 1;
            distinct.insert((p.kind.clone(), p.name.clone(), p.definition.clone()));
            let refs = p.char_filters.iter().chain(&p.tokenizer).chain(&p.filters);
            for c in refs {
                *classes.entry(c.class).or_default() += 1;
            }
        }
        for u in search_analysis_users(d) {
            if u.users.is_empty() {
                no_user += 1;
            }
        }
        for (i, f) in d.fields.iter().enumerate() {
            analyzer += f.analyzer.is_some() as usize;
            search += f.search_analyzer.is_some() as usize;
            normalizer += f.normalizer.is_some() as usize;
            match search_field_role(&d.fields, i) {
                SearchFieldRole::MultiField => multi += 1,
                SearchFieldRole::Property => object += 1,
                SearchFieldRole::Field => {}
            }
            if search_field_analyzers(d, &f.path).is_some()
                && search_analyzer_difference(d, &f.path).is_some()
            {
                diff += 1;
            }
        }
    }
    println!("declared parts per kind: {declared:?}");
    println!("chain refs per class: {classes:?}");
    println!(
        "fields with analyzer/search_analyzer/normalizer: {analyzer} / {search} / {normalizer}"
    );
    println!(
        "multi-fields: {multi}; object children: {object}; fields with differing index/search chain: {diff}"
    );
    println!(
        "parts without user: {no_user}; merged parts (total - distinct): {}",
        total - distinct.len()
    );
}
