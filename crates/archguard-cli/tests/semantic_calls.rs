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
    // Static resolution: builtins are external, nothing is guessed.
    assert_eq!(result["status"], "partial");
    assert_eq!(result["analyzers"][0]["language"], "python");
    assert_eq!(result["methods"][0]["name"], "main");
    assert_eq!(result["methods"][0]["complexity"]["cyclomatic"], 2);
    assert_eq!(result["calls"].as_array().unwrap().len(), 1);
    assert_eq!(result["calls"][0]["resolution"], "external");
}

#[test]
fn calls_command_resolves_php_calls_statically() {
    let root = tempfile::tempdir().unwrap();
    for (path, source) in [
        (
            "src/Shop/Cart.php",
            "<?php\nnamespace Shop;\n\nuse Shop\\Price as P;\n\nclass Cart {\n    public function total($items) {\n        $this->check();\n        $items->sum();\n        return P::of(count($items));\n    }\n    private function check() {}\n}\n",
        ),
        (
            "src/Shop/Price.php",
            "<?php\nnamespace Shop;\n\nclass Price {\n    public static function of($n) { return new static(); }\n}\n",
        ),
    ] {
        let path = root.path().join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, source).unwrap();
    }
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
    // Types stay unresolved, so coverage stays partial.
    assert_eq!(result["status"], "partial");
    assert_eq!(result["analyzers"][0]["language"], "php");
    let calls = result["calls"].as_array().unwrap();
    let count = |kind: &str| calls.iter().filter(|c| c["resolution"] == kind).count();
    // $this->check(), P::of(), new static(); count(); $items->sum().
    assert_eq!(
        (
            calls.len(),
            count("project"),
            count("external"),
            count("unresolved")
        ),
        (5, 3, 1, 1),
        "{calls:#?}"
    );
    let of = calls
        .iter()
        .find(|c| c["code"] == "P::of(count($items))")
        .unwrap();
    // Combined reports prefix ids with their language.
    assert_eq!(of["to_class"], "php:Shop/Price.php::Price");
    assert_eq!(of["dispatch"], "static");
    assert!(
        result["analyzers"][0]["diagnostics"][0]
            .as_str()
            .unwrap()
            .starts_with("Calls: 5 sites, 3 project, 1 external, 1 unresolved")
    );
}
