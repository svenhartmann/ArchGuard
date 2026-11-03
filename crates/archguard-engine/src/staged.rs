//! Read-only Git snapshot comparisons: two refs or a ref versus the index.
use crate::comparison::{
    MAX_TREE_BYTES, bounded_git, comparison_findings, extract_snapshot, extract_tree, git, identity,
};
use archguard_core::{
    ArchguardError, ArchitectureDiffDetails, ArchitectureEdgeChange, BranchArchitectureDiff, Graph,
    Ref, StagedArchitectureDiff,
};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    path::{Path, PathBuf},
};

fn invalid(message: impl std::fmt::Display) -> ArchguardError {
    ArchguardError::Config(format!("architecture comparison: {message}"))
}
fn index(repo: &Path, commit: &str) -> Result<Vec<u8>, ArchguardError> {
    let records = bounded_git(
        repo,
        &["ls-files".into(), "--stage".into(), "-z".into()],
        MAX_TREE_BYTES,
    )?;
    // `ls-files --stage` includes intent-to-add entries as empty blobs. Those
    // entries are absent from the staged tree, even though they have a path.
    let names = |visibility: &str| -> Result<BTreeSet<Vec<u8>>, ArchguardError> {
        let bytes = bounded_git(
            repo,
            &[
                "diff".into(),
                "--cached".into(),
                visibility.into(),
                "--name-only".into(),
                "--no-renames".into(),
                "--no-ext-diff".into(),
                "--no-textconv".into(),
                "-z".into(),
                commit.into(),
                "--".into(),
            ],
            MAX_TREE_BYTES,
        )?;
        Ok(bytes
            .split(|b| *b == 0)
            .filter(|p| !p.is_empty())
            .map(|p| p.to_vec())
            .collect())
    };
    let visible = names("--ita-visible-in-index")?;
    let invisible = names("--ita-invisible-in-index")?;
    let pending: BTreeSet<_> = visible.difference(&invisible).collect();
    let mut filtered = vec![];
    for record in records.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let path = record
            .splitn(2, |b| *b == b'\t')
            .nth(1)
            .ok_or_else(|| invalid("invalid index record"))?;
        if pending.iter().any(|p| p.as_slice() == path) {
            continue;
        }
        filtered.extend_from_slice(record);
        filtered.push(0);
    }
    Ok(filtered)
}
fn index_tree(records: &[u8]) -> Result<Vec<u8>, ArchguardError> {
    let mut tree = Vec::new();
    for record in records.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let (metadata, path) = std::str::from_utf8(record)
            .map_err(invalid)?
            .split_once('\t')
            .ok_or_else(|| invalid("invalid index record"))?;
        let fields: Vec<_> = metadata.split_whitespace().collect();
        if fields.len() != 3 || fields[2] != "0" {
            return Err(invalid(
                "unmerged index: resolve staged conflicts before comparing",
            ));
        }
        tree.extend_from_slice(format!("{} blob {}\t{}\0", fields[0], fields[1], path).as_bytes());
    }
    Ok(tree)
}
fn edges(graph: &Graph) -> BTreeMap<(String, String), Vec<Ref>> {
    let names: BTreeMap<_, _> = graph.nodes.iter().map(|n| (n.id, &n.name)).collect();
    graph
        .edges
        .iter()
        .map(|e| {
            (
                (names[&e.from].clone(), names[&e.to].clone()),
                e.refs.clone(),
            )
        })
        .collect()
}
fn repository(root: &Path) -> Result<(PathBuf, PathBuf), ArchguardError> {
    let root = root.canonicalize()?;
    let repository =
        String::from_utf8(git(&root, &["rev-parse", "--show-toplevel"])?).map_err(invalid)?;
    let repo = Path::new(repository.trim_end_matches('\n')).canonicalize()?;
    Ok((root, repo))
}
fn resolve_commit(repo: &Path, reference: &str) -> Result<String, ArchguardError> {
    let commit = String::from_utf8(git(
        repo,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{reference}^{{commit}}"),
        ],
    )?)
    .map_err(invalid)?
    .trim()
    .to_string();
    if !matches!(commit.len(), 40 | 64) || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid("invalid commit"));
    }
    Ok(commit)
}
#[derive(Debug, serde::Serialize)]
pub struct GitBranch {
    pub name: String,
    pub reference: String,
    pub commit: String,
}
#[derive(Debug, serde::Serialize)]
pub struct GitBranches {
    pub current: Option<String>,
    pub branches: Vec<GitBranch>,
    pub truncated: bool,
}
/// Local branch and remote-tracking refs only; never fetches or changes Git state.
pub fn list_git_branches(root: &Path) -> Result<GitBranches, ArchguardError> {
    let (_, repo) = repository(root)?;
    let bytes = bounded_git(
        &repo,
        &[
            "for-each-ref".into(),
            "--sort=refname".into(),
            "--count=2001".into(),
            "--format=%(refname)%00%(objectname)%00%(symref)".into(),
            "refs/heads/".into(),
            "refs/remotes/".into(),
        ],
        MAX_TREE_BYTES,
    )?;
    let text = String::from_utf8(bytes).map_err(invalid)?;
    let mut branches = Vec::new();
    for line in text.lines() {
        let fields: Vec<_> = line.split('\0').collect();
        if fields.len() != 3 || !fields[2].is_empty() {
            continue;
        }
        let reference = fields[0];
        let name = reference
            .strip_prefix("refs/heads/")
            .or_else(|| reference.strip_prefix("refs/remotes/"));
        if let Some(name) = name {
            branches.push(GitBranch {
                name: name.into(),
                reference: reference.into(),
                commit: fields[1].into(),
            });
        }
    }
    let truncated = text.lines().count() > 2000;
    branches.truncate(2000);
    let current = git(&repo, &["symbolic-ref", "--quiet", "HEAD"])
        .ok()
        .and_then(|b| String::from_utf8(b).ok())
        .map(|s| s.trim_end_matches('\n').to_owned());
    Ok(GitBranches {
        current,
        branches,
        truncated,
    })
}
fn source_patch(repo: &Path, relative: &Path, commits: &[&str]) -> Result<String, ArchguardError> {
    let mut args: Vec<OsString> = [
        "--literal-pathspecs",
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        "--unified=3",
    ]
    .into_iter()
    .map(Into::into)
    .collect();
    args.extend(commits.iter().map(OsString::from));
    args.push("--".into());
    args.push(if relative.as_os_str().is_empty() {
        ".".into()
    } else {
        relative.as_os_str().to_owned()
    });
    let patch = bounded_git(repo, &args, MAX_TREE_BYTES)?;
    Ok(String::from_utf8_lossy(&patch).into_owned())
}
fn limitations(policy: &str) -> Vec<String> {
    vec![policy.into(),
        "Renames are represented by graph path identity, as removed and added nodes. Changed edges include changes to their source evidence.".into(),
        "Git symlinks, submodules and analysis caches are excluded. Missing historical compiler dependencies can reduce semantic coverage; compare the coverage on both sides.".into()]
}
fn compare_snapshots(
    root: &Path,
    relative: &Path,
    previous: &Path,
    target: &Path,
    explicit_config: Option<&Path>,
) -> Result<ArchitectureDiffDetails, ArchguardError> {
    for snapshot in [previous, target] {
        git(snapshot, &["init", "--quiet", "--template=", "."])?;
        std::fs::create_dir_all(snapshot.join(relative))?;
    }
    let target_root = target.join(relative);
    let base_root = previous.join(relative);
    let loaded = crate::config::load_config(explicit_config, &target_root)?;
    let run =
        crate::effective::effective_run(&target_root, &loaded, crate::pipeline::Level::Module)?;
    // Freeze the target policy and derived zoom/layers for the baseline.
    let files = crate::pipeline::walk_files(&base_root, &run.config)?;
    let (mut base_graph, _) = crate::pipeline::build_graph(&base_root, &files, &run.config)?;
    crate::semantic_calls::integrate_graph(&base_root, &files, &run.config, &mut base_graph)?;
    let (base_report, _) = crate::effective::build_effective_report(
        &base_root,
        &base_graph,
        &run.config,
        crate::pipeline::Level::Module,
    )?;
    let old_findings = comparison_findings(&base_graph, &base_report, &run.config)?;
    let new_findings = comparison_findings(&run.graph, &run.report, &run.config)?;
    let old_ids: BTreeSet<_> = old_findings.iter().map(identity).collect();
    let new_ids: BTreeSet<_> = new_findings.iter().map(identity).collect();
    let before = edges(&base_graph);
    let after = edges(&run.graph);
    let mut added_edges = vec![];
    let mut removed_edges = vec![];
    let mut changed_edges = vec![];
    let keys: BTreeSet<_> = before.keys().chain(after.keys()).collect();
    for (from, to) in keys {
        let key = (from.clone(), to.clone());
        let old = before.get(&key);
        let new = after.get(&key);
        if old == new {
            continue;
        }
        let edge = ArchitectureEdgeChange {
            from: from.clone(),
            to: to.clone(),
            before: old.cloned().unwrap_or_default(),
            after: new.cloned().unwrap_or_default(),
        };
        match (old, new) {
            (None, _) => added_edges.push(edge),
            (_, None) => removed_edges.push(edge),
            _ => changed_edges.push(edge),
        }
    }
    let old_nodes: BTreeSet<_> = base_graph.nodes.iter().map(|n| n.name.clone()).collect();
    let new_nodes: BTreeSet<_> = run.graph.nodes.iter().map(|n| n.name.clone()).collect();
    let mut target_graph = run.graph;
    base_graph.root = root.display().to_string();
    target_graph.root = root.display().to_string();
    Ok(ArchitectureDiffDetails {
        module_depth: run.config.modules.depth,
        base_graph,
        target_graph,
        base_summary: base_report.summary,
        target_summary: run.report.summary,
        added_nodes: new_nodes.difference(&old_nodes).cloned().collect(),
        removed_nodes: old_nodes.difference(&new_nodes).cloned().collect(),
        added_edges,
        removed_edges,
        changed_edges,
        new_findings: new_findings
            .into_iter()
            .filter(|f| !old_ids.contains(&identity(f)))
            .collect(),
        resolved_findings: old_findings
            .into_iter()
            .filter(|f| !new_ids.contains(&identity(f)))
            .collect(),
    })
}
/// Compare the exact tips of two refs, pinned to commit IDs before extraction.
/// Neither HEAD, the index nor worktree sources are used as comparison inputs.
pub fn compare_branches(
    root: &Path,
    base: &str,
    target: &str,
    explicit_config: Option<&Path>,
) -> Result<BranchArchitectureDiff, ArchguardError> {
    let (root, repo) = repository(root)?;
    let relative = root.strip_prefix(&repo).map_err(invalid)?;
    let base_commit = resolve_commit(&repo, base)?;
    let target_commit = resolve_commit(&repo, target)?;
    let previous = tempfile::tempdir()?;
    let next = tempfile::tempdir()?;
    extract_snapshot(&repo, &base_commit, relative, previous.path())?;
    extract_snapshot(&repo, &target_commit, relative, next.path())?;
    let details = compare_snapshots(
        &root,
        relative,
        previous.path(),
        next.path(),
        explicit_config,
    )?;
    let source_diff = source_patch(&repo, relative, &[&base_commit, &target_commit])?;
    Ok(BranchArchitectureDiff {
        definition: "git-refs-architecture/v1".into(),
        target: "ref".into(),
        base_ref: base.into(),
        base_commit,
        target_ref: target.into(),
        target_commit,
        policy_source: if explicit_config.is_some() {
            "explicit"
        } else {
            "target"
        }
        .into(),
        details,
        source_diff,
        limitations: limitations(
            "Both sides use the target ref's effective policy (or the explicit configuration) and the same module depth. Compares the exact ref tips, not the merge base. The index, unstaged and untracked files are excluded. Refs are resolved once and recorded as immutable commit IDs.",
        ),
    })
}
/// Both graphs are extracted from Git objects; the working tree is never read
/// unless the caller explicitly supplies an external policy file.
pub fn compare_staged(
    root: &Path,
    base: &str,
    explicit_config: Option<&Path>,
) -> Result<StagedArchitectureDiff, ArchguardError> {
    let (root, repo) = repository(root)?;
    let relative = root.strip_prefix(&repo).map_err(invalid)?;
    let commit = resolve_commit(&repo, base)?;
    let records = index(&repo, &commit)?;
    let tree = index_tree(&records)?;
    let staged = tempfile::tempdir()?;
    let previous = tempfile::tempdir()?;
    extract_tree(&repo, &tree, relative, staged.path())?;
    extract_snapshot(&repo, &commit, relative, previous.path())?;
    let details = compare_snapshots(
        &root,
        relative,
        previous.path(),
        staged.path(),
        explicit_config,
    )?;
    let source_diff = source_patch(&repo, relative, &["--cached", &commit])?;
    if records != index(&repo, &commit)? {
        return Err(invalid(
            "index changed during analysis; retry the comparison",
        ));
    }
    Ok(StagedArchitectureDiff {
        definition: "git-index-architecture/v1".into(),
        target: "index".into(),
        base_ref: base.into(),
        base_commit: commit,
        index_fingerprint: format!("{:x}", Sha256::digest(&records)),
        policy_source: if explicit_config.is_some() {
            "explicit"
        } else {
            "index"
        }
        .into(),
        module_depth: details.module_depth,
        base_graph: details.base_graph,
        staged_graph: details.target_graph,
        base_summary: details.base_summary,
        staged_summary: details.target_summary,
        added_nodes: details.added_nodes,
        removed_nodes: details.removed_nodes,
        added_edges: details.added_edges,
        removed_edges: details.removed_edges,
        changed_edges: details.changed_edges,
        new_findings: details.new_findings,
        resolved_findings: details.resolved_findings,
        source_diff,
        limitations: limitations(
            "Both sides use the staged effective policy (or the explicit configuration) and the same module depth. Unstaged and untracked files are excluded.",
        ),
    })
}
