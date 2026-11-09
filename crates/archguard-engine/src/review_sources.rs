//! Source-size and Git change evidence for architecture review candidates.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

use archguard_core::{
    ArchguardError, Config, GitEvidence, GitStatus, Graph, NodeKind, ReviewSources, SourceEvidence,
};
use archguard_extract::{extractor_for, strip_root};

pub(crate) const MAX_COMMITS: usize = 10_000;
pub(crate) const MAX_LOG_BYTES: u64 = 16 * 1024 * 1024;
const COMMIT_MARKER: &[u8] = b"\0\0\0ARCHGUARD_COMMIT:";

pub(crate) struct GitCommit {
    pub sha: String,
    pub author: String,
    pub paths: BTreeSet<String>,
}

pub(crate) struct GitHistory {
    pub evidence: GitEvidence,
    pub commits: Vec<GitCommit>,
}

/// Collect source-backed review inputs for the current full hierarchical graph.
pub fn collect_review_sources(
    root: &Path,
    config: &Config,
    graph: &Graph,
    since: &str,
) -> Result<ReviewSources, ArchguardError> {
    collect_with_limits(root, config, graph, since, MAX_COMMITS, MAX_LOG_BYTES)
}

fn collect_with_limits(
    root: &Path,
    config: &Config,
    graph: &Graph,
    since: &str,
    max_commits: usize,
    max_bytes: u64,
) -> Result<ReviewSources, ArchguardError> {
    crate::config::validate_review_since(since)?;
    let mut files = collect_source_evidence(root, config, graph)?;
    let history = collect_git_history(root, since, max_commits, max_bytes);
    let mut touches = BTreeMap::<&str, u32>::new();
    for commit in &history.commits {
        for path in &commit.paths {
            *touches.entry(path).or_default() += 1;
        }
    }
    if history.evidence.status == GitStatus::Available {
        for file in &mut files {
            file.changes = file
                .path
                .as_ref()
                .map(|path| touches.get(path.as_str()).copied().unwrap_or(0));
        }
    }
    Ok(ReviewSources {
        mapped_files: files.iter().filter(|file| file.path.is_some()).count() as u32,
        total_files: files.len() as u32,
        files,
        git: history.evidence,
    })
}

pub(crate) fn collect_source_evidence(
    root: &Path,
    config: &Config,
    graph: &Graph,
) -> Result<Vec<SourceEvidence>, ArchguardError> {
    let files = crate::pipeline::walk_files(root, config)?;
    source_evidence_from_files(config, graph, &files)
}

fn source_evidence_from_files(
    config: &Config,
    graph: &Graph,
    files: &[archguard_extract::SourceFile],
) -> Result<Vec<SourceEvidence>, ArchguardError> {
    // Reuse the extraction pipeline's exact logical naming rule. A physical
    // path suffix is insufficient for declared Java/C#/PHP package names.
    let mut candidates = BTreeMap::<String, Vec<(String, u64)>>::new();
    for file in files {
        // The walker's display path is lossy. Only exact paths can be joined
        // with Git's UTF-8 path evidence; otherwise zero would be invented.
        if file.path.to_str().is_none() {
            continue;
        }
        let Some(extractor) = extractor_for(file.lang) else {
            continue;
        };
        let content = String::from_utf8_lossy(&fs::read(&file.path)?).into_owned();
        let stripped = strip_root(&file.rel_path, &config.project.source_roots);
        let extraction = extractor.extract(&stripped, &content);
        let name = crate::pipeline::file_node_name(&stripped, &extraction.declarations);
        candidates
            .entry(name)
            .or_default()
            .push((file.rel_path.clone(), content.lines().count() as u64));
    }
    let files: Vec<_> = graph
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::File)
        .map(|node| {
            let source = candidates
                .get(&node.name)
                .filter(|paths| paths.len() == 1)
                .map(|paths| &paths[0]);
            SourceEvidence {
                node_id: node.id,
                path: source.map(|(path, _)| path.clone()),
                lines: source.map(|(_, lines)| *lines),
                changes: None,
            }
        })
        .collect();
    Ok(files)
}

pub(crate) fn git_query(root: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output.status.success().then_some(output.stdout)
}

pub(crate) fn collect_git_history(
    root: &Path,
    since: &str,
    max_commits: usize,
    max_bytes: u64,
) -> GitHistory {
    let mut evidence = GitEvidence {
        since: since.to_string(),
        message: Some("Git history is unavailable: Git is missing, this is not a worktree, or HEAD has no readable history.".into()),
        ..GitEvidence::default()
    };
    let unavailable = Vec::new();
    if git_query(root, &["rev-parse", "--is-inside-work-tree"]).as_deref() != Some(b"true\n") {
        return GitHistory {
            evidence,
            commits: unavailable,
        };
    }
    let Some(prefix) = git_query(root, &["rev-parse", "--show-prefix"])
        .and_then(|bytes| String::from_utf8(bytes).ok())
    else {
        return GitHistory {
            evidence,
            commits: unavailable,
        };
    };
    let prefix = prefix.strip_suffix('\n').unwrap_or(&prefix);
    evidence.shallow =
        git_query(root, &["rev-parse", "--is-shallow-repository"]).as_deref() == Some(b"true\n");
    let Ok(mut child) = Command::new("git")
        .current_dir(root)
        .args([
            "log",
            "--no-ext-diff",
            "--no-textconv",
            "--no-merges",
            "--no-renames",
            "--format=%x00%x00%x00ARCHGUARD_COMMIT:%H%x00%aE%x00",
            "--name-only",
            "-z",
        ])
        .arg(format!("--since={since}"))
        .arg(format!("--max-count={}", max_commits.saturating_add(1)))
        .args(["--", "."])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return GitHistory {
            evidence,
            commits: unavailable,
        };
    };
    let mut bytes = Vec::new();
    let read = child
        .stdout
        .take()
        .expect("stdout is piped")
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes);
    let byte_limit = bytes.len() as u64 > max_bytes;
    if byte_limit || read.is_err() {
        let _ = child.kill();
    }
    let status = child.wait();
    if read.is_err() || (!byte_limit && !status.is_ok_and(|status| status.success())) {
        return GitHistory {
            evidence,
            commits: unavailable,
        };
    }
    let (commits, truncated) = parse_log(&bytes, prefix, max_commits, byte_limit);
    evidence.status = GitStatus::Available;
    evidence.scanned_commits = commits.len() as u32;
    evidence.truncated = truncated;
    evidence.message = Some(if evidence.shallow || truncated {
        "Partial Git history: change counts are lower bounds. Counts commit touches at current paths; merge commits and rename history are excluded."
    } else {
        "Counts commit touches at current paths within the requested window; merge commits and rename history are excluded."
    }.into());
    GitHistory { evidence, commits }
}

fn parse_log(
    bytes: &[u8],
    prefix: &str,
    max_commits: usize,
    byte_limit: bool,
) -> (Vec<GitCommit>, bool) {
    // The delimiter contains NUL bytes, which cannot occur inside a filename.
    // Hash-looking names, tabs, quotes and newlines therefore stay literal.
    let starts: Vec<_> = bytes
        .windows(COMMIT_MARKER.len())
        .enumerate()
        .filter_map(|(index, bytes)| (bytes == COMMIT_MARKER).then_some(index))
        .collect();
    let complete = starts.len().saturating_sub(usize::from(byte_limit));
    let mut commits = Vec::new();
    for (index, start) in starts.iter().take(complete.min(max_commits)).enumerate() {
        let end = starts.get(index + 1).copied().unwrap_or(bytes.len());
        let record = &bytes[start + COMMIT_MARKER.len()..end];
        let Some(hash_end) = record.iter().position(|byte| *byte == 0) else {
            continue;
        };
        let author_start = hash_end + 1;
        let Some(author_length) = record[author_start..].iter().position(|byte| *byte == 0) else {
            continue;
        };
        let author = String::from_utf8_lossy(&record[author_start..author_start + author_length])
            .into_owned();
        let body = &record[author_start + author_length + 1..];
        let body = body.strip_prefix(b"\0\n").unwrap_or(body);
        let paths: BTreeSet<_> = body
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .filter_map(|path| std::str::from_utf8(path).ok())
            .filter_map(|path| path.strip_prefix(prefix))
            .map(String::from)
            .collect();
        commits.push(GitCommit {
            sha: String::from_utf8_lossy(&record[..hash_end]).into_owned(),
            author,
            paths,
        });
    }
    (commits, byte_limit || complete > max_commits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{GitStatus, NodeKind};
    use std::fs;
    use std::process::Command;

    fn write(root: &Path, name: &str, contents: &str) {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .current_dir(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn init_git(root: &Path) {
        git(root, &["init", "--quiet"]);
        git(
            root,
            &["config", "user.email", "review-test@example.invalid"],
        );
        git(root, &["config", "user.name", "Review Test"]);
        git(root, &["config", "commit.gpgsign", "false"]);
    }

    fn commit(root: &Path, message: &str) {
        git(root, &["add", "--all"]);
        git(root, &["commit", "--quiet", "-m", message]);
    }

    fn graph(root: &Path, config: &Config) -> Graph {
        let files = crate::pipeline::walk_files(root, config).unwrap();
        crate::pipeline::build_graph(root, &files, config)
            .unwrap()
            .0
    }

    #[test]
    fn declaration_names_map_to_real_sources_and_blank_lines_count() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "src/main/java/physical/Thing.java",
            "package logical.pkg;\n\npublic class Thing {}\n",
        );
        let mut config = Config::default();
        config.project.source_roots = vec!["src/main/java".into()];
        let graph = graph(dir.path(), &config);
        let result = collect_review_sources(dir.path(), &config, &graph, "180.days").unwrap();
        assert_eq!(
            result.mapped_files, 1,
            "the Java declaration must map back to its physical source"
        );
        assert_eq!(result.total_files, 1);
        let evidence = &result.files[0];
        assert_eq!(
            graph.node_name(evidence.node_id),
            Some("logical/pkg/Thing.java")
        );
        assert_eq!(
            evidence.path.as_deref(),
            Some("src/main/java/physical/Thing.java")
        );
        assert_eq!(evidence.lines, Some(3));
        assert_eq!(
            evidence.changes, None,
            "no repository is unknown, not zero changes"
        );
        assert_eq!(result.git.status, GitStatus::Unavailable);
    }

    #[test]
    fn ambiguous_declared_names_are_not_assigned_arbitrary_files() {
        let dir = tempfile::tempdir().unwrap();
        for path in ["src/main/java/a/Thing.java", "src/test/java/b/Thing.java"] {
            write(dir.path(), path, "package logical.pkg;\nclass Thing {}\n");
        }
        let config = Config::default();
        let graph = graph(dir.path(), &config);
        let result = collect_review_sources(dir.path(), &config, &graph, "180.days").unwrap();
        assert_eq!(result.total_files, 1);
        assert_eq!(result.mapped_files, 0);
        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].path, None);
        assert_eq!(result.files[0].lines, None);
        assert_eq!(result.files[0].changes, None);
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_source_paths_are_unknown_instead_of_zero_churn() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "source.ts", "export const a = 1;\n");
        let config = Config::default();
        let graph = graph(dir.path(), &config);
        let mut files = crate::pipeline::walk_files(dir.path(), &config).unwrap();
        let name = OsString::from_vec(b"source-\xff.ts".to_vec());
        // macOS rejects these filenames at creation time. Supply the walker's
        // path value directly and require rejection before any filesystem read.
        files[0].path = dir.path().join(name);
        let result = source_evidence_from_files(&config, &graph, &files)
            .expect("an unrepresentable path remains unmapped without reading it");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, None);
        assert_eq!(result[0].lines, None);
        assert_eq!(
            result[0].changes, None,
            "unrepresentable Git filenames must not appear as untouched"
        );
    }

    #[test]
    fn git_counts_commit_touches_with_literal_newline_paths_and_subproject_scope() {
        let dir = tempfile::tempdir().unwrap();
        init_git(dir.path());
        write(dir.path(), "project/src/a.ts", "export const a = 1;\n");
        write(
            dir.path(),
            "project/src/with\nnewline.ts",
            "export const b = 1;\n",
        );
        write(dir.path(), "outside/a.ts", "export const outside = 1;\n");
        commit(dir.path(), "initial");
        write(dir.path(), "project/src/a.ts", "export const a = 2;\n");
        commit(dir.path(), "modify a");
        write(dir.path(), "outside/a.ts", "export const outside = 2;\n");
        commit(dir.path(), "outside only");
        write(
            dir.path(),
            "project/src/untracked.ts",
            "export const c = 1;\n",
        );
        let root = dir.path().join("project");
        let mut config = Config::default();
        config.project.source_roots = vec!["src".into()];
        let graph = graph(&root, &config);
        let result = collect_review_sources(&root, &config, &graph, "180.days").unwrap();
        assert_eq!(result.git.status, GitStatus::Available);
        assert_eq!(
            result.git.scanned_commits, 2,
            "outside-only commits are out of scope"
        );
        let touches = |name| {
            let id = graph
                .nodes
                .iter()
                .find(|node| node.name == name)
                .unwrap()
                .id;
            result
                .files
                .iter()
                .find(|file| file.node_id == id)
                .unwrap()
                .changes
        };
        assert_eq!(touches("a.ts"), Some(2));
        assert_eq!(touches("with\nnewline.ts"), Some(1));
        assert_eq!(touches("untracked.ts"), Some(0));
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
    fn git_date_validation_rejects_options_and_invalid_calendar_dates() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::default();
        let graph = graph(dir.path(), &config);
        for since in [
            "",
            "--all",
            "HEAD",
            "2026-02-30",
            "2026-13-01",
            "0.days",
            "1.day --all",
            "$(touch marker)",
        ] {
            assert!(
                collect_review_sources(dir.path(), &config, &graph, since).is_err(),
                "reject {since}"
            );
        }
        for since in ["180.days", "6.months", "1.year.ago", "2024-02-29"] {
            assert!(
                collect_review_sources(dir.path(), &config, &graph, since).is_ok(),
                "accept {since}"
            );
        }
    }

    #[test]
    fn history_limits_and_shallow_checkouts_report_lower_bounds() {
        let dir = tempfile::tempdir().unwrap();
        init_git(dir.path());
        for value in 0..3 {
            write(
                dir.path(),
                "a.ts",
                &format!("export const value = {value};\n"),
            );
            commit(dir.path(), &format!("change {value}"));
        }
        let config = Config::default();
        let graph = graph(dir.path(), &config);
        let limited =
            collect_with_limits(dir.path(), &config, &graph, "180.days", 1, MAX_LOG_BYTES).unwrap();
        assert_eq!(limited.git.status, GitStatus::Available);
        assert_eq!(limited.git.scanned_commits, 1);
        assert!(limited.git.truncated);
        assert_eq!(limited.files[0].changes, Some(1));
        assert!(
            limited
                .git
                .message
                .as_deref()
                .unwrap()
                .contains("lower bounds")
        );
        let byte_limited =
            collect_with_limits(dir.path(), &config, &graph, "180.days", MAX_COMMITS, 80).unwrap();
        assert_eq!(byte_limited.git.status, GitStatus::Available);
        assert!(byte_limited.git.truncated);
        assert_eq!(
            byte_limited.git.scanned_commits, 0,
            "a partially read commit cannot be counted as complete"
        );
        assert_eq!(byte_limited.files[0].changes, Some(0));

        let head = git_query(dir.path(), &["rev-parse", "HEAD"]).unwrap();
        fs::write(dir.path().join(".git/shallow"), head).unwrap();
        let shallow = collect_review_sources(dir.path(), &config, &graph, "180.days").unwrap();
        assert_eq!(shallow.git.status, GitStatus::Available);
        assert!(shallow.git.shallow);
        assert!(!shallow.git.truncated);
        assert_eq!(shallow.git.scanned_commits, 1);
        assert_eq!(shallow.files[0].changes, Some(1));
        assert!(
            shallow
                .git
                .message
                .as_deref()
                .unwrap()
                .contains("lower bounds")
        );
    }

    #[test]
    fn supplied_window_excludes_older_commit_touches() {
        let dir = tempfile::tempdir().unwrap();
        init_git(dir.path());
        write(dir.path(), "a.ts", "export const a = 1;\n");
        git(dir.path(), &["add", "--all"]);
        let output = Command::new("git")
            .current_dir(dir.path())
            .args(["commit", "--quiet", "-m", "old change"])
            .env("GIT_AUTHOR_DATE", "2000-01-01T12:00:00+0000")
            .env("GIT_COMMITTER_DATE", "2000-01-01T12:00:00+0000")
            .output()
            .unwrap();
        assert!(output.status.success());
        write(dir.path(), "a.ts", "export const a = 2;\n");
        commit(dir.path(), "recent change");
        let config = Config::default();
        let graph = graph(dir.path(), &config);
        let result = collect_review_sources(dir.path(), &config, &graph, "2020-01-01").unwrap();
        assert_eq!(result.git.since, "2020-01-01");
        assert_eq!(result.git.scanned_commits, 1);
        assert_eq!(result.files[0].changes, Some(1));
    }
}
