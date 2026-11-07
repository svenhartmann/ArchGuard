//! Explicit test-report imports. Test execution and producer revision are not inferred.
use archguard_core::{ArchguardError, Config, Graph};
use roxmltree::{Document, Node};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

const SNAPSHOT: &str = ".archguard/coverage.json";
const MAX_BYTES: u64 = 32 * 1024 * 1024;
const MAX_SOURCE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_FILES: usize = 100_000;
const REVISION_NOTE: &str = "Report/source revision at import is unverified. Hashes only detect later changes to mapped source files, not test/build configuration changes; these counts do not establish current or complete test coverage.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageCounter {
    pub covered: u64,
    pub total: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverageLine {
    pub line: u32,
    pub covered: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestCoverageFile {
    pub node: String,
    pub path: String,
    pub lines: Option<CoverageCounter>,
    pub branches: Option<CoverageCounter>,
    /// Reported executable source lines. None means a legacy/aggregate-only
    /// report; an empty list is only accepted with an explicit zero line total.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_hits: Option<Vec<CoverageLine>>,
    pub source_sha256: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestCoverageReport {
    pub schema_version: u32,
    pub status: String,
    pub format: Option<String>,
    pub report_path: Option<String>,
    pub imported_at_unix: Option<u64>,
    pub report_modified_unix: Option<u64>,
    pub files: Vec<TestCoverageFile>,
    pub unmatched_files: usize,
    pub diagnostics: Vec<String>,
}

impl TestCoverageReport {
    fn empty(status: &str) -> Self {
        Self {
            schema_version: 1,
            status: status.into(),
            format: None,
            report_path: None,
            imported_at_unix: None,
            report_modified_unix: None,
            files: vec![],
            unmatched_files: 0,
            diagnostics: vec![REVISION_NOTE.into()],
        }
    }
    fn diagnostic(&mut self, message: impl Into<String>) {
        if self.diagnostics.len() < 100 {
            self.diagnostics.push(message.into());
        }
    }
}

fn error(message: impl std::fmt::Display) -> ArchguardError {
    ArchguardError::Analysis(format!("test coverage: {message}"))
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn unix(time: SystemTime) -> Option<u64> {
    time.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())
}

/// Only the bounded snapshot is read. Source freshness is checked by read_test_coverage.
pub fn test_coverage_fingerprint(root: &Path) -> String {
    match crate::project_files::read_repo_file(root, Path::new(SNAPSHOT), MAX_BYTES) {
        Ok(bytes) => hash(&bytes),
        Err(e) => format!("unavailable:{:?}", e.kind()),
    }
}

fn source_paths(
    root: &Path,
    config: &Config,
    graph: &Graph,
) -> Result<BTreeMap<String, String>, ArchguardError> {
    let nodes: BTreeMap<_, _> = graph.nodes.iter().map(|n| (n.id, &n.name)).collect();
    let mut candidates = BTreeMap::<String, Vec<String>>::new();
    for evidence in crate::review_sources::collect_source_evidence(root, config, graph)? {
        if let (Some(path), Some(node)) = (evidence.path, nodes.get(&evidence.node_id)) {
            candidates.entry(path).or_default().push((*node).clone());
        }
    }
    Ok(candidates
        .into_iter()
        .filter_map(|(path, mut nodes)| {
            nodes.sort();
            nodes.dedup();
            (nodes.len() == 1).then(|| (path, nodes.remove(0)))
        })
        .collect())
}

/// Return a saved measurement with current source freshness; never execute tests.
pub fn read_test_coverage(root: &Path, config: &Config, graph: &Graph) -> TestCoverageReport {
    let bytes = match crate::project_files::read_repo_file(root, Path::new(SNAPSHOT), MAX_BYTES) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut report = TestCoverageReport::empty("not_imported");
            report.diagnostic("No test report imported. Use archguard coverage --report <JaCoCo/Cobertura/Clover XML or LCOV tracefile>.");
            return report;
        }
        Err(e) => {
            let mut report = TestCoverageReport::empty("error");
            report.diagnostic(format!("Cannot read {SNAPSHOT}: {e}"));
            return report;
        }
    };
    let mut report: TestCoverageReport = match serde_json::from_slice(&bytes) {
        Ok(report) => report,
        Err(e) => {
            let mut report = TestCoverageReport::empty("error");
            report.diagnostic(format!("Invalid coverage snapshot: {e}"));
            return report;
        }
    };
    let mut identities = BTreeSet::new();
    if report.schema_version != 1
        || !matches!(report.status.as_str(), "ready" | "partial")
        || !matches!(
            report.format.as_deref(),
            Some("jacoco" | "cobertura" | "lcov" | "clover")
        )
        || report.files.len() > MAX_FILES
        || report.diagnostics.len() > 100
        || report.files.iter().any(|f| {
            f.status != "unchanged_since_import"
                || !identities.insert(f.path.clone())
                || !relative(&f.path).is_ok_and(|p| p == f.path)
                || f.source_sha256.len() != 64
                || !f.source_sha256.bytes().all(|b| b.is_ascii_hexdigit())
                || [&f.lines, &f.branches]
                    .iter()
                    .any(|c| c.as_ref().is_some_and(|c| c.covered > c.total))
                || f.line_hits.as_ref().is_some_and(|hits| {
                    hits.len() > 500_000
                        || hits.iter().any(|hit| hit.line == 0)
                        || hits.windows(2).any(|pair| pair[0].line >= pair[1].line)
                        || f.lines.as_ref().is_none_or(|counter| {
                            counter.total != hits.len() as u64
                                || counter.covered
                                    != hits.iter().filter(|hit| hit.covered).count() as u64
                        })
                })
        })
    {
        let mut report = TestCoverageReport::empty("error");
        report.diagnostic(
            "Unsupported or inconsistent coverage snapshot; import a test report again.",
        );
        return report;
    }
    if !report.diagnostics.iter().any(|d| d == REVISION_NOTE) {
        report.diagnostic(REVISION_NOTE);
    }
    let paths = match source_paths(root, config, graph) {
        Ok(paths) => paths,
        Err(e) => {
            report.status = "error".into();
            report.files.clear();
            report.diagnostic(e.to_string());
            return report;
        }
    };
    let mut stale = 0;
    for file in &mut report.files {
        let unchanged = paths.get(&file.path) == Some(&file.node)
            && crate::project_files::read_repo_file(root, Path::new(&file.path), MAX_SOURCE_BYTES)
                .is_ok_and(|bytes| hash(&bytes) == file.source_sha256);
        if !unchanged {
            file.status = "changed_since_import".into();
            stale += 1;
        }
    }
    if stale > 0 {
        report.status = "stale".into();
        report.diagnostic(format!("{stale} source files changed, disappeared or left the analysis scope since import. Their saved counts are historical."));
    }
    report
}

/// Import one explicit report, replacing the previous snapshot only after validation.
pub fn import_test_coverage(
    root: &Path,
    config: &Config,
    graph: &Graph,
    report_path: &Path,
    source_root: Option<&Path>,
) -> Result<TestCoverageReport, ArchguardError> {
    let input = if report_path.is_absolute() {
        report_path.to_path_buf()
    } else {
        root.join(report_path)
    };
    let file = fs::File::open(&input)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return Err(error("report must be a regular file of at most 32 MiB"));
    }
    let mut xml = String::new();
    file.take(MAX_BYTES + 1).read_to_string(&mut xml)?;
    if xml.len() as u64 > MAX_BYTES {
        return Err(error("report exceeds 32 MiB"));
    }
    let (format, entries, bases, diagnostics) = parse_report(&xml)?;
    let canonical_root = root.canonicalize()?;
    let mut root_names = vec![clean_path(&canonical_root.to_string_lossy())?];
    if root.is_absolute() {
        root_names.push(clean_path(&root.to_string_lossy())?);
    }
    let prefix = source_root
        .map(|p| {
            clean_path(&p.to_string_lossy())?;
            let location = if p.is_absolute() {
                p.to_path_buf()
            } else {
                root.join(p)
            }
            .canonicalize()?;
            let value = location
                .strip_prefix(&canonical_root)
                .map_err(|_| error("source root must remain inside the project"))?;
            relative(&value.to_string_lossy())
        })
        .transpose()?;
    let paths = source_paths(root, config, graph)?;
    let mut report = TestCoverageReport::empty("ready");
    report.format = Some(format.into());
    report.report_path = Some(report_path.to_string_lossy().into_owned());
    report.imported_at_unix = unix(SystemTime::now());
    report.report_modified_unix = metadata.modified().ok().and_then(unix);
    if !diagnostics.is_empty() {
        report.status = "partial".into();
    }
    for diagnostic in diagnostics {
        report.diagnostic(diagnostic);
    }
    let mut mapped = BTreeMap::<String, Vec<&ParsedFile>>::new();
    for entry in &entries {
        let candidates =
            mapping_candidates(&entry.path, &bases, &root_names, prefix.as_deref(), config)?;
        let found: Vec<_> = candidates
            .iter()
            .filter(|path| paths.contains_key(*path))
            .collect();
        if found.len() != 1 {
            report.unmatched_files += 1;
            report.diagnostic(format!(
                "{}: {} matching source files; coverage is not assigned.",
                entry.path,
                found.len()
            ));
        } else {
            mapped.entry((*found[0]).clone()).or_default().push(entry);
        }
    }
    for (path, entries) in mapped {
        if entries.len() != 1 {
            report.unmatched_files += entries.len();
            report.diagnostic(format!(
                "{path}: multiple report entries map to one source; coverage is not assigned."
            ));
            continue;
        }
        let entry = entries[0];
        let bytes =
            match crate::project_files::read_repo_file(root, Path::new(&path), MAX_SOURCE_BYTES) {
                Ok(bytes) => bytes,
                Err(e) => {
                    report.unmatched_files += 1;
                    report.diagnostic(format!("{path}: cannot hash source: {e}"));
                    continue;
                }
            };
        report.files.push(TestCoverageFile {
            node: paths[&path].clone(),
            path,
            lines: entry.lines.clone(),
            branches: entry.branches.clone(),
            line_hits: entry.line_hits.clone(),
            source_sha256: hash(&bytes),
            status: "unchanged_since_import".into(),
        });
    }
    if report.unmatched_files > 0 || report.files.is_empty() {
        report.status = "partial".into();
    }
    if report.files.is_empty() {
        report.diagnostic("No report files could be mapped to analyzed source files; missing coverage is unknown, not zero.");
    }
    let bytes = serde_json::to_vec_pretty(&report).map_err(error)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(error("normalized snapshot exceeds 32 MiB"));
    }
    crate::project_files::write_project_file(root, Path::new("coverage.json"), &bytes, true)?;
    Ok(report)
}

// Report paths are identifiers only, never file reads. No traversal is accepted.
fn clean_path(value: &str) -> Result<String, ArchguardError> {
    let value = value.replace('\\', "/");
    if value.contains('\0') || value.split('/').any(|p| p == "..") {
        return Err(error("coverage path contains traversal or NUL"));
    }
    let prefix = if value.starts_with('/') { "/" } else { "" };
    Ok(format!(
        "{prefix}{}",
        value
            .split('/')
            .filter(|p| !p.is_empty() && *p != ".")
            .collect::<Vec<_>>()
            .join("/")
    ))
}
fn relative(value: &str) -> Result<String, ArchguardError> {
    let value = clean_path(value)?;
    if value.starts_with('/') || value.split('/').next().is_some_and(|p| p.contains(':')) {
        return Err(error("expected a project-relative coverage path"));
    }
    Ok(value)
}
fn mapping_candidates(
    path: &str,
    bases: &[String],
    roots: &[String],
    prefix: Option<&str>,
    config: &Config,
) -> Result<BTreeSet<String>, ArchguardError> {
    let path = clean_path(path)?;
    let mut relative_names = BTreeSet::new();
    if let Ok(name) = relative(&path) {
        relative_names.insert(name);
    }
    for base in bases.iter().chain(roots) {
        if let Some(name) = path.strip_prefix(&(base.to_string() + "/")) {
            relative_names.insert(relative(name)?);
        }
    }
    let mut candidates = BTreeSet::new();
    for name in relative_names {
        if let Some(prefix) = prefix {
            candidates.insert(relative(
                format!("{prefix}/{name}").trim_start_matches('/'),
            )?);
        } else {
            candidates.insert(name.clone());
            for base in config.project.source_roots.iter().chain(bases) {
                let base = relative(base).ok().or_else(|| {
                    roots.iter().find_map(|root| {
                        if base == root {
                            Some(String::new())
                        } else {
                            base.strip_prefix(&(root.clone() + "/")).map(String::from)
                        }
                    })
                });
                if let Some(base) = base {
                    candidates.insert(relative(format!("{base}/{name}").trim_start_matches('/'))?);
                }
            }
        }
    }
    Ok(candidates)
}

struct ParsedFile {
    path: String,
    lines: Option<CoverageCounter>,
    branches: Option<CoverageCounter>,
    line_hits: Option<Vec<CoverageLine>>,
}
fn number(node: Node<'_, '_>, name: &str) -> Result<u64, ArchguardError> {
    node.attribute(name)
        .ok_or_else(|| error(format!("missing {name}")))?
        .parse()
        .map_err(|_| error(format!("invalid nonnegative counter {name}")))
}
fn counter(node: Node<'_, '_>, kind: &str) -> Result<Option<CoverageCounter>, ArchguardError> {
    let values: Vec<_> = node
        .children()
        .filter(|n| n.has_tag_name("counter") && n.attribute("type") == Some(kind))
        .collect();
    if values.len() > 1 {
        return Err(error(format!("duplicate {kind} counter")));
    }
    values
        .first()
        .map(|n| {
            let covered = number(*n, "covered")?;
            let missed = number(*n, "missed")?;
            Ok(CoverageCounter {
                covered,
                total: covered
                    .checked_add(missed)
                    .ok_or_else(|| error("counter overflow"))?,
            })
        })
        .transpose()
}
type ParsedReport = (&'static str, Vec<ParsedFile>, Vec<String>, Vec<String>);
fn parse_report(xml: &str) -> Result<ParsedReport, ArchguardError> {
    // LCOV tracefiles are plain text; anything else is XML and keeps the guards below.
    if !xml
        .trim_start_matches('\u{feff}')
        .trim_start()
        .starts_with('<')
    {
        return parse_lcov(xml);
    }
    // JaCoCo emits a harmless external DOCTYPE. roxmltree never fetches it;
    // reject internal subsets/entities rather than expanding attacker input.
    if xml.contains("<!ENTITY")
        || xml.find("<!DOCTYPE").is_some_and(|start| {
            xml[start..]
                .split('>')
                .next()
                .is_some_and(|d| d.contains('['))
        })
    {
        return Err(error("internal DTDs and entities are not supported"));
    }
    let document = Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions {
            allow_dtd: true,
            nodes_limit: 500_000,
            ..Default::default()
        },
    )
    .map_err(error)?;
    let root = document.root_element();
    match root.tag_name().name() {
        "report" => parse_jacoco(root),
        "coverage" => {
            // Clover and Cobertura share the <coverage> root; classify by their
            // structural elements instead of guessing from mixed documents.
            let clover = root.descendants().any(|n| {
                n.has_tag_name("project")
                    || (n.has_tag_name("file") && n.attribute("path").is_some())
            });
            let cobertura = root
                .descendants()
                .any(|n| n.has_tag_name("class") && n.attribute("filename").is_some());
            match (clover, cobertura) {
                (true, true) => Err(error("report mixes Cobertura and Clover elements")),
                (true, false) => parse_clover(root),
                _ => parse_cobertura(root),
            }
        }
        _ => Err(error(
            "expected JaCoCo <report>, Cobertura/Clover <coverage> XML or an LCOV text report",
        )),
    }
}
fn parse_jacoco(root: Node<'_, '_>) -> Result<ParsedReport, ArchguardError> {
    let mut files = Vec::new();
    let mut seen = BTreeSet::new();
    let mut diagnostics = Vec::new();
    for source in root.descendants().filter(|n| n.has_tag_name("sourcefile")) {
        let package = source
            .parent()
            .filter(|n| n.has_tag_name("package"))
            .ok_or_else(|| error("sourcefile requires a package"))?;
        let name = source
            .attribute("name")
            .filter(|s| !s.is_empty())
            .ok_or_else(|| error("sourcefile needs a name"))?;
        let path = relative(
            format!("{}/{}", package.attribute("name").unwrap_or(""), name).trim_start_matches('/'),
        )?;
        if !seen.insert(path.clone()) {
            return Err(error(format!(
                "duplicate JaCoCo source path {path}; import a module report separately"
            )));
        }
        let mut lines = counter(source, "LINE")?;
        let mut branches = counter(source, "BRANCH")?;
        let mut line_ids = BTreeSet::new();
        let mut hits = Vec::new();
        let mut covered = 0_u64;
        let mut branch_covered = 0_u64;
        let mut branch_total = 0_u64;
        for line in source.children().filter(|n| n.has_tag_name("line")) {
            let id = number(line, "nr")?;
            if id == 0 || !line_ids.insert(id) {
                return Err(error("duplicate or zero JaCoCo line number"));
            }
            number(line, "mi")?;
            let covered_line = number(line, "ci")? > 0;
            covered += u64::from(covered_line);
            hits.push(CoverageLine {
                line: u32::try_from(id).map_err(|_| error("line number exceeds u32"))?,
                covered: covered_line,
            });
            let missed = number(line, "mb")?;
            let hit = number(line, "cb")?;
            branch_covered = branch_covered
                .checked_add(hit)
                .ok_or_else(|| error("branch overflow"))?;
            branch_total = branch_total
                .checked_add(missed)
                .and_then(|n| n.checked_add(hit))
                .ok_or_else(|| error("branch overflow"))?;
        }
        if !line_ids.is_empty() {
            lines = lines.or(Some(CoverageCounter {
                covered,
                total: line_ids.len() as u64,
            }));
            branches = branches.or(Some(CoverageCounter {
                covered: branch_covered,
                total: branch_total,
            }));
        }
        if lines.is_none() && diagnostics.len() < 99 {
            diagnostics.push(format!(
                "{path}: no source-line counter; line coverage is unknown."
            ));
        }
        hits.sort_by_key(|hit| hit.line);
        let line_hits = if lines
            .as_ref()
            .is_some_and(|counter| counter.total == hits.len() as u64 && counter.covered == covered)
        {
            Some(hits)
        } else {
            if !hits.is_empty() && diagnostics.len() < 99 {
                diagnostics.push(format!("{path}: source-line records differ from the LINE counter; changed-line coverage is unavailable."));
            }
            None
        };
        files.push(ParsedFile {
            path,
            lines,
            branches,
            line_hits,
        });
        if files.len() > MAX_FILES {
            return Err(error("too many report files"));
        }
    }
    Ok(("jacoco", files, vec![], diagnostics))
}
fn parse_cobertura(root: Node<'_, '_>) -> Result<ParsedReport, ArchguardError> {
    let bases = root
        .children()
        .filter(|n| n.has_tag_name("sources"))
        .flat_map(|n| n.children())
        .filter(|n| n.has_tag_name("source"))
        .filter_map(|n| n.text())
        .map(|s| clean_path(s.trim()))
        .collect::<Result<Vec<_>, _>>()?;
    let mut files = BTreeMap::<String, BTreeMap<u64, (bool, Option<CoverageCounter>)>>::new();
    let mut unknown_branches = BTreeSet::new();
    let mut instrumented_branches = BTreeSet::new();
    let mut diagnostics = Vec::new();
    for class in root.descendants().filter(|n| n.has_tag_name("class")) {
        let path = clean_path(
            class
                .attribute("filename")
                .filter(|s| !s.is_empty())
                .ok_or_else(|| error("Cobertura class needs filename"))?,
        )?;
        let lines = files.entry(path.clone()).or_default();
        for line in class
            .children()
            .filter(|n| n.has_tag_name("lines"))
            .flat_map(|n| n.children())
            .filter(|n| n.has_tag_name("line"))
        {
            let id = number(line, "number")?;
            if id == 0 {
                return Err(error("zero Cobertura line number"));
            }
            let covered = number(line, "hits")? > 0;
            if !matches!(line.attribute("branch"), None | Some("true" | "false")) {
                return Err(error("invalid Cobertura branch flag"));
            }
            if line.attribute("branch").is_some() {
                instrumented_branches.insert(path.clone());
            }
            let branch = if line.attribute("branch") == Some("true") {
                if let Some(value) = line.attribute("condition-coverage") {
                    let ratio = value
                        .rsplit_once('(')
                        .and_then(|(_, s)| s.strip_suffix(')'))
                        .and_then(|s| s.split_once('/'))
                        .ok_or_else(|| error("invalid condition-coverage"))?;
                    let covered: u64 = ratio
                        .0
                        .parse()
                        .map_err(|_| error("invalid branch covered count"))?;
                    let total: u64 = ratio
                        .1
                        .parse()
                        .map_err(|_| error("invalid branch total count"))?;
                    if covered > total || total == 0 {
                        return Err(error("invalid branch counter"));
                    }
                    Some(CoverageCounter { covered, total })
                } else {
                    unknown_branches.insert(path.clone());
                    None
                }
            } else {
                Some(CoverageCounter {
                    covered: 0,
                    total: 0,
                })
            };
            if let Some((old_covered, old_branch)) = lines.get_mut(&id) {
                *old_covered |= covered;
                if old_branch.as_ref().is_some_and(|b| b.total > 0)
                    || branch.as_ref().is_some_and(|b| b.total > 0)
                {
                    unknown_branches.insert(path.clone());
                    *old_branch = None;
                }
            } else {
                lines.insert(id, (covered, branch));
            }
        }
        if files.len() > MAX_FILES {
            return Err(error("too many report files"));
        }
    }
    let mut result = Vec::new();
    for (path, lines) in files {
        if !instrumented_branches.contains(&path) {
            unknown_branches.insert(path.clone());
        }
        let line_counter = (!lines.is_empty()).then(|| CoverageCounter {
            covered: lines.values().filter(|(covered, _)| *covered).count() as u64,
            total: lines.len() as u64,
        });
        let branches = if lines.is_empty() || unknown_branches.contains(&path) {
            None
        } else {
            let mut sum = CoverageCounter {
                covered: 0,
                total: 0,
            };
            for (_, branch) in lines.values() {
                let branch = branch.as_ref().expect("known branches");
                sum.covered = sum
                    .covered
                    .checked_add(branch.covered)
                    .ok_or_else(|| error("branch overflow"))?;
                sum.total = sum
                    .total
                    .checked_add(branch.total)
                    .ok_or_else(|| error("branch overflow"))?;
            }
            Some(sum)
        };
        if unknown_branches.contains(&path) && diagnostics.len() < 99 {
            diagnostics.push(format!(
                "{path}: missing or overlapping branch identities; branch coverage is unknown."
            ));
        }
        if line_counter.is_none() && diagnostics.len() < 99 {
            diagnostics.push(format!(
                "{path}: no source-line records; line coverage is unknown."
            ));
        }
        result.push(ParsedFile {
            path,
            lines: line_counter,
            branches,
            line_hits: if lines.is_empty() {
                None
            } else {
                Some(
                    lines
                        .into_iter()
                        .map(|(id, (covered, _))| {
                            Ok(CoverageLine {
                                line: u32::try_from(id)
                                    .map_err(|_| error("line number exceeds u32"))?,
                                covered,
                            })
                        })
                        .collect::<Result<Vec<_>, ArchguardError>>()?,
                )
            },
        });
    }
    Ok(("cobertura", result, bases, diagnostics))
}

fn parse_clover(root: Node<'_, '_>) -> Result<ParsedReport, ArchguardError> {
    let mut files = BTreeMap::<String, BTreeMap<u64, (bool, Option<CoverageCounter>)>>::new();
    let mut unknown_branches = BTreeSet::new();
    let mut instrumented_branches = BTreeSet::new();
    let mut diagnostics = Vec::new();
    for file in root.descendants().filter(|n| n.has_tag_name("file")) {
        let path = relative(
            file.attribute("path")
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| error("Clover file needs a path"))?,
        )?;
        let lines = files.entry(path.clone()).or_default();
        for line in file.children().filter(|n| n.has_tag_name("line")) {
            let id = number(line, "num")?;
            if id == 0 {
                return Err(error("zero Clover line number"));
            }
            let covered = number(line, "count")? > 0;
            let branch = match line
                .attribute("type")
                .ok_or_else(|| error("Clover line needs a type"))?
            {
                "stmt" | "method" => Some(CoverageCounter {
                    covered: 0,
                    total: 0,
                }),
                "cond" => {
                    // A condition only counts when both flags report an outcome;
                    // missing or non-numeric flags stay unknown, never 0/2.
                    let flags = ["true", "false"].map(|name| {
                        line.attribute(name)
                            .and_then(|value| value.parse::<u64>().ok())
                    });
                    instrumented_branches.insert(path.clone());
                    match flags {
                        [Some(true_hits), Some(false_hits)] => Some(CoverageCounter {
                            covered: u64::from(true_hits > 0) + u64::from(false_hits > 0),
                            total: 2,
                        }),
                        _ => {
                            unknown_branches.insert(path.clone());
                            None
                        }
                    }
                }
                other => {
                    return Err(error(format!("unsupported Clover line type {other}")));
                }
            };
            if let Some((old_covered, old_branch)) = lines.get_mut(&id) {
                *old_covered |= covered;
                if old_branch.as_ref().is_some_and(|b| b.total > 0)
                    || branch.as_ref().is_some_and(|b| b.total > 0)
                {
                    unknown_branches.insert(path.clone());
                    *old_branch = None;
                }
            } else {
                lines.insert(id, (covered, branch));
            }
        }
        if files.len() > MAX_FILES {
            return Err(error("too many report files"));
        }
    }
    let mut result = Vec::new();
    for (path, lines) in files {
        if !instrumented_branches.contains(&path) {
            unknown_branches.insert(path.clone());
        }
        let line_counter = (!lines.is_empty()).then(|| CoverageCounter {
            covered: lines.values().filter(|(covered, _)| *covered).count() as u64,
            total: lines.len() as u64,
        });
        let branches = if lines.is_empty() || unknown_branches.contains(&path) {
            None
        } else {
            let mut sum = CoverageCounter {
                covered: 0,
                total: 0,
            };
            for (_, branch) in lines.values() {
                let branch = branch.as_ref().expect("known branches");
                sum.covered = sum
                    .covered
                    .checked_add(branch.covered)
                    .ok_or_else(|| error("branch overflow"))?;
                sum.total = sum
                    .total
                    .checked_add(branch.total)
                    .ok_or_else(|| error("branch overflow"))?;
            }
            Some(sum)
        };
        if unknown_branches.contains(&path) && diagnostics.len() < 99 {
            diagnostics.push(format!(
                "{path}: missing or conflicting condition flags; branch coverage is unknown."
            ));
        }
        if line_counter.is_none() && diagnostics.len() < 99 {
            diagnostics.push(format!(
                "{path}: no source-line records; line coverage is unknown."
            ));
        }
        result.push(ParsedFile {
            path,
            lines: line_counter,
            branches,
            line_hits: if lines.is_empty() {
                None
            } else {
                Some(
                    lines
                        .into_iter()
                        .map(|(id, (covered, _))| {
                            Ok(CoverageLine {
                                line: u32::try_from(id)
                                    .map_err(|_| error("line number exceeds u32"))?,
                                covered,
                            })
                        })
                        .collect::<Result<Vec<_>, ArchguardError>>()?,
                )
            },
        });
    }
    Ok(("clover", result, vec![], diagnostics))
}

struct LcovFile {
    records: u32,
    hits: BTreeMap<u64, bool>,
    branch_ids: BTreeMap<(u64, u64, u64), bool>,
    lines: Option<CoverageCounter>,
    branches: Option<CoverageCounter>,
    summaries_dropped: bool,
    branch_unknown: bool,
}

impl LcovFile {
    fn new() -> Self {
        Self {
            records: 0,
            hits: BTreeMap::new(),
            branch_ids: BTreeMap::new(),
            lines: None,
            branches: None,
            summaries_dropped: false,
            branch_unknown: false,
        }
    }
}

/// Per-record LF/LH and BRF/BRH are authoritative only for a single record;
/// merging them across records would double-count shared lines, so multi-record
/// files fall back to the merged DA/BRDA detail (unknown when no detail exists).
fn parse_lcov(text: &str) -> Result<ParsedReport, ArchguardError> {
    let mut files = BTreeMap::<String, LcovFile>::new();
    let mut diagnostics = Vec::new();
    let mut active: Option<String> = None;
    let mut pending: (Option<u64>, Option<u64>, Option<u64>, Option<u64>) =
        (None, None, None, None);
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if line == "end_of_record" {
            let path = active
                .as_deref()
                .ok_or_else(|| error("LCOV end_of_record without SF"))?
                .to_string();
            commit_lcov(&mut files, &pending, Some(path))?;
            pending = (None, None, None, None);
            active = None;
            continue;
        }
        let Some((kind, value)) = line.split_once(':') else {
            return Err(error(format!("LCOV line is not a record: {line}")));
        };
        match kind {
            "SF" => {
                commit_lcov(&mut files, &pending, active.take())?;
                pending = (None, None, None, None);
                // Absolute like Cobertura: mapping strips the project root and
                // leaves outside files (dependency sources) unmatched.
                let path = clean_path(value.trim())?;
                if path.is_empty() {
                    return Err(error("empty LCOV source file path"));
                }
                let file = files.entry(path.clone()).or_insert_with(LcovFile::new);
                file.records += 1;
                if file.records > MAX_FILES as u32 {
                    return Err(error("too many report files"));
                }
                active = Some(path);
            }
            "DA" => {
                let path = active
                    .as_deref()
                    .ok_or_else(|| error("LCOV line record without SF"))?
                    .to_string();
                let fields: Vec<_> = value.split(',').collect();
                if !matches!(fields.len(), 2 | 3) {
                    return Err(error("DA needs <line>,<hits>[,<checksum>]"));
                }
                let id: u64 = fields[0]
                    .trim()
                    .parse()
                    .map_err(|_| error("invalid LCOV line number"))?;
                if id == 0 {
                    return Err(error("zero LCOV line number"));
                }
                let hits: u64 = fields[1]
                    .trim()
                    .parse()
                    .map_err(|_| error("invalid LCOV line hits"))?;
                let file = files.get_mut(&path).expect("active SF file");
                if file.hits.len() >= 500_000 && !file.hits.contains_key(&id) {
                    return Err(error("too many reported lines"));
                }
                *file.hits.entry(id).or_insert(false) |= hits > 0;
            }
            "BRDA" => {
                let path = active
                    .as_deref()
                    .ok_or_else(|| error("LCOV branch record without SF"))?
                    .to_string();
                let fields: Vec<_> = value.split(',').collect();
                if fields.len() != 4 {
                    return Err(error("BRDA needs <line>,<block>,<branch>,<taken>"));
                }
                let identity: Vec<u64> = fields[..3]
                    .iter()
                    .map(|field| {
                        field
                            .trim()
                            .parse::<u64>()
                            .map_err(|_| error("invalid LCOV branch identity"))
                    })
                    .collect::<Result<_, ArchguardError>>()?;
                let key = (identity[0], identity[1], identity[2]);
                let file = files.get_mut(&path).expect("active SF file");
                // '-' (or an unparsable outcome) means "not instrumented": unknown, not zero.
                match fields[3].trim().parse::<u64>() {
                    Ok(taken) => {
                        if file.branch_ids.len() >= 500_000 && !file.branch_ids.contains_key(&key) {
                            return Err(error("too many reported branches"));
                        }
                        // Union over repeated records: one identity counts once.
                        *file.branch_ids.entry(key).or_insert(false) |= taken > 0;
                    }
                    Err(_) => file.branch_unknown = true,
                }
            }
            "LF" | "LH" | "BRF" | "BRH" => {
                if active.is_none() {
                    return Err(error("LCOV counter without SF"));
                }
                let value: u64 = value
                    .trim()
                    .parse()
                    .map_err(|_| error(format!("invalid LCOV counter {kind}")))?;
                let slot = match kind {
                    "LF" => &mut pending.0,
                    "LH" => &mut pending.1,
                    "BRF" => &mut pending.2,
                    _ => &mut pending.3,
                };
                if slot.is_some() {
                    return Err(error(format!("duplicate LCOV counter {kind}")));
                }
                *slot = Some(value);
            }
            // TN, FN, FNDA, FNF, FNH and future fields carry no line/branch evidence.
            _ => {}
        }
    }
    commit_lcov(&mut files, &pending, active.take())?;
    if files.is_empty() {
        return Err(error("LCOV report contains no SF records"));
    }
    let mut result = Vec::new();
    for (path, file) in files {
        let covered_hits = file.hits.values().filter(|covered| **covered).count() as u64;
        let derived = (!file.hits.is_empty()).then_some(CoverageCounter {
            covered: covered_hits,
            total: file.hits.len() as u64,
        });
        let lines = if file.summaries_dropped {
            derived
        } else {
            file.lines.or(derived)
        };
        let mut branches = if file.summaries_dropped {
            None
        } else {
            file.branches
        };
        if branches.is_none() && !file.branch_ids.is_empty() && !file.branch_unknown {
            branches = Some(CoverageCounter {
                covered: file.branch_ids.values().filter(|taken| **taken).count() as u64,
                total: file.branch_ids.len() as u64,
            });
        }
        if file.branch_unknown && diagnostics.len() < 99 {
            diagnostics.push(format!(
                "{path}: branch outcome not instrumented; branch coverage is unknown."
            ));
        }
        if lines.is_none() && diagnostics.len() < 99 {
            diagnostics.push(format!(
                "{path}: no line records; line coverage is unknown."
            ));
        }
        let line_hits: Vec<_> = file
            .hits
            .iter()
            .map(|(id, covered)| {
                Ok(CoverageLine {
                    line: u32::try_from(*id).map_err(|_| error("line number exceeds u32"))?,
                    covered: *covered,
                })
            })
            .collect::<Result<_, ArchguardError>>()?;
        let line_hits = if lines.as_ref().is_some_and(|counter| {
            counter.total == line_hits.len() as u64 && counter.covered == covered_hits
        }) {
            Some(line_hits)
        } else {
            if !line_hits.is_empty() && diagnostics.len() < 99 {
                diagnostics.push(format!(
                    "{path}: source-line records differ from the line counter; changed-line coverage is unavailable."
                ));
            }
            None
        };
        result.push(ParsedFile {
            path,
            lines,
            branches,
            line_hits,
        });
    }
    Ok(("lcov", result, vec![], diagnostics))
}

fn commit_lcov(
    files: &mut BTreeMap<String, LcovFile>,
    pending: &(Option<u64>, Option<u64>, Option<u64>, Option<u64>),
    path: Option<String>,
) -> Result<(), ArchguardError> {
    let Some(path) = path else {
        if pending.0.is_some() || pending.1.is_some() || pending.2.is_some() || pending.3.is_some()
        {
            return Err(error("LCOV counter outside a record"));
        }
        return Ok(());
    };
    let file = files.get_mut(&path).expect("committed SF file");
    // A second record for one file makes per-record roll-ups ambiguous — even a
    // counter-less follow-up record adds details — so drop them entirely and
    // rely on the merged DA/BRDA detail instead.
    if file.records > 1 {
        file.summaries_dropped = true;
        file.lines = None;
        file.branches = None;
        return Ok(());
    }
    let has_lines = pending.0.is_some() || pending.1.is_some();
    let has_branches = pending.2.is_some() || pending.3.is_some();
    if !has_lines && !has_branches {
        return Ok(());
    }
    if has_lines {
        let (Some(lf), Some(lh)) = (pending.0, pending.1) else {
            return Err(error("LCOV LF/LH must appear as a pair"));
        };
        if lh > lf {
            return Err(error("LCOV LH exceeds LF"));
        }
        file.lines = Some(CoverageCounter {
            covered: lh,
            total: lf,
        });
    }
    if has_branches {
        let (Some(brf), Some(brh)) = (pending.2, pending.3) else {
            return Err(error("LCOV BRF/BRH must appear as a pair"));
        };
        if brh > brf {
            return Err(error("LCOV BRH exceeds BRF"));
        }
        file.branches = Some(CoverageCounter {
            covered: brh,
            total: brf,
        });
    }
    Ok(())
}
