//! The Elasticsearch graph model (ticket 20): synthetic hand-built topologies only.
use archguard_engine::{
    SearchGraph, SearchNode, analyze_services,
    core::{
        SearchArtifact, SearchFinding, SearchIndexUsage, SearchReference, Service, ServiceTopology,
    },
    search_graph,
};
use std::path::Path;

fn svc(name: &str) -> Service {
    Service {
        id: name.into(),
        name: name.into(),
        compose_file: String::new(),
        build_context: None,
        networks: vec![],
        profiles: vec![],
        project: None,
    }
}

fn art(kind: &str, name: Option<&str>, address: &str, scope: &str) -> SearchArtifact {
    SearchArtifact {
        kind: kind.into(),
        name: name.map(Into::into),
        address: address.into(),
        scope: scope.into(),
        patterns: vec![],
        references: vec![],
        evidence: vec![],
    }
}

fn use_(service: &str, index: &str, access: &str) -> SearchIndexUsage {
    SearchIndexUsage {
        service: service.into(),
        index: index.into(),
        access: access.into(),
        evidence: vec![],
    }
}

fn finding(kind: &str, subject: &str) -> SearchFinding {
    SearchFinding {
        kind: kind.into(),
        subject: subject.into(),
        services: vec![],
        evidence: vec![],
    }
}

fn topo(
    services: &[&str],
    artifacts: Vec<SearchArtifact>,
    usages: Vec<SearchIndexUsage>,
) -> ServiceTopology {
    let mut t = ServiceTopology::default();
    t.services = services.iter().map(|s| svc(s)).collect();
    t.elasticsearch.artifacts = artifacts;
    t.elasticsearch.usages = usages;
    t
}

fn index(name: &str) -> SearchArtifact {
    art("index", Some(name), &format!("x.{name}"), "")
}

fn node<'a>(g: &'a SearchGraph, label: &str) -> (usize, &'a SearchNode) {
    g.nodes
        .iter()
        .enumerate()
        .find(|(_, n)| n.label == label)
        .unwrap_or_else(|| panic!("no node {label}"))
}

fn edges(g: &SearchGraph) -> Vec<(String, String, &'static str)> {
    g.edges
        .iter()
        .map(|e| {
            (
                g.nodes[e.from].label.clone(),
                g.nodes[e.to].label.clone(),
                e.relation,
            )
        })
        .collect()
}

#[test]
fn nodes_are_services_in_usages_then_artifacts_in_order() {
    let t = topo(
        &["api", "web", "idle"],
        vec![index("logs-a"), art("ingest_pipeline", Some("p1"), "x", "")],
        vec![
            use_("web", "logs-a", "read"),
            use_("api", "logs-a", "write"),
        ],
    );
    let g = search_graph(&t);
    let got: Vec<_> = g
        .nodes
        .iter()
        .map(|n| (n.kind.as_str(), n.label.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            ("service", "web"),
            ("service", "api"),
            ("index", "logs-a"),
            ("ingest_pipeline", "p1")
        ]
    );
    assert_eq!(g.nodes[0].service, Some(1));
    assert_eq!(g.nodes[2].artifact, Some(0));
    assert!(g.nodes.iter().all(|n| n.declared));
}

#[test]
fn usage_edges_follow_access() {
    let t = topo(
        &["api", "web"],
        vec![index("logs-a")],
        vec![
            use_("api", "logs-a", "write"),
            use_("web", "logs-a", "read"),
            use_("web", "logs-a", "unknown"),
        ],
    );
    assert_eq!(
        edges(&search_graph(&t)),
        [
            ("api".into(), "logs-a".into(), "write"),
            ("logs-a".into(), "web".into(), "read"),
            ("web".into(), "logs-a".into(), "unknown"),
        ]
    );
}

#[test]
fn comma_lists_use_the_findings_split_rule() {
    let t = topo(
        &["api"],
        vec![index("a"), index("b")],
        vec![use_("api", "a,b", "write"), use_("api", "a,-b", "read")],
    );
    let e = edges(&search_graph(&t));
    assert!(e.contains(&("api".into(), "a".into(), "write")));
    assert!(e.contains(&("api".into(), "b".into(), "write")));
    assert!(e.contains(&("a".into(), "api".into(), "read")));
    assert!(!e.contains(&("b".into(), "api".into(), "read")));
    assert_eq!(e.len(), 3);
}

#[test]
fn undeclared_index_becomes_one_ghost_for_all_usages() {
    let t = topo(
        &["api", "web"],
        vec![],
        vec![
            use_("api", "logs-a", "write"),
            use_("web", "logs-a", "read"),
        ],
    );
    let g = search_graph(&t);
    let ghosts: Vec<_> = g.nodes.iter().filter(|n| !n.declared).collect();
    assert_eq!(ghosts.len(), 1);
    let (i, n) = node(&g, "logs-a");
    assert!(n.warn && !n.declared && n.kind == "index" && n.artifact.is_none());
    assert_eq!(
        g.edges.iter().filter(|e| e.from == i || e.to == i).count(),
        2
    );
}

#[test]
fn reference_edges_and_unresolved_ghost() {
    let mut a = art("index_template", Some("t1"), "x.t1", "");
    a.references = vec![
        SearchReference {
            kind: "ingest_pipeline".into(),
            name: "p1".into(),
        },
        SearchReference {
            kind: "ingest_pipeline".into(),
            name: "gone".into(),
        },
    ];
    let t = topo(
        &[],
        vec![a, art("ingest_pipeline", Some("p1"), "x.p1", "")],
        vec![],
    );
    let g = search_graph(&t);
    let e = edges(&g);
    assert!(e.contains(&("t1".into(), "p1".into(), "reference")));
    assert!(e.contains(&("t1".into(), "gone".into(), "reference")));
    let (_, ghost) = node(&g, "gone");
    assert!(ghost.warn && !ghost.declared && ghost.kind == "ingest_pipeline");
    assert!(!node(&g, "p1").1.warn);
}

#[test]
fn covers_edges_go_from_patterns_to_declared_indices_and_aliases() {
    let mut tpl = art("index_template", Some("t1"), "x.t1", "");
    tpl.patterns = vec!["logs-*".into()];
    let t = topo(
        &[],
        vec![
            tpl,
            index("logs-a"),
            art("alias", Some("logs-z"), "x.z", ""),
            index("other"),
        ],
        vec![],
    );
    assert_eq!(
        edges(&search_graph(&t)),
        [
            ("t1".into(), "logs-a".into(), "covers"),
            ("t1".into(), "logs-z".into(), "covers"),
        ]
    );
}

#[test]
fn warn_follows_subject_against_name_pattern_or_reference_but_not_lookalikes() {
    let mut tpl = art("index_template", Some("t1"), "x.t1", "");
    tpl.patterns = vec!["logs-*".into()];
    let mut by_ref = art("index_template", Some("t2"), "x.t2", "");
    by_ref.references = vec![SearchReference {
        kind: "ingest_pipeline".into(),
        name: "p1".into(),
    }];
    let mut t = topo(
        &[],
        vec![index("logs-a"), index("logs-ab"), tpl, by_ref],
        vec![],
    );
    t.elasticsearch.findings = vec![
        finding("undeclared_index", "logs-a"),
        finding("x", "logs-*"),
        finding("y", "p1"),
    ];
    let g = search_graph(&t);
    assert!(node(&g, "logs-a").1.warn);
    assert!(!node(&g, "logs-ab").1.warn);
    assert!(node(&g, "t1").1.warn);
    assert!(node(&g, "t2").1.warn);
}

fn label_of(a: SearchArtifact) -> (String, String) {
    let g = search_graph(&topo(&[], vec![a], vec![]));
    (g.nodes[0].label.clone(), g.nodes[0].title.clone())
}

#[test]
fn label_is_name_title_is_name_or_address() {
    assert_eq!(
        label_of(art("index", Some("orders"), "x.y", "")),
        ("orders".into(), "orders".into())
    );
}

#[test]
fn label_without_name_is_the_resource_name_from_the_address() {
    let addr = r#"module.m.elasticstack_elasticsearch_index.orders["x"]"#;
    assert_eq!(
        label_of(art("index", None, addr, "")),
        (r#"orders["x"]"#.into(), addr.into())
    );
    let nested = "module.a.module.b.elasticstack_kibana_data_view.dv";
    assert_eq!(label_of(art("data_view", None, nested, "")).0, "dv");
    assert_eq!(label_of(art("index", None, "plain.res", "")).0, "plain.res");
}

#[test]
fn equal_labels_of_different_scopes_get_the_scope() {
    let t = topo(
        &[],
        vec![
            art("index", Some("same"), "a", "root-a"),
            art("index", Some("same"), "b", "root-b"),
            art("index", Some("solo"), "c", "root-a"),
        ],
        vec![],
    );
    let labels: Vec<_> = search_graph(&t)
        .nodes
        .into_iter()
        .map(|n| n.label)
        .collect();
    assert_eq!(labels, ["same (root-a)", "same (root-b)", "solo"]);
}

#[test]
fn labels_over_32_characters_are_cut_to_31_plus_ellipsis_on_char_boundaries() {
    let long = "ä".repeat(40);
    let (label, title) = label_of(art("index", Some(&long), "a", ""));
    assert_eq!(label, format!("{}…", "ä".repeat(31)));
    assert_eq!(label.chars().count(), 32);
    assert_eq!(title, long);
    let exact = "b".repeat(32);
    assert_eq!(label_of(art("index", Some(&exact), "a", "")).0, exact);
}

/// Counts only, no names; the path comes from the environment (spec `## Abnahme-Probe`).
#[test]
#[ignore]
fn probe() {
    let root = std::env::var("ARCHGUARD_ES_PROBE").expect("ARCHGUARD_ES_PROBE");
    let t = analyze_services(Path::new(&root)).unwrap();
    let g = search_graph(&t);
    let mut kinds = std::collections::BTreeMap::<&str, usize>::new();
    for n in &g.nodes {
        *kinds.entry(n.kind.as_str()).or_default() += 1;
    }
    let mut rel = std::collections::BTreeMap::<&str, usize>::new();
    for e in &g.edges {
        *rel.entry(e.relation).or_default() += 1;
    }
    let ghosts = g.nodes.iter().filter(|n| !n.declared).count();
    let unnamed = t
        .elasticsearch
        .artifacts
        .iter()
        .filter(|a| a.name.is_none())
        .count();
    let longest = g
        .nodes
        .iter()
        .map(|n| n.label.chars().count())
        .max()
        .unwrap_or(0);
    let mut seen = std::collections::BTreeMap::<&str, usize>::new();
    for n in &g.nodes {
        *seen.entry(n.label.as_str()).or_default() += 1;
    }
    let dup = seen.values().filter(|c| **c > 1).count();
    let shown: Vec<_> = g
        .nodes
        .iter()
        .filter(|n| !n.declared || matches!(n.kind.as_str(), "service" | "index" | "alias"))
        .collect();
    let groups = shown
        .iter()
        .filter(|n| n.kind != "service")
        .map(|n| n.kind.as_str())
        .collect::<std::collections::BTreeSet<_>>()
        .len();
    println!("nodes by kind: {kinds:?}");
    println!("edges by relation: {rel:?}");
    println!("warn: {}", g.nodes.iter().filter(|n| n.warn).count());
    println!("ghosts: {ghosts}, nodes without literal name: {unnamed}");
    println!("longest label: {longest}, duplicate labels: {dup}");
    println!("default view rows: {}", shown.len() + groups);
}
