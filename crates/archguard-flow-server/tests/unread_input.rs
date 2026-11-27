//! An input that cannot be read is a gap, never a silent skip: the flow is incomplete.
//! Covers `ui_flow_go` (`go.mod`, discovery settings) and `ui_flow_go::oathkeeper`.

use archguard_core::{DiagnosticSeverity, UiFlow};
use archguard_flow_server::ui_flow_go;

/// `FF FE` announces UTF-16, the single byte after it is half a unit: `InvalidData`.
const BROKEN_UTF16: &[u8] = &[0xFF, 0xFE, 0x41];

const GO: &[u8] = b"package main\nfunc main() {}\n";

fn extract(files: &[(&str, &[u8])]) -> UiFlow {
    let dir = tempfile::tempdir().unwrap();
    for (rel, bytes) in files {
        std::fs::write(dir.path().join(rel), bytes).unwrap();
    }
    ui_flow_go::extract(dir.path())
}

fn names(flow: &UiFlow, file: &str) -> bool {
    flow.diagnostics.iter().any(|d| d.file == file)
}

#[test]
fn unreadable_go_mod_is_a_gap() {
    let flow = extract(&[("go.mod", BROKEN_UTF16), ("main.go", GO)]);
    assert!(
        !flow.complete,
        "an unreadable go.mod must not leave the flow complete"
    );
    assert!(
        names(&flow, "go.mod"),
        "no gap names go.mod: {:?}",
        flow.diagnostics
    );
}

#[test]
fn unreadable_discovery_settings_are_a_gap() {
    let flow = extract(&[
        ("archguard.toml", b"[project]\nexclude = 1\n"),
        ("main.go", GO),
    ]);
    assert!(
        !flow.complete,
        "unreadable discovery settings must not leave the flow complete"
    );
    assert!(
        names(&flow, "."),
        "no gap names `.`: {:?}",
        flow.diagnostics
    );
}

#[test]
fn unreadable_oathkeeper_candidate_is_reported() {
    let flow = extract(&[
        ("go.mod", b"module x\n"),
        ("main.go", GO),
        ("rules.yml", BROKEN_UTF16),
    ]);
    assert!(
        names(&flow, "rules.yml"),
        "no diagnostic names rules.yml: {:?}",
        flow.diagnostics
    );
    // Deliberately a note, not a gap: every YAML/JSON file is a candidate here,
    // so a gap would make any Go flow incomplete over an unrelated lock file
    // above 1 MiB. These two assertions pin that decision.
    let about = flow
        .diagnostics
        .iter()
        .find(|d| d.file == "rules.yml")
        .expect("checked above");
    assert_eq!(
        about.severity,
        DiagnosticSeverity::Note,
        "an unreadable candidate stays a note: {about:?}"
    );
    assert!(
        flow.complete,
        "a candidate nobody asked for must not make the flow incomplete: {:?}",
        flow.diagnostics
    );
}

#[test]
fn laravel_unreadable_discovery_settings_are_a_gap() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("archguard.toml"),
        b"[project]\nexclude = 1\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("artisan"), b"#!/usr/bin/env php\n").unwrap();
    let flow = archguard_flow_server::ui_flow_laravel::extract(dir.path());
    assert!(
        !flow.complete,
        "unreadable discovery settings must not leave the flow complete"
    );
    assert!(
        names(&flow, "."),
        "no gap names `.`: {:?}",
        flow.diagnostics
    );
}
