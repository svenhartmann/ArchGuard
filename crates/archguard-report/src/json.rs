//! Canonical JSON serialization of [`Report`] (Kap. 5, T18).
//!
//! Field order is the struct declaration order of `archguard_core`, and the
//! [`Report`]'s maps are `BTreeMap`s, so two equal reports serialize to
//! exactly the same bytes — the property the golden tests below pin down.

use archguard_core::Report;

/// Serialize `r` to canonical JSON: struct field order, `BTreeMap` key
/// order, enum names as defined by `archguard_core`'s serde attributes.
/// `pretty` selects the indented form (2 spaces), otherwise the compact
/// form. [`Report`] contains only plain data, so serialization cannot
/// fail and never panics in practice.
pub fn to_json(r: &Report, pretty: bool) -> String {
    let serialized = if pretty {
        serde_json::to_string_pretty(r)
    } else {
        serde_json::to_string(r)
    };
    serialized.expect("Report is plain data and always serializable")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::to_json;
    use archguard_core::{
        ConfigSource, Finding, FindingKind, ModuleMetrics, Report, Severity, Status, SuggestedCut,
        Summary,
    };

    /// Hand-built report mirroring the Kap. 5 example shape, with the
    /// `config` and `metrics` entries deliberately inserted out of sort
    /// order so the golden strings also pin down the `BTreeMap` ordering.
    fn sample_report() -> Report {
        Report {
            semantic: None,
            review_kpis: None,
            schema_version: 1,
            summary: Summary {
                nodes: 142,
                edges: 519,
                sccs: 2,
                largest_scc: 7,
                layer_violations: 3,
                status: Status::Fail,
                ccd: 0,
                acd: 0.0,
                nccd: 0.0,
            },
            config: [
                ("rules".to_string(), ConfigSource::Default),
                ("project".to_string(), ConfigSource::Derived),
                ("layers".to_string(), ConfigSource::Mixed),
            ]
            .into_iter()
            .collect(),
            findings: vec![
                Finding {
                    message: None,
                    evidence: Vec::new(),
                    kind: FindingKind::Cycle,
                    id: "scc-3".to_string(),
                    severity: Severity::Error,
                    nodes: vec![
                        "app/billing".to_string(),
                        "app/tax".to_string(),
                        "domain/money".to_string(),
                    ],
                    edge_count: 5,
                    cycles_sampled: 4,
                    sampling: None,
                    suggested_cuts: vec![SuggestedCut {
                        from: "domain/money".to_string(),
                        to: "app/tax".to_string(),
                        score: 0.91,
                        reason: "Nur 1 Import-Stelle; Trennen bricht 4 von 4 Zyklen.".to_string(),
                    }],
                },
                Finding {
                    message: None,
                    evidence: Vec::new(),
                    kind: FindingKind::LayerViolation,
                    id: "lv-1".to_string(),
                    severity: Severity::Info,
                    nodes: vec!["domain/money".to_string(), "app/report".to_string()],
                    edge_count: 1,
                    cycles_sampled: 0,
                    sampling: None,
                    suggested_cuts: Vec::new(),
                },
            ],
            metrics: [
                (
                    "domain/money".to_string(),
                    ModuleMetrics {
                        fan_in: 5,
                        fan_out: 0,
                        instability: 0.0,
                        in_cycle: false,
                    },
                ),
                (
                    "app/billing".to_string(),
                    ModuleMetrics {
                        fan_in: 3,
                        fan_out: 2,
                        instability: 0.4,
                        in_cycle: true,
                    },
                ),
            ]
            .into_iter()
            .collect(),
        }
    }

    #[test]
    fn to_json_compact_matches_golden() {
        assert_eq!(
            to_json(&sample_report(), false),
            r#"{"schema_version":1,"summary":{"nodes":142,"edges":519,"sccs":2,"largest_scc":7,"layer_violations":3,"status":"fail","ccd":0,"acd":0.0,"nccd":0.0},"config":{"layers":"mixed","project":"derived","rules":"default"},"findings":[{"kind":"cycle","id":"scc-3","severity":"error","nodes":["app/billing","app/tax","domain/money"],"edge_count":5,"cycles_sampled":4,"suggested_cuts":[{"from":"domain/money","to":"app/tax","score":0.91,"reason":"Nur 1 Import-Stelle; Trennen bricht 4 von 4 Zyklen."}]},{"kind":"layer_violation","id":"lv-1","severity":"info","nodes":["domain/money","app/report"],"edge_count":1,"cycles_sampled":0,"suggested_cuts":[]}],"metrics":{"app/billing":{"fan_in":3,"fan_out":2,"instability":0.4,"in_cycle":true},"domain/money":{"fan_in":5,"fan_out":0,"instability":0.0,"in_cycle":false}}}"#
        );
    }

    #[test]
    fn to_json_pretty_matches_golden() {
        let mut report = sample_report();
        report.findings.clear();
        report.metrics.clear();
        report.config.clear();
        report.summary.status = Status::Pass;
        assert_eq!(
            to_json(&report, true),
            r#"{
  "schema_version": 1,
  "summary": {
    "nodes": 142,
    "edges": 519,
    "sccs": 2,
    "largest_scc": 7,
    "layer_violations": 3,
    "status": "pass",
    "ccd": 0,
    "acd": 0.0,
    "nccd": 0.0
  },
  "config": {},
  "findings": [],
  "metrics": {}
}"#
        );
    }

    #[test]
    fn to_json_is_stable_and_pretty_is_indented() {
        let report = sample_report();
        // Same input, same bytes — the determinism promise of Kap. 12.
        assert_eq!(to_json(&report, false), to_json(&report, false));
        assert_eq!(to_json(&report, true), to_json(&report, true));
        assert!(!to_json(&report, false).contains('\n'));
        assert!(to_json(&report, true).starts_with("{\n  \"schema_version\": 1"));
    }

    #[test]
    fn empty_maps_still_serialize_as_present_blocks() {
        // `config` is `#[serde(default)]` but never skipped: an empty
        // section serializes as `{}`, never as an absent key.
        let mut report = sample_report();
        report.config = BTreeMap::new();
        // Compact form: the `config` block sits between `summary` and
        // `findings` (struct order) and is `{}`, never an absent key.
        assert!(to_json(&report, false).contains(r#""config":{},"findings":[{"#));
    }

    #[test]
    fn review_kpis_block_is_absent_not_empty() {
        // Backward compatibility (same pattern as `semantic`): a report
        // without the KPI pass serializes byte-identically to the
        // goldens — no `review_kpis` key at all.
        assert!(!to_json(&sample_report(), false).contains("review_kpis"));
    }

    #[test]
    fn review_kpis_block_serializes_in_field_order() {
        let mut report = sample_report();
        report.review_kpis = Some(archguard_core::ReviewKpis::default());
        // Struct order puts the block after `metrics` (and after the
        // skipped `semantic`); empty vectors serialize as `[]` inside.
        assert!(to_json(&report, false).contains(r#""review_kpis":{"propagation_cost":[]"#));
    }
}
