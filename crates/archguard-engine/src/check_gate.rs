//! The `check` gate shared by `archguard check` and the MCP `check` tool:
//! the empty-analysis refusal, the `[review]` KPI gates (R30/R31) and the
//! fail rule with its baseline acceptance (Kap. 5/7).
use std::path::Path;

use archguard_analysis::{Baseline, cycle_hash};
use archguard_core::{Config, Finding, FindingKind, Graph, Ref, Report, Severity};

/// Nothing analysed is no evidence of a clean architecture: a wrong path
/// (e.g. the GitHub Action's `path:`) must not turn the gate green.
pub fn empty_analysis_error(root: &Path, graph: &Graph) -> Option<String> {
    graph.nodes.is_empty().then(|| {
        format!(
            "no source files found under {}; check the path and [project] source_roots/languages",
            root.display()
        )
    })
}

/// Read and parse a baseline JSON (Kap. 7); any IO or parse problem is a
/// displayable error naming the file.
pub fn read_baseline(path: &Path) -> Result<Baseline, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|err| format!("baseline {}: {err}", path.display()))?;
    serde_json::from_str(&text)
        .map_err(|err| format!("baseline {}: invalid baseline JSON: {err}", path.display()))
}

/// Whether `baseline` accepts `finding`: only **cycle** findings whose
/// membership [`cycle_hash`] the baseline stores are accepted.
pub fn accepted_by_baseline(baseline: Option<&Baseline>, finding: &Finding) -> bool {
    match (baseline, finding.kind) {
        (Some(baseline), FindingKind::Cycle) => {
            baseline.hashes.contains(&cycle_hash(&finding.nodes))
        }
        _ => false,
    }
}

/// Gate semantics: fail as soon as one `Error` finding is not accepted
/// by the baseline — every non-cycle Error fails regardless.
pub fn gate_fails(findings: &[Finding], baseline: Option<&Baseline>) -> bool {
    findings
        .iter()
        .any(|f| f.severity == Severity::Error && !accepted_by_baseline(baseline, f))
}

/// The gate findings `check` adds to the analysis findings: the `[review]`
/// KPI gates on the report's KPI payload, then the `[licenses]` policy over
/// the lockfiles under `root` (ADR 0016). Both speak about the current
/// state, so they apply even when a comparison replaced the finding list;
/// an unconfigured `[licenses]` section reads no lockfile.
pub fn check_gate_findings(root: &Path, config: &Config, report: &Report) -> Vec<Finding> {
    let mut findings = review_gate_findings(config, report);
    let policy = &config.licenses;
    if policy.is_active() {
        findings.extend(crate::sbom::license_findings(
            &crate::sbom::sbom(root, policy),
            policy,
        ));
    }
    findings.extend(crate::view_conventions::convention_findings(root, config));
    findings
}

/// How many per-violation evidence strings a gate finding samples at most.
const GATE_EVIDENCE_CAP: usize = 5;

/// Evaluate the `[review]` KPI gates (R30) against the report payload.
///
/// `None` means *not measured*: such a gate stays silent instead of
/// comparing against a fabricated zero — the same honesty rule as the KPI
/// tables. Only a real breach yields an error finding.
pub fn review_gate_findings(config: &Config, report: &Report) -> Vec<Finding> {
    let Some(kpis) = &report.review_kpis else {
        return Vec::new();
    };
    let review = &config.review;
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
    use archguard_core::{PropagationCost, ReviewKpis, SdpViolation, Status, Tangledness};
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
            summary: archguard_core::Summary {
                nodes: 4,
                edges: 3,
                sccs: 1,
                largest_scc: 2,
                layer_violations: 0,
                status: Status::Pass,
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

    #[test]
    fn unconfigured_gates_produce_no_findings() {
        let findings = review_gate_findings(&Config::default(), &report(Some(kpis())));
        assert!(findings.is_empty(), "{findings:?}");
    }

    #[test]
    fn missing_kpi_payload_keeps_gates_silent() {
        let mut config = Config::default();
        config.review.max_tangledness = Some(0.0);
        config.review.max_propagation_cost = Some(0.0);
        let findings = review_gate_findings(&config, &report(None));
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
        let findings = review_gate_findings(&config, &report(Some(kpis())));
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
        assert!(review_gate_findings(&config, &report(Some(kpis()))).is_empty());
    }

    #[test]
    fn check_gate_findings_fail_the_gate_on_a_breach() {
        let mut config = Config::default();
        config.review.max_tangledness = Some(0.4);
        let dir = tempfile::tempdir().unwrap();
        let findings = check_gate_findings(dir.path(), &config, &report(Some(kpis())));
        assert_eq!(findings[0].id, "metric-max-tangledness");
        assert!(gate_fails(&findings, None));
    }

    #[test]
    fn check_gate_findings_add_licence_violations() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package-lock.json"),
            r#"{"packages":{"":{},"node_modules/gpl-thing":{"version":"2.0.0","license":"GPL-3.0-only"}}}"#,
        )
        .unwrap();
        let mut config = Config::default();
        config.licenses.deny = vec!["GPL-3.0-only".into()];
        let findings = check_gate_findings(dir.path(), &config, &report(None));
        let kinds: Vec<FindingKind> = findings.iter().map(|f| f.kind).collect();
        assert_eq!(kinds, [FindingKind::License], "{findings:?}");
        assert!(gate_fails(&findings, None));
    }

    #[test]
    fn check_gate_findings_add_view_conventions() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".archguard")).unwrap();
        std::fs::write(
            dir.path().join(".archguard/ui-flow.json"),
            r#"{"definition":"ui-flow/v1","complete":false,"transitions":[{"from":{"screen":"a"},"target":{"unresolved":"url"},"file":"A.razor","line":2,"source":"x"}]}"#,
        )
        .unwrap();
        let config: Config =
            toml::from_str("[views]\nconventions = [\"navigation_literal\"]").unwrap();
        let findings = check_gate_findings(dir.path(), &config, &report(None));
        let ids: Vec<&str> = findings.iter().map(|f| f.id.as_str()).collect();
        assert_eq!(ids, ["view-navigation_literal:A.razor:url"]);
        assert!(gate_fails(&findings, None));
    }

    #[test]
    fn a_baseline_accepts_only_recorded_cycles() {
        let nodes = vec!["a".to_string(), "b".to_string()];
        let cycle = Finding {
            kind: FindingKind::Cycle,
            ..gate_finding("scc-1".into(), nodes.clone(), vec![], String::new())
        };
        let baseline = Baseline {
            schema_version: 1,
            hashes: [cycle_hash(&nodes)].into(),
        };
        assert!(gate_fails(std::slice::from_ref(&cycle), None));
        assert!(!gate_fails(std::slice::from_ref(&cycle), Some(&baseline)));
        let metric = gate_finding("m".into(), nodes, vec![], String::new());
        assert!(gate_fails(&[metric], Some(&baseline)));
    }

    #[test]
    fn an_empty_graph_is_refused() {
        let graph = archguard_core::GraphBuilder::new().build(String::new());
        let message = empty_analysis_error(Path::new("/x"), &graph).expect("refused");
        assert!(
            message.contains("no source files found under /x"),
            "{message}"
        );
    }
}
