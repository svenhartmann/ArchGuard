//! The decision document of a review, as `archguard review --decisions`
//! imports it: the desktop app records and exports it, `review-decision
//! export` prints it. The import (`apply_review_decisions`) stays the only
//! validation; this module only assembles.
use crate::review_register::{IMPORT_SCHEMA, ReviewDecisionImport};
use crate::review_session::{ReviewFinding, ReviewReport};
use archguard_core::{DecisionKind, ReviewDecision};
use std::collections::BTreeMap;

impl ReviewFinding {
    /// A decision bound to this finding's current evidence; the caller
    /// fills author, note and links, the import stamps the time.
    pub fn draft_decision(&self, decision: DecisionKind) -> ReviewDecision {
        ReviewDecision {
            id: self.id.clone(),
            finding_ref: self.id.clone(),
            finding_fingerprint: Some(self.fingerprint.clone()),
            decision,
            ..Default::default()
        }
    }
}

/// The review's recorded decisions with `drafts` replacing those of the
/// same finding, bound to the review revision and its decision state.
pub fn review_decision_document(
    report: &ReviewReport,
    drafts: &[ReviewDecision],
) -> ReviewDecisionImport {
    let mut decisions: BTreeMap<&str, &ReviewDecision> = report
        .decisions
        .iter()
        .map(|d| (d.finding_ref.as_str(), d))
        .collect();
    for d in drafts {
        decisions.insert(&d.finding_ref, d);
    }
    ReviewDecisionImport {
        schema_version: IMPORT_SCHEMA,
        review_revision: report.revision.clone(),
        decision_revision: Some(report.decision_fingerprint.clone()),
        decisions: decisions.into_values().cloned().collect(),
    }
}
