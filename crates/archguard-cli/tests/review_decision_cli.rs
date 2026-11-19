//! `archguard review-decision` (R29): record → list → show against the
//! register at `.archguard/reviews.json`.

use assert_cmd::Command;
use tempfile::tempdir;

fn archguard(dir: &tempfile::TempDir, args: &[&str]) -> std::process::Output {
    Command::cargo_bin("archguard")
        .expect("binary")
        .args(args)
        .current_dir(dir.path())
        .output()
        .expect("runs")
}

#[test]
fn record_then_list_and_show() {
    let dir = tempdir().expect("tempdir");
    let recorded = archguard(
        &dir,
        &[
            "review-decision",
            "record",
            "--finding",
            "scc-3",
            "--decision",
            "accept-risk",
            "--adr",
            "docs/adr/004.md",
            "--note",
            "bewusst",
        ],
    );
    assert!(recorded.status.success());
    assert!(
        dir.path().join(".archguard/reviews.json").is_file(),
        "register written"
    );

    let list = archguard(&dir, &["review-decision", "list"]);
    assert!(list.status.success());
    let stdout = String::from_utf8_lossy(&list.stdout);
    assert!(stdout.contains("scc-3"), "{stdout}");
    assert!(stdout.contains("AcceptRisk"), "{stdout}");
    assert!(stdout.contains("docs/adr/004.md"), "{stdout}");
    assert!(stdout.contains("bewusst"), "{stdout}");

    let show = archguard(&dir, &["review-decision", "show", "scc-3"]);
    assert!(show.status.success());
    let stdout = String::from_utf8_lossy(&show.stdout);
    assert!(stdout.contains("note: bewusst"), "{stdout}");
    // ts is ISO-8601 UTC as documented on the register field.
    assert!(regex_lite_ish(&stdout), "no ISO timestamp in:\n{stdout}");
}

/// `ts: YYYY-MM-DDTHH:MM:SSZ`
fn regex_lite_ish(text: &str) -> bool {
    let Some(rest) = text.split("ts: ").nth(1) else {
        return false;
    };
    let ts: String = rest.chars().take_while(|c| *c != '\n').collect();
    let bytes = ts.as_bytes();
    bytes.len() == 20
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes[10] == b'T'
        && bytes[13] == b':'
        && bytes[16] == b':'
        && bytes[19] == b'Z'
}

#[test]
fn accept_risk_without_adr_warns_but_records() {
    let dir = tempdir().expect("tempdir");
    let recorded = archguard(
        &dir,
        &[
            "review-decision",
            "record",
            "--finding",
            "scc-9",
            "--decision",
            "accept-risk",
        ],
    );
    assert!(recorded.status.success(), "warning, not failure");
    let stderr = String::from_utf8_lossy(&recorded.stderr);
    assert!(stderr.contains("--adr"), "{stderr}");
    let list = archguard(&dir, &["review-decision", "list"]);
    assert!(
        String::from_utf8_lossy(&list.stdout).contains("scc-9"),
        "still recorded"
    );
}

#[test]
fn show_unknown_id_is_a_usage_error() {
    let dir = tempdir().expect("tempdir");
    let missing = archguard(&dir, &["review-decision", "show", "nope"]);
    assert_eq!(missing.status.code(), Some(2));
}

#[test]
fn adr_gate_rejects_record_without_adr() {
    let dir = tempdir().expect("tempdir");
    std::fs::write(
        dir.path().join("archguard.toml"),
        "[review]\nadr_required_on_accept_risk = true\n",
    )
    .expect("config");
    let blocked = archguard(
        &dir,
        &[
            "review-decision",
            "record",
            "--finding",
            "scc-5",
            "--decision",
            "accept-risk",
        ],
    );
    assert_eq!(blocked.status.code(), Some(2), "gate on → usage error");
    let stderr = String::from_utf8_lossy(&blocked.stderr);
    assert!(stderr.contains("adr_required_on_accept_risk"), "{stderr}");
    assert!(
        !dir.path().join(".archguard/reviews.json").exists(),
        "nothing recorded while the gate is breached"
    );

    let allowed = archguard(
        &dir,
        &[
            "review-decision",
            "record",
            "--finding",
            "scc-5",
            "--decision",
            "accept-risk",
            "--adr",
            "docs/adr/001.md",
        ],
    );
    assert!(allowed.status.success(), "with --adr the gate passes");
}

#[test]
fn record_upserts_by_id() {
    let dir = tempdir().expect("tempdir");
    for note in ["first", "second"] {
        let recorded = archguard(
            &dir,
            &[
                "review-decision",
                "record",
                "--finding",
                "scc-1",
                "--decision",
                "defer",
                "--note",
                note,
            ],
        );
        assert!(recorded.status.success());
    }
    let list = archguard(&dir, &["review-decision", "list", "--format", "json"]);
    let value: serde_json::Value = serde_json::from_slice(&list.stdout).expect("json");
    let decisions = value["decisions"].as_array().expect("array");
    assert_eq!(decisions.len(), 1, "upsert by id, no duplicates");
    assert_eq!(decisions[0]["note"], "second");
}

/// `review-decision export` writes the document `review --decisions`
/// imports; an export older than the register is refused.
#[test]
fn export_round_trips_through_review_decisions() {
    let dir = tempdir().expect("tempdir");
    std::fs::create_dir(dir.path().join("src")).expect("src");
    for (file, source) in [("a.ts", "import './b';\n"), ("b.ts", "import './a';\n")] {
        std::fs::write(dir.path().join("src").join(file), source).expect("source");
    }
    std::fs::write(dir.path().join("archguard.toml"), "derive_layers=false\n[project]\nsource_roots=['src']\nlanguages=['ts']\nexclude=[]\n[rules]\ndeny_cycles='file'\nmax_scc_size=100\n").expect("config");
    let export = archguard(&dir, &["review-decision", "export", "--out", "d.json"]);
    let stderr = String::from_utf8_lossy(&export.stderr);
    assert!(export.status.success(), "{stderr}");
    let doc: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.path().join("d.json")).expect("written"))
            .expect("json");
    assert_eq!(doc["schema_version"], 2);
    assert_eq!(doc["review_revision"].as_str().map(str::len), Some(64));
    let import = archguard(&dir, &["review", "--decisions", "d.json"]);
    let stderr = String::from_utf8_lossy(&import.stderr);
    assert!(import.status.success(), "{stderr}");
    let recorded = archguard(
        &dir,
        &[
            "review-decision",
            "record",
            "--finding",
            "x",
            "--decision",
            "fix",
        ],
    );
    assert!(recorded.status.success());
    let stale = archguard(&dir, &["review", "--decisions", "d.json"]);
    assert_eq!(stale.status.code(), Some(3), "stale export refused");
}
