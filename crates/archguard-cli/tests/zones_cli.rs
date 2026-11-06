//! `scan`/`check --measure-zones`: the opt-in compiler chain fills the
//! Martin A/I zones row the pipeline leaves unmeasured. Needs a JDK on PATH,
//! like the other javac-backed tests.

use std::{fs, path::Path};

use assert_cmd::Command;
use tempfile::tempdir;

fn java_fixture() -> tempfile::TempDir {
    let dir = tempdir().expect("tempdir");
    for (rel, source) in [
        (
            "src/a/Port.java",
            "package a; public interface Port { void run(); }\n",
        ),
        (
            "src/b/Adapter.java",
            "package b; public class Adapter implements a.Port { public void run() {} }\n",
        ),
    ] {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().unwrap()).expect("dirs");
        fs::write(path, source).expect("source");
    }
    dir
}

fn stdout(dir: &Path, args: &[&str]) -> String {
    let output = Command::cargo_bin("archguard")
        .expect("binary")
        .args(args)
        .current_dir(dir)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    String::from_utf8(output).expect("utf-8")
}

fn zones_row(text: &str) -> &str {
    text.lines()
        .find(|line| line.trim_start().starts_with("zones"))
        .unwrap_or_else(|| panic!("no zones row in:\n{text}"))
}

#[test]
fn scan_measures_zones_only_on_request() {
    let dir = java_fixture();
    let plain = stdout(dir.path(), &["scan"]);
    assert!(zones_row(&plain).contains("not measured"), "{plain}");
    let measured = stdout(dir.path(), &["scan", "--measure-zones"]);
    assert!(
        zones_row(&measured).contains("pain 0, useless 0, main 2, unmeasured 0"),
        "{measured}"
    );
}

#[test]
fn check_json_carries_measured_zones() {
    let dir = java_fixture();
    let json: serde_json::Value = serde_json::from_str(&stdout(
        dir.path(),
        &["check", "--measure-zones", "--format", "json"],
    ))
    .expect("json");
    assert_eq!(json["review_kpis"]["zones"]["main_sequence"], 2, "{json}");
    let plain: serde_json::Value =
        serde_json::from_str(&stdout(dir.path(), &["check", "--format", "json"])).expect("json");
    assert!(plain["review_kpis"]["zones"].is_null(), "{plain}");
}
