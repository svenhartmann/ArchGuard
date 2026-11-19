//! Explicit repository-wide evidence. No project configuration or source is written.
use crate::args::{Cli, Format};
use crate::output::{EXIT_OK, EXIT_USAGE, analysis_error};
use serde_json::json;
use std::path::Path;

pub(crate) fn run(cli: &Cli, root: &Path, sources: bool, targets: &[String]) -> i32 {
    if !matches!(cli.format, Format::Text | Format::Json)
        || cli.config.is_some()
        || (!sources && !targets.is_empty())
        || targets.len() > 256
        || targets
            .iter()
            .any(|t| t.is_empty() || t.len() > 4096 || t.contains('\0'))
    {
        eprintln!(
            "archguard: workspace supports text/json and per-project configuration; --target requires --sources (at most 256 targets)"
        );
        return EXIT_USAGE;
    }
    let workspace = match archguard_engine::analyze_workspace(root) {
        Ok(v) => v,
        Err(e) => return analysis_error(e),
    };
    let sources = if sources {
        match archguard_engine::analyze_workspace_sources(root, &workspace) {
            Ok(v) => Some(v),
            Err(e) => return analysis_error(e),
        }
    } else {
        None
    };
    let impact = sources
        .as_ref()
        .filter(|_| !targets.is_empty())
        .map(|s| archguard_engine::workspace_test_impact(s, targets));
    let output = match cli.format {
        Format::Json => serde_json::to_string_pretty(
            &json!({"workspace":workspace,"sources":sources,"impact":impact}),
        )
        .expect("workspace serializes"),
        Format::Text => {
            let mut text = format!(
                "Workspace: {}\n{} projects, {} declared local dependencies, {} cyclic groups\n",
                workspace.status,
                workspace.projects.len(),
                workspace.dependencies.len(),
                workspace.cycles.len()
            );
            for edge in &workspace.dependencies {
                text.push_str(&format!(
                    "{} -> {} [{}; {}]\n",
                    edge.from, edge.to, edge.scope, edge.kind
                ));
            }
            for unresolved in &workspace.unresolved {
                text.push_str(&format!(
                    "Unresolved {} -> {}: {}\n",
                    unresolved.project, unresolved.target, unresolved.reason
                ));
            }
            for diagnostic in &workspace.diagnostics {
                text.push_str(&format!("Note: {diagnostic}\n"));
            }
            if let Some(source) = &sources {
                text.push_str(&format!(
                    "Sources: {} · {} files · {} dependencies · {} compiler calls\n",
                    source.status,
                    source.files.len(),
                    source.graph.edges.len(),
                    source.calls.calls.len()
                ));
                for diagnostic in &source.diagnostics {
                    text.push_str(&format!("Note: {diagnostic}\n"));
                }
            }
            if let Some(impact) = &impact {
                text.push_str(&format!(
                    "Impact: {} · {} affected projects · {} recommended tests · {} unknown tests\n",
                    impact.status,
                    impact.affected_projects.len(),
                    impact.selected.len(),
                    impact.unknown_tests
                ));
            }
            for limitation in &workspace.limitations {
                text.push_str(&format!("Limit: {limitation}\n"));
            }
            text
        }
        _ => unreachable!(),
    };
    if !cli.quiet {
        outln!("{output}");
    }
    EXIT_OK
}
