use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::Value;

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn sarif(root: &Path) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_archguard"))
        .current_dir(root)
        .args(["check", "--format", "sarif"])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn config(roots: &str, from: &str, to: &str) -> String {
    format!(
        "derive_layers=false\n[project]\nsource_roots={roots}\nlanguages=['ts','java']\nexclude=[]\n[modules]\ndepth=0\n[rules]\ndeny_cycles='off'\nmax_scc_size=100\n[[dependency_rules]]\nname='boundary'\nfrom=['{from}']\nto=['{to}']\nseverity='error'\nmessage='Use the application API'\n"
    )
}

fn locations(value: &Value) -> &Vec<Value> {
    value["runs"][0]["results"]
        .as_array()
        .unwrap()
        .iter()
        .find(|result| result["ruleId"] == "archguard/dependency-rule")
        .unwrap()["locations"]
        .as_array()
        .unwrap()
}

#[test]
fn zero_result_sarif_retains_semantic_coverage_limitations() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), "main.ts", "export const answer = 42;\n");
    let output = Command::new(env!("CARGO_BIN_EXE_archguard"))
        .current_dir(root.path())
        .args(["check", "--format", "sarif"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(value["runs"][0]["results"].as_array().unwrap().is_empty());
    let properties = &value["runs"][0]["properties"];
    assert_eq!(properties["semantic"]["status"], "unavailable");
    assert_eq!(properties["absence_of_cycles_proven"], false);
    assert!(
        properties["dependency_scope"]
            .as_str()
            .unwrap()
            .contains("type-only references are not covered")
    );
}

#[test]
fn sarif_uses_real_source_root_path_and_percent_encodes_uri_characters() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), "src/ui/Space #%.ts", "import '../data/Db';\n");
    write(root.path(), "src/data/Db.ts", "export const db=1;\n");
    write(
        root.path(),
        "archguard.toml",
        &config("['src']", "ui/**", "data/**"),
    );
    let value = sarif(root.path());
    assert_eq!(
        locations(&value)[0]["physicalLocation"]["artifactLocation"]["uri"],
        "src/ui/Space%20%23%25.ts",
        "SARIF must point at the existing project-relative source, encoded as a URI"
    );
    assert_eq!(
        locations(&value)[0]["physicalLocation"]["region"]["startLine"],
        1
    );
}

#[test]
fn sarif_keeps_java_physical_path_instead_of_declared_package_path() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "src/main/java/physical/View.java",
        "package ui;\nimport data.Db;\nclass View {}\n",
    );
    write(
        root.path(),
        "src/main/java/unusual/Db.java",
        "package data;\npublic class Db {}\n",
    );
    write(
        root.path(),
        "archguard.toml",
        &config("['src/main/java']", "ui/**", "data/**"),
    );
    let value = sarif(root.path());
    assert_eq!(
        locations(&value)[0]["physicalLocation"]["artifactLocation"]["uri"],
        "src/main/java/physical/View.java"
    );
    assert_eq!(
        locations(&value)[0]["physicalLocation"]["region"]["startLine"],
        2
    );
}

#[test]
fn ambiguous_source_roots_use_logical_locations_instead_of_guessing_a_file() {
    let root = tempfile::tempdir().unwrap();
    for path in ["src/ui/View.ts", "other/ui/View.ts"] {
        write(root.path(), path, "import '../data/Db';\n");
    }
    write(root.path(), "src/data/Db.ts", "export const db=1;\n");
    write(
        root.path(),
        "archguard.toml",
        &config("['src','other']", "ui/**", "data/**"),
    );
    let value = sarif(root.path());
    assert!(
        locations(&value)
            .iter()
            .all(|location| location.get("physicalLocation").is_none()),
        "two possible physical files must not produce a fabricated SARIF file location"
    );
    assert!(
        locations(&value)
            .iter()
            .any(|location| location.get("logicalLocations").is_some())
    );
}
