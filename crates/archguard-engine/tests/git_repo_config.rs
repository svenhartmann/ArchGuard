//! Repository-local Git config must never start a program during analysis:
//! `core.fsmonitor` and `log.showSignature` + `gpg.program` are planted in
//! the analysed repository and every public entry point must leave no marker.
#![cfg(unix)]
use archguard_engine::core::{Config, GraphBuilder};
use archguard_engine::{collect_review_sources, compare_branches, compare_staged};
use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

/// A repo whose HEAD carries a `gpgsig` header, with a staged change and a
/// hostile local config. Returns (repo, marker file, guard dir).
fn hostile_repo() -> (tempfile::TempDir, std::path::PathBuf, tempfile::TempDir) {
    let repo = tempfile::tempdir().unwrap();
    let tools = tempfile::tempdir().unwrap();
    let root = repo.path();
    git(root, &["init", "--quiet", "--template="]);
    git(root, &["config", "user.name", "T"]);
    git(root, &["config", "user.email", "t@example.invalid"]);
    git(root, &["config", "commit.gpgsign", "false"]);
    fs::create_dir(root.join("src")).unwrap();
    fs::write(
        root.join("archguard.toml"),
        "derive_layers=false\n[project]\nsource_roots=['src']\nlanguages=['ts']\n[modules]\ndepth=0\n",
    )
    .unwrap();
    fs::write(
        root.join("src/a.ts"),
        "import './b';\nexport const a = 1;\n",
    )
    .unwrap();
    fs::write(root.join("src/b.ts"), "export const b = 1;\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "--quiet", "-m", "base"]);
    fs::write(root.join("src/b.ts"), "export const b = 2;\n").unwrap();
    git(root, &["add", "src/b.ts"]);
    let tree = git(root, &["write-tree"]);
    let parent = git(root, &["rev-parse", "HEAD"]);
    let commit = tools.path().join("commit.txt");
    fs::write(&commit, format!(
        "tree {tree}\nparent {parent}\nauthor t <t@example.invalid> 1700000000 +0000\ncommitter t <t@example.invalid> 1700000000 +0000\ngpgsig -----BEGIN PGP SIGNATURE-----\n \n AAAA\n -----END PGP SIGNATURE-----\n\nsigned\n"
    )).unwrap();
    let signed = git(
        root,
        &[
            "hash-object",
            "-t",
            "commit",
            "-w",
            commit.to_str().unwrap(),
        ],
    );
    git(root, &["update-ref", "HEAD", &signed]);
    fs::write(root.join("src/a.ts"), "export const a = 2;\n").unwrap();
    git(root, &["add", "src/a.ts"]);
    let marker = tools.path().join("marker");
    let hook = tools.path().join("hook.sh");
    fs::write(
        &hook,
        format!(
            "#!/bin/sh\necho \"$0 $*\" >> '{}'\nexit 1\n",
            marker.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    let hook = hook.to_str().unwrap();
    git(root, &["config", "core.fsmonitor", hook]);
    git(root, &["config", "log.showSignature", "true"]);
    git(root, &["config", "gpg.program", hook]);
    (repo, marker, tools)
}

fn assert_no_marker(marker: &Path, entry: &str) {
    let ran = fs::read_to_string(marker).unwrap_or_default();
    assert!(
        ran.is_empty(),
        "{entry} ran a repository-configured program: {ran}"
    );
}

#[test]
fn staged_and_branch_comparisons_ignore_repository_fsmonitor_and_gpg() {
    let (repo, marker, _tools) = hostile_repo();
    compare_staged(repo.path(), "HEAD", None).unwrap();
    assert_no_marker(&marker, "compare_staged");
    compare_branches(repo.path(), "HEAD~1", "HEAD", None).unwrap();
    assert_no_marker(&marker, "compare_branches");
}

#[test]
fn git_history_ignores_repository_signature_verification() {
    let (repo, marker, _tools) = hostile_repo();
    let mut builder = GraphBuilder::new();
    builder.ensure_path("a.ts", Some("typescript"));
    let graph = builder.build(repo.path().to_string_lossy().into_owned());
    let mut config = Config::default();
    config.project.source_roots = vec!["src".into()];
    let sources = collect_review_sources(repo.path(), &config, &graph, "2000-01-01").unwrap();
    assert!(sources.git.scanned_commits >= 1);
    assert_no_marker(&marker, "collect_review_sources");
}
