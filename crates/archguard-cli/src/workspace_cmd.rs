//! Explicit repository-wide evidence. No project configuration or source is written.
use crate::args::{Cli, Format};
use crate::output::{EXIT_OK, EXIT_USAGE, analysis_error};
use archguard_engine::{
    WorkspaceSourceReport, WorkspaceTestImpact,
    core::{GraphBuilder, WorkspaceReport},
};
use serde_json::{Value, json};
use std::{collections::HashSet, path::Path};

pub(crate) fn source_insights(source: &WorkspaceSourceReport) -> Value {
    let mut value = json!(source);
    value
        .as_object_mut()
        .expect("source report object")
        .remove("calls");
    value["call_summary"] = json!({"status":source.calls.status,"files":source.calls.files,"classes":source.calls.classes.len(),"methods":source.calls.methods.len(),"calls":source.calls.calls.len()});
    value
}

pub(crate) fn html(
    root: &Path,
    workspace: &WorkspaceReport,
    sources: Option<&WorkspaceSourceReport>,
    impact: Option<&WorkspaceTestImpact>,
) -> Result<String, archguard_engine::core::ArchguardError> {
    let empty = GraphBuilder::new().build(root.to_string_lossy().into_owned());
    let graph = sources.map(|s| &s.graph).unwrap_or(&empty);
    let topology = archguard_engine::analyze_services(root)?;
    let mut cyclic = HashSet::new();
    if let Some(sources) = sources {
        for group in &sources.cycles {
            let ids: HashSet<_> = graph
                .nodes
                .iter()
                .filter(|n| group.contains(&n.name))
                .map(|n| n.id)
                .collect();
            cyclic.extend(
                graph
                    .edges
                    .iter()
                    .filter(|e| ids.contains(&e.from) && ids.contains(&e.to))
                    .map(|e| (e.from, e.to)),
            );
        }
    }
    let insights = json!({"workspace":workspace,"workspace_sources":sources.map(source_insights),"workspace_impact":impact,"contracts":archguard_engine::read_api_contracts(root,&topology)});
    Ok(archguard_engine::report::to_html_with_context(
        graph,
        None,
        2,
        &cyclic,
        &[],
        Some("services"),
        &archguard_engine::report::HtmlContext {
            calls: sources.map(|s| &s.calls),
            insights: Some(&insights),
            services: Some(&topology),
            ..Default::default()
        },
    ))
}

pub(crate) fn run(cli: &Cli, root: &Path, sources: bool, targets: &[String]) -> i32 {
    if !matches!(cli.format, Format::Text | Format::Json | Format::Html)
        || cli.config.is_some()
        || (!sources && !targets.is_empty())
        || targets.len() > 256
        || targets
            .iter()
            .any(|t| t.is_empty() || t.len() > 4096 || t.contains('\0'))
    {
        eprintln!(
            "archguard: workspace supports text/json/html and per-project configuration; --target requires --sources (at most 256 targets)"
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
        Format::Html => match html(root, &workspace, sources.as_ref(), impact.as_ref()) {
            Ok(v) => v,
            Err(e) => return analysis_error(e),
        },
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
        println!("{output}");
    }
    EXIT_OK
}
