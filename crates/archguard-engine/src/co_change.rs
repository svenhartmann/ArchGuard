//! Git co-change candidates without an observed static import connection.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use archguard_core::{ArchguardError, Config, GitEvidence, GitStatus, NodeId};
use serde::{Deserialize, Serialize};

use crate::config::validate_review_since;
use crate::pipeline::NamedGraph;
use crate::review_sources::{
    MAX_COMMITS, MAX_LOG_BYTES, collect_git_history, collect_source_evidence,
};

const MAX_FILES_PER_COMMIT: usize = 100;
const MAX_PAIR_OCCURRENCES: u64 = 1_000_000;
const MAX_CANDIDATE_PAIRS: usize = 100_000;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoChangeOptions {
    pub since: String,
    pub min_shared: u32,
    /// Minimum shared/touched ratio required in both directions, in [0, 1].
    pub min_ratio: f64,
    pub max_results: usize,
}

impl Default for CoChangeOptions {
    fn default() -> Self {
        Self {
            since: "180.days".into(),
            min_shared: 3,
            min_ratio: 0.5,
            max_results: 50,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoChangeFile {
    pub node_id: NodeId,
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoChangePair {
    pub left: CoChangeFile,
    pub right: CoChangeFile,
    pub shared_commits: u32,
    pub left_commits: u32,
    pub right_commits: u32,
    /// P(right changed | left changed) in the considered history.
    pub left_to_right: f64,
    /// P(left changed | right changed) in the considered history.
    pub right_to_left: f64,
    pub jaccard: f64,
    pub evidence_commits: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoChangeReport {
    pub git: GitEvidence,
    pub options: CoChangeOptions,
    pub mapped_files: u32,
    pub total_files: u32,
    pub considered_commits: u32,
    pub skipped_bulk_commits: u32,
    pub pair_occurrences: u64,
    pub pair_search_truncated: bool,
    pub total_candidates: usize,
    pub results_truncated: bool,
    pub max_files_per_commit: usize,
    pub max_pair_occurrences: u64,
    pub max_candidate_pairs: usize,
    pub pairs: Vec<CoChangePair>,
    pub warnings: Vec<String>,
}

/// Report review candidates, not proof of missing abstractions or runtime coupling.
/// Pass `(graph, names)` from the run that built `graph` to skip re-extraction.
pub fn co_change<'a>(
    root: &Path,
    config: &Config,
    graph: impl Into<NamedGraph<'a>>,
    options: &CoChangeOptions,
) -> Result<CoChangeReport, ArchguardError> {
    co_change_with_limits(
        root,
        config,
        graph.into(),
        options,
        (
            MAX_FILES_PER_COMMIT,
            MAX_PAIR_OCCURRENCES,
            MAX_CANDIDATE_PAIRS,
        ),
    )
}

fn co_change_with_limits(
    root: &Path,
    config: &Config,
    NamedGraph { graph, names }: NamedGraph<'_>,
    options: &CoChangeOptions,
    limits: (usize, u64, usize),
) -> Result<CoChangeReport, ArchguardError> {
    validate_review_since(&options.since)?;
    if options.min_shared < 3
        || !options.min_ratio.is_finite()
        || !(0.0..=1.0).contains(&options.min_ratio)
        || !(1..=1000).contains(&options.max_results)
    {
        return Err(ArchguardError::Config(
            "co-change requires min_shared >= 3, min_ratio in [0, 1], and max_results in [1, 1000]"
                .into(),
        ));
    }
    let sources = collect_source_evidence(root, config, NamedGraph { graph, names })?;
    let history = collect_git_history(root, &options.since, MAX_COMMITS, MAX_LOG_BYTES);
    let mut report = CoChangeReport {
        git: history.evidence, options: options.clone(),
        mapped_files: sources.iter().filter(|file| file.path.is_some()).count() as u32,
        total_files: sources.len() as u32, considered_commits: 0, skipped_bulk_commits: 0,
        pair_occurrences: 0, pair_search_truncated: false, total_candidates: 0,
        results_truncated: false, max_files_per_commit: limits.0, max_pair_occurrences: limits.1,
        max_candidate_pairs: limits.2, pairs: vec![], warnings: vec![
            "Co-change is a review signal, not proof of a missing abstraction. Only current uniquely mapped source paths and observed import edges are considered.".into(),
        ],
    };
    if report.mapped_files < report.total_files {
        report.warnings.push("Some source files could not be mapped uniquely; they are excluded from co-change analysis.".into());
    }
    if report.git.status != GitStatus::Available {
        return Ok(report);
    }
    let by_path: BTreeMap<_, _> = sources
        .iter()
        .filter_map(|file| file.path.as_ref().map(|path| (path.as_str(), file.node_id)))
        .collect();
    let mut touched = BTreeMap::<NodeId, u32>::new();
    let mut commits = Vec::new();
    for commit in &history.commits {
        let ids: BTreeSet<_> = commit
            .paths
            .iter()
            .filter_map(|path| by_path.get(path.as_str()).copied())
            .collect();
        if ids.len() > limits.0 {
            report.skipped_bulk_commits += 1;
            continue;
        }
        if ids.is_empty() {
            continue;
        }
        report.considered_commits += 1;
        for id in &ids {
            *touched.entry(*id).or_default() += 1;
        }
        commits.push((commit.sha.as_str(), ids.into_iter().collect::<Vec<_>>()));
    }
    let static_pairs: BTreeSet<_> = graph
        .edges
        .iter()
        .map(|edge| (edge.from.min(edge.to), edge.from.max(edge.to)))
        .collect();
    let mut shared = BTreeMap::<(NodeId, NodeId), (u32, Vec<String>)>::new();
    'search: for (sha, ids) in &commits {
        for (index, left) in ids.iter().enumerate() {
            for right in &ids[index + 1..] {
                if report.pair_occurrences >= limits.1 {
                    report.pair_search_truncated = true;
                    break 'search;
                }
                report.pair_occurrences += 1;
                let pair = (*left, *right);
                if static_pairs.contains(&pair) {
                    continue;
                }
                if !shared.contains_key(&pair) && shared.len() >= limits.2 {
                    report.pair_search_truncated = true;
                    break 'search;
                }
                let (count, evidence) = shared.entry(pair).or_default();
                *count += 1;
                if evidence.len() < 5 {
                    evidence.push((*sha).to_string());
                }
            }
        }
    }
    let files: BTreeMap<_, _> = sources
        .iter()
        .filter_map(|file| {
            Some((
                file.node_id,
                CoChangeFile {
                    node_id: file.node_id,
                    name: graph.node_name(file.node_id)?.to_string(),
                    path: file.path.clone()?,
                },
            ))
        })
        .collect();
    for ((mut left, mut right), (shared_commits, evidence_commits)) in shared {
        if shared_commits < options.min_shared {
            continue;
        }
        if files[&left].name > files[&right].name {
            std::mem::swap(&mut left, &mut right);
        }
        let left_commits = touched[&left];
        let right_commits = touched[&right];
        let left_to_right = shared_commits as f64 / left_commits as f64;
        let right_to_left = shared_commits as f64 / right_commits as f64;
        if left_to_right < options.min_ratio || right_to_left < options.min_ratio {
            continue;
        }
        report.pairs.push(CoChangePair {
            left: files[&left].clone(),
            right: files[&right].clone(),
            shared_commits,
            left_commits,
            right_commits,
            left_to_right,
            right_to_left,
            jaccard: shared_commits as f64 / (left_commits + right_commits - shared_commits) as f64,
            evidence_commits,
        });
    }
    report.pairs.sort_by(|a, b| {
        b.left_to_right
            .min(b.right_to_left)
            .total_cmp(&a.left_to_right.min(a.right_to_left))
            .then_with(|| b.jaccard.total_cmp(&a.jaccard))
            .then_with(|| b.shared_commits.cmp(&a.shared_commits))
            .then_with(|| a.left.name.cmp(&b.left.name))
            .then_with(|| a.right.name.cmp(&b.right.name))
    });
    report.total_candidates = report.pairs.len();
    report.results_truncated = report.pairs.len() > options.max_results;
    report.pairs.truncate(options.max_results);
    if report.skipped_bulk_commits > 0 {
        report.warnings.push(format!("{} bulk commits changing more than {} mapped source files were excluded from both support counts and ratios.",report.skipped_bulk_commits,limits.0));
    }
    if report.pair_search_truncated {
        report.warnings.push("Pair-search budget exhausted: shared counts and ratios are lower bounds; the ranking covers only examined pairs.".into());
    }
    if report.git.shallow || report.git.truncated {
        report.warnings.push("Git history is partial: ratios describe only the observed history and are not estimates for missing commits.".into());
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{GitStatus, Graph, NodeKind};
    use std::fs;
    use std::process::Command;

    fn git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn commit(root: &Path, files: &[&str], value: u32) {
        for file in files {
            let path = root.join(file);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, format!("export const value = {value};\n")).unwrap();
        }
        git(root, &["add", "--all"]);
        git(
            root,
            &["commit", "--quiet", "-m", &format!("change {value}")],
        );
    }

    fn fixture() -> (tempfile::TempDir, Config, Graph) {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "--quiet"]);
        git(dir.path(), &["config", "user.name", "Co-change Test"]);
        git(
            dir.path(),
            &["config", "user.email", "test@example.invalid"],
        );
        git(dir.path(), &["config", "commit.gpgsign", "false"]);
        for value in 0..3 {
            commit(
                dir.path(),
                &["project/src/a.ts", "project/src/b.ts", "project/src/c.ts"],
                value,
            );
        }
        commit(dir.path(), &["project/src/a.ts"], 4);
        commit(dir.path(), &["outside/noise.ts"], 5);
        let root = dir.path().join("project");
        let mut config = Config::default();
        config.project.source_roots = vec!["src".into()];
        let files = crate::pipeline::walk_files(&root, &config).unwrap();
        let graph = crate::pipeline::build_graph(&root, &files, &config)
            .unwrap()
            .0;
        (dir, config, graph)
    }

    #[test]
    fn reports_real_shared_commits_ratios_and_excludes_existing_imports() {
        let (dir, config, mut graph) = fixture();
        let id = |name: &str| {
            graph
                .nodes
                .iter()
                .find(|node| node.name == name)
                .unwrap()
                .id
        };
        graph.edges.push(archguard_core::Edge {
            from: id("c.ts"),
            to: id("a.ts"),
            count: Some(1),
            refs: vec![],
        });
        let result = co_change(
            &dir.path().join("project"),
            &config,
            &graph,
            &CoChangeOptions::default(),
        )
        .unwrap();
        assert_eq!(result.git.status, GitStatus::Available);
        assert_eq!(result.considered_commits, 4);
        assert_eq!(
            result.pairs.len(),
            2,
            "the existing c→a import excludes that unordered candidate"
        );
        let pair = result
            .pairs
            .iter()
            .find(|pair| pair.left.name == "a.ts")
            .unwrap();
        assert_eq!(pair.right.name, "b.ts");
        assert_eq!(pair.shared_commits, 3);
        assert_eq!(pair.left_commits, 4);
        assert_eq!(pair.right_commits, 3);
        assert_eq!(pair.left_to_right, 0.75);
        assert_eq!(pair.right_to_left, 1.0);
        assert_eq!(pair.jaccard, 0.75);
        assert_eq!(pair.evidence_commits.len(), 3);
        assert!(
            pair.evidence_commits
                .iter()
                .all(|sha| sha.len() == 40 || sha.len() == 64)
        );
        assert_eq!(pair.left.path, "src/a.ts");
        assert_eq!(result.mapped_files, 3);
        assert_eq!(
            result.total_files as usize,
            graph
                .nodes
                .iter()
                .filter(|node| node.kind == NodeKind::File)
                .count()
        );
    }

    #[test]
    fn filters_both_conditional_ratios_and_reports_result_limit() {
        let (dir, config, graph) = fixture();
        let mut options = CoChangeOptions {
            min_ratio: 0.8,
            ..CoChangeOptions::default()
        };
        let result = co_change(&dir.path().join("project"), &config, &graph, &options).unwrap();
        assert_eq!(result.pairs.len(), 1, "a's conditional ratio is only 0.75");
        assert_eq!(result.pairs[0].left.name, "b.ts");
        options.min_ratio = 0.5;
        options.max_results = 1;
        let result = co_change(&dir.path().join("project"), &config, &graph, &options).unwrap();
        assert_eq!(result.pairs.len(), 1);
        assert_eq!(result.total_candidates, 3);
        assert!(result.results_truncated);
        assert!(!result.pair_search_truncated);
    }

    #[test]
    fn invalid_thresholds_fail_before_history_collection() {
        let (dir, config, graph) = fixture();
        for options in [
            CoChangeOptions {
                min_shared: 0,
                ..CoChangeOptions::default()
            },
            CoChangeOptions {
                min_ratio: f64::NAN,
                ..CoChangeOptions::default()
            },
            CoChangeOptions {
                min_ratio: 1.1,
                ..CoChangeOptions::default()
            },
            CoChangeOptions {
                since: "--all".into(),
                ..CoChangeOptions::default()
            },
        ] {
            assert!(co_change(&dir.path().join("project"), &config, &graph, &options).is_err());
        }
    }
    #[test]
    fn bulk_commits_and_pair_budgets_are_visible_coverage_limits() {
        let (dir, config, graph) = fixture();
        let root = dir.path().join("project");
        let bulk = co_change_with_limits(
            &root,
            &config,
            (&graph).into(),
            &CoChangeOptions::default(),
            (2, MAX_PAIR_OCCURRENCES, MAX_CANDIDATE_PAIRS),
        )
        .unwrap();
        assert_eq!(bulk.skipped_bulk_commits, 3);
        assert_eq!(bulk.considered_commits, 1);
        assert!(bulk.pairs.is_empty());
        assert!(
            bulk.warnings
                .iter()
                .any(|warning| warning.contains("bulk commits"))
        );
        let limited = co_change_with_limits(
            &root,
            &config,
            (&graph).into(),
            &CoChangeOptions::default(),
            (MAX_FILES_PER_COMMIT, 1, MAX_CANDIDATE_PAIRS),
        )
        .unwrap();
        assert!(limited.pair_search_truncated);
        assert_eq!(limited.pair_occurrences, 1);
        assert_eq!(limited.considered_commits, 4);
        assert!(
            limited
                .warnings
                .iter()
                .any(|warning| warning.contains("lower bounds"))
        );
        let pair_limit = co_change_with_limits(
            &root,
            &config,
            (&graph).into(),
            &CoChangeOptions::default(),
            (MAX_FILES_PER_COMMIT, MAX_PAIR_OCCURRENCES, 1),
        )
        .unwrap();
        assert!(pair_limit.pair_search_truncated);
        assert_eq!(pair_limit.max_candidate_pairs, 1);
    }
}
