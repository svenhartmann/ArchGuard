//! `archguard sbom` writes CycloneDX 1.6 and a summary from the lockfiles
//! in the repository; `archguard check` gates the `[licenses]` policy
//! (ADR 0016).

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use serde_json::Value;
use tempfile::tempdir;

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sbom-polyglot")
}

fn archguard(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::cargo_bin("archguard")
        .expect("binary")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("runs")
}

fn json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{error}; stderr {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn sbom_writes_cyclonedx_1_6() {
    let output = archguard(&fixture(), &["sbom", ".", "--format", "cyclonedx"]);
    assert!(output.status.success(), "{output:?}");
    let bom = json(&output);
    assert_eq!(bom["bomFormat"], "CycloneDX");
    assert_eq!(bom["specVersion"], "1.6");
    assert_eq!(bom["metadata"]["component"]["name"], "sbom-polyglot");
    assert_eq!(bom["components"].as_array().expect("components").len(), 24);
}

#[test]
fn sbom_summary_in_json_and_text() {
    let output = archguard(&fixture(), &["sbom", ".", "--format", "json"]);
    assert!(output.status.success(), "{output:?}");
    let summary = json(&output);
    assert_eq!(summary["components"].as_array().expect("list").len(), 24);
    assert_eq!(summary["license_states"]["unknown"], 18);
    assert_eq!(summary["violations"], 0, "no [licenses] policy configured");

    let output = archguard(&fixture(), &["sbom", "."]);
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("24 third-party components"), "{text}");
    assert!(text.contains("unknown: 18"), "{text}");
    assert!(text.contains("services/web/package-lock.json"), "{text}");
}

#[test]
fn sbom_rejects_foreign_formats() {
    let output = archguard(&fixture(), &["sbom", ".", "--format", "sarif"]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
}

fn project(licenses: &str) -> tempfile::TempDir {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("archguard.toml"), licenses).expect("config");
    fs::create_dir(dir.path().join("src")).expect("src");
    fs::write(dir.path().join("src/a.ts"), "export const a = 1;\n").expect("a");
    fs::write(
        dir.path().join("package-lock.json"),
        r#"{"packages":{"":{},"node_modules/gpl-thing":{"version":"2.0.0","license":"GPL-3.0-only"},"node_modules/quiet":{"version":"1.0.0"}}}"#,
    )
    .expect("lock");
    dir
}

#[test]
fn check_without_licence_policy_ignores_licences() {
    let dir = project("");
    let output = archguard(dir.path(), &["check"]);
    assert!(output.status.success(), "{output:?}");
}

#[test]
fn check_fails_on_a_denied_licence_and_keeps_unknown_apart() {
    let dir = project("[licenses]\ndeny = ['GPL-3.0-only']\n");
    let output = archguard(dir.path(), &["check", "--format", "json"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let report = json(&output);
    let ids: Vec<&str> = report["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .filter(|f| f["kind"] == "license")
        .map(|f| f["id"].as_str().expect("id"))
        .collect();
    assert_eq!(
        ids,
        ["license:pkg:npm/gpl-thing@2.0.0"],
        "unknown is no violation"
    );

    let dir = project("[licenses]\nunknown = 'deny'\n");
    let output = archguard(dir.path(), &["check"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("license:pkg:npm/quiet@1.0.0"), "{text}");
    assert!(!text.contains("gpl-thing"), "{text}");
}

#[test]
fn check_reads_a_yarn_lock_with_a_byte_order_mark() {
    let dir = project("");
    fs::write(
        dir.path().join("yarn.lock"),
        "\u{FEFF}# yarn lockfile v1\n\nlodash@^4.17.21:\n  version \"4.17.21\"\n",
    )
    .expect("yarn.lock");
    let output = archguard(dir.path(), &["check"]);
    assert!(output.status.success(), "{output:?}");
}

#[test]
fn check_gates_a_lockfile_deep_in_a_monorepo() {
    let dir = project("[licenses]\ndeny = ['GPL-3.0-only']\n");
    let deep = dir.path().join("a/b/c/d/e");
    fs::create_dir_all(&deep).expect("mkdir");
    fs::rename(
        dir.path().join("package-lock.json"),
        deep.join("package-lock.json"),
    )
    .expect("move");
    let output = archguard(dir.path(), &["check"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("license:pkg:npm/gpl-thing@2.0.0"), "{text}");
}

#[test]
fn check_warns_when_a_licence_policy_finds_no_lockfile() {
    let dir = project("[licenses]\ndeny = ['GPL-3.0-only']\n");
    fs::remove_file(dir.path().join("package-lock.json")).expect("rm");
    let output = archguard(dir.path(), &["check", "--format", "json"]);
    assert!(
        output.status.success(),
        "a warning does not fail: {output:?}"
    );
    let report = json(&output);
    let warnings: Vec<&Value> = report["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .filter(|f| f["id"] == "license-coverage")
        .collect();
    assert_eq!(warnings.len(), 1, "{report}");
    assert_eq!(warnings[0]["severity"], "warning");
}
