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
    /// `format · report path` of the import, when known.
    pub report: Option<String>,
}

/// The key of the review entry for `toggle-section`.
pub const REVIEW: &str = "coverage-review";

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
        let report = coverage.report_path.as_ref().map(|p| {
            let format = coverage.format.as_deref().unwrap_or("");
            format!("{format} · {p}")
        });
        Files {
            status: coverage.status.clone(),
            by_node,
            report,
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
            measured(out, files, f, l, false);
            out.push(line(kind::PATH, f.path.as_str(), "", ""));
        }
        None => out.push(line(kind::COV_NONE, "", "", "")),
    }
    out.push(line(kind::COV_SOURCE, "", "", ""));
    if files.status == "error" {
        out.push(line(kind::COV_ERROR, "", "", ""));
    }
}

/// "Coverage and impact review" (the web's `openCoverageReview`): an
/// entry that opens the imported files ranked by review index, highest
/// first; `open` the number of rows shown, `None` closed.
pub fn review(out: &mut Vec<StartLine>, files: &Files, open: Option<usize>, l: Locale) {
    out.push(StartLine {
        open: open.is_some(),
        ..line(kind::COV_REVIEW_OPEN, "", "", REVIEW)
    });
    let Some(limit) = open else { return };
    let mut rows: Vec<(&String, &File)> = files.by_node.iter().collect();
    let score = |f: &File| files.score(f).unwrap_or(-1.0);
    rows.sort_by(|a, b| score(b.1).total_cmp(&score(a.1)).then_with(|| a.0.cmp(b.0)));
    let status = if files.status.is_empty() {
        "not_imported"
    } else {
        files.status.as_str()
    };
    let shown = l.count(limit.min(rows.len()));
    out.push(line(kind::COV_STATUS, status, shown, l.count(rows.len())));
    out.push(line(kind::COV_DEFINITION, "", "", ""));
    out.push(line(kind::COV_UNVERIFIED, "", "", ""));
    if let Some(r) = &files.report {
        out.push(line(kind::PATH, r.as_str(), "", ""));
    }
    out.push(line(kind::COV_IMPORT, "", "", ""));
    let command =
        "archguard coverage . --report path/to/report.xml\n# optional: --source-root src/main/java";
    out.push(line(kind::PATH, command, "", ""));
    for (node, f) in rows.iter().take(limit) {
        out.push(line(kind::COV_ROW, node.as_str(), "", ""));
        measured(out, files, f, l, true);
    }
    if rows.is_empty() {
        out.push(line(kind::COV_EMPTY, status, "", ""));
    }
    if rows.len() > limit {
        out.push(line(kind::MORE, l.count(rows.len() - limit), "", ""));
    }
}

/// `coverageFileText` and `coverageReviewText` of one file; `nested`
/// indents them under a review row.
fn measured(out: &mut Vec<StartLine>, files: &Files, f: &File, l: Locale, nested: bool) {
    let fresh = match f.status.as_str() {
        "unchanged_since_import" => "",
        "changed_since_import" => "changed",
        _ => "unknown",
    };
    let (lines, branches) = (
        counter(l, f.lines.as_ref()),
        counter(l, f.branches.as_ref()),
    );
    let score = files.score(f).map_or(UNKNOWN.into(), |s| l.dec(s, 2));
    let dependents = f.dependents.map_or(UNKNOWN.into(), |d| l.int(d.into()));
    for row in [
        line(kind::COV_FILE, lines, branches, fresh),
        line(kind::COV_REVIEW, score, dependents, ""),
    ] {
        out.push(StartLine {
            flag: nested,
            ..row
        });
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
            report: None,
        }
    }

    fn kinds(out: &[StartLine]) -> Vec<i32> {
        out.iter().map(|l| l.kind).collect()
    }

    /// Closed, the review is its entry only.
    #[test]
    fn the_review_starts_as_a_closed_entry() {
        let mut out = Vec::new();
        review(
            &mut out,
            &files("ready", "unchanged_since_import", Some(0.5)),
            None,
            Locale::En,
        );
        assert_eq!(kinds(&out), [kind::COV_REVIEW_OPEN]);
        assert_eq!((out[0].c.as_str(), out[0].open), (REVIEW, false));
    }

    /// Open, it ranks the files by review index, highest first; an
    /// unknown index sorts last; each row opens its file.
    #[test]
    fn the_open_review_ranks_files_by_review_index() {
        let mut f = files("ready", "unchanged_since_import", Some(0.5));
        let mut b = f.by_node["a.rs"].clone();
        b.score = Some(0.9);
        f.by_node.insert("b.rs".into(), b.clone());
        b.score = None;
        f.by_node.insert("c.rs".into(), b);
        f.report = Some("lcov · lcov.info".into());
        let mut out = Vec::new();
        review(&mut out, &f, Some(2), Locale::En);
        assert_eq!(
            kinds(&out)[..7],
            [
                kind::COV_REVIEW_OPEN,
                kind::COV_STATUS,
                kind::COV_DEFINITION,
                kind::COV_UNVERIFIED,
                kind::PATH,
                kind::COV_IMPORT,
                kind::PATH,
            ]
        );
        assert!(out[0].open);
        assert_eq!(
            (out[1].a.as_str(), out[1].b.as_str(), out[1].c.as_str()),
            ("ready", "2", "3")
        );
        assert_eq!(out[4].a, "lcov · lcov.info");
        let rows: Vec<&str> = out
            .iter()
            .filter(|l| l.kind == kind::COV_ROW)
            .map(|l| l.a.as_str())
            .collect();
        assert_eq!(rows, ["b.rs", "a.rs"]);
        let first = out.iter().position(|l| l.kind == kind::COV_ROW).unwrap();
        assert_eq!(out[first + 2].a, "0.90");
        assert!(out[first + 1].flag && out[first + 2].flag);
        assert_eq!(
            (out.last().unwrap().kind, out.last().unwrap().a.as_str()),
            (kind::MORE, "1")
        );
    }

    /// Without an import the review says coverage is unknown.
    #[test]
    fn without_an_import_the_review_has_no_rows() {
        let f = Files {
            status: "not_imported".into(),
            ..Default::default()
        };
        let mut out = Vec::new();
        review(&mut out, &f, Some(20), Locale::En);
        let last = out.last().unwrap();
        assert_eq!(
            (last.kind, last.a.as_str()),
            (kind::COV_EMPTY, "not_imported")
        );
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
