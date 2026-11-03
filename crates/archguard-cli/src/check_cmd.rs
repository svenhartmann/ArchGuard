//! Optional Git diff and relative ACD gate around the existing check command.
use crate::args::Cli;
use crate::output::{analysis_error, usage};
use archguard_engine::core::{Finding, FindingKind, Ref, Report, Severity};
use archguard_engine::{LoadedConfig, compare_to_ref, read_trend};
use std::path::Path;
#[expect(
    clippy::too_many_arguments,
    reason = "explicit check flags and shared run"
)]
pub(crate) fn run(
    cli: &Cli,
    root: &Path,
    loaded: &LoadedConfig,
    graph: &archguard_engine::core::Graph,
    report: &Report,
    baseline: Option<&Path>,
    base: Option<&str>,
    acd_limit: Option<f64>,
) -> i32 {
    if acd_limit.is_some_and(|n| !n.is_finite() || n < 0.0) {
        return usage("--fail-on-acd-regression must be finite and nonnegative");
    }
    let comparison = match base
        .map(|base| compare_to_ref(root, loaded, graph, report, base))
        .transpose()
    {
        Ok(value) => value,
        Err(error) => return analysis_error(error),
    };
    let mut checked = report.clone();
    if let Some(comparison) = &comparison {
        checked.findings = comparison.new_findings.clone();
    }
    if let Some(limit) = acd_limit {
        let history = read_trend(root, 1);
        if comparison.is_none()
            && history.last().is_some_and(|row| {
                row.analysis_scope.as_deref()
                    != Some(&archguard_engine::snapshot_scope(&loaded.config))
            })
        {
            return analysis_error(
                "ACD snapshot scope is missing or differs from current configuration; use --base or create a new snapshot with this scope",
            );
        }
        let previous = match comparison
            .as_ref()
            .map(|c| c.base_summary.acd)
            .or_else(|| history.last().map(|row| row.acd))
        {
            Some(value) => value,
            None => {
                return analysis_error(
                    "ACD regression gate needs --base or a saved snapshot; run archguard snapshot first",
                );
            }
        };
        if report.summary.acd > previous * (1.0 + limit / 100.0) {
            checked.findings.push(Finding {kind:FindingKind::MetricThreshold,id:"metric-acd-regression".into(),severity:Severity::Error,nodes:vec![],edge_count:0,cycles_sampled:0,sampling:None,suggested_cuts:vec![],evidence:vec![],message:Some(format!("ACD rose from {previous:.4} to {:.4}; allowed growth is {limit}%. This measures structural coupling, not elapsed build or runtime cost.",report.summary.acd))});
        }
    }
    // R31: the `[review]` KPI gates (R30) join the finding list, so text,
    // json and sarif all carry them and any breach flips the gate to fail.
    // Like the ACD regression gate they speak about the current structure,
    // so they are evaluated even when a comparison replaced the list.
    checked
        .findings
        .extend(review_gate_findings(loaded, report));
    let paths = if cli.format == crate::args::Format::Sarif {
        match archguard_engine::source_reference_paths(root, &loaded.config) {
            Ok(paths) => paths,
            Err(error) => return analysis_error(error),
        }
    } else {
        Default::default()
    };
    crate::check(cli, baseline, &checked, comparison.as_ref(), &paths)
}

/// How many per-violation evidence strings a gate finding samples at most.
const GATE_EVIDENCE_CAP: usize = 5;

/// Evaluate the `[review]` KPI gates (R30) against the report payload.
///
/// `None` means *not measured*: such a gate stays silent instead of
/// comparing against a fabricated zero — the same honesty rule as the KPI
/// tables. Only a real breach yields an error finding, which makes
/// [`crate::check`] exit 1 and shows up in text, json and sarif alike.
fn review_gate_findings(loaded: &LoadedConfig, report: &Report) -> Vec<Finding> {
    let Some(kpis) = &report.review_kpis else {
        return Vec::new();
    };
    let review = &loaded.config.review;
    let mut findings = Vec::new();
    if let Some(max) = review.max_sdp_violations {
        let violations = &kpis.sdp_violations;
        if (violations.len() as u32) > max {
            let mut nodes: Vec<String> = violations
                .iter()
                .flat_map(|violation| [violation.from.clone(), violation.to.clone()])
                .collect();
            nodes.sort();
            nodes.dedup();
            let evidence = violations
                .iter()
                .take(GATE_EVIDENCE_CAP)
                .map(|violation| {
                    let site = violation.evidence.first();
                    let mut reference = evidence_ref(site.map_or("", |site| site.as_str()));
                    reference.raw = match site {
                        Some(site) => format!("{} -> {} @ {site}", violation.from, violation.to),
                        None => format!("{} -> {}", violation.from, violation.to),
                    };
                    reference
                })
                .collect();
            findings.push(gate_finding(
                "metric-max-sdp-violations".into(),
                nodes,
                evidence,
                format!(
                    "{} SDP violations exceed the gate of {max} (a stable module depends on a less stable one).",
                    violations.len()
                ),
            ));
        }
    }
    // Aggregate numbers belong to the message — there is no single
    // source reference for a share over the whole view.
    if let (Some(max), Some(tangled)) = (review.max_tangledness, kpis.tangledness.as_ref())
        && tangled.quote > max
    {
        findings.push(gate_finding(
            "metric-max-tangledness".into(),
            Vec::new(),
            Vec::new(),
            format!(
                "Tangledness {:.3} exceeds the gate of {max:.3} ({} of {} nodes live in cyclic SCCs).",
                tangled.quote, tangled.tangled_nodes, tangled.total
            ),
        ));
    }
    if let Some(max) = review.max_propagation_cost {
        for cost in &kpis.propagation_cost {
            if cost.pc > max {
                findings.push(gate_finding(
                    format!("metric-max-propagation-cost-{}", cost.scc_id),
                    vec![cost.scc_id.clone()],
                    Vec::new(),
                    format!(
                        "Propagation cost {:.3} of {} exceeds the gate of {max:.3} ({} of {} nodes affected, {} inside the SCC).",
                        cost.pc, cost.scc_id, cost.affected, cost.total, cost.size
                    ),
                ));
            }
        }
    }
    findings
}

/// Parse a `file:line` evidence string into a [`Ref`]; a site without a
/// numeric line suffix stays raw with line `0` (sarif clamps it to 1).
fn evidence_ref(site: &str) -> Ref {
    match site.rsplit_once(':') {
        Some((file, line)) if !file.is_empty() && line.parse::<u32>().is_ok() => Ref {
            file: file.to_string(),
            line: line.parse().unwrap_or_default(),
            raw: site.to_string(),
        },
        _ => Ref {
            file: site.to_string(),
            line: 0,
            raw: site.to_string(),
        },
    }
}

fn gate_finding(id: String, nodes: Vec<String>, evidence: Vec<Ref>, message: String) -> Finding {
    Finding {
        kind: FindingKind::MetricThreshold,
        id,
        severity: Severity::Error,
        nodes,
        edge_count: 0,
        cycles_sampled: 0,
        sampling: None,
        suggested_cuts: Vec::new(),
        evidence,
        message: Some(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::{Config, PropagationCost, ReviewKpis, SdpViolation, Tangledness};
    use std::collections::BTreeMap;

    fn kpis() -> ReviewKpis {
        ReviewKpis {
            propagation_cost: vec![PropagationCost {
                scc_id: "scc-1".into(),
                size: 2,
                affected: 2,
                total: 4,
                pc: 0.5,
            }],
            sdp_violations: vec![SdpViolation {
                from: "domain".into(),
                to: "ui".into(),
                i_from: Some(0.1),
                i_to: Some(0.9),
                evidence: vec!["src/domain/a.ts:1".into()],
            }],
            zones: None,
            tangledness: Some(Tangledness {
                tangled_nodes: 2,
                total: 4,
                quote: 0.5,
            }),
            hub_gini: None,
            layer_drift: None,
            debt: None,
            api_surface: vec![],
            leaks: vec![],
            risk_gaps: vec![],
            dependencies: None,
            health: None,
        }
    }

    fn report(kpis: Option<ReviewKpis>) -> Report {
        Report {
            schema_version: 1,
            summary: archguard_engine::core::Summary {
                nodes: 4,
                edges: 3,
                sccs: 1,
                largest_scc: 2,
                layer_violations: 0,
                status: archguard_engine::core::Status::Pass,
                ccd: 0,
                acd: 0.0,
                nccd: 0.0,
            },
            config: BTreeMap::new(),
            findings: vec![],
            metrics: BTreeMap::new(),
            semantic: None,
            review_kpis: kpis,
        }
    }

    fn loaded(config: Config) -> LoadedConfig {
        LoadedConfig {
            config,
            sources: BTreeMap::new(),
        }
    }

    #[test]
    fn unconfigured_gates_produce_no_findings() {
        let findings = review_gate_findings(&loaded(Config::default()), &report(Some(kpis())));
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn missing_kpi_payload_keeps_gates_silent() {
        let mut config = Config::default();
        config.review.max_tangledness = Some(0.0);
        config.review.max_propagation_cost = Some(0.0);
        let findings = review_gate_findings(&loaded(config), &report(None));
        assert!(
            findings.is_empty(),
            "not measured is not zero: {findings:?}"
        );
    }

    #[test]
    fn every_breached_gate_yields_one_error_finding_with_evidence() {
        let mut config = Config::default();
        config.review.max_sdp_violations = Some(0);
        config.review.max_tangledness = Some(0.4);
        config.review.max_propagation_cost = Some(0.4);
        let findings = review_gate_findings(&loaded(config), &report(Some(kpis())));
        let ids: Vec<&str> = findings.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "metric-max-sdp-violations",
                "metric-max-tangledness",
                "metric-max-propagation-cost-scc-1"
            ]
        );
        assert!(findings.iter().all(|f| f.severity == Severity::Error));
        assert_eq!(findings[0].nodes, ["domain", "ui"]);
        assert_eq!(findings[0].evidence[0].file, "src/domain/a.ts");
        assert_eq!(findings[0].evidence[0].line, 1);
        assert!(
            findings[0].evidence[0]
                .raw
                .starts_with("domain -> ui @ src/domain/a.ts:1")
        );
        // The aggregate gates carry their numbers in the message.
        assert!(
            findings[1]
                .message
                .as_ref()
                .expect("message")
                .contains("2 of 4 nodes")
        );
        assert!(
            findings[2]
                .message
                .as_ref()
                .expect("message")
                .contains("2 of 4 nodes affected")
        );
    }

    #[test]
    fn a_value_at_the_gate_is_allowed() {
        let mut config = Config::default();
        config.review.max_tangledness = Some(0.5);
        config.review.max_propagation_cost = Some(0.5);
        config.review.max_sdp_violations = Some(1);
        assert!(review_gate_findings(&loaded(config), &report(Some(kpis()))).is_empty());
    }
}
