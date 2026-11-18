//! Scala joins the JVM package namespace with Java and Kotlin: imports and
//! qualified names resolve across the languages in one graph, and the
//! token backend resolves Scala call targets statically (ADR 0020).

use std::collections::BTreeSet;
use std::fs;

use archguard_core::Config;
use archguard_engine::{Level, analyze, analyze_calls};

const FIXTURE: [(&str, &str); 5] = [
    (
        "src/main/scala/com/shop/order/Order.scala",
        "package com.shop\npackage order\n\nimport com.shop.pay.Payment\nimport com.shop.util.{formatDate, Clock => C}\n\n\
         final case class Order(payment: Payment):\n  def label: String = formatDate(C.now())\n",
    ),
    (
        "src/main/java/com/shop/pay/Payment.java",
        "package com.shop.pay;\n\nimport com.shop.order.Order;\n\npublic class Payment {\n    public enum Status { OPEN }\n    Order order;\n}\n",
    ),
    (
        "src/main/scala/com/shop/util/Dates.scala",
        "package com.shop.util\n\ndef formatDate(at: Long): String = \"today\"\n\nobject Clock:\n  def now(): Long = 0L\n",
    ),
    (
        "src/main/scala/com/shop/report/Report.scala",
        "package com.shop.report\n\nimport com.shop.pay.Payment.Status\nimport scala.util.Try\n\n\
         abstract class Base { def close(): Unit = () }\n\n\
         class Report extends Base {\n  def open = Status.OPEN\n  def make(p: com.shop.pay.Payment) = new com.shop.order.Order(p)\n\
         \x20 def again(p: com.shop.pay.Payment) = com.shop.order.Order(p)\n\
         \x20 def done(): Unit = { this.close(); super.close(); Try(1); helper(2) }\n\
         \x20 private def helper(n: Int) = n\n}\n",
    ),
    (
        "src/main/kotlin/com/shop/note/Note.kt",
        "package com.shop.note\n\nimport com.shop.report.Report\n\nclass Note(val report: Report)\n",
    ),
];

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    for (rel, content) in &FIXTURE {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().expect("parent dir")).expect("create dirs");
        fs::write(&path, content).expect("write fixture file");
    }
    dir
}

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
fn scala_java_and_kotlin_resolve_each_other() {
    let dir = project();
    let (graph, _) = analyze(dir.path(), &Config::default(), Level::File).expect("analyze");
    let pair = |from: &str, to: &str| (from.to_string(), to.to_string());
    let found = edges(&graph);
    for expected in [
        // Scala → Java and Java → Scala through plain imports; the chained
        // package clauses name the module `com/shop/order`.
        pair("Order.scala", "Payment.java"),
        pair("Payment.java", "Order.scala"),
        // A nested type resolves to the file of its outer class.
        pair("Report.scala", "Payment.java"),
        // A qualified name in code links without an import.
        pair("Report.scala", "Order.scala"),
        // A top-level def has no file of its name: the package stands in.
        pair("Order.scala", "util/*"),
        // Kotlin → Scala.
        pair("Note.kt", "Report.scala"),
    ] {
        assert!(
            found.contains(&expected),
            "missing {expected:?} in {found:?}"
        );
    }
    assert!(graph.nodes.iter().any(|node| {
        node.name.ends_with("com/shop/order/Order.scala") && node.lang.as_deref() == Some("scala")
    }));
}

#[test]
fn scala_call_targets_resolve_statically() {
    let dir = project();
    let report = analyze_calls(dir.path(), &Config::default());
    let scala = report
        .analyzers
        .iter()
        .find(|a| a.language == "scala")
        .expect("scala analyzer");
    assert_eq!((scala.status.as_str(), scala.files), ("partial", 3));
    let call = |label: &str| {
        report
            .calls
            .iter()
            .find(|c| c.target_label == label)
            .unwrap_or_else(|| panic!("no call {label}: {:?}", report.calls))
    };
    let target = |label: &str| {
        let c = call(label);
        (
            c.resolution.as_str(),
            c.dispatch.as_str(),
            // Ids start with the file path; its last segment is enough here.
            c.to_method
                .as_deref()
                .or(c.to_class.as_deref())
                .map(|id| id.rsplit('/').next().unwrap_or(id))
                .unwrap_or("")
                .to_string(),
        )
    };
    let dates = "Dates.scala";
    let report_file = "Report.scala";
    let order = "Order.scala::Order";
    assert_eq!(
        target("formatDate"),
        (
            "project",
            "static",
            format!("{dates}::(module)#formatDate@3")
        )
    );
    // An import alias binds the object.
    assert_eq!(
        target("C.now"),
        ("project", "static", format!("{dates}::Clock#now@6"))
    );
    assert_eq!(
        target("new com.shop.order.Order"),
        ("project", "constructor", order.to_string())
    );
    // A case class without an own `apply` is built by its constructor.
    assert_eq!(
        target("com.shop.order.Order"),
        ("project", "constructor", order.to_string())
    );
    assert_eq!(
        target("this.close"),
        (
            "project",
            "instance",
            format!("{report_file}::Base#close@6")
        )
    );
    assert_eq!(
        target("super.close"),
        (
            "project",
            "instance",
            format!("{report_file}::Base#close@6")
        )
    );
    assert_eq!(
        target("helper"),
        (
            "project",
            "instance",
            format!("{report_file}::Report#helper@13")
        )
    );
    // Imported from outside the project: external, never guessed.
    assert_eq!(
        target("Try"),
        // The report prefixes ids with the language.
        ("external", "unknown", "scala:scala.util.Try".into())
    );
}
