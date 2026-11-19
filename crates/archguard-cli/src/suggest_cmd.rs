//! `archguard suggest`: the cycle findings' single cuts, or with `--plans`
//! up to three repair plans per cycle finding
//! ([`plan_cycle_repairs`], the desktop explorer's "Repair plans"). Each
//! plan's `ops` are SimOp cuts that `archguard simulate --ops` replays.

use archguard_engine::analysis::{
    DEFAULT_MAX_EXAMINED, RepairPlans, RepairStats, SimOp, plan_cycle_repairs,
};
use archguard_engine::core::{Finding, FindingKind, Graph, Report};
use serde_json::{Value, json};

use crate::args::{Cli, Command, Format};
use crate::output::{EXIT_OK, analysis_error, emit, to_pretty_json, usage};

/// `suggest`: the plans with `--plans`, else the findings' single cuts.
pub(crate) fn run(cli: &Cli, graph: &Graph, report: &Report) -> i32 {
    let Command::Suggest {
        cycle,
        max,
        plans: with_plans,
        ..
    } = &cli.command
    else {
        unreachable!("dispatched for suggest only");
    };
    if *with_plans {
        plans(cli, cycle.as_deref(), graph, report)
    } else {
        single_cuts(cli, cycle.as_deref(), *max, report)
    }
}

/// Handler for `suggest --plans [--cycle ID]`; an unknown id is a usage error.
fn plans(cli: &Cli, cycle: Option<&str>, graph: &Graph, report: &Report) -> i32 {
    if !matches!(cli.format, Format::Text | Format::Json) {
        return usage(format!(
            "invalid value '{}' for '--format': suggest supports text and json",
            cli.format_name()
        ));
    }
    let findings: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.kind == FindingKind::Cycle && cycle.is_none_or(|id| f.id == id))
        .collect();
    if let Some(id) = cycle
        && findings.is_empty()
    {
        return usage(format!("unknown cycle '{id}'"));
    }
    let mut out = Vec::new();
    for finding in findings {
        match plan_cycle_repairs(graph, &finding.nodes, DEFAULT_MAX_EXAMINED) {
            Ok(plans) => out.push((finding.id.as_str(), plans)),
            Err(e) => return analysis_error(format!("{}: {e}", finding.id)),
        }
    }
    if cli.quiet {
        return EXIT_OK;
    }
    if cli.format == Format::Json {
        let value: Vec<Value> = out
            .iter()
            .map(|(id, plans)| {
                let mut v = json!(plans);
                v["id"] = json!(id);
                v
            })
            .collect();
        emit(&to_pretty_json(&json!(value)));
    } else if out.is_empty() {
        outln!("no cycle findings");
    } else {
        for (id, plans) in &out {
            text(id, plans);
        }
    }
    EXIT_OK
}

fn text(id: &str, r: &RepairPlans) {
    let search = if r.search.complete {
        "search complete"
    } else {
        "search incomplete, more alternatives may exist"
    };
    let members = r.scope_names.len();
    outln!(
        "{id}: {members} members, {search} ({} candidates)",
        r.search.examined
    );
    if r.plans.is_empty() {
        outln!("  no plan found");
    }
    for (i, p) in r.plans.iter().enumerate() {
        let outcome = if p.complete {
            "cycle removed"
        } else {
            "partial improvement"
        };
        let bound = if p.reference_lower_bound { "+" } else { "" };
        outln!(
            "  option {}: {outcome}; affected files: {}, references: {}{bound}",
            i + 1,
            p.affected_files.len(),
            p.reference_count
        );
        for (op, refs) in p.ops.iter().zip(&p.op_references) {
            if let SimOp::CutEdge { from, to } = op {
                outln!("    cut {from} -> {to} (references: {refs})");
            }
        }
        outln!("    scope:  {}", stats(&r.before, &p.after));
        outln!("    source: {}", stats(&r.source_before, &p.source_after));
    }
    if r.ranking_lower_bound {
        outln!("  order uncertain: some reference totals are lower bounds");
    }
}

fn stats(before: &RepairStats, after: &RepairStats) -> String {
    format!(
        "cyclic nodes {} -> {}, cyclic edges {} -> {}, largest group {} -> {}",
        before.cyclic_nodes,
        after.cyclic_nodes,
        before.cyclic_edges,
        after.cyclic_edges,
        before.largest_scc,
        after.largest_scc
    )
}

/// `suggest`: the [`SuggestedCut`]s already carried by the cycle
/// findings (Kap. 7). `--cycle <ID>` selects one finding (an unknown id
/// is a usage error), `--max N` caps the cuts per cycle.
fn single_cuts(cli: &Cli, cycle: Option<&str>, max: Option<usize>, report: &Report) -> i32 {
    if matches!(
        cli.format,
        Format::Mermaid
            | Format::Sarif
            | Format::Dot
            | Format::Graphml
            | Format::Gexf
            | Format::Table
    ) {
        return usage(format!(
            "invalid value '{}' for '--format': suggest supports text and json",
            cli.format_name()
        ));
    }
    let findings: Vec<&Finding> = report
        .findings
        .iter()
        .filter(|finding| finding.kind == FindingKind::Cycle)
        .filter(|finding| cycle.is_none_or(|id| finding.id == id))
        .collect();
    if let Some(id) = cycle
        && findings.is_empty()
    {
        return usage(format!("unknown cycle '{id}'"));
    }
    if cli.quiet {
        return EXIT_OK;
    }
    if cli.format == Format::Json {
        let out: Vec<Value> = findings
            .iter()
            .map(|finding| {
                let cuts: Vec<Value> = capped_cuts(finding, max)
                    .iter()
                    .map(|cut| {
                        json!({
                            "from": cut.from,
                            "to": cut.to,
                            "score": cut.score,
                            "reason": cut.reason,
                        })
                    })
                    .collect();
                json!({ "id": finding.id, "cuts": cuts })
            })
            .collect();
        emit(&to_pretty_json(&json!(out)));
    } else {
        for finding in &findings {
            outln!("{}:", finding.id);
            for cut in capped_cuts(finding, max) {
                outln!(
                    "  {} -> {} (score {:.2}): {}",
                    cut.from,
                    cut.to,
                    cut.score,
                    cut.reason
                );
            }
        }
        if findings.is_empty() {
            outln!("no cycle findings");
        }
    }
    EXIT_OK
}

/// The first `max` suggested cuts of a finding (all of them without a cap).
fn capped_cuts(finding: &Finding, max: Option<usize>) -> &[archguard_engine::core::SuggestedCut] {
    match max {
        Some(max) => &finding.suggested_cuts[..finding.suggested_cuts.len().min(max)],
        None => &finding.suggested_cuts,
    }
}
