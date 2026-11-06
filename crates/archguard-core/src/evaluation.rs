//! Expert assessment of a system (ADR 0004): the problems an architect
//! expects ArchGuard to report, and the architect's verdicts on reported
//! findings. ArchGuard never writes verdicts or expected problems itself;
//! this module only describes and validates what a human recorded.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::model::{ArchguardError, FindingKind};

/// One assessment file (`.archguard/assessment.toml`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assessment {
    pub assessment: AssessmentMeta,
    #[serde(default)]
    pub expected: Vec<ExpectedProblem>,
    #[serde(default)]
    pub judged: Vec<JudgedFinding>,
}

/// Who assessed which revision, and what the assessment covers.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssessmentMeta {
    pub assessor: String,
    pub date: String,
    /// Git commit the assessment refers to.
    pub revision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

/// Problem kind of an expected item: an ArchGuard [`FindingKind`], or
/// `other` for problems ArchGuard has no finding kind for. `other` can
/// never be matched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpectedKind {
    Finding(FindingKind),
    Other,
}

impl fmt::Display for ExpectedKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExpectedKind::Other => f.write_str("other"),
            ExpectedKind::Finding(kind) => f.write_str(finding_kind_name(*kind)),
        }
    }
}

/// The snake_case name of a [`FindingKind`], as in reports.
pub fn finding_kind_name(kind: FindingKind) -> &'static str {
    match kind {
        FindingKind::Cycle => "cycle",
        FindingKind::LayerViolation => "layer_violation",
        FindingKind::MetricThreshold => "metric_threshold",
        FindingKind::LayerUnranked => "layer_unranked",
        FindingKind::DependencyRule => "dependency_rule",
        FindingKind::OwnershipRule => "ownership_rule",
        FindingKind::Orphan => "orphan",
        FindingKind::TargetArchitecture => "target_architecture",
    }
}

const FINDING_KINDS: [FindingKind; 8] = [
    FindingKind::Cycle,
    FindingKind::LayerViolation,
    FindingKind::MetricThreshold,
    FindingKind::LayerUnranked,
    FindingKind::DependencyRule,
    FindingKind::OwnershipRule,
    FindingKind::Orphan,
    FindingKind::TargetArchitecture,
];

impl<'de> Deserialize<'de> for ExpectedKind {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let text = String::deserialize(d)?;
        if text == "other" {
            return Ok(ExpectedKind::Other);
        }
        FINDING_KINDS
            .into_iter()
            .find(|k| finding_kind_name(*k) == text)
            .map(ExpectedKind::Finding)
            .ok_or_else(|| {
                let names: Vec<_> = FINDING_KINDS
                    .iter()
                    .map(|k| finding_kind_name(*k))
                    .collect();
                serde::de::Error::custom(format!(
                    "unknown kind {text:?}; use one of {} or \"other\"",
                    names.join(", ")
                ))
            })
    }
}

impl Serialize for ExpectedKind {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

/// A problem the expert sees in the system.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedProblem {
    pub id: String,
    pub kind: ExpectedKind,
    /// Globs over finding node names; each must match a node of the finding.
    #[serde(default)]
    pub nodes: Vec<String>,
    #[serde(default)]
    pub note: Option<String>,
}

/// The expert's verdict on one reported finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    TruePositive,
    FalsePositive,
}

/// A verdict on a reported finding, named by its stable finding id
/// (the SHA-256 `id` of `archguard review`).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JudgedFinding {
    pub finding: String,
    /// `None` (key absent or `""`) means not judged yet.
    #[serde(default, deserialize_with = "optional_verdict")]
    pub verdict: Option<Verdict>,
    #[serde(default)]
    pub note: Option<String>,
}

/// `""` is an explicit "not judged yet"; it never defaults to a verdict.
fn optional_verdict<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Verdict>, D::Error> {
    let text = String::deserialize(d)?;
    match text.as_str() {
        "" => Ok(None),
        "true_positive" => Ok(Some(Verdict::TruePositive)),
        "false_positive" => Ok(Some(Verdict::FalsePositive)),
        other => Err(serde::de::Error::custom(format!(
            "unknown verdict {other:?}; use \"true_positive\", \"false_positive\" or \"\" (not judged)"
        ))),
    }
}

impl Assessment {
    /// Reject assessments whose meaning would be ambiguous.
    pub fn validate(&self) -> Result<(), ArchguardError> {
        let invalid = |m: String| Err(ArchguardError::Config(format!("assessment: {m}")));
        let mut ids = BTreeSet::new();
        for e in &self.expected {
            if e.id.trim().is_empty() {
                return invalid("expected item with empty id".into());
            }
            if !ids.insert(e.id.as_str()) {
                return invalid(format!("duplicate expected id {:?}", e.id));
            }
            if matches!(e.kind, ExpectedKind::Finding(_)) && e.nodes.is_empty() {
                return invalid(format!("expected {:?} needs at least one node glob", e.id));
            }
        }
        let mut findings = BTreeSet::new();
        for j in &self.judged {
            if j.finding.trim().is_empty() {
                return invalid("judged item with empty finding id".into());
            }
            if !findings.insert(j.finding.as_str()) {
                return invalid(format!("finding {:?} is judged twice", j.finding));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const META: &str = "[assessment]\nassessor='A'\ndate='2026-11-06'\nrevision='abc'\n";

    fn parse(body: &str) -> Result<Assessment, String> {
        let a: Assessment = toml::from_str(&format!("{META}{body}")).map_err(|e| e.to_string())?;
        a.validate().map_err(|e| e.to_string())?;
        Ok(a)
    }

    #[test]
    fn unknown_verdict_is_rejected() {
        let err = parse("[[judged]]\nfinding='f1'\nverdict='maybe'\n").unwrap_err();
        assert!(err.contains("maybe"), "{err}");
    }

    #[test]
    fn empty_or_missing_verdict_is_unjudged() {
        let a = parse("[[judged]]\nfinding='f1'\nverdict=''\n[[judged]]\nfinding='f2'\n").unwrap();
        assert_eq!(a.judged[0].verdict, None);
        assert_eq!(a.judged[1].verdict, None);
    }

    #[test]
    fn verdicts_and_kinds_parse() {
        let a = parse(
            "[[expected]]\nid='E1'\nkind='cycle'\nnodes=['a/*']\n[[expected]]\nid='E2'\nkind='other'\n\
             [[judged]]\nfinding='f1'\nverdict='true_positive'\n[[judged]]\nfinding='f2'\nverdict='false_positive'\n",
        )
        .unwrap();
        assert_eq!(
            a.expected[0].kind,
            ExpectedKind::Finding(FindingKind::Cycle)
        );
        assert_eq!(a.expected[1].kind, ExpectedKind::Other);
        assert_eq!(a.judged[0].verdict, Some(Verdict::TruePositive));
        assert_eq!(a.judged[1].verdict, Some(Verdict::FalsePositive));
    }

    #[test]
    fn unknown_kind_names_the_alternatives() {
        let err = parse("[[expected]]\nid='E1'\nkind='hotspot'\nnodes=['a']\n").unwrap_err();
        assert!(err.contains("\"other\""), "{err}");
    }

    #[test]
    fn ambiguous_assessments_are_rejected() {
        assert!(parse("[[expected]]\nid='E1'\nkind='cycle'\n").is_err());
        assert!(
            parse("[[expected]]\nid='E1'\nkind='other'\n[[expected]]\nid='E1'\nkind='other'\n")
                .is_err()
        );
        assert!(parse("[[judged]]\nfinding='f'\n[[judged]]\nfinding='f'\n").is_err());
        assert!(parse("[[judged]]\nverdict='true_positive'\n").is_err());
        assert!(parse("[[judged]]\nfinding='f'\nverdikt='x'\n").is_err());
    }
}
