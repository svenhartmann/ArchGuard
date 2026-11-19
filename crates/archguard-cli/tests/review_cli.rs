use serde_json::Value;
use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn hotspots_exposes_ranked_evidence_without_claiming_missing_git_is_zero() {
    let root = tempdir().unwrap();
    for (path, source) in [
        ("a.ts", "import './b';\n"),
        ("b.ts", "import './a';\n"),
        ("unused.ts", "export const unused = 1;\n"),
    ] {
        fs::write(root.path().join(path), source).unwrap();
    }
    fs::write(
        root.path().join("archguard.toml"),
        "derive_layers=false\n[project]\nsource_roots=[]\nlanguages=['ts']\n[modules]\ndepth=0\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_archguard"))
        .args(["hotspots", "--format", "json"])
        .arg(root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["nodes"].as_array().unwrap().len(), 3);
    assert_eq!(value["nodes"][0]["name"], "a.ts");
    assert!(value["nodes"][0]["score"].as_u64().unwrap() > 0);
    assert!(value["nodes"][0]["changes"].is_null());
    assert_eq!(value["nodes"][2]["name"], "unused.ts");
    assert_eq!(value["nodes"][2]["orphan"], "isolated");
}
