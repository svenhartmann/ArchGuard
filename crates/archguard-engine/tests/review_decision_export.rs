//! The decision document a review exports is what `review --decisions` imports.
use archguard_engine::core::DecisionKind;
use archguard_engine::{
    apply_review_decisions, generate_review, load_register, review_decision_document,
    upsert_decision,
};
use std::fs;
const NOW: u64 = 1_793_232_000;
fn project() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("src")).unwrap();
    for (file, source) in [
        ("a.ts", "import './b';\n"),
        ("b.ts", "import './c';\n"),
        ("c.ts", "import './a';\n"),
    ] {
        fs::write(root.path().join("src").join(file), source).unwrap();
    }
    fs::write(root.path().join("archguard.toml"),"derive_layers=false\n[project]\nsource_roots=['src']\nlanguages=['ts']\nexclude=[]\n[modules]\ndepth=0\n[rules]\ndeny_cycles='file'\nmax_scc_size=100\n").unwrap();
    root
}
fn draft(r: &archguard_engine::ReviewReport) -> archguard_engine::core::ReviewDecision {
    let mut d = r.findings[0].draft_decision(DecisionKind::Defer);
    d.author = "Reviewer".into();
    d.note = "After the release".into();
    d.adr_link = Some("docs/adr/0007.md".into());
    d
}
#[test]
fn an_exported_document_round_trips_through_the_import() {
    let root = project();
    let (mut r, _) = generate_review(root.path(), None, None, NOW).unwrap();
    let doc = review_decision_document(&r, &[draft(&r)]);
    let bytes = serde_json::to_vec(&doc).unwrap();
    apply_review_decisions(root.path(), &mut r, &bytes, NOW).unwrap();
    let register = load_register(root.path()).unwrap();
    let d = register.decision_for(&r.findings[0].id).expect("recorded");
    assert_eq!(d.decision, DecisionKind::Defer);
    assert_eq!(d.adr_link.as_deref(), Some("docs/adr/0007.md"));
    assert!(!d.ts.is_empty(), "the import stamps the time");
    assert_eq!(r.findings[0].disposition, "deferred");
    // A second export keeps the recorded decision without a draft.
    let (fresh, _) = generate_review(root.path(), None, None, NOW).unwrap();
    let again = review_decision_document(&fresh, &[]);
    assert_eq!(again.decisions.len(), 1);
    assert_eq!(
        again.decision_revision.as_deref(),
        Some(fresh.decision_fingerprint.as_str())
    );
}
/// `[review] adr_required_on_accept_risk` holds for the import (desktop,
/// `review --decisions`) as for `review-decision record` and MCP.
#[test]
fn the_import_keeps_the_adr_gate_on_accepted_risks() {
    let root = project();
    let toml = root.path().join("archguard.toml");
    let gated = fs::read_to_string(&toml).unwrap() + "[review]\nadr_required_on_accept_risk=true\n";
    fs::write(&toml, gated).unwrap();
    let (mut r, _) = generate_review(root.path(), None, None, NOW).unwrap();
    let mut risk = draft(&r);
    risk.decision = DecisionKind::AcceptRisk;
    risk.expires_on = Some("2099-12-31".into());
    risk.adr_link = None;
    let bytes = serde_json::to_vec(&review_decision_document(&r, &[risk.clone()])).unwrap();
    let err = apply_review_decisions(root.path(), &mut r, &bytes, NOW).unwrap_err();
    assert!(
        err.to_string().contains("adr_required_on_accept_risk"),
        "{err}"
    );
    assert!(
        !root.path().join(".archguard/reviews.json").exists(),
        "nothing recorded"
    );
    assert!(
        upsert_decision(root.path(), risk.clone()).is_err(),
        "the register refuses too"
    );
    risk.adr_link = Some("docs/adr/0007.md".into());
    let bytes = serde_json::to_vec(&review_decision_document(&r, &[risk])).unwrap();
    apply_review_decisions(root.path(), &mut r, &bytes, NOW).unwrap();
}

#[test]
fn a_document_exported_before_a_newer_decision_is_refused() {
    let root = project();
    let (r, _) = generate_review(root.path(), None, None, NOW).unwrap();
    let stale = serde_json::to_vec(&review_decision_document(&r, &[draft(&r)])).unwrap();
    let mut other = draft(&r);
    other.ts = "2099-01-01T00:00:00Z".into();
    upsert_decision(root.path(), other).unwrap();
    let before = fs::read(root.path().join(".archguard/reviews.json")).unwrap();
    let (mut fresh, _) = generate_review(root.path(), None, None, NOW).unwrap();
    let err = apply_review_decisions(root.path(), &mut fresh, &stale, NOW).unwrap_err();
    assert!(err.to_string().contains("older decision revision"), "{err}");
    assert_eq!(
        fs::read(root.path().join(".archguard/reviews.json")).unwrap(),
        before
    );
}
