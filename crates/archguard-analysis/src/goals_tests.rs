use crate::goals::assess_goals;
use archguard_core::{
    ArchitectureGoal, EvidenceSource, Finding, FindingKind, GoalAdvisory, GoalEvidenceSource,
    GoalKpi, GoalStatus, ImportedEvidence, PropagationCost, QualityAttribute, ReviewKpis, Severity,
    Tangledness,
};

fn goal(name: &str, attribute: Option<QualityAttribute>, scope: &[&str]) -> ArchitectureGoal {
    ArchitectureGoal {
        name: name.into(),
        priority: "must for checkout".into(),
        attribute,
        scope: scope.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    }
}

fn finding(id: &str, kind: FindingKind, severity: Severity, nodes: &[&str]) -> Finding {
    Finding {
        kind,
        id: id.into(),
        severity,
        message: None,
        evidence: vec![],
        nodes: nodes.iter().map(|s| s.to_string()).collect(),
        edge_count: 0,
        cycles_sampled: 0,
        sampling: None,
        suggested_cuts: vec![],
    }
}

fn imported(source: EvidenceSource, status: &str) -> ImportedEvidence {
    ImportedEvidence {
        source,
        status: status.into(),
        kpis: vec![GoalKpi {
            name: "files".into(),
            value: 3.0,
        }],
        advisories: vec![],
    }
}

fn present(source: EvidenceSource, present: bool) -> GoalEvidenceSource {
    GoalEvidenceSource { source, present }
}

#[test]
fn a_goal_without_characteristic_is_unclassified_and_asks_for_one() {
    let out = assess_goals(&[goal("Fast delivery", None, &[])], &[], None, &[]).unwrap();
    assert_eq!(out[0].status, GoalStatus::Unclassified);
    assert_eq!(out[0].attribute, None);
    assert!(out[0].evidence_sources.is_empty());
    assert_eq!(
        out[0].gaps,
        vec!["Which ISO 25010 characteristic does \"Fast delivery\" address?"]
    );
}

#[test]
fn characteristics_without_archguard_evidence_are_not_assessable() {
    for attribute in [
        QualityAttribute::FunctionalSuitability,
        QualityAttribute::Compatibility,
        QualityAttribute::Usability,
        QualityAttribute::Portability,
    ] {
        let cycle = finding("scc-1", FindingKind::Cycle, Severity::Error, &["a"]);
        let out = assess_goals(&[goal("G", Some(attribute), &[])], &[cycle], None, &[]).unwrap();
        assert_eq!(out[0].status, GoalStatus::NotAssessable, "{attribute:?}");
        assert!(out[0].findings.is_empty());
        assert!(out[0].evidence_sources.is_empty());
    }
}

#[test]
fn an_absent_security_scan_is_no_evidence_not_a_pass() {
    let out = assess_goals(
        &[goal("Safe", Some(QualityAttribute::Security), &[])],
        &[],
        None,
        &[],
    )
    .unwrap();
    assert_eq!(out[0].status, GoalStatus::NoEvidence);
    assert_eq!(
        out[0].evidence_sources,
        vec![present(EvidenceSource::DependencyAdvisories, false)]
    );
    assert!(out[0].gaps[0].contains(".archguard/security.json"));
}

#[test]
fn every_structural_finding_kind_links_to_maintainability_with_severity_counts() {
    let findings = vec![
        finding("scc-1", FindingKind::Cycle, Severity::Error, &["a"]),
        finding(
            "layer-1",
            FindingKind::LayerViolation,
            Severity::Error,
            &["a"],
        ),
        finding(
            "unranked-1",
            FindingKind::LayerUnranked,
            Severity::Warning,
            &["a"],
        ),
        finding(
            "rule-1",
            FindingKind::DependencyRule,
            Severity::Warning,
            &["a"],
        ),
        finding(
            "owner-1",
            FindingKind::OwnershipRule,
            Severity::Warning,
            &["a"],
        ),
        finding(
            "target-1",
            FindingKind::TargetArchitecture,
            Severity::Error,
            &["a"],
        ),
        finding(
            "metric-1",
            FindingKind::MetricThreshold,
            Severity::Info,
            &["a"],
        ),
        finding("orphan-1", FindingKind::Orphan, Severity::Info, &["a"]),
    ];
    let goals = [goal(
        "Changeable",
        Some(QualityAttribute::Maintainability),
        &[],
    )];
    let out = assess_goals(&goals, &findings, None, &[]).unwrap();
    assert_eq!(out[0].status, GoalStatus::Findings);
    assert_eq!(
        out[0].findings,
        findings.iter().map(|f| f.id.clone()).collect::<Vec<_>>()
    );
    let counts: Vec<_> = out[0]
        .finding_count_by_severity
        .iter()
        .map(|(k, v)| (k.as_str(), *v))
        .collect();
    assert_eq!(counts, vec![("error", 3), ("info", 2), ("warning", 3)]);
    assert_eq!(
        out[0].evidence_sources,
        vec![
            present(EvidenceSource::StaticAnalysis, true),
            present(EvidenceSource::ReviewKpis, false)
        ]
    );
}

#[test]
fn scope_globs_select_findings_by_node_and_name_project_wide_kpis() {
    let findings = vec![
        finding(
            "scc-1",
            FindingKind::Cycle,
            Severity::Error,
            &["src/app/a.ts"],
        ),
        finding(
            "scc-2",
            FindingKind::Cycle,
            Severity::Error,
            &["src/lib/b.ts"],
        ),
        finding(
            "scc-3",
            FindingKind::Cycle,
            Severity::Warning,
            &["src/lib/c.ts", "src/app/d.ts"],
        ),
    ];
    let goals = [goal(
        "App",
        Some(QualityAttribute::Maintainability),
        &["src/app/**"],
    )];
    let kpis = ReviewKpis::default();
    let out = assess_goals(&goals, &findings, Some(&kpis), &[]).unwrap();
    assert_eq!(out[0].findings, vec!["scc-1", "scc-3"]);
    assert_eq!(out[0].scope, vec!["src/app/**"]);
    assert!(out[0].gaps.iter().any(|gap| gap.contains("project-wide")));
}

#[test]
fn present_sources_without_matches_report_no_findings_never_goal_met() {
    let goals = [
        goal("App", Some(QualityAttribute::Maintainability), &["none/**"]),
        goal("Robust", Some(QualityAttribute::Reliability), &[]),
    ];
    let cycle = finding("scc-1", FindingKind::Cycle, Severity::Error, &["src/a.ts"]);
    let evidence = [imported(EvidenceSource::Coverage, "stale")];
    let out = assess_goals(&goals, &[cycle], None, &evidence).unwrap();
    assert_eq!(out[0].status, GoalStatus::NoFindings);
    assert_eq!(out[1].status, GoalStatus::NoFindings);
    assert_eq!(
        out[1].evidence_sources,
        vec![
            present(EvidenceSource::TestResults, false),
            present(EvidenceSource::Coverage, true),
            present(EvidenceSource::Mutation, false),
            present(EvidenceSource::MeasurementRuns, false),
        ]
    );
    assert_eq!(out[1].kpis[0].name, "coverage.files");
    assert!(out[1].gaps.iter().any(|gap| gap.contains("stale")));
    for assessment in &out {
        assert!(
            assessment
                .gaps
                .iter()
                .any(|gap| gap.contains("No findings from the available sources"))
        );
        let text = serde_json::to_string(assessment).unwrap().to_lowercase();
        assert!(
            !text.contains("goal met") && !text.contains("is met"),
            "{text}"
        );
    }
}

#[test]
fn reliability_and_performance_without_imports_have_no_evidence() {
    let goals = [
        goal("Robust", Some(QualityAttribute::Reliability), &[]),
        goal("Fast", Some(QualityAttribute::PerformanceEfficiency), &[]),
    ];
    let out = assess_goals(&goals, &[], None, &[]).unwrap();
    assert_eq!(out[0].status, GoalStatus::NoEvidence);
    assert_eq!(out[1].status, GoalStatus::NoEvidence);
    assert_eq!(
        out[1].evidence_sources,
        vec![present(EvidenceSource::MeasurementRuns, false)]
    );
}

#[test]
fn saved_advisories_are_security_findings_unless_a_scope_needs_modules() {
    let mut scan = imported(EvidenceSource::DependencyAdvisories, "ready");
    scan.kpis.clear();
    scan.advisories = vec![
        GoalAdvisory {
            id: "CVE-1".into(),
            package: "lodash".into(),
            severity: "HIGH".into(),
        },
        GoalAdvisory {
            id: "CVE-2".into(),
            package: "serde".into(),
            severity: "LOW".into(),
        },
    ];
    let goals = [
        goal("Safe", Some(QualityAttribute::Security), &[]),
        goal(
            "Safe app",
            Some(QualityAttribute::Security),
            &["src/app/**"],
        ),
    ];
    let out = assess_goals(&goals, &[], None, &[scan]).unwrap();
    assert_eq!(out[0].status, GoalStatus::Findings);
    assert_eq!(out[0].findings, vec!["CVE-1@lodash", "CVE-2@serde"]);
    assert_eq!(out[0].finding_count_by_severity["HIGH"], 1);
    assert_eq!(out[1].status, GoalStatus::NoEvidence);
    assert!(out[1].findings.is_empty());
    assert!(out[1].gaps.iter().any(|gap| gap.contains("manifests")));
}

#[test]
fn maintainability_kpis_come_from_measured_review_values_only() {
    let kpis = ReviewKpis {
        propagation_cost: vec![
            PropagationCost {
                scc_id: "scc-1".into(),
                size: 2,
                affected: 3,
                total: 10,
                pc: 0.3,
            },
            PropagationCost {
                scc_id: "scc-2".into(),
                size: 2,
                affected: 5,
                total: 10,
                pc: 0.5,
            },
        ],
        tangledness: Some(Tangledness {
            tangled_nodes: 2,
            total: 10,
            quote: 0.2,
        }),
        ..Default::default()
    };
    let goals = [goal(
        "Changeable",
        Some(QualityAttribute::Maintainability),
        &[],
    )];
    let out = assess_goals(&goals, &[], Some(&kpis), &[]).unwrap();
    let names: Vec<_> = out[0]
        .kpis
        .iter()
        .map(|k| (k.name.as_str(), k.value))
        .collect();
    assert_eq!(
        names,
        vec![
            ("propagation_cost_max", 0.5),
            ("tangledness", 0.2),
            ("sdp_violations", 0.0)
        ],
        "unmeasured zones stay absent instead of 0"
    );
    assert!(out[0].evidence_sources[1].present);
}

#[test]
fn goals_keep_file_order_and_the_organizations_priority_words() {
    let goals = [
        goal("Zeta", None, &[]),
        goal("Alpha", Some(QualityAttribute::Usability), &[]),
    ];
    let out = assess_goals(&goals, &[], None, &[]).unwrap();
    assert_eq!(out[0].goal, "Zeta");
    assert_eq!(out[1].goal, "Alpha");
    assert_eq!(out[1].priority, "must for checkout");
}

#[test]
fn invalid_scope_globs_are_rejected() {
    let goals = [goal(
        "G",
        Some(QualityAttribute::Maintainability),
        &["src/[app"],
    )];
    assert!(assess_goals(&goals, &[], None, &[]).is_err());
}
