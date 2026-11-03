//! Persistent review decision register.
//!
//! Stub created by the R01 skeleton task; the decision semantics
//! (idempotent upsert, open-vs-decided filtering) are implemented in R02.

use serde::{Deserialize, Serialize};

/// A finding as addressed from the decision register: stable id plus the
/// severity it was reported with.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FindingRef {
    /// Stable finding id, e.g. `scc-3`.
    pub finding_id: String,
    /// Severity name as reported (`error`, `warning`, `info`).
    pub severity: String,
}

/// What a reviewer decided about a finding.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum DecisionKind {
    /// Fix it (stays open until the finding is gone).
    #[default]
    Fix,
    /// Fix it later.
    Defer,
    /// Live with it; an ADR link is expected (gated by `[review]`).
    AcceptRisk,
}

/// One recorded decision about one finding.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReviewDecision {
    /// Stable decision id; re-recording the same id replaces the entry.
    pub id: String,
    /// The finding this decision is about.
    pub finding_ref: String,
    pub decision: DecisionKind,
    /// ADR link; required for `AcceptRisk` when the gate is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adr_link: Option<String>,
    pub note: String,
    pub author: String,
    /// ISO-8601 timestamp as recorded.
    pub ts: String,
}

/// The register persisted to `.archguard/reviews.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReviewRegister {
    /// File format version for forward-compatible extension.
    pub version: u32,
    pub decisions: Vec<ReviewDecision>,
}

impl ReviewRegister {
    /// Record a decision, replacing any earlier decision with the same
    /// `id` (idempotent upsert: re-recording never duplicates entries).
    pub fn upsert(&mut self, decision: ReviewDecision) {
        match self.decisions.iter_mut().find(|d| d.id == decision.id) {
            Some(existing) => *existing = decision,
            None => self.decisions.push(decision),
        }
    }

    /// The latest decision recorded for a finding, if any.
    pub fn decision_for(&self, finding_id: &str) -> Option<&ReviewDecision> {
        self.decisions.iter().find(|d| d.finding_ref == finding_id)
    }

    /// Findings that still need reviewer attention.
    ///
    /// A finding counts as decided only when a `Defer` or `AcceptRisk`
    /// decision references it; a plain `Fix` decision keeps it open
    /// (fixing does not resolve it — the finding disappearing does).
    pub fn open_findings(&self, all: &[FindingRef]) -> Vec<FindingRef> {
        all.iter()
            .filter(|f| match self.decision_for(&f.finding_id) {
                None => true,
                Some(d) => d.decision == DecisionKind::Fix,
            })
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decision(id: &str, finding: &str, kind: DecisionKind) -> ReviewDecision {
        ReviewDecision {
            id: id.to_string(),
            finding_ref: finding.to_string(),
            decision: kind,
            adr_link: None,
            note: "checked".to_string(),
            author: "sven".to_string(),
            ts: "2026-10-30T09:00:00Z".to_string(),
        }
    }

    #[test]
    fn register_round_trips_through_json() {
        let mut register = ReviewRegister {
            version: 1,
            decisions: Vec::new(),
        };
        let mut accepted = decision("d-1", "scc-3", DecisionKind::AcceptRisk);
        accepted.adr_link = Some("docs/adr/004.md".to_string());
        register.upsert(accepted);
        let json = serde_json::to_string(&register).expect("serialization");
        let back: ReviewRegister = serde_json::from_str(&json).expect("deserialization");
        assert_eq!(register, back);
    }

    #[test]
    fn decision_kind_uses_kebab_case_names() {
        for (kind, name) in [
            (DecisionKind::Fix, "fix"),
            (DecisionKind::Defer, "defer"),
            (DecisionKind::AcceptRisk, "accept-risk"),
        ] {
            let json = serde_json::to_string(&kind).expect("serialization");
            assert_eq!(json, format!("\"{name}\""));
            let back: DecisionKind = serde_json::from_str(&json).expect("deserialization");
            assert_eq!(back, kind);
        }
    }

    #[test]
    fn upsert_is_idempotent_by_id() {
        let mut register = ReviewRegister::default();
        register.upsert(decision("d-1", "scc-3", DecisionKind::Defer));
        register.upsert(decision("d-1", "scc-3", DecisionKind::AcceptRisk));
        assert_eq!(register.decisions.len(), 1);
        let d = register.decision_for("scc-3").expect("recorded");
        assert_eq!(d.decision, DecisionKind::AcceptRisk);
        // A different id adds an entry instead of replacing.
        register.upsert(decision("d-2", "scc-4", DecisionKind::Fix));
        assert_eq!(register.decisions.len(), 2);
    }

    #[test]
    fn open_findings_filters_defer_and_accept_risk() {
        let all = vec![
            FindingRef {
                finding_id: "scc-1".to_string(),
                severity: "error".to_string(),
            },
            FindingRef {
                finding_id: "scc-2".to_string(),
                severity: "warning".to_string(),
            },
            FindingRef {
                finding_id: "scc-3".to_string(),
                severity: "warning".to_string(),
            },
            FindingRef {
                finding_id: "scc-4".to_string(),
                severity: "error".to_string(),
            },
        ];
        let mut register = ReviewRegister::default();
        // scc-2 deferred, scc-3 accepted with an ADR, scc-4 planned fix
        // (planned fixes stay open).
        register.upsert(decision("d-2", "scc-2", DecisionKind::Defer));
        let mut accepted = decision("d-3", "scc-3", DecisionKind::AcceptRisk);
        accepted.adr_link = Some("docs/adr/001.md".to_string());
        register.upsert(accepted);
        register.upsert(decision("d-4", "scc-4", DecisionKind::Fix));

        let open: Vec<String> = register
            .open_findings(&all)
            .into_iter()
            .map(|f| f.finding_id)
            .collect();
        assert_eq!(open, ["scc-1", "scc-4"]);
    }

    #[test]
    fn empty_register_keeps_everything_open() {
        let all = vec![FindingRef {
            finding_id: "scc-1".to_string(),
            severity: "error".to_string(),
        }];
        let register = ReviewRegister::default();
        assert_eq!(register.open_findings(&all), all);
    }
}
