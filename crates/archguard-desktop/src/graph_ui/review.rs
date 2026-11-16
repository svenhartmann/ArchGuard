//! Explorer → Review and the review part of the details panel, as the web
//! viewer's review panel (`renderReviewPanel`, `renderReviewDetails`): the
//! visible rows ranked by review priority, Git evidence and orphan
//! candidates.

use archguard_engine::core::{GitStatus, ModuleReview, OrphanCandidate, ReviewPriority};
use slint::ModelRc;

use crate::model_sync::sync_rows;
use crate::state::GraphState;
use crate::ui::{ReasonItem, ReviewData, ReviewItem};

/// A reason of `analyze_review` as a known kind with its numbers, so the
/// window can translate it (the web viewer's `reviewReasonText`); unknown
/// reasons keep their text (kind 0).
fn reason(text: &str) -> ReasonItem {
    let nums: Vec<i32> = text
        .split(|c: char| !c.is_ascii_digit())
        .filter_map(|n| n.parse().ok())
        .collect();
    let n = |i: usize| nums.get(i).copied().unwrap_or(0);
    let kind = match text {
        t if t.starts_with("Cyclic group: ") && nums.len() == 2 => 1,
        t if t.starts_with("Direct coupling: ") && nums.len() == 3 => 2,
        t if t.starts_with("Transitive dependents: ") && nums.len() == 2 => 3,
        t if t.starts_with("Git change touches × coupling: ") && nums.len() == 3 => 4,
        t if t.starts_with("No incoming imports in this scope") => 5,
        _ => 0,
    };
    ReasonItem {
        kind,
        a: n(0),
        b: n(1),
        c: n(2),
        text: text.into(),
    }
}

fn priority(p: ReviewPriority) -> i32 {
    match p {
        ReviewPriority::High => 1,
        ReviewPriority::Medium => 2,
        ReviewPriority::Low => 3,
    }
}

fn orphan(o: Option<OrphanCandidate>) -> i32 {
    match o {
        None => 0,
        Some(OrphanCandidate::Isolated) => 1,
        Some(OrphanCandidate::Unreferenced) => 2,
    }
}

/// The rows Explorer → Review lists: ranked, orphan candidates only if
/// asked.
fn listed(g: &GraphState) -> Vec<&ModuleReview> {
    let mut rows: Vec<&ModuleReview> = g
        .review
        .iter()
        .flat_map(|r| &r.nodes)
        .filter(|m| !g.review_orphans || m.orphan.is_some())
        .collect();
    rows.sort_by_key(|m| m.rank);
    rows
}

/// What Graph `g` shows of its review, in the models of `old`.
pub fn data(g: &GraphState, old: &ReviewData) -> ReviewData {
    let Some(report) = &g.review else {
        return ReviewData::default();
    };
    let rows = listed(g);
    let label = |m: &ModuleReview| {
        let row = m.node_id.0 as usize;
        g.view
            .rows
            .get(row)
            .map_or(m.name.clone(), |r| g.tree.nodes[r.node].name.clone())
    };
    let items = rows.iter().take(g.review_limit).map(|m| ReviewItem {
        row: m.node_id.0 as i32,
        rank: m.rank as i32,
        name: label(m).into(),
        priority: priority(m.priority),
        score: m.score as i32,
        reason: m.reasons.first().map(|r| reason(r)).unwrap_or_default(),
        orphan: orphan(m.orphan),
    });
    let selected = g
        .selected_row()
        .and_then(|row| report.nodes.iter().find(|m| m.node_id.0 as usize == row));
    let git = &report.git;
    ReviewData {
        available: true,
        nodes: report.nodes.len() as i32,
        high: report.summary.high_priority as i32,
        git_available: git.status == GitStatus::Available,
        git_since: git.since.as_str().into(),
        git_commits: git.scanned_commits as i32,
        git_incomplete: git.shallow || git.truncated,
        // The window's sentence ends it with its own period.
        git_message: git
            .message
            .as_deref()
            .unwrap_or("")
            .trim_end_matches('.')
            .into(),
        orphans_only: g.review_orphans,
        items: sync_rows(&old.items, items),
        more: rows.len().saturating_sub(g.review_limit) as i32,
        has_selected: selected.is_some(),
        selected_priority: selected.map_or(0, |m| priority(m.priority)),
        selected_score: selected.map_or(0, |m| m.score as i32),
        selected_orphan: selected.map_or(0, |m| orphan(m.orphan)),
        selected_entry: selected.is_some_and(|m| m.entry_point),
        selected_reasons: selected.map_or(ModelRc::default(), |m| {
            sync_rows(&old.selected_reasons, m.reasons.iter().map(|r| reason(r)))
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The structural reasons `analyze_review` writes are recognised with
    /// their numbers; anything else keeps its text.
    #[test]
    fn reasons_are_recognised_for_translation() {
        let r = reason("Cyclic group: 2 nodes (+32)");
        assert_eq!((r.kind, r.a, r.b), (1, 2, 32));
        let r = reason("Direct coupling: 3 incoming + 4 outgoing (+14)");
        assert_eq!((r.kind, r.a, r.b, r.c), (2, 3, 4, 14));
        let r = reason("Transitive dependents: 5 (+5)");
        assert_eq!((r.kind, r.a, r.b), (3, 5, 5));
        let r = reason("Git change touches × coupling: 7 × 2 (+9)");
        assert_eq!((r.kind, r.a, r.b, r.c), (4, 7, 2, 9));
        let r = reason(
            "No incoming imports in this scope; check entry points and dynamic consumers before removing code.",
        );
        assert_eq!(r.kind, 5);
        let r = reason("Something new (+1)");
        assert_eq!((r.kind, r.text.as_str()), (0, "Something new (+1)"));
    }
}
