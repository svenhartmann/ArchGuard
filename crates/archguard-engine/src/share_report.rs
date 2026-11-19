//! The shareable report of one analysis run (`archguard report`, desktop
//! File → Export → Report…): gathers what [`archguard_report::render_share_report`]
//! writes, so the CLI and the desktop write the same file.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use archguard_analysis::analyze_review;
use archguard_core::{ArchguardError, Cut, aggregate};
use archguard_report::{ShareReport, render_share_report};

use crate::effective::AnalysisRun;
use crate::quality_findings::read_quality_findings;
use crate::security::read_security;

/// The report of `run` over the project at `root`, as Markdown.
pub fn share_report(root: &Path, run: &AnalysisRun) -> Result<String, ArchguardError> {
    let doc = crate::arc42::arc42_document(root, &run.config, &run.graph, &run.report)?;
    let view = aggregate(
        &run.graph,
        &Cut::at_depth(&run.graph, run.config.modules.depth as usize),
    );
    let named = (&run.graph, &run.source_names);
    // Review and services are optional parts; the report says when they are missing.
    let review = crate::review_sources::collect_review_sources(
        root,
        &run.config,
        named,
        &run.config.review.git_since,
    )
    .and_then(|s| analyze_review(&run.graph, &view, &s, &run.config.review.entry_points))
    .ok();
    let services = crate::services::analyze_services(root).ok();
    let project = root.file_name().map_or_else(
        || root.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let revision = crate::snapshot::git_sha(root);
    // Not `doc.scope`: that is the absolute local path, nothing to hand on.
    let scope = format!(
        "source roots {}, modules at depth {}",
        run.config.project.source_roots.join(", "),
        run.config.modules.depth
    );
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    Ok(render_share_report(&ShareReport {
        project: &project,
        revision: revision.as_deref(),
        date: &crate::review_register::iso_from_unix(now),
        scope: &scope,
        report: &run.report,
        review: review.as_ref(),
        services: services.as_ref(),
        scans: &scans(root, run),
        diagrams: &doc.diagrams,
    }))
}

/// Summaries of the imported quality and security scans that exist.
fn scans(root: &Path, run: &AnalysisRun) -> Vec<(String, Vec<String>)> {
    let mut out = vec![];
    let quality = read_quality_findings(root, &run.config, (&run.graph, &run.source_names));
    if quality.status != "not_imported" {
        let open = quality.findings.iter().filter(|f| !f.suppressed).count();
        out.push((
            "Code quality".into(),
            vec![
                format!("Status: {}", quality.status),
                format!(
                    "{open} open findings of {} from {}",
                    quality.findings.len(),
                    quality.tools.join(", ")
                ),
            ],
        ));
    }
    let security = read_security(root, &run.config);
    if security.status != "not_scanned" {
        let mut lines = vec![format!(
            "Status: {}, {} packages scanned by {}",
            security.status, security.packages_scanned, security.scanner
        )];
        for severity in ["CRITICAL", "HIGH", "MEDIUM", "LOW"] {
            let n = security
                .findings
                .iter()
                .filter(|f| f.severity.eq_ignore_ascii_case(severity))
                .count();
            if n > 0 {
                lines.push(format!("{n} {} vulnerabilities", severity.to_lowercase()));
            }
        }
        out.push(("Security".into(), lines));
    }
    out
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::*;
    use crate::config::load_config;
    use crate::effective::analyze_run_quiet;
    use crate::pipeline::Level;

    fn copy_dir(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for entry in fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_dir(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), &target).unwrap();
            }
        }
    }

    /// The report of a temp copy of `fixtures/<name>`.
    fn report_of(name: &str) -> (tempfile::TempDir, PathBuf, String) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join(name);
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures")
                .join(name),
            &root,
        );
        let loaded = load_config(None, &root).expect("config");
        let run = analyze_run_quiet(&root, &loaded, Level::Module).expect("analysis");
        let text = share_report(&root, &run).expect("report");
        (dir, root, text)
    }

    /// Layers with a deliberate upward import: a failed gate, no file cycle,
    /// review priorities and the shown diagrams; no local path.
    #[test]
    fn the_report_of_a_layered_project_has_every_section() {
        let (_dir, root, text) = report_of("py-layers");
        for section in [
            "# ArchGuard report: py-layers",
            "| Revision | not a git repository |",
            "## Health and KPIs",
            "Gate status: **fail**",
            "## Gate findings",
            "layer_violation",
            "## Cycles and suggested cuts",
            "No dependency cycles.",
            "| Scope | source roots src, modules at depth",
            "## Review priorities",
            "## Services",
            "## Diagrams",
            "```mermaid",
            "<summary>Text drawing</summary>",
        ] {
            assert!(text.contains(section), "{section}:\n{text}");
        }
        assert!(!text.contains("<script"), "no script");
        assert!(
            !text.contains(&*root.to_string_lossy()),
            "no local path:\n{text}"
        );
        assert!(!text.contains("## Security"), "no scan imported:\n{text}");
    }

    /// Compose services appear with their file and dependencies.
    #[test]
    fn the_report_of_a_compose_project_lists_its_services() {
        let (_dir, _, text) = report_of("compose-services");
        for service in ["`orders`", "`billing`", "`shipping`", "compose.yaml"] {
            assert!(text.contains(service), "{service}:\n{text}");
        }
        assert!(
            !text.contains("No services or deployable projects found."),
            "{text}"
        );
    }
}
