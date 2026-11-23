//! Painless call analyzer (ADR 0023): one method per `.painless` file and per literal inline script.
use archguard_core::Config;
use archguard_engine::{analyze_calls, semantic_fingerprints};
use std::path::{Path, PathBuf};

fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/painless-scripts")
}

fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let target = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy(&e.path(), &target);
        } else {
            std::fs::copy(e.path(), target).unwrap();
        }
    }
}

#[test]
fn one_method_per_file_and_per_literal_inline_script() {
    let calls = analyze_calls(&fixture(), &Config::default());
    let analyzer = calls
        .analyzers
        .iter()
        .find(|a| a.language == "painless")
        .unwrap();
    assert_eq!(
        analyzer.status, "partial",
        "the non-literal script is a gap"
    );
    let mut classes: Vec<_> = calls
        .classes
        .iter()
        .map(|c| (c.file.clone(), c.graph_node.clone()))
        .collect();
    classes.sort();
    assert_eq!(classes.len(), 4, "{classes:?}");
    assert!(
        classes
            .iter()
            .all(|(f, n)| f == n && !f.contains(".terraform"))
    );
    assert!(
        calls
            .classes
            .iter()
            .all(|c| c.language.as_deref() == Some("painless"))
    );
    let file = calls
        .methods
        .iter()
        .find(|m| m.file == "scripts/score.painless")
        .unwrap();
    let c = file.complexity.as_ref().unwrap();
    assert_eq!(
        (c.cyclomatic, c.max_nesting, c.definition.as_str()),
        (4, 2, "ast-decisions-v1")
    );
    assert_eq!(file.line, 1);
    let inline = calls.methods.iter().find(|m| m.file == "main.tf").unwrap();
    let c = inline.complexity.as_ref().unwrap();
    assert_eq!((c.cyclomatic, c.max_nesting), (2, 1));
    assert_eq!(inline.line, 7, "line of the source attribute");
    assert_eq!(
        calls.methods.len(),
        2,
        "file() adds none, var.code is a gap"
    );
    assert!(
        calls
            .diagnostics
            .iter()
            .any(|d| d.contains("1 script source") && d.contains("main.tf")),
        "{:?}",
        calls.diagnostics
    );
}

#[test]
fn without_painless_the_report_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.tf"), "resource \"x\" \"y\" {}\n").unwrap();
    let calls = analyze_calls(dir.path(), &Config::default());
    assert!(calls.analyzers.iter().all(|a| a.language != "painless"));
    assert_eq!(calls.status, "not_applicable");
}

#[test]
fn a_changed_script_changes_the_fingerprint() {
    let dir = tempfile::tempdir().unwrap();
    copy(&fixture(), dir.path());
    let config = Config::default();
    let before = semantic_fingerprints(dir.path(), &config);
    assert_eq!(before, semantic_fingerprints(dir.path(), &config));
    std::fs::write(dir.path().join("scripts/score.painless"), "return 1;\n").unwrap();
    assert_ne!(
        before,
        semantic_fingerprints(dir.path(), &config),
        "painless"
    );
    let after = semantic_fingerprints(dir.path(), &config);
    std::fs::write(dir.path().join("main.tf"), "# changed\n").unwrap();
    assert_ne!(after, semantic_fingerprints(dir.path(), &config), "tf");
}
