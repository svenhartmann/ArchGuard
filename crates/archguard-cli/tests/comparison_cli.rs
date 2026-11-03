use serde_json::Value;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
fn cli(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_archguard"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}
fn git(root: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("archguard.toml"),"derive_layers=false\n[project]\nsource_roots=[]\nlanguages=['ts']\n[modules]\ndepth=0\n[rules]\ndeny_cycles='off'\nmax_scc_size=100\n").unwrap();
    fs::write(root.join("a.ts"), "import './b';\n").unwrap();
    fs::write(root.join("b.ts"), "export const b=1;\n").unwrap();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.name", "Test"]);
    git(root, &["config", "user.email", "test@example.invalid"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "base"]);
    git(root, &["tag", "base"]);
    dir
}
#[test]
fn base_filter_and_acd_gate_share_scope_and_preserve_worktree() {
    let dir = fixture();
    let root = dir.path();
    let empty = cli(root, &["check", "--base", "base", "--format", "json"]);
    assert!(
        empty.status.success(),
        "{}",
        String::from_utf8_lossy(&empty.stderr)
    );
    assert!(
        serde_json::from_slice::<Value>(&empty.stdout).unwrap()["findings"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    fs::write(root.join("b.ts"), "import './a';\n").unwrap();
    let failed = cli(
        root,
        &[
            "check",
            "--base",
            "base",
            "--fail-on-acd-regression",
            "10",
            "--format",
            "sarif",
        ],
    );
    assert_eq!(
        failed.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&failed.stderr)
    );
    let sarif: Value = serde_json::from_slice(&failed.stdout).unwrap();
    assert!(
        sarif["runs"][0]["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|result| result["message"]["text"].as_str().unwrap().contains("ACD"))
    );
    assert_eq!(
        fs::read_to_string(root.join("b.ts")).unwrap(),
        "import './a';\n"
    );
}
#[test]
fn acd_gate_uses_last_snapshot_and_errors_without_reference() {
    let dir = fixture();
    let root = dir.path();
    let missing = cli(root, &["check", "--fail-on-acd-regression"]);
    assert_eq!(missing.status.code(), Some(3));
    assert!(cli(root, &["snapshot"]).status.success());
    fs::write(root.join("b.ts"), "import './a';\n").unwrap();
    assert_eq!(
        cli(root, &["check", "--fail-on-acd-regression", "10"])
            .status
            .code(),
        Some(1)
    );
    assert_eq!(
        cli(root, &["check", "--fail-on-acd-regression", "50"])
            .status
            .code(),
        Some(0)
    );
}
#[test]
fn snapshot_gate_rejects_changed_analysis_scope() {
    let dir = fixture();
    let root = dir.path();
    assert!(cli(root, &["snapshot"]).status.success());
    let config = fs::read_to_string(root.join("archguard.toml")).unwrap();
    fs::write(
        root.join("archguard.toml"),
        config.replace("depth=0", "depth=2"),
    )
    .unwrap();
    let out = cli(root, &["check", "--fail-on-acd-regression"]);
    assert_eq!(
        out.status.code(),
        Some(3),
        "a differently scoped snapshot cannot prove an ACD regression: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}
