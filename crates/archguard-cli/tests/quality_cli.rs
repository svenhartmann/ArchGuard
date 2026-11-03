use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};

fn run(root: &Path, command: &str, extra: &[&str]) -> std::process::Output {
    Command::new(assert_cmd::cargo::cargo_bin!("archguard"))
        .arg(command)
        .arg(root)
        .args(extra)
        .output()
        .unwrap()
}
fn value(out: &std::process::Output, code: i32) -> Value {
    assert_eq!(
        out.status.code(),
        Some(code),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}
fn project() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("lib")).unwrap();
    fs::write(root.path().join("lib/a.ts"), "export const a=1;\n").unwrap();
    fs::write(root.path().join("archguard.toml"), "derive_layers=false\n[project]\nsource_roots=[]\nlanguages=['ts']\nexclude=[]\n[rules]\ndeny_cycles='off'\n").unwrap();
    root
}
fn sarif(root: &Path) {
    fs::write(root.join("lint.sarif"),serde_json::to_vec(&json!({
        "version":"2.1.0","runs":[{"tool":{"driver":{"name":"ExampleLint","rules":[{"id":"EX001","helpUri":"https://example.test/rules/EX001"}]}},
        "results":[{"ruleId":"EX001","level":"warning","message":{"text":"Review this expression"},"locations":[{"physicalLocation":{"artifactLocation":{"uri":"lib/a.ts"},"region":{"startLine":1,"startColumn":1}}}]}]}]
    })).unwrap()).unwrap();
}
fn git(root: &Path, args: &[&str]) {
    let out = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
#[test]
fn quality_import_read_stale_and_invalid_import_preserve_evidence() {
    let root = project();
    let empty = value(&run(root.path(), "quality", &["--format", "json"]), 0);
    assert_eq!(empty["report"]["status"], "not_imported");
    assert!(!root.path().join(".archguard/quality.json").exists());
    sarif(root.path());
    let imported = value(
        &run(
            root.path(),
            "quality",
            &["--report", "lint.sarif", "--format", "json"],
        ),
        0,
    );
    assert_eq!(imported["report"]["status"], "ready");
    assert_eq!(imported["report"]["findings"][0]["node"], "lib/a.ts");
    assert_eq!(imported["report"]["findings"][0]["rule_id"], "EX001");
    assert_eq!(imported["report"]["findings"][0]["level"], "warning");
    assert_eq!(imported["report"]["findings"][0]["line"], 1);
    let saved = fs::read(root.path().join(".archguard/quality.json")).unwrap();
    assert_eq!(
        value(&run(root.path(), "quality", &["--format", "json"]), 0),
        imported
    );
    fs::write(root.path().join("lib/a.ts"), "export const a=2;\n").unwrap();
    let stale = value(&run(root.path(), "quality", &["--format", "json"]), 0);
    assert_eq!(stale["report"]["status"], "stale");
    assert_eq!(
        stale["report"]["findings"][0]["status"],
        "changed_since_import"
    );
    assert_eq!(
        fs::read(root.path().join(".archguard/quality.json")).unwrap(),
        saved
    );
    fs::write(root.path().join("lint.sarif"), "invalid").unwrap();
    assert_eq!(
        run(root.path(), "quality", &["--report", "lint.sarif"])
            .status
            .code(),
        Some(3)
    );
    assert_eq!(
        fs::read(root.path().join(".archguard/quality.json")).unwrap(),
        saved
    );
}
#[test]
fn quality_threshold_options_are_explicit_and_validated_before_import() {
    let root = project();
    sarif(root.path());
    for args in [
        vec!["--base", "HEAD"],
        vec!["--min-changed-coverage", "80"],
        vec!["--max-complexity", "10"],
        vec!["--max-nesting", "3"],
        vec!["--base", "HEAD", "--min-changed-coverage", "NaN"],
        vec!["--base", "HEAD", "--min-changed-coverage", "101"],
        vec!["--base", "HEAD", "--max-complexity", "0"],
        vec!["--format", "sarif"],
        vec!["--source-root", "lib"],
    ] {
        let out = run(root.path(), "quality", &args);
        assert_eq!(
            out.status.code(),
            Some(2),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    assert_eq!(
        run(
            root.path(),
            "quality",
            &[
                "--report",
                "lint.sarif",
                "--base",
                "HEAD",
                "--min-changed-coverage",
                "NaN"
            ]
        )
        .status
        .code(),
        Some(2)
    );
    assert!(!root.path().join(".archguard/quality.json").exists());
}
#[test]
fn quality_changed_line_coverage_gate_has_pass_fail_and_incomplete_exit_codes() {
    let root = project();
    git(root.path(), &["init", "--quiet", "--template="]);
    git(root.path(), &["add", "lib/a.ts", "archguard.toml"]);
    git(
        root.path(),
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "--quiet",
            "-m",
            "base",
        ],
    );
    fs::write(
        root.path().join("lib/a.ts"),
        "export const a=2;\nexport const b=3;\n",
    )
    .unwrap();
    let absent = value(
        &run(
            root.path(),
            "quality",
            &[
                "--base",
                "HEAD",
                "--min-changed-coverage",
                "80",
                "--format",
                "json",
            ],
        ),
        3,
    );
    assert_eq!(absent["gate"]["status"], "incomplete");
    fs::write(root.path().join("coverage.xml"),r#"<coverage><packages><package><classes><class filename="lib/a.ts"><lines><line number="1" hits="1"/><line number="2" hits="0"/></lines></class></classes></package></packages></coverage>"#).unwrap();
    value(
        &run(
            root.path(),
            "coverage",
            &["--report", "coverage.xml", "--format", "json"],
        ),
        0,
    );
    let failed = value(
        &run(
            root.path(),
            "quality",
            &[
                "--base",
                "HEAD",
                "--min-changed-coverage",
                "80",
                "--format",
                "json",
            ],
        ),
        1,
    );
    assert_eq!(failed["gate"]["status"], "fail");
    assert_eq!(failed["gate"]["coverage"]["percent"], 50.0);
    let pass = value(
        &run(
            root.path(),
            "quality",
            &[
                "--base",
                "HEAD",
                "--min-changed-coverage",
                "50",
                "--format",
                "json",
            ],
        ),
        0,
    );
    assert_eq!(pass["gate"]["status"], "pass");
    assert_eq!(pass["gate"]["coverage"]["covered_lines"], 1);
    assert_eq!(pass["gate"]["coverage"]["total_lines"], 2);
    let unsupported = value(
        &run(
            root.path(),
            "quality",
            &[
                "--base",
                "HEAD",
                "--max-complexity",
                "10",
                "--format",
                "json",
            ],
        ),
        3,
    );
    assert_eq!(unsupported["gate"]["status"], "incomplete");
    fs::write(
        root.path().join("lib/a.ts"),
        "export const a=4;\nexport const b=3;\n",
    )
    .unwrap();
    let stale = value(
        &run(
            root.path(),
            "quality",
            &[
                "--base",
                "HEAD",
                "--min-changed-coverage",
                "0",
                "--format",
                "json",
            ],
        ),
        3,
    );
    assert_eq!(stale["gate"]["status"], "incomplete");
}
