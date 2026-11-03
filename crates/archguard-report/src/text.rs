//! Compact human/agent-readable text rendering of [`Report`] (Kap. 5, T19).
//!
//! The output is deliberately plain: no colors, no ANSI escapes, one line
//! per piece of information, so it can be piped through `grep` or read by
//! an agent unchanged. It starts with a summary line of the form
//! `FAIL: 2 cycles, 3 layer violations (142 modules, 519 edges)` — the
//! per-kind counts are counted over [`Report::findings`] in
//! [`FindingKind`] declaration order (zero-count kinds are omitted; a
//! report without findings reads `no findings`) — followed by one block
//! per finding: id, severity, kind, member list (wrapped at
//! [`MAX_MEMBERS`] names with a trailing `…`) and the top suggested cut.
//!
//! Findings are rendered in the order the [`Report`] carries them (Kap. 5
//! already fixes that as severity, then id); the text layer never re-sorts.
//! The verdict token is `FAIL`/`PASS` from [`Status`] — core defines no
//! other verdict. The string ends with a trailing newline.

use archguard_core::{Finding, FindingKind, HealthLevel, Report, ReviewKpis, Severity, Status};

/// Member names shown per finding before the list is wrapped with `…`.
const MAX_MEMBERS: usize = 5;

/// Render `r` as compact plain text. See the module docs for the format.
pub fn to_text(r: &Report) -> String {
    let mut out = summary_line(r);
    out.push('\n');
    if let Some(semantic) = &r.semantic {
        out.push_str(&format!(
            "Semantic dependencies: {}; {} resolved call sites, {} inheritance references, {} unmapped project calls.\n",
            semantic.status, semantic.projected_calls, semantic.projected_supertypes,
            semantic.skipped_project_calls,
        ));
        out.push_str("Scope: imports, compiler-resolved calls, inheritance and available declaration types; runtime wiring is not covered. No detected cycles is not proof of an acyclic codebase.\n");
    }
    if let Some(kpis) = &r.review_kpis {
        out.push('\n');
        out.push_str(&kpi_table(kpis));
    }
    for finding in &r.findings {
        out.push('\n');
        out.push_str(&finding_block(finding));
    }
    out
}

/// Shown whenever a KPI has no value — absence is "not measured", never 0.
const NOT_MEASURED: &str = "not measured";

/// One `KPI | Wert | Delta | Ampel` table row, plain text, fixed columns.
/// `delta` is only available where the payload carries a baseline (the
/// health trend); `ampel` only where a level is derived (health). Both
/// stay `-` otherwise — a delta needs a previous snapshot (the CLI adds
/// those) and an ampel needs configured gates; the report invents neither.
fn kpi_row(name: &str, value: &str, delta: &str, ampel: &str) -> String {
    format!("  {name:<22} {value:>24}  delta {delta:>6}  ampel {ampel}\n")
}

/// The review-KPI table (R26), one line per KPI in [`ReviewKpis`]
/// declaration order. Deterministic: no clocks, no locale numbers.
fn kpi_table(kpis: &ReviewKpis) -> String {
    let mut out = String::from("Review KPIs:\n");
    let propagation = if kpis.propagation_cost.is_empty() {
        "0 cycles".to_string()
    } else {
        let max = kpis
            .propagation_cost
            .iter()
            .map(|p| p.pc)
            .fold(0.0_f64, f64::max);
        format!("{} cycles, max {:.2}", kpis.propagation_cost.len(), max)
    };
    out.push_str(&kpi_row("propagation_cost", &propagation, "-", "-"));
    out.push_str(&kpi_row(
        "sdp_violations",
        &format!("{}", kpis.sdp_violations.len()),
        "-",
        "-",
    ));
    let zones = match &kpis.zones {
        Some(z) => format!(
            "pain {}, useless {}, main {}, unmeasured {}",
            z.zone_of_pain, z.zone_of_uselessness, z.main_sequence, z.unmeasured
        ),
        None => NOT_MEASURED.to_string(),
    };
    out.push_str(&kpi_row("zones", &zones, "-", "-"));
    let pct = |v: f64| format!("{:.2}", v);
    out.push_str(&kpi_row(
        "tangledness",
        &kpis
            .tangledness
            .as_ref()
            .map_or_else(|| NOT_MEASURED.to_string(), |t| pct(t.quote)),
        "-",
        "-",
    ));
    out.push_str(&kpi_row(
        "hub_gini",
        &kpis
            .hub_gini
            .as_ref()
            .map_or_else(|| NOT_MEASURED.to_string(), |h| pct(h.gini)),
        "-",
        "-",
    ));
    out.push_str(&kpi_row(
        "layer_drift_agreement",
        &kpis.layer_drift.as_ref().map_or_else(
            || NOT_MEASURED.to_string(),
            |d| d.agreement.map_or_else(|| NOT_MEASURED.to_string(), pct),
        ),
        "-",
        "-",
    ));
    let debt = match &kpis.debt {
        Some(d) => match d.per_kloc {
            Some(per_kloc) => format!("{} markers, {:.2}/kloc", d.markers, per_kloc),
            None => format!("{} markers, {} loc", d.markers, d.loc),
        },
        None => NOT_MEASURED.to_string(),
    };
    out.push_str(&kpi_row("debt_density", &debt, "-", "-"));
    let exported: usize = kpis.api_surface.iter().map(|a| a.exported_symbols).sum();
    out.push_str(&kpi_row(
        "api_surface",
        &format!(
            "{exported} symbols in {} files",
            kpis.api_surface.iter().map(|a| a.files).sum::<usize>()
        ),
        "-",
        "-",
    ));
    out.push_str(&kpi_row(
        "leaks",
        &format!("{}", kpis.leaks.len()),
        "-",
        "-",
    ));
    out.push_str(&kpi_row(
        "risk_coverage_gaps",
        &format!("{}", kpis.risk_gaps.len()),
        "-",
        "-",
    ));
    let dependencies = match &kpis.dependencies {
        Some(inv) => match inv.stale_count {
            Some(stale) => format!("{} deps, {} stale", inv.total, stale),
            None => format!("{} deps, stale {}", inv.total, NOT_MEASURED),
        },
        None => NOT_MEASURED.to_string(),
    };
    out.push_str(&kpi_row("dependencies", &dependencies, "-", "-"));
    let (score, delta, ampel) = match &kpis.health {
        Some(h) => (
            h.score
                .map_or_else(|| NOT_MEASURED.to_string(), |s| format!("{s:.1}")),
            h.trend.map_or("-".to_string(), |t| format!("{t:+.1}")),
            match h.level {
                Some(HealthLevel::Green) => "green",
                Some(HealthLevel::Yellow) => "yellow",
                Some(HealthLevel::Red) => "red",
                None => "-",
            }
            .to_string(),
        ),
        None => (NOT_MEASURED.to_string(), "-".to_string(), "-".to_string()),
    };
    out.push_str(&kpi_row("health", &score, &delta, &ampel));
    out
}

/// `FAIL: 2 cycles, 3 layer violations (142 modules, 519 edges)` —
/// without trailing newline.
fn summary_line(r: &Report) -> String {
    let verdict = match r.summary.status {
        Status::Pass => "PASS",
        Status::Fail => "FAIL",
    };
    let counts: Vec<String> = [
        FindingKind::Cycle,
        FindingKind::LayerViolation,
        FindingKind::MetricThreshold,
        FindingKind::LayerUnranked,
        FindingKind::DependencyRule,
        FindingKind::OwnershipRule,
        FindingKind::Orphan,
        FindingKind::TargetArchitecture,
    ]
    .into_iter()
    .filter_map(|kind| {
        let n = r.findings.iter().filter(|f| f.kind == kind).count();
        (n > 0).then(|| format!("{n} {}", pluralize(kind_label(kind), n)))
    })
    .collect();
    let body = if counts.is_empty() {
        "no findings".to_string()
    } else {
        counts.join(", ")
    };
    format!(
        "{verdict}: {body} ({} modules, {} edges)",
        r.summary.nodes, r.summary.edges
    )
}

/// One finding block, terminated by a newline:
///
/// ```text
/// [scc-3] error cycle
///   members: app/billing, app/tax, domain/money
///   cut: domain/money -> app/tax (score 0.91): Nur 1 Import-Stelle; …
/// ```
fn finding_block(finding: &Finding) -> String {
    let severity = match finding.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
    };
    let mut block = format!(
        "[{}] {} {}\n  members: {}\n",
        finding.id,
        severity,
        kind_token(finding.kind),
        member_list(&finding.nodes),
    );
    if let Some(message) = &finding.message {
        block.push_str(&format!("  message: {message}\n"));
    }
    for evidence in &finding.evidence {
        block.push_str(&format!(
            "  evidence: {}:{} {}\n",
            evidence.file, evidence.line, evidence.raw
        ));
    }
    // The top cut is the highest-scoring candidate; on a score tie the
    // first emitted one wins (`.rev().max_by` keeps it — `max_by` alone
    // would return the last).
    match finding
        .suggested_cuts
        .iter()
        .rev()
        .max_by(|a, b| a.score.total_cmp(&b.score))
    {
        Some(cut) => block.push_str(&format!(
            "  cut: {} -> {} (score {}): {}\n",
            cut.from, cut.to, cut.score, cut.reason
        )),
        None => block.push_str("  cut: none\n"),
    }
    block
}

/// Member names joined by `, `, truncated to [`MAX_MEMBERS`] with a
/// trailing `…`; `(none)` for the (degenerate) empty list.
fn member_list(nodes: &[String]) -> String {
    if nodes.is_empty() {
        return "(none)".to_string();
    }
    let mut shown: Vec<&str> = nodes.iter().take(MAX_MEMBERS).map(String::as_str).collect();
    if nodes.len() > MAX_MEMBERS {
        shown.push("…");
    }
    shown.join(", ")
}

/// Machine-stable kind token, identical to the canonical JSON name.
fn kind_token(kind: FindingKind) -> &'static str {
    match kind {
        FindingKind::Cycle => "cycle",
        FindingKind::LayerViolation => "layer_violation",
        FindingKind::MetricThreshold => "metric_threshold",
        FindingKind::LayerUnranked => "layer_unranked",
        FindingKind::DependencyRule => "dependency_rule",
        FindingKind::OwnershipRule => "ownership_rule",
        FindingKind::Orphan => "orphan",
        FindingKind::TargetArchitecture => "target_architecture",
    }
}

/// English summary wording for `kind`, singular, pluralized by `n`.
fn kind_label(kind: FindingKind) -> &'static str {
    match kind {
        FindingKind::Cycle => "cycle",
        FindingKind::LayerViolation => "layer violation",
        FindingKind::MetricThreshold => "metric threshold",
        FindingKind::LayerUnranked => "unranked module",
        FindingKind::DependencyRule => "dependency rule violation",
        FindingKind::OwnershipRule => "ownership rule violation",
        FindingKind::Orphan => "orphan candidate",
        FindingKind::TargetArchitecture => "target architecture violation",
    }
}

/// Naive English pluralization: every label in [`kind_label`] pluralizes
/// with a plain `s`.
fn pluralize(label: &str, n: usize) -> String {
    if n == 1 {
        label.to_string()
    } else {
        format!("{label}s")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::to_text;
    use archguard_core::{
        ConfigSource, Finding, FindingKind, HealthLevel, ModuleMetrics, Report, ReviewKpis,
        Severity, Status, SuggestedCut, Summary,
    };

    fn cut(from: &str, to: &str, score: f32, reason: &str) -> SuggestedCut {
        SuggestedCut {
            from: from.to_string(),
            to: to.to_string(),
            score,
            reason: reason.to_string(),
        }
    }

    fn finding(
        kind: FindingKind,
        id: &str,
        severity: Severity,
        nodes: &[&str],
        cuts: Vec<SuggestedCut>,
    ) -> Finding {
        Finding {
            message: None,
            evidence: Vec::new(),
            kind,
            id: id.to_string(),
            severity,
            nodes: nodes.iter().map(|n| n.to_string()).collect(),
            edge_count: 5,
            cycles_sampled: 4,
            sampling: None,
            suggested_cuts: cuts,
        }
    }

    /// Same shape as `json.rs`'s golden sample: one cycle finding with a
    /// suggested cut and one bare layer violation.
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
                layer_violations: 1,
                status: Status::Fail,
                ccd: 0,
                acd: 0.0,
                nccd: 0.0,
            },
            config: [(("rules".to_string()), ConfigSource::Default)]
                .into_iter()
                .collect(),
            findings: vec![
                finding(
                    FindingKind::Cycle,
                    "scc-3",
                    Severity::Error,
                    &["app/billing", "app/tax", "domain/money"],
                    vec![cut(
                        "domain/money",
                        "app/tax",
                        0.91,
                        "Nur 1 Import-Stelle; Trennen bricht 4 von 4 Zyklen.",
                    )],
                ),
                finding(
                    FindingKind::LayerViolation,
                    "lv-1",
                    Severity::Info,
                    &["domain/money", "app/report"],
                    Vec::new(),
                ),
            ],
            metrics: [(
                "domain/money".to_string(),
                ModuleMetrics {
                    fan_in: 5,
                    fan_out: 0,
                    instability: 0.0,
                    in_cycle: false,
                },
            )]
            .into_iter()
            .collect(),
        }
    }

    #[test]
    fn to_text_matches_golden() {
        assert_eq!(
            to_text(&sample_report()),
            "FAIL: 1 cycle, 1 layer violation (142 modules, 519 edges)\n\n[scc-3] error cycle\n  members: app/billing, app/tax, domain/money\n  cut: domain/money -> app/tax (score 0.91): Nur 1 Import-Stelle; Trennen bricht 4 von 4 Zyklen.\n\n[lv-1] info layer_violation\n  members: domain/money, app/report\n  cut: none\n"
        );
    }

    #[test]
    fn to_text_passes_clean_reports_without_blocks() {
        let mut report = sample_report();
        report.findings.clear();
        report.summary.status = Status::Pass;
        assert_eq!(
            to_text(&report),
            "PASS: no findings (142 modules, 519 edges)\n"
        );
    }

    #[test]
    fn zero_cycles_preserves_semantic_coverage_limitations() {
        let mut report = sample_report();
        report.findings.clear();
        report.summary.status = Status::Pass;
        report.summary.sccs = 0;
        report.semantic = Some(archguard_core::SemanticDependencyCoverage {
            status: "partial".into(),
            projected_calls: 7,
            skipped_project_calls: 2,
            ..Default::default()
        });
        let rendered = to_text(&report);
        assert!(rendered.contains("Semantic dependencies: partial"));
        assert!(rendered.contains("7 resolved call sites"));
        assert!(rendered.contains("2 unmapped project calls"));
        assert!(rendered.contains("No detected cycles is not proof of an acyclic codebase"));
        assert!(rendered.contains("runtime wiring is not covered"));
    }

    #[test]
    fn to_text_wraps_members_at_five_names() {
        let mut report = sample_report();
        report.findings = vec![finding(
            FindingKind::Cycle,
            "scc-9",
            Severity::Warning,
            &[
                "a", "b", "c", "d", "e", "f", "g", // six and seven are hidden
            ],
            Vec::new(),
        )];
        assert!(to_text(&report).contains("  members: a, b, c, d, e, …\n"));
        assert!(!to_text(&report).contains("f"));
    }

    #[test]
    fn to_text_counts_every_kind_in_declaration_order() {
        let mut report = sample_report();
        report.summary.status = Status::Pass;
        report.findings = vec![
            finding(
                FindingKind::LayerUnranked,
                "u-2",
                Severity::Info,
                &["x"],
                Vec::new(),
            ),
            finding(
                FindingKind::LayerViolation,
                "lv-3",
                Severity::Info,
                &["y"],
                Vec::new(),
            ),
            finding(
                FindingKind::MetricThreshold,
                "mt-1",
                Severity::Warning,
                &["z"],
                Vec::new(),
            ),
            finding(
                FindingKind::Cycle,
                "scc-1",
                Severity::Error,
                &["c"],
                Vec::new(),
            ),
            finding(
                FindingKind::MetricThreshold,
                "mt-2",
                Severity::Warning,
                &["w"],
                Vec::new(),
            ),
        ];
        // Sorted by kind (not by the order the findings appear in), pluralized.
        let first_line = to_text(&report)
            .lines()
            .next()
            .expect("summary line")
            .to_string();
        assert_eq!(
            first_line,
            "PASS: 1 cycle, 1 layer violation, 2 metric thresholds, 1 unranked module \
             (142 modules, 519 edges)"
        );
    }

    #[test]
    fn to_text_shows_highest_scoring_cut_first_wins_ties() {
        let mut report = sample_report();
        report.findings = vec![finding(
            FindingKind::Cycle,
            "scc-4",
            Severity::Error,
            &["a", "b"],
            vec![
                cut("a", "b", 0.5, "lower"),
                cut("b", "c", 0.9, "best"),
                cut("c", "d", 0.9, "tie: first wins"),
            ],
        )];
        assert!(
            to_text(&report).contains("  cut: b -> c (score 0.9): best\n"),
            "{}",
            to_text(&report)
        );
    }

    #[test]
    fn to_text_is_plain_and_trailing_newline_terminated() {
        let text = to_text(&sample_report());
        // Pipe-safe: no ANSI escapes anywhere.
        assert!(!text.contains('\u{1b}'));
        assert!(text.ends_with('\n'));
        assert!(!text.ends_with("\n\n"));
        // Determinism: same input, same bytes.
        assert_eq!(text, to_text(&sample_report()));
    }

    #[test]
    fn to_text_handles_empty_member_lists() {
        let mut report = sample_report();
        report.findings = vec![finding(
            FindingKind::LayerViolation,
            "lv-9",
            Severity::Info,
            &[],
            Vec::new(),
        )];
        assert!(to_text(&report).contains("  members: (none)\n"));
    }

    #[test]
    fn review_kpis_absent_keeps_the_output_free_of_a_table() {
        // The KPI block is additive: without it the text report is the
        // pre-R26 output byte for byte (the goldens above pin that).
        assert!(!to_text(&sample_report()).contains("Review KPIs:"));
    }

    #[test]
    fn review_kpis_render_one_row_per_kpi() {
        let mut report = sample_report();
        report.review_kpis = Some(ReviewKpis {
            tangledness: Some(archguard_core::Tangledness {
                tangled_nodes: 4,
                total: 10,
                quote: 0.4,
            }),
            debt: Some(archguard_core::DebtDensity {
                markers: 6,
                loc: 0,
                per_kloc: None,
                by_kind: BTreeMap::new(),
            }),
            dependencies: Some(archguard_core::DependencyInventory {
                total: 12,
                by_ecosystem: BTreeMap::new(),
                stale_count: None,
                dependencies: Vec::new(),
                notes: Vec::new(),
            }),
            health: Some(archguard_core::HealthIndex {
                score: Some(85.0),
                level: Some(HealthLevel::Green),
                trend: Some(2.5),
                parts: BTreeMap::new(),
            }),
            ..ReviewKpis::default()
        });
        let text = to_text(&report);
        assert!(text.contains("Review KPIs:\n"));
        assert!(text.contains("tangledness"));
        assert!(text.contains("0.40"));
        assert!(text.contains("6 markers, 0 loc"));
        assert!(text.contains("12 deps, stale not measured"));
        // Health is the one row carrying a real delta and ampel; the
        // column labels stay fixed, values right-aligned.
        assert!(text.contains("+2.5  ampel green"));
        // Unmeasured KPIs stay visible as "not measured", not as 0.
        assert!(text.contains("zones"));
        assert!(text.contains("not measured"));
        // Deterministic.
        assert_eq!(text, to_text(&report));
    }
}
