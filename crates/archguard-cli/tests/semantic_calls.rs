use assert_cmd::Command;
use serde_json::Value;

#[test]
fn calls_command_reports_kotlin_token_measurements_as_partial() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("Main.kt"),
        "fun main(flag: Boolean) {\n    if (flag) println(\"hello\")\n}\n",
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
    // Calls stay unresolved: partial coverage and no call edges, never a
    // clean, complete call graph.
    assert_eq!(result["status"], "partial");
    assert_eq!(result["analyzers"][0]["language"], "kotlin");
    assert_eq!(result["analyzers"][0]["backend"], "archguard tokens");
    assert_eq!(result["analyzers"][0]["files"], 1);
    assert_eq!(result["calls"].as_array().unwrap().len(), 0);
    assert_eq!(result["methods"][0]["name"], "main");
    assert_eq!(result["methods"][0]["complexity"]["cyclomatic"], 2);
}

#[test]
fn calls_command_reports_python_syntax_measurements_as_partial() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("main.py"),
        "def main(flag):\n    if flag:\n        print('hello')\n",
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
    // Syntax only: measured functions, but no resolved calls are claimed.
    assert_eq!(result["status"], "partial");
    assert_eq!(result["analyzers"][0]["language"], "python");
    assert_eq!(result["methods"][0]["name"], "main");
    assert_eq!(result["methods"][0]["complexity"]["cyclomatic"], 2);
    assert_eq!(result["calls"].as_array().unwrap().len(), 0);
}
