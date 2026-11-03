use serde_json::Value;
use std::{fs, path::Path, process::Command};

fn run(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(assert_cmd::cargo::cargo_bin!("archguard"))
        .arg("workspace")
        .arg(root)
        .args(args)
        .output()
        .unwrap()
}
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for project in ["app", "lib"] {
        fs::create_dir(dir.path().join(project)).unwrap();
    }
    fs::write(
        dir.path().join("app/package.json"),
        r#"{"name":"app","version":"1.0.0","dependencies":{"lib":"file:../lib"}}"#,
    )
    .unwrap();
    fs::write(
        dir.path().join("lib/package.json"),
        r#"{"name":"lib","version":"1.0.0"}"#,
    )
    .unwrap();
    dir
}
#[test]
fn workspace_manifest_json_and_html_keep_declared_edges_and_do_not_write_project_state() {
    let dir = fixture();
    let result = run(dir.path(), &["--format", "json"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let data: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(data["workspace"]["projects"].as_array().unwrap().len(), 2);
    let edges = data["workspace"]["dependencies"].as_array().unwrap();
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0]["from"], "project:app/package.json");
    assert_eq!(edges[0]["to"], "project:lib/package.json");
    assert!(data["sources"].is_null() && data["impact"].is_null());
    let page = run(dir.path(), &["--format", "html"]);
    assert!(
        page.status.success(),
        "{}",
        String::from_utf8_lossy(&page.stderr)
    );
    let page = String::from_utf8(page.stdout).unwrap();
    assert!(page.contains("project:app/package.json") && page.contains("workspace"));
    assert!(!dir.path().join(".archguard").exists());
    assert!(!dir.path().join("app/.archguard").exists());
}
#[test]
fn workspace_rejects_inapplicable_options_before_analysis() {
    let dir = fixture();
    for args in [
        vec!["--target", "app"],
        vec!["--format", "sarif"],
        vec!["--config", "missing.toml"],
    ] {
        let result = run(dir.path(), &args);
        assert_eq!(
            result.status.code(),
            Some(2),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    assert!(!dir.path().join(".archguard").exists());
}

#[test]
fn workspace_sources_keep_duplicate_file_names_separate_and_targets_resolve_to_files() {
    let dir = fixture();
    for project in ["app", "lib"] {
        fs::write(dir.path().join(format!("{project}/same.py")), "value = 1\n").unwrap();
    }
    let result = run(
        dir.path(),
        &["--sources", "--target", "lib/same.py", "--format", "json"],
    );
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let data: Value = serde_json::from_slice(&result.stdout).unwrap();
    let files = data["sources"]["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    assert_ne!(files[0]["id"], files[1]["id"]);
    assert_eq!(data["impact"]["unresolved_targets"], serde_json::json!([]));
    assert_eq!(data["impact"]["safe_to_skip_other_tests"], false);
    for project in ["app", "lib"] {
        assert!(!dir.path().join(format!("{project}/.archguard")).exists());
    }
}
