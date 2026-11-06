use archguard_engine::{apply_review_decisions, generate_review, read_saved_review, save_review};
use serde_json::{Value, json};
use std::{fs, path::Path};
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
fn document(r: &archguard_engine::ReviewReport) -> Vec<u8> {
    let f = r
        .findings
        .iter()
        .find(|f| f.evidence["kind"] == "cycle")
        .unwrap();
    serde_json::to_vec(&json!({"schema_version":2,"review_revision":r.revision,"decision_revision":r.decision_fingerprint,"decisions":[{
        "id":f.id,"finding_ref":f.id,"finding_fingerprint":f.fingerprint,"decision":"accept-risk","author":"@core","note":"Migrate in stages","expires_on":"2099-12-31","links":[]
    }]})).unwrap()
}
#[test]
fn snapshot_is_immutable_and_expiry_and_denser_cycles_reopen_review() {
    let root = project();
    let (mut report, run) = generate_review(root.path(), None, None, NOW).unwrap();
    let decisions = document(&report);
    // The public byte importer has the same validation as the CLI file importer.
    apply_review_decisions(root.path(), &mut report, &decisions, NOW).unwrap();
    save_review(root.path(), &report).unwrap();
    let bytes = fs::read(root.path().join(".archguard/review.json")).unwrap();
    let loaded = read_saved_review(root.path(), &run.effective_config(), NOW);
    assert_eq!(loaded.status, "current");
    assert_eq!(
        read_saved_review(root.path(), &run.effective_config(), 4_102_444_800).status,
        "stale"
    );
    let (expired, _) = generate_review(root.path(), None, None, 4_102_444_800).unwrap();
    assert!(expired.findings.iter().any(|f| f.disposition == "expired"));
    fs::write(
        root.path().join("src/a.ts"),
        "import './b';\nimport './c';\n",
    )
    .unwrap();
    let (changed, new_run) = generate_review(root.path(), None, None, NOW).unwrap();
    let old = report
        .findings
        .iter()
        .find(|f| f.evidence["kind"] == "cycle")
        .unwrap();
    let current = changed.findings.iter().find(|f| f.id == old.id).unwrap();
    assert_ne!(current.fingerprint, old.fingerprint);
    assert_eq!(current.disposition, "changed");
    assert_eq!(
        read_saved_review(root.path(), &new_run.effective_config(), NOW).status,
        "stale"
    );
    assert_eq!(
        fs::read(root.path().join(".archguard/review.json")).unwrap(),
        bytes
    );
}
#[test]
fn decision_validation_is_atomic_and_rejects_invalid_dates_duplicates_and_foreign_findings() {
    let root = project();
    let (mut r, _) = generate_review(root.path(), None, None, NOW).unwrap();
    let good: Value = serde_json::from_slice(&document(&r)).unwrap();
    for patch in [
        json!({"expires_on":"2027-02-29"}),
        json!({"expires_on":"2000-01-01"}),
        json!({"expires_on":null}),
        json!({"author":""}),
        json!({"note":""}),
        json!({"finding_fingerprint":"f".repeat(64)}),
        json!({"finding_fingerprint":null}),
        json!({"id":"f".repeat(64),"finding_ref":"f".repeat(64)}),
        json!({"id":"other"}),
    ] {
        let mut doc = good.clone();
        for (key, value) in patch.as_object().unwrap() {
            doc["decisions"][0][key] = value.clone();
        }
        assert!(
            apply_review_decisions(root.path(), &mut r, &serde_json::to_vec(&doc).unwrap(), NOW)
                .is_err(),
            "{patch}"
        );
        assert!(!root.path().join(".archguard/reviews.json").exists());
    }
    let mut duplicate = good.clone();
    duplicate["decisions"]
        .as_array_mut()
        .unwrap()
        .push(good["decisions"][0].clone());
    assert!(
        apply_review_decisions(
            root.path(),
            &mut r,
            &serde_json::to_vec(&duplicate).unwrap(),
            NOW
        )
        .is_err()
    );
}
#[test]
fn stale_export_cannot_overwrite_a_newer_decision() {
    let root = project();
    let (mut r, _) = generate_review(root.path(), None, None, NOW).unwrap();
    let initial = document(&r);
    apply_review_decisions(root.path(), &mut r, &initial, NOW).unwrap();
    let before = fs::read(root.path().join(".archguard/reviews.json")).unwrap();
    let (mut fresh, _) = generate_review(root.path(), None, None, NOW).unwrap();
    assert!(
        apply_review_decisions(root.path(), &mut fresh, &initial, NOW).is_err(),
        "an older export must not overwrite newer decision state"
    );
    assert_eq!(
        fs::read(root.path().join(".archguard/reviews.json")).unwrap(),
        before
    );
}
#[test]
fn source_changes_between_generation_and_recording_are_rejected() {
    let root = project();
    let (mut r, _) = generate_review(root.path(), None, None, NOW).unwrap();
    let decisions = document(&r);
    fs::write(
        root.path().join("src/a.ts"),
        "import './b';\nconst changed=1;\n",
    )
    .unwrap();
    assert!(apply_review_decisions(root.path(), &mut r, &decisions, NOW).is_err());
    assert!(save_review(root.path(), &r).is_err());
    assert!(!root.path().join(".archguard/reviews.json").exists());
    assert!(!root.path().join(".archguard/review.json").exists());
}
#[test]
fn report_imports_test_mappings_and_retains_the_unverified_producer_gap() {
    let root = project();
    let (_, run) = generate_review(root.path(), None, None, NOW).unwrap();
    fs::write(
        root.path().join("junit.xml"),
        r#"<testsuite name="unit"><testcase name="checks cycle" time="0.1"/></testsuite>"#,
    )
    .unwrap();
    fs::write(root.path().join("mapping.json"),r#"{"schema_version":1,"tests":[{"suite":"unit","name":"checks cycle","sources":[{"path":"src/a.ts","lines":[1]}]}]}"#).unwrap();
    archguard_engine::import_test_evidence(
        root.path(),
        &run.config,
        &run.graph,
        Path::new("junit.xml"),
        Some(Path::new("mapping.json")),
        None,
    )
    .unwrap();
    let (report, _) = generate_review(root.path(), None, None, NOW).unwrap();
    assert_eq!(
        report.impact["tests"]["selected"][0]["name"],
        "checks cycle"
    );
    assert_eq!(report.impact["safe_to_skip_other_tests"], false);
    assert!(
        report
            .gaps
            .iter()
            .any(|g| g["source"] == "tests" && g["status"] == "unverified")
    );
}
#[cfg(unix)]
#[test]
fn review_storage_rejects_symlinked_directories() {
    let root = project();
    let (report, _) = generate_review(root.path(), None, None, NOW).unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join(".archguard")).unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join(".archguard/reviews")).unwrap();
    assert!(save_review(root.path(), &report).is_err());
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
}

#[test]
fn repository_level_ownership_edits_invalidate_a_subproject_review() {
    let repo = tempfile::tempdir().unwrap();
    let git = std::process::Command::new("git")
        .current_dir(repo.path())
        .args(["init", "--quiet", "--template="])
        .output()
        .unwrap();
    assert!(git.status.success());
    let root = repo.path().join("project");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("a.py"), "a=1\n").unwrap();
    fs::write(repo.path().join("CODEOWNERS"), "* @before\n").unwrap();
    let (report, run) = generate_review(&root, None, None, NOW).unwrap();
    save_review(&root, &report).unwrap();
    assert_eq!(
        read_saved_review(&root, &run.effective_config(), NOW).status,
        "current"
    );
    fs::write(repo.path().join("CODEOWNERS"), "* @after\n").unwrap();
    assert_eq!(
        read_saved_review(&root, &run.effective_config(), NOW).status,
        "stale"
    );
}

#[test]
fn failed_tests_and_undetected_mutations_become_actionable_with_original_evidence() {
    let root = project();
    let (_, run) = generate_review(root.path(), None, None, NOW).unwrap();
    fs::write(root.path().join("junit.xml"),"<testsuite name='orders'><testcase name='rejects invalid order'><failure message='expected rejection'/></testcase><testcase name='setup'><error message='database unavailable'/></testcase><testcase name='ignored'><skipped/></testcase><testcase name='works'/></testsuite>").unwrap();
    archguard_engine::import_test_evidence(
        root.path(),
        &run.config,
        &run.graph,
        Path::new("junit.xml"),
        None,
        None,
    )
    .unwrap();
    let source = fs::read_to_string(root.path().join("src/a.ts")).unwrap();
    let mutants: Vec<_>=["Survived","NoCoverage","Killed","Timeout","Ignored"].into_iter().enumerate().map(|(i,status)|json!({"id":format!("m{i}"),"mutatorName":"StringLiteral","status":status,"location":{"start":{"line":1,"column":1},"end":{"line":1,"column":2}}})).collect();
    fs::write(root.path().join("mutations.json"),serde_json::to_vec(&json!({"schemaVersion":"2","thresholds":{"high":80,"low":60},"files":{"src/a.ts":{"language":"typescript","source":source,"mutants":mutants}}})).unwrap()).unwrap();
    archguard_engine::import_mutations(
        root.path(),
        &run.config,
        &run.graph,
        Path::new("mutations.json"),
        None,
    )
    .unwrap();
    let (mut report, _) = generate_review(root.path(), None, None, NOW).unwrap();
    let tests: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.category == "tests")
        .collect();
    assert_eq!(
        tests.len(),
        2,
        "only reported failed/error tests become findings"
    );
    assert!(
        tests
            .iter()
            .all(|f| f.nodes.is_empty() && f.source_status == "partial")
    );
    assert!(
        tests
            .iter()
            .any(|f| f.evidence["test"]["message"] == "expected rejection")
    );
    let mutations: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.category == "mutations")
        .collect();
    assert_eq!(
        mutations.len(),
        2,
        "only Survived and NoCoverage are undetected outcomes"
    );
    assert!(
        mutations
            .iter()
            .all(|f| f.nodes == ["a.ts"] && f.change == "not_compared")
    );
    let f = mutations[0];
    let id = f.id.clone();
    let fingerprint = f.fingerprint.clone();
    let document=serde_json::to_vec(&json!({"schema_version":2,"review_revision":report.revision,"decision_revision":report.decision_fingerprint,"decisions":[{"id":f.id,"finding_ref":f.id,"finding_fingerprint":f.fingerprint,"decision":"accept-risk","author":"@tests","note":"Review equivalent mutation","expires_on":"2099-12-31","links":[]}]})).unwrap();
    apply_review_decisions(root.path(), &mut report, &document, NOW).unwrap();
    fs::write(root.path().join("src/a.ts"), source + "// changed\n").unwrap();
    let (changed, _) = generate_review(root.path(), None, None, NOW).unwrap();
    let f = changed.findings.iter().find(|f| f.id == id).unwrap();
    assert_ne!(f.fingerprint, fingerprint);
    assert_eq!(f.disposition, "changed");
    assert_eq!(f.source_status, "changed_since_report");
    assert!(
        changed
            .gaps
            .iter()
            .any(|g| g["source"] == "mutations" && g["status"] == "unverified")
    );
}

#[test]
fn accepting_stale_duplicates_cannot_hide_further_source_changes() {
    let root = project();
    let (_, run) = generate_review(root.path(), None, None, NOW).unwrap();
    fs::write(root.path().join("cpd.xml"),"<pmd-cpd><duplication lines='1' tokens='5'><file path='src/a.ts' line='1'/><file path='src/b.ts' line='1'/></duplication></pmd-cpd>").unwrap();
    archguard_engine::import_duplicates(
        root.path(),
        &run.config,
        &run.graph,
        Path::new("cpd.xml"),
        None,
    )
    .unwrap();
    fs::write(
        root.path().join("src/b.ts"),
        "import './c'; // first edit\n",
    )
    .unwrap();
    let (mut report, _) = generate_review(root.path(), None, None, NOW).unwrap();
    let finding = report
        .findings
        .iter()
        .find(|f| f.category == "duplication")
        .unwrap();
    assert_eq!(finding.source_status, "stale");
    let id = finding.id.clone();
    let fingerprint = finding.fingerprint.clone();
    let document=serde_json::to_vec(&json!({"schema_version":2,"review_revision":report.revision,"decision_revision":report.decision_fingerprint,"decisions":[{"id":id,"finding_ref":id,"finding_fingerprint":fingerprint,"decision":"accept-risk","author":"@quality","note":"Reviewed both current copies","expires_on":"2099-12-31","links":[]}]})).unwrap();
    apply_review_decisions(root.path(), &mut report, &document, NOW).unwrap();
    let (accepted, _) = generate_review(root.path(), None, None, NOW).unwrap();
    assert_eq!(
        accepted
            .findings
            .iter()
            .find(|f| f.id == id)
            .unwrap()
            .disposition,
        "accepted_risk"
    );
    fs::write(
        root.path().join("src/b.ts"),
        "import './c'; // second edit\n",
    )
    .unwrap();
    let (changed, _) = generate_review(root.path(), None, None, NOW).unwrap();
    let finding = changed.findings.iter().find(|f| f.id == id).unwrap();
    assert_eq!(finding.source_status, "stale");
    assert_ne!(
        finding.fingerprint, fingerprint,
        "stale status alone must not bind a decision to every future source version"
    );
    assert_eq!(finding.disposition, "changed");
}

#[test]
fn review_imports_share_the_register_with_recorded_decisions() {
    let root = project();
    let recorded = archguard_engine::core::ReviewDecision {
        id: "scc-1".into(),
        finding_ref: "scc-1".into(),
        note: "later".into(),
        author: "cli".into(),
        ..Default::default()
    };
    archguard_engine::upsert_decision(root.path(), recorded).unwrap();
    let (mut r, _) = generate_review(root.path(), None, None, NOW).unwrap();
    assert!(r.decisions.is_empty() && r.unmatched_decisions.is_empty());
    let decisions = document(&r);
    apply_review_decisions(root.path(), &mut r, &decisions, NOW).unwrap();
    let register = archguard_engine::load_register(root.path()).unwrap();
    assert_eq!(register.decisions.len(), 2, "the import keeps CLI entries");
    assert_eq!(
        r.decisions[0].ts, "2026-10-29T00:00:00Z",
        "import stamps ts"
    );
    let cycle = r.findings.iter().find(|f| f.evidence["kind"] == "cycle");
    assert_eq!(cycle.unwrap().disposition, "accepted_risk");
}
