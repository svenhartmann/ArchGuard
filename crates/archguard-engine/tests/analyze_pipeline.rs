//! T22 integration test: the complete [`archguard_engine::analyze`]
//! pipeline over a tiny inline fixture tree — three TypeScript files
//! whose relative imports form exactly one cycle.
//!
//! Asserted: 1 SCC found, `Status::Fail` (the default
//! `rules.deny_cycles = module` matches the analysed module level, so
//! the cycle finding is an `Error`), and at least one suggested cut.

use std::fs;

use archguard_core::{Config, FindingKind, Severity, Status};
use archguard_engine::{Level, analyze};

/// A → B → C → A via relative path imports, under the default
/// source root `src`.
const FIXTURE: [(&str, &str); 3] = [
    (
        "src/a.ts",
        "import { b } from \"./b\";\nexport const a = b + 1;\n",
    ),
    (
        "src/b.ts",
        "import { c } from \"./c\";\nexport const b = c + 1;\n",
    ),
    (
        "src/c.ts",
        "import { a } from \"./a\";\nexport const c = a + 1;\n",
    ),
];

#[test]
fn three_importing_typescript_files_form_one_failing_cycle() {
    let dir = tempfile::tempdir().expect("temp dir");
    for (rel, content) in &FIXTURE {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().expect("parent dir")).expect("create dirs");
        fs::write(&path, content).expect("write fixture file");
    }

    let (graph, report) =
        analyze(dir.path(), &Config::default(), Level::Module).expect("analyze succeeds");

    // The full hierarchical graph: three file nodes, three import edges.
    assert_eq!(graph.nodes.len(), 3, "nodes: {:?}", graph.nodes);
    assert_eq!(graph.edges.len(), 3, "edges: {:?}", graph.edges);
    assert_eq!(graph.root, dir.path().to_string_lossy());

    // One cycle in the default module view, gate-failing, with a cut.
    assert_eq!(report.summary.sccs, 1);
    assert_eq!(report.summary.largest_scc, 3);
    assert_eq!(report.summary.status, Status::Fail);
    let cycles: Vec<_> = report
        .findings
        .iter()
        .filter(|finding| finding.kind == FindingKind::Cycle)
        .collect();
    assert_eq!(cycles.len(), 1, "findings: {:?}", report.findings);
    let cycle = &cycles[0];
    assert_eq!(
        cycle.severity,
        Severity::Error,
        "deny_cycles=module must make the module-level cycle an Error"
    );
    assert_eq!(
        cycle.nodes,
        ["a.ts".to_string(), "b.ts".to_string(), "c.ts".to_string()],
    );
    assert!(
        cycle.cycles_sampled >= 1,
        "sampled: {}",
        cycle.cycles_sampled
    );
    assert!(!cycle.suggested_cuts.is_empty(), "a cut must be suggested");
    let cut = &cycle.suggested_cuts[0];
    assert!(
        cycle.nodes.contains(&cut.from) && cycle.nodes.contains(&cut.to),
        "cut {cut:?} must lie inside the cycle"
    );
}

fn two_crates(dependency: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (name, content) in [
        ("archguard.toml", "[project]\nsource_roots = []\n"),
        ("core/Cargo.toml", "[package]\nname = \"app-core\"\n"),
        ("core/src/lib.rs", "pub struct Thing;\n"),
        ("cli/src/lib.rs", "use app_core::Thing;\n"),
    ] {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    std::fs::write(
        dir.path().join("cli/Cargo.toml"),
        format!("[package]\nname = \"app-cli\"\n{dependency}"),
    )
    .unwrap();
    dir
}

#[test]
fn cached_graph_notices_a_changed_cargo_manifest() {
    let dir = two_crates("[dependencies]\napp-core = { path = \"../core\" }\n");
    let run = || {
        let loaded = archguard_engine::load_config(None, dir.path()).unwrap();
        archguard_engine::analyze_run_quiet(dir.path(), &loaded, archguard_engine::Level::File)
            .unwrap()
            .graph
            .edges
            .len()
    };
    assert_eq!(run(), 1);
    std::fs::write(
        dir.path().join("cli/Cargo.toml"),
        "[package]\nname = \"app-cli\"\n",
    )
    .unwrap();
    assert_eq!(run(), 0, "the manifest decides the edge");
}

#[test]
fn configured_languages_limit_the_analyzed_sources() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("keep.rs"), "pub struct Keep;\n").unwrap();
    std::fs::write(dir.path().join("drop.ts"), "export const a = 1;\n").unwrap();
    std::fs::write(dir.path().join("go.go"), "package main\n").unwrap();
    let names = |languages: &str| {
        std::fs::write(
            dir.path().join("archguard.toml"),
            format!("derive_layers = false\n[project]\nlanguages = {languages}\n"),
        )
        .unwrap();
        let loaded = archguard_engine::load_config(None, dir.path()).unwrap();
        let mut names: Vec<_> =
            archguard_engine::analyze_run_quiet(dir.path(), &loaded, archguard_engine::Level::File)
                .unwrap()
                .graph
                .nodes
                .iter()
                .filter(|node| node.name.contains('.'))
                .map(|node| node.name.clone())
                .collect();
        names.sort();
        names
    };
    assert_eq!(names("['rs']"), ["keep.rs"]);
    assert_eq!(names("['rs', 'go']"), ["go.go", "keep.rs"]);
    assert_eq!(names("['auto']"), ["drop.ts", "go.go", "keep.rs"]);
}

#[test]
fn derived_source_roots_never_merge_files_of_different_packages() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["cli/src/lib.rs", "core/src/lib.rs"] {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "pub struct Item;\n").unwrap();
    }
    let loaded = archguard_engine::load_config(None, dir.path()).unwrap();
    let graph =
        archguard_engine::analyze_run_quiet(dir.path(), &loaded, archguard_engine::Level::File)
            .unwrap()
            .graph;
    let files: Vec<_> = graph
        .nodes
        .iter()
        .filter(|node| node.name.ends_with(".rs"))
        .map(|node| node.name.clone())
        .collect();
    assert_eq!(files.len(), 2, "two source files stay two nodes: {files:?}");
}

#[test]
fn configured_source_roots_that_merge_files_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["cli/src/lib.rs", "core/src/lib.rs"] {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "pub struct Item;\n").unwrap();
    }
    std::fs::write(
        dir.path().join("archguard.toml"),
        "[project]\nsource_roots = [\"cli\", \"core\"]\n",
    )
    .unwrap();
    let loaded = archguard_engine::load_config(None, dir.path()).unwrap();
    let run =
        archguard_engine::analyze_run_quiet(dir.path(), &loaded, archguard_engine::Level::File)
            .unwrap();
    assert!(
        run.warnings
            .iter()
            .any(|w| w.contains("cli/src/lib.rs") && w.contains("core/src/lib.rs")),
        "{:?}",
        run.warnings
    );
}

#[test]
fn rust_paths_in_code_link_their_crate_local_target() {
    let dir = tempfile::tempdir().unwrap();
    for (name, content) in [
        (
            "archguard.toml",
            "derive_layers = false\n[project]\nsource_roots = []\n",
        ),
        ("src/lib.rs", "mod a;\nmod b;\n"),
        (
            "src/a.rs",
            "pub fn make() {\n    let _ = crate::b::Repo::new();\n    let _ = Vec::<u8>::new();\n}\n",
        ),
        (
            "src/b.rs",
            "pub struct Repo;\nimpl Repo {\n    pub fn new() -> Self {\n        Repo\n    }\n}\n",
        ),
    ] {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    let loaded = archguard_engine::load_config(None, dir.path()).unwrap();
    let run =
        archguard_engine::analyze_run_quiet(dir.path(), &loaded, archguard_engine::Level::File)
            .unwrap();
    let name = |id: archguard_core::NodeId| run.graph.nodes[id.0 as usize].name.clone();
    let edges: Vec<_> = run
        .graph
        .edges
        .iter()
        .map(|edge| (name(edge.from), name(edge.to), edge.refs[0].line))
        .collect();
    assert!(
        edges.contains(&("src/a.rs".into(), "src/b.rs".into(), 2)),
        "edges: {edges:?}"
    );
}

#[test]
fn external_rust_paths_never_link_a_same_named_reexport() {
    let dir = tempfile::tempdir().unwrap();
    for (name, content) in [
        (
            "archguard.toml",
            "derive_layers = false\n[project]\nsource_roots = []\n",
        ),
        ("src/lib.rs", "mod drift;\npub use drift::drift;\n"),
        (
            "src/drift.rs",
            "use std::collections::BTreeMap;\npub fn drift() {\n    let _ = BTreeMap::<u8, u8>::new();\n}\n",
        ),
    ] {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    let loaded = archguard_engine::load_config(None, dir.path()).unwrap();
    let run =
        archguard_engine::analyze_run_quiet(dir.path(), &loaded, archguard_engine::Level::File)
            .unwrap();
    let name = |id: archguard_core::NodeId| run.graph.nodes[id.0 as usize].name.clone();
    let edges: Vec<_> = run
        .graph
        .edges
        .iter()
        .map(|edge| (name(edge.from), name(edge.to)))
        .collect();
    assert_eq!(edges, [("src/lib.rs".into(), "src/drift.rs".into())]);
}
