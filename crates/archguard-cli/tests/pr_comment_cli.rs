//! `archguard review --format pr-comment` (R27): the Markdown comment
//! diffs the base priority report against the working tree and can be
//! written to `.archguard/reports/pr-comment.md`.

use std::{fs, path::Path};

use assert_cmd::Command;
use tempfile::tempdir;

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("HOME", dir)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn fixture() -> tempfile::TempDir {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("archguard.toml"), "").expect("config");
    fs::create_dir(dir.path().join("src")).expect("src");
    fs::write(dir.path().join("src/a.ts"), "import './b';\n").expect("a");
    fs::write(dir.path().join("src/b.ts"), "import './a';\n").expect("b");
    git(dir.path(), &["init", "--quiet", "--template=", "."]);
    git(dir.path(), &["add", "-A"]);
    git(
        dir.path(),
        &[
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "user.name=test",
            "commit",
            "--quiet",
            "-m",
            "base",
        ],
    );
    dir
}

#[test]
fn pr_comment_diffs_working_tree_against_base_and_writes_out() {
    let dir = fixture();
    // Uncommitted second cycle: the PR adds it, base does not have it.
    fs::write(dir.path().join("src/c.ts"), "import './d';\n").expect("c");
    fs::write(dir.path().join("src/d.ts"), "import './c';\n").expect("d");

    let output = Command::cargo_bin("archguard")
        .expect("binary")
        .args(["review", "--format", "pr-comment", "--base", "HEAD"])
        .current_dir(dir.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).expect("utf-8 stdout");
    assert!(stdout.starts_with("## Architektur-Review"), "{stdout}");
    assert!(stdout.contains("### Neue Befunde seit base"), "{stdout}");
    assert!(
        !stdout.contains("Keine neuen Befunde gegenüber base."),
        "the new c/d cycle must show up:\n{stdout}"
    );
    assert!(
        stdout.contains("Keine akzeptierten Risiken registriert."),
        "{stdout}"
    );

    // `--out` without a value takes the documented default path.
    Command::cargo_bin("archguard")
        .expect("binary")
        .args([
            "review",
            "--format",
            "pr-comment",
            "--base",
            "HEAD",
            "--out",
            "--quiet",
        ])
        .current_dir(dir.path())
        .assert()
        .success();
    let written =
        fs::read_to_string(dir.path().join(".archguard/reports/pr-comment.md")).expect("written");
    assert_eq!(written, stdout);
}

#[test]
fn pr_comment_without_base_is_a_usage_error() {
    let dir = fixture();
    Command::cargo_bin("archguard")
        .expect("binary")
        .args(["review", "--format", "pr-comment"])
        .current_dir(dir.path())
        .assert()
        .failure()
        .code(2);
}

/// `--out` and its default `.archguard/reports/pr-comment.md` must not
/// follow a committed symlink to a file outside the project.
#[cfg(unix)]
#[test]
fn pr_comment_out_refuses_to_write_through_a_symlink() {
    let dir = fixture();
    let outside = tempdir().expect("outside");
    let victim = outside.path().join("victim.txt");
    fs::write(&victim, "precious\n").expect("victim");
    let run = |args: &[&str]| {
        Command::cargo_bin("archguard")
            .expect("binary")
            .args([
                "review",
                "--format",
                "pr-comment",
                "--base",
                "HEAD",
                "--quiet",
            ])
            .args(args)
            .current_dir(dir.path())
            .assert()
    };
    // Default path, with the file itself a symlink ...
    fs::create_dir_all(dir.path().join(".archguard/reports")).expect("reports");
    let default = dir.path().join(".archguard/reports/pr-comment.md");
    std::os::unix::fs::symlink(&victim, &default).expect("symlink");
    run(&["--out"]).failure();
    // ... or its directory: .archguard/reports -> outside.
    fs::remove_file(&default).expect("remove");
    fs::remove_dir(dir.path().join(".archguard/reports")).expect("rmdir");
    std::os::unix::fs::symlink(outside.path(), dir.path().join(".archguard/reports"))
        .expect("dir symlink");
    run(&["--out"]).failure();
    assert!(!outside.path().join("pr-comment.md").exists());
    // An explicit --out that is a symlink.
    std::os::unix::fs::symlink(&victim, dir.path().join("comment.md")).expect("symlink");
    run(&["--out", "comment.md"]).failure();
    assert_eq!(fs::read_to_string(&victim).expect("victim"), "precious\n");
    // An explicit path outside the project stays allowed (documented).
    let chosen = outside.path().join("chosen.md");
    run(&["--out", chosen.to_str().expect("utf-8")]).success();
    assert!(
        fs::read_to_string(&chosen)
            .expect("written")
            .starts_with("## Architektur-Review")
    );
}
