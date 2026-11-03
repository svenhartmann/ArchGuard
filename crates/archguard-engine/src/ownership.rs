//! CODEOWNERS ownership evidence and explicitly forbidden owner directions.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use archguard_core::{
    ArchguardError, Finding, FindingKind, Graph, NodeId, NodeKind, OwnershipRule, Ref,
    ReviewSources,
};
use globset::{GlobBuilder, GlobMatcher};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAX_CODEOWNERS_BYTES: u64 = 3 * 1024 * 1024;
const MAX_RULES: usize = 10_000;

struct OwnerPattern {
    line: u32,
    owners: Vec<String>,
    matchers: Vec<GlobMatcher>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnershipStatus {
    Available,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerAssignment {
    pub node_id: NodeId,
    pub name: String,
    pub path: Option<String>,
    pub owners: Vec<String>,
    pub rule_line: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnershipDependency {
    pub from: NodeId,
    pub to: NodeId,
    pub from_name: String,
    pub to_name: String,
    pub from_owners: Vec<String>,
    pub to_owners: Vec<String>,
    /// True only when the owner sets are nonempty and disjoint.
    pub cross_owner: bool,
    pub count: u64,
    pub count_is_lower_bound: bool,
    pub evidence: Vec<Ref>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OwnershipReport {
    pub status: OwnershipStatus,
    /// Path relative to the Git repository, or the project root outside Git.
    pub codeowners_path: Option<String>,
    pub complete: bool,
    pub total_files: u32,
    pub mapped_files: u32,
    pub owned_files: u32,
    pub unowned_files: u32,
    pub cross_owner_dependencies: u32,
    pub assignments: Vec<OwnerAssignment>,
    pub dependencies: Vec<OwnershipDependency>,
    pub findings: Vec<Finding>,
    pub warnings: Vec<String>,
}

/// Read local ownership declarations. No GitHub account or permissions lookup.
pub fn analyze_ownership(
    root: &Path,
    graph: &Graph,
    sources: &ReviewSources,
    rules: &[OwnershipRule],
) -> Result<OwnershipReport, ArchguardError> {
    let by_id: BTreeMap<_, _> = sources
        .files
        .iter()
        .map(|file| (file.node_id, file))
        .collect();
    let assignments: Vec<_> = graph
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::File)
        .map(|node| {
            let path = by_id
                .get(&node.id)
                .and_then(|file| file.path.clone())
                .filter(|path| {
                    !path.is_empty()
                        && Path::new(path)
                            .components()
                            .all(|component| matches!(component, Component::Normal(_)))
                });
            OwnerAssignment {
                node_id: node.id,
                name: node.name.clone(),
                path,
                owners: vec![],
                rule_line: None,
            }
        })
        .collect();
    let mut report=OwnershipReport {
        status:OwnershipStatus::Unavailable,codeowners_path:None,complete:false,
        total_files:assignments.len() as u32,mapped_files:assignments.iter().filter(|file|file.path.is_some()).count() as u32,
        owned_files:0,unowned_files:0,cross_owner_dependencies:0,assignments,dependencies:vec![],findings:vec![],
        warnings:vec!["Ownership comes from local CODEOWNERS declarations; GitHub account existence, team membership and repository permissions are not verified.".into()],
    };
    let canonical_root = fs::canonicalize(root)?;
    let repository = repository_root(root)?;
    let prefix = canonical_root
        .strip_prefix(&repository)
        .expect("repository contains the root")
        .to_string_lossy()
        .replace('\\', "/");
    let Some(relative) = [".github/CODEOWNERS", "CODEOWNERS", "docs/CODEOWNERS"]
        .into_iter()
        .find(|name| repository.join(name).symlink_metadata().is_ok())
    else {
        report
            .warnings
            .push("No CODEOWNERS file found in .github/, repository root, or docs/.".into());
        return Ok(report);
    };
    report.codeowners_path = Some(relative.into());
    let path = repository.join(relative);
    let metadata = fs::symlink_metadata(&path)?;
    if !metadata.is_file() || metadata.len() >= MAX_CODEOWNERS_BYTES {
        report.warnings.push(
            "CODEOWNERS must be a regular file smaller than 3 MiB; ownership was not evaluated."
                .into(),
        );
        return Ok(report);
    }
    let mut contents = String::new();
    fs::File::open(path)?
        .take(MAX_CODEOWNERS_BYTES)
        .read_to_string(&mut contents)?;
    if contents.len() as u64 >= MAX_CODEOWNERS_BYTES {
        report.warnings.push(
            "CODEOWNERS exceeded the 3 MiB reading limit; ownership was not evaluated.".into(),
        );
        return Ok(report);
    }
    let (patterns, warnings) = parse_codeowners(&contents);
    if patterns.len() > MAX_RULES {
        report.warnings.push(format!(
            "CODEOWNERS exceeds the {MAX_RULES}-rule processing limit; ownership was not evaluated."
        ));
        return Ok(report);
    }
    report.status = OwnershipStatus::Available;
    report.complete = warnings.is_empty() && report.mapped_files == report.total_files;
    report.warnings.extend(warnings);
    if report.mapped_files < report.total_files {
        report.warnings.push(
            "Some graph files lack a unique physical source path; their owners are unknown.".into(),
        );
    }
    for file in &mut report.assignments {
        let Some(path) = &file.path else { continue };
        let path = if prefix.is_empty() {
            path.clone()
        } else {
            format!("{prefix}/{path}")
        };
        if let Some(pattern) = patterns.iter().rev().find(|pattern| {
            pattern
                .matchers
                .iter()
                .any(|matcher| matcher.is_match(&path))
        }) {
            file.owners = pattern.owners.clone();
            file.rule_line = Some(pattern.line);
        }
    }
    report.owned_files = report
        .assignments
        .iter()
        .filter(|file| !file.owners.is_empty())
        .count() as u32;
    report.unowned_files = report.mapped_files - report.owned_files;
    let owners: BTreeMap<_, _> = report
        .assignments
        .iter()
        .map(|file| (file.node_id, file))
        .collect();
    for edge in &graph.edges {
        let (Some(from), Some(to)) = (owners.get(&edge.from), owners.get(&edge.to)) else {
            continue;
        };
        if from.owners.is_empty() || to.owners.is_empty() {
            continue;
        }
        let cross_owner = !from.owners.iter().any(|owner| to.owners.contains(owner));
        if cross_owner {
            report.cross_owner_dependencies += 1;
        }
        report.dependencies.push(OwnershipDependency {
            from: edge.from,
            to: edge.to,
            from_name: from.name.clone(),
            to_name: to.name.clone(),
            from_owners: from.owners.clone(),
            to_owners: to.owners.clone(),
            cross_owner,
            count: edge.weight(),
            count_is_lower_bound: edge.count.is_none(),
            evidence: edge.refs.iter().take(10).cloned().collect(),
        });
        // Explicit policy is checked for every declared owner pair. A shared
        // third owner cannot bypass a forbidden A -> B direction.
        for rule in rules {
            if !from.owners.contains(&rule.from) || !to.owners.contains(&rule.to) {
                continue;
            }
            let mut hash = Sha256::new();
            for value in [&rule.from, &rule.to, &from.name, &to.name] {
                hash.update(value.as_bytes());
                hash.update([0]);
            }
            report.findings.push(Finding {
                kind: FindingKind::OwnershipRule,
                id: format!("ownership-{:x}", hash.finalize()),
                severity: rule.severity,
                nodes: vec![from.name.clone(), to.name.clone()],
                edge_count: 1,
                cycles_sampled: 0,
                sampling: None,
                suggested_cuts: vec![],
                message: Some(if rule.message.is_empty() {
                    format!("Forbidden ownership dependency {} → {}", rule.from, rule.to)
                } else {
                    rule.message.clone()
                }),
                evidence: edge.refs.iter().take(10).cloned().collect(),
            });
        }
    }
    report.dependencies.sort_by(|a, b| {
        a.from_name
            .cmp(&b.from_name)
            .then_with(|| a.to_name.cmp(&b.to_name))
    });
    report
        .findings
        .sort_by(|a, b| a.severity.cmp(&b.severity).then_with(|| a.id.cmp(&b.id)));
    Ok(report)
}

pub(crate) fn repository_root(root: &Path) -> Result<PathBuf, ArchguardError> {
    let canonical = root.canonicalize()?;
    Ok(
        crate::review_sources::git_query(root, &["rev-parse", "--show-toplevel"])
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .map(|text| PathBuf::from(text.strip_suffix('\n').unwrap_or(&text)))
            .and_then(|path| path.canonicalize().ok())
            .filter(|path| canonical.starts_with(path))
            .unwrap_or(canonical),
    )
}

fn parse_codeowners(contents: &str) -> (Vec<OwnerPattern>, Vec<String>) {
    let mut patterns = Vec::new();
    let mut warnings = Vec::new();
    for (index, line) in contents.lines().enumerate() {
        if warnings.len() >= 100 {
            warnings.push("CODEOWNERS parsing stopped after 100 invalid lines; ownership coverage is incomplete.".into());
            break;
        }
        let mut words = line.split_whitespace();
        let Some(pattern) = words.next() else {
            continue;
        };
        if pattern.starts_with('#') {
            continue;
        }
        if patterns.len() > MAX_RULES {
            break;
        }
        let owners: Vec<_> = words
            .take_while(|owner| !owner.starts_with('#'))
            .map(String::from)
            .collect();
        let error = if pattern.len() > 4096 {
            Some("pattern exceeds the 4096-byte processing limit".to_string())
        } else if pattern.starts_with('!') || pattern.contains(['[', ']', '\\', '{', '}']) {
            Some("unsupported pattern (negation, ranges, escapes and brace patterns are not accepted)".to_string())
        } else if owners.iter().any(|owner| !valid_owner(owner)) {
            Some("owner must be @user, @org/team or an email address".to_string())
        } else {
            None
        };
        if let Some(error) = error {
            warnings.push(format!(
                "CODEOWNERS line {}: {error}; line skipped.",
                index + 1
            ));
            continue;
        }
        match owner_matchers(pattern) {
            Ok(matchers) => {
                let mut owners = owners;
                owners.sort();
                owners.dedup();
                patterns.push(OwnerPattern {
                    line: index as u32 + 1,
                    owners,
                    matchers,
                });
            }
            Err(error) => warnings.push(format!(
                "CODEOWNERS line {}: {error}; line skipped.",
                index + 1
            )),
        }
    }
    (patterns, warnings)
}

fn valid_owner(owner: &str) -> bool {
    if let Some(name) = owner.strip_prefix('@') {
        !name.is_empty()
            && !name.contains(['*', '?', '!', '#', '[', ']'])
            && name.split('/').count() <= 2
            && name.split('/').all(|part| !part.is_empty())
    } else {
        owner
            .split_once('@')
            .is_some_and(|(local, domain)| !local.is_empty() && !domain.is_empty())
    }
}

fn owner_matchers(pattern: &str) -> Result<Vec<GlobMatcher>, String> {
    let directory = pattern.ends_with('/');
    let trimmed = pattern.trim_matches('/');
    if trimmed.is_empty() {
        return Err("empty path pattern".into());
    }
    let anchored = pattern.starts_with('/') || trimmed.contains('/');
    let base = if anchored {
        trimmed.to_string()
    } else {
        format!("**/{trimmed}")
    };
    let mut patterns = vec![if directory {
        format!("{base}/**")
    } else {
        base.clone()
    }];
    // Literal directory names also own their descendants. A wildcard final
    // segment such as docs/* remains one level deep, as in GitHub's examples.
    if !directory
        && !trimmed
            .rsplit('/')
            .next()
            .unwrap_or(trimmed)
            .contains(['*', '?'])
    {
        patterns.push(format!("{base}/**"));
    }
    patterns
        .into_iter()
        .map(|pattern| {
            GlobBuilder::new(&pattern)
                .literal_separator(true)
                .backslash_escape(false)
                .build()
                .map(|glob| glob.compile_matcher())
                .map_err(|error| error.to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{Config, Severity};
    use std::fs;

    fn write(root: &Path, path: &str, contents: &str) {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn fixture() -> (tempfile::TempDir, Graph, ReviewSources) {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "src/ui/Button.ts",
            "import '../storage/Db';\nimport '../common/Shared';\n",
        );
        for path in [
            "src/storage/Db.ts",
            "src/common/Shared.ts",
            "src/ui/Local.ts",
            "src/empty/None.ts",
        ] {
            write(dir.path(), path, "export const value=1;\n");
        }
        write(dir.path(), "CODEOWNERS", "* @wrong-root-owner\n");
        write(dir.path(), "docs/CODEOWNERS", "* @wrong-docs-owner\n");
        write(
            dir.path(),
            ".github/CODEOWNERS",
            "* @org/platform\n/src/ui/ @org/ui\n/src/storage/ @org/data\n/src/common/ @org/ui @org/data\n/src/ui/Local.ts @org/local # last match wins\n/src/empty/\n",
        );
        let mut config = Config::default();
        config.project.source_roots = vec!["src".into()];
        let files = crate::pipeline::walk_files(dir.path(), &config).unwrap();
        let graph = crate::pipeline::build_graph(dir.path(), &files, &config)
            .unwrap()
            .0;
        let sources =
            crate::review_sources::collect_review_sources(dir.path(), &config, &graph, "180.days")
                .unwrap();
        (dir, graph, sources)
    }

    #[test]
    fn precedence_last_match_unowned_and_shared_owner_groups_are_explicit() {
        let (dir, graph, sources) = fixture();
        let result = analyze_ownership(dir.path(), &graph, &sources, &[]).unwrap();
        assert_eq!(result.status, OwnershipStatus::Available);
        assert_eq!(
            result.codeowners_path.as_deref(),
            Some(".github/CODEOWNERS")
        );
        assert!(result.complete);
        let assignment = |name: &str| {
            result
                .assignments
                .iter()
                .find(|entry| entry.name == name)
                .unwrap()
        };
        assert_eq!(assignment("ui/Local.ts").owners, vec!["@org/local"]);
        assert_eq!(assignment("empty/None.ts").owners, Vec::<String>::new());
        assert_eq!(assignment("empty/None.ts").rule_line, Some(6));
        assert_eq!(result.unowned_files, 1);
        assert_eq!(result.cross_owner_dependencies, 1);
        assert_eq!(result.dependencies.len(), 2);
        assert!(
            !result
                .dependencies
                .iter()
                .find(|edge| edge.to_name == "common/Shared.ts")
                .unwrap()
                .cross_owner
        );
        assert!(
            result.findings.is_empty(),
            "observed cross-team imports are not automatically forbidden"
        );
    }

    #[test]
    fn explicit_rule_cannot_be_bypassed_by_a_shared_owner() {
        let (dir, graph, sources) = fixture();
        let rule = OwnershipRule {
            from: "@org/ui".into(),
            to: "@org/data".into(),
            severity: Severity::Warning,
            message: "Use the data API".into(),
        };
        let result = analyze_ownership(dir.path(), &graph, &sources, &[rule]).unwrap();
        assert_eq!(result.cross_owner_dependencies, 1);
        assert_eq!(
            result.findings.len(),
            2,
            "the explicitly forbidden owner pair also exists on a co-owned target"
        );
        for finding in &result.findings {
            assert_eq!(finding.severity, Severity::Warning);
            assert_eq!(finding.message.as_deref(), Some("Use the data API"));
            assert_eq!(finding.evidence.len(), 1);
            assert_eq!(finding.evidence[0].file, "ui/Button.ts");
            assert!(finding.id.starts_with("ownership-"));
        }
    }

    #[test]
    fn unsupported_patterns_are_reported_and_missing_file_is_unavailable() {
        let (dir, graph, sources) = fixture();
        write(
            dir.path(),
            ".github/CODEOWNERS",
            "* @org/base\n!secret/ @org/a\n[ab].ts @org/b\n\\#literal @org/c\n",
        );
        let result = analyze_ownership(dir.path(), &graph, &sources, &[]).unwrap();
        assert_eq!(result.status, OwnershipStatus::Available);
        assert!(!result.complete);
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("line 2"))
        );
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("line 3"))
        );
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("line 4"))
        );
        for path in [".github/CODEOWNERS", "CODEOWNERS", "docs/CODEOWNERS"] {
            fs::remove_file(dir.path().join(path)).unwrap();
        }
        let missing = analyze_ownership(dir.path(), &graph, &sources, &[]).unwrap();
        assert_eq!(missing.status, OwnershipStatus::Unavailable);
        assert!(missing.findings.is_empty());
        assert!(
            missing
                .warnings
                .iter()
                .any(|warning| warning.contains("CODEOWNERS"))
        );
    }
    #[test]
    fn documented_directory_globs_preserve_root_and_depth_semantics() {
        for (pattern, positive, negative) in [
            ("*.ts", vec!["a.ts", "src/a.ts"], vec!["src/a.java"]),
            (
                "/docs/",
                vec!["docs/a.ts", "docs/nested/a.ts"],
                vec!["src/docs/a.ts"],
            ),
            (
                "docs/*",
                vec!["docs/a.ts"],
                vec!["docs/nested/a.ts", "src/docs/a.ts"],
            ),
            (
                "apps/",
                vec!["apps/a.ts", "src/apps/a.ts"],
                vec!["myapps/a.ts"],
            ),
            (
                "**/logs",
                vec!["logs/a.ts", "src/logs/a.ts"],
                vec!["src/mylogs/a.ts"],
            ),
            (
                "/apps/github",
                vec!["apps/github/a.ts"],
                vec!["src/apps/github/a.ts"],
            ),
        ] {
            let matchers = owner_matchers(pattern).unwrap();
            for path in positive {
                assert!(
                    matchers.iter().any(|matcher| matcher.is_match(path)),
                    "{pattern} must match {path}"
                );
            }
            for path in negative {
                assert!(
                    !matchers.iter().any(|matcher| matcher.is_match(path)),
                    "{pattern} must not match {path}"
                );
            }
        }
    }

    #[test]
    fn finding_identity_uses_owners_and_source_pair_not_rule_or_edge_order() {
        let (dir, mut graph, sources) = fixture();
        let policy = OwnershipRule {
            from: "@org/ui".into(),
            to: "@org/data".into(),
            ..OwnershipRule::default()
        };
        let ids = |report: OwnershipReport| {
            report
                .findings
                .into_iter()
                .map(|finding| finding.id)
                .collect::<Vec<_>>()
        };
        let first =
            ids(
                analyze_ownership(dir.path(), &graph, &sources, std::slice::from_ref(&policy))
                    .unwrap(),
            );
        graph.nodes.reverse();
        graph.edges.reverse();
        let unrelated = OwnershipRule {
            from: "@org/local".into(),
            to: "@org/no-import".into(),
            ..OwnershipRule::default()
        };
        let next =
            ids(analyze_ownership(dir.path(), &graph, &sources, &[unrelated, policy]).unwrap());
        assert_eq!(first, next);
    }
}
