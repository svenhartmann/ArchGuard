//! PHP classes from the call analysis name the file nodes of the import graph.
//! Kept outside `php_calls`: the full pipeline itself depends on that module.

use std::fs;

use archguard_core::Config;
use archguard_engine::{Level, analyze, analyze_calls};

#[test]
fn php_classes_reach_their_graph_nodes() {
    let dir = tempfile::tempdir().unwrap();
    for (path, source) in [
        (
            "src/Domain/User.php",
            "<?php\nnamespace App\\Domain;\n\nabstract class User {\n    public function name($a) { return $a ? 'x' : 'y'; }\n}\n",
        ),
        ("src/broken.php", "<?php\nfunction broken() { if (\n"),
    ] {
        let path = dir.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }
    let calls = analyze_calls(dir.path(), &Config::default());
    let user = calls.classes.iter().find(|c| c.name == "User").unwrap();
    let (graph, _) = analyze(dir.path(), &Config::default(), Level::File).unwrap();
    assert!(
        graph.nodes.iter().any(|node| node.name == user.graph_node),
        "{} is not a graph node: {:?}",
        user.graph_node,
        graph.nodes.iter().map(|n| &n.name).collect::<Vec<_>>()
    );
}
