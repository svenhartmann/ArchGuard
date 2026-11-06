//! Quality goal assessment with the fixed evidence mapping of
//! docs/adr/0003-quality-goal-assessment.md. A goal is linked to findings
//! only through its declared ISO/IEC 25010 characteristic; nothing is
//! inferred from goal names, and no outcome reads as "goal met".
use archguard_core::{
    ArchguardError, ArchitectureGoal, EvidenceSource, Finding, FindingKind, GoalAssessment,
    GoalEvidenceSource, GoalKpi, GoalStatus, ImportedEvidence, QualityAttribute, ReviewKpis,
    Severity,
};
use globset::{Glob, GlobSet, GlobSetBuilder};
use std::collections::BTreeMap;

const NO_FINDINGS: &str = "No findings from the available sources; absent findings are not proof that the goal is reached.";

/// The characteristic a finding kind is evidence for. Exhaustive on purpose:
/// a new finding kind has to be placed in the mapping.
pub fn finding_attribute(kind: FindingKind) -> QualityAttribute {
    match kind {
        FindingKind::Cycle
        | FindingKind::LayerViolation
        | FindingKind::LayerUnranked
        | FindingKind::DependencyRule
        | FindingKind::OwnershipRule
        | FindingKind::TargetArchitecture
        | FindingKind::MetricThreshold
        | FindingKind::Orphan => QualityAttribute::Maintainability,
    }
}

/// The evidence sources ArchGuard has for a characteristic; empty means
/// the characteristic is not assessable.
pub fn goal_evidence_sources(attribute: QualityAttribute) -> &'static [EvidenceSource] {
    use EvidenceSource::*;
    match attribute {
        QualityAttribute::Maintainability => &[StaticAnalysis, ReviewKpis],
        QualityAttribute::Security => &[DependencyAdvisories],
        QualityAttribute::PerformanceEfficiency => &[MeasurementRuns],
        QualityAttribute::Reliability => &[TestResults, Coverage, Mutation, MeasurementRuns],
        QualityAttribute::FunctionalSuitability
        | QualityAttribute::Compatibility
        | QualityAttribute::Usability
        | QualityAttribute::Portability => &[],
    }
}

/// One assessment per goal, in the given order.
pub fn assess_goals(
    goals: &[ArchitectureGoal],
    findings: &[Finding],
    kpis: Option<&ReviewKpis>,
    imported: &[ImportedEvidence],
) -> Result<Vec<GoalAssessment>, ArchguardError> {
    let evidence = Evidence {
        findings,
        kpis,
        imported,
    };
    goals
        .iter()
        .map(|goal| Ok(evidence.assess(goal, &compile_scope(goal)?)))
        .collect()
}

fn compile_scope(goal: &ArchitectureGoal) -> Result<GlobSet, ArchguardError> {
    let invalid = |err: globset::Error| {
        ArchguardError::Config(format!("architecture.goals[{}].scope: {err}", goal.name))
    };
    let mut builder = GlobSetBuilder::new();
    for pattern in &goal.scope {
        builder.add(Glob::new(pattern).map_err(invalid)?);
    }
    builder.build().map_err(invalid)
}

struct Evidence<'a> {
    findings: &'a [Finding],
    kpis: Option<&'a ReviewKpis>,
    imported: &'a [ImportedEvidence],
}

impl Evidence<'_> {
    fn assess(&self, goal: &ArchitectureGoal, scope: &GlobSet) -> GoalAssessment {
        let mut out = GoalAssessment {
            goal: goal.name.clone(),
            priority: goal.priority.clone(),
            attribute: goal.attribute,
            scope: goal.scope.clone(),
            status: GoalStatus::Unclassified,
            findings: vec![],
            finding_count_by_severity: BTreeMap::new(),
            kpis: vec![],
            evidence_sources: vec![],
            gaps: vec![],
        };
        let Some(attribute) = goal.attribute else {
            out.gaps.push(format!(
                "Which ISO 25010 characteristic does {:?} address?",
                goal.name
            ));
            return out;
        };
        let sources = goal_evidence_sources(attribute);
        if sources.is_empty() {
            out.status = GoalStatus::NotAssessable;
            out.gaps.push(format!(
                "ArchGuard has no evidence source for {}.",
                attribute.as_str()
            ));
            return out;
        }
        out.evidence_sources = sources
            .iter()
            .map(|&source| GoalEvidenceSource {
                source,
                present: self.present(source),
            })
            .collect();
        let usable = match attribute {
            QualityAttribute::Maintainability => self.maintainability(&mut out, scope),
            QualityAttribute::Security => self.security(&mut out),
            _ => self.imported_counts(&mut out),
        };
        out.status = if !out.findings.is_empty() {
            GoalStatus::Findings
        } else if usable {
            out.gaps.push(NO_FINDINGS.into());
            GoalStatus::NoFindings
        } else {
            GoalStatus::NoEvidence
        };
        out
    }

    fn present(&self, source: EvidenceSource) -> bool {
        match source {
            EvidenceSource::StaticAnalysis => true,
            EvidenceSource::ReviewKpis => self.kpis.is_some(),
            _ => self.snapshot(source).is_some(),
        }
    }

    fn snapshot(&self, source: EvidenceSource) -> Option<&ImportedEvidence> {
        self.imported.iter().find(|e| e.source == source)
    }

    /// Report findings in scope plus project-wide review KPIs.
    fn maintainability(&self, out: &mut GoalAssessment, scope: &GlobSet) -> bool {
        for finding in self.findings {
            let in_scope =
                out.scope.is_empty() || finding.nodes.iter().any(|node| scope.is_match(node));
            if finding_attribute(finding.kind) == QualityAttribute::Maintainability && in_scope {
                out.findings.push(finding.id.clone());
                *out.finding_count_by_severity
                    .entry(severity_name(finding.severity).into())
                    .or_default() += 1;
            }
        }
        match self.kpis {
            Some(kpis) => out.kpis = maintainability_kpis(kpis),
            None => out
                .gaps
                .push("Review KPIs were not measured in this analysis.".into()),
        }
        if !out.scope.is_empty() && self.kpis.is_some() {
            out.gaps.push(
                "Review KPIs are project-wide; the goal's scope is not applied to them.".into(),
            );
        }
        true
    }

    fn security(&self, out: &mut GoalAssessment) -> bool {
        let Some(scan) = self.snapshot(EvidenceSource::DependencyAdvisories) else {
            out.gaps.push("No saved dependency security scan (.archguard/security.json); run archguard security explicitly.".into());
            return false;
        };
        note_status(out, scan);
        if !out.scope.is_empty() {
            out.gaps.push("Dependency advisories name manifests, not modules; they are not assigned to a scoped goal.".into());
            return false;
        }
        for advisory in &scan.advisories {
            out.findings
                .push(format!("{}@{}", advisory.id, advisory.package));
            *out.finding_count_by_severity
                .entry(advisory.severity.clone())
                .or_default() += 1;
        }
        true
    }

    /// Imported runtime and test evidence: presence and the readers' counts.
    fn imported_counts(&self, out: &mut GoalAssessment) -> bool {
        let mut usable = false;
        for source in out.evidence_sources.clone() {
            let Some(snapshot) = self.snapshot(source.source) else {
                out.gaps
                    .push(format!("No imported {} evidence.", source.source.as_str()));
                continue;
            };
            usable = true;
            note_status(out, snapshot);
            out.kpis.extend(snapshot.kpis.iter().map(|kpi| GoalKpi {
                name: format!("{}.{}", source.source.as_str(), kpi.name),
                value: kpi.value,
            }));
        }
        if usable {
            out.gaps.push("Imported evidence is reported as counts only; ArchGuard derives no findings from it.".into());
            if !out.scope.is_empty() {
                out.gaps.push(
                    "Imported counts are project-wide; the goal's scope is not applied to them."
                        .into(),
                );
            }
        }
        usable
    }
}

fn note_status(out: &mut GoalAssessment, snapshot: &ImportedEvidence) {
    if snapshot.status != "ready" {
        out.gaps.push(format!(
            "{} evidence is {}.",
            snapshot.source.as_str(),
            snapshot.status
        ));
    }
}

fn maintainability_kpis(kpis: &ReviewKpis) -> Vec<GoalKpi> {
    let kpi = |name: &str, value: f64| GoalKpi {
        name: name.into(),
        value,
    };
    let mut out = vec![];
    if let Some(max) = kpis.propagation_cost.iter().map(|c| c.pc).reduce(f64::max) {
        out.push(kpi("propagation_cost_max", max));
    }
    if let Some(tangled) = &kpis.tangledness {
        out.push(kpi("tangledness", tangled.quote));
    }
    out.push(kpi("sdp_violations", kpis.sdp_violations.len() as f64));
    if let Some(zones) = &kpis.zones {
        out.push(kpi("zone_of_pain", zones.zone_of_pain as f64));
        out.push(kpi("zone_of_uselessness", zones.zone_of_uselessness as f64));
    }
    out
}

fn severity_name(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
    }
}
