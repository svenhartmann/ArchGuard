use assert_cmd::Command;
use serde_json::Value;

#[test]
fn calls_command_reports_unsupported_language_without_inventing_a_clean_graph() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("main.py"),
        "def main():\n    print('hello')\n",
    )
    .unwrap();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("archguard"))
        .args(["calls", root.path().to_str().unwrap(), "--format", "json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["status"], "unavailable");
    assert_eq!(result["analyzers"][0]["language"], "python");
    assert_eq!(result["analyzers"][0]["files"], 1);
    assert_eq!(result["calls"].as_array().unwrap().len(), 0);
}
