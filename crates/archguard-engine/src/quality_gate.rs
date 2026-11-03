//! Changed-code gates evaluate supplied source evidence; they do not run tests.
use archguard_core::{ArchguardError, CallAnalysis, Config, Graph};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Component, Path},
    process::{Command, Stdio},
};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct QualityGateOptions {
    pub min_changed_coverage: Option<f64>,
    pub max_complexity: Option<u32>,
    pub max_nesting: Option<u32>,
}
#[derive(Debug, Clone, Serialize)]
pub struct ChangedCodeCoverage {
    pub covered_lines: u64,
    pub total_lines: u64,
    pub percent: Option<f64>,
    pub unknown_files: usize,
    pub status: String,
    pub files: Vec<ChangedFileCoverage>,
}
#[derive(Debug, Clone, Serialize)]
pub struct ChangedFileCoverage {
    pub path: String,
    pub node: Option<String>,
    pub status: String,
    pub changed_lines: usize,
    pub covered_lines: Option<u64>,
    pub total_lines: Option<u64>,
    pub percent: Option<f64>,
    pub uncovered_lines: Vec<u32>,
}
#[derive(Debug, Clone, Serialize)]
pub struct QualityGateMethod {
    pub id: String,
    pub file: String,
    pub line: u32,
    pub end_line: u32,
    pub cyclomatic: u32,
    pub max_nesting: u32,
}
#[derive(Debug, Clone, Serialize)]
pub struct QualityGateFinding {
    pub kind: String,
    pub message: String,
    pub path: Option<String>,
    pub node: Option<String>,
    pub line: Option<u32>,
    pub method: Option<String>,
    pub actual: Option<f64>,
    pub limit: Option<f64>,
}
#[derive(Debug, Clone, Serialize)]
pub struct QualityGateReport {
    pub status: String,
    pub base_ref: String,
    pub base_commit: String,
    pub options: QualityGateOptions,
    pub changed_files: usize,
    pub changed_lines: usize,
    pub coverage: Option<ChangedCodeCoverage>,
    pub methods: Vec<QualityGateMethod>,
    pub findings: Vec<QualityGateFinding>,
    pub diagnostics: Vec<String>,
    pub semantic_status: Option<String>,
}
pub fn evaluate_quality_gate(
    root: &Path,
    config: &Config,
    graph: &Graph,
    base: &str,
    options: &QualityGateOptions,
) -> Result<QualityGateReport, ArchguardError> {
    if options.min_changed_coverage.is_none()
        && options.max_complexity.is_none()
        && options.max_nesting.is_none()
    {
        return Err(error("at least one changed-code threshold is required"));
    }
    if options
        .min_changed_coverage
        .is_some_and(|value| !value.is_finite() || !(0.0..=100.0).contains(&value))
        || options.max_complexity == Some(0)
    {
        return Err(error(
            "coverage must be finite and between 0 and 100; maximum complexity must be at least 1",
        ));
    }
    let root = root.canonicalize()?;
    let (commit, changes) = changed_sources(&root, config, base)?;
    let names: BTreeMap<_, _> = graph
        .nodes
        .iter()
        .map(|node| (node.id, node.name.clone()))
        .collect();
    let mut node_paths = BTreeMap::<String, BTreeSet<String>>::new();
    for source in crate::review_sources::collect_source_evidence(&root, config, graph)? {
        if let (Some(path), Some(name)) = (source.path, names.get(&source.node_id)) {
            node_paths.entry(path).or_default().insert(name.clone());
        }
    }
    let nodes = node_paths
        .into_iter()
        .filter_map(|(path, names)| {
            (names.len() == 1).then(|| (path, names.into_iter().next().unwrap()))
        })
        .collect();
    let mut report=QualityGateReport {
        status:"pass".into(),base_ref:base.into(),base_commit:commit,options:options.clone(),
        changed_files:changes.len(),changed_lines:changes.iter().map(|change|change.lines.len()).sum(),
        coverage:None,methods:vec![],findings:vec![],semantic_status:None,
        diagnostics:vec![
            "Compared the selected Git commit directly with current working files, including staged, unstaged and unignored new sources in this project. No merge-base is inferred; deletions have no new lines.".into(),
            "Only analyzed source files are gated. Tests/build configuration and non-source changes are not measured by these thresholds. No project tests or builds were executed.".into(),
        ],
    };
    if options.min_changed_coverage.is_some() {
        let coverage = crate::test_coverage::read_test_coverage(&root, config, graph);
        check_coverage(&mut report, &changes, &coverage, &nodes);
    }
    if options.max_complexity.is_some() || options.max_nesting.is_some() {
        if changes.iter().any(|change| !change.lines.is_empty()) {
            let mut calls = crate::semantic_calls::analyze_calls(&root, config);
            resolve_method_sources(&root, config, &mut calls)?;
            check_methods(&mut report, &changes, &calls, &nodes);
        } else {
            report.semantic_status = Some("not_applicable".into());
        }
    }
    report.findings.sort_by(|a, b| {
        (&a.path, &a.line, &a.kind, &a.method).cmp(&(&b.path, &b.line, &b.kind, &b.method))
    });
    report.status = if report
        .findings
        .iter()
        .any(|finding| finding.kind.ends_with("_unavailable"))
    {
        "incomplete"
    } else if report.findings.is_empty() {
        "pass"
    } else {
        "fail"
    }
    .into();
    Ok(report)
}

#[derive(Debug)]
struct ChangedSource {
    path: String,
    language: String,
    lines: BTreeSet<u32>,
    line_count: u32,
}
fn resolve_method_sources(
    root: &Path,
    config: &Config,
    calls: &mut CallAnalysis,
) -> Result<(), ArchguardError> {
    let paths = crate::reference_paths::source_reference_paths(root, config)?;
    let resolve = |file: &mut String| {
        *file = paths
            .get(file.as_str())
            .cloned()
            .unwrap_or_else(|| format!("<unmapped compiler source>/{file}"));
    };
    for class in &mut calls.classes {
        resolve(&mut class.file);
        for file in class.declaration_files.iter_mut().flatten() {
            resolve(file);
        }
    }
    for method in &mut calls.methods {
        resolve(&mut method.file);
    }
    Ok(())
}
const MAX_GIT_BYTES: usize = 16 * 1024 * 1024;
const MAX_CHANGED_LINES: usize = 1_000_000;

fn error(message: impl std::fmt::Display) -> ArchguardError {
    ArchguardError::Analysis(format!("quality gate: {message}"))
}
/// `git -c` values disabling the drivers named in `git config --null --list
/// --name-only` output. Even --no-ext-diff/--no-textconv can invoke Git
/// clean/process filters, so every comparison reads only their names.
pub(crate) fn filter_driver_overrides(names: &[u8]) -> Result<Vec<String>, std::str::Utf8Error> {
    let mut overrides = BTreeSet::new();
    for key in names.split(|byte| *byte == 0).filter(|key| !key.is_empty()) {
        let key = std::str::from_utf8(key)?;
        if key.starts_with("filter.")
            && [".clean", ".process", ".required"]
                .iter()
                .any(|suffix| key.ends_with(suffix))
        {
            overrides.insert(format!(
                "{key}={}",
                if key.ends_with(".required") {
                    "false"
                } else {
                    ""
                }
            ));
        }
    }
    Ok(overrides.into_iter().collect())
}

fn git(root: &Path, args: &[&str], overrides: &[String]) -> Result<Vec<u8>, ArchguardError> {
    let mut child = Command::new("git")
        .current_dir(root)
        .args([
            "--no-optional-locks",
            "--literal-pathspecs",
            "-c",
            "core.fsmonitor=false",
        ])
        .args(overrides.iter().flat_map(|value| ["-c", value.as_str()]))
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let result = (|| {
        let mut bytes = vec![];
        child
            .stdout
            .take()
            .ok_or_else(|| error("missing Git output"))?
            .take(MAX_GIT_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_GIT_BYTES {
            return Err(error(
                "Git result exceeds 16 MiB; no partial gate result is reported",
            ));
        }
        if !child.wait()?.success() {
            return Err(error(
                "Git command failed; verify repository and base reference",
            ));
        }
        Ok(bytes)
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}
fn paths(bytes: &[u8]) -> Result<BTreeSet<String>, ArchguardError> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .map(|part| {
            let name = std::str::from_utf8(part)
                .map_err(|_| error("non-UTF-8 Git paths cannot be mapped safely"))?;
            if Path::new(name)
                .components()
                .any(|part| !matches!(part, Component::Normal(_)))
                || name.contains('\\')
            {
                return Err(error("Git returned a non-project-relative source path"));
            }
            Ok(name.into())
        })
        .collect()
}
fn changed_sources(
    root: &Path,
    config: &Config,
    base: &str,
) -> Result<(String, Vec<ChangedSource>), ArchguardError> {
    let keys = git(root, &["config", "--null", "--list", "--name-only"], &[])?;
    let overrides = filter_driver_overrides(&keys).map_err(error)?;
    let bytes = git(
        root,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{base}^{{commit}}"),
        ],
        &overrides,
    )?;
    let commit = std::str::from_utf8(&bytes)
        .map_err(error)?
        .trim_end_matches('\n')
        .to_string();
    if !matches!(commit.len(), 40 | 64) || !commit.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(error("invalid resolved Git commit"));
    }
    let tracked = paths(&git(
        root,
        &[
            "diff",
            "--name-only",
            "-z",
            "--relative",
            "--no-renames",
            "--no-ext-diff",
            "--no-textconv",
            &commit,
            "--",
            ".",
        ],
        &overrides,
    )?)?;
    let untracked = paths(&git(
        root,
        &[
            "ls-files",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            ".",
        ],
        &overrides,
    )?)?;
    let all: BTreeSet<_> = tracked.union(&untracked).cloned().collect();
    if all.len() > 100_000 {
        return Err(error("too many changed paths"));
    }
    let mut changes = vec![];
    let mut count = 0usize;
    for source in crate::pipeline::walk_files(root, config)? {
        if !all.contains(&source.rel_path) {
            continue;
        }
        let bytes =
            crate::security::read_repo_file(root, Path::new(&source.rel_path), 8 * 1024 * 1024)?;
        let text = std::str::from_utf8(&bytes).map_err(error)?;
        let line_count = u32::try_from(text.lines().count()).map_err(error)?;
        let lines = if untracked.contains(&source.rel_path) {
            (1..=line_count).collect()
        } else {
            let diff = git(
                root,
                &[
                    "diff",
                    "--no-color",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--no-renames",
                    "--text",
                    "--unified=0",
                    &commit,
                    "--",
                    &source.rel_path,
                ],
                &overrides,
            )?;
            added_lines(&diff)?
        };
        count += lines.len();
        if count > MAX_CHANGED_LINES {
            return Err(error("more than one million changed lines"));
        }
        let language = match source.lang {
            archguard_extract::Lang::Java => "java",
            archguard_extract::Lang::CSharp => "csharp",
            archguard_extract::Lang::Ts => "typescript",
            archguard_extract::Lang::Js => "javascript",
            archguard_extract::Lang::Py => "python",
            archguard_extract::Lang::Php => "php",
            archguard_extract::Lang::Go => "go",
            archguard_extract::Lang::Rust => "rust",
        };
        changes.push(ChangedSource {
            path: source.rel_path,
            language: language.into(),
            lines,
            line_count,
        });
    }
    Ok((commit, changes))
}
fn added_lines(diff: &[u8]) -> Result<BTreeSet<u32>, ArchguardError> {
    let text = std::str::from_utf8(diff).map_err(error)?;
    let mut result = BTreeSet::new();
    for line in text.lines().filter(|line| line.starts_with("@@ ")) {
        let range = line
            .strip_prefix("@@ -")
            .and_then(|s| s.split_once(" +"))
            .and_then(|(_, s)| s.split_once(" @@"))
            .map(|(s, _)| s)
            .ok_or_else(|| error("invalid Git hunk header"))?;
        let (start, count) = range.split_once(',').unwrap_or((range, "1"));
        let start: u32 = start.parse().map_err(error)?;
        let count: u32 = count.parse().map_err(error)?;
        if count as usize > MAX_CHANGED_LINES || result.len() + count as usize > MAX_CHANGED_LINES {
            return Err(error("Git hunk exceeds changed-line limit"));
        }
        let end = start
            .checked_add(count)
            .ok_or_else(|| error("Git line range overflow"))?;
        if count > 0 && start == 0 {
            return Err(error("invalid zero start line"));
        }
        result.extend(start..end);
    }
    Ok(result)
}
fn finding(
    kind: &str,
    message: String,
    path: Option<&str>,
    nodes: &BTreeMap<String, String>,
) -> QualityGateFinding {
    QualityGateFinding {
        kind: kind.into(),
        message,
        path: path.map(String::from),
        node: path.and_then(|path| nodes.get(path).cloned()),
        line: None,
        method: None,
        actual: None,
        limit: None,
    }
}
fn check_coverage(
    report: &mut QualityGateReport,
    changes: &[ChangedSource],
    coverage: &crate::test_coverage::TestCoverageReport,
    nodes: &BTreeMap<String, String>,
) {
    let mut result = ChangedCodeCoverage {
        covered_lines: 0,
        total_lines: 0,
        percent: None,
        unknown_files: 0,
        status: "not_applicable".into(),
        files: vec![],
    };
    let files: BTreeMap<_, _> = coverage
        .files
        .iter()
        .map(|file| (file.path.as_str(), file))
        .collect();
    for change in changes.iter().filter(|change| !change.lines.is_empty()) {
        let mut detail = ChangedFileCoverage {
            path: change.path.clone(),
            node: nodes.get(&change.path).cloned(),
            status: "unavailable".into(),
            changed_lines: change.lines.len(),
            covered_lines: None,
            total_lines: None,
            percent: None,
            uncovered_lines: vec![],
        };
        let file = files.get(change.path.as_str());
        let hits = if matches!(coverage.status.as_str(), "ready" | "partial" | "stale") {
            file.filter(|file| file.status == "unchanged_since_import")
                .and_then(|file| file.line_hits.as_ref())
                .filter(|hits| hits.iter().all(|hit| hit.line <= change.line_count))
        } else {
            None
        };
        let Some(hits) = hits else {
            result.unknown_files += 1;
            result.files.push(detail);
            report.findings.push(finding("coverage_unavailable",format!("{}: current per-line coverage is unavailable (missing, stale, aggregate-only or source line numbers outside the current file).",change.path),Some(&change.path),nodes));
            continue;
        };
        let (mut total, mut covered) = (0_u64, 0_u64);
        for hit in hits.iter().filter(|hit| change.lines.contains(&hit.line)) {
            total += 1;
            covered += u64::from(hit.covered);
            if !hit.covered {
                detail.uncovered_lines.push(hit.line);
            }
        }
        detail.covered_lines = Some(covered);
        detail.total_lines = Some(total);
        detail.percent = (total > 0).then(|| 100.0 * covered as f64 / total as f64);
        detail.status = if total == 0 {
            "not_applicable"
        } else {
            "measured"
        }
        .into();
        result.total_lines += total;
        result.covered_lines += covered;
        result.files.push(detail);
    }
    if result.total_lines > 0 && result.unknown_files == 0 {
        let percent = 100.0 * result.covered_lines as f64 / result.total_lines as f64;
        result.percent = Some(percent);
        result.status = "measured".into();
        if percent < report.options.min_changed_coverage.unwrap() {
            let mut item = finding(
                "changed_coverage",
                format!(
                    "Changed executable-line coverage is {percent:.2}%, below the configured threshold."
                ),
                None,
                nodes,
            );
            item.actual = Some(percent);
            item.limit = report.options.min_changed_coverage;
            report.findings.push(item);
        }
    }
    if result.unknown_files > 0 {
        result.status = "incomplete".into();
    }
    report.diagnostics.push("Changed-line coverage intersects Git's added/replaced lines with executable lines reported by JaCoCo, Cobertura, Clover or LCOV. A missing line in a mapped detailed report is not counted as executable; no executable changed lines yields no percentage.".into());
    report.diagnostics.push("Report/source revision at import is unverified. The gate evaluates the supplied report, not a fresh test run; source hashes detect later changes only. Generate/import coverage for this working tree before using this gate in CI.".into());
    report.coverage = Some(result);
}
fn check_methods(
    report: &mut QualityGateReport,
    changes: &[ChangedSource],
    calls: &CallAnalysis,
    nodes: &BTreeMap<String, String>,
) {
    report.semantic_status = Some(calls.status.clone());
    report.diagnostics.extend(
        calls
            .diagnostics
            .iter()
            .take(20)
            .map(|message| format!("Compiler: {message}")),
    );
    if calls.diagnostics.len() > 20 {
        report.diagnostics.push(format!("{} further compiler diagnostics omitted; use archguard calls --format json for the complete snapshot.",calls.diagnostics.len()-20));
    }
    let mut compiler_files = BTreeSet::new();
    for class in &calls.classes {
        compiler_files.insert(class.file.as_str());
        for file in class.declaration_files.iter().flatten() {
            compiler_files.insert(file.as_str());
        }
    }
    for method in &calls.methods {
        compiler_files.insert(method.file.as_str());
    }
    for change in changes.iter().filter(|change| !change.lines.is_empty()) {
        if !compiler_files.contains(change.path.as_str()) {
            report.findings.push(finding("methods_unavailable",format!("{}: no compiler declaration proves that this file is included in the analyzed project; changed methods cannot be treated as passing.",change.path),Some(&change.path),nodes));
            continue;
        }
        let applicable: Vec<_> = calls
            .analyzers
            .iter()
            .filter(|analyzer| analyzer.language == change.language)
            .collect();
        if !matches!(
            change.language.as_str(),
            "java" | "csharp" | "typescript" | "javascript" | "go"
        ) || if applicable.is_empty() {
            calls.status != "ready"
        } else {
            applicable.iter().any(|analyzer| analyzer.status != "ready")
        } {
            report.findings.push(finding("methods_unavailable",format!("{}: complete compiler analysis for {} is unavailable; missing methods cannot be treated as passing.",change.path,change.language),Some(&change.path),nodes));
        }
        for method in calls
            .methods
            .iter()
            .filter(|method| method.file == change.path)
        {
            let Some(end_line) = method
                .end_line
                .filter(|end| method.line > 0 && *end >= method.line && *end <= change.line_count)
            else {
                let mut item = finding(
                    "methods_unavailable",
                    format!("{}: compiler source range is unavailable.", method.id),
                    Some(&change.path),
                    nodes,
                );
                item.method = Some(method.id.clone());
                report.findings.push(item);
                continue;
            };
            if change.lines.range(method.line..=end_line).next().is_none() {
                continue;
            }
            let Some(metric) = method
                .complexity
                .as_ref()
                .filter(|metric| metric.definition == "ast-decisions-v1" && metric.cyclomatic >= 1)
            else {
                let mut item = finding(
                    "methods_unavailable",
                    format!(
                        "{}: no recognized method-body measurement (including declarations without a body).",
                        method.id
                    ),
                    Some(&change.path),
                    nodes,
                );
                item.method = Some(method.id.clone());
                item.line = Some(method.line);
                report.findings.push(item);
                continue;
            };
            report.methods.push(QualityGateMethod {
                id: method.id.clone(),
                file: method.file.clone(),
                line: method.line,
                end_line,
                cyclomatic: metric.cyclomatic,
                max_nesting: metric.max_nesting,
            });
            for (kind, actual, limit) in [
                (
                    "method_complexity",
                    metric.cyclomatic,
                    report.options.max_complexity,
                ),
                (
                    "method_nesting",
                    metric.max_nesting,
                    report.options.max_nesting,
                ),
            ] {
                if let Some(limit) = limit.filter(|limit| actual > *limit) {
                    let mut item = finding(
                        kind,
                        format!(
                            "{}: {kind} {actual} exceeds {limit} in a method touched by changed lines.",
                            method.id
                        ),
                        Some(&change.path),
                        nodes,
                    );
                    item.line = Some(method.line);
                    item.method = Some(method.id.clone());
                    item.actual = Some(actual.into());
                    item.limit = Some(limit.into());
                    report.findings.push(item);
                }
            }
        }
    }
    report
        .methods
        .sort_by(|a, b| (&a.file, a.line, &a.id).cmp(&(&b.file, b.line, &b.id)));
    report.diagnostics.push("Method thresholds apply to current methods whose inclusive compiler declaration range intersects added/replaced lines. They do not measure a complexity delta against the base. Deletion-only edits are excluded; nested declarations may overlap their parent's range.".into());
}

#[cfg(test)]
mod tests {
    use super::*;
    fn report() -> QualityGateReport {
        QualityGateReport {
            status: "pass".into(),
            base_ref: "HEAD".into(),
            base_commit: "a".repeat(40),
            options: QualityGateOptions {
                min_changed_coverage: None,
                max_complexity: Some(4),
                max_nesting: Some(0),
            },
            changed_files: 1,
            changed_lines: 1,
            coverage: None,
            methods: vec![],
            findings: vec![],
            diagnostics: vec![],
            semantic_status: None,
        }
    }
    fn calls() -> CallAnalysis {
        serde_json::from_value(serde_json::json!({"status":"ready","files":1,"classes":[],"calls":[],"diagnostics":[],"methods":[
            {"id":"A#changed()","class_id":"A","name":"changed","signature":"changed()","file":"src/A.java","line":2,"end_line":5,"is_static":false,"complexity":{"cyclomatic":5,"max_nesting":1,"definition":"ast-decisions-v1"}},
            {"id":"A#old()","class_id":"A","name":"old","signature":"old()","file":"src/A.java","line":10,"end_line":20,"is_static":false,"complexity":{"cyclomatic":100,"max_nesting":50,"definition":"ast-decisions-v1"}}
        ]})).unwrap()
    }
    #[test]
    fn method_thresholds_use_compiler_range_not_only_declaration_line_or_whole_file() {
        let mut report = report();
        let changes = [ChangedSource {
            path: "src/A.java".into(),
            language: "java".into(),
            lines: BTreeSet::from([4]),
            line_count: 20,
        }];
        check_methods(&mut report, &changes, &calls(), &BTreeMap::new());
        assert_eq!(report.methods.len(), 1);
        assert_eq!(report.methods[0].id, "A#changed()");
        assert_eq!(report.findings.len(), 2);
        assert!(
            report
                .findings
                .iter()
                .all(|finding| finding.method.as_deref() == Some("A#changed()"))
        );
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.kind == "method_complexity"
                    && finding.actual == Some(5.0)
                    && finding.limit == Some(4.0))
        );
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.kind == "method_nesting")
        );
    }
    #[test]
    fn complete_typescript_and_allowed_javascript_methods_can_satisfy_thresholds() {
        for language in ["typescript", "javascript"] {
            let mut calls = calls();
            for method in &mut calls.methods {
                method.file = "src/A.ts".into();
            }
            calls.analyzers = vec![archguard_core::CallAnalyzer {
                language: language.into(),
                backend: "typescript".into(),
                status: "ready".into(),
                files: 1,
                version: Some("5".into()),
                project: None,
                diagnostics: vec![],
            }];
            let mut report = report();
            report.options.max_complexity = Some(5);
            report.options.max_nesting = Some(1);
            let changes = [ChangedSource {
                path: "src/A.ts".into(),
                language: language.into(),
                lines: BTreeSet::from([4]),
                line_count: 20,
            }];
            check_methods(&mut report, &changes, &calls, &BTreeMap::new());
            assert!(
                report.findings.is_empty(),
                "{}: {:?}",
                language,
                report.findings
            );
            assert_eq!(report.methods.len(), 1);
        }
    }
    #[test]
    fn compiler_source_roots_resolve_to_physical_changed_paths() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("src")).unwrap();
        std::fs::write(root.path().join("src/A.java"), "class A {}").unwrap();
        let mut config = Config::default();
        config.project.source_roots = vec!["src".into()];
        let mut calls = calls();
        for method in &mut calls.methods {
            method.file = "A.java".into();
        }
        resolve_method_sources(root.path(), &config, &mut calls).unwrap();
        assert!(
            calls
                .methods
                .iter()
                .all(|method| method.file == "src/A.java")
        );
    }
    #[test]
    fn missing_ranges_partial_analysis_and_unknown_metric_versions_are_incomplete() {
        let changes = [ChangedSource {
            path: "src/A.java".into(),
            language: "java".into(),
            lines: BTreeSet::from([4]),
            line_count: 20,
        }];
        for mutation in 0..3 {
            let mut calls = calls();
            match mutation {
                0 => calls.methods[0].end_line = None,
                1 => calls.status = "partial".into(),
                _ => {
                    calls.methods[0].complexity.as_mut().unwrap().definition = "future-rule".into()
                }
            }
            let mut report = report();
            check_methods(&mut report, &changes, &calls, &BTreeMap::new());
            assert!(
                report
                    .findings
                    .iter()
                    .any(|finding| finding.kind == "methods_unavailable")
            );
        }
    }
    #[test]
    fn zero_count_deletion_hunks_and_overflow_are_handled_explicitly() {
        assert!(
            added_lines(b"@@ -2,3 +1,0 @@\n-deleted\n")
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            added_lines(b"@@ -2 +2,2 @@\n-old\n+new\n+newer\n").unwrap(),
            BTreeSet::from([2, 3])
        );
        assert!(added_lines(b"@@ -1 +4294967295,2 @@\n").is_err());
        assert!(added_lines(b"@@ -1 +0,1 @@\n").is_err());
    }

    #[test]
    fn ready_analyzer_does_not_claim_changed_files_outside_its_compiler_project() {
        let mut report = report();
        let changes = [ChangedSource {
            path: "B/B.cs".into(),
            language: "csharp".into(),
            lines: BTreeSet::from([2]),
            line_count: 3,
        }];
        let calls:CallAnalysis=serde_json::from_value(serde_json::json!({"status":"ready","files":1,"classes":[{"id":"A","name":"A","package":"","file":"A/A.cs","graph_node":"A/A.cs","line":1,"kind":"class"}],"methods":[],"calls":[],"diagnostics":[],"analyzers":[{"language":"csharp","backend":"roslyn","status":"ready","files":1,"project":"A/A.csproj","diagnostics":[]}]})).unwrap();
        check_methods(&mut report, &changes, &calls, &BTreeMap::new());
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.kind == "methods_unavailable"),
            "ready compiler for A must not claim that changed B passed"
        );
    }
}
