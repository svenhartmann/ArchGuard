use serde_json::Value;
use std::{fs, path::Path, process::Command};

fn git(root: &Path, args: &[&str]) -> Vec<u8> {
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
    out.stdout
}
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q"]);
    git(root, &["config", "user.name", "Test"]);
    git(root, &["config", "user.email", "test@example.invalid"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    fs::create_dir(root.join(".archguard")).unwrap();
    fs::write(
        root.join(".archguard/archguard.toml"),
        "derive_layers=false\n[project]\nsource_roots=[]\nlanguages=['ts']\n[modules]\ndepth=0\n",
    )
    .unwrap();
    fs::write(root.join("a.ts"), "import './b';\n").unwrap();
    fs::write(root.join("b.ts"), "export const b=1;\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "base"]);
    dir
}
fn diff(root: &Path, format: &str) -> std::process::Output {
    Command::new(assert_cmd::cargo::cargo_bin!("archguard"))
        .args(["diff", "--staged", "--format", format])
        .arg(root)
        .output()
        .unwrap()
}
#[test]
fn staged_diff_uses_complete_index_and_preserves_partial_staging() {
    let dir = fixture();
    let root = dir.path();
    fs::write(root.join("b.ts"), "import './a';\n").unwrap();
    git(root, &["add", "b.ts"]);
    // The staged version creates a cycle; the worktree removes it again.
    fs::write(root.join("b.ts"), "export const unstaged=2;\n").unwrap();
    fs::write(root.join("untracked.ts"), "import './a';\n").unwrap();
    // Invalid unstaged policy must not influence a staged analysis.
    fs::write(root.join(".archguard/archguard.toml"), "[invalid").unwrap();
    let index = fs::read(root.join(".git/index")).unwrap();
    let worktree = fs::read(root.join("b.ts")).unwrap();
    let status = git(root, &["status", "--porcelain=v1", "-z"]);
    let out = diff(root, "json");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let result: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(result["target"], "index");
    assert_eq!(result["policy_source"], "index");
    assert_eq!(result["added_edges"].as_array().unwrap().len(), 1);
    assert_eq!(result["added_edges"][0]["from"], "b.ts");
    assert_eq!(result["added_edges"][0]["to"], "a.ts");
    assert!(
        result["new_findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["kind"] == "cycle")
    );
    assert!(!String::from_utf8_lossy(&out.stdout).contains("untracked.ts"));
    assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
    assert_eq!(fs::read(root.join("b.ts")).unwrap(), worktree);
    assert_eq!(git(root, &["status", "--porcelain=v1", "-z"]), status);
    let html = diff(root, "html");
    assert!(
        html.status.success(),
        "{}",
        String::from_utf8_lossy(&html.stderr)
    );
    let html = String::from_utf8(html.stdout).unwrap();
    assert!(html.contains("id=\"architecture-diff-data\""));
    assert!(html.contains("Git-Index"));
}
#[test]
fn unchanged_index_and_staged_deletions_are_distinguished() {
    let dir = fixture();
    let root = dir.path();
    fs::write(root.join("b.ts"), "import './a';\n").unwrap();
    let out = diff(root, "json");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let result: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(result["added_edges"], serde_json::json!([]));
    assert_eq!(result["new_findings"], serde_json::json!([]));
    git(root, &["rm", "a.ts"]);
    let out = diff(root, "json");
    assert!(out.status.success());
    let result: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(result["removed_edges"][0]["from"], "a.ts");
    assert_eq!(result["removed_nodes"], serde_json::json!(["a.ts"]));
}

#[test]
fn intent_to_add_is_not_a_staged_architecture_node() {
    let dir = fixture();
    let root = dir.path();
    fs::write(root.join("pending.ts"), "import './a';\n").unwrap();
    git(root, &["add", "--intent-to-add", "pending.ts"]);
    let out = diff(root, "json");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let result: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(result["added_nodes"], serde_json::json!([]));
    assert!(
        !result["staged_graph"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["name"] == "pending.ts")
    );
}

#[test]
fn comparison_is_scoped_and_rejects_conflicts_and_unknown_refs() {
    let dir = fixture();
    let root = dir.path();
    fs::create_dir(root.join("nested")).unwrap();
    fs::write(root.join("nested/a.ts"), "export const a=1;\n").unwrap();
    fs::write(root.join("nested/b.ts"), "export const b=1;\n").unwrap();
    git(root, &["add", "nested"]);
    git(root, &["commit", "-qm", "nested"]);
    fs::write(root.join("nested/a.ts"), "import './b';\n").unwrap();
    fs::write(root.join("b.ts"), "import './a';\n").unwrap();
    git(root, &["add", "nested/a.ts", "b.ts"]);
    let out = diff(&root.join("nested"), "json");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let result: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(result["added_edges"].as_array().unwrap().len(), 1);
    assert_eq!(result["added_edges"][0]["from"], "a.ts");
    assert!(
        !result["source_diff"]
            .as_str()
            .unwrap()
            .contains("diff --git a/b.ts")
    );
    let out = Command::new(assert_cmd::cargo::cargo_bin!("archguard"))
        .args(["diff", "--staged", "--base", "no-such-commit"])
        .arg(root)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    use std::io::Write;
    let oid = String::from_utf8(git(root, &["rev-parse", "HEAD:a.ts"])).unwrap();
    let mut command = Command::new("git")
        .current_dir(root)
        .args(["update-index", "--index-info"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    command
        .stdin
        .take()
        .unwrap()
        .write_all(
            format!(
                "0 {}\ta.ts\n100644 {} 1\ta.ts\n100644 {} 2\ta.ts\n",
                "0".repeat(40),
                oid.trim(),
                oid.trim()
            )
            .as_bytes(),
        )
        .unwrap();
    assert!(command.wait().unwrap().success());
    let out = diff(root, "json");
    assert_eq!(out.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&out.stderr).contains("unmerged index"));
}
