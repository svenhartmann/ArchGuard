//! What View → Review decisions… shows of a review besides the decisions,
//! as the web viewer's assessment (`renderAssessment`): which findings a
//! search and status filter keep, and the report's impact, evidence gaps,
//! architecture questions and limits as plain lines. No window here.

use archguard_engine::{ReviewFinding, ReviewReport};
use serde_json::Value;

/// The status filter's entries, in the dialog's order.
pub const FILTERS: [&str; 4] = ["all", "open", "new", "accepted_risk"];

/// Whether finding `f` matches `query` (finding, nodes, owner, source;
/// case-insensitive) and filter `filter` (index into [`FILTERS`]).
pub fn keeps(f: &ReviewFinding, query: &str, filter: usize) -> bool {
    let q = query.trim().to_lowercase();
    let owner = f.decision.as_ref().map_or("", |d| d.author.as_str());
    let text = [f.title.as_str(), &f.category, &f.source_status, owner]
        .into_iter()
        .chain(f.nodes.iter().map(String::as_str))
        .chain(f.suggested_owners.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let status = match FILTERS.get(filter).copied().unwrap_or("all") {
        "open" => !matches!(f.disposition.as_str(), "accepted_risk" | "not_observed"),
        "new" => f.change == "new",
        "accepted_risk" => f.disposition == "accepted_risk",
        _ => true,
    };
    status && (q.is_empty() || text.contains(&q))
}

/// The indexes of the findings `query` and `filter` keep, in report order.
pub fn kept(r: &ReviewReport, query: &str, filter: usize) -> Vec<usize> {
    (0..r.findings.len())
        .filter(|&i| keeps(&r.findings[i], query, filter))
        .collect()
}

fn text(v: &Value) -> String {
    v.as_str().map_or_else(|| v.to_string(), str::to_string)
}

fn list(v: &Value) -> Vec<String> {
    v.as_array().into_iter().flatten().map(text).collect()
}

fn count(v: &Value) -> i32 {
    v.as_i64().unwrap_or(0) as i32
}

/// The report's sections below the findings.
#[derive(Debug, Default, PartialEq)]
pub struct Overview {
    /// Open, accepted risks, evidence gaps.
    pub cards: [i32; 3],
    pub components: Vec<String>,
    pub files: Vec<String>,
    /// `suite / name · outcome`.
    pub tests: Vec<String>,
    pub test_status: String,
    /// `(source · status, reason)`.
    pub gaps: Vec<(String, String)>,
    /// `source · status · producer revision`.
    pub origin: Vec<String>,
    pub questions: Vec<String>,
    /// `id · title · status · chosen option`.
    pub decisions: Vec<String>,
    pub unmatched: i32,
    pub limits: Vec<String>,
}

pub fn overview(r: &ReviewReport) -> Overview {
    let s = &r.summary;
    let tests = &r.impact["tests"];
    let line = |parts: &[&Value]| {
        parts
            .iter()
            .map(|v| text(v))
            .collect::<Vec<_>>()
            .join(" · ")
    };
    Overview {
        cards: [
            count(&s["open"]),
            count(&s["accepted_risk"]),
            count(&s["evidence_gaps"]),
        ],
        components: list(&r.impact["affected_components"]),
        files: list(&r.impact["affected_files"]),
        tests: tests["selected"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|t| {
                format!(
                    "{} / {} · {}",
                    text(&t["suite"]),
                    text(&t["name"]),
                    text(&t["outcome"])
                )
            })
            .collect(),
        test_status: tests["status"].as_str().unwrap_or("").into(),
        gaps: r
            .gaps
            .iter()
            .map(|g| (line(&[&g["source"], &g["status"]]), text(&g["reason"])))
            .collect(),
        origin: r
            .evidence
            .iter()
            .map(|e| line(&[&e["source"], &e["status"], &e["producer_revision"]]))
            .collect(),
        questions: list(&r.architecture["questions"]),
        decisions: r.architecture["intent"]["decisions"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|d| line(&[&d["id"], &d["title"], &d["status"], &d["chosen_option"]]))
            .collect(),
        unmatched: r.unmatched_decisions.len() as i32,
        limits: r.limitations.clone(),
    }
}

/// A finding's evidence as indented JSON.
pub fn evidence(f: &ReviewFinding) -> String {
    serde_json::to_string_pretty(&f.evidence).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::{DecisionKind, ReviewDecision};

    fn finding(title: &str, change: &str, disposition: &str) -> ReviewFinding {
        ReviewFinding {
            id: title.into(),
            fingerprint: String::new(),
            category: "cycle".into(),
            severity: "error".into(),
            title: title.into(),
            nodes: vec!["src/app".into()],
            change: change.into(),
            source_status: "observed".into(),
            evidence: serde_json::json!({"members": ["a.ts", "b.ts"]}),
            suggested_owners: vec!["@team-app".into()],
            disposition: disposition.into(),
            decision: None,
        }
    }

    #[test]
    fn search_and_status_filter_keep_the_findings_the_web_kept() {
        let mut risk = finding("Cycle billing", "existing", "accepted_risk");
        risk.decision = Some(ReviewDecision {
            author: "Dana".into(),
            decision: DecisionKind::AcceptRisk,
            ..Default::default()
        });
        let fresh = finding("Layer breach", "new", "open");
        let gone = finding("Old cycle", "existing", "not_observed");
        let filter = |name| FILTERS.iter().position(|f| *f == name).unwrap();
        let keep = |f: &ReviewFinding, q, by| keeps(f, q, filter(by));
        assert!([&risk, &fresh, &gone].iter().all(|f| keep(f, "", "all")));
        assert_eq!(
            (
                keep(&risk, "", "open"),
                keep(&fresh, "", "open"),
                keep(&gone, "", "open")
            ),
            (false, true, false)
        );
        assert_eq!(
            (keep(&risk, "", "new"), keep(&fresh, "", "new")),
            (false, true)
        );
        assert_eq!(
            (
                keep(&risk, "", "accepted_risk"),
                keep(&fresh, "", "accepted_risk")
            ),
            (true, false)
        );
        assert!(keep(&risk, "dana", "all"), "owner of the decision");
        assert!(keep(&fresh, "TEAM-APP", "all"), "suggested owner");
        assert!(keep(&fresh, "src/app", "all"), "source node");
        assert!(!keep(&fresh, "billing", "all"));
        assert!(evidence(&fresh).contains("\"members\""));
    }
}
