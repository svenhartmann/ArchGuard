//! Kotlin and Java share the JVM package namespace: imports and qualified
//! names resolve across both languages in one graph.

use std::collections::BTreeSet;
use std::fs;

use archguard_core::Config;
use archguard_engine::{Level, analyze};

const FIXTURE: [(&str, &str); 4] = [
    (
        "src/main/kotlin/com/shop/order/Order.kt",
        "package com.shop.order\n\nimport com.shop.pay.Payment\nimport com.shop.util.formatDate\n\nclass Order(val payment: Payment) {\n    fun label() = formatDate()\n}\n",
    ),
    (
        "src/main/java/com/shop/pay/Payment.java",
        "package com.shop.pay;\n\nimport com.shop.order.Order;\n\npublic class Payment {\n    public enum Status { OPEN }\n    Order order;\n}\n",
    ),
    (
        "src/main/kotlin/com/shop/util/Dates.kt",
        "package com.shop.util\n\nfun formatDate() = \"today\"\n",
    ),
    (
        "src/main/kotlin/com/shop/report/Report.kt",
        "package com.shop.report\n\nimport com.shop.pay.Payment.Status\n\nval open = Status.OPEN\nval type = com.shop.order.Order::class\n",
    ),
];

/// The file name (or pseudo-node suffix) at each end of every edge.
fn edges(graph: &archguard_core::Graph) -> BTreeSet<(String, String)> {
    let short = |id| {
        let name = &graph.nodes.iter().find(|node| node.id == id).unwrap().name;
        match name.strip_suffix("/*") {
            Some(module) => format!("{}/*", module.rsplit('/').next().unwrap()),
            None => name.rsplit('/').next().unwrap().to_string(),
        }
    };
    graph
        .edges
        .iter()
        .map(|edge| (short(edge.from), short(edge.to)))
        .collect()
}

#[test]
fn kotlin_and_java_resolve_each_other() {
    let dir = tempfile::tempdir().expect("temp dir");
    for (rel, content) in &FIXTURE {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().expect("parent dir")).expect("create dirs");
        fs::write(&path, content).expect("write fixture file");
    }
    let (graph, _) = analyze(dir.path(), &Config::default(), Level::File).expect("analyze");
    let pair = |from: &str, to: &str| (from.to_string(), to.to_string());
    let found = edges(&graph);
    for expected in [
        // Kotlin → Java and Java → Kotlin through plain imports.
        pair("Order.kt", "Payment.java"),
        pair("Payment.java", "Order.kt"),
        // A nested type resolves to the file of its outer class.
        pair("Report.kt", "Payment.java"),
        // A qualified name in code links without an import.
        pair("Report.kt", "Order.kt"),
        // A top-level function has no file of its name: the package stands in.
        pair("Order.kt", "util/*"),
    ] {
        assert!(
            found.contains(&expected),
            "missing {expected:?} in {found:?}"
        );
    }
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.name.ends_with("Order.kt") && node.lang.as_deref() == Some("kt"))
    );
}

#[test]
fn kt_is_a_configurable_language() {
    let dir = tempfile::tempdir().expect("temp dir");
    for (rel, content) in &FIXTURE {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().expect("parent dir")).expect("create dirs");
        fs::write(&path, content).expect("write fixture file");
    }
    fs::write(
        dir.path().join("archguard.toml"),
        "[project]\nlanguages = ['kt']\n",
    )
    .expect("write config");
    let loaded = archguard_engine::load_config(None, dir.path()).expect("kt is accepted");
    let graph = archguard_engine::analyze_run_quiet(dir.path(), &loaded, Level::File)
        .expect("analyze")
        .graph;
    let files: Vec<_> = graph
        .nodes
        .iter()
        .filter_map(|node| node.lang.as_deref())
        .collect();
    assert!(
        !files.is_empty() && files.iter().all(|lang| *lang == "kt"),
        "{files:?}"
    );
}

#[test]
fn kotlin_classes_reach_their_graph_nodes() {
    let dir = tempfile::tempdir().expect("temp dir");
    for (rel, content) in [FIXTURE[0], FIXTURE[2]] {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().expect("parent dir")).expect("create dirs");
        fs::write(&path, content).expect("write fixture file");
    }
    let calls = archguard_engine::analyze_calls(dir.path(), &Config::default());
    assert_eq!(calls.status, "partial", "{:?}", calls.diagnostics);
    let analyzer = &calls.analyzers[0];
    assert_eq!(
        (
            analyzer.language.as_str(),
            analyzer.status.as_str(),
            analyzer.files
        ),
        ("kotlin", "partial", 2)
    );
    let order = calls.classes.iter().find(|c| c.name == "Order").unwrap();
    let (graph, _) = analyze(dir.path(), &Config::default(), Level::File).expect("analyze");
    assert!(
        graph.nodes.iter().any(|node| node.name == order.graph_node),
        "{} is not a graph node: {:?}",
        order.graph_node,
        graph.nodes.iter().map(|n| &n.name).collect::<Vec<_>>()
    );
    let label = calls.methods.iter().find(|m| m.name == "label").unwrap();
    assert_eq!(label.class_id, order.id);
    assert_eq!(label.complexity.as_ref().unwrap().cyclomatic, 1);
    // The token scanner cannot promise every method: enumeration incomplete.
    let details = calls.details.as_ref().unwrap();
    assert_eq!(details.enumeration_complete, Some(false));
    // `formatDate()` is imported from the other file.
    let format = calls
        .methods
        .iter()
        .find(|m| m.name == "formatDate")
        .unwrap();
    assert_eq!(calls.calls.len(), 1, "{:#?}", calls.calls);
    assert_eq!(calls.calls[0].resolution, "project");
    assert_eq!(calls.calls[0].to_method.as_ref(), Some(&format.id));
}

/// A Kotlin-only project with one call of each kind the resolver tells apart.
const SHOP: [(&str, &str); 3] = [
    (
        "src/main/kotlin/com/shop/order/Order.kt",
        "package com.shop.order\n\nimport com.shop.pay.Payment\nimport com.shop.util.formatDate\n\nclass Order(val payment: Payment) {\n    fun label() = formatDate()\n    fun pay() = Payment.create(total())\n    fun total() = 1\n}\n",
    ),
    (
        "src/main/kotlin/com/shop/pay/Payment.kt",
        "package com.shop.pay\n\nclass Payment(val amount: Int) {\n    companion object {\n        fun create(amount: Int) = Payment(amount)\n    }\n}\n",
    ),
    (
        "src/main/kotlin/com/shop/util/Dates.kt",
        "package com.shop.util\n\nfun formatDate() = \"today\".uppercase()\nfun log(text: String) = println(text)\n",
    ),
];

#[test]
fn kotlin_calls_resolve_statically_and_reach_the_graph() {
    let dir = tempfile::tempdir().expect("temp dir");
    for (rel, content) in &SHOP {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().expect("parent dir")).expect("create dirs");
        fs::write(&path, content).expect("write fixture file");
    }
    let calls = archguard_engine::analyze_calls(dir.path(), &Config::default());
    assert_eq!(calls.status, "partial");
    let count = |kind: &str| calls.calls.iter().filter(|c| c.resolution == kind).count();
    // formatDate(), Payment.create(…), total(), Payment(amount); println;
    // a member call on a string literal.
    assert_eq!(
        (
            calls.calls.len(),
            count("project"),
            count("external"),
            count("unresolved")
        ),
        (6, 4, 1, 1),
        "{:#?}",
        calls.calls
    );
    // Project calls between two files become graph evidence: Order.kt →
    // Dates.kt and Order.kt → Payment.kt; `total()` and `Payment(amount)`
    // stay inside their files.
    let (graph, _) = analyze(dir.path(), &Config::default(), Level::File).expect("analyze");
    let semantic = graph.semantic.as_ref().unwrap();
    assert_eq!(
        (semantic.projected_calls, semantic.skipped_project_calls),
        (2, 0)
    );
    let found = edges(&graph);
    assert!(
        found.contains(&("Order.kt".to_string(), "Dates.kt".to_string())),
        "{found:?}"
    );
}
