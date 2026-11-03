//! Read-only comparison against a Git commit, under the current analysis policy.
use crate::config::LoadedConfig;
use archguard_core::{
    ArchguardError, Cut, Finding, FindingKind, Graph, Report, Summary, aggregate,
};
use serde::Serialize;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

pub(crate) const MAX_TREE_BYTES: usize = 32 * 1024 * 1024;
const MAX_SNAPSHOT_BYTES: usize = 256 * 1024 * 1024;
const MAX_SNAPSHOT_FILES: usize = 100_000;

#[derive(Debug, Clone, Serialize)]
pub struct Comparison {
    pub base_ref: String,
    pub base_commit: String,
    pub base_summary: Summary,
    pub current_summary: Summary,
    pub new_findings: Vec<Finding>,
    pub resolved_findings: Vec<Finding>,
    pub unchanged_findings: usize,
    pub acd_delta: f64,
    pub acd_percent: Option<f64>,
    pub limitations: Vec<String>,
    /// Priority review report of the isolated base tree, computed with
    /// the same policy (R27). Consumed in-process by the PR-comment
    /// renderer; never serialized into the comparison JSON.
    #[serde(skip)]
    pub base_review: Option<archguard_core::ReviewReport>,
}

/// Compare the supplied current graph/report with an isolated historical tree.
/// Extraction uses the current effective policy, including layer origins.
pub fn compare_to_ref(
    root: &Path,
    loaded: &LoadedConfig,
    current_graph: &Graph,
    current: &Report,
    base: &str,
) -> Result<Comparison, ArchguardError> {
    crate::config::validate_config(&loaded.config)?;
    let repository =
        String::from_utf8(git(root, &["rev-parse", "--show-toplevel"])?).map_err(invalid)?;
    let repo = Path::new(repository.strip_suffix('\n').unwrap_or(&repository)).canonicalize()?;
    let root = root.canonicalize()?;
    let relative = root.strip_prefix(&repo).map_err(invalid)?;
    let commit = String::from_utf8(git(
        &repo,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{base}^{{commit}}"),
        ],
    )?)
    .map_err(invalid)?;
    let commit = commit.strip_suffix('\n').unwrap_or(&commit).to_string();
    if !matches!(commit.len(), 40 | 64) || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid("Git returned an invalid commit id"));
    }
    let snapshot = tempfile::tempdir()?;
    extract_snapshot(&repo, &commit, relative, snapshot.path())?;
    // Retain repository-relative ownership and ancestor-ignore semantics without
    // sharing the real index, worktree, hooks or historical cache.
    git(snapshot.path(), &["init", "--quiet", "--template=", "."])?;
    let base_root = snapshot.path().join(relative);
    std::fs::create_dir_all(&base_root)?;
    let config = &loaded.config;
    let files = crate::pipeline::walk_files(&base_root, config)?;
    let (mut base_graph, _) = crate::pipeline::build_graph(&base_root, &files, config)?;
    crate::semantic_calls::integrate_graph(&base_root, &files, config, &mut base_graph)?;
    let (previous, _) = crate::effective::build_effective_report(
        &base_root,
        &base_graph,
        config,
        crate::pipeline::Level::Module,
    )?;
    let previous_findings = comparison_findings(&base_graph, &previous, config)?;
    let current_findings = comparison_findings(current_graph, current, config)?;
    // R27: the base side of the PR comment reuses this isolated snapshot
    // so the renderer can diff priority findings without a second
    // checkout. The freshly initialized snapshot repo carries no
    // history, so git churn evidence is absent on the base side; the
    // graph metrics that drive the findings are complete.
    let base_view = aggregate(
        &base_graph,
        &Cut::at_depth(&base_graph, config.modules.depth as usize),
    );
    let base_review = archguard_analysis::analyze_review(
        &base_graph,
        &base_view,
        &crate::review_sources::collect_review_sources(
            &base_root,
            config,
            &base_graph,
            &config.review.git_since,
        )?,
        &config.review.entry_points,
    )?;
    let old: BTreeSet<_> = previous_findings.iter().map(identity).collect();
    let new: BTreeSet<_> = current_findings.iter().map(identity).collect();
    Ok(Comparison {
        base_ref:base.into(),base_commit:commit,
        base_summary:previous.summary.clone(),current_summary:current.summary.clone(),
        new_findings:current_findings.iter().filter(|finding|!old.contains(&identity(finding))).cloned().collect(),
        resolved_findings:previous_findings.iter().filter(|finding|!new.contains(&identity(finding))).cloned().collect(),
        unchanged_findings:current_findings.iter().filter(|finding|old.contains(&identity(finding))).count(),
        acd_delta:current.summary.acd-previous.summary.acd,
        acd_percent:(previous.summary.acd>0.0).then(||100.0*(current.summary.acd/previous.summary.acd-1.0)),
        base_review:Some(base_review),
        limitations:vec![
            "Both revisions use current policy and module depth, preserving declared/derived layer origins. Uncommitted source changes are included in the supplied current graph.".into(),
            "Layer violations are compared as complete directed pairs rather than sampled groups; counts can differ from the grouped check report.".into(),
            "Historical repository-relative CODEOWNERS and ancestor ignore files are retained. Git symlinks, submodules and .archguard caches are excluded from the isolated base.".into(),
            "Both revisions include resolved compiler calls and supertypes. Missing historical dependency assets may reduce semantic coverage; zero findings do not prove cycle freedom.".into(),
        ],
    })
}

pub(crate) fn comparison_findings(
    graph: &Graph,
    report: &Report,
    config: &archguard_core::Config,
) -> Result<Vec<Finding>, ArchguardError> {
    let view = aggregate(graph, &Cut::at_depth(graph, config.modules.depth as usize));
    let mut findings: Vec<_> = report
        .findings
        .iter()
        .filter(|finding| finding.kind != FindingKind::LayerViolation)
        .cloned()
        .collect();
    findings.extend(archguard_analysis::check_layer_pairs(
        &view,
        &config.layers,
    )?);
    findings.sort_by(|left, right| {
        left.severity
            .cmp(&right.severity)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(findings)
}

fn context_paths(relative: &Path) -> BTreeSet<PathBuf> {
    let mut paths = BTreeSet::from([
        PathBuf::from(".github/CODEOWNERS"),
        PathBuf::from("CODEOWNERS"),
        PathBuf::from("docs/CODEOWNERS"),
    ]);
    let mut ancestor = Some(relative);
    while let Some(parent) = ancestor {
        paths.insert(parent.join(".gitignore"));
        paths.insert(parent.join(".ignore"));
        ancestor = parent.parent();
    }
    paths
}

pub(crate) fn extract_snapshot(
    repo: &Path,
    commit: &str,
    relative: &Path,
    destination: &Path,
) -> Result<(), ArchguardError> {
    let context = context_paths(relative);
    let mut args: Vec<OsString> = [
        "--literal-pathspecs",
        "ls-tree",
        "-r",
        "-z",
        "--full-tree",
        commit,
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    if !relative.as_os_str().is_empty() {
        args.push("--".into());
        args.push(relative.as_os_str().into());
        args.extend(context.iter().map(|path| path.as_os_str().into()));
    }
    let tree = bounded_git(repo, &args, MAX_TREE_BYTES)?;
    extract_tree(repo, &tree, relative, destination)
}

/// Shared blob extraction for committed trees and normalized index records.
pub(crate) fn extract_tree(
    repo: &Path,
    tree: &[u8],
    relative: &Path,
    destination: &Path,
) -> Result<(), ArchguardError> {
    let context = context_paths(relative);
    let mut batch = Command::new("git")
        .current_dir(repo)
        .args(["cat-file", "--batch"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let result = (|| {
        let mut input = batch
            .stdin
            .take()
            .ok_or_else(|| invalid("missing Git input"))?;
        let mut output = BufReader::new(
            batch
                .stdout
                .take()
                .ok_or_else(|| invalid("missing Git output"))?,
        );
        let mut bytes = 0usize;
        let mut files = 0usize;
        for record in tree
            .split(|byte| *byte == 0)
            .filter(|record| !record.is_empty())
        {
            let record = std::str::from_utf8(record).map_err(invalid)?;
            let (metadata, path) = record
                .split_once('\t')
                .ok_or_else(|| invalid("invalid Git tree record"))?;
            let path = Path::new(path);
            if path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
            {
                return Err(invalid("unsafe path in Git tree"));
            }
            if path.components().any(|component| {
                component
                    .as_os_str()
                    .to_string_lossy()
                    .eq_ignore_ascii_case(".git")
            }) {
                return Err(invalid("Git metadata is not allowed in a base snapshot"));
            }
            if path != relative.join(crate::config::DEFAULT_CONFIG_PATH)
                && path
                    .components()
                    .any(|component| component.as_os_str() == ".archguard")
            {
                continue;
            }
            if !relative.as_os_str().is_empty()
                && !path.starts_with(relative)
                && !context.contains(path)
            {
                continue;
            }
            let mut fields = metadata.split_whitespace();
            let mode = fields.next().unwrap_or("");
            let kind = fields.next().unwrap_or("");
            let oid = fields.next().unwrap_or("");
            if kind != "blob" || !matches!(mode, "100644" | "100755") {
                continue;
            }
            if !matches!(oid.len(), 40 | 64) || !oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(invalid("invalid Git object id"));
            }
            files += 1;
            if files > MAX_SNAPSHOT_FILES {
                return Err(invalid("base snapshot exceeds 100000 files"));
            }
            writeln!(input, "{oid}")?;
            input.flush()?;
            let mut header = String::new();
            output.read_line(&mut header)?;
            let fields: Vec<_> = header.split_whitespace().collect();
            if fields.len() != 3 || fields[0] != oid || fields[1] != "blob" {
                return Err(invalid("Git base blob is unavailable"));
            }
            let size: usize = fields[2].parse().map_err(invalid)?;
            bytes = bytes.saturating_add(size);
            if bytes > MAX_SNAPSHOT_BYTES {
                return Err(invalid("base snapshot exceeds 256 MiB"));
            }
            let mut content = vec![0; size];
            output.read_exact(&mut content)?;
            let mut newline = [0];
            output.read_exact(&mut newline)?;
            if &newline != b"\n" {
                return Err(invalid("invalid Git batch record delimiter"));
            }
            let target = destination.join(path);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(target, content)?;
        }
        Ok(())
    })();
    let _ = batch.kill();
    let _ = batch.wait();
    result
}

pub(crate) fn bounded_git(
    root: &Path,
    args: &[OsString],
    limit: usize,
) -> Result<Vec<u8>, ArchguardError> {
    let mut child = Command::new("git")
        .current_dir(root)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let result = (|| {
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| invalid("missing Git tree output"))?;
        let mut bytes = vec![];
        stdout.take(limit as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > limit {
            return Err(invalid(
                "base tree listing exceeds the bounded snapshot limit",
            ));
        }
        if !child.wait()?.success() {
            return Err(invalid("could not list the base tree"));
        }
        Ok(bytes)
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}

pub(crate) fn identity(finding: &Finding) -> String {
    if finding.kind == archguard_core::FindingKind::LayerViolation {
        let mut pairs: Vec<_> = finding.nodes.chunks(2).map(|pair| pair.to_vec()).collect();
        pairs.sort();
        return format!(
            "layer|{:?}|{}",
            finding.severity,
            serde_json::to_string(&pairs).unwrap()
        );
    }
    let mut nodes = finding.nodes.clone();
    nodes.sort();
    nodes.dedup();
    let rule = if finding.kind == archguard_core::FindingKind::Cycle {
        if finding.id.starts_with("file-") {
            "file-cycle"
        } else {
            "cycle"
        }
    } else if finding.id.starts_with("metric-max-scc-size-") {
        "metric-max-scc-size"
    } else {
        &finding.id
    };
    if rule == "metric-max-acd" {
        nodes.clear();
    }
    format!(
        "{:?}|{:?}|{}|{}",
        finding.kind,
        finding.severity,
        rule,
        serde_json::to_string(&nodes).unwrap()
    )
}

fn invalid(error: impl std::fmt::Display) -> ArchguardError {
    ArchguardError::Config(format!("base comparison: {error}"))
}
pub(crate) fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>, ArchguardError> {
    let output = Command::new("git").current_dir(root).args(args).output()?;
    if !output.status.success() {
        return Err(invalid(String::from_utf8_lossy(&output.stderr).trim()));
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, process::Command};
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
    fn fixture() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        git(root.path(), &["init", "-q"]);
        git(
            root.path(),
            &["config", "user.email", "test@example.invalid"],
        );
        git(root.path(), &["config", "user.name", "Test"]);
        git(root.path(), &["config", "commit.gpgsign", "false"]);
        fs::write(root.path().join("archguard.toml"),"derive_layers=false\n[project]\nsource_roots=[]\nlanguages=['ts']\n[modules]\ndepth=0\n").unwrap();
        fs::write(root.path().join("a.ts"), "import './b';\n").unwrap();
        fs::write(root.path().join("b.ts"), "export const b=1;\n").unwrap();
        git(root.path(), &["add", "."]);
        git(root.path(), &["commit", "-qm", "base"]);
        git(root.path(), &["tag", "base"]);
        root
    }
    #[test]
    fn finds_only_new_violations_without_touching_working_tree() {
        let root = fixture();
        fs::write(root.path().join("b.ts"), "import './a';\n").unwrap();
        let loaded = crate::config::load_config(None, root.path()).unwrap();
        let run = crate::effective::analyze_run_quiet(
            root.path(),
            &loaded,
            crate::pipeline::Level::Module,
        )
        .unwrap();
        let comparison = compare_to_ref(
            root.path(),
            &run.effective_config(),
            &run.graph,
            &run.report,
            "base",
        )
        .unwrap();
        assert!(
            comparison
                .new_findings
                .iter()
                .any(|f| f.kind == archguard_core::FindingKind::Cycle)
        );
        assert!(comparison.base_summary.acd < comparison.current_summary.acd);
        assert_eq!(
            fs::read_to_string(root.path().join("b.ts")).unwrap(),
            "import './a';\n"
        );
        git(root.path(), &["add", "b.ts"]);
        git(root.path(), &["commit", "-qm", "cycle"]);
        let same = compare_to_ref(
            root.path(),
            &run.effective_config(),
            &run.graph,
            &run.report,
            "HEAD",
        )
        .unwrap();
        assert!(same.new_findings.is_empty());
        assert!(same.unchanged_findings > 0);
    }
    #[test]
    fn unknown_ref_is_an_error_instead_of_clean_comparison() {
        let root = fixture();
        let loaded = crate::config::load_config(None, root.path()).unwrap();
        let run = crate::effective::analyze_run_quiet(
            root.path(),
            &loaded,
            crate::pipeline::Level::Module,
        )
        .unwrap();
        assert!(
            compare_to_ref(
                root.path(),
                &run.effective_config(),
                &run.graph,
                &run.report,
                "missing-ref"
            )
            .is_err()
        );
    }
    #[test]
    fn comparison_identity_ignores_display_positions_but_preserves_direction() {
        let mut finding = Finding {
            kind: archguard_core::FindingKind::LayerViolation,
            id: "layer-1".into(),
            severity: archguard_core::Severity::Error,
            nodes: vec!["a".into(), "b".into()],
            edge_count: 1,
            cycles_sampled: 0,
            sampling: None,
            suggested_cuts: vec![],
            message: None,
            evidence: vec![],
        };
        let first = identity(&finding);
        finding.id = "layer-8".into();
        assert_eq!(identity(&finding), first);
        finding.nodes.reverse();
        assert_ne!(identity(&finding), first);
        finding.kind = archguard_core::FindingKind::MetricThreshold;
        finding.id = "metric-max-scc-size-scc-1".into();
        let first = identity(&finding);
        finding.id = "metric-max-scc-size-scc-7".into();
        assert_eq!(identity(&finding), first);
    }
    #[test]
    fn frozen_derived_layers_are_not_promoted_to_declared_gates_in_base() {
        use archguard_core::{LayerConfig, LayerOrigin, Status};
        let root = fixture();
        let loaded = crate::config::load_config(None, root.path()).unwrap();
        let run = crate::effective::analyze_run_quiet(
            root.path(),
            &loaded,
            crate::pipeline::Level::Module,
        )
        .unwrap();
        let mut policy = run.effective_config();
        policy.config.layers = vec![
            LayerConfig {
                name: "derived-core".into(),
                contains: vec!["a.ts".into()],
                may_depend_on: vec![],
                origin: LayerOrigin::Derived,
            },
            LayerConfig {
                name: "derived-target".into(),
                contains: vec!["b.ts".into()],
                may_depend_on: vec![],
                origin: LayerOrigin::Derived,
            },
        ];
        policy.config.rules.max_instability_core = 0.5;
        let current = crate::pipeline::build_report(
            &run.graph,
            &policy.config,
            crate::pipeline::Level::Module,
        )
        .unwrap();
        assert_eq!(current.summary.status, Status::Pass);
        assert!(
            current
                .findings
                .iter()
                .any(|finding| finding.kind == archguard_core::FindingKind::LayerViolation)
        );
        let compared = compare_to_ref(root.path(), &policy, &run.graph, &current, "HEAD").unwrap();
        assert_eq!(
            compared.base_summary.status,
            Status::Pass,
            "frozen inferred layers must remain information rather than become error gates"
        );
        assert!(compared.new_findings.is_empty());
        assert!(compared.resolved_findings.is_empty());
    }
    #[test]
    fn same_revision_preserves_unranked_cycle_information() {
        let root = fixture();
        fs::write(root.path().join("archguard.toml"),"derive_layers=true\n[project]\nsource_roots=[]\nlanguages=['ts']\n[modules]\ndepth=0\n[rules]\ndeny_cycles='off'\nmax_scc_size=100\n").unwrap();
        fs::write(root.path().join("b.ts"), "import './a';\n").unwrap();
        fs::write(root.path().join("c.ts"), "export const c=1;\n").unwrap();
        git(root.path(), &["add", "."]);
        git(
            root.path(),
            &["commit", "-qm", "mixed cycle and acyclic module"],
        );
        let run = crate::effective::analyze_run_quiet(
            root.path(),
            &crate::config::load_config(None, root.path()).unwrap(),
            crate::pipeline::Level::Module,
        )
        .unwrap();
        assert!(
            run.report
                .findings
                .iter()
                .any(|finding| finding.kind == archguard_core::FindingKind::LayerUnranked)
        );
        let compared = compare_to_ref(
            root.path(),
            &run.effective_config(),
            &run.graph,
            &run.report,
            "HEAD",
        )
        .unwrap();
        assert!(
            compared.new_findings.is_empty(),
            "unchanged unranked information is not a new finding"
        );
        assert!(compared.resolved_findings.is_empty());
    }
    #[test]
    fn new_layer_violation_after_the_ten_pair_sample_is_still_reported() {
        let root = fixture();
        fs::create_dir_all(root.path().join("source")).unwrap();
        fs::create_dir_all(root.path().join("target")).unwrap();
        fs::write(root.path().join("target/db.ts"), "export const value=1;\n").unwrap();
        for index in 0..10 {
            fs::write(
                root.path().join(format!("source/s{index:02}.ts")),
                "import '../target/db';\n",
            )
            .unwrap();
        }
        fs::write(root.path().join("archguard.toml"),"derive_layers=false\n[project]\nsource_roots=[]\nlanguages=['ts']\n[modules]\ndepth=1\n[rules]\ndeny_cycles='off'\nmax_scc_size=100\n[[layers]]\nname='source'\ncontains=['source/**']\nmay_depend_on=[]\n[[layers]]\nname='target'\ncontains=['target/**']\nmay_depend_on=[]\n").unwrap();
        git(root.path(), &["add", "."]);
        git(root.path(), &["commit", "-qm", "ten violations"]);
        fs::write(
            root.path().join("source/s10.ts"),
            "import '../target/db';\n",
        )
        .unwrap();
        let run = crate::effective::analyze_run_quiet(
            root.path(),
            &crate::config::load_config(None, root.path()).unwrap(),
            crate::pipeline::Level::Module,
        )
        .unwrap();
        let compared = compare_to_ref(
            root.path(),
            &run.effective_config(),
            &run.graph,
            &run.report,
            "HEAD",
        )
        .unwrap();
        let new: Vec<_> = compared
            .new_findings
            .iter()
            .filter(|finding| finding.kind == archguard_core::FindingKind::LayerViolation)
            .collect();
        assert_eq!(
            new.len(),
            1,
            "a new edge outside the display sample remains a new violation"
        );
        assert!(new[0].nodes.contains(&"source/s10.ts".to_string()));
        assert!(new[0].nodes.contains(&"target/db.ts".to_string()));
    }
}
