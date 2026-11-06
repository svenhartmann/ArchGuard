//! One decision register: a former `review-decisions.json` is read into
//! `.archguard/reviews.json` and retired on the next write.
use archguard_engine::core::{DecisionKind, ReviewDecision};
use archguard_engine::{load_register, upsert_decision};
use std::fs;

const LEGACY: &str = include_str!("data/legacy-review-decisions.json");

fn project_with_legacy_file() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join(".archguard")).unwrap();
    fs::write(root.path().join(".archguard/review-decisions.json"), LEGACY).unwrap();
    root
}

#[test]
fn legacy_decisions_are_read_into_the_register_without_writing() {
    let root = project_with_legacy_file();
    let register = load_register(root.path()).unwrap();
    let kinds: Vec<_> = register
        .decisions
        .iter()
        .map(|d| (d.finding_ref.chars().next().unwrap(), d.decision))
        .collect();
    assert_eq!(
        kinds,
        [
            ('1', DecisionKind::AcceptRisk),
            ('2', DecisionKind::Fix),
            ('3', DecisionKind::Defer)
        ]
    );
    let accepted = &register.decisions[0];
    assert_eq!(accepted.id, accepted.finding_ref);
    assert_eq!(accepted.author, "@team/orders");
    assert_eq!(accepted.note, "Temporary migration boundary");
    assert_eq!(accepted.expires_on.as_deref(), Some("2099-12-31"));
    assert_eq!(accepted.links, ["docs/adr/001.md"]);
    assert_eq!(
        accepted.finding_fingerprint.as_deref(),
        Some(&*"a".repeat(64))
    );
    assert!(accepted.ts.is_empty(), "the old file recorded no timestamp");
    // Reading never migrates: read-only callers (MCP, PR comment) write nothing.
    assert!(!root.path().join(".archguard/reviews.json").exists());
    assert!(
        root.path()
            .join(".archguard/review-decisions.json")
            .exists()
    );
}

#[test]
fn the_next_write_stores_legacy_decisions_once_and_retires_the_old_file() {
    let root = project_with_legacy_file();
    let cli = ReviewDecision {
        id: "scc-1".into(),
        finding_ref: "scc-1".into(),
        decision: DecisionKind::Defer,
        note: "later".into(),
        author: "cli".into(),
        ts: "2026-11-06T00:00:00Z".into(),
        ..Default::default()
    };
    upsert_decision(root.path(), cli).unwrap();
    assert!(
        !root
            .path()
            .join(".archguard/review-decisions.json")
            .exists()
    );
    let stored: serde_json::Value =
        serde_json::from_slice(&fs::read(root.path().join(".archguard/reviews.json")).unwrap())
            .unwrap();
    assert_eq!(stored["decisions"].as_array().unwrap().len(), 4);
    assert_eq!(stored["decisions"][0]["decision"], "accept-risk");
    assert_eq!(stored["decisions"][3]["finding_ref"], "scc-1");
    // A decision without review binding keeps the register's old shape.
    assert!(stored["decisions"][3].get("finding_fingerprint").is_none());
    assert_eq!(load_register(root.path()).unwrap().decisions.len(), 4);
}

#[test]
fn register_entries_win_over_legacy_entries_with_the_same_id() {
    let root = project_with_legacy_file();
    let id = "2".repeat(64);
    let replacement = ReviewDecision {
        id: id.clone(),
        finding_ref: id.clone(),
        decision: DecisionKind::AcceptRisk,
        note: "decided again".into(),
        author: "cli".into(),
        ..Default::default()
    };
    upsert_decision(root.path(), replacement).unwrap();
    let register = load_register(root.path()).unwrap();
    assert_eq!(register.decisions.len(), 3);
    let d = register.decisions.iter().find(|d| d.id == id).unwrap();
    assert_eq!(d.note, "decided again");
}

#[test]
fn a_corrupt_legacy_file_is_an_error_and_stays_untouched() {
    let root = project_with_legacy_file();
    fs::write(
        root.path().join(".archguard/review-decisions.json"),
        "{ not json",
    )
    .unwrap();
    assert!(load_register(root.path()).is_err());
    let decision = ReviewDecision {
        id: "x".into(),
        finding_ref: "x".into(),
        ..Default::default()
    };
    assert!(upsert_decision(root.path(), decision).is_err());
    assert_eq!(
        fs::read_to_string(root.path().join(".archguard/review-decisions.json")).unwrap(),
        "{ not json"
    );
    assert!(!root.path().join(".archguard/reviews.json").exists());
}
