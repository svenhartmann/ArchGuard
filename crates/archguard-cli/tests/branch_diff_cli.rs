use serde_json::Value;
use std::{fs, path::Path, process::Command};

fn git(root: &Path, args: &[&str]) -> Vec<u8> {
    let out = Command::new("git")
        .current_dir(root)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
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
    git(root, &["init", "-q", "--template=", "-b", "main"]);
    git(root, &["config", "user.name", "Test"]);
    git(root, &["config", "user.email", "test@example.invalid"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    fs::create_dir_all(root.join(".archguard")).unwrap();
    fs::write(
        root.join(".archguard/archguard.toml"),
        "derive_layers=false\n[project]\nsource_roots=[]\nlanguages=['py']\n[modules]\ndepth=0\n",
    )
    .unwrap();
    fs::write(root.join("a.ts"), "import './b';\n").unwrap();
    fs::write(root.join("b.ts"), "export const b=1;\n").unwrap();
    fs::write(root.join("removed.ts"), "export const old=1;\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "base"]);
    git(root, &["switch", "-qc", "feature/architecture"]);
    fs::write(
        root.join(".archguard/archguard.toml"),
        "derive_layers=false\n[project]\nsource_roots=[]\nlanguages=['ts']\n[modules]\ndepth=0\n",
    )
    .unwrap();
    fs::write(root.join("a.ts"), "// moved evidence\nimport './b';\n").unwrap();
    fs::write(
        root.join("b.ts"),
        "import './a'; // </script><script>globalThis.INJECTED=true</script>\n",
    )
    .unwrap();
    fs::write(root.join("added.ts"), "import './b';\n").unwrap();
    fs::remove_file(root.join("removed.ts")).unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "feature"]);
    git(root, &["switch", "-qc", "working", "main"]);
    fs::write(root.join("unrelated.ts"), "export const unrelated=1;\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "other branch"]);
    fs::write(root.join("b.ts"), "export const dirty_index=1;\n").unwrap();
    fs::write(root.join(".archguard/archguard.toml"), "[invalid").unwrap();
    git(root, &["add", "."]);
    fs::write(root.join("b.ts"), "export const dirty_worktree=1;\n").unwrap();
    fs::write(root.join("untracked.ts"), "export const untracked=1;\n").unwrap();
    dir
}
fn diff(root: &Path, from: &str, to: &str, format: &str) -> std::process::Output {
    Command::new(assert_cmd::cargo::cargo_bin!("archguard"))
        .arg("diff")
        // Keep option-like input a literal ref so Git resolution is exercised.
        .arg(format!("--from={from}"))
        .args(["--to", to, "--format", format])
        .arg(root)
        .output()
        .unwrap()
}
fn json(out: std::process::Output) -> Value {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
#[test]
fn branches_are_compared_without_checkout_or_index_input_under_target_policy() {
    let dir = fixture();
    let root = dir.path();
    let status = git(root, &["status", "--porcelain=v1", "-z"]);
    let index = fs::read(root.join(".git/index")).unwrap();
    let head = git(root, &["symbolic-ref", "HEAD"]);
    let result = json(diff(root, "main", "feature/architecture", "json"));
    assert_eq!(result["definition"], "git-refs-architecture/v1");
    assert_eq!(result["target"], "ref");
    assert_eq!(result["base_ref"], "main");
    assert_eq!(result["target_ref"], "feature/architecture");
    assert_eq!(result["policy_source"], "target");
    for (field, branch) in [
        ("base_commit", "main"),
        ("target_commit", "feature/architecture"),
    ] {
        assert_eq!(
            result[field].as_str().unwrap(),
            String::from_utf8(git(root, &["rev-parse", branch]))
                .unwrap()
                .trim()
        );
    }
    assert_eq!(
        result["base_graph"]["edges"].as_array().unwrap().len(),
        1,
        "both sides use the target branch's TypeScript policy"
    );
    assert_eq!(result["target_graph"]["edges"].as_array().unwrap().len(), 3);
    assert_eq!(result["added_edges"].as_array().unwrap().len(), 2);
    assert_eq!(result["changed_edges"][0]["from"], "a.ts");
    assert_eq!(result["added_nodes"], serde_json::json!(["added.ts"]));
    assert_eq!(result["removed_nodes"], serde_json::json!(["removed.ts"]));
    assert!(
        result["new_findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["kind"] == "cycle")
    );
    let rendered = result.to_string();
    for excluded in [
        "dirty_index",
        "dirty_worktree",
        "untracked.ts",
        "unrelated.ts",
    ] {
        assert!(
            !rendered.contains(excluded),
            "unexpected worktree/index content: {excluded}"
        );
    }
    assert!(
        !result["source_diff"]
            .as_str()
            .unwrap()
            .contains("unrelated")
    );
    let html = diff(root, "main", "feature/architecture", "html");
    assert!(html.status.success());
    let html = String::from_utf8(html.stdout).unwrap();
    assert!(html.contains("id=\"architecture-diff-data\""));
    assert!(!html.contains("</script><script>globalThis.INJECTED"));
    assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
    assert_eq!(git(root, &["symbolic-ref", "HEAD"]), head);
    assert_eq!(git(root, &["status", "--porcelain=v1", "-z"]), status);
    assert_eq!(
        fs::read_to_string(root.join("b.ts")).unwrap(),
        "export const dirty_worktree=1;\n"
    );
}
#[test]
fn identical_refs_and_explicit_policy_work_even_with_unmerged_index() {
    let dir = fixture();
    let root = dir.path();
    use std::io::Write;
    let oid = String::from_utf8(git(root, &["rev-parse", "main:a.ts"])).unwrap();
    let mut child = Command::new("git")
        .current_dir(root)
        .args(["update-index", "--index-info"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
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
    assert!(child.wait().unwrap().success());
    let index = fs::read(root.join(".git/index")).unwrap();
    let same = json(diff(
        root,
        "feature/architecture",
        "feature/architecture",
        "json",
    ));
    assert_eq!(same["added_edges"], serde_json::json!([]));
    assert_eq!(same["removed_edges"], serde_json::json!([]));
    assert_eq!(same["changed_edges"], serde_json::json!([]));
    assert_eq!(same["new_findings"], serde_json::json!([]));
    assert_eq!(same["source_diff"], "");
    let config = root.join("explicit.toml");
    fs::write(
        &config,
        "derive_layers=false\n[project]\nlanguages=['ts']\n[modules]\ndepth=0\n",
    )
    .unwrap();
    let reverse = json(
        Command::new(assert_cmd::cargo::cargo_bin!("archguard"))
            .args([
                "diff",
                "--base",
                "feature/architecture",
                "--to",
                "main",
                "--format",
                "json",
                "--config",
            ])
            .arg(config)
            .arg(root)
            .output()
            .unwrap(),
    );
    assert_eq!(reverse["policy_source"], "explicit");
    assert_eq!(reverse["removed_edges"].as_array().unwrap().len(), 2);
    assert!(
        reverse["resolved_findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["kind"] == "cycle")
    );
    assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
}
#[test]
fn unknown_refs_and_ambiguous_target_options_are_rejected() {
    let dir = fixture();
    for (from, to) in [("missing", "main"), ("main", "missing"), ("--all", "main")] {
        assert_eq!(diff(dir.path(), from, to, "json").status.code(), Some(3));
    }
    for flags in [
        vec!["diff", "--from", "main"],
        vec!["diff", "--staged", "--to", "main"],
    ] {
        let out = Command::new(assert_cmd::cargo::cargo_bin!("archguard"))
            .args(flags)
            .arg(dir.path())
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2));
    }
}

#[test]
fn divergent_branches_use_both_tips_instead_of_the_merge_base() {
    let dir = fixture();
    git(dir.path(), &["branch", "-f", "main", "working"]);
    let result = json(diff(dir.path(), "main", "feature/architecture", "json"));
    assert!(
        result["removed_nodes"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("unrelated.ts"))
    );
    assert!(
        result["source_diff"]
            .as_str()
            .unwrap()
            .contains("unrelated.ts")
    );
}

#[test]
fn branch_comparison_respects_nested_project_scope_and_config() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q", "--template=", "-b", "main"]);
    git(root, &["config", "user.name", "Test"]);
    git(root, &["config", "user.email", "test@example.invalid"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    let project = root.join("nested project");
    fs::create_dir_all(project.join(".archguard")).unwrap();
    fs::write(
        project.join(".archguard/archguard.toml"),
        "derive_layers=false\n[project]\nsource_roots=[]\nlanguages=['ts']\n[modules]\ndepth=0\n",
    )
    .unwrap();
    fs::write(project.join("a.ts"), "import './b';\n").unwrap();
    fs::write(project.join("b.ts"), "export const b=1;\n").unwrap();
    fs::write(root.join("outside.ts"), "export const outside=1;\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "base"]);
    git(root, &["switch", "-qc", "feature"]);
    fs::write(project.join("b.ts"), "import './a';\n").unwrap();
    fs::write(root.join("outside.ts"), "import './nested project/a';\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "changes"]);
    let result = json(diff(&project, "main", "feature", "json"));
    assert_eq!(result["added_edges"].as_array().unwrap().len(), 1);
    assert_eq!(result["added_edges"][0]["from"], "b.ts");
    assert!(!result.to_string().contains("outside.ts"));
    assert!(
        result["source_diff"]
            .as_str()
            .unwrap()
            .contains("nested project/b.ts")
    );
}
