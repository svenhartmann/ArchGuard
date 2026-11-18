//! How a workspace dependency or an unresolved declaration is recorded,
//! within the declaration limit.
use archguard_core::{
    ServiceEvidence, ServiceProject, WorkspaceDependency, WorkspaceReport, WorkspaceUnresolved,
};

pub(super) const MAX_DECLARATIONS: usize = 20_000;

pub(super) fn evidence(
    project: &ServiceProject,
    pointer: &str,
    line: Option<u32>,
    description: &str,
) -> ServiceEvidence {
    ServiceEvidence {
        file: project.manifest.clone(),
        pointer: pointer.into(),
        line,
        description: description.into(),
    }
}
pub(super) fn unresolved(
    report: &mut WorkspaceReport,
    project: &ServiceProject,
    target: &str,
    reason: &str,
    evidence: ServiceEvidence,
) {
    report.status = "partial".into();
    if report.unresolved.len() + report.dependencies.len() >= MAX_DECLARATIONS {
        if !report
            .diagnostics
            .iter()
            .any(|s| s.starts_with("Workspace declaration limit"))
        {
            report.diagnostics.push(
                "Workspace declaration limit reached; additional dependencies were not analyzed."
                    .into(),
            );
        }
        return;
    }
    report.unresolved.push(WorkspaceUnresolved {
        project: project.id.clone(),
        target: target.chars().take(1024).collect(),
        reason: reason.into(),
        evidence: vec![evidence],
    });
}
pub(super) fn dependency(
    report: &mut WorkspaceReport,
    project: &ServiceProject,
    target: &str,
    scope: &str,
    kind: &str,
    evidence: ServiceEvidence,
) {
    if target == project.id {
        unresolved(
            report,
            project,
            target,
            "self dependency does not identify a separate local project",
            evidence,
        );
        return;
    }
    if report.dependencies.len() + report.unresolved.len() >= MAX_DECLARATIONS {
        unresolved(report, project, target, "declaration limit", evidence);
        return;
    }
    if let Some(existing) = report
        .dependencies
        .iter_mut()
        .find(|d| d.from == project.id && d.to == target && d.scope == scope && d.kind == kind)
    {
        if !existing.evidence.contains(&evidence) {
            existing.evidence.push(evidence);
        }
    } else {
        report.dependencies.push(WorkspaceDependency {
            from: project.id.clone(),
            to: target.into(),
            scope: scope.into(),
            kind: kind.into(),
            evidence: vec![evidence],
        });
    }
}
