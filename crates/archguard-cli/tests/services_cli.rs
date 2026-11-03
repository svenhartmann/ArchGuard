use serde_json::Value;
use std::{fs, process::Command};

fn compose_project(api_cycle: bool) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for context in ["api", "worker"] {
        fs::create_dir(dir.path().join(context)).unwrap();
    }
    let endpoint = if api_cycle {
        "http://api:8080"
    } else {
        "https://external.example"
    };
    fs::write(dir.path().join("compose.yaml"), format!(
        "services:\n  api:\n    build: ./api\n    environment:\n      WORKER_URL: http://worker:8080\n    depends_on: [worker]\n  worker:\n    build: ./worker\n    environment:\n      API_URL: {endpoint}\n    depends_on: [api]\n"
    )).unwrap();
    // Services discovery is independent of the import-analysis configuration.
    fs::write(
        dir.path().join("archguard.toml"),
        "not a valid TOML document = [",
    )
    .unwrap();
    dir
}

#[test]
fn services_finds_configured_api_cycles_without_source_roots() {
    let dir = compose_project(true);
    let output = Command::new(env!("CARGO_BIN_EXE_archguard"))
        .args(["services", "--format", "json", "--fail-on-found"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["monorepo"], true);
    assert_eq!(value["services"].as_array().unwrap().len(), 2);
    let cycles = value["cycles"].as_array().unwrap();
    assert!(
        cycles
            .iter()
            .any(|cycle| cycle["kind"] == "configured_http")
    );
    assert!(cycles.iter().any(|cycle| cycle["kind"] == "startup"));
    assert_eq!(value["dependencies"].as_array().unwrap().len(), 4);
    assert!(
        !dir.path().join(".archguard").exists(),
        "service discovery must not invoke the import graph pipeline"
    );
}

#[test]
fn services_startup_cycles_do_not_fail_the_configured_api_gate() {
    let dir = compose_project(false);
    let output = Command::new(env!("CARGO_BIN_EXE_archguard"))
        .args(["services", "--fail-on-found"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("Configured HTTP cycle groups: 0"), "{text}");
    assert!(text.contains("Startup cycle groups: 1"), "{text}");
    assert!(text.contains("not observed runtime calls"), "{text}");
}

#[test]
fn services_quiet_preserves_exit_code_and_rejects_foreign_formats() {
    let dir = compose_project(true);
    let output = Command::new(env!("CARGO_BIN_EXE_archguard"))
        .args(["services", "--fail-on-found", "--quiet"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let output = Command::new(env!("CARGO_BIN_EXE_archguard"))
        .args(["services", "--format", "sarif"])
        .arg(dir.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn services_reports_manifest_apps_and_tools_without_compose_or_source_analysis() {
    let dir = tempfile::tempdir().unwrap();
    for (folder, name) in [
        ("functions/search", "search"),
        ("functions/import", "import"),
    ] {
        let path = dir.path().join(folder);
        fs::create_dir_all(&path).unwrap();
        fs::write(path.join("pom.xml"), format!("<project><modelVersion>4.0.0</modelVersion><artifactId>{name}</artifactId><build><plugins><plugin><groupId>com.microsoft.azure</groupId><artifactId>azure-functions-maven-plugin</artifactId></plugin></plugins></build></project>")).unwrap();
    }
    fs::create_dir(dir.path().join("tests")).unwrap();
    fs::write(dir.path().join("tests/package.json"), r#"{"name":"browser-tests","scripts":{"dev":"playwright test --ui","test":"playwright test"},"devDependencies":{"@playwright/test":"1.0.0"}}"#).unwrap();
    for format in ["json", "text"] {
        let output = Command::new(env!("CARGO_BIN_EXE_archguard"))
            .args(["services", "--format", format, "--fail-on-found"])
            .arg(dir.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        if format == "json" {
            let report: Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(report["monorepo"], true);
            assert_eq!(report["projects"].as_array().unwrap().len(), 3);
            assert_eq!(report["services"].as_array().unwrap().len(), 2);
            assert!(
                report["dependencies"].as_array().unwrap().is_empty(),
                "shared libraries do not prove API calls"
            );
            assert!(
                report["services"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|service| service["project"].is_string())
            );
        } else {
            let text = String::from_utf8(output.stdout).unwrap();
            assert!(text.contains("Projects: 3"), "{text}");
            assert!(
                text.contains("application") && text.contains("tooling"),
                "{text}"
            );
            assert!(
                text.contains("functions/search") && text.contains("tests/package.json"),
                "{text}"
            );
        }
    }
    assert!(
        !dir.path().join(".archguard").exists(),
        "read-only inventory must not build or scan imports"
    );
}
