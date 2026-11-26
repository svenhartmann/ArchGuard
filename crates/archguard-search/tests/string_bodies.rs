//! JSON string constants passed as body to a recognized index or template call become artifacts (ticket 27).
use archguard_core::{SearchArtifact, Service, ServiceTopology};
use archguard_flow_common::support::Discovery;
use archguard_search::is_elasticsearch_gap;
use std::path::{Path, PathBuf};

const SERVICES: [&str; 4] = ["java-shop", "cs-shop", "py-shop", "ts-shop"];

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/es-classdiagram/27")
}

fn analyze(root: &Path, services: &[&str]) -> ServiceTopology {
    let mut report = ServiceTopology::default();
    for dir in services {
        report.services.push(Service {
            id: (*dir).into(),
            name: (*dir).into(),
            compose_file: String::new(),
            build_context: Some((*dir).into()),
            networks: vec![],
            profiles: vec![],
            project: None,
        });
    }
    let discovery = Discovery::new(root, &mut report).expect("discovery");
    let read = |_: &Path, p: &Path| std::fs::read_to_string(p).ok();
    archguard_search::analyze(root, &discovery, &read, &mut report);
    report
}

fn code(report: &ServiceTopology) -> Vec<&SearchArtifact> {
    let mut all: Vec<_> = report
        .elasticsearch
        .artifacts
        .iter()
        .filter(|a| a.scope.starts_with("code:"))
        .collect();
    all.sort_by_key(|a| (a.scope.clone(), a.name.clone()));
    all
}

fn named<'a>(report: &'a ServiceTopology, name: &str) -> &'a SearchArtifact {
    code(report)
        .into_iter()
        .find(|a| a.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("no artifact {name}"))
}

#[test]
fn string_bodies_of_recognized_calls_become_artifacts_and_lookalikes_do_not() {
    let report = analyze(&fixture(), &SERVICES);
    let got: Vec<String> = code(&report)
        .iter()
        .map(|a| {
            format!(
                "{} {} {} {}",
                a.kind,
                a.name.as_deref().unwrap_or("?"),
                a.scope,
                a.flavor
            )
        })
        .collect();
    let want = [
        "index cs-orders code:cs-shop ",
        "index cs-plain code:cs-shop ",
        "index_template cs-tpl code:cs-shop ",
        "index_template blk-tpl code:java-shop ",
        "index inline-v1 code:java-shop ",
        "index invoices-v1 code:java-shop ",
        "index modern-v1 code:java-shop ",
        "index orders-v1 code:java-shop ",
        "index py-orders code:py-shop ",
        "ingest_pipeline py-pipe code:py-shop ",
        "index py-raw code:py-shop ",
        "index_template py-tpl code:py-shop ",
        "index ts-map code:ts-shop opensearch",
        "index ts-orders code:ts-shop opensearch",
        "index_template ts-tpl code:ts-shop opensearch",
    ];
    assert_eq!(
        got, want,
        "lookalikes, dict literals and unused constants are none"
    );
}

#[test]
fn an_artifact_carries_the_detail_and_both_evidence_sites() {
    let report = analyze(&fixture(), &SERVICES);
    let a = named(&report, "orders-v1");
    let d = a.detail.as_ref().expect("detail");
    let fields: Vec<_> = d
        .fields
        .iter()
        .map(|f| (f.path.as_str(), f.kind.as_str(), f.analyzer.as_deref()))
        .collect();
    assert_eq!(
        fields,
        [("sku", "keyword", None), ("title", "text", Some("folded"))]
    );
    assert_eq!(d.analysis.len(), 1);
    assert_eq!(d.analysis[0].name, "folded");
    let files: Vec<_> = a
        .evidence
        .iter()
        .map(|e| (e.file.as_str(), e.line))
        .collect();
    assert_eq!(
        files,
        [
            ("java-shop/src/main/java/demo/Setup.java", Some(7)),
            ("java-shop/src/main/java/demo/Bodies.java", Some(5)),
        ],
        "the call site and the declaration of the constant"
    );
    let t = named(&report, "blk-tpl");
    assert_eq!(t.patterns, ["blk-*"], "text blocks are read");
}

#[test]
fn what_is_not_readable_is_a_gap_with_its_line() {
    let report = analyze(&fixture(), &SERVICES);
    let mut got: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| is_elasticsearch_gap(d) && d.message.contains("body"))
        .map(|d| {
            let line = d.message.rsplit(' ').next().unwrap().trim_end_matches('.');
            format!("{}:{line}", d.file)
        })
        .collect();
    got.sort();
    let want = [
        "cs-shop/Setup.cs:19",
        "java-shop/src/main/java/demo/Setup.java:12",
        "java-shop/src/main/java/demo/Setup.java:13",
        "java-shop/src/main/java/demo/Setup.java:14",
        "py-shop/setup.py:13",
        "py-shop/setup.py:14",
        "py-shop/setup.py:15",
        "ts-shop/index.ts:12",
        "ts-shop/index.ts:13",
    ];
    assert_eq!(got, want, "composed, interpolated, invalid and non-literal");
    assert!(!report.complete);
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("archguard-27-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("svc")).unwrap();
    dir
}

#[test]
fn a_file_over_one_mib_is_a_gap_not_skipped() {
    let root = scratch("big");
    std::fs::write(
        root.join("svc/big.py"),
        format!("# {}\n", "a".repeat(1 << 20)),
    )
    .unwrap();
    let report = analyze(&root, &["svc"]);
    std::fs::remove_dir_all(&root).unwrap();
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.file == "svc/big.py" && is_elasticsearch_gap(d))
    );
}

#[test]
fn hostile_unclosed_calls_end_as_gaps_without_panic() {
    let root = scratch("hostile");
    let text = "es.indices.create(".repeat(20_000) + &"\"é".repeat(10);
    std::fs::write(root.join("svc/hostile.py"), text).unwrap();
    let report = analyze(&root, &["svc"]);
    std::fs::remove_dir_all(&root).unwrap();
    assert!(code(&report).is_empty());
}
