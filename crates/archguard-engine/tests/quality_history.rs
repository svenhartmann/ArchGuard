use archguard_engine::{
    build_quality_snapshot, compare_quality_snapshots,
    core::{CallAnalysis, Config, Graph, GraphBuilder},
    import_quality_findings, import_test_coverage, quality_history_fingerprint,
    read_quality_history, record_quality_snapshot,
};
use std::{fs, path::Path};

fn setup() -> (tempfile::TempDir, Config, Graph, CallAnalysis) {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("src/pkg")).unwrap();
    fs::write(
        root.path().join("src/pkg/A.java"),
        "package pkg;\npublic class A { int f() { return 1; } }\n",
    )
    .unwrap();
    let mut builder = GraphBuilder::new();
    builder.ensure_path("pkg/A.java", Some("java"));
    let calls=serde_json::from_value(serde_json::json!({"status":"ready","files":1,"analyzers":[{"language":"java","backend":"javac","status":"ready","files":1,"version":"21","project":null,"diagnostics":[]}],"classes":[{"language":"java","id":"pkg.A","name":"A","package":"pkg","file":"src/pkg/A.java","graph_node":"pkg/A.java","line":2,"kind":"class"}],"methods":[{"id":"pkg.A#f()","class_id":"pkg.A","name":"f","signature":"f()","file":"src/pkg/A.java","line":2,"end_line":2,"is_static":false,"complexity":{"cyclomatic":2,"max_nesting":1,"definition":"ast-decisions-v1"}}],"calls":[],"diagnostics":[]})).unwrap();
    let graph = builder.build(root.path().to_string_lossy().into_owned());
    (root, Config::default(), graph, calls)
}
#[test]
fn extended_history_tracks_class_metrics_and_package_cycles_without_inventing_legacy_values() {
    let (root, config, mut graph, mut calls) = setup();
    calls.details=Some(serde_json::from_value(serde_json::json!({"definition":"source-types-state/v1","type_uses":[],"class_states":[{"class_id":"pkg.A","status":"ready","fields":[],"accesses":[]}],"method_metrics":[{"method_id":"pkg.A#f()","definition":"source-maintainability/v1","cognitive":3,"distinct_operators":2,"distinct_operands":2,"operators":3,"operands":5,"source_lines":2}],"inheritance":[{"class_id":"pkg.A","base_class":null,"depth":0,"status":"ready"}]})).unwrap());
    let (c, q) = measurements(root.path(), &config, &graph);
    let before = build_quality_snapshot(root.path(), &config, &graph, &calls, &c, &q).unwrap();
    let v = serde_json::to_value(&before).unwrap();
    assert_eq!(v["modules"][0]["extended"]["lcom4.mean"]["value"], 1.0);
    assert_eq!(v["modules"][0]["extended"]["cognitive.max"]["value"], 3.0);
    assert_eq!(
        v["modules"][0]["extended"]["package.cyclic_dependencies"]["value"],
        0.0
    );
    calls.details.as_mut().unwrap().method_metrics[0].cognitive = 1;
    let after = build_quality_snapshot(root.path(), &config, &graph, &calls, &c, &q).unwrap();
    let delta = serde_json::to_value(compare_quality_snapshots(&before, &after)).unwrap();
    assert_eq!(
        delta["modules"][0]["extended"]["cognitive.max"]["delta"],
        -2.0
    );
    let mut legacy = before.clone();
    legacy.modules[0].extended.clear();
    let delta = serde_json::to_value(compare_quality_snapshots(&legacy, &after)).unwrap();
    assert!(delta["modules"][0]["extended"]["cognitive.max"]["delta"].is_null());
    let mut b = GraphBuilder::new();
    let a = b.ensure_path("pkg/A.java", Some("java"));
    let x = b.ensure_path("other/X.java", Some("java"));
    fs::create_dir(root.path().join("other")).unwrap();
    fs::write(
        root.path().join("other/X.java"),
        "package other; class X {}",
    )
    .unwrap();
    for (from, to) in [(a, x), (x, a)] {
        b.edge(
            from,
            to,
            archguard_engine::core::Ref {
                file: "src/pkg/A.java".into(),
                line: 2,
                raw: "cycle".into(),
            },
        )
        .unwrap();
    }
    graph = b.build(root.path().to_string_lossy().into_owned());
    let cycled = build_quality_snapshot(root.path(), &config, &graph, &calls, &c, &q).unwrap();
    let pkg = cycled.modules.iter().find(|m| m.node == "pkg").unwrap();
    assert_eq!(
        serde_json::to_value(pkg).unwrap()["extended"]["package.cyclic_dependencies"]["value"],
        1.0
    );
}
fn measurements(
    root: &Path,
    config: &Config,
    graph: &Graph,
) -> (
    archguard_engine::TestCoverageReport,
    archguard_engine::QualityReport,
) {
    fs::write(root.join("coverage.xml"),r#"<report name="unit"><package name="pkg"><sourcefile name="A.java"><line nr="1" ci="1" mi="0" mb="0" cb="0"/><line nr="2" ci="0" mi="1" mb="0" cb="0"/><counter type="LINE" missed="1" covered="1"/></sourcefile></package></report>"#).unwrap();
    fs::write(root.join("quality.sarif"),r#"{"version":"2.1.0","runs":[{"tool":{"driver":{"name":"lint","version":"1"}},"results":[{"ruleId":"R1","level":"warning","message":{"text":"review"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"src/pkg/A.java"},"region":{"startLine":2}}}]}]}]}"#).unwrap();
    (
        import_test_coverage(root, config, graph, Path::new("coverage.xml"), None).unwrap(),
        import_quality_findings(root, config, graph, Path::new("quality.sarif"), None).unwrap(),
    )
}

#[test]
fn snapshot_records_real_measurements_and_never_claims_producer_revision_verification() {
    let (root, config, graph, calls) = setup();
    let (coverage, quality) = measurements(root.path(), &config, &graph);
    let snapshot =
        build_quality_snapshot(root.path(), &config, &graph, &calls, &coverage, &quality).unwrap();
    assert_eq!(snapshot.source_sha256.len(), 64);
    assert_eq!(snapshot.producer_revision, "unverified");
    assert_eq!(
        serde_json::to_value(&snapshot).unwrap()["analyzers"][0]["version"],
        "21"
    );
    let m = &snapshot.modules[0];
    assert_eq!(m.node, "pkg");
    assert_eq!(m.files, 1);
    assert_eq!(m.mapped_files, 1);
    assert_eq!(m.line_coverage.as_ref().unwrap().covered, 1);
    assert_eq!(m.mean_complexity, Some(2.0));
    assert_eq!(m.active_findings, Some(1));
    assert_eq!(m.coverage_status, "ready");
    assert_eq!(m.semantic_status, "ready");
    fs::write(
        root.path().join("src/pkg/A.java"),
        "package pkg;\npublic class A { int f() { return 5; } }\n",
    )
    .unwrap();
    let next =
        build_quality_snapshot(root.path(), &config, &graph, &calls, &coverage, &quality).unwrap();
    assert_ne!(
        next.source_sha256, snapshot.source_sha256,
        "source identity hashes contents"
    );
    assert_eq!(
        next.modules[0].coverage_status, "stale",
        "freshness verified even if caller retained an older report object"
    );
}

#[test]
fn comparison_separates_compiler_deltas_from_unverified_imports_and_incompatible_config() {
    let (root, config, graph, calls) = setup();
    let (c, q) = measurements(root.path(), &config, &graph);
    let before = build_quality_snapshot(root.path(), &config, &graph, &calls, &c, &q).unwrap();
    let mut after = before.clone();
    after.timestamp += 1;
    after.modules[0].mean_complexity = Some(1.0);
    after.modules[0].max_complexity = Some(1);
    after.modules[0].line_coverage.as_mut().unwrap().covered = 2;
    after.modules[0].active_findings = Some(0);
    let comparison = compare_quality_snapshots(&before, &after);
    assert_eq!(comparison.modules[0].mean_complexity.status, "comparable");
    assert_eq!(comparison.modules[0].mean_complexity.delta, Some(-1.0));
    assert_eq!(comparison.modules[0].line_coverage.status, "unverified");
    assert_eq!(comparison.modules[0].line_coverage.delta, Some(50.0));
    assert_eq!(comparison.modules[0].active_findings.status, "unverified");
    after.config_sha256 = "changed".into();
    let changed = compare_quality_snapshots(&before, &after);
    assert_eq!(changed.modules[0].mean_complexity.status, "incomparable");
    assert_eq!(changed.modules[0].mean_complexity.delta, None);
    assert_eq!(changed.modules[0].line_coverage.delta, None);
}

#[test]
fn immutable_records_read_latest_pair_and_missing_history_creates_nothing() {
    let (root, config, graph, calls) = setup();
    assert_eq!(read_quality_history(root.path()).status, "not_recorded");
    assert!(!root.path().join(".archguard").exists());
    let (c, q) = measurements(root.path(), &config, &graph);
    let snapshot = build_quality_snapshot(root.path(), &config, &graph, &calls, &c, &q).unwrap();
    let fingerprint = quality_history_fingerprint(root.path());
    let first = record_quality_snapshot(root.path(), &snapshot, Some("../label")).unwrap();
    let second = record_quality_snapshot(root.path(), &snapshot, Some("second")).unwrap();
    assert_ne!(first, second);
    assert!(first.exists());
    assert!(second.exists());
    assert_ne!(fingerprint, quality_history_fingerprint(root.path()));
    let history = read_quality_history(root.path());
    assert_eq!(history.status, "ready");
    assert_eq!(history.records.len(), 2);
    let value = serde_json::to_value(&history).unwrap();
    assert_eq!(
        value["series"]
            .as_array()
            .expect("recorded measurements need a timeline")
            .len(),
        2
    );
    assert_eq!(
        value["series"][0]["modules"][0]["metrics"]["mean_complexity"],
        2.0
    );
    assert_eq!(history.latest.unwrap().tag.as_deref(), Some("second"));
    assert!(history.comparison.is_some());
}

#[test]
fn corrupt_records_are_reported_and_symlinked_storage_is_not_followed() {
    let (root, config, graph, calls) = setup();
    let (c, q) = measurements(root.path(), &config, &graph);
    let snapshot = build_quality_snapshot(root.path(), &config, &graph, &calls, &c, &q).unwrap();
    record_quality_snapshot(root.path(), &snapshot, None).unwrap();
    fs::write(
        root.path().join(".archguard/quality-history/bad.json"),
        "{}",
    )
    .unwrap();
    let history = read_quality_history(root.path());
    assert_eq!(history.status, "partial");
    assert_eq!(history.records.len(), 1);
    assert!(!history.diagnostics.is_empty());
    #[cfg(unix)]
    {
        let other = tempfile::tempdir().unwrap();
        let symlinked = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(other.path(), symlinked.path().join(".archguard")).unwrap();
        assert!(record_quality_snapshot(symlinked.path(), &snapshot, None).is_err());
        assert_eq!(read_quality_history(symlinked.path()).status, "error");
        assert_eq!(fs::read_dir(other.path()).unwrap().count(), 0);
    }
}

#[test]
fn unanalyzed_package_files_do_not_inherit_another_files_ready_compiler_status() {
    let (root, config, _, calls) = setup();
    fs::write(
        root.path().join("src/pkg/B.java"),
        "package pkg; public class B {}\n",
    )
    .unwrap();
    let mut builder = GraphBuilder::new();
    builder.ensure_path("pkg/A.java", Some("java"));
    builder.ensure_path("pkg/B.java", Some("java"));
    let graph = builder.build(root.path().to_string_lossy().into_owned());
    let (coverage, quality) = measurements(root.path(), &config, &graph);
    let snapshot =
        build_quality_snapshot(root.path(), &config, &graph, &calls, &coverage, &quality).unwrap();
    assert_eq!(
        snapshot.modules[0].semantic_status, "partial",
        "ready analyzer alone does not prove membership of B.java"
    );
    let comparison = compare_quality_snapshots(&snapshot, &snapshot);
    assert_eq!(comparison.modules[0].mean_complexity.status, "incomparable");
    assert_eq!(comparison.modules[0].line_coverage.status, "incomparable");
}

#[test]
fn package_without_coverage_does_not_inherit_another_packages_ready_report_status() {
    let (root, config, _, calls) = setup();
    fs::create_dir_all(root.path().join("src/unmeasured")).unwrap();
    fs::write(
        root.path().join("src/unmeasured/B.java"),
        "package unmeasured; public class B {}\n",
    )
    .unwrap();
    let mut builder = GraphBuilder::new();
    builder.ensure_path("pkg/A.java", Some("java"));
    builder.ensure_path("unmeasured/B.java", Some("java"));
    let graph = builder.build(root.path().to_string_lossy().into_owned());
    let (coverage, quality) = measurements(root.path(), &config, &graph);
    assert_eq!(coverage.status, "ready", "the imported report is valid");
    let snapshot =
        build_quality_snapshot(root.path(), &config, &graph, &calls, &coverage, &quality).unwrap();
    let measured = snapshot.modules.iter().find(|m| m.node == "pkg").unwrap();
    assert_eq!(measured.coverage_status, "ready");
    let absent = snapshot
        .modules
        .iter()
        .find(|m| m.node == "unmeasured")
        .unwrap();
    assert_eq!(absent.files, 1);
    assert_eq!(absent.coverage_files, 0);
    assert_eq!(
        absent.line_coverage, None,
        "missing coverage is unknown, not zero"
    );
    assert_eq!(absent.coverage_status, "partial");
    let comparison = compare_quality_snapshots(&snapshot, &snapshot);
    let absent_delta = comparison
        .modules
        .iter()
        .find(|m| m.node == "unmeasured")
        .unwrap();
    assert_eq!(absent_delta.line_coverage.status, "incomparable");
    assert_eq!(absent_delta.line_coverage.delta, None);
}

#[test]
fn history_overview_is_bounded_without_removing_full_records_and_tool_changes_block_deltas() {
    let (root, config, graph, calls) = setup();
    let (coverage, quality) = measurements(root.path(), &config, &graph);
    let snapshot =
        build_quality_snapshot(root.path(), &config, &graph, &calls, &coverage, &quality).unwrap();
    for i in 0..23 {
        record_quality_snapshot(root.path(), &snapshot, Some(&i.to_string())).unwrap();
    }
    let history = read_quality_history(root.path());
    assert_eq!(history.records.len(), 20);
    assert!(history.records_truncated);
    assert_eq!(history.records[0].tag.as_deref(), Some("22"));
    assert_eq!(
        fs::read_dir(root.path().join(".archguard/quality-history"))
            .unwrap()
            .count(),
        23
    );
    let mut changed = snapshot.clone();
    changed.analyzer_signature = "changed".into();
    changed.quality_tools = vec!["another analyzer".into()];
    assert_eq!(
        compare_quality_snapshots(&snapshot, &changed).modules[0]
            .mean_complexity
            .delta,
        None
    );
    assert_eq!(
        compare_quality_snapshots(&snapshot, &changed).modules[0]
            .active_findings
            .delta,
        None
    );
}
