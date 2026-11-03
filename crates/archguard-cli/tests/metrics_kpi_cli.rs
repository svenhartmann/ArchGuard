//! `archguard metrics` review-KPI table (R28): the KPI rows render below
//! the module table and their delta column reads the newest snapshot.

use std::{fs, path::Path};

use assert_cmd::Command;
use tempfile::tempdir;

fn fixture() -> tempfile::TempDir {
    let dir = tempdir().expect("tempdir");
    fs::write(dir.path().join("archguard.toml"), "").expect("config");
    fs::create_dir(dir.path().join("src")).expect("src");
    fs::write(dir.path().join("src/a.ts"), "import './b';\n").expect("a");
    fs::write(dir.path().join("src/b.ts"), "import './a';\n").expect("b");
    dir
}

fn metrics(dir: &Path) -> String {
    let output = Command::cargo_bin("archguard")
        .expect("binary")
        .args(["metrics"])
        .current_dir(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(output).expect("utf-8")
}

#[test]
fn metrics_renders_the_kpi_table_with_snapshot_deltas() {
    let dir = fixture();
    let first = metrics(dir.path());
    assert!(
        first.contains("Review KPIs (delta vs. newest snapshot):"),
        "{first}"
    );
    // Without any snapshot every delta is a dash — never an invented 0.
    assert!(first.contains("delta      -"), "{first}");

    Command::cargo_bin("archguard")
        .expect("binary")
        .args(["snapshot", "--quiet"])
        .current_dir(dir.path())
        .assert()
        .success();
    assert!(dir.path().join(".archguard/history").is_dir());

    // A new debt marker must show up as a positive delta against the
    // snapshot that was taken without it.
    fs::write(
        dir.path().join("src/a.ts"),
        "import './b';\n// TODO: split this\n",
    )
    .expect("a2");
    let second = metrics(dir.path());
    let debt_row = second
        .lines()
        .find(|line| line.contains("debt_markers"))
        .unwrap_or_else(|| panic!("no debt row in:\n{second}"));
    assert!(debt_row.contains("+1"), "debt row: {debt_row}");
}
