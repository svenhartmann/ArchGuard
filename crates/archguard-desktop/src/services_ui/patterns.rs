//! Services → architecture pattern candidates: the web viewer's
//! `renderPatternAssessments` and `focusPattern` (`viewer_patterns.js`)
//! over `archguard_engine::analyze_architecture_patterns`, the engine
//! behind `archguard patterns` and MCP `architecture_patterns`.

use std::cmp::Reverse;

use archguard_engine::{ArchitecturePatternReport, PatternAssessment, PatternEvidence};

use crate::tab_support::model;
use crate::ui::{SvcPattern, SvcPatternDetail, SvcPatterns, SvcRecord};

/// Assessments and evidence records per page, as the web's.
pub const PAGE: usize = 20;
/// The web's `patternText` cut.
const TEXT_LIMIT: usize = 4096;

const LABELS: [&str; 8] = [
    "layered",
    "dependency_layers",
    "dependency_inversion",
    "pipeline",
    "modular_monolith",
    "ports_and_adapters",
    "microservices",
    "event_driven",
];
const STATUSES: [&str; 4] = [
    "candidate",
    "mixed",
    "insufficient_evidence",
    "not_observed",
];
/// Structural observations, not architecture styles.
const STRUCTURAL: [&str; 3] = ["dependency_layers", "dependency_inversion", "pipeline"];

/// Source files and `(from, to)` file pairs to mark in the Graph.
pub type Evidence = (Vec<String>, Vec<(String, String)>);

/// The section's state; `Default` while the report is not computed.
#[derive(Default)]
pub struct Patterns {
    report: Option<ArchitecturePatternReport>,
    /// Assessment indices in the web's order.
    order: Vec<usize>,
    /// The open assessment (position in `order`).
    open: Option<usize>,
    limit: usize,
    evidence: usize,
    /// Its evidence is marked in the Graph.
    pub highlighted: bool,
}

fn found(p: &PatternAssessment) -> bool {
    matches!(p.status.as_str(), "candidate" | "mixed")
}

/// The web's order: candidates (structural observations after the styles),
/// then the rest; larger scopes first, then by name.
fn order(report: &ArchitecturePatternReport) -> Vec<usize> {
    let priority = |p: &PatternAssessment| match p.pattern.as_str() {
        _ if !found(p) => 4,
        "dependency_layers" => 1,
        "pipeline" => 2,
        "dependency_inversion" => 3,
        _ => 0,
    };
    let files = |p: &PatternAssessment| p.metrics["source_files"].as_u64().unwrap_or(0);
    let mut order: Vec<usize> = (0..report.assessments.len()).collect();
    order.sort_by_key(|&i| {
        let p = &report.assessments[i];
        (
            priority(p),
            Reverse(files(p)),
            format!("{} {}", p.pattern, p.scope),
        )
    });
    order
}

fn cut(text: &str) -> String {
    match text.char_indices().nth(TEXT_LIMIT) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}

fn record(e: &PatternEvidence) -> SvcRecord {
    let ends: Vec<&str> = [&e.from, &e.to]
        .into_iter()
        .flatten()
        .map(String::as_str)
        .collect();
    let mut place = e.file.clone().unwrap_or_default();
    if let Some(line) = e.line {
        place += &format!(":{line}");
    }
    if let Some(pointer) = &e.pointer {
        place += &format!("#{pointer}");
    }
    SvcRecord {
        title: ends.join(" → ").into(),
        place: place.into(),
        message: cut(&e.message).into(),
    }
}

fn index_of(list: &[&str], value: &str) -> i32 {
    list.iter()
        .position(|&v| v == value)
        .map_or(-1, |i| i as i32)
}

impl Patterns {
    pub fn new(report: ArchitecturePatternReport) -> Self {
        Patterns {
            order: order(&report),
            report: Some(report),
            limit: PAGE,
            evidence: PAGE,
            ..Default::default()
        }
    }

    /// Open listed assessment `i`, or close it when it is open.
    pub fn choose(&mut self, i: usize) {
        self.open = (self.open != Some(i) && i < self.order.len()).then_some(i);
        self.evidence = PAGE;
    }

    pub fn more(&mut self) {
        self.limit += PAGE;
    }

    pub fn more_evidence(&mut self) {
        self.evidence += PAGE;
    }

    fn chosen(&self) -> Option<&PatternAssessment> {
        let i = *self.order.get(self.open?)?;
        self.report.as_ref().map(|r| &r.assessments[i])
    }

    /// The open assessment's retained source files and the file pairs of
    /// its evidence and counter-evidence, to mark in the Graph.
    pub fn highlight(&self) -> Option<Evidence> {
        let p = self.chosen()?;
        let pairs = p
            .evidence
            .iter()
            .chain(&p.counter_evidence)
            .filter_map(|e| Some((e.from.clone()?, e.to.clone()?)))
            .collect();
        Some((p.nodes.clone(), pairs))
    }

    pub fn model(&self) -> SvcPatterns {
        let Some(report) = &self.report else {
            return SvcPatterns::default();
        };
        let all = || report.assessments.iter();
        let candidates = all().filter(|p| found(p)).count();
        let structures = all()
            .filter(|p| found(p) && STRUCTURAL.contains(&p.pattern.as_str()))
            .count();
        let rows = self
            .order
            .iter()
            .take(self.limit)
            .enumerate()
            .map(|(n, &i)| {
                let p = &report.assessments[i];
                SvcPattern {
                    kind: index_of(&LABELS, &p.pattern),
                    name: p.pattern.as_str().into(),
                    status: index_of(&STATUSES, &p.status),
                    status_name: p.status.as_str().into(),
                    scope: p.scope.as_str().into(),
                    open: self.open == Some(n),
                }
            });
        SvcPatterns {
            state: 1,
            candidates: (candidates - structures) as i32,
            structures: structures as i32,
            semantic: match report.coverage["semantic_status"].as_str() {
                Some("ready") => 0,
                Some("partial") => 1,
                Some("not_applicable") => 2,
                _ => 3,
            },
            coverage: serde_json::to_string_pretty(&report.coverage)
                .unwrap_or_default()
                .into(),
            notes: model(
                report
                    .limitations
                    .iter()
                    .chain(&report.diagnostics)
                    .map(|n| cut(n).into()),
            ),
            total: report.assessments_total.max(report.assessments.len()) as i32,
            rows: model(rows.collect::<Vec<_>>()),
            more: self.order.len() > self.limit,
            detail: self.chosen().map(|p| self.detail(p)).unwrap_or_default(),
            highlighted: self.highlighted,
            // The caller knows whether the Graph is simulated.
            simulated: false,
        }
    }

    fn detail(&self, p: &PatternAssessment) -> SvcPatternDetail {
        let page = |list: &[PatternEvidence]| model(list.iter().take(self.evidence).map(record));
        SvcPatternDetail {
            basis: p.basis.as_str().into(),
            metrics: serde_json::to_string_pretty(&p.metrics)
                .unwrap_or_default()
                .into(),
            limits: model(p.missing_evidence.iter().map(|m| cut(m).into())),
            nodes: p.nodes.len() as i32,
            nodes_total: p.nodes_total.max(p.nodes.len()) as i32,
            services: p.services.join(", ").into(),
            evidence: page(&p.evidence),
            evidence_retained: p.evidence.len() as i32,
            evidence_total: p.evidence_total.max(p.evidence.len()) as i32,
            counter: page(&p.counter_evidence),
            counter_retained: p.counter_evidence.len() as i32,
            counter_total: p.counter_evidence_total.max(p.counter_evidence.len()) as i32,
            more_evidence: p.evidence.len().max(p.counter_evidence.len()) > self.evidence,
        }
    }
}
