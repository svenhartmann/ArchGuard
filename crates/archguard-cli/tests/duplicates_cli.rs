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

/// A TypeScript function of `8 + 9 * statements + 1` lexical tokens.
fn function(name: &str, var: &str, statements: usize) -> String {
    let mut out = format!("function {name}(a, b) {{\n");
    for i in 0..statements {
        out.push_str(&format!("  let {var}{i} = a + b * {i};\n"));
    }
    out.push_str("}\n");
    out
}
fn scan_fixture(files: &[(&str, String)], exclude: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (path, text) in files {
        let path = dir.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    fs::write(dir.path().join("archguard.toml"),format!("derive_layers=false\n[project]\nlanguages=['ts']\nsource_roots=['src']\nexclude=[{exclude}]\n[rules]\ndeny_cycles='off'\n")).unwrap();
    dir
}

#[test]
fn builtin_scan_reports_a_copied_function_and_reads_back_like_an_import() {
    let body = function("f", "x", 13);
    let dir = scan_fixture(
        &[
            ("src/a.ts", format!("const head = 1\n{body}")),
            ("src/b.ts", format!("const other = [1, 2]\n\n{body}")),
        ],
        "",
    );
    let root = dir.path();
    let scanned = value(call(root, &["--scan", "--format", "json"]));
    assert_eq!(scanned["status"], "ready", "{scanned:#}");
    assert_eq!(scanned["producer"], "ArchGuard token scan");
    assert!(scanned["report_path"].is_null());
    assert!(
        scanned["diagnostics"][0]
            .as_str()
            .unwrap()
            .contains("not proven semantic equivalence")
    );
    let groups = scanned["groups"].as_array().unwrap();
    assert_eq!(groups.len(), 1, "{scanned:#}");
    assert_eq!(groups[0]["tokens"], 126);
    let occurrences = groups[0]["occurrences"].as_array().unwrap();
    assert_eq!(occurrences.len(), 2);
    assert_eq!(occurrences[0]["path"], "src/a.ts");
    assert_eq!(occurrences[0]["node"], "a.ts");
    assert_eq!(occurrences[0]["line"], 2);
    assert_eq!(occurrences[1]["path"], "src/b.ts");
    assert_eq!(occurrences[1]["line"], 3);
    assert!(groups[0]["snippet"].as_str().unwrap().contains("let x12"));
    let read = value(call(root, &["--format", "json"]));
    assert_eq!(read["status"], "ready");
    assert_eq!(read["producer"], "ArchGuard token scan");
    assert_eq!(read["groups"], scanned["groups"]);
    let review = value(
        Command::new(assert_cmd::cargo::cargo_bin!("archguard"))
            .arg("review")
            .arg(root)
            .args(["--format", "json"])
            .output()
            .unwrap(),
    );
    assert_eq!(review["findings"][0]["category"], "duplication");
}

#[test]
fn builtin_scan_detects_renamed_copies_only_when_normalized() {
    let dir = scan_fixture(
        &[
            ("src/a.ts", function("f", "x", 13)),
            ("src/b.ts", function("g", "y", 13).replace("a + b", "p + q")),
        ],
        "",
    );
    let root = dir.path();
    let exact = value(call(root, &["--scan", "--format", "json"]));
    assert_eq!(exact["groups"].as_array().unwrap().len(), 0);
    let normalized = value(call(root, &["--scan", "--normalize", "--format", "json"]));
    assert_eq!(normalized["groups"].as_array().unwrap().len(), 1);
    assert!(
        normalized["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d.as_str().unwrap().contains("normalize=true"))
    );
}

#[test]
fn builtin_scan_respects_threshold_excludes_and_gitignore() {
    let body = function("f", "x", 13);
    let short = function("s", "z", 8);
    let dir = scan_fixture(
        &[
            ("src/a.ts", format!("{body}{short}")),
            ("src/generated/g.ts", body.clone()),
            ("src/vendor/v.ts", body.clone()),
            (".gitignore", "src/vendor/\n".into()),
            ("src/c.ts", format!("const c = 0\n{short}")),
        ],
        "'**/generated/**'",
    );
    let root = dir.path();
    let report = value(call(root, &["--scan", "--format", "json"]));
    assert_eq!(report["groups"].as_array().unwrap().len(), 0, "{report:#}");
    let lower = value(call(
        root,
        &["--scan", "--min-tokens", "60", "--format", "json"],
    ));
    let groups = lower["groups"].as_array().unwrap();
    assert_eq!(groups.len(), 1, "{lower:#}");
    let paths: Vec<_> = groups[0]["occurrences"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, ["src/a.ts", "src/c.ts"]);
    assert_eq!(
        call(root, &["--scan", "--min-tokens", "0"]).status.code(),
        Some(2)
    );
}

#[test]
fn builtin_scan_never_silently_replaces_an_imported_cpd_report() {
    let dir = fixture();
    let root = dir.path();
    fs::write(root.join("cpd.xml"), xml()).unwrap();
    value(call(root, &["--report", "cpd.xml", "--format", "json"]));
    let saved = fs::read(root.join(".archguard/duplicates.json")).unwrap();
    let refused = call(root, &["--scan", "--format", "json"]);
    assert_eq!(refused.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&refused.stderr).contains("--replace"));
    assert_eq!(
        fs::read(root.join(".archguard/duplicates.json")).unwrap(),
        saved
    );
    assert_eq!(value(call(root, &["--format", "json"]))["producer"], "CPD");
    let replaced = value(call(root, &["--scan", "--replace", "--format", "json"]));
    assert_eq!(replaced["producer"], "ArchGuard token scan");
    // A later scan may refresh its own snapshot without --replace.
    value(call(root, &["--scan", "--format", "json"]));
    assert_eq!(
        call(root, &["--scan", "--report", "cpd.xml"]).status.code(),
        Some(2)
    );
}
