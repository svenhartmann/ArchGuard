use archguard_engine::core::{FindingKind, Severity};

use super::super::preview::run;
use super::super::{ConfigEditor, FieldValue};
use super::fixture;

#[test]
fn a_threshold_change_shows_in_the_preview() {
    let dir = fixture("ts-cyclic");
    let mut editor = ConfigEditor::load(dir.path()).unwrap();
    editor.set("rules.max_acd", FieldValue::Float(1.0)).unwrap();
    let preview = run(&editor.preview_request()).unwrap();
    let before = preview.before.as_ref().unwrap();
    assert!(preview.changed());
    assert_eq!(preview.after.errors, before.errors + 1, "{preview:?}");
    assert_eq!(preview.added.len(), 1);
    assert_eq!(preview.added[0].kind, FindingKind::MetricThreshold);
    assert!(preview.removed.is_empty());
}

#[test]
fn allowing_cycles_turns_the_check_green() {
    let dir = fixture("ts-cyclic");
    let mut editor = ConfigEditor::load(dir.path()).unwrap();
    editor
        .set("rules.deny_cycles", FieldValue::Text("off".into()))
        .unwrap();
    editor
        .set("rules.max_scc_size", FieldValue::Int(100))
        .unwrap();
    let preview = run(&editor.preview_request()).unwrap();
    let before = preview.before.as_ref().unwrap();
    assert!(!before.pass, "ts-cyclic fails with the defaults");
    assert!(preview.after.pass, "{preview:?}");
    let cycle_errors = |s: &super::super::CheckSummary| {
        s.findings
            .iter()
            .filter(|f| f.kind == FindingKind::Cycle && f.severity == Severity::Error)
            .count()
    };
    assert!(cycle_errors(before) > 0);
    assert_eq!(cycle_errors(&preview.after), 0, "{preview:?}");
}

#[test]
fn an_invalid_edit_is_an_error_not_a_preview() {
    let dir = fixture("ts-cyclic");
    let mut editor = ConfigEditor::load(dir.path()).unwrap();
    editor
        .set("performance.min_barrel_ratio", FieldValue::Float(2.0))
        .unwrap();
    let err = run(&editor.preview_request()).unwrap_err();
    assert_eq!(err.path, "performance.min_barrel_ratio");
}
