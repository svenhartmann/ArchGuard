use archguard_engine::core::GoalStatus;
use archguard_engine::{Level, analyze_run, arc42_document, architecture_context, load_config};
use std::fs;

const GOALS: &str = r#"
[[architecture.goals]]
name = "Changeable checkout"
priority = "first"
rationale = "Weekly releases"
attribute = "maintainability"
scope = ["app", "app/**"]
scenario = "A pricing change ships within one day"
[[architecture.goals]]
name = "Unrelated area"
attribute = "maintainability"
scope = ["billing/**"]
[[architecture.goals]]
name = "Safe dependencies"
attribute = "security"
[[architecture.goals]]
name = "Pleasant UI"
attribute = "usability"
[[architecture.goals]]
name = "Fast delivery"
"#;

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (path, contents) in [
        (
            "src/app/one.ts",
            "import { two } from '../lib/two';\nexport const one = () => two();\n",
        ),
        (
            "src/lib/two.ts",
            "import { one } from '../app/one';\nexport const two = () => one();\n",
        ),
    ] {
        let path = dir.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
    fs::write(
        dir.path().join("archguard.toml"),
        format!(
            "derive_layers = false\n[project]\nsource_roots = ['src']\nlanguages = ['ts']\nexclude = []\n[modules]\ndepth = 1\n[rules]\ndeny_cycles = 'module'\n{GOALS}"
        ),
    )
    .unwrap();
    dir
}

#[test]
fn a_cycle_is_linked_to_the_maintainability_goal_whose_scope_it_touches() {
    let dir = project();
    let loaded = load_config(None, dir.path()).unwrap();
    let run = analyze_run(dir.path(), &loaded, Level::Module).unwrap();
    let cycle = run
        .report
        .findings
        .iter()
        .find(|f| f.kind == archguard_engine::core::FindingKind::Cycle)
        .expect("fixture has a cycle");
    let context = architecture_context(dir.path(), &run.config, &run.graph, &run.report);
    let goals = &context.goal_assessment;
    let names: Vec<_> = goals.iter().map(|g| g.goal.as_str()).collect();
    assert_eq!(
        names,
        [
            "Changeable checkout",
            "Unrelated area",
            "Safe dependencies",
            "Pleasant UI",
            "Fast delivery"
        ]
    );
    assert_eq!(goals[0].status, GoalStatus::Findings);
    assert!(goals[0].findings.contains(&cycle.id));
    // Every finding of the fixture touches `app` (the cycle and the SCC size
    // threshold on it), and every structural kind maps to maintainability.
    let all: Vec<_> = run.report.findings.iter().map(|f| f.id.clone()).collect();
    assert_eq!(goals[0].findings, all);
    assert_eq!(goals[0].priority, "first");
    assert_eq!(goals[1].status, GoalStatus::NoFindings);
    assert_eq!(goals[2].status, GoalStatus::NoEvidence, "no saved scan");
    assert_eq!(goals[3].status, GoalStatus::NotAssessable);
    assert_eq!(goals[4].status, GoalStatus::Unclassified);
    assert!(
        context
            .questions
            .contains(&"Which ISO 25010 characteristic does \"Fast delivery\" address?".into())
    );

    let doc = arc42_document(dir.path(), &run.config, &run.graph, &run.report).unwrap();
    let chapter1 = &doc.chapters[0].markdown;
    assert!(chapter1.contains("| Changeable checkout | first | Weekly releases | maintainability | app, app/\\*\\* | Findings |"), "{chapter1}");
    let chapter10 = &doc.chapters[9].markdown;
    assert!(
        chapter10.contains("A pricing change ships within one day"),
        "{chapter10}"
    );
    assert!(
        chapter10.contains(&format!("`{}`", cycle.id)),
        "{chapter10}"
    );
    assert!(chapter10.contains("No findings from the available sources"));
}

#[test]
fn a_saved_security_scan_turns_no_evidence_into_findings() {
    let dir = project();
    let loaded = load_config(None, dir.path()).unwrap();
    let run = analyze_run(dir.path(), &loaded, Level::Module).unwrap();
    let security = || {
        architecture_context(dir.path(), &run.config, &run.graph, &run.report).goal_assessment[2]
            .clone()
    };
    fs::create_dir_all(dir.path().join(".archguard")).unwrap();
    fs::write(dir.path().join(".archguard/security.json"), "{}").unwrap();
    assert_eq!(
        security().status,
        GoalStatus::NoEvidence,
        "an unreadable snapshot is not evidence"
    );
    let fingerprint = archguard_engine::security_fingerprint(dir.path(), &run.config).unwrap();
    let snapshot = serde_json::json!({
        "schema_version": 1, "status": "ready", "scanner": "trivy", "scanner_version": null,
        "scanned_at": null, "database_updated_at": null, "fingerprint": fingerprint,
        "packages_scanned": 1, "targets": [], "diagnostics": [],
        "findings": [{"id": "CVE-2026-1", "package": "left-pad", "installed_version": "1.0.0",
            "fixed_version": null, "ecosystem": "npm", "severity": "HIGH", "title": "t",
            "url": null, "targets": ["package-lock.json"]}]
    });
    fs::write(
        dir.path().join(".archguard/security.json"),
        snapshot.to_string(),
    )
    .unwrap();
    let assessed = security();
    assert_eq!(assessed.status, GoalStatus::Findings);
    assert_eq!(assessed.findings, vec!["CVE-2026-1@left-pad"]);
    assert_eq!(assessed.finding_count_by_severity["HIGH"], 1);
}
