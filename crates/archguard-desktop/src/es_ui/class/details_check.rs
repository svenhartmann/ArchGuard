//! Tests for the detail model, ticket 07.

use std::path::Path;

use archguard_engine::core::{SearchIndexUsage, ServiceEvidence, ServiceTopology};
use archguard_engine::{SearchFieldRole, SearchGraph, analyze_services, search_graph};

use super::details::*;
use super::types::Key;

fn topology() -> ServiceTopology {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/es-classdiagram/02/terraform");
    analyze_services(&root).unwrap()
}

fn node(g: &SearchGraph, kind: &str, title: &str) -> usize {
    g.nodes
        .iter()
        .position(|n| n.kind == kind && n.title == title)
        .unwrap_or_else(|| panic!("{kind} {title}"))
}

fn index(t: &ServiceTopology, g: &SearchGraph) -> IndexDetails {
    match details(t, g, &Key::Node(node(g, "index", "docs"))) {
        Details::Index(i) => i,
        other => panic!("{other:?}"),
    }
}

fn part(t: &ServiceTopology, g: &SearchGraph, kind: &str, name: &str) -> PartDetails {
    let key = Key::Part {
        owner: node(g, "index", "docs"),
        kind: kind.into(),
        name: name.into(),
    };
    match details(t, g, &key) {
        Details::Part(p) => p,
        other => panic!("{other:?}"),
    }
}

fn s(a: &str, b: &str) -> (String, String) {
    (a.into(), b.into())
}

fn stage(
    stage: &'static str,
    n: usize,
    name: &str,
    class: &'static str,
    p: &[(&str, &str)],
) -> StageRow {
    StageRow {
        stage,
        n,
        name: name.into(),
        class,
        params: p.iter().map(|(k, v)| s(k, v)).collect(),
    }
}

#[test]
fn index_details_list_fields_with_analysis_and_their_roles() {
    let t = topology();
    let g = search_graph(&t);
    let i = index(&t, &g);
    let fields: Vec<(&str, SearchFieldRole)> =
        i.fields.iter().map(|f| (f.path.as_str(), f.role)).collect();
    assert_eq!(
        fields,
        [
            ("title", SearchFieldRole::Field),
            ("title.raw", SearchFieldRole::MultiField),
            ("title.shingle", SearchFieldRole::MultiField),
            ("same", SearchFieldRole::Field),
            ("author.name", SearchFieldRole::Property),
        ]
    );
    assert_eq!(i.fields[0].search_analyzer.as_deref(), Some("qry_a"));
    assert_eq!(i.fields[1].normalizer.as_deref(), Some("fold_n"));
    assert_eq!(i.page.mapping.len(), 8);
}

#[test]
fn index_details_have_one_chain_table_per_used_declared_analyzer() {
    let t = topology();
    let g = search_graph(&t);
    let i = index(&t, &g);
    let names: Vec<&str> = i.chains.iter().map(|c| c.analyzer.as_str()).collect();
    assert_eq!(
        names,
        ["idx_a", "qry_a", "shingle_a", "twin_a", "fold_n"],
        "unused_a, default and default_search have no field role"
    );
    let chain = |n: &str| i.chains.iter().find(|c| c.analyzer == n).unwrap();
    assert_eq!(chain("idx_a").roles, ["index"]);
    assert_eq!(chain("qry_a").roles, ["search"]);
    assert_eq!(chain("fold_n").roles, ["normalizer"]);
    assert_eq!(
        chain("idx_a").stages,
        [
            stage(
                "char_filter",
                0,
                "dash_map",
                "declared",
                &[("mappings", "- => _")]
            ),
            stage("tokenizer", 0, "standard", "builtin", &[]),
            stage("filter", 1, "lowercase", "builtin", &[]),
            stage("filter", 2, "my_stop", "declared", &[]),
        ]
    );
    assert_eq!(
        chain("qry_a").stages[2],
        stage("filter", 2, "syn", "declared", &[("synonyms", "a, b")])
    );
}

#[test]
fn index_details_hint_only_where_index_and_search_analyzers_differ() {
    let t = topology();
    let g = search_graph(&t);
    let hints = index(&t, &g).hints;
    let fields: Vec<&str> = hints.iter().map(|h| h.field.as_str()).collect();
    assert_eq!(fields, ["title", "body"], "same: equal stages, no hint");
    let title = &hints[0];
    assert_eq!(
        (title.index.as_str(), title.search.as_str()),
        ("idx_a", "qry_a")
    );
    assert!(title.tokenizer_differs);
    assert_eq!(title.only_index, ["dash_map", "my_stop"]);
    assert_eq!(title.only_search, ["syn"]);
}

#[test]
fn part_details_have_definition_parameters_stages_and_used_by() {
    let t = topology();
    let g = search_graph(&t);
    let syn = part(&t, &g, "filter", "syn");
    assert_eq!(syn.kind_type.as_deref(), Some("synonym"));
    assert_eq!(syn.params, [s("synonyms", "a, b")]);
    assert_eq!(syn.used_by, ["analyzer:qry_a"]);
    assert_eq!(
        syn.definition,
        [s("synonyms", "a, b"), s("type", "synonym")]
    );
    assert_eq!(syn.owner, "docs");
    assert!(syn.declaration.is_some());
    let idx = part(&t, &g, "analyzer", "idx_a");
    assert_eq!(idx.stages.len(), 4);
    assert_eq!(
        idx.used_by,
        ["title · index", "same · index", "author.name · index"]
    );
}

#[test]
fn other_kinds_show_the_search_detail() {
    let t = topology();
    let g = search_graph(&t);
    assert!(matches!(
        details(&t, &g, &Key::Node(node(&g, "index", "docs"))),
        Details::Index(_)
    ));
    let svc = g.nodes.iter().position(|n| n.kind == "service");
    if let Some(i) = svc {
        assert!(matches!(details(&t, &g, &Key::Node(i)), Details::Other(_)));
    }
    assert!(matches!(
        details(&t, &g, &Key::Node(usize::MAX)),
        Details::Other(_)
    ));
}

#[test]
fn usages_own_are_clickable_foreign_are_not_and_writes_reads_list_services() {
    let mut t = topology();
    let ev = |f: &str| ServiceEvidence {
        file: f.into(),
        pointer: String::new(),
        line: Some(3),
        description: String::new(),
    };
    t.elasticsearch.projects = vec!["other".into()];
    for (svc, acc) in [("api", "write"), ("other:web", "read")] {
        t.elasticsearch.usages.push(SearchIndexUsage {
            service: svc.into(),
            index: "docs".into(),
            access: acc.into(),
            evidence: vec![ev("src/a.rs")],
        });
    }
    let g = search_graph(&t);
    let i = index(&t, &g);
    let click: Vec<(&str, bool)> = i
        .page
        .usages
        .iter()
        .map(|u| (u.service.as_str(), u.clickable))
        .collect();
    assert_eq!(click, [("api", true), ("other:web", false)]);
    assert_eq!(
        (i.writes, i.reads),
        (vec!["api".to_string()], vec!["other:web".to_string()])
    );
}
