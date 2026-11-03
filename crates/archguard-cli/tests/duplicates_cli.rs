use serde_json::Value;
use std::{fs, path::Path, process::Command};
fn call(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(assert_cmd::cargo::cargo_bin!("archguard"))
        .arg("duplicates")
        .arg(root)
        .args(args)
        .output()
        .unwrap()
}
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    for name in ["a.ts", "b.ts"] {
        fs::write(
            dir.path().join("src").join(name),
            "export const a=1;\nexport const b=2;\nexport const c=3;\n",
        )
        .unwrap();
    }
    fs::write(dir.path().join("archguard.toml"),"derive_layers=false\n[project]\nlanguages=['ts']\nsource_roots=['src']\nexclude=[]\n[rules]\ndeny_cycles='off'\n").unwrap();
    dir
}
fn xml() -> String {
    "<pmd-cpd xmlns='https://pmd-code.org/schema/cpd-report' version='1.0.0' pmdVersion='7.3.0'><duplication lines='2' tokens='12'><file path='src/a.ts' line='1' endline='2'/><file path='src/b.ts' line='1' endline='2'/><codefragment><![CDATA[<not-html> source]]></codefragment></duplication></pmd-cpd>".into()
}
fn value(out: std::process::Output) -> Value {
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
#[test]
fn cpd_groups_preserve_every_location_and_become_review_findings_with_freshness() {
    let dir = fixture();
    let root = dir.path();
    assert_eq!(
        value(call(root, &["--format", "json"]))["status"],
        "not_imported"
    );
    assert!(!root.join(".archguard/duplicates.json").exists());
    fs::write(root.join("cpd.xml"), xml()).unwrap();
    let report = value(call(root, &["--report", "cpd.xml", "--format", "json"]));
    assert!(
        fs::read_to_string(root.join(".archguard/.gitignore"))
            .unwrap()
            .lines()
            .any(|line| line == "/duplicates.json")
    );
    assert_eq!(report["status"], "ready");
    assert_eq!(report["groups"].as_array().unwrap().len(), 1);
    let group = &report["groups"][0];
    assert_eq!(group["occurrences"].as_array().unwrap().len(), 2);
    assert_eq!(group["occurrences"][0]["node"], "a.ts");
    assert_eq!(group["snippet"], "<not-html> source");
    let review = || {
        value(
            Command::new(assert_cmd::cargo::cargo_bin!("archguard"))
                .arg("review")
                .arg(root)
                .args(["--format", "json"])
                .output()
                .unwrap(),
        )
    };
    let before = review();
    let finding = &before["findings"][0];
    assert_eq!(finding["category"], "duplication");
    assert_eq!(finding["nodes"], serde_json::json!(["a.ts", "b.ts"]));
    assert_eq!(
        finding["evidence"]["group"]["occurrences"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    fs::write(
        root.join("src/b.ts"),
        "export const a=9;\nexport const b=2;\nexport const c=3;\n",
    )
    .unwrap();
    let stale = value(call(root, &["--format", "json"]));
    assert_eq!(stale["status"], "stale");
    assert_eq!(
        stale["groups"][0]["occurrences"][1]["status"],
        "changed_since_import"
    );
    let after = review();
    assert_eq!(after["findings"][0]["id"], finding["id"]);
    assert_ne!(after["findings"][0]["fingerprint"], finding["fingerprint"]);
    assert_eq!(after["findings"][0]["source_status"], "stale");
}
#[test]
fn unsafe_or_invalid_cpd_never_replaces_the_previous_snapshot() {
    let dir = fixture();
    let root = dir.path();
    fs::write(root.join("cpd.xml"), xml()).unwrap();
    value(call(root, &["--report", "cpd.xml", "--format", "json"]));
    let saved = fs::read(root.join(".archguard/duplicates.json")).unwrap();
    for invalid in [
        xml().replace("src/a.ts", "../outside.ts"),
        xml().replace("line='1'", "line='0'"),
        xml().replace("endline='2'", "endline='999'"),
        xml().replace("src/b.ts", "src/a.ts"),
        "<!DOCTYPE pmd-cpd [<!ENTITY x SYSTEM 'file:///etc/passwd'>]><pmd-cpd>&x;</pmd-cpd>".into(),
    ] {
        fs::write(root.join("cpd.xml"), invalid).unwrap();
        assert_eq!(call(root, &["--report", "cpd.xml"]).status.code(), Some(3));
        assert_eq!(
            fs::read(root.join(".archguard/duplicates.json")).unwrap(),
            saved
        );
    }
}
#[test]
fn cpd_supports_local_absolute_paths_same_file_copies_and_partial_producer_runs() {
    let dir = fixture();
    let root = dir.path();
    let absolute = root.join("src/a.ts").canonicalize().unwrap();
    let text = format!(
        "<pmd-cpd><duplication lines='1' tokens='6'><file path='{}' line='1'/><file path='{}' line='2'/><codefragment>source</codefragment></duplication><error filename='skipped.ts' msg='lexing failed'/></pmd-cpd>",
        absolute.display(),
        absolute.display()
    );
    fs::write(root.join("cpd.xml"), text).unwrap();
    let report = value(call(root, &["--report", "cpd.xml", "--format", "json"]));
    assert_eq!(report["status"], "partial");
    assert_eq!(report["producer_errors"], 1);
    assert_eq!(report["groups"][0]["occurrences"][0]["path"], "src/a.ts");
    assert_eq!(report["groups"][0]["occurrences"][1]["line"], 2);
}

#[test]
fn cpd_source_root_ordering_and_missing_or_corrupt_evidence_remain_explicit() {
    let dir = fixture();
    let root = dir.path();
    fs::write(root.join("cpd.xml"), xml().replace("src/", "")).unwrap();
    let report = value(call(
        root,
        &[
            "--report",
            "cpd.xml",
            "--source-root",
            "src",
            "--format",
            "json",
        ],
    ));
    assert_eq!(report["status"], "ready");
    let reordered = xml()
        .replace("src/a.ts", "src/TEMP.ts")
        .replace("src/b.ts", "src/a.ts")
        .replace("src/TEMP.ts", "src/b.ts");
    fs::write(root.join("cpd.xml"), reordered).unwrap();
    let imported = value(call(root, &["--report", "cpd.xml", "--format", "json"]));
    assert_eq!(report["groups"][0]["id"], imported["groups"][0]["id"]);
    fs::remove_file(root.join("src/b.ts")).unwrap();
    let missing = value(call(root, &["--format", "json"]));
    assert_eq!(missing["status"], "partial");
    assert_eq!(missing["groups"][0]["occurrences"][1]["status"], "unmapped");
    assert!(missing["groups"][0]["occurrences"][1]["node"].is_null());
    let mut corrupt = imported;
    corrupt["groups"][0]["occurrences"][0]["path"] = "../outside.ts".into();
    fs::write(
        root.join(".archguard/duplicates.json"),
        serde_json::to_vec(&corrupt).unwrap(),
    )
    .unwrap();
    let output = call(root, &["--format", "json"]);
    assert_eq!(
        output.status.code(),
        Some(3),
        "corrupt evidence is an analysis error"
    );
    let failed: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(failed["status"], "error");
    assert_eq!(failed["groups"].as_array().unwrap().len(), 0);
}
