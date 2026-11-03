use std::{fs, process::Command};
#[test]
fn metrics_cli_imports_observations_and_exposes_code_and_git_coverage() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("A.java"),
        "class A { int x; int run(int n){ if(n>0)return x;return n;} }",
    )
    .unwrap();
    fs::write(root.path().join("runs.json"),serde_json::json!({"schema_version":1,"runs":[{"run_id":"one","kind":"build","name":"compile","revision":"a".repeat(40),"environment":"local","producer":"timer/1","timestamp":100,"outcome":"passed","duration_seconds":12.5}]}).to_string()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_archguard"))
        .args([
            "metrics",
            root.path().to_str().unwrap(),
            "--extended",
            "--report",
            "runs.json",
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["measurements"]["groups"][0]["median_seconds"], 12.5);
    assert_eq!(value["code"]["definition"], "source-metrics/v1");
    assert_eq!(value["knowledge"]["status"], "unavailable");
    assert_eq!(
        value["code"]["methods"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["method_id"].as_str().unwrap().contains("#run("))
            .unwrap()["cognitive"],
        1
    );
    let html = Command::new(env!("CARGO_BIN_EXE_archguard"))
        .args(["graph", root.path().to_str().unwrap(), "--format", "html"])
        .output()
        .unwrap();
    assert!(
        html.status.success(),
        "{}",
        String::from_utf8_lossy(&html.stderr)
    );
    let html = String::from_utf8(html.stdout).unwrap();
    assert!(
        html.contains("code_metrics")
            && html.contains("measurement_runs")
            && html.contains("knowledge_metrics")
    );
}
