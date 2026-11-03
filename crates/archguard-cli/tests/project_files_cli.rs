use serde_json::Value;
use std::{fs, process::Command};
use tempfile::TempDir;

fn project() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.ts"), "import './b';\n").unwrap();
    fs::write(dir.path().join("b.ts"), "export const b = 1;\n").unwrap();
    dir
}
fn run(cwd: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(assert_cmd::cargo::cargo_bin!("archguard"))
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap()
}
fn success(result: &std::process::Output) {
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn init_stores_config_and_generated_ignore_rules_inside_project() {
    let dir = project();
    success(&run(dir.path(), &["init", "--defaults"]));
    let path = dir.path().join(".archguard/archguard.toml");
    assert!(
        path.is_file(),
        "init must use the project-local config directory"
    );
    assert!(!dir.path().join("archguard.toml").exists());
    let original = fs::read_to_string(&path).unwrap();
    assert!(original.contains("deny_cycles"));
    let ignore = fs::read_to_string(dir.path().join(".archguard/.gitignore")).unwrap();
    assert!(ignore.contains("/reports/"));
    assert!(ignore.contains("/graph.json"));
    assert!(
        !ignore.contains("archguard.toml")
            && !ignore.contains("baseline.json")
            && !ignore.contains("scenarios")
    );
    let again = run(dir.path(), &["init"]);
    assert_eq!(again.status.code(), Some(3));
    assert_eq!(fs::read_to_string(path).unwrap(), original);
}

#[test]
fn init_does_not_shadow_an_existing_legacy_config() {
    let dir = project();
    fs::write(dir.path().join("archguard.toml"), "derive_layers = false\n").unwrap();
    let output = run(dir.path(), &["init", "--defaults"]);
    assert_eq!(output.status.code(), Some(3));
    assert!(!dir.path().join(".archguard/archguard.toml").exists());
    assert_eq!(
        fs::read_to_string(dir.path().join("archguard.toml")).unwrap(),
        "derive_layers = false\n"
    );
}

#[test]
fn scan_default_is_relative_to_analyzed_project_not_callers_directory() {
    let dir = project();
    let caller = tempfile::tempdir().unwrap();
    success(&run(caller.path(), &["scan", dir.path().to_str().unwrap()]));
    let data: Value = serde_json::from_slice(
        &fs::read(dir.path().join(".archguard/graph.json"))
            .expect("graph belongs to analyzed project"),
    )
    .unwrap();
    assert_eq!(data["edges"].as_array().unwrap().len(), 1);
    assert!(!caller.path().join(".archguard").exists());
    assert!(
        dir.path()
            .join(".archguard/effective-config.json")
            .is_file()
    );
}

#[test]
fn graph_exports_are_saved_without_changing_stdout_and_quiet_still_saves() {
    let dir = project();
    let caller = tempfile::tempdir().unwrap();
    for (format, extension) in [
        ("dot", "dot"),
        ("json", "json"),
        ("mermaid", "mmd"),
        ("graphml", "graphml"),
        ("gexf", "gexf"),
        ("html", "html"),
    ] {
        let result = run(
            caller.path(),
            &["graph", dir.path().to_str().unwrap(), "--format", format],
        );
        success(&result);
        let path = dir
            .path()
            .join(format!(".archguard/reports/graph.{extension}"));
        assert_eq!(
            fs::read_to_string(&path).expect("default export must be saved"),
            String::from_utf8(result.stdout).unwrap()
        );
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let result = run(
            caller.path(),
            &[
                "graph",
                dir.path().to_str().unwrap(),
                "--format",
                format,
                "--quiet",
            ],
        );
        success(&result);
        assert!(result.stdout.is_empty());
        assert_eq!(
            fs::metadata(path).unwrap().modified().unwrap(),
            modified,
            "unchanged exports must not be rewritten"
        );
    }
    let source: Value =
        serde_json::from_slice(&fs::read(dir.path().join(".archguard/graph.json")).unwrap())
            .unwrap();
    assert_eq!(source["edges"].as_array().unwrap().len(), 1);
    assert!(!caller.path().join(".archguard").exists());
}

#[test]
fn explicit_scan_output_remains_an_override_and_effective_config_is_not_intent() {
    let dir = project();
    let caller = tempfile::tempdir().unwrap();
    let config = caller.path().join("policy.toml");
    fs::write(
        &config,
        "derive_layers=false\n[project]\nsource_roots=[]\nlanguages=['ts']\n[modules]\ndepth=0\n",
    )
    .unwrap();
    success(&run(
        caller.path(),
        &[
            "scan",
            dir.path().to_str().unwrap(),
            "--out",
            "chosen.json",
            "--config",
            config.to_str().unwrap(),
        ],
    ));
    assert!(caller.path().join("chosen.json").is_file());
    let effective: Value = serde_json::from_slice(
        &fs::read(dir.path().join(".archguard/effective-config.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(effective["config"]["project"]["languages"][0], "ts");
    assert_eq!(effective["sources"]["project"], "declared");
    assert!(
        !dir.path().join(".archguard/archguard.toml").exists(),
        "derived snapshots must not become declared intent automatically"
    );
}

#[test]
fn generated_files_and_user_policy_survive_failed_analysis() {
    let dir = project();
    success(&run(dir.path(), &["graph", "--format", "html"]));
    let report = dir.path().join(".archguard/reports/graph.html");
    let before = fs::read(&report).expect("saved report");
    fs::write(
        dir.path().join(".archguard/archguard.toml"),
        "invalid = [\n",
    )
    .unwrap();
    let result = run(dir.path(), &["graph", "--format", "html"]);
    assert_eq!(result.status.code(), Some(3));
    assert_eq!(fs::read(report).unwrap(), before);
}

#[test]
fn cycle_diagrams_are_saved_even_when_quiet_and_keep_the_gate_exit_code() {
    let dir = project();
    fs::write(dir.path().join("b.ts"), "import './a';\n").unwrap();
    let caller = tempfile::tempdir().unwrap();
    for (format, extension) in [("html", "html"), ("mermaid", "mmd")] {
        let result = run(
            caller.path(),
            &[
                "cycles",
                dir.path().to_str().unwrap(),
                "--format",
                format,
                "--fail-on-found",
                "--quiet",
            ],
        );
        assert_eq!(result.status.code(), Some(1));
        assert!(result.stdout.is_empty());
        let saved = fs::read_to_string(
            dir.path()
                .join(format!(".archguard/reports/cycles.{extension}")),
        )
        .expect("cycle diagram must be saved inside the analyzed project");
        let result = run(
            caller.path(),
            &["cycles", dir.path().to_str().unwrap(), "--format", format],
        );
        success(&result);
        assert_eq!(saved, String::from_utf8(result.stdout).unwrap());
    }
    assert!(dir.path().join(".archguard/graph.json").is_file());
    assert!(
        dir.path()
            .join(".archguard/effective-config.json")
            .is_file()
    );
    assert!(!caller.path().join(".archguard").exists());
}
