//! Review-process metrics (R20).
//!
//! Answers "how is the review being worked off", not "what is wrong":
//! age of the oldest open finding, resolution rate over the observed
//! period, and ADR coverage of accepted risks from the decision register
//! (R19). The evidence base is the snapshot history under
//! `.archguard/history/` (same files [`crate::snapshot::read_trend`] reads, but
//! the full [`Snapshot`]s — only they carry `cycle_hashes`, the stable
//! identity that links a register entry to its first appearance) plus
//! the register itself. Unknown values stay `None` with a reason — an
//! empty history never yields an invented zero.
//!
//! The register stores no severity, so every open decision (kind `fix`)
//! counts toward the age metric; `finding_ref` must equal a snapshot's
//! cycle hash for the age to be measurable. Ages are measured against
//! the newest snapshot ("as of the last recorded run"), which keeps the
//! metric deterministic and free of wall-clock reads.

use std::collections::BTreeSet;
use std::path::Path;

use archguard_core::{DecisionKind, ReviewRegister};
use serde::{Deserialize, Serialize};

use crate::snapshot::Snapshot;

const SECONDS_PER_DAY: f64 = 86_400.0;

/// Aggregate review-workflow metrics.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReviewMetrics {
    /// Register entries decided `fix` — a planned fix keeps the finding
    /// open until the finding is gone (R02 semantics).
    pub open_findings: usize,
    /// Age in days of the oldest open finding, measured from its first
    /// appearance in the snapshot history to the newest snapshot;
    /// `None` when no open finding can be tied to the history.
    pub oldest_open_p1_age_days: Option<f64>,
    /// Findings resolved per day between the oldest and the newest
    /// snapshot; `None` when fewer than two usable snapshots exist or
    /// the period is zero.
    pub resolution_rate_per_day: Option<f64>,
    /// `accept-risk` decisions carrying an ADR link.
    pub accepted_with_adr: usize,
    /// `accept-risk` decisions without an ADR link.
    pub accepted_without_adr: usize,
    /// Human-readable reasons for every `None` above.
    pub reasons: Vec<String>,
}

/// Load the full snapshots oldest-first — the same tolerance as
/// [`crate::snapshot::read_trend`]: an unparseable history file is skipped.
fn load_snapshots(root: &Path) -> Vec<Snapshot> {
    let dir = root.join(".archguard").join("history");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut snapshots: Vec<Snapshot> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                return None;
            }
            let text = std::fs::read_to_string(&path).ok()?;
            serde_json::from_str::<Snapshot>(&text).ok()
        })
        .collect();
    snapshots
        .sort_by(|a, b| (a.timestamp, a.git_sha.as_str()).cmp(&(b.timestamp, b.git_sha.as_str())));
    snapshots
}

/// Compute the review-workflow metrics from the snapshot history and
/// the decision register.
pub fn review_metrics(root: &Path, register: &ReviewRegister) -> ReviewMetrics {
    let mut metrics = ReviewMetrics::default();
    for decision in &register.decisions {
        match decision.decision {
            DecisionKind::Fix => metrics.open_findings += 1,
            DecisionKind::AcceptRisk => {
                if decision.adr_link.as_deref().is_none_or(str::is_empty) {
                    metrics.accepted_without_adr += 1;
                } else {
                    metrics.accepted_with_adr += 1;
                }
            }
            DecisionKind::Defer => {}
        }
    }

    let snapshots = load_snapshots(root);
    let (Some(first), Some(last)) = (snapshots.first(), snapshots.last()) else {
        metrics.reasons.push(
            "no snapshot history below .archguard/history: ages and rates unknown".to_string(),
        );
        return metrics;
    };

    // Age of the oldest open finding: first snapshot carrying its hash,
    // measured to the newest snapshot.
    let mut oldest_age_secs: Option<u64> = None;
    let mut untracked = 0usize;
    for decision in register
        .decisions
        .iter()
        .filter(|d| d.decision == DecisionKind::Fix)
    {
        let first_seen = snapshots
            .iter()
            .find(|snapshot| snapshot.cycle_hashes.contains(&decision.finding_ref));
        match first_seen {
            Some(snapshot) if snapshot.timestamp <= last.timestamp => {
                let age = last.timestamp - snapshot.timestamp;
                oldest_age_secs = Some(oldest_age_secs.map_or(age, |current| current.max(age)));
            }
            _ => untracked += 1,
        }
    }
    if let Some(age) = oldest_age_secs {
        metrics.oldest_open_p1_age_days = Some(age as f64 / SECONDS_PER_DAY);
    } else if metrics.open_findings > 0 {
        metrics.reasons.push(format!(
            "{untracked} open finding(s) never appeared in the snapshot history: age unknown"
        ));
    }

    // Resolution rate: hashes present in the oldest snapshot but gone
    // in the newest, per day of the observed period.
    if snapshots.len() < 2 {
        metrics.reasons.push(
            "fewer than two snapshots: resolution rate needs a period to divide by".to_string(),
        );
    } else {
        let period_days = (last.timestamp - first.timestamp) as f64 / SECONDS_PER_DAY;
        if period_days <= 0.0 {
            metrics
                .reasons
                .push("oldest and newest snapshot share a timestamp: rate unknown".to_string());
        } else {
            let latest: BTreeSet<&str> = last.cycle_hashes.iter().map(String::as_str).collect();
            let resolved = first
                .cycle_hashes
                .iter()
                .filter(|hash| !latest.contains(hash.as_str()))
                .count();
            metrics.resolution_rate_per_day = Some(resolved as f64 / period_days);
        }
    }
    metrics
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{DecisionKind, ReviewDecision};

    fn snapshot(timestamp: u64, hashes: &[&str]) -> Snapshot {
        Snapshot {
            analysis_scope: None,
            schema_version: 1,
            timestamp,
            git_sha: "abc1234".to_string(),
            tag: None,
            nodes: 1,
            edges: 1,
            sccs: hashes.len() as u32,
            largest_scc: 2,
            acd: 1.0,
            nccd: 0.0,
            cycle_hashes: hashes.iter().map(|h| h.to_string()).collect(),
            review_kpis: None,
        }
    }

    fn write_history(root: &Path, snapshots: &[Snapshot]) {
        let dir = root.join(".archguard").join("history");
        std::fs::create_dir_all(&dir).expect("history dir");
        for (index, snapshot) in snapshots.iter().enumerate() {
            let text = serde_json::to_string(snapshot).expect("serialize");
            std::fs::write(
                dir.join(format!("{}-run{index}.json", snapshot.timestamp)),
                text,
            )
            .expect("write snapshot");
        }
    }

    fn decision(id: &str, finding: &str, kind: DecisionKind, adr: Option<&str>) -> ReviewDecision {
        ReviewDecision {
            id: id.to_string(),
            finding_ref: finding.to_string(),
            decision: kind,
            adr_link: adr.map(str::to_string),
            note: String::new(),
            author: "sven".to_string(),
            ts: "2026-10-30T09:00:00Z".to_string(),
        }
    }

    const DAY: u64 = 86_400;

    #[test]
    fn empty_history_yields_null_with_reason_never_zero() {
        let dir = tempfile::tempdir().expect("tempdir");
        let register = ReviewRegister {
            version: 1,
            decisions: vec![decision("d-1", "hash-a", DecisionKind::Fix, None)],
        };
        let metrics = review_metrics(dir.path(), &register);
        assert_eq!(metrics.oldest_open_p1_age_days, None);
        assert_eq!(metrics.resolution_rate_per_day, None);
        assert_eq!(metrics.open_findings, 1);
        assert!(
            metrics
                .reasons
                .iter()
                .any(|r| r.contains("no snapshot history")),
            "{:?}",
            metrics.reasons
        );
    }

    #[test]
    fn age_measures_first_appearance_to_newest_snapshot() {
        let dir = tempfile::tempdir().expect("tempdir");
        // hash-a first seen 30 days before the newest run; hash-b only
        // in the newest run (age 0, measured — not unknown).
        write_history(
            dir.path(),
            &[
                snapshot(1_000_000, &["hash-a"]),
                snapshot(1_000_000 + 30 * DAY, &["hash-a", "hash-b"]),
            ],
        );
        let register = ReviewRegister {
            version: 1,
            decisions: vec![
                decision("d-1", "hash-a", DecisionKind::Fix, None),
                decision("d-2", "hash-b", DecisionKind::Fix, None),
            ],
        };
        let metrics = review_metrics(dir.path(), &register);
        assert_eq!(metrics.oldest_open_p1_age_days, Some(30.0));
    }

    #[test]
    fn resolution_rate_is_resolved_per_day() {
        let dir = tempfile::tempdir().expect("tempdir");
        // hash-a resolved, hash-b kept, hash-c newly appeared.
        write_history(
            dir.path(),
            &[
                snapshot(1_000_000, &["hash-a", "hash-b"]),
                snapshot(1_000_000 + 10 * DAY, &["hash-b", "hash-c"]),
            ],
        );
        let metrics = review_metrics(dir.path(), &ReviewRegister::default());
        assert_eq!(metrics.resolution_rate_per_day, Some(0.1));
    }

    #[test]
    fn single_snapshot_or_zero_period_stays_unknown() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_history(dir.path(), &[snapshot(1_000_000, &["hash-a"])]);
        let metrics = review_metrics(dir.path(), &ReviewRegister::default());
        assert_eq!(metrics.resolution_rate_per_day, None);
        assert!(
            metrics.reasons.iter().any(|r| r.contains("fewer than two")),
            "{:?}",
            metrics.reasons
        );
    }

    #[test]
    fn untracked_open_finding_never_invents_an_age() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_history(dir.path(), &[snapshot(1_000_000, &["hash-a"])]);
        let register = ReviewRegister {
            version: 1,
            decisions: vec![decision("d-9", "scc-7", DecisionKind::Fix, None)],
        };
        let metrics = review_metrics(dir.path(), &register);
        assert_eq!(
            metrics.oldest_open_p1_age_days, None,
            "no history evidence for scc-7"
        );
        assert!(
            metrics.reasons.iter().any(|r| r.contains("1 open finding")),
            "{:?}",
            metrics.reasons
        );
    }

    #[test]
    fn adr_coverage_splits_accept_risk_entries() {
        let dir = tempfile::tempdir().expect("tempdir");
        let register = ReviewRegister {
            version: 1,
            decisions: vec![
                decision(
                    "d-1",
                    "hash-a",
                    DecisionKind::AcceptRisk,
                    Some("docs/adr/1.md"),
                ),
                decision("d-2", "hash-b", DecisionKind::AcceptRisk, None),
                decision("d-3", "hash-c", DecisionKind::AcceptRisk, Some("")),
                decision("d-4", "hash-d", DecisionKind::Defer, None),
            ],
        };
        let metrics = review_metrics(dir.path(), &register);
        assert_eq!(metrics.accepted_with_adr, 1);
        assert_eq!(metrics.accepted_without_adr, 2, "None and empty both count");
        assert_eq!(metrics.open_findings, 0);
    }

    #[test]
    fn corrupt_history_file_is_skipped_like_the_trend() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_history(dir.path(), &[snapshot(1_000_000, &["hash-a"])]);
        std::fs::write(
            dir.path().join(".archguard/history/broken.json"),
            "{ not json",
        )
        .expect("write corrupt");
        let metrics = review_metrics(dir.path(), &ReviewRegister::default());
        // The good snapshot still defines the history; the corrupt one
        // is skipped, not fatal.
        assert_eq!(metrics.resolution_rate_per_day, None);
        assert!(metrics.reasons.iter().any(|r| r.contains("fewer than two")));
    }

    #[test]
    fn signature_is_stable() {
        let _: fn(&Path, &ReviewRegister) -> ReviewMetrics = review_metrics;
    }
}
