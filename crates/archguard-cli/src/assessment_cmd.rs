//! Explicit review generation and recording; the desktop app reads the same report.
use crate::args::{Cli, Format};
use crate::output::{EXIT_OK, analysis_error, emit, usage};
use crate::project_files;
use archguard_engine::core::ArchguardError;
use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) fn run(
    cli: &Cli,
    root: &Path,
    base: Option<&str>,
    decisions: Option<&Path>,
    save: bool,
    out: Option<&Path>,
) -> i32 {
    if cli.format == Format::PrComment {
        // R27: the PR comment compares against an explicit base and
        // never records anything, so it skips the review-session
        // machinery entirely.
        return crate::review_cmd::pr_comment(cli, root, base, out);
    }
    if out.is_some() {
        return usage("--out applies to --format pr-comment only");
    }
    if !matches!(cli.format, Format::Text | Format::Json | Format::Markdown) {
        return usage("review supports text, json, markdown and pr-comment");
    }
    let result = || -> Result<(), ArchguardError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| ArchguardError::Analysis(e.to_string()))?
            .as_secs();
        let (mut report, _) =
            archguard_engine::generate_review(root, cli.config.as_deref(), base, now)?;
        if let Some(path) = decisions {
            archguard_engine::import_review_decisions(root, &mut report, path, now)?;
            project_files::ensure_ignore_rules(root)?;
        }
        // Render before persistence, so a renderer error cannot publish a record.
        let text = render(cli.format, &report);
        if save {
            project_files::ensure_ignore_rules(root)?;
            archguard_engine::save_review(root, &report)?;
        }
        if !cli.quiet {
            emit(&text);
        }
        Ok(())
    };
    match result() {
        Ok(()) => EXIT_OK,
        Err(e) => analysis_error(e),
    }
}

/// The review in the chosen format: JSON, the desktop's Markdown or text.
fn render(format: Format, report: &archguard_engine::ReviewReport) -> String {
    match format {
        Format::Json => serde_json::to_string_pretty(report).expect("review serializes"),
        Format::Markdown => archguard_engine::review_markdown(report),
        _ => {
            let mut text = format!(
                "Architecture review · {}\nRevision: {}\nOpen: {} · accepted risks: {} · evidence gaps: {}\n",
                report.status,
                report.revision,
                report.summary["open"],
                report.summary["accepted_risk"],
                report.summary["evidence_gaps"]
            );
            if let Some(base) = &report.base_ref {
                text.push_str(&format!(
                    "Base: {base} (direct comparison, current policy)\n"
                ));
            }
            for finding in &report.findings {
                text.push_str(&format!(
                    "\n{} · {} · {} · {}\n  {}\n  id: {}\n",
                    finding.severity,
                    finding.category,
                    finding.change,
                    finding.disposition,
                    finding.title,
                    finding.id
                ));
                if let Some(d) = &finding.decision {
                    text.push_str(&format!(
                        "  Owner: {} · {} · expires: {}\n",
                        d.author,
                        d.note,
                        d.expires_on.as_deref().unwrap_or("—")
                    ));
                }
            }
            for gap in &report.gaps {
                text.push_str(&format!(
                    "\nEvidence gap: {} · {}\n",
                    gap["source"].as_str().unwrap_or(""),
                    gap["status"].as_str().unwrap_or("")
                ));
            }
            text.push_str("\nReview decisions do not change CI gates. Missing evidence does not establish a clean architecture.\n");
            text
        }
    }
}
