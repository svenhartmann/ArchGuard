//! PHP classes from the call analysis name the file nodes of the import graph.
//! Kept outside `php_calls`: the full pipeline itself depends on that module.

use std::fs;

use archguard_core::Config;
use archguard_engine::{Level, analyze, analyze_calls};

/// A PSR-4 project with one call of each kind the resolver distinguishes.
const SHOP: [(&str, &str); 4] = [
    (
        "composer.json",
        r#"{"autoload": {"psr-4": {"App\\": "src/"}}}"#,
    ),
    (
        "src/Service/Report.php",
        "<?php\nnamespace App\\Service;\n\nuse App\\Model\\Order;\n\nclass Report {\n    public function build($repo) {\n        $order = new Order();\n        $repo->save($order);\n        strtoupper('x');\n        return \\App\\Support\\money($order->total());\n    }\n}\n",
    ),
    (
        "src/Model/Order.php",
        "<?php\nnamespace App\\Model;\n\nclass Order {\n    public function total() { return $this->sum(); }\n    private function sum() { return 1; }\n}\n",
    ),
    (
        "src/Support/money.php",
        "<?php\nnamespace App\\Support;\n\nfunction money($x) { return $x; }\n",
    ),
];

fn write(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (path, source) in files {
        let path = dir.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }
    dir
}

#[test]
fn php_calls_resolve_statically_and_reach_the_graph() {
    let dir = write(&SHOP);
    let calls = analyze_calls(dir.path(), &Config::default());
    assert_eq!(calls.status, "partial");
    let count = |kind: &str| calls.calls.iter().filter(|c| c.resolution == kind).count();
    // new Order, money(), $this->sum(); strtoupper; $repo->save, $order->total.
    assert_eq!(
        (
            calls.calls.len(),
            count("project"),
            count("external"),
            count("unresolved")
        ),
        (6, 3, 1, 2),
        "{:#?}",
        calls.calls
    );
    // Project calls between two files become graph evidence: Report → Order
    // and Report → money.php; `$this->sum()` stays inside Order.php.
    let (graph, _) = analyze(dir.path(), &Config::default(), Level::File).unwrap();
    let semantic = graph.semantic.unwrap();
    assert_eq!(
        (semantic.projected_calls, semantic.skipped_project_calls),
        (2, 0)
    );
}

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
