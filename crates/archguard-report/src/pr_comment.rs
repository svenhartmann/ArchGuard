//! PR-comment Markdown generator.
//!
//! [`render_pr_comment`] turns a base/current pair of [`ReviewReport`]s
//! plus the decision [`ReviewRegister`] into the Markdown body of an
//! architecture PR comment: a summary of the KPI deltas, the findings
//! that appeared since base, the findings that disappeared (fixed), and
//! the accepted risks with their ADR links.
//!
//! A finding is a [`ModuleReview`] reported with `High` or `Medium`
//! priority — `Low` entries are background, not PR-review material.
//! Findings are keyed by node name (stable across runs, unlike
//! `node_id`). Evidence is the node name plus its review reasons; the
//! report carries no per-finding line numbers, so none are invented —
//! a file-scope finding shows its file, a module-scope finding its
//! module. Every list is sorted, so two runs render byte-identical
//! Markdown.

use std::collections::BTreeSet;

use archguard_core::{DecisionKind, ModuleReview, ReviewPriority, ReviewRegister, ReviewReport};

/// Whether a reviewed node is PR-comment material.
fn is_finding(node: &ModuleReview) -> bool {
    matches!(node.priority, ReviewPriority::High | ReviewPriority::Medium)
}

/// Findings of a report keyed by (stable) node name.
fn findings(report: &ReviewReport) -> BTreeSet<&str> {
    report
        .nodes
        .iter()
        .filter(|node| is_finding(node))
        .map(|node| node.name.as_str())
        .collect()
}

/// One bullet per finding: the node name as the file/module reference
/// plus its recorded reasons, both taken verbatim from the report.
fn finding_lines(report: &ReviewReport, names: &BTreeSet<&str>) -> Vec<String> {
    report
        .nodes
        .iter()
        .filter(|node| is_finding(node) && names.contains(node.name.as_str()))
        .map(|node| {
            let mut line = format!("- `{}`", node.name);
            if let Some(lines) = node.lines {
                line.push_str(&format!(" ({lines} lines)"));
            }
            if !node.reasons.is_empty() {
                line.push_str(&format!(" — {}", node.reasons.join("; ")));
            }
            line
        })
        .collect()
}

/// `label: base → current (+delta)`; integers only, so the delta is exact.
fn delta(label: &str, base: u32, current: u32) -> String {
    match current as i64 - base as i64 {
        0 => format!("- {label}: {base}"),
        d if d > 0 => format!("- {label}: {base} → {current} (+{d})"),
        d => format!("- {label}: {base} → {current} ({d})"),
    }
}

/// `label: base → current (±0.00)` for the float summary fields.
fn delta_f64(label: &str, base: f64, current: f64) -> String {
    let d = current - base;
    if d.abs() < f64::EPSILON {
        format!("- {label}: {base:.2}")
    } else {
        format!("- {label}: {base:.2} → {current:.2} ({d:+.2})")
    }
}

/// Render the Markdown body of an architecture PR comment comparing
/// `base` against `current`, annotated with the accepted risks from
/// `register`. Deterministic output: every list is sorted.
pub fn render_pr_comment(
    base: &ReviewReport,
    current: &ReviewReport,
    register: &ReviewRegister,
) -> String {
    let base_findings = findings(base);
    let current_findings = findings(current);
    let new: BTreeSet<&str> = current_findings
        .difference(&base_findings)
        .copied()
        .collect();
    let fixed: BTreeSet<&str> = base_findings
        .difference(&current_findings)
        .copied()
        .collect();

    let mut out = String::from("## Architektur-Review\n\n### Zusammenfassung\n\n");
    for line in [
        delta(
            "Priorisierte Befunde",
            base.summary.high_priority,
            current.summary.high_priority,
        ),
        delta(
            "Zyklische Knoten",
            base.summary.cyclic_nodes,
            current.summary.cyclic_nodes,
        ),
        delta(
            "Größte SCC",
            base.summary.largest_scc,
            current.summary.largest_scc,
        ),
        delta_f64("ACD", base.summary.acd, current.summary.acd),
        delta_f64("NCCD", base.summary.nccd, current.summary.nccd),
    ] {
        out.push_str(&line);
        out.push('\n');
    }

    out.push_str("\n### Neue Befunde seit base\n\n");
    if new.is_empty() {
        out.push_str("Keine neuen Befunde gegenüber base.\n");
    } else {
        for line in finding_lines(current, &new) {
            out.push_str(&line);
            out.push('\n');
        }
    }

    out.push_str("\n### Behoben seit base\n\n");
    if fixed.is_empty() {
        out.push_str("Keine Befunde gegenüber base verschwunden.\n");
    } else {
        for line in finding_lines(base, &fixed) {
            out.push_str(&line);
            out.push('\n');
        }
    }

    out.push_str("\n### Akzeptierte Risiken\n\n");
    let mut accepted: Vec<&archguard_core::ReviewDecision> = register
        .decisions
        .iter()
        .filter(|d| d.decision == DecisionKind::AcceptRisk)
        .collect();
    accepted.sort_by(|a, b| {
        a.finding_ref
            .cmp(&b.finding_ref)
            .then_with(|| a.id.cmp(&b.id))
    });
    if accepted.is_empty() {
        out.push_str("Keine akzeptierten Risiken registriert.\n");
    } else {
        for decision in accepted {
            let adr = match decision.adr_link.as_deref() {
                Some(link) if !link.is_empty() => format!("[ADR]({link})"),
                _ => "(kein ADR-Link)".to_string(),
            };
            let note = if decision.note.is_empty() {
                String::new()
            } else {
                format!(" — {}", decision.note)
            };
            out.push_str(&format!("- `{}` {}{note}\n", decision.finding_ref, adr));
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{
        GitEvidence, NodeId, NodeKind, ReviewDecision, ReviewScore, ReviewSummary,
    };

    fn node(name: &str, priority: ReviewPriority, reasons: &[&str]) -> ModuleReview {
        ModuleReview {
            node_id: NodeId(0),
            source_id: None,
            name: name.to_string(),
            kind: NodeKind::File,
            rank: 0,
            score: 42,
            priority,
            fan_in: 2,
            fan_out: 1,
            instability: 0.33,
            scc_size: 2,
            dependencies: 3,
            dependents: 5,
            file_count: 1,
            mapped_files: 1,
            lines: Some(120),
            changes: Some(4),
            entry_point: false,
            orphan: None,
            components: ReviewScore {
                cycle: 30,
                coupling: 4,
                impact: 5,
                churn: 0,
            },
            reasons: reasons.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// Mirrors the ts-cyclic fixture: two files in one cycle, both
    /// reported high priority.
    fn ts_cyclic(names: &[&str]) -> ReviewReport {
        ReviewReport {
            schema_version: 1,
            score_version: "v1".to_string(),
            summary: ReviewSummary {
                scope_nodes: names.len() as u32,
                scope_edges: names.len() as u32,
                ccd: 2,
                acd: 1.0,
                nccd: 0.5,
                cyclic_nodes: names.len() as u32,
                largest_scc: 2,
                high_priority: names.len() as u32,
                orphan_candidates: 0,
                source_files: names.len() as u32,
                mapped_files: names.len() as u32,
                lines: Some(240),
                changes: Some(8),
            },
            nodes: names
                .iter()
                .map(|n| {
                    node(
                        n,
                        ReviewPriority::High,
                        &[
                            "Cyclic group: 2 nodes (+30)",
                            "Direct coupling: 2 incoming + 1 outgoing (+4)",
                        ],
                    )
                })
                .collect(),
            git: GitEvidence::default(),
            entry_points: Vec::new(),
            entry_point_ids: Vec::new(),
            limitations: Vec::new(),
        }
    }

    #[test]
    fn golden_full_diff_new_fixed_and_accepted() {
        let base = ts_cyclic(&["src/a.ts", "src/b.ts"]);
        let mut current = ts_cyclic(&["src/a.ts", "src/c.ts"]);
        current.summary.high_priority = 2;
        let register = ReviewRegister {
            version: 1,
            decisions: vec![
                ReviewDecision {
                    id: "d-2".to_string(),
                    finding_ref: "scc-2".to_string(),
                    decision: DecisionKind::AcceptRisk,
                    adr_link: None,
                    note: String::new(),
                    author: "reviewer".to_string(),
                    ts: "2026-01-01T00:00:00Z".to_string(),
                },
                ReviewDecision {
                    id: "d-1".to_string(),
                    finding_ref: "scc-1".to_string(),
                    decision: DecisionKind::AcceptRisk,
                    adr_link: Some("docs/adr/004.md".to_string()),
                    note: "bewusst in Kauf genommen".to_string(),
                    author: "reviewer".to_string(),
                    ts: "2026-01-01T00:00:00Z".to_string(),
                },
                ReviewDecision {
                    id: "d-3".to_string(),
                    finding_ref: "scc-3".to_string(),
                    decision: DecisionKind::Fix,
                    adr_link: None,
                    note: String::new(),
                    author: "reviewer".to_string(),
                    ts: "2026-01-01T00:00:00Z".to_string(),
                },
            ],
        };

        let rendered = render_pr_comment(&base, &current, &register);
        assert_eq!(
            rendered,
            "## Architektur-Review\n\
             \n\
             ### Zusammenfassung\n\
             \n\
             - Priorisierte Befunde: 2\n\
             - Zyklische Knoten: 2\n\
             - Größte SCC: 2\n\
             - ACD: 1.00\n\
             - NCCD: 0.50\n\
             \n\
             ### Neue Befunde seit base\n\
             \n\
             - `src/c.ts` (120 lines) — Cyclic group: 2 nodes (+30); Direct coupling: 2 incoming + 1 outgoing (+4)\n\
             \n\
             ### Behoben seit base\n\
             \n\
             - `src/b.ts` (120 lines) — Cyclic group: 2 nodes (+30); Direct coupling: 2 incoming + 1 outgoing (+4)\n\
             \n\
             ### Akzeptierte Risiken\n\
             \n\
             - `scc-1` [ADR](docs/adr/004.md) — bewusst in Kauf genommen\n\
             - `scc-2` (kein ADR-Link)\n"
        );
        // Deterministic: a second render is byte-identical.
        assert_eq!(rendered, render_pr_comment(&base, &current, &register));
    }

    #[test]
    fn empty_diff_reports_no_new_findings() {
        let report = ts_cyclic(&["src/a.ts", "src/b.ts"]);
        let rendered = render_pr_comment(&report, &report, &ReviewRegister::default());
        assert!(rendered.contains("Keine neuen Befunde gegenüber base."));
        assert!(rendered.contains("Keine Befunde gegenüber base verschwunden."));
        assert!(rendered.contains("Keine akzeptierten Risiken registriert."));
        // No deltas at all → the summary lists plain values without arrows.
        assert!(rendered.contains("- Priorisierte Befunde: 2\n"));
        assert!(!rendered.contains('→'));
    }

    #[test]
    fn low_priority_nodes_are_not_pr_material() {
        let base = ts_cyclic(&["src/a.ts"]);
        let mut current = ts_cyclic(&["src/a.ts", "src/new.ts"]);
        for n in &mut current.nodes {
            if n.name == "src/new.ts" {
                n.priority = ReviewPriority::Low;
            }
        }
        let rendered = render_pr_comment(&base, &current, &ReviewRegister::default());
        assert!(rendered.contains("Keine neuen Befunde gegenüber base."));
        assert!(!rendered.contains("src/new.ts"));
    }

    #[test]
    fn summary_shows_signed_deltas() {
        let base = ts_cyclic(&["src/a.ts", "src/b.ts"]);
        let mut current = ts_cyclic(&["src/a.ts"]);
        current.summary.high_priority = 1;
        current.summary.acd = 1.5;
        let rendered = render_pr_comment(&base, &current, &ReviewRegister::default());
        assert!(rendered.contains("- Priorisierte Befunde: 2 → 1 (-1)\n"));
        assert!(rendered.contains("- ACD: 1.00 → 1.50 (+0.50)\n"));
    }
}
