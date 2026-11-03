//! `archguard review-decision` (R29): record and inspect entries of the
//! decision register at `.archguard/reviews.json`. All persistence goes
//! through the engine register API (R19) — this file only maps CLI
//! arguments to [`ReviewDecision`] values and renders them.

use crate::args::{Cli, DecisionArg, Format};
use crate::output::{EXIT_OK, EXIT_USAGE, analysis_error};
use archguard_engine::core::{DecisionKind, ReviewDecision, ReviewRegister};
use archguard_engine::{load_register, upsert_decision};
use std::path::Path;

/// Unix seconds to `YYYY-MM-DDTHH:MM:SSZ` (proleptic Gregorian, UTC) —
/// the civil-from-days conversion, no date crate needed.
pub(crate) fn iso_from_unix(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        (rem / 60) % 60,
        rem % 60
    )
}

/// The three record actions share the quiet/format handling.
pub(crate) fn run(cli: &Cli, root: &Path, action: &crate::args::ReviewDecisionAction) -> i32 {
    let result = || -> Result<i32, archguard_engine::core::ArchguardError> {
        match action {
            crate::args::ReviewDecisionAction::Record {
                finding,
                decision,
                adr,
                note,
                author,
                id,
            } => {
                let kind = match decision {
                    DecisionArg::Fix => DecisionKind::Fix,
                    DecisionArg::Defer => DecisionKind::Defer,
                    DecisionArg::AcceptRisk => DecisionKind::AcceptRisk,
                };
                if kind == DecisionKind::AcceptRisk && adr.is_none() {
                    // R30's gate, enforced here (R31): with the gate on,
                    // a risk without ADR is a usage error, not a record.
                    let loaded = archguard_engine::load_config(None, root)?;
                    if loaded.config.review.adr_required_on_accept_risk {
                        eprintln!(
                            "archguard: [review] adr_required_on_accept_risk is set — pass --adr docs/adr/NNN.md"
                        );
                        return Ok(EXIT_USAGE);
                    }
                    if !cli.quiet {
                        eprintln!(
                            "archguard: warning: accept-risk without --adr — set [review] adr_required_on_accept_risk to require one"
                        );
                    }
                }
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or_default();
                let decision = ReviewDecision {
                    id: id.clone().unwrap_or_else(|| finding.clone()),
                    finding_ref: finding.clone(),
                    decision: kind,
                    adr_link: adr.as_ref().map(|p| p.display().to_string()),
                    note: note.clone().unwrap_or_default(),
                    author: author.clone().unwrap_or_else(|| "cli".to_string()),
                    ts: iso_from_unix(now),
                };
                let id = decision.id.clone();
                let finding = decision.finding_ref.clone();
                upsert_decision(root, decision)?;
                if !cli.quiet {
                    println!("recorded decision {id} for {finding}");
                }
                Ok(EXIT_OK)
            }
            crate::args::ReviewDecisionAction::List => {
                let register = load_register(root)?;
                if cli.format == Format::Json {
                    crate::output::emit(
                        &serde_json::to_string_pretty(&register).expect("serializes"),
                    );
                    return Ok(EXIT_OK);
                }
                if !cli.quiet {
                    if register.decisions.is_empty() {
                        println!("no decisions recorded");
                    }
                    for decision in sorted(&register) {
                        println!(
                            "{} · {} · {:?} · adr: {} · by {} at {}{}",
                            decision.id,
                            decision.finding_ref,
                            decision.decision,
                            decision.adr_link.as_deref().unwrap_or("—"),
                            decision.author,
                            decision.ts,
                            if decision.note.is_empty() {
                                String::new()
                            } else {
                                format!(" · {}", decision.note)
                            }
                        );
                    }
                }
                Ok(EXIT_OK)
            }
            crate::args::ReviewDecisionAction::Show { id } => {
                let register = load_register(root)?;
                let found = register
                    .decisions
                    .iter()
                    .find(|d| &d.id == id)
                    .or_else(|| register.decision_for(id));
                match found {
                    Some(decision) => {
                        if cli.format == Format::Json {
                            crate::output::emit(
                                &serde_json::to_string_pretty(decision).expect("serializes"),
                            );
                        } else if !cli.quiet {
                            println!(
                                "id: {}\nfinding: {}\ndecision: {:?}\nadr: {}\nauthor: {}\nts: {}\nnote: {}",
                                decision.id,
                                decision.finding_ref,
                                decision.decision,
                                decision.adr_link.as_deref().unwrap_or("—"),
                                decision.author,
                                decision.ts,
                                decision.note,
                            );
                        }
                        Ok(EXIT_OK)
                    }
                    None => {
                        eprintln!("archguard: no decision '{id}' recorded");
                        Ok(EXIT_USAGE)
                    }
                }
            }
        }
    };
    match result() {
        Ok(code) => code,
        Err(error) => analysis_error(error),
    }
}

/// Deterministic list order: finding first, then id.
fn sorted(register: &ReviewRegister) -> Vec<&ReviewDecision> {
    let mut decisions: Vec<&ReviewDecision> = register.decisions.iter().collect();
    decisions.sort_by(|a, b| {
        a.finding_ref
            .cmp(&b.finding_ref)
            .then_with(|| a.id.cmp(&b.id))
    });
    decisions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_civil_from_unix_is_exact() {
        assert_eq!(iso_from_unix(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_from_unix(951_868_800), "2000-03-01T00:00:00Z");
        assert_eq!(iso_from_unix(1_767_225_600), "2026-01-01T00:00:00Z");
        assert_eq!(iso_from_unix(42_611), "1970-01-01T11:50:11Z");
    }
}
