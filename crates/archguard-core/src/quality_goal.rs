//! Quality goals linked to ISO/IEC 25010 characteristics and the evidence
//! ArchGuard has for them (docs/adr/0003-quality-goal-assessment.md).
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The product quality characteristics of ISO/IEC 25010.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityAttribute {
    FunctionalSuitability,
    PerformanceEfficiency,
    Compatibility,
    Usability,
    Reliability,
    Security,
    Maintainability,
    Portability,
}

impl QualityAttribute {
    /// The configuration spelling, e.g. `performance_efficiency`.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FunctionalSuitability => "functional_suitability",
            Self::PerformanceEfficiency => "performance_efficiency",
            Self::Compatibility => "compatibility",
            Self::Usability => "usability",
            Self::Reliability => "reliability",
            Self::Security => "security",
            Self::Maintainability => "maintainability",
            Self::Portability => "portability",
        }
    }
}

/// What ArchGuard can say about one goal; never "goal met".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    /// The goal names no characteristic.
    Unclassified,
    /// ArchGuard has no evidence source for the characteristic.
    NotAssessable,
    /// The needed evidence source is absent or cannot be applied.
    NoEvidence,
    /// At least one finding matches the goal.
    Findings,
    /// Sources are present; no findings from the available sources.
    NoFindings,
}

impl GoalStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unclassified => "unclassified",
            Self::NotAssessable => "not_assessable",
            Self::NoEvidence => "no_evidence",
            Self::Findings => "findings",
            Self::NoFindings => "no_findings",
        }
    }
}

/// An evidence source of the fixed mapping in ADR 0003.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceSource {
    /// Findings of the analysis report; always present.
    StaticAnalysis,
    ReviewKpis,
    /// `.archguard/security.json`
    DependencyAdvisories,
    /// `.archguard/measurement-runs.json`
    MeasurementRuns,
    /// `.archguard/tests.json`
    TestResults,
    /// `.archguard/coverage.json`
    Coverage,
    /// `.archguard/mutation.json`
    Mutation,
}

impl EvidenceSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::StaticAnalysis => "static_analysis",
            Self::ReviewKpis => "review_kpis",
            Self::DependencyAdvisories => "dependency_advisories",
            Self::MeasurementRuns => "measurement_runs",
            Self::TestResults => "test_results",
            Self::Coverage => "coverage",
            Self::Mutation => "mutation",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoalKpi {
    pub name: String,
    pub value: f64,
}

/// One saved dependency advisory; severity is the scanner's own word.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalAdvisory {
    pub id: String,
    pub package: String,
    pub severity: String,
}

/// A saved report that its reader accepted (`ready`, `partial` or `stale`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportedEvidence {
    pub source: EvidenceSource,
    /// The reader's status, e.g. `ready`.
    pub status: String,
    /// Counts the reader already reports.
    pub kpis: Vec<GoalKpi>,
    pub advisories: Vec<GoalAdvisory>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalEvidenceSource {
    pub source: EvidenceSource,
    pub present: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GoalAssessment {
    pub goal: String,
    /// The organization's own priority vocabulary.
    pub priority: String,
    pub attribute: Option<QualityAttribute>,
    pub scope: Vec<String>,
    pub status: GoalStatus,
    /// Ids of the matching findings, in report order.
    pub findings: Vec<String>,
    pub finding_count_by_severity: BTreeMap<String, usize>,
    pub kpis: Vec<GoalKpi>,
    pub evidence_sources: Vec<GoalEvidenceSource>,
    pub gaps: Vec<String>,
}
