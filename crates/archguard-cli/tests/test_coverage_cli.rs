use serde_json::Value;
use std::{fs, path::Path, process::Command};

fn run(root: &Path, command: &str, extra: &[&str]) -> std::process::Output {
    Command::new(assert_cmd::cargo::cargo_bin!("archguard"))
        .arg(command)
        .arg(root)
        .args(extra)
        .output()
        .unwrap()
}
fn value(out: &std::process::Output) -> Value {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
#[test]
fn coverage_import_survives_reload_reaches_html_and_excludes_stale_scores() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("lib")).unwrap();
    fs::write(root.path().join("lib/a.ts"), "export const a=1;\n").unwrap();
    fs::write(
        root.path().join("lib/b.ts"),
        "import {a} from './a';\nexport const b=a;\n",
    )
    .unwrap();
    fs::write(root.path().join("lib/c.ts"), "import {b} from './b';\n").unwrap();
    fs::write(root.path().join("archguard.toml"), "derive_layers=false\n[project]\nsource_roots=[]\nlanguages=['ts']\nexclude=[]\n[rules]\ndeny_cycles='off'\n").unwrap();
    let absent = value(&run(root.path(), "coverage", &["--format", "json"]));
    assert_eq!(absent["status"], "not_imported");
    assert!(absent["review"]["files"].as_array().unwrap().is_empty());
    assert!(!root.path().join(".archguard/coverage.json").exists());
    let report = root.path().join("coverage.xml");
    fs::write(&report,r#"<coverage><packages><package name="lib"><classes><class name="a" filename="lib/a.ts"><lines><line number="1" hits="0"/></lines></class><class name="b" filename="lib/b.ts"><lines><line number="1" hits="1"/><line number="2" hits="1"/></lines></class></classes></package></packages></coverage>"#).unwrap();
    let imported = value(&run(
        root.path(),
        "coverage",
        &["--report", report.to_str().unwrap(), "--format", "json"],
    ));
    assert_eq!(imported["format"], "cobertura");
    assert_eq!(imported["files"].as_array().unwrap().len(), 2);
    let rows = imported["review"]["files"].as_array().unwrap();
    assert_eq!(rows[0]["node"], "lib/a.ts");
    assert_eq!(rows[0]["dependents"], 2);
    assert_eq!(rows[0]["score"], 3.0);
    assert_eq!(rows[1]["score"], 0.0);
    let read = value(&run(root.path(), "coverage", &["--format", "json"]));
    assert_eq!(read, imported);
    let html = run(root.path(), "graph", &["--format", "html"]);
    assert!(
        html.status.success(),
        "{}",
        String::from_utf8_lossy(&html.stderr)
    );
    let page = String::from_utf8(html.stdout).unwrap();
    let payload = page
        .split("<script id=\"archguard-data\" type=\"application/json\">")
        .nth(1)
        .unwrap()
        .split("</script>")
        .next()
        .unwrap();
    let data: Value = serde_json::from_str(payload).unwrap();
    assert_eq!(data["insights"]["test_coverage"]["files"], read["files"]);
    assert_eq!(data["insights"]["coverage_review"], read["review"]);
    fs::write(root.path().join("lib/a.ts"), "export const a=2;\n").unwrap();
    let stale = value(&run(root.path(), "coverage", &["--format", "json"]));
    assert_eq!(stale["status"], "stale");
    let changed = stale["review"]["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["node"] == "lib/a.ts")
        .unwrap();
    assert!(
        changed["score"].is_null(),
        "changed source must not retain a current risk score"
    );
    let saved = fs::read(root.path().join(".archguard/coverage.json")).unwrap();
    fs::write(&report, "<coverage>invalid").unwrap();
    assert_eq!(
        run(
            root.path(),
            "coverage",
            &["--report", report.to_str().unwrap()]
        )
        .status
        .code(),
        Some(3)
    );
    assert_eq!(
        fs::read(root.path().join(".archguard/coverage.json")).unwrap(),
        saved
    );
}

#[test]
fn lcov_tracefile_imports_with_the_lcov_format() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("lib")).unwrap();
    fs::write(root.path().join("lib/a.ts"), "export const a=1;\n").unwrap();
    fs::write(root.path().join("archguard.toml"), "derive_layers=false\n[project]\nsource_roots=[]\nlanguages=['ts']\nexclude=[]\n[rules]\ndeny_cycles='off'\n").unwrap();
    let report = root.path().join("lcov.info");
    fs::write(
        &report,
        "TN:\nSF:lib/a.ts\nDA:1,2\nLF:1\nLH:1\nend_of_record\n",
    )
    .unwrap();
    let imported = value(&run(
        root.path(),
        "coverage",
        &["--report", report.to_str().unwrap(), "--format", "json"],
    ));
    assert_eq!(imported["format"], "lcov");
    assert_eq!(imported["files"][0]["node"], "lib/a.ts");
    assert_eq!(imported["files"][0]["lines"]["covered"], 1);
    assert_eq!(imported["files"][0]["lines"]["total"], 1);
}
