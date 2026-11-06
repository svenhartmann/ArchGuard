//! `archguard evaluate`: agreement of findings with an expert assessment
//! (ADR 0004). Not a gate: exit 0 whatever the numbers are.
use crate::args::{Cli, Format};
use crate::output::{EXIT_OK, analysis_error, emit, usage};
use archguard_engine::{Evaluation, Ratio};
use std::fmt::Write;
use std::path::Path;

pub(crate) fn run(cli: &Cli, root: &Path, assessment: Option<&Path>, init: bool) -> i32 {
    if !matches!(cli.format, Format::Text | Format::Json) {
        return usage("evaluate supports text and json");
    }
    if init {
        return match archguard_engine::init_assessment(root, cli.config.as_deref(), assessment) {
            Ok(path) => {
                if !cli.quiet {
                    emit(&format!(
                        "wrote {}; record expected problems and verdicts there",
                        path.display()
                    ));
                }
                EXIT_OK
            }
            Err(err) => analysis_error(err),
        };
    }
    match archguard_engine::evaluate_project(root, cli.config.as_deref(), assessment) {
        Ok(evaluation) => {
            for warning in &evaluation.warnings {
                eprintln!("archguard: warning: {warning}");
            }
            if !cli.quiet {
                emit(&match cli.format {
                    Format::Json => {
                        serde_json::to_string_pretty(&evaluation).expect("evaluation serializes")
                    }
                    _ => text(&evaluation),
                });
            }
            EXIT_OK
        }
        Err(err) => analysis_error(err),
    }
}

fn ratio(name: &str, r: &Ratio, empty: &str) -> String {
    match r.value {
        Some(v) => format!("{name}: {}/{} = {v:.2}\n", r.numerator, r.denominator),
        None => format!("{name}: not measured ({empty})\n"),
    }
}

fn text(e: &Evaluation) -> String {
    let a = &e.assessment;
    let mut out = format!(
        "assessment: {} by {} on {}, revision {} (HEAD {})\n",
        e.assessment_path,
        a.assessor,
        a.date,
        a.revision,
        e.head.as_deref().unwrap_or("unknown")
    );
    out.push_str(&ratio("recall", &e.recall, "no expected problems"));
    out.push_str(&ratio(
        "precision",
        &e.precision,
        "no judged current findings",
    ));
    if !e.missed.is_empty() {
        out.push_str("missed expected problems:\n");
        for m in &e.missed {
            let _ = writeln!(out, "  {} ({}): {}", m.id, m.kind, m.reason);
        }
    }
    if !e.unjudged.is_empty() {
        let _ = writeln!(
            out,
            "unjudged findings ({} of {}):",
            e.unjudged.len(),
            e.findings_total
        );
        for f in &e.unjudged {
            let _ = writeln!(
                out,
                "  {} {} {}: {}",
                f.id,
                archguard_engine::core::finding_kind_name(f.kind),
                f.report_id,
                f.nodes.join(", ")
            );
        }
    }
    if !e.stale.is_empty() {
        out.push_str("stale judged ids (no current finding):\n");
        for id in &e.stale {
            let _ = writeln!(out, "  {id}");
        }
    }
    out
}
