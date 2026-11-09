//! `archguard check` must not report green when it analysed the wrong
//! thing: a flat Python package (`mypkg/__init__.py` at the repo root)
//! must keep its package name so its cycle is found, and a directory
//! without any source file is an error (exit 3), not a pass.

use std::fs;

use assert_cmd::Command;
use tempfile::tempdir;

fn check(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::cargo_bin("archguard")
        .expect("binary")
        .arg("check")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("runs")
}

/// A flat-layout Python project whose two modules import each other.
fn flat_python_cycle(a_import: &str, b_import: &str) -> tempfile::TempDir {
    let dir = tempdir().expect("tempdir");
    fs::write(
        dir.path().join("pyproject.toml"),
        "[project]\nname = \"mypkg\"\n",
    )
    .expect("pyproject");
    fs::create_dir(dir.path().join("mypkg")).expect("mypkg");
    fs::write(dir.path().join("mypkg/__init__.py"), "").expect("init");
    fs::write(dir.path().join("mypkg/a.py"), a_import).expect("a");
    fs::write(dir.path().join("mypkg/b.py"), b_import).expect("b");
    dir
}

#[test]
fn flat_python_package_cycle_via_from_import_fails_the_check() {
    let dir = flat_python_cycle("from mypkg import b\n", "from mypkg import a\n");
    let output = check(dir.path(), &["."]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "cycle must fail: stdout {} stderr {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn flat_python_package_cycle_via_dotted_import_fails_the_check() {
    let dir = flat_python_cycle("import mypkg.b\n", "import mypkg.a\n");
    let output = check(dir.path(), &["."]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "cycle must fail: stdout {} stderr {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn empty_project_is_an_error_not_a_pass() {
    let dir = tempdir().expect("tempdir");
    for args in [&["."][..], &[".", "--format", "sarif"][..]] {
        let output = check(dir.path(), args);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            output.status.code(),
            Some(3),
            "{args:?}: {stdout} / {stderr}"
        );
        assert!(!stdout.contains("PASS"), "{args:?}: {stdout}");
        assert!(stderr.contains("no source files"), "{args:?}: {stderr}");
    }
}
