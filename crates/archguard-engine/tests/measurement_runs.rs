use archguard_engine::{import_measurement_runs, read_measurement_runs};
use serde_json::json;
use std::{fs, path::Path};
fn sample(id: &str, kind: &str, outcome: &str, seconds: f64, revision: &str) -> serde_json::Value {
    json!({"run_id":id,"kind":kind,"name":"suite","revision":revision,"environment":"linux-ci","producer":"runner/1","timestamp":100,"outcome":outcome,"duration_seconds":seconds})
}
#[test]
fn repeated_runs_measure_timing_and_test_instability_only_with_matching_context() {
    let root = tempfile::tempdir().unwrap();
    let revision = "a".repeat(40);
    assert_eq!(read_measurement_runs(root.path()).status, "not_imported");
    assert!(!root.path().join(".archguard").exists());
    let input = json!({"schema_version":1,"runs":[sample("one","test","failed",2.0,&revision),sample("two","test","passed",1.0,&revision),sample("build","build","passed",12.0,&revision),sample("r1","runtime","passed",0.1,&revision),sample("r2","runtime","passed",0.3,&revision),sample("other","test","passed",1.0,&"b".repeat(40))]});
    fs::write(root.path().join("runs.json"), input.to_string()).unwrap();
    let report = import_measurement_runs(root.path(), Path::new("runs.json")).unwrap();
    let value = serde_json::to_value(&report).unwrap();
    assert_eq!(value["flaky_test_percent"], 100.0);
    let tests = value["groups"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["kind"] == "test")
        .collect::<Vec<_>>();
    assert_eq!(
        tests.len(),
        2,
        "different revisions never get blended into flakiness"
    );
    assert_eq!(tests[0]["inconsistent"], true);
    assert_eq!(
        tests[1]["inconsistent"],
        serde_json::Value::Null,
        "single observations cannot measure stability"
    );
    let runtime = value["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["kind"] == "runtime")
        .unwrap();
    assert_eq!(runtime["p95_seconds"], 0.3);
    assert_eq!(runtime["median_seconds"], 0.2);
    assert_eq!(
        import_measurement_runs(root.path(), Path::new("runs.json")).unwrap(),
        report,
        "reimport is idempotent"
    );
    assert_eq!(read_measurement_runs(root.path()), report);
    let stored = fs::read(root.path().join(".archguard/measurement-runs.json")).unwrap();
    let mut conflict = input;
    conflict["runs"][0]["outcome"] = json!("passed");
    fs::write(root.path().join("runs.json"), conflict.to_string()).unwrap();
    assert!(import_measurement_runs(root.path(), Path::new("runs.json")).is_err());
    assert_eq!(
        fs::read(root.path().join(".archguard/measurement-runs.json")).unwrap(),
        stored,
        "conflicting history cannot overwrite evidence"
    );
}
#[test]
fn invalid_runs_and_missing_revisions_cannot_produce_scores() {
    let root = tempfile::tempdir().unwrap();
    for row in [
        sample("bad", "test", "passed", -1.0, &"a".repeat(40)),
        sample("bad", "test", "passed", 1.0, ""),
        sample("bad", "unknown", "passed", 1.0, &"a".repeat(40)),
    ] {
        fs::write(
            root.path().join("runs.json"),
            json!({"schema_version":1,"runs":[row]}).to_string(),
        )
        .unwrap();
        assert!(import_measurement_runs(root.path(), Path::new("runs.json")).is_err());
    }
    assert!(!root.path().join(".archguard").exists());
}
