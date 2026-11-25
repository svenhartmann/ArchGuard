//! Elasticsearch definition quality: the one rule table, the result types and
//! the `[search_quality]` configuration (docs: .scratch/es-quality/spec.md).

use serde::{Deserialize, Serialize};

use crate::model::Severity;
use crate::quality_goal::QualityAttribute;
use crate::services::ServiceEvidence;

/// Whether a rule produces findings (with a severity) or only a KPI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleKind {
    Finding,
    Kpi,
}

/// One quality rule. Findings carry a default severity and an ISO 25010
/// attribute; KPI-only rules carry neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SearchRule {
    pub id: &'static str,
    pub kind: RuleKind,
    pub severity: Option<Severity>,
    pub attribute: Option<QualityAttribute>,
    /// mapping, analysis, drift, lifecycle, routing, coupling, declared or existing.
    pub group: &'static str,
}

const fn finding(
    id: &'static str,
    severity: Severity,
    attribute: QualityAttribute,
    group: &'static str,
) -> SearchRule {
    SearchRule {
        id,
        kind: RuleKind::Finding,
        severity: Some(severity),
        attribute: Some(attribute),
        group,
    }
}

const fn kpi(id: &'static str, group: &'static str) -> SearchRule {
    SearchRule {
        id,
        kind: RuleKind::Kpi,
        severity: None,
        attribute: None,
        group,
    }
}

use QualityAttribute::{Maintainability as M, PerformanceEfficiency as P, Reliability as R};
use Severity::{Info, Warning};

/// Every rule, in spec order. Search emits only these ids; the engine reads
/// severity, `errors` and `disabled` from here, goals read the attribute.
pub const SEARCH_RULES: &[SearchRule] = &[
    finding("total_fields_limit", Warning, R, "mapping"),
    finding("depth_limit", Warning, R, "mapping"),
    finding("nested_fields_limit", Warning, R, "mapping"),
    finding("source_disabled", Warning, M, "declared"),
    kpi("dynamic_mode", "declared"),
    kpi("text_without_keyword", "mapping"),
    finding("analysis_drift", Info, M, "analysis"),
    finding("unused_analysis", Info, M, "analysis"),
    finding("environment_drift", Warning, M, "drift"),
    finding("zero_replicas", Warning, R, "lifecycle"),
    finding("refresh_interval_low", Warning, P, "lifecycle"),
    finding("time_series_without_ilm", Info, R, "declared"),
    finding("deletion_protection_off", Warning, R, "declared"),
    finding("write_bypasses_alias", Info, M, "routing"),
    finding("alias_without_write_index", Warning, R, "declared"),
    finding("template_covers_nothing", Info, M, "routing"),
    finding("template_priority_conflict", Warning, R, "declared"),
    finding("unused_pipeline", Info, M, "routing"),
    finding("undeclared_index", Info, M, "existing"),
    finding("uncovered_pattern", Info, M, "existing"),
    finding("alert_undeclared_index", Info, M, "existing"),
    finding("undeclared_pipeline", Warning, R, "existing"),
    finding("shared_write_index", Warning, M, "existing"),
    kpi("reader_fan_in", "coupling"),
    kpi("cross_project_access", "coupling"),
    kpi("readable_share", "coupling"),
    kpi("painless_complexity", "coupling"),
];

/// The rule with this id.
pub fn search_rule(id: &str) -> Option<&'static SearchRule> {
    SEARCH_RULES.iter().find(|rule| rule.id == id)
}

/// One quality finding; sorts by (rule, scope, subject).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SearchQualityFinding {
    pub rule: String,
    pub scope: String,
    pub subject: String,
    pub message: String,
    pub evidence: Vec<ServiceEvidence>,
    pub services: Vec<String>,
    pub diff: Vec<String>,
}

/// One KPI value; `numerator`/`denominator` are set for ratios.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchKpi {
    pub id: String,
    pub subject: Option<String>,
    pub value: f64,
    pub numerator: Option<f64>,
    pub denominator: Option<f64>,
}

/// Findings sorted by (rule, scope, subject), KPIs by id; no timestamps.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchQuality {
    pub findings: Vec<SearchQualityFinding>,
    pub kpis: Vec<SearchKpi>,
}

/// `[search_quality]`: opt-in gate, severity overrides and thresholds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SearchQualityConfig {
    /// `check` includes the findings only when true.
    pub check: bool,
    /// Finding rules raised to `error`; needs `check = true`.
    pub errors: Vec<String>,
    /// Finding rules switched off everywhere.
    pub disabled: Vec<String>,
    /// Elastic "Mapping limit settings": `index.mapping.total_fields.limit`, default 1000.
    pub total_fields_limit: u64,
    /// Elastic "Mapping limit settings": `index.mapping.depth.limit`, default 20.
    pub depth_limit: u64,
    /// Elastic 8.19 "Mapping limit settings": `index.mapping.nested_fields.limit`,
    /// default 50 (9.x: 100; projects on 9.x set 100).
    pub nested_fields_limit: u64,
    /// ArchGuard starting value, not from Elastic: a finding fires at this
    /// share of each of the three limits.
    pub limit_ratio: f64,
    /// Elastic "Index modules": `index.refresh_interval` defaults to `1s`.
    pub min_refresh_interval: String,
}

impl Default for SearchQualityConfig {
    fn default() -> Self {
        Self {
            check: false,
            errors: Vec::new(),
            disabled: Vec::new(),
            total_fields_limit: 1000,
            depth_limit: 20,
            nested_fields_limit: 50,
            limit_ratio: 0.8,
            min_refresh_interval: "1s".into(),
        }
    }
}

/// An Elasticsearch time value in milliseconds (d, h, m, s, ms, micros,
/// nanos); `-1` and anything unreadable give `None`.
pub fn parse_es_duration_ms(text: &str) -> Option<f64> {
    let text = text.trim();
    let split = text.find(|c: char| !c.is_ascii_digit() && c != '.')?;
    let number: f64 = text[..split].parse().ok()?;
    let unit_ms = match &text[split..] {
        "d" => 86_400_000.0,
        "h" => 3_600_000.0,
        "m" => 60_000.0,
        "s" => 1000.0,
        "ms" => 1.0,
        "micros" => 0.001,
        "nanos" => 0.000_001,
        _ => return None,
    };
    Some(number * unit_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    const IDS: [&str; 27] = [
        "total_fields_limit",
        "depth_limit",
        "nested_fields_limit",
        "source_disabled",
        "dynamic_mode",
        "text_without_keyword",
        "analysis_drift",
        "unused_analysis",
        "environment_drift",
        "zero_replicas",
        "refresh_interval_low",
        "time_series_without_ilm",
        "deletion_protection_off",
        "write_bypasses_alias",
        "alias_without_write_index",
        "template_covers_nothing",
        "template_priority_conflict",
        "unused_pipeline",
        "undeclared_index",
        "uncovered_pattern",
        "alert_undeclared_index",
        "undeclared_pipeline",
        "shared_write_index",
        "reader_fan_in",
        "cross_project_access",
        "readable_share",
        "painless_complexity",
    ];

    #[test]
    fn rule_table_is_complete_and_unique() {
        let ids: BTreeSet<_> = SEARCH_RULES.iter().map(|rule| rule.id).collect();
        assert_eq!(ids.len(), SEARCH_RULES.len(), "ids are unique");
        assert_eq!(ids, IDS.into_iter().collect::<BTreeSet<_>>());
        for rule in SEARCH_RULES {
            let is_finding = rule.kind == RuleKind::Finding;
            assert_eq!(rule.severity.is_some(), is_finding, "{}", rule.id);
            assert_eq!(rule.attribute.is_some(), is_finding, "{}", rule.id);
            assert!(rule.severity != Some(Severity::Error), "{}", rule.id);
        }
        assert_eq!(
            search_rule("zero_replicas").map(|r| r.group),
            Some("lifecycle")
        );
        assert!(search_rule("nope").is_none());
    }

    #[test]
    fn durations_parse_to_milliseconds() {
        assert_eq!(parse_es_duration_ms("1s"), Some(1000.0));
        assert_eq!(parse_es_duration_ms("500ms"), Some(500.0));
        assert_eq!(parse_es_duration_ms("1m"), Some(60_000.0));
        assert_eq!(parse_es_duration_ms("-1"), None);
        assert_eq!(parse_es_duration_ms("5"), None);
        assert_eq!(parse_es_duration_ms("1x"), None);
    }
}
