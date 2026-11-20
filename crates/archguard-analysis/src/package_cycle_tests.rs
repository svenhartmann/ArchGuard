use archguard_core::{Edge, Graph, Node, NodeId, NodeKind, Ref};

use crate::package_cycle::{SourceScope, explain_package_cycle, source_scope};

/// Packages are the names without a dot; a file's parent is its directory.
fn graph(names: &[&str], edges: &[(&str, &str, &[&str])]) -> Graph {
    let id = |name: &str| NodeId(names.iter().position(|n| *n == name).expect("listed") as u32);
    Graph {
        schema_version: 1,
        root: "/abs".into(),
        nodes: names
            .iter()
            .map(|n| Node {
                id: id(n),
                name: n.to_string(),
                kind: if n.contains('.') {
                    NodeKind::File
                } else {
                    NodeKind::Module
                },
                parent: n.rsplit_once('/').map(|(p, _)| id(p)),
                lang: None,
            })
            .collect(),
        edges: edges
            .iter()
            .map(|(from, to, files)| Edge {
                from: id(from),
                to: id(to),
                refs: files
                    .iter()
                    .map(|f| Ref {
                        file: f.to_string(),
                        line: 1,
                        raw: "import".into(),
                    })
                    .collect(),
                count: Some(files.len() as u64),
            })
            .collect(),
        semantic: None,
        review_kpis: None,
    }
}

fn edge(files: &[&str], count: Option<u64>) -> Edge {
    Edge {
        from: NodeId(0),
        to: NodeId(1),
        refs: files
            .iter()
            .map(|f| Ref {
                file: f.to_string(),
                line: 1,
                raw: String::new(),
            })
            .collect(),
        count,
    }
}

#[test]
fn source_paths_decide_production_test_mixed_or_unknown() {
    let scope = |files: &[&str]| source_scope(&edge(files, Some(files.len() as u64)));
    assert_eq!(scope(&["a/src/main/java/A.java"]), SourceScope::Production);
    assert_eq!(
        scope(&["main/A.java"]),
        SourceScope::Production,
        "a root at the top"
    );
    assert_eq!(scope(&["a/src/testFixtures/F.java"]), SourceScope::Test);
    assert_eq!(scope(&["web/__tests__/a.ts"]), SourceScope::Test);
    assert_eq!(
        scope(&["src/app/main.py"]),
        SourceScope::Unknown,
        "a file named main"
    );
    assert_eq!(
        scope(&["src/main/A.java", "src/test/AT.java"]),
        SourceScope::Mixed
    );
    assert_eq!(
        scope(&["src/main/A.java", "lib/b.py"]),
        SourceScope::Unknown
    );
    assert_eq!(scope(&[]), SourceScope::Unknown, "no evidence");
    assert_eq!(
        source_scope(&edge(&["src/main/A.java"], Some(3))),
        SourceScope::Unknown,
        "an incomplete sample proves nothing"
    );
    assert_eq!(
        source_scope(&edge(&["src/main/A.java"], None)),
        SourceScope::Unknown
    );
}

#[test]
fn one_direction_only_is_no_package_cycle() {
    let g = graph(
        &["a", "b", "a/x.py", "b/y.py"],
        &[("a/x.py", "b/y.py", &["a/x.py"])],
    );
    assert!(explain_package_cycle(&g, "a", "b").is_none());
    assert!(explain_package_cycle(&g, "a", "missing").is_none());
}

/// a/x → b/y → a/x: the example closes at the same file, one file cycle.
#[test]
fn a_return_to_the_same_file_closes_the_chain() {
    let g = graph(
        &["a", "b", "a/x.py", "a/z.py", "b/y.py"],
        &[
            ("a/x.py", "b/y.py", &["a/x.py"]),
            ("b/y.py", "a/x.py", &["b/y.py"]),
            ("a/z.py", "b/y.py", &["a/z.py"]),
        ],
    );
    let e = explain_package_cycle(&g, "a", "b").expect("both ways");
    assert!(e.witness.connected && e.witness.closed);
    assert_eq!(e.witness.files, ["a/x.py", "b/y.py"]);
    assert_eq!(e.witness.edges.len(), 2);
    assert_eq!(e.file_cycles, 1);
    assert_eq!(
        (
            e.directions[0].counts.unknown,
            e.directions[1].counts.unknown
        ),
        (2, 1)
    );
}

/// The return reaches another file of the start package: no file cycle.
#[test]
fn a_return_to_another_file_is_connected_but_open() {
    let g = graph(
        &["a", "b", "a/x.py", "a/z.py", "b/y.py"],
        &[
            ("a/x.py", "b/y.py", &["a/x.py"]),
            ("b/y.py", "a/z.py", &["b/y.py"]),
        ],
    );
    let e = explain_package_cycle(&g, "a", "b").expect("both ways");
    assert!(e.witness.connected && !e.witness.closed);
    assert_eq!(
        (e.witness.start.as_str(), e.witness.middle.as_str()),
        ("a", "b")
    );
    assert_eq!(e.witness.files, ["a/x.py", "b/y.py", "a/z.py"]);
    assert_eq!(e.file_cycles, 0);
}

/// Neither direction continues where the other one ends: separate examples.
#[test]
fn unrelated_directions_give_separate_examples() {
    let g = graph(
        &["a", "b", "a/x.py", "a/z.py", "b/y.py", "b/w.py"],
        &[
            ("a/x.py", "b/y.py", &["a/x.py"]),
            ("b/w.py", "a/z.py", &["b/w.py"]),
        ],
    );
    let e = explain_package_cycle(&g, "a", "b").expect("both ways");
    assert!(!e.witness.connected);
    assert_eq!(e.witness.edges.len(), 2);
    assert_eq!(e.witness.files, ["a/x.py", "b/y.py", "b/w.py", "a/z.py"]);
}

/// Production evidence is preferred over test evidence for the example,
/// and the counts split by source path.
#[test]
fn the_example_prefers_production_code() {
    let g = graph(
        &["a", "b", "a/T.java", "a/P.java", "b/Q.java"],
        &[
            ("a/T.java", "b/Q.java", &["m/src/test/a/T.java"]),
            ("a/P.java", "b/Q.java", &["m/src/main/a/P.java"]),
            ("b/Q.java", "a/P.java", &["m/src/main/b/Q.java"]),
            ("b/Q.java", "a/T.java", &["m/src/test/b/Q.java"]),
        ],
    );
    let e = explain_package_cycle(&g, "a", "b").expect("both ways");
    assert_eq!(e.witness.files, ["a/P.java", "b/Q.java"]);
    assert!(e.witness.closed);
    let c = &e.directions[0].counts;
    assert_eq!((c.production, c.test, c.mixed, c.unknown), (1, 1, 0, 0));
}
