use serde_json::Value;
use std::{fs, process::Command};
#[test]
fn analysis_commands_return_evidence_and_explicit_coverage() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("archguard.toml"),
        "derive_layers=false\n[project]\nsource_roots=[]\nlanguages=['py']\n",
    )
    .unwrap();
    fs::write(dir.path().join("service.py"), "import pandas\n").unwrap();
    fs::write(dir.path().join("CODEOWNERS"), "*.py @example/team\n").unwrap();
    for command in ["co-change", "ownership", "performance"] {
        let out = Command::new(env!("CARGO_BIN_EXE_archguard"))
            .current_dir(dir.path())
            .args([command, "--format", "json"])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{command}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let value: Value = serde_json::from_slice(&out.stdout).unwrap();
        match command {
            "co-change" => assert_eq!(value["git"]["status"], "unavailable"),
            "ownership" => assert_eq!(value["assignments"][0]["owners"][0], "@example/team"),
            _ => assert!(
                value["source_findings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|f| f["kind"] == "heavy_import" && f["line"] == 1)
            ),
        }
    }
}
