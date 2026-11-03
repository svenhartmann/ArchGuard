use archguard_engine::{
    core::{Config, Graph, GraphBuilder},
    import_mutations, mutation_fingerprint, read_mutations,
};
use serde_json::{Value, json};
use std::{fs, path::Path};
const SOURCE: &str = "export const a = 1;\n";
fn project() -> (tempfile::TempDir, Config, Graph) {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("src")).unwrap();
    fs::write(root.path().join("src/A.ts"), SOURCE).unwrap();
    let mut b = GraphBuilder::new();
    b.ensure_path("A.ts", Some("ts"));
    (root, Config::default(), b.build("test".into()))
}
fn report(statuses: &[&str]) -> Value {
    json!({"schemaVersion":"2","thresholds":{"high":80,"low":60},"framework":{"name":"Stryker","version":"test"},"files":{"src/A.ts":{"language":"typescript","source":SOURCE,"mutants":statuses.iter().enumerate().map(|(i,status)|json!({"id":i.to_string(),"mutatorName":"NumberLiteral","status":status,"location":{"start":{"line":1,"column":18},"end":{"line":1,"column":19}},"replacement":"2","duration":10.5,"coveredBy":["test1"]})).collect::<Vec<_>>()}}})
}
fn import(
    root: &Path,
    config: &Config,
    graph: &Graph,
    value: Value,
) -> Result<archguard_engine::MutationReport, archguard_engine::core::ArchguardError> {
    fs::write(root.join("mutations.json"), value.to_string()).unwrap();
    import_mutations(root, config, graph, Path::new("mutations.json"), None)
}
#[test]
fn imports_measured_outcomes_snippets_duration_and_content_verified_sources() {
    let (root, config, graph) = project();
    let before = mutation_fingerprint(root.path());
    let r = import(
        root.path(),
        &config,
        &graph,
        report(&["Killed", "Survived", "NoCoverage"]),
    )
    .unwrap();
    assert_eq!(r.status, "ready");
    assert_eq!(r.files[0].node.as_deref(), Some("A.ts"));
    assert_eq!(r.files[0].status, "matches_reported_source");
    assert_eq!(r.files[0].mutants[0].original, "1");
    assert_eq!(r.files[0].mutants[0].duration_ms, Some(10.5));
    assert_eq!(r.summary.total, 3);
    assert_eq!(r.summary.survived, 1);
    assert_eq!(r.summary.no_coverage, 1);
    assert!((r.summary.killed_percent.unwrap() - 100.0 / 3.0).abs() < 1e-9);
    assert_ne!(before, mutation_fingerprint(root.path()));
    fs::remove_file(root.path().join("mutations.json")).unwrap();
    assert_eq!(read_mutations(root.path(), &config, &graph), r);
    fs::write(root.path().join("src/A.ts"), "export const a = 2;\n").unwrap();
    let stale = read_mutations(root.path(), &config, &graph);
    assert_eq!(stale.status, "stale");
    assert_eq!(stale.files[0].status, "changed_since_report");
    assert_eq!(stale.summary.killed_percent, None);
}
#[test]
fn incomplete_outcomes_and_source_mismatch_cannot_claim_full_detection() {
    let (root, config, graph) = project();
    let r = import(
        root.path(),
        &config,
        &graph,
        report(&[
            "Killed",
            "Timeout",
            "Pending",
            "RuntimeError",
            "CompileError",
            "Ignored",
        ]),
    )
    .unwrap();
    assert_eq!(r.status, "partial");
    assert_eq!(r.summary.timeouts, 1);
    assert_eq!(r.summary.pending, 1);
    assert_eq!(r.summary.runtime_errors, 1);
    assert_eq!(r.summary.killed_percent, None);
    let mut v = report(&["Killed"]);
    v["files"]["src/A.ts"]["source"] = json!("export const a = 8;\n");
    let r = import(root.path(), &config, &graph, v).unwrap();
    assert_eq!(r.status, "stale");
    assert_eq!(r.summary.killed_percent, None);
}
#[test]
fn rejects_unsupported_schema_locations_duplicates_and_traversal_atomically() {
    let (root, config, graph) = project();
    import(root.path(), &config, &graph, report(&["Killed"])).unwrap();
    let before = fs::read(root.path().join(".archguard/mutation.json")).unwrap();
    for case in 0..5 {
        let mut v = report(&["Killed", "Survived"]);
        match case {
            0 => v["schemaVersion"] = json!("3"),
            1 => v["files"]["src/A.ts"]["mutants"][0]["location"]["end"]["line"] = json!(999),
            2 => v["files"]["src/A.ts"]["mutants"][1]["id"] = json!("0"),
            3 => v["files"] = json!({"../secret.ts":v["files"]["src/A.ts"].clone()}),
            _ => v["files"]["src/A.ts"]["mutants"][0]["status"] = json!("Unknown"),
        };
        assert!(
            import(root.path(), &config, &graph, v).is_err(),
            "case {case}"
        );
        assert_eq!(
            fs::read(root.path().join(".archguard/mutation.json")).unwrap(),
            before
        );
    }
}
#[test]
fn unassigned_files_remain_visible_and_snapshot_validation_fails_closed() {
    let (root, config, graph) = project();
    let mut v = report(&["Survived"]);
    v["files"] = json!({"unscanned/A.ts":v["files"]["src/A.ts"].clone()});
    let r = import(root.path(), &config, &graph, v).unwrap();
    assert_eq!(r.status, "partial");
    assert_eq!(r.files[0].status, "unmapped");
    assert_eq!(r.files[0].node, None);
    assert_eq!(r.summary.killed_percent, None);
    let mut saved = serde_json::to_value(r).unwrap();
    saved["files"][0]["path"] = json!("../secret");
    fs::write(
        root.path().join(".archguard/mutation.json"),
        saved.to_string(),
    )
    .unwrap();
    assert_eq!(read_mutations(root.path(), &config, &graph).status, "error");
}
