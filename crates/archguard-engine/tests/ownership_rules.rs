use archguard_engine::core::{FindingKind, Status};
use archguard_engine::{Level, analyze_run_quiet, load_config};
use std::fs;

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("src/ui")).unwrap();
    fs::create_dir_all(dir.path().join("src/data")).unwrap();
    fs::write(dir.path().join("src/ui/View.ts"), "import '../data/Db';\n").unwrap();
    fs::write(dir.path().join("src/data/Db.ts"), "export const db=1;\n").unwrap();
    fs::write(
        dir.path().join("CODEOWNERS"),
        "/src/ui/ @org/ui @org/shared\n/src/data/ @org/data @org/shared\n",
    )
    .unwrap();
    dir
}

fn config(root: &std::path::Path, severity: &str) {
    fs::write(
        root.join("archguard.toml"),
        format!(
            r#"
derive_layers=false
[project]
source_roots=["src"]
languages=["ts"]
exclude=[]
[modules]
depth=0
[rules]
deny_cycles="off"
max_scc_size=100
[[ownership_rules]]
from="@org/ui"
to="@org/data"
severity="{severity}"
message="Use the published data API"
"#
        ),
    )
    .unwrap();
}

#[test]
fn ownership_gate_uses_full_sources_even_with_shared_owner_and_cached_depth_zero_view() {
    let dir = project();
    config(dir.path(), "error");
    let loaded = load_config(None, dir.path()).unwrap();
    let first = analyze_run_quiet(dir.path(), &loaded, Level::Module).unwrap();
    let cached = analyze_run_quiet(dir.path(), &loaded, Level::Module).unwrap();
    assert!(cached.cache_hit);
    for run in [first, cached] {
        assert_eq!(
            run.report.summary.status,
            Status::Fail,
            "an ownership violation must fail independently of display depth"
        );
        let finding = run
            .report
            .findings
            .iter()
            .find(|finding| finding.kind == FindingKind::OwnershipRule)
            .unwrap();
        assert_eq!(finding.nodes, vec!["ui/View.ts", "data/Db.ts"]);
        assert_eq!(
            finding.message.as_deref(),
            Some("Use the published data API")
        );
        assert_eq!(finding.evidence[0].line, 1);
    }
}

#[test]
fn ownership_changes_are_rechecked_without_invalidating_import_cache() {
    let dir = project();
    config(dir.path(), "error");
    let loaded = load_config(None, dir.path()).unwrap();
    assert_eq!(
        analyze_run_quiet(dir.path(), &loaded, Level::Module)
            .unwrap()
            .report
            .summary
            .status,
        Status::Fail
    );
    fs::write(dir.path().join("CODEOWNERS"), "* @org/ui\n").unwrap();
    let changed = analyze_run_quiet(dir.path(), &loaded, Level::Module).unwrap();
    assert!(changed.cache_hit);
    assert_eq!(changed.report.summary.status, Status::Pass);
    assert!(
        !changed
            .report
            .findings
            .iter()
            .any(|finding| finding.kind == FindingKind::OwnershipRule)
    );
}

#[test]
fn explicit_ownership_policy_rejects_missing_or_unsupported_coverage() {
    let dir = project();
    config(dir.path(), "error");
    let loaded = load_config(None, dir.path()).unwrap();
    for contents in [None, Some("* @org/ui\n!src/data/ @org/data\n")] {
        if let Some(contents) = contents {
            fs::write(dir.path().join("CODEOWNERS"), contents).unwrap();
        } else {
            fs::remove_file(dir.path().join("CODEOWNERS")).unwrap();
        }
        let error = analyze_run_quiet(dir.path(), &loaded, Level::Module)
            .expect_err("configured ownership policy cannot silently pass without coverage");
        assert!(error.to_string().contains("CODEOWNERS"));
    }
}

#[test]
fn warning_ownership_rule_is_reported_without_failing_the_gate() {
    let dir = project();
    config(dir.path(), "warning");
    let run = analyze_run_quiet(
        dir.path(),
        &load_config(None, dir.path()).unwrap(),
        Level::Module,
    )
    .unwrap();
    assert_eq!(run.report.summary.status, Status::Pass);
    assert!(
        run.report
            .findings
            .iter()
            .any(|finding| finding.kind == FindingKind::OwnershipRule)
    );
}
