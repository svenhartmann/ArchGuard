//! Relative imports climb the real file tree: a derived source root only
//! renames the files below it, it must not cut `../` off at its border.

use std::fs;

use archguard_engine::Level;

/// The run over a project of `files`.
fn run(files: &[(&str, &str)]) -> archguard_engine::AnalysisRun {
    let dir = tempfile::tempdir().expect("temp dir");
    for (rel, content) in files {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().expect("parent dir")).expect("create dirs");
        fs::write(&path, content).expect("write fixture file");
    }
    let loaded = archguard_engine::load_config(None, dir.path()).expect("config");
    archguard_engine::analyze_run_quiet(dir.path(), &loaded, Level::File).expect("analyze")
}

/// Whether the run has an edge between the nodes of two walked paths.
fn has_edge(run: &archguard_engine::AnalysisRun, from: &str, to: &str) -> bool {
    let id = |path: &str| {
        let name = &run.source_names[path];
        run.graph.nodes.iter().find(|n| &n.name == name).unwrap().id
    };
    let (from, to) = (id(from), id(to));
    run.graph.edges.iter().any(|e| e.from == from && e.to == to)
}

#[test]
fn relative_imports_cross_two_top_level_folders() {
    let run = run(&[
        ("app/a.ts", "import '../shared/y';\nexport const a=1;\n"),
        ("shared/y.ts", "import '../app/a';\nexport const y=1;\n"),
    ]);
    assert!(run.warnings.is_empty(), "warnings: {:?}", run.warnings);
    assert!(has_edge(&run, "app/a.ts", "shared/y.ts"));
    assert!(has_edge(&run, "shared/y.ts", "app/a.ts"));
    assert_eq!(run.report.summary.sccs, 1, "the app <-> shared cycle");
}

#[test]
fn relative_imports_reach_an_index_file_of_another_root() {
    let run = run(&[
        ("app/a.ts", "import '../shared';\nexport const a=1;\n"),
        ("shared/index.ts", "export const y=1;\n"),
    ]);
    assert!(run.warnings.is_empty(), "warnings: {:?}", run.warnings);
    assert!(has_edge(&run, "app/a.ts", "shared/index.ts"));
}

#[test]
fn php_requires_cross_two_top_level_folders() {
    let run = run(&[
        (
            "app/a.php",
            "<?php\nrequire __DIR__ . '/../shared/y.php';\n",
        ),
        (
            "shared/y.php",
            "<?php\nrequire_once __DIR__ . '/../app/a.php';\n",
        ),
    ]);
    assert!(run.warnings.is_empty(), "warnings: {:?}", run.warnings);
    assert!(has_edge(&run, "app/a.php", "shared/y.php"));
    assert!(has_edge(&run, "shared/y.php", "app/a.php"));
}
