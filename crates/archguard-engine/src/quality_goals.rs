//! Collects the evidence that quality goals need and assesses them with the
//! fixed mapping of docs/adr/0003-quality-goal-assessment.md. Only saved
//! snapshots are read; no scanner, test or build runs.
use archguard_core::{
    Config, EvidenceSource, Finding, GoalAdvisory, GoalAssessment, GoalKpi, Graph,
    ImportedEvidence, ReviewKpis,
};
use std::collections::BTreeSet;
use std::path::Path;

/// One assessment per declared goal, in file order.
pub fn quality_goal_assessment(
    root: &Path,
    config: &Config,
    graph: &Graph,
    findings: &[Finding],
    kpis: Option<&ReviewKpis>,
) -> Vec<GoalAssessment> {
    let goals = &config.architecture.goals;
    let needed: BTreeSet<EvidenceSource> = goals
        .iter()
        .filter_map(|goal| goal.attribute)
        .flat_map(archguard_analysis::goal_evidence_sources)
        .copied()
        .collect();
    let imported: Vec<_> = needed
        .into_iter()
        .filter_map(|source| read(source, root, config, graph))
        .collect();
    archguard_analysis::assess_goals(goals, findings, kpis, &imported)
        .expect("loaded goal scopes were validated")
}

/// A reader's snapshot counts as evidence when it is ready, partial or stale.
fn read(
    source: EvidenceSource,
    root: &Path,
    config: &Config,
    graph: &Graph,
) -> Option<ImportedEvidence> {
    let (status, kpis, advisories) = match source {
        EvidenceSource::StaticAnalysis | EvidenceSource::ReviewKpis => return None,
        EvidenceSource::DependencyAdvisories => security(root, config),
        EvidenceSource::MeasurementRuns => measurement_runs(root),
        EvidenceSource::TestResults => test_results(root, config, graph),
        EvidenceSource::Coverage => coverage(root, config, graph),
        EvidenceSource::Mutation => mutation(root, config, graph),
    };
    matches!(status.as_str(), "ready" | "partial" | "stale").then_some(ImportedEvidence {
        source,
        status,
        kpis,
        advisories,
    })
}

type Snapshot = (String, Vec<GoalKpi>, Vec<GoalAdvisory>);

fn kpi(name: &str, value: impl Into<f64>) -> GoalKpi {
    GoalKpi {
        name: name.into(),
        value: value.into(),
    }
}

fn count(value: usize) -> f64 {
    value as f64
}

fn security(root: &Path, config: &Config) -> Snapshot {
    let report = crate::security::read_security(root, config);
    let advisories = report
        .findings
        .iter()
        .map(|f| GoalAdvisory {
            id: f.id.clone(),
            package: f.package.clone(),
            severity: f.severity.clone(),
        })
        .collect();
    let kpis = vec![kpi("packages_scanned", count(report.packages_scanned))];
    (report.status, kpis, advisories)
}

fn measurement_runs(root: &Path) -> Snapshot {
    let report = crate::measurement_runs::read_measurement_runs(root);
    let mut kpis = vec![
        kpi("runs", count(report.runs)),
        kpi("groups", count(report.groups.len())),
        kpi(
            "inconsistent_test_groups",
            count(report.inconsistent_test_groups),
        ),
    ];
    kpis.extend(
        report
            .flaky_test_percent
            .map(|value| kpi("flaky_test_percent", value)),
    );
    (report.status, kpis, vec![])
}

fn test_results(root: &Path, config: &Config, graph: &Graph) -> Snapshot {
    let report = crate::test_evidence::read_test_evidence(root, config, graph);
    let failed = report
        .tests
        .iter()
        .filter(|t| matches!(t.outcome.as_str(), "failed" | "error"))
        .count();
    let kpis = vec![
        kpi("tests", count(report.tests.len())),
        kpi("tests_failed", count(failed)),
    ];
    (report.status, kpis, vec![])
}

fn coverage(root: &Path, config: &Config, graph: &Graph) -> Snapshot {
    let report = crate::test_coverage::read_test_coverage(root, config, graph);
    let kpis = vec![
        kpi("files", count(report.files.len())),
        kpi("unmatched_files", count(report.unmatched_files)),
    ];
    (report.status, kpis, vec![])
}

fn mutation(root: &Path, config: &Config, graph: &Graph) -> Snapshot {
    let report = crate::mutation_report::read_mutations(root, config, graph);
    let summary = &report.summary;
    let mut kpis = vec![
        kpi("mutants", count(summary.total)),
        kpi("survived", count(summary.survived)),
    ];
    kpis.extend(
        summary
            .killed_percent
            .map(|value| kpi("killed_percent", value)),
    );
    (report.status, kpis, vec![])
}
