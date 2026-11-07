//! Per-file test coverage in the Test gaps details: the port of
//! `renderTestCoverageDetails` with `coverageFileText` and
//! `coverageReviewText` (viewer_quality_metrics.js). Coverage belongs to
//! files; the review index is the engine's `test_coverage_review`.

use std::collections::HashMap;

use archguard_engine::{CoverageCounter, CoverageReview, TestCoverageReport};

use super::lines::{kind, line};
use crate::format::{Locale, UNKNOWN};
use crate::ui::StartLine;

/// One imported file measurement with its review row.
#[derive(Debug, Clone, PartialEq)]
pub struct File {
    pub path: String,
    pub lines: Option<CoverageCounter>,
    pub branches: Option<CoverageCounter>,
    /// `unchanged_since_import`, `changed_since_import` or unknown.
    pub status: String,
    pub dependents: Option<u32>,
    pub score: Option<f64>,
}

/// The imported coverage by source node.
#[derive(Debug, Clone, Default)]
pub struct Files {
    /// The import's status (`TestCoverageReport::status`).
    pub status: String,
    pub by_node: HashMap<String, File>,
}

impl Files {
    pub fn new(coverage: &TestCoverageReport, review: &CoverageReview) -> Self {
        let rows: HashMap<&str, _> = review.files.iter().map(|r| (r.node.as_str(), r)).collect();
        let by_node = coverage
            .files
            .iter()
            .map(|f| {
                let row = rows.get(f.node.as_str());
                let file = File {
                    path: f.path.clone(),
                    lines: f.lines.clone(),
                    branches: f.branches.clone(),
                    status: f.status.clone(),
                    dependents: row.and_then(|r| r.dependents),
                    score: row.and_then(|r| r.score),
                };
                (f.node.clone(), file)
            })
            .collect();
        Files {
            status: coverage.status.clone(),
            by_node,
        }
    }

    /// `coverageReviewScore`: only a fresh file of a usable import has one.
    fn score(&self, f: &File) -> Option<f64> {
        let usable = matches!(self.status.as_str(), "ready" | "partial" | "stale");
        let fresh = f.status == "unchanged_since_import";
        let measured = counter(Locale::En, f.lines.as_ref()) != UNKNOWN;
        f.score
            .filter(|s| usable && fresh && measured && s.is_finite())
    }
}

/// `coverageCounterText`: `75.0% (3 / 4)`, `—` without a valid count.
fn counter(l: Locale, c: Option<&CoverageCounter>) -> String {
    match c {
        Some(c) if c.total > 0 && c.covered <= c.total => {
            let (covered, total) = (c.covered as f64, c.total as f64);
            let pct = l.pct(100.0 * covered / total, 1);
            format!("{pct} ({})", l.ratio(c.covered as i64, c.total as i64))
        }
        _ => UNKNOWN.into(),
    }
}

/// The coverage section of one source file (`node`).
pub fn lines(out: &mut Vec<StartLine>, files: &Files, node: &str, l: Locale) {
    out.push(line(kind::COV_HEAD, "", "", ""));
    match files.by_node.get(node) {
        Some(f) => {
            let fresh = match f.status.as_str() {
                "unchanged_since_import" => "",
                "changed_since_import" => "changed",
                _ => "unknown",
            };
            let (lines, branches) = (
                counter(l, f.lines.as_ref()),
                counter(l, f.branches.as_ref()),
            );
            out.push(line(kind::COV_FILE, lines, branches, fresh));
            let score = files.score(f).map_or(UNKNOWN.into(), |s| l.dec(s, 2));
            let dependents = f.dependents.map_or(UNKNOWN.into(), |d| l.int(d.into()));
            out.push(line(kind::COV_REVIEW, score, dependents, ""));
            out.push(line(kind::PATH, f.path.as_str(), "", ""));
        }
        None => out.push(line(kind::COV_NONE, "", "", "")),
    }
    out.push(line(kind::COV_SOURCE, "", "", ""));
    if files.status == "error" {
        out.push(line(kind::COV_ERROR, "", "", ""));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(import: &str, status: &str, score: Option<f64>) -> Files {
        let file = File {
            path: "src/a.rs".into(),
            lines: Some(CoverageCounter {
                covered: 3,
                total: 4,
            }),
            branches: None,
            status: status.into(),
            dependents: Some(2),
            score,
        };
        Files {
            status: import.into(),
            by_node: HashMap::from([("a.rs".to_string(), file)]),
        }
    }

    fn texts(out: &[StartLine]) -> Vec<(i32, String, String, String)> {
        out.iter()
            .map(|l| (l.kind, l.a.to_string(), l.b.to_string(), l.c.to_string()))
            .collect()
    }

    /// A measured file shows its lines, branches, review index and path.
    #[test]
    fn a_measured_file_shows_its_coverage_and_review_index() {
        let mut out = Vec::new();
        lines(
            &mut out,
            &files("ready", "unchanged_since_import", Some(0.5)),
            "a.rs",
            Locale::En,
        );
        let s = |v: &str| v.to_string();
        assert_eq!(
            texts(&out),
            [
                (kind::COV_HEAD, s(""), s(""), s("")),
                (kind::COV_FILE, s("75.0% (3 / 4)"), s("—"), s("")),
                (kind::COV_REVIEW, s("0.50"), s("2"), s("")),
                (kind::PATH, s("src/a.rs"), s(""), s("")),
                (kind::COV_SOURCE, s(""), s(""), s("")),
            ]
        );
    }

    /// A changed source has no review index; an unmeasured file says so;
    /// a failed import is named.
    #[test]
    fn stale_unknown_and_failed_imports_stay_unknown() {
        let mut out = Vec::new();
        lines(
            &mut out,
            &files("ready", "changed_since_import", Some(0.5)),
            "a.rs",
            Locale::En,
        );
        assert_eq!(out[1].c, "changed");
        assert_eq!(out[2].a, "—");
        let mut out = Vec::new();
        lines(&mut out, &files("error", "", None), "b.rs", Locale::En);
        let kinds: Vec<i32> = out.iter().map(|l| l.kind).collect();
        assert_eq!(
            kinds,
            [
                kind::COV_HEAD,
                kind::COV_NONE,
                kind::COV_SOURCE,
                kind::COV_ERROR
            ]
        );
    }
}
