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
fn index_details_list_every_field_as_a_tree_with_roles_and_depth() {
    let t = topology();
    let g = search_graph(&t);
    let i = index(&t, &g);
    assert_eq!(
        i.fields.len(),
        i.page.mapping.len(),
        "all fields, not only analyzed ones"
    );
    let row = |p: &str| {
        i.fields
            .iter()
            .find(|f| f.path == p)
            .unwrap_or_else(|| panic!("{p}"))
    };
    let shape = |p: &str| (row(p).role, row(p).depth);
    assert_eq!(shape("title"), (SearchFieldRole::Field, 0));
    assert_eq!(shape("title.raw"), (SearchFieldRole::MultiField, 1));
    assert_eq!(shape("body"), (SearchFieldRole::Field, 0));
    assert_eq!(shape("author.name"), (SearchFieldRole::Property, 1));
    assert_eq!(shape("lines.sku"), (SearchFieldRole::Property, 1));
    assert_eq!(row("title").search_analyzer.as_deref(), Some("qry_a"));
    assert_eq!(row("title.raw").normalizer.as_deref(), Some("fold_n"));
}

#[test]
fn index_details_have_one_chain_table_per_used_declared_analyzer() {
    let t = topology();
    let g = search_graph(&t);
    let i = index(&t, &g);
    let names: Vec<&str> = i.chains.iter().map(|c| c.analyzer.as_str()).collect();
    assert_eq!(
        names,
        ["idx_a", "qry_a", "twin_a", "shingle_a", "fold_n"],
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
    assert_eq!(
        fields,
        ["title", "title.shingle", "body", "author.name"],
        "same: equal stages, no hint"
    );
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

fn field(t: &ServiceTopology, g: &SearchGraph, path: &str) -> FieldDetails {
    field_details(t, g, node(g, "index", "docs"), path).unwrap_or_else(|| panic!("{path}"))
}

#[test]
fn field_details_resolve_the_index_and_the_search_chain() {
    let t = topology();
    let g = search_graph(&t);
    let f = field(&t, &g, "title");
    assert_eq!((f.kind.as_str(), f.role), ("text", SearchFieldRole::Field));
    let (index, search) = (f.index.unwrap(), f.search.unwrap());
    assert_eq!((index.name.as_str(), index.declared), ("idx_a", true));
    assert_eq!(
        index.stages[0],
        stage(
            "char_filter",
            0,
            "dash_map",
            "declared",
            &[("mappings", "- => _")]
        )
    );
    assert_eq!(index.stages.len(), 4);
    assert_eq!(search.name, "qry_a");
    assert_eq!(
        search.stages[2],
        stage("filter", 2, "syn", "declared", &[("synonyms", "a, b")])
    );
    assert!(f.normalizer.is_none());
    let hint = f.hint.expect("the stages differ");
    assert!(hint.tokenizer_differs && hint.field == "title");
}

#[test]
fn field_details_follow_the_search_order_of_the_spec() {
    let t = topology();
    let g = search_graph(&t);
    // `body` names nothing: declared `default` indexes, declared `default_search` searches.
    let f = field(&t, &g, "body");
    let (index, search) = (f.index.unwrap(), f.search.unwrap());
    assert_eq!(
        (index.name.as_str(), search.name.as_str()),
        ("default", "default_search")
    );
    assert!(index.declared && search.declared);
    assert_eq!(search.stages.last().map(|s| s.name.as_str()), Some("trim"));
    // `title.shingle` has an analyzer and no search analyzer: `default_search` still wins over it.
    let f = field(&t, &g, "title.shingle");
    assert_eq!(f.index.unwrap().name, "shingle_a");
    assert_eq!(f.search.unwrap().name, "default_search");
}

#[test]
fn field_details_have_the_normalizer_chain_and_none_for_fields_without_analysis() {
    let t = topology();
    let g = search_graph(&t);
    let raw = field(&t, &g, "title.raw");
    assert_eq!(raw.role, SearchFieldRole::MultiField);
    assert!(raw.index.is_none() && raw.search.is_none() && raw.hint.is_none());
    let n = raw.normalizer.expect("fold_n");
    assert_eq!((n.name.as_str(), n.declared), ("fold_n", true));
    assert_eq!(n.stages, [stage("filter", 1, "lowercase", "builtin", &[])]);
    let sku = field(&t, &g, "lines.sku");
    assert_eq!(sku.role, SearchFieldRole::Property);
    assert!(sku.index.is_none() && sku.normalizer.is_none());
    assert!(field_details(&t, &g, node(&g, "index", "docs"), "nope").is_none());
}

fn opensearch_topology() -> ServiceTopology {
    let art = |kind: &str, name: &str| {
        serde_json::json!({"kind": kind, "name": name, "address": format!("res.{name}"),
            "scope": "terraform:infra/main", "flavor": "opensearch",
            "patterns": [], "references": [], "evidence": []})
    };
    let es = serde_json::json!({"findings": [], "usages": [],
        "artifacts": [art("index", "os-idx"), art("ism_policy", "os-ism")]});
    ServiceTopology {
        elasticsearch: serde_json::from_value(es).unwrap(),
        ..Default::default()
    }
}

#[test]
fn opensearch_artifacts_show_the_product_in_the_head_scope() {
    let t = opensearch_topology();
    let g = search_graph(&t);
    let at = |kind: &str| Key::Node(g.nodes.iter().position(|n| n.kind == kind).unwrap());
    match details(&t, &g, &at("index")) {
        Details::Index(i) => assert_eq!(i.page.overview.scope, "OpenSearch \u{b7} infra/main"),
        other => panic!("{other:?}"),
    }
    match details(&t, &g, &at("ism_policy")) {
        Details::Other(o) => assert_eq!(o.scope.as_str(), "OpenSearch \u{b7} terraform:infra/main"),
        other => panic!("{other:?}"),
    }
}
