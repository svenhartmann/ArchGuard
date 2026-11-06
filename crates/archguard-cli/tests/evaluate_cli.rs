//! `archguard evaluate` (ADR 0004) over a copy of `fixtures/ts-cyclic`
//! with a hand-written expert assessment.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::{TempDir, tempdir};

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

fn fixture() -> (TempDir, PathBuf) {
    let dir = tempdir().unwrap();
    let root = dir.path().join("ts-cyclic");
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/ts-cyclic"),
        &root,
    );
    (dir, root)
}

fn archguard(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_archguard"))
        .args(args)
        .arg(root)
        .output()
        .unwrap()
}

fn json(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

/// Stable id of the current finding with this report id.
fn id_of(unjudged: &Value, report_id: &str) -> String {
    unjudged
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["report_id"] == report_id)
        .unwrap_or_else(|| panic!("no finding {report_id} in {unjudged}"))["id"]
        .as_str()
        .unwrap()
        .to_string()
}

const META: &str =
    "[assessment]\nassessor = \"Test Expert\"\ndate = \"2026-11-06\"\nrevision = \"0123abc\"\n";

#[test]
fn evaluate_reports_recall_precision_unjudged_missed_and_stale() {
    let (_dir, root) = fixture();
    fs::create_dir_all(root.join(".archguard")).unwrap();
    let path = root.join(".archguard/assessment.toml");
    fs::write(&path, META).unwrap();
    let before = json(&archguard(&root, &["evaluate", "--format", "json"]));
    assert!(before["recall"]["value"].is_null());
    assert!(before["precision"]["value"].is_null());
    let findings = before["findings_total"].as_u64().unwrap();
    assert!(findings >= 3, "{before}");
    let cycle = id_of(&before["unjudged"], "scc-1");
    let unranked = id_of(&before["unjudged"], "layers-unranked");

    fs::write(
        &path,
        format!(
            "{META}\
             [[expected]]\nid = \"E1\"\nkind = \"cycle\"\nnodes = [\"a/*\", \"b/*\"]\n\
             [[expected]]\nid = \"E2\"\nkind = \"layer_violation\"\nnodes = [\"app.ts\"]\n\
             [[expected]]\nid = \"E3\"\nkind = \"other\"\nnote = \"god class in app.ts\"\n\
             [[judged]]\nfinding = \"{cycle}\"\nverdict = \"true_positive\"\n\
             [[judged]]\nfinding = \"{unranked}\"\nverdict = \"false_positive\"\n\
             [[judged]]\nfinding = \"deadbeef\"\nverdict = \"true_positive\"\n"
        ),
    )
    .unwrap();
    let e = json(&archguard(&root, &["evaluate", "--format", "json"]));
    assert_eq!(e["recall"]["numerator"], 1);
    assert_eq!(e["recall"]["denominator"], 3);
    assert_eq!(e["precision"]["numerator"], 1);
    assert_eq!(e["precision"]["denominator"], 2);
    assert_eq!(e["precision"]["value"], 0.5);
    assert_eq!(e["matched"][0]["id"], "E1");
    assert_eq!(e["matched"][0]["findings"][0], cycle.as_str());
    assert_eq!(
        e["missed"][0]["reason"],
        "no layer_violation finding reported"
    );
    assert_eq!(e["missed"][1]["reason"], "no ArchGuard finding kind");
    assert_eq!(e["stale"], serde_json::json!(["deadbeef"]));
    assert_eq!(e["unjudged"].as_array().unwrap().len() as u64, findings - 2);
    assert!(
        e["warnings"][0].as_str().unwrap().contains("HEAD unknown"),
        "{e}"
    );

    let text = archguard(&root, &["evaluate"]);
    assert!(text.status.success());
    let text = String::from_utf8(text.stdout).unwrap();
    assert!(text.contains("recall: 1/3 = 0.33"), "{text}");
    assert!(text.contains("precision: 1/2 = 0.50"), "{text}");
    assert!(
        text.contains("E3 (other): no ArchGuard finding kind"),
        "{text}"
    );
    assert!(text.contains("deadbeef"), "{text}");
}

#[test]
fn judged_ids_are_the_stable_review_finding_ids() {
    let (_dir, root) = fixture();
    let template = root.join("assessment.toml");
    let init = archguard(
        &root,
        &[
            "evaluate",
            "--init",
            "--assessment",
            template.to_str().unwrap(),
        ],
    );
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );
    let review = json(&archguard(&root, &["review", "--format", "json"]));
    let text = fs::read_to_string(&template).unwrap();
    let architecture: Vec<&str> = review["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["category"] == "architecture")
        .map(|f| f["id"].as_str().unwrap())
        .collect();
    assert!(!architecture.is_empty());
    for id in architecture {
        assert!(
            text.contains(&format!("finding = \"{id}\"")),
            "{id} missing in\n{text}"
        );
    }
}

#[test]
fn init_writes_unjudged_stubs_and_never_overwrites() {
    let (_dir, root) = fixture();
    let first = archguard(&root, &["evaluate", "--init"]);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let path = root.join(".archguard/assessment.toml");
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("[[judged]]"));
    assert!(text.contains("verdict = \"\""));
    assert!(!text.contains("verdict = \"true_positive\""));
    assert!(!text.contains("verdict = \"false_positive\""));

    let unjudged = json(&archguard(&root, &["evaluate", "--format", "json"]));
    assert!(unjudged["precision"]["value"].is_null());
    assert_eq!(
        unjudged["unjudged"].as_array().unwrap().len() as u64,
        unjudged["findings_total"].as_u64().unwrap()
    );

    fs::write(&path, "# my work\n").unwrap();
    let second = archguard(&root, &["evaluate", "--init"]);
    assert_eq!(second.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&second.stderr).contains("already exists"));
    assert_eq!(fs::read_to_string(&path).unwrap(), "# my work\n");
}

#[test]
fn missing_assessment_and_foreign_format_fail_with_cli_exit_codes() {
    let (_dir, root) = fixture();
    assert_eq!(archguard(&root, &["evaluate"]).status.code(), Some(3));
    fs::create_dir_all(root.join(".archguard")).unwrap();
    fs::write(
        root.join(".archguard/assessment.toml"),
        format!("{META}[[judged]]\nfinding = \"x\"\nverdict = \"probably\"\n"),
    )
    .unwrap();
    let bad = archguard(&root, &["evaluate"]);
    assert_eq!(bad.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&bad.stderr).contains("probably"));
    assert_eq!(
        archguard(&root, &["evaluate", "--format", "sarif"])
            .status
            .code(),
        Some(2)
    );
}

fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(root)
        .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

#[test]
fn template_records_head_and_a_later_commit_warns() {
    let (_dir, root) = fixture();
    git(&root, &["init", "-q"]);
    git(&root, &["add", "src"]);
    git(&root, &["commit", "-q", "-m", "first"]);
    let head = git(&root, &["rev-parse", "HEAD"]);
    let init = archguard(&root, &["evaluate", "--init"]);
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );
    let text = fs::read_to_string(root.join(".archguard/assessment.toml")).unwrap();
    assert!(text.contains(&format!("revision = \"{head}\"")), "{text}");
    let same = json(&archguard(&root, &["evaluate", "--format", "json"]));
    assert_eq!(same["head"], head.as_str());
    assert_eq!(same["warnings"], serde_json::json!([]));

    git(&root, &["commit", "-q", "--allow-empty", "-m", "second"]);
    let later = archguard(&root, &["evaluate"]);
    assert!(later.status.success());
    let stderr = String::from_utf8_lossy(&later.stderr);
    assert!(
        stderr.contains(&format!("assessment revision {head} differs from HEAD")),
        "{stderr}"
    );
}
