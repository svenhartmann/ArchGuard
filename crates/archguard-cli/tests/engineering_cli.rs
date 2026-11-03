use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};
fn run(root: &Path, command: &str, args: &[&str]) -> std::process::Output {
    Command::new(assert_cmd::cargo::cargo_bin!("archguard"))
        .arg(command)
        .arg(root)
        .args(args)
        .output()
        .unwrap()
}
fn value(output: &std::process::Output, code: i32) -> Value {
    assert_eq!(
        output.status.code(),
        Some(code),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn project() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("lib")).unwrap();
    fs::write(root.path().join("lib/a.ts"), "export const a = 1;\n").unwrap();
    fs::write(
        root.path().join("lib/b.ts"),
        "import { a } from './a';\nexport const b = a;\n",
    )
    .unwrap();
    fs::write(root.path().join("archguard.toml"),"derive_layers=false\n[project]\nsource_roots=[]\nlanguages=['ts']\nexclude=[]\n[rules]\ndeny_cycles='off'\n").unwrap();
    root
}
#[test]
fn cohesion_and_explicit_quality_history_are_exposed_without_implicit_history_writes() {
    let root = project();
    let cohesion = value(&run(root.path(), "cohesion", &["--format", "json"]), 0);
    let package = cohesion["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["node"] == "lib")
        .unwrap();
    assert_eq!(package["internal_dependencies"], 1);
    assert_eq!(package["files"], 2);
    let empty = value(
        &run(root.path(), "quality-history", &["--format", "json"]),
        0,
    );
    assert_eq!(empty["status"], "not_recorded");
    assert!(!root.path().join(".archguard/quality-history").exists());
    let first = value(
        &run(
            root.path(),
            "quality-history",
            &["--record", "--tag", "initial", "--format", "json"],
        ),
        0,
    );
    assert_eq!(first["latest"]["tag"], "initial");
    assert_eq!(first["latest"]["producer_revision"], "unverified");
    let second = value(
        &run(
            root.path(),
            "quality-history",
            &["--record", "--format", "json"],
        ),
        0,
    );
    assert_eq!(second["records"].as_array().unwrap().len(), 2);
    assert!(second["comparison"]["modules"].is_array());
    assert_eq!(
        value(
            &run(root.path(), "quality-history", &["--format", "json"]),
            0
        ),
        second
    );
}
#[test]
fn test_evidence_import_keeps_durations_mapping_and_target_recommendations() {
    let root = project();
    fs::write(
        root.path().join("tests.xml"),
        "<testsuite name='unit'><testcase classname='Example' name='a' time='0.125'/></testsuite>",
    )
    .unwrap();
    fs::write(root.path().join("map.json"),r#"{"schema_version":1,"tests":[{"class_name":"Example","name":"a","sources":[{"path":"lib/a.ts","lines":[1]}]}]}"#).unwrap();
    let imported = value(
        &run(
            root.path(),
            "test-evidence",
            &[
                "--report",
                "tests.xml",
                "--mapping",
                "map.json",
                "--target",
                "lib/a.ts",
                "--format",
                "json",
            ],
        ),
        0,
    );
    assert_eq!(imported["report"]["tests"][0]["duration_seconds"], 0.125);
    assert_eq!(
        imported["impact"]["selected"][0]["reason"],
        "mapped_changed_source"
    );
    let saved = fs::read(root.path().join(".archguard/tests.json")).unwrap();
    assert_eq!(
        value(
            &run(
                root.path(),
                "test-evidence",
                &["--target", "lib/a.ts", "--format", "json"]
            ),
            0
        ),
        imported
    );
    fs::write(root.path().join("tests.xml"), "<broken>").unwrap();
    assert_eq!(
        run(root.path(), "test-evidence", &["--report", "tests.xml"])
            .status
            .code(),
        Some(3)
    );
    assert_eq!(
        fs::read(root.path().join(".archguard/tests.json")).unwrap(),
        saved
    );
    assert!(
        fs::read_to_string(root.path().join(".archguard/.gitignore"))
            .unwrap()
            .contains("/tests.json")
    );
}
#[test]
fn mutations_import_is_informative_and_preserves_snapshot_on_error() {
    let root = project();
    let report = json!({"schemaVersion":"2","thresholds":{"high":80,"low":60},"framework":{"name":"Stryker","version":"test"},"files":{"lib/a.ts":{"language":"typescript","source":"export const a = 1;\n","mutants":[{"id":"1","mutatorName":"NumberLiteral","status":"Survived","location":{"start":{"line":1,"column":18},"end":{"line":1,"column":19}},"replacement":"2"}]}}});
    fs::write(root.path().join("mutations.json"), report.to_string()).unwrap();
    let imported = value(
        &run(
            root.path(),
            "mutations",
            &["--report", "mutations.json", "--format", "json"],
        ),
        0,
    );
    assert_eq!(imported["summary"]["survived"], 1);
    assert_eq!(imported["files"][0]["node"], "lib/a.ts");
    assert_eq!(
        value(&run(root.path(), "mutations", &["--format", "json"]), 0),
        imported
    );
    let saved = fs::read(root.path().join(".archguard/mutation.json")).unwrap();
    fs::write(root.path().join("mutations.json"), "{}").unwrap();
    assert_eq!(
        run(root.path(), "mutations", &["--report", "mutations.json"])
            .status
            .code(),
        Some(3)
    );
    assert_eq!(
        fs::read(root.path().join(".archguard/mutation.json")).unwrap(),
        saved
    );
    assert!(
        run(root.path(), "mutations", &["--quiet"])
            .stdout
            .is_empty()
    );
}
#[test]
fn contracts_work_for_compose_only_repositories_and_report_changed_operations() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("compose.yaml"),
        "services:\n  orders:\n    image: orders\n",
    )
    .unwrap();
    let services = value(&run(root.path(), "services", &["--format", "json"]), 0);
    let provider = services["services"][0]["id"].as_str().unwrap();
    let spec = json!({"openapi":"3.1.0","info":{"title":"Orders","version":"1"},"paths":{"/orders":{"get":{"responses":{"200":{"description":"OK"}}}}}});
    fs::write(root.path().join("api.json"), spec.to_string()).unwrap();
    let first = value(
        &run(
            root.path(),
            "contracts",
            &[
                "--report",
                "api.json",
                "--service",
                provider,
                "--format",
                "json",
            ],
        ),
        0,
    );
    assert_eq!(first["contracts"][0]["comparison_status"], "no_baseline");
    let mut second = spec;
    second["paths"] = json!({});
    fs::write(root.path().join("api.json"), second.to_string()).unwrap();
    let changed = value(
        &run(
            root.path(),
            "contracts",
            &[
                "--report",
                "api.json",
                "--service",
                provider,
                "--format",
                "json",
            ],
        ),
        0,
    );
    assert_eq!(
        changed["contracts"][0]["changes"][0]["kind"],
        "operation_removed"
    );
    assert_eq!(
        value(&run(root.path(), "contracts", &["--format", "json"]), 0),
        changed
    );
}
#[test]
fn new_command_usage_is_checked_before_import_writes() {
    let root = project();
    for (command, args) in [
        ("quality-history", vec!["--tag", "x"]),
        ("test-evidence", vec!["--mapping", "x.json"]),
        ("test-evidence", vec!["--source-root", "lib"]),
        (
            "test-evidence",
            vec!["--report", "tests.xml", "--source-root", "lib"],
        ),
        ("mutations", vec!["--source-root", "lib"]),
        ("contracts", vec!["--report", "api.json"]),
        ("contracts", vec!["--service", "orders"]),
        ("contracts", vec!["--usage", "x.json"]),
    ] {
        assert_eq!(
            run(root.path(), command, &args).status.code(),
            Some(2),
            "{command} {args:?}"
        );
    }
    for command in [
        "cohesion",
        "quality-history",
        "test-evidence",
        "mutations",
        "contracts",
    ] {
        assert_eq!(
            run(root.path(), command, &["--format", "dot"])
                .status
                .code(),
            Some(2),
            "{command}"
        );
    }
    assert!(!root.path().join(".archguard/tests.json").exists());
    assert!(!root.path().join(".archguard/quality-history").exists());
}

#[test]
fn class_cohesion_has_a_separate_compiler_evidence_view() {
    let root = project();
    let report = value(
        &run(root.path(), "cohesion", &["--classes", "--format", "json"]),
        0,
    );
    assert_eq!(report["definition"], "lcom4-source/v1");
    assert!(report["classes"].is_array());
    assert!(report["analyzers"].is_array());
    assert!(report["status"].is_string());
}
