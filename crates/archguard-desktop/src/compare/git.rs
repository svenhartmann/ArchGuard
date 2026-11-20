//! The refs the user picks from and the comparison run, through the
//! engine's read-only Git comparisons (`archguard diff --from/--to` and
//! `--staged`).

use std::path::Path;

use archguard_engine::core::{ArchitectureDiffDetails, StagedArchitectureDiff};

use super::diff::Comparison;

/// What the base is compared with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Ref(String),
    /// The Git index (`archguard diff --staged`).
    Staged,
}

/// The local branches and the preselected pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refs {
    pub branches: Vec<String>,
    pub from: usize,
    /// `None`: the Git index.
    pub to: Option<usize>,
}

/// The local branches of the repository around `root`.
pub fn refs(root: &Path) -> Result<Refs, String> {
    let listed = archguard_engine::list_git_branches(root).map_err(|e| e.to_string())?;
    let branches = listed
        .branches
        .into_iter()
        .filter(|b| b.reference.starts_with("refs/heads/"))
        .map(|b| b.name)
        .collect();
    let current = listed
        .current
        .as_deref()
        .and_then(|c| c.strip_prefix("refs/heads/"));
    Ok(preselect(branches, current))
}

/// main or master (else the current branch) as the base; the current
/// branch as the target, or the index when that is the base itself.
pub fn preselect(branches: Vec<String>, current: Option<&str>) -> Refs {
    let at = |name: &str| branches.iter().position(|b| b == name);
    let now = current.and_then(at);
    let from = at("main").or_else(|| at("master")).or(now).unwrap_or(0);
    let to = now.filter(|&i| i != from);
    Refs { branches, from, to }
}

/// Compare `from` with `to` in the repository around `root`; blocks for
/// two analyses, so it runs on a worker.
pub fn run(root: &Path, from: &str, to: &Target) -> Result<Comparison, String> {
    match to {
        Target::Ref(to) => archguard_engine::compare_branches(root, from, to, None)
            .map(|d| Comparison::build(&d.details, from, to)),
        // The view names the index itself (translated), so its label is empty.
        Target::Staged => archguard_engine::compare_staged(root, from, None)
            .map(|d| Comparison::build(&staged_details(d), from, "")),
    }
    .map_err(|e| e.to_string())
}

fn staged_details(d: StagedArchitectureDiff) -> ArchitectureDiffDetails {
    ArchitectureDiffDetails {
        module_depth: d.module_depth,
        base_graph: d.base_graph,
        target_graph: d.staged_graph,
        base_summary: d.base_summary,
        target_summary: d.staged_summary,
        added_nodes: d.added_nodes,
        removed_nodes: d.removed_nodes,
        added_edges: d.added_edges,
        removed_edges: d.removed_edges,
        changed_edges: d.changed_edges,
        new_findings: d.new_findings,
        resolved_findings: d.resolved_findings,
    }
}
