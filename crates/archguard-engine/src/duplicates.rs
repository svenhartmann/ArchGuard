//! Explicit CPD observations or ArchGuard's own token scan; source hashes
//! detect changes after import or scan, not producer freshness.
use crate::evidence_import::{self as evidence, MAX_BYTES, MAX_RECORDS, Sources};
use archguard_core::{ArchguardError, Config, Graph};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    path::Path,
    time::{Duration, Instant},
};

const SNAPSHOT: &str = ".archguard/duplicates.json";
const NOTE: &str = "CPD groups are producer-reported token matches, not proven semantic equivalence or an instruction to extract a shared abstraction. Scope, thresholds, ignored identifiers/literals and producer revision remain unverified. Local hashes only detect later source changes. Importing launches no duplicate scan; `--scan` runs ArchGuard's built-in token scanner instead.";
const CPD: &str = "CPD";
const SCANNER: &str = "ArchGuard token scan";
const SCAN_NOTE: &str = "ArchGuard token scan groups are identical lexical token runs (comments and layout skipped), not proven semantic equivalence or an instruction to extract a shared abstraction. Tokens come from ArchGuard's lightweight lexer, not a compiler; one punctuation character is one token, so counts differ from PMD CPD. Local hashes only detect later source changes.";
/// Largest groups kept in a scan snapshot; each holds a snippet of up to 4 KiB.
const MAX_SCAN_GROUPS: usize = 5_000;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateOccurrence {
    pub path: String,
    pub node: Option<String>,
    pub line: u32,
    pub end_line: u32,
    pub column: Option<u32>,
    pub end_column: Option<u32>,
    pub source_sha256: Option<String>,
    pub status: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateGroup {
    pub id: String,
    pub lines: u32,
    pub tokens: u32,
    pub status: String,
    pub occurrences: Vec<DuplicateOccurrence>,
    pub snippet: String,
    pub snippet_truncated: bool,
    pub fragment_sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicationReport {
    pub schema_version: u32,
    pub status: String,
    /// `CPD` for an imported report, `ArchGuard token scan` for the built-in scanner.
    #[serde(default = "cpd")]
    pub producer: String,
    pub report_path: Option<String>,
    pub imported_at_unix: Option<u64>,
    pub producer_version: Option<String>,
    pub producer_timestamp: Option<String>,
    pub producer_errors: usize,
    pub groups: Vec<DuplicateGroup>,
    pub diagnostics: Vec<String>,
}
fn error(message: impl std::fmt::Display) -> ArchguardError {
    ArchguardError::Analysis(format!("duplication evidence: {message}"))
}
fn cpd() -> String {
    CPD.into()
}
fn empty(status: &str) -> DuplicationReport {
    DuplicationReport {
        schema_version: 1,
        status: status.into(),
        producer: cpd(),
        report_path: None,
        imported_at_unix: None,
        producer_version: None,
        producer_timestamp: None,
        producer_errors: 0,
        groups: vec![],
        diagnostics: vec![NOTE.into()],
    }
}
pub fn duplication_fingerprint(root: &Path) -> String {
    evidence::fingerprint(root, SNAPSHOT)
}
fn number(node: roxmltree::Node<'_, '_>, name: &str) -> Result<u32, ArchguardError> {
    let value = node
        .attribute(name)
        .ok_or_else(|| error(format!("missing {name}")))?
        .parse::<u32>()
        .map_err(error)?;
    if value == 0 {
        return Err(error(format!("{name} must be positive")));
    }
    Ok(value)
}
fn location(o: &DuplicateOccurrence) -> (&str, u32, u32, Option<u32>, Option<u32>) {
    (&o.path, o.line, o.end_line, o.column, o.end_column)
}
fn identity(occurrences: &[DuplicateOccurrence]) -> Result<String, ArchguardError> {
    Ok(evidence::hash(
        &serde_json::to_vec(&occurrences.iter().map(location).collect::<Vec<_>>())
            .map_err(error)?,
    ))
}
fn source_path(root: &Path, sources: &Sources<'_>, raw: &str) -> Result<String, ArchguardError> {
    let normalized = raw.replace('\\', "/");
    if raw.chars().any(char::is_control) || normalized.split('/').any(|p| p == "..") {
        return Err(error(
            "source path contains traversal or control characters",
        ));
    }
    let path = Path::new(&normalized);
    if path.is_absolute() {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_owned());
        let relative=canonical.strip_prefix(root).map_err(|_|error("absolute CPD paths must remain inside the analyzed project; use project-relative paths for reports from another checkout"))?;
        evidence::relative(&relative.to_string_lossy())
    } else {
        sources.path(&normalized)
    }
}
fn update_status(report: &mut DuplicationReport) {
    for group in &mut report.groups {
        group.status = if group
            .occurrences
            .iter()
            .any(|o| o.status == "changed_since_import")
        {
            "stale"
        } else if group
            .occurrences
            .iter()
            .any(|o| o.status != "unchanged_since_import")
        {
            "partial"
        } else {
            "ready"
        }
        .into();
    }
    report.status = if report.groups.iter().any(|g| g.status == "stale") {
        "stale"
    } else if report.producer_errors > 0 || report.groups.iter().any(|g| g.status == "partial") {
        "partial"
    } else {
        "ready"
    }
    .into();
}
fn validate(report: &DuplicationReport) -> Result<(), ArchguardError> {
    if report.schema_version != 1 || report.groups.len() > MAX_RECORDS {
        return Err(error("invalid snapshot schema or group count"));
    }
    let mut ids = BTreeSet::new();
    let mut total = 0;
    for group in &report.groups {
        total += group.occurrences.len();
        if total > MAX_RECORDS
            || group.occurrences.len() < 2
            || group.lines == 0
            || group.tokens == 0
            || !evidence::valid_hash(&group.fragment_sha256)
            || group.snippet.chars().count() > 4096
        {
            return Err(error("invalid duplicate group or location count"));
        }
        let mut previous = None;
        for o in &group.occurrences {
            if evidence::relative(&o.path)? != o.path
                || o.line == 0
                || o.end_line < o.line
                || o.column == Some(0)
                || o.end_column == Some(0)
                || o.source_sha256
                    .as_ref()
                    .is_some_and(|h| !evidence::valid_hash(h))
            {
                return Err(error("invalid duplicate source location"));
            }
            if o.line == o.end_line && o.column.zip(o.end_column).is_some_and(|(a, b)| b < a) {
                return Err(error("duplicate columns are reversed"));
            }
            let current = location(o);
            if previous.is_some_and(|p| p >= current) {
                return Err(error("duplicate locations must be unique and sorted"));
            }
            previous = Some(current);
        }
        if group.id != identity(&group.occurrences)? || !ids.insert(&group.id) {
            return Err(error("duplicate group identity is invalid or repeated"));
        }
    }
    Ok(())
}
pub fn import_duplicates(
    root: &Path,
    config: &Config,
    graph: &Graph,
    path: &Path,
    source_root: Option<&Path>,
) -> Result<DuplicationReport, ArchguardError> {
    let root = root.canonicalize()?;
    let bytes = evidence::read_input(&root, path)?;
    let xml = std::str::from_utf8(&bytes).map_err(error)?;
    let document = roxmltree::Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 500_000,
            ..Default::default()
        },
    )
    .map_err(error)?;
    let top = document.root_element();
    if top.tag_name().name() != "pmd-cpd" || top.attribute("version").is_some_and(|v| v != "1.0.0")
    {
        return Err(error(
            "expected a CPD XML report (schema 1.0.0 or legacy without schema version)",
        ));
    }
    let mut sources = Sources::new(&root, config, graph, source_root)?;
    let mut report = empty("ready");
    report.report_path = Some(path.to_string_lossy().into_owned());
    report.imported_at_unix = evidence::now();
    report.producer_version = top.attribute("pmdVersion").map(str::to_owned);
    report.producer_timestamp = top.attribute("timestamp").map(str::to_owned);
    let mut locations = 0;
    for child in top.children().filter(|n| n.is_element()) {
        match child.tag_name().name() {
            "file" => {}
            "error" => {
                report.producer_errors += 1;
                if report.diagnostics.len() < 100 {
                    report.diagnostics.push(
                        format!(
                            "CPD error: {} · {}",
                            child.attribute("filename").unwrap_or(""),
                            child.attribute("msg").unwrap_or("producer error")
                        )
                        .chars()
                        .take(4096)
                        .collect(),
                    );
                }
            }
            "duplication" => {
                let lines = number(child, "lines")?;
                let tokens = number(child, "tokens")?;
                let mut occurrences = vec![];
                let mut fragment = None;
                for item in child.children().filter(|n| n.is_element()) {
                    match item.tag_name().name() {
                        "file" => {
                            locations += 1;
                            if locations > MAX_RECORDS {
                                return Err(error("too many duplicate source locations"));
                            }
                            let path = source_path(
                                &root,
                                &sources,
                                item.attribute("path")
                                    .ok_or_else(|| error("missing CPD source path"))?,
                            )?;
                            let line = number(item, "line")?;
                            let end_line = if item.has_attribute("endline") {
                                number(item, "endline")?
                            } else {
                                line.checked_add(lines - 1)
                                    .ok_or_else(|| error("line range overflow"))?
                            };
                            let column = item
                                .attribute("column")
                                .map(|_| number(item, "column"))
                                .transpose()?;
                            let end_column = item
                                .attribute("endcolumn")
                                .map(|_| number(item, "endcolumn"))
                                .transpose()?;
                            let source = sources.get(&path);
                            if source.is_some_and(|s| end_line > s.lines) {
                                return Err(error(format!(
                                    "CPD range exceeds current source: {path}"
                                )));
                            }
                            occurrences.push(DuplicateOccurrence {
                                path,
                                node: source.map(|s| s.node.clone()),
                                line,
                                end_line,
                                column,
                                end_column,
                                source_sha256: source.map(|s| s.hash.clone()),
                                status: if source.is_some() {
                                    "unchanged_since_import"
                                } else {
                                    "unmapped"
                                }
                                .into(),
                            });
                        }
                        "codefragment" => {
                            if fragment.replace(item.text().unwrap_or("")).is_some() {
                                return Err(error("repeated CPD codefragment"));
                            }
                        }
                        tag => {
                            return Err(error(format!(
                                "unsupported CPD duplication element: {tag}"
                            )));
                        }
                    }
                }
                occurrences.sort_by(|a, b| location(a).cmp(&location(b)));
                let fragment = fragment.unwrap_or("");
                report.groups.push(DuplicateGroup {
                    id: identity(&occurrences)?,
                    lines,
                    tokens,
                    status: String::new(),
                    occurrences,
                    snippet: fragment.chars().take(4096).collect(),
                    snippet_truncated: fragment.chars().count() > 4096,
                    fragment_sha256: evidence::hash(fragment.as_bytes()),
                });
            }
            tag => return Err(error(format!("unsupported CPD report element: {tag}"))),
        }
    }
    report.groups.sort_by(|a, b| a.id.cmp(&b.id));
    validate(&report)?;
    update_status(&mut report);
    evidence::save(&root, "duplicates.json", &report)?;
    Ok(report)
}
pub fn read_duplicates(root: &Path, config: &Config, graph: &Graph) -> DuplicationReport {
    let read = || -> Result<DuplicationReport, ArchguardError> {
        let bytes = match crate::project_files::read_repo_file(root, Path::new(SNAPSHOT), MAX_BYTES)
        {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(empty("not_imported")),
            Err(e) => return Err(e.into()),
        };
        let mut report: DuplicationReport = serde_json::from_slice(&bytes).map_err(error)?;
        validate(&report)?;
        let mut sources = Sources::new(root, config, graph, None)?;
        for group in &mut report.groups {
            for o in &mut group.occurrences {
                o.status = match (sources.get(&o.path), o.source_sha256.as_ref()) {
                    (Some(source), Some(hash)) => {
                        o.node = Some(source.node.clone());
                        if hash == &source.hash && o.end_line <= source.lines {
                            "unchanged_since_import"
                        } else {
                            "changed_since_import"
                        }
                    }
                    _ => {
                        o.node = None;
                        "unmapped"
                    }
                }
                .into();
            }
        }
        update_status(&mut report);
        Ok(report)
    };
    read().unwrap_or_else(|e| {
        let mut report = empty("error");
        report.diagnostics.push(e.to_string());
        report
    })
}

/// Limits and normalization for [`scan_duplicates`].
#[derive(Debug, Clone)]
pub struct DuplicateScanOptions {
    /// Shortest reported clone in lexical tokens.
    pub min_tokens: usize,
    /// Treat identifiers and literals as equal (Type-2 clones).
    pub normalize: bool,
    /// Overwrite an imported CPD snapshot instead of refusing.
    pub replace: bool,
    pub max_file_bytes: u64,
    pub max_tokens: usize,
    pub time_limit: Duration,
}
impl Default for DuplicateScanOptions {
    fn default() -> Self {
        Self {
            min_tokens: archguard_extract::clones::DEFAULT_MIN_TOKENS,
            normalize: false,
            replace: false,
            max_file_bytes: 1024 * 1024,
            max_tokens: 5_000_000,
            time_limit: Duration::from_secs(120),
        }
    }
}
/// Refuse to overwrite anything but an earlier built-in scan unless `replace`.
fn guard_snapshot(root: &Path, replace: bool) -> Result<(), ArchguardError> {
    let bytes = match crate::project_files::read_repo_file(root, Path::new(SNAPSHOT), MAX_BYTES) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let producer = serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|v| {
            v.get("producer")
                .and_then(|p| p.as_str())
                .map(str::to_owned)
        });
    if replace || producer.as_deref() == Some(SCANNER) {
        return Ok(());
    }
    Err(error(format!(
        "{SNAPSHOT} holds an imported {} report; pass --replace to overwrite it with a built-in scan",
        producer.unwrap_or_else(cpd)
    )))
}
/// A readable source file of the scan: path, language, text and hash.
struct ScanFile {
    path: String,
    lang: archguard_extract::Lang,
    text: String,
    hash: String,
}
fn read_scan_files(
    root: &Path,
    config: &Config,
    max_bytes: u64,
    report: &mut DuplicationReport,
) -> Result<Vec<ScanFile>, ArchguardError> {
    let mut files = vec![];
    for file in crate::pipeline::walk_files(root, config)? {
        let read = crate::project_files::read_repo_file(root, Path::new(&file.rel_path), max_bytes)
            .map_err(|e| e.to_string())
            .and_then(|b| {
                let hash = evidence::hash(&b);
                String::from_utf8(b)
                    .map(|text| (text, hash))
                    .map_err(|_| "not UTF-8".into())
            });
        match read {
            Ok((text, hash)) => files.push(ScanFile {
                path: file.rel_path,
                lang: file.lang,
                text,
                hash,
            }),
            Err(e) => {
                report.producer_errors += 1;
                if report.diagnostics.len() < 100 {
                    report.diagnostics.push(format!(
                        "skipped {}: {e} (limit {max_bytes} bytes)",
                        file.rel_path
                    ));
                }
            }
        }
    }
    Ok(files)
}
fn scan_group(
    files: &[ScanFile],
    sources: &Sources<'_>,
    clone: &archguard_extract::CloneGroup,
) -> Result<DuplicateGroup, ArchguardError> {
    let mut occurrences = vec![];
    for at in &clone.occurrences {
        let file = &files[at.file];
        let node = sources.node(&file.path);
        occurrences.push(DuplicateOccurrence {
            path: file.path.clone(),
            status: if node.is_some() {
                "unchanged_since_import"
            } else {
                "unmapped"
            }
            .into(),
            node,
            line: at.line,
            end_line: at.end_line,
            column: Some(at.column),
            end_column: Some(at.end_column),
            source_sha256: Some(file.hash.clone()),
        });
    }
    occurrences.sort_by(|a, b| location(a).cmp(&location(b)));
    let first = &clone.occurrences[0];
    let fragment = files[first.file]
        .text
        .lines()
        .skip(first.line as usize - 1)
        .take((first.end_line - first.line + 1) as usize)
        .collect::<Vec<_>>()
        .join("\n");
    Ok(DuplicateGroup {
        id: identity(&occurrences)?,
        lines: first.end_line - first.line + 1,
        tokens: u32::try_from(clone.tokens).map_err(error)?,
        status: String::new(),
        occurrences,
        snippet: fragment.chars().take(4096).collect(),
        snippet_truncated: fragment.chars().count() > 4096,
        fragment_sha256: evidence::hash(fragment.as_bytes()),
    })
}
/// Keep at most [`MAX_SCAN_GROUPS`], largest first, and say so.
fn cap_groups(
    mut clones: Vec<archguard_extract::CloneGroup>,
    report: &mut DuplicationReport,
) -> Vec<archguard_extract::CloneGroup> {
    if clones.len() > MAX_SCAN_GROUPS {
        report.producer_errors += 1;
        report.diagnostics.push(format!(
            "Scan truncated: kept the {MAX_SCAN_GROUPS} largest of {} groups",
            clones.len()
        ));
        clones.sort_by_key(|g| std::cmp::Reverse(g.tokens));
        clones.truncate(MAX_SCAN_GROUPS);
    }
    clones
}
/// Run the built-in token clone scanner over the project's walked sources
/// and save the result where an imported CPD report would live.
pub fn scan_duplicates(
    root: &Path,
    config: &Config,
    graph: &Graph,
    options: &DuplicateScanOptions,
) -> Result<DuplicationReport, ArchguardError> {
    let root = root.canonicalize()?;
    guard_snapshot(&root, options.replace)?;
    let mut report = empty("ready");
    report.producer = SCANNER.into();
    report.producer_version = Some(format!("archguard {}", env!("CARGO_PKG_VERSION")));
    report.imported_at_unix = evidence::now();
    report.diagnostics = vec![SCAN_NOTE.into()];
    let files = read_scan_files(&root, config, options.max_file_bytes, &mut report)?;
    let inputs: Vec<_> = files.iter().map(|f| (f.lang, f.text.as_str())).collect();
    let scan = archguard_extract::find_clones(
        &inputs,
        &archguard_extract::CloneOptions {
            min_tokens: options.min_tokens,
            normalize: options.normalize,
            max_tokens: options.max_tokens,
            deadline: Some(Instant::now() + options.time_limit),
        },
    );
    report.diagnostics.insert(
        1,
        format!(
            "Scan settings: min_tokens={} normalize={} · {} files, {} tokens scanned",
            options.min_tokens, options.normalize, scan.files_scanned, scan.tokens_scanned
        ),
    );
    if let Some(reason) = &scan.truncated {
        report.producer_errors += 1;
        report.diagnostics.push(format!("Scan truncated: {reason}"));
    }
    let sources = Sources::new(&root, config, graph, None)?;
    for clone in &cap_groups(scan.groups, &mut report) {
        report.groups.push(scan_group(&files, &sources, clone)?);
    }
    report.groups.sort_by(|a, b| a.id.cmp(&b.id));
    validate(&report)?;
    update_status(&mut report);
    evidence::save(&root, "duplicates.json", &report)?;
    Ok(report)
}
