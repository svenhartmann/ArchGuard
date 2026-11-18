//! `archguard cycles --fail-on-found` is a gate like `check`: a path without
//! any source file must not pass it (exit 3, same refusal as `check`).

use std::fs;

use assert_cmd::Command;
use tempfile::tempdir;

fn cycles(dir: &std::path::Path) -> std::process::Output {
    Command::cargo_bin("archguard")
        .expect("binary")
        .args(["cycles", ".", "--fail-on-found"])
        .current_dir(dir)
        .output()
        .expect("runs")
}

#[test]
fn empty_path_is_an_error_not_a_pass() {
    let dir = tempdir().expect("tempdir");
    let output = cycles(dir.path());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(3), "{stderr}");
    assert!(stderr.contains("no source files found under"), "{stderr}");
}

#[test]
fn project_without_cycles_still_passes() {
    let dir = tempdir().expect("tempdir");
    fs::write(
        dir.path().join("pyproject.toml"),
        "[project]\nname = \"mypkg\"\n",
    )
    .expect("toml");
    fs::create_dir(dir.path().join("mypkg")).expect("mypkg");
    fs::write(dir.path().join("mypkg/__init__.py"), "").expect("init");
    fs::write(dir.path().join("mypkg/a.py"), "from mypkg import b\n").expect("a");
    fs::write(dir.path().join("mypkg/b.py"), "X = 1\n").expect("b");
    let output = cycles(dir.path());
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
