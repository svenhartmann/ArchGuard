use assert_cmd::Command;
#[test]
fn architecture_command_explains_absent_model_without_fabricating_one() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.py"), "x=1\n").unwrap();
    let output = Command::new(assert_cmd::cargo::cargo_bin!("archguard"))
        .args([
            "architecture",
            dir.path().to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["status"], "not_configured");
    assert!(result["components"].is_null());
}

#[test]
fn component_contracts_expose_private_access_and_coupling_violations_in_all_interfaces() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("ui")).unwrap();
    std::fs::create_dir(dir.path().join("core")).unwrap();
    std::fs::write(
        dir.path().join("ui/page.ts"),
        "import '../core/internal';\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("core/internal.ts"), "export const x=1;\n").unwrap();
    std::fs::write(dir.path().join("archguard.toml"),"derive_layers=false\n[project]\nlanguages=['ts']\nsource_roots=[]\n[rules]\ndeny_cycles='off'\n[architecture.model]\nname='Boundaries'\n[[architecture.model.components]]\nname='Web'\ninclude=['ui/**']\n[[architecture.model.components]]\nname='Domain'\ninclude=['core/**']\npublic=[]\nmax_incoming_components=0\n[[architecture.model.allowed_dependencies]]\nfrom='Web'\nto='Domain'\n").unwrap();
    let run = |command: &str, format: &str| {
        Command::new(assert_cmd::cargo::cargo_bin!("archguard"))
            .args([command, dir.path().to_str().unwrap(), "--format", format])
            .output()
            .unwrap()
    };
    let output = run("architecture", "text");
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("Private source accesses: 1"), "{text}");
    assert!(text.contains("Coupling limit violations: 1"), "{text}");
    assert!(text.contains("ui/page.ts → core/internal.ts"), "{text}");
    let output = run("check", "sarif");
    assert_eq!(output.status.code(), Some(1));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["runs"][0]["results"].as_array().unwrap().len(), 2);
    let output = run("review", "json");
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["findings"].as_array().unwrap().len(), 2);
    assert!(
        value["findings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["category"] == "architecture")
    );
}

#[test]
fn logical_model_is_queryable_and_violations_reach_check_text_and_sarif() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["ui", "storage"] {
        std::fs::create_dir(dir.path().join(name)).unwrap();
    }
    std::fs::write(
        dir.path().join("ui/A.ts"),
        "import {b} from '../storage/B';\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("storage/B.ts"), "export const b=1;\n").unwrap();
    std::fs::write(dir.path().join("archguard.toml"),"derive_layers=false\n[project]\nsource_roots=[]\nlanguages=['ts']\nexclude=[]\n[rules]\ndeny_cycles='off'\n[architecture.model]\nname='Boundary'\n[[architecture.model.components]]\nname='Presentation'\ninclude=['ui/**']\n[[architecture.model.components]]\nname='Data'\ninclude=['storage/**']\n").unwrap();
    let run = |command: &str, format: &str| {
        Command::new(assert_cmd::cargo::cargo_bin!("archguard"))
            .args([command, dir.path().to_str().unwrap(), "--format", format])
            .output()
            .unwrap()
    };
    let output = run("architecture", "json");
    assert!(output.status.success());
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["status"], "complete");
    assert_eq!(result["violating_dependencies"], 1);
    assert_eq!(result["dependencies"][0]["from"], "Presentation");
    assert_eq!(result["dependencies"][0]["evidence"][0]["line"], 1);
    let output = run("check", "json");
    assert_eq!(output.status.code(), Some(1));
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["findings"][0]["kind"], "target_architecture");
    let output = run("check", "text");
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stdout).contains("target architecture violation"));
    let output = run("check", "sarif");
    assert_eq!(output.status.code(), Some(1));
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        result["runs"][0]["results"][0]["ruleId"],
        "archguard/target-architecture"
    );
    assert_eq!(
        result["runs"][0]["results"][0]["locations"][0]["physicalLocation"]["artifactLocation"]["uri"],
        "ui/A.ts"
    );
    assert_eq!(
        result["runs"][0]["results"][0]["locations"][0]["physicalLocation"]["region"]["startLine"],
        1
    );
}
