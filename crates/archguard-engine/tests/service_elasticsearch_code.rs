//! Index names a service's Java and C# source names literally, with access (ADR 0023).
use archguard_engine::{analyze_services, core::ServiceTopology};
use std::{fs, path::Path};

fn put(root: &Path, path: &str, text: &str) {
    let p = root.join(path);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, text).unwrap();
}

/// (index, access) of one service, sorted as reported.
fn usages(report: &ServiceTopology, service: &str) -> Vec<(String, String)> {
    report
        .elasticsearch
        .usages
        .iter()
        .filter(|u| u.service.ends_with(service))
        .map(|u| (u.index.clone(), u.access.clone()))
        .collect()
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
}

fn gaps(report: &ServiceTopology) -> usize {
    report
        .diagnostics
        .iter()
        .filter(|d| d.message.starts_with("Index name is not a literal"))
        .count()
}

#[test]
fn fixture_lists_java_and_csharp_indices_with_access() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/elasticsearch-services");
    let report = analyze_services(&root).unwrap();
    assert_eq!(
        usages(&report, "java-writer"),
        pairs(&[
            ("audit-log", "write"),
            ("hdr", "read"),
            ("legacy-1", "write"),
            ("misc", "unknown"),
            ("orders-archive", "read"),
            ("orders-v1", "read"),
            ("orders-v1", "write"),
        ])
    );
    assert_eq!(
        usages(&report, "dotnet-reader"),
        pairs(&[
            ("audit-log", "write"),
            ("default-ix", "unknown"),
            ("named", "read"),
            ("orders-v1", "read"),
            ("reader-out", "write"),
            ("stats", "read"),
        ])
    );
    assert_eq!(gaps(&report), 0);
    assert!(report.complete);
    let first = report
        .elasticsearch
        .usages
        .iter()
        .find(|u| u.index == "legacy-1")
        .unwrap();
    assert!(first.evidence[0].file.ends_with("shop/Writer.java"));
    assert_eq!(first.evidence[0].line, Some(11));
}

#[test]
fn non_literals_are_gaps_and_tests_and_foreign_files_do_not_count() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    put(
        root,
        "compose.yaml",
        "services:\n  api:\n    build: ./api\n",
    );
    put(
        root,
        "api/src/Main.java",
        "import org.elasticsearch.client.Client;\nclass M {\n  void f() {\n    var r = new SearchRequest.Builder().index(Names.ORDERS);\n    var s = new SearchRequest.Builder().index(\"fine\");\n  }\n}\n",
    );
    // The same gap text in one file is reported once, so the second gap sits in its own file.
    put(
        root,
        "api/src/Second.java",
        "import org.elasticsearch.client.Client;\nclass S { var r = new SearchRequest.Builder().index(\"a\" + suffix); }\n",
    );
    put(
        root,
        "api/src/test/java/MainTest.java",
        "import org.elasticsearch.client.Client;\nclass T { var r = new SearchRequest.Builder().index(\"in-test\"); }\n",
    );
    put(
        root,
        "api/src/Other.java",
        "class O { var r = list.index(\"no-import\"); }\n",
    );
    let report = analyze_services(root).unwrap();
    assert_eq!(usages(&report, "api"), pairs(&[("fine", "read")]));
    assert_eq!(gaps(&report), 2);
    assert!(!report.complete);
}
