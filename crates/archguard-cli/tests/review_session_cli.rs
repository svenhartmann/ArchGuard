use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};

fn run(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(assert_cmd::cargo::cargo_bin!("archguard"))
        .arg("review")
        .arg(root)
        .args(args)
        .output()
        .unwrap()
}
fn value(output: std::process::Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .args([
            "-c",
            "user.name=Review Test",
            "-c",
            "user.email=review@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn project() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("src")).unwrap();
    fs::write(root.path().join("src/a.ts"), "export const a=1;\n").unwrap();
    fs::write(root.path().join("src/b.ts"), "export const b=1;\n").unwrap();
    fs::write(root.path().join("archguard.toml"), "derive_layers=false\n[project]\nsource_roots=['src']\nlanguages=['ts']\nexclude=[]\n[rules]\ndeny_cycles='off'\n[[dependency_rules]]\nname='Domain boundary'\nfrom=['a.ts']\nto=['b.ts']\n").unwrap();
    git(root.path(), &["init", "--quiet", "--template="]);
    git(root.path(), &["add", "."]);
    git(root.path(), &["commit", "--quiet", "-m", "base"]);
    fs::write(
        root.path().join("src/a.ts"),
        "import {b} from './b';\nexport const a=b;\n",
    )
    .unwrap();
    root
}
fn issue(report: &Value) -> &Value {
    report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["category"] == "architecture" && f["evidence"]["kind"] == "dependency_rule")
        .unwrap()
}
fn decision_document(report: &Value) -> Value {
    let item = issue(report);
    json!({"schema_version":1,"review_revision":report["revision"],"decisions":[{
        "finding_id":item["id"],"finding_fingerprint":item["fingerprint"],
        "status":"accepted_risk","owner":"@team/orders","reason":"Temporary migration boundary",
        "expires_on":"2099-12-31","links":["docs/adr/001.md"]
    }]})
}

#[test]
fn review_compares_worktree_and_exposes_missing_evidence_without_implicit_recording() {
    let root = project();
    let report = value(run(root.path(), &["--base", "HEAD", "--format", "json"]));
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["revision"].as_str().unwrap().len(), 64);
    assert_eq!(report["status"], "needs_review");
    assert_eq!(issue(&report)["change"], "new");
    assert_eq!(issue(&report)["disposition"], "open");
    assert_eq!(report["changes"]["files"]["src/a.ts"], "M");
    assert!(
        report["impact"]["affected_files"]
            .as_array()
            .unwrap()
            .contains(&json!("a.ts"))
    );
    assert!(
        report["gaps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|g| g["source"] == "coverage")
    );
    assert!(!root.path().join(".archguard/review.json").exists());
    let again = value(run(root.path(), &["--base", "HEAD", "--format", "json"]));
    assert_eq!(again["revision"], report["revision"]);
    assert_eq!(issue(&again)["id"], issue(&report)["id"]);
}

#[test]
fn review_retains_the_explicit_policy_path_for_reproducible_refresh_commands() {
    let root = project();
    let config = root.path().join("review policy.toml");
    fs::rename(root.path().join("archguard.toml"), &config).unwrap();
    let report = value(run(
        root.path(),
        &["--config", config.to_str().unwrap(), "--format", "json"],
    ));
    assert_eq!(
        report["config_path"],
        config.canonicalize().unwrap().to_string_lossy().as_ref()
    );
    assert_eq!(issue(&report)["change"], "not_compared");
}

#[test]
fn decisions_are_explicit_bound_to_evidence_and_do_not_disable_existing_gates() {
    let root = project();
    let initial = value(run(root.path(), &["--base", "HEAD", "--format", "json"]));
    let path = root.path().join("decisions.json");
    fs::write(
        &path,
        serde_json::to_vec(&decision_document(&initial)).unwrap(),
    )
    .unwrap();
    let reviewed = value(run(
        root.path(),
        &[
            "--base",
            "HEAD",
            "--decisions",
            path.to_str().unwrap(),
            "--save",
            "--format",
            "json",
        ],
    ));
    assert_eq!(issue(&reviewed)["disposition"], "accepted_risk");
    assert_eq!(issue(&reviewed)["decision"]["owner"], "@team/orders");
    assert!(root.path().join(".archguard/review.json").exists());
    assert_eq!(
        fs::read_dir(root.path().join(".archguard/reviews"))
            .unwrap()
            .count(),
        1
    );
    let saved = fs::read(root.path().join(".archguard/review-decisions.json")).unwrap();
    let mut foreign = decision_document(&initial);
    foreign["review_revision"] = json!("f".repeat(64));
    fs::write(&path, serde_json::to_vec(&foreign).unwrap()).unwrap();
    assert_eq!(
        run(
            root.path(),
            &["--base", "HEAD", "--decisions", path.to_str().unwrap()]
        )
        .status
        .code(),
        Some(3)
    );
    assert_eq!(
        fs::read(root.path().join(".archguard/review-decisions.json")).unwrap(),
        saved
    );
    let gate = Command::new(assert_cmd::cargo::cargo_bin!("archguard"))
        .arg("check")
        .arg(root.path())
        .output()
        .unwrap();
    assert_eq!(gate.status.code(), Some(1));
}

#[test]
fn review_html_contains_the_shared_report_and_rejects_foreign_formats_before_writing() {
    let root = project();
    let output = run(
        root.path(),
        &["--base", "HEAD", "--save", "--format", "html"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let html = String::from_utf8(output.stdout).unwrap();
    let payload = html
        .split("<script id=\"archguard-data\" type=\"application/json\">")
        .nth(1)
        .unwrap()
        .split("</script>")
        .next()
        .unwrap();
    let data: Value = serde_json::from_str(payload).unwrap();
    assert_eq!(data["initialTab"], "assessment");
    assert_eq!(data["insights"]["review_session"]["status"], "current");
    assert_eq!(
        issue(&data["insights"]["review_session"]["report"])["change"],
        "new"
    );
    assert!(root.path().join(".archguard/reports/review.html").exists());
    let previous = fs::read(root.path().join(".archguard/review.json")).unwrap();
    assert_eq!(
        run(root.path(), &["--save", "--format", "sarif"])
            .status
            .code(),
        Some(2)
    );
    assert_eq!(
        fs::read(root.path().join(".archguard/review.json")).unwrap(),
        previous
    );
}
