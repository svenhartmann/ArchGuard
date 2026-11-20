//! Details → pattern candidates on temp copies of the repo fixtures, built as the tab builds them (`build_from`), without a
//! window.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use slint::Model;

use super::state::{ServicesState, build_from};
use crate::state::marks::Marks;
use crate::tree::Tree;

/// A copy of the tracked files of `fixtures/<name>`.
fn fixture(name: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name);
    let listed = std::process::Command::new("git")
        .arg("-C")
        .arg(&src)
        .args(["ls-files", "-z", "."])
        .output()
        .unwrap();
    assert!(listed.status.success());
    for file in listed.stdout.split(|&b| b == 0).filter(|f| !f.is_empty()) {
        let file = Path::new(std::str::from_utf8(file).unwrap());
        let to = dir.path().join(file);
        fs::create_dir_all(to.parent().unwrap()).unwrap();
        fs::copy(src.join(file), &to).unwrap();
    }
    dir
}

/// The tab state of `root` after an analysis (`sections::read`, as
/// `build_from` does), and the analysed graph.
fn state(root: &Path) -> (ServicesState, archguard_engine::core::Graph) {
    let loaded = archguard_engine::load_config(None, root).unwrap();
    let run = archguard_engine::analyze_run_quiet(root, &loaded, archguard_engine::Level::Module)
        .unwrap();
    let calls = archguard_engine::analyze_calls(root, &run.config);
    let topology = Arc::new(archguard_engine::analyze_services(root).map_err(|e| e.to_string()));
    let mut s = build_from(topology);
    super::sections::read(&mut s, root, &run, &calls);
    (s, run.graph)
}

/// py-layers: the layered candidate with its counter-evidence comes
/// first, the dependency levels after it as a structural observation; an
/// open candidate lists its evidence and marks it on the Graph's rows.
#[test]
fn pattern_candidates_list_evidence_and_mark_the_graph() {
    let dir = fixture("py-layers");
    let root = dir.path().canonicalize().unwrap();
    let (mut s, graph) = state(&root);
    let m = s.patterns.model();
    assert_eq!(m.state, 1);
    let rows: Vec<_> = m.rows.iter().collect();
    assert_eq!(
        (rows[0].kind, rows[0].status),
        (0, 1),
        "layered, with counter-evidence"
    );
    assert_eq!(
        (rows[1].kind, rows[1].status),
        (1, 0),
        "dependency levels, candidate"
    );
    assert_eq!((m.candidates, m.structures), (1, 1));
    assert_eq!(m.total as usize, rows.len());
    assert!(rows.iter().all(|r| !r.open));

    s.patterns.choose(0);
    let d = s.patterns.model().detail;
    assert_eq!((d.evidence_total, d.counter_total), (10, 2));
    let counter: Vec<_> = d.counter.iter().collect();
    assert_eq!(counter[0].title, "domain/ledger.py → app/service.py");
    assert!(counter[0].place.contains("domain/ledger.py:"));
    assert!(!d.more_evidence);
    assert!(d.nodes > 0 && d.metrics.contains("backward_dependencies"));

    let (files, pairs) = s.patterns.highlight().unwrap();
    assert!(files.contains(&"domain/ledger.py".to_string()));
    let tree = Tree::from_graph(&graph);
    let marks = Marks::new(&tree, &files, &pairs);
    let open: BTreeSet<usize> = (0..tree.nodes.len()).collect();
    let view = tree.view(&open);
    let name = |r: usize| tree.nodes[view.rows[r].node].name.as_str();
    let links: Vec<_> = marks
        .links(&tree, &view)
        .into_iter()
        .map(|(a, b)| (name(a), name(b)))
        .collect();
    assert!(
        links.contains(&("domain/ledger.py", "app/service.py")),
        "{links:?}"
    );

    s.patterns.choose(0);
    assert!(s.patterns.highlight().is_none(), "closed again");
    assert!(!s.patterns.model().rows.row_data(0).unwrap().open);
}
