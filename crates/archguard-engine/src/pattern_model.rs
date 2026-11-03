//! The assessment records every pattern check fills in.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub(super) const DETAIL_LIMIT: usize = 256;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternEvidence {
    pub kind: String,
    pub from: Option<String>,
    pub to: Option<String>,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub pointer: Option<String>,
    pub message: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternAssessment {
    pub id: String,
    pub pattern: String,
    pub scope: String,
    /// candidate/mixed/insufficient_evidence/not_observed; never a certification.
    pub status: String,
    pub basis: String,
    pub nodes: Vec<String>,
    pub nodes_total: usize,
    pub services: Vec<String>,
    pub services_total: usize,
    pub metrics: Value,
    pub evidence: Vec<PatternEvidence>,
    pub evidence_total: usize,
    pub counter_evidence: Vec<PatternEvidence>,
    pub counter_evidence_total: usize,
    pub missing_evidence: Vec<String>,
}
pub(super) fn assessment(
    pattern: &str,
    scope: &str,
    basis: &str,
    missing: &[&str],
) -> PatternAssessment {
    PatternAssessment {
        id: crate::evidence_import::hash(format!("{pattern}\0{scope}").as_bytes()),
        pattern: pattern.into(),
        scope: scope.into(),
        status: "insufficient_evidence".into(),
        basis: basis.into(),
        nodes: vec![],
        nodes_total: 0,
        services: vec![],
        services_total: 0,
        metrics: json!({}),
        evidence: vec![],
        evidence_total: 0,
        counter_evidence: vec![],
        counter_evidence_total: 0,
        missing_evidence: missing.iter().map(|s| (*s).into()).collect(),
    }
}
pub(super) fn evidence(
    kind: &str,
    from: Option<&str>,
    to: Option<&str>,
    file: Option<&str>,
    line: Option<u32>,
    message: &str,
) -> PatternEvidence {
    PatternEvidence {
        kind: kind.into(),
        from: from.map(str::to_owned),
        to: to.map(str::to_owned),
        file: file.map(str::to_owned),
        line,
        pointer: None,
        message: message.into(),
    }
}
pub(super) fn add(p: &mut PatternAssessment, item: PatternEvidence, counter: bool) {
    let (rows, total) = if counter {
        (&mut p.counter_evidence, &mut p.counter_evidence_total)
    } else {
        (&mut p.evidence, &mut p.evidence_total)
    };
    *total += 1;
    if rows.len() < DETAIL_LIMIT {
        rows.push(item);
    }
}
