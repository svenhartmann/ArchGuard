use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};
fn fixture(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("archguard.toml"),"derive_layers=false\n[project]\nlanguages=['ts']\nsource_roots=['src']\nexclude=[]\n[rules]\ndeny_cycles='off'\n").unwrap();
    for (name, source) in files {
        let path = dir.path().join("src").join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }
    dir
}
fn run(root: &Path, args: &[&str]) -> Value {
    let output = Command::new(assert_cmd::cargo::cargo_bin!("archguard"))
        .arg("patterns")
        .arg(root)
        .args(args)
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn pattern<'a>(report: &'a Value, name: &str) -> &'a Value {
    report["assessments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["pattern"] == name)
        .unwrap()
}
#[test]
fn patterns_explain_layered_and_pipeline_candidates_without_creating_policy() {
    let root = fixture(&[
        ("web/page.ts", "import '../application/service';\n"),
        ("application/service.ts", "import '../persistence/store';\n"),
        ("persistence/store.ts", "export const data=1;\n"),
    ]);
    let before = fs::read(root.path().join("archguard.toml")).unwrap();
    let report = run(root.path(), &[]);
    assert_eq!(pattern(&report, "layered")["status"], "candidate");
    assert_eq!(pattern(&report, "pipeline")["status"], "candidate");
    assert_eq!(
        pattern(&report, "layered")["metrics"]["forward_dependencies"],
        2
    );
    assert!(
        pattern(&report, "layered")["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["file"] == "src/web/page.ts" && e["line"] == 1)
    );
    assert!(
        !pattern(&report, "microservices")["missing_evidence"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        fs::read(root.path().join("archguard.toml")).unwrap(),
        before
    );
    assert!(
        !root.path().join(".archguard/patterns.json").exists(),
        "analysis is not a policy or imported snapshot"
    );
    fs::write(
        root.path().join("src/persistence/store.ts"),
        "import '../web/page';\n",
    )
    .unwrap();
    let cyclic = run(root.path(), &[]);
    assert_eq!(pattern(&cyclic, "layered")["status"], "mixed");
    assert_eq!(
        pattern(&cyclic, "layered")["id"],
        pattern(&report, "layered")["id"]
    );
    assert!(
        !pattern(&cyclic, "layered")["counter_evidence"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_ne!(pattern(&cyclic, "pipeline")["status"], "candidate");
}
#[test]
fn names_and_an_arbitrary_dag_do_not_prove_an_architecture_pattern() {
    let root = fixture(&[
        ("a.ts", "import './b';\n"),
        ("b.ts", "import './c';\n"),
        ("c.ts", "export const c=1;\n"),
        ("domain/FakePort.ts", "export interface FakePort {}\n"),
    ]);
    let report = run(root.path(), &[]);
    assert_eq!(
        pattern(&report, "layered")["status"],
        "insufficient_evidence"
    );
    assert_eq!(
        pattern(&report, "ports_and_adapters")["status"],
        "insufficient_evidence"
    );
    assert_eq!(report["coverage"]["import_resolution"], "not_measured");
    assert!(
        report["limitations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v.as_str().unwrap().contains("intent"))
    );
}
#[test]
fn asyncapi_contracts_keep_direction_and_unknown_addresses_without_inventing_service_links() {
    let root = fixture(&[("a.ts", "export const a=1;\n")]);
    assert_eq!(
        pattern(&run(root.path(), &[]), "event_driven")["status"],
        "insufficient_evidence"
    );
    fs::write(root.path().join("asyncapi.json"),json!({"asyncapi":"3.0.0","info":{"title":"Messaging","version":"1"},"channels":{"orders":{"address":"orders.created"},"dynamic":{}},"operations":{"emit":{"action":"send","channel":{"$ref":"#/channels/orders"}},"receive":{"action":"receive","channel":{"$ref":"#/channels/dynamic"}}}}).to_string()).unwrap();
    let report = run(root.path(), &[]);
    let event = pattern(&report, "event_driven");
    assert_eq!(event["status"], "candidate");
    assert_eq!(event["metrics"]["send_operations"], 1);
    assert_eq!(event["metrics"]["receive_operations"], 1);
    assert!(event["services"].as_array().unwrap().is_empty());
    assert!(
        event["missing_evidence"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m.as_str().unwrap().contains("event"))
    );
    fs::write(root.path().join("asyncapi.json"),json!({"asyncapi":"3.0.0","operations":{"remote":{"$ref":"https://invalid.example/operation"}}}).to_string()).unwrap();
    let bad = run(root.path(), &[]);
    assert_eq!(
        pattern(&bad, "event_driven")["status"],
        "insufficient_evidence"
    );
    assert_eq!(bad["coverage"]["asyncapi_status"], "partial");
    assert!(
        bad["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d.as_str().unwrap().contains("reference"))
    );
}

#[test]
fn standalone_pattern_report_contains_the_same_assessment_and_ui() {
    let root = fixture(&[
        ("web/a.ts", "import '../application/b';\n"),
        ("application/b.ts", "import '../data/c';\n"),
        ("data/c.ts", "export const c=1;\n"),
    ]);
    let report = run(root.path(), &[]);
    let output = Command::new(assert_cmd::cargo::cargo_bin!("archguard"))
        .arg("patterns")
        .arg(root.path())
        .args(["--format", "html"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let html = String::from_utf8(output.stdout).unwrap();
    let data: Value = serde_json::from_str(
        html.split("<script id=\"archguard-data\" type=\"application/json\">")
            .nth(1)
            .unwrap()
            .split("</script>")
            .next()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(data["insights"]["patterns"], report);
    assert_eq!(data["initialTab"], "architecture");
    assert!(html.contains("function renderPatternAssessments("));
}
