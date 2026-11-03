//! The CLI adapter stores dependency-advisory snapshots without analyzing source code.
#![cfg(unix)]
use serde_json::Value;
use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};
use tempfile::{TempDir, tempdir};

fn fixture() -> (TempDir, TempDir) {
    let root = tempdir().unwrap();
    fs::write(root.path().join("Cargo.lock"), "version = 4\n[[package]]\nname = 'demo-dep'\nversion = '1.0.0'\nsource = 'registry+https://github.com/rust-lang/crates.io-index'\n").unwrap();
    let bin = tempdir().unwrap();
    let scanner = bin.path().join("trivy");
    fs::write(&scanner, r#"#!/bin/sh
if [ "$ARCHGUARD_TEST_FAIL" = 1 ]; then echo 'fixture scanner unavailable' >&2; exit 2; fi
case "$1" in
  version) printf '%s\n' '{"Version":"0.76.0","VulnerabilityDB":{"UpdatedAt":"2026-10-27T08:00:00Z"}}';;
  *) printf '%s\n' '{"SchemaVersion":2,"CreatedAt":"2026-10-27T09:00:00Z","Results":[{"Target":"Cargo.lock","Class":"lang-pkgs","Type":"cargo","Packages":[{"Name":"demo-dep","Version":"1.0.0"}],"Vulnerabilities":[{"VulnerabilityID":"CVE-2026-11111","PkgName":"demo-dep","InstalledVersion":"1.0.0","FixedVersion":"1.0.1","Severity":"HIGH","Title":"Synthetic test advisory","PrimaryURL":"https://example.invalid/advisory/CVE-2026-11111"}]}]}' ;;
esac
"#).unwrap();
    fs::set_permissions(scanner, fs::Permissions::from_mode(0o700)).unwrap();
    (root, bin)
}
fn run(root: &Path, bin: &Path, args: &[&str], fail: bool) -> std::process::Output {
    Command::new(assert_cmd::cargo::cargo_bin!("archguard"))
        .current_dir(bin)
        .arg("security")
        .arg(root)
        .args(args)
        .env("PATH", bin)
        .env("ARCHGUARD_TEST_FAIL", if fail { "1" } else { "0" })
        .output()
        .unwrap()
}
fn data(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "{error}; stderr {}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}
#[test]
fn missing_cached_scan_is_explicit_and_never_a_successful_gate() {
    let (root, bin) = fixture();
    let result = run(
        root.path(),
        bin.path(),
        &["--cached", "--format", "json"],
        false,
    );
    assert_eq!(
        result.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(data(&result)["status"], "not_scanned");
    assert!(!root.path().join(".archguard/security.json").exists());
    assert!(!root.path().join(".archguard/cache.json").exists());
}
#[test]
fn fresh_scan_saves_in_analyzed_project_and_gate_counts_real_findings() {
    let (root, bin) = fixture();
    let result = run(root.path(), bin.path(), &["--format", "json"], false);
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let report = data(&result);
    assert_eq!(report["status"], "ready");
    assert_eq!(report["packages_scanned"], 1);
    assert_eq!(report["findings"][0]["id"], "CVE-2026-11111");
    assert_eq!(report["findings"][0]["fixed_version"], "1.0.1");
    assert_eq!(
        serde_json::from_slice::<Value>(
            &fs::read(root.path().join(".archguard/security.json")).unwrap()
        )
        .unwrap(),
        report
    );
    assert!(!bin.path().join(".archguard/security.json").exists());
    assert!(
        !root.path().join(".archguard/cache.json").exists(),
        "dependency-only scan must not require source extraction"
    );
    assert!(
        fs::read_to_string(root.path().join(".archguard/.gitignore"))
            .unwrap()
            .contains("/security.json")
    );
    let result = run(
        root.path(),
        bin.path(),
        &["--cached", "--fail-on-found", "--quiet"],
        true,
    );
    assert_eq!(
        result.status.code(),
        Some(1),
        "cached mode must not execute the failing scanner"
    );
    assert!(result.stdout.is_empty());
}
#[test]
fn failure_preserves_snapshot_and_changed_dependencies_mark_it_stale() {
    let (root, bin) = fixture();
    assert!(
        run(root.path(), bin.path(), &["--quiet"], false)
            .status
            .success()
    );
    let before = fs::read(root.path().join(".archguard/security.json")).unwrap();
    let result = run(root.path(), bin.path(), &["--quiet"], true);
    assert_eq!(result.status.code(), Some(3));
    assert_eq!(
        fs::read(root.path().join(".archguard/security.json")).unwrap(),
        before
    );
    fs::write(root.path().join("Cargo.lock"), "version = 4\n").unwrap();
    let result = run(
        root.path(),
        bin.path(),
        &["--cached", "--format", "json", "--fail-on-found"],
        false,
    );
    assert_eq!(result.status.code(), Some(3));
    let report = data(&result);
    assert_eq!(report["status"], "stale");
    assert_eq!(report["findings"].as_array().unwrap().len(), 1);
}
#[test]
fn scanner_specific_options_and_unsupported_formats_fail_before_scanning() {
    let (root, bin) = fixture();
    for args in [vec!["--format", "html"], vec!["--cached", "--offline"]] {
        let result = run(root.path(), bin.path(), &args, false);
        assert_eq!(result.status.code(), Some(2));
        assert!(!root.path().join(".archguard/security.json").exists());
    }
}
