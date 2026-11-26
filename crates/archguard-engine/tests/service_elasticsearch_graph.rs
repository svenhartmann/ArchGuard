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
        flavor: String::new(),
        kind: kind.into(),
        name: name.map(Into::into),
        address: address.into(),
        scope: scope.into(),
        patterns: vec![],
        references: vec![],
        evidence: vec![],
        origins: vec![],
        detail: None,
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
    let mut t = ServiceTopology {
        services: services.iter().map(|s| svc(s)).collect(),
        ..Default::default()
    };
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
    let mut got = edges(&search_graph(&t));
    got.sort();
    assert_eq!(
        got,
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
fn a_watch_pattern_covers_matching_indices_only() {
    let mut w = art("watch", Some("w"), "x.w", "");
    w.patterns = vec!["logs-*".into()];
    let t = topo(&[], vec![w, index("logs-a"), index("other")], vec![]);
    assert_eq!(
        edges(&search_graph(&t)),
        [("w".into(), "logs-a".into(), "covers")]
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

fn labels_of(artifacts: Vec<SearchArtifact>) -> Vec<String> {
    search_graph(&topo(&[], artifacts, vec![]))
        .nodes
        .into_iter()
        .map(|n| n.label)
        .collect()
}

#[test]
fn equal_long_names_with_long_scopes_stay_unique_and_within_32() {
    let name = "n".repeat(40);
    let labels = labels_of(vec![
        art(
            "index",
            Some(&name),
            "a",
            "infrastructure/environments/production-eu",
        ),
        art(
            "index",
            Some(&name),
            "b",
            "infrastructure/environments/production-us",
        ),
    ]);
    assert_ne!(labels[0], labels[1]);
    assert!(labels.iter().all(|l| l.chars().count() <= 32), "{labels:?}");
}

#[test]
fn scopes_with_the_same_last_segment_get_the_next_segment() {
    let labels = labels_of(vec![
        art("index", Some("same"), "a", "x/env"),
        art("index", Some("same"), "b", "y/env"),
    ]);
    assert_eq!(labels, ["same (x/env)", "same (y/env)"]);
}

#[test]
fn labels_are_unique_over_a_generated_graph() {
    let arts: Vec<_> = (0..400)
        .map(|i| {
            let scope = format!("roots/team-{}/environments/stage-{}", i % 7, i % 3);
            art(
                "index",
                Some(&format!("{}-{}", "name".repeat(i % 12), i % 20)),
                "a",
                &scope,
            )
        })
        .collect();
    let labels = labels_of(arts);
    let unique: std::collections::HashSet<_> = labels.iter().collect();
    assert_eq!(unique.len(), labels.len());
    assert!(labels.iter().all(|l| l.chars().count() <= 32));
}

const TF: &str = "elasticstack_elasticsearch_index";

fn tf(name: &str, scope: &str, module: &str) -> SearchArtifact {
    let address = format!("{module}{TF}.res");
    art("index", Some(name), &address, &format!("terraform:{scope}"))
}

#[test]
fn service_label_is_the_service_name_the_id_stays_in_the_title() {
    let mut t = topo(
        &[],
        vec![index("logs-a")],
        vec![use_("project:functions/api", "logs-a", "write")],
    );
    let mut s = svc("api");
    s.id = "project:functions/api".into();
    t.services = vec![s];
    let g = search_graph(&t);
    assert_eq!(
        (g.nodes[0].label.as_str(), g.nodes[0].title.as_str()),
        ("api", "project:functions/api")
    );
}

#[test]
fn terraform_roots_are_told_apart_by_folder_path_and_instances_by_module_name() {
    let mut arts = Vec::new();
    for scope in ["infra", "local/infra"] {
        for module in ["module.a.", "module.b."] {
            arts.push(tf("doka", scope, module));
        }
    }
    arts.push(tf("solo", "infra", "module.a."));
    let labels = labels_of(arts);
    assert_eq!(
        labels,
        [
            "doka (infra) · module.a",
            "doka (infra) · module.b",
            "doka (local/infra) · module.a",
            "doka (local/infra) · module.b",
            "solo",
        ]
    );
}

#[test]
fn a_scope_prefix_is_not_a_path_segment_and_a_lone_root_has_no_addition() {
    let labels = labels_of(vec![
        tf("cms", "x/terraform", ""),
        tf("cms", "y/terraform", ""),
        tf("one", "terraform", ""),
    ]);
    assert_eq!(labels, ["cms (x/terraform)", "cms (y/terraform)", "one"]);
}

#[test]
fn instances_of_one_root_without_a_second_root_get_no_scope() {
    let labels = labels_of(vec![
        tf("doka", "infra", "module.a."),
        tf("doka", "infra", "module.b."),
    ]);
    assert_eq!(labels, ["doka · module.a", "doka · module.b"]);
}

fn from(origins: &[&str], mut a: SearchArtifact) -> SearchArtifact {
    a.origins = origins.iter().map(|o| o.to_string()).collect();
    a
}

fn set_topo(projects: &[&str]) -> ServiceTopology {
    let mut t = topo(
        &[],
        vec![
            from(&["infra"], index("orders")),
            from(&["infra", "adapter"], index("audit")),
            from(&["adapter"], index("local")),
        ],
        vec![
            use_("writer", "audit", "write"),
            use_("adapter:writer", "orders", "write"),
        ],
    );
    t.elasticsearch.projects = projects.iter().map(|p| p.to_string()).collect();
    t
}

fn foreign_labels(t: &ServiceTopology) -> Vec<String> {
    let mut v: Vec<_> = search_graph(t)
        .nodes
        .iter()
        .filter(|n| n.foreign)
        .map(|n| n.label.clone())
        .collect();
    v.sort();
    v
}

#[test]
fn without_a_set_nothing_is_foreign() {
    let mut t = set_topo(&[]);
    t.elasticsearch
        .artifacts
        .iter_mut()
        .for_each(|a| a.origins.clear());
    assert!(search_graph(&t).nodes.iter().all(|n| !n.foreign));
}

#[test]
fn foreign_follows_the_primary_project_in_either_order() {
    assert_eq!(foreign_labels(&set_topo(&["adapter", "infra"])), ["orders"]);
    let l = foreign_labels(&set_topo(&["infra", "adapter"]));
    assert_eq!(l.len(), 2);
    assert!(l[0] == "local" && l[1].starts_with("writer"));
}

#[test]
fn a_deduplicated_artifact_with_the_primary_among_its_origins_is_not_foreign() {
    let g = search_graph(&set_topo(&["adapter", "infra"]));
    assert!(!node(&g, "audit").1.foreign);
}

#[test]
fn foreign_services_keep_short_unique_labels() {
    let g = search_graph(&set_topo(&["infra", "adapter"]));
    let writers: Vec<_> = g.nodes.iter().filter(|n| n.kind == "service").collect();
    assert_eq!(writers.len(), 2);
    assert!(writers.iter().all(|n| !n.label.contains("adapter:")));
    assert_ne!(writers[0].label, writers[1].label);
    assert!(g.nodes.iter().all(|n| n.label.chars().count() <= 32));
    let foreign = writers.iter().find(|n| n.foreign).unwrap();
    assert!(foreign.label.starts_with("writer"));
    assert!(foreign.title.ends_with(" · adapter"));
}

#[test]
fn a_foreign_service_shows_its_name_not_its_compose_id() {
    let mut t = set_topo(&["infra", "adapter"]);
    t.elasticsearch.usages = vec![use_("adapter:compose.yaml::writer", "orders", "write")];
    let g = search_graph(&t);
    let n = g.nodes.iter().find(|n| n.kind == "service").unwrap();
    assert_eq!((n.label.as_str(), n.foreign), ("writer", true));
    assert_eq!(n.projects, ["adapter"]);
}

#[test]
fn projects_are_the_origins_and_the_service_prefix_even_with_a_separator_in_the_name() {
    let mut t = set_topo(&["infra", "a · b"]);
    t.elasticsearch.artifacts = vec![from(&["a · b"], index("x")), index("plain")];
    t.elasticsearch.usages = vec![use_("a · b:compose.yaml::w", "x", "write")];
    let g = search_graph(&t);
    assert_eq!(node(&g, "x").1.projects, ["a · b"]);
    assert_eq!(node(&g, "w").1.projects, ["a · b"]);
    assert!(node(&g, "plain").1.projects.is_empty());
    let g = search_graph(&set_topo(&["infra", "adapter"]));
    assert_eq!(node(&g, "audit").1.projects, ["infra", "adapter"]);
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
    let numbered = g.nodes.iter().filter(|n| n.label.contains(" #")).count();
    let colon = g
        .nodes
        .iter()
        .filter(|n| {
            n.label
                .split_once(" (")
                .is_some_and(|(_, z)| z.contains(':'))
        })
        .count();
    let cut = g
        .nodes
        .iter()
        .filter(|n| n.kind == "service" && n.label.ends_with('…'))
        .count();
    println!(
        "labels with #n: {numbered}, with ':' in the addition: {colon}, cut service labels: {cut}"
    );
    println!("nodes by kind: {kinds:?}");
    println!("edges by relation: {rel:?}");
    println!("warn: {}", g.nodes.iter().filter(|n| n.warn).count());
    println!("ghosts: {ghosts}, nodes without literal name: {unnamed}");
    println!("longest label: {longest}, duplicate labels: {dup}");
    println!("default view rows: {}", shown.len() + groups);
}
