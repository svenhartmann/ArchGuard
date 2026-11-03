//! Import mutation-testing-elements reports, preserving outcomes and source provenance.
use crate::evidence_import::{
    self as evidence, MAX_BYTES, MAX_RECORDS, Sources, error, hash, relative, valid_hash,
};
use archguard_core::{ArchguardError, Config, Graph};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutationPosition {
    pub line: u32,
    pub column: u32,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutationLocation {
    pub start: MutationPosition,
    pub end: MutationPosition,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MutationRecord {
    pub id: String,
    pub mutator_name: String,
    pub status: String,
    pub location: MutationLocation,
    pub description: Option<String>,
    pub replacement: Option<String>,
    pub original: String,
    pub original_truncated: bool,
    pub duration_ms: Option<f64>,
    pub covered_by: Vec<String>,
    pub killed_by: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MutationFile {
    pub path: String,
    pub node: Option<String>,
    pub language: String,
    pub source_sha256: String,
    pub status: String,
    pub mutants: Vec<MutationRecord>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct MutationSummary {
    pub total: usize,
    pub killed: usize,
    pub survived: usize,
    pub no_coverage: usize,
    pub timeouts: usize,
    pub pending: usize,
    pub ignored: usize,
    pub compile_errors: usize,
    pub runtime_errors: usize,
    pub killed_percent: Option<f64>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MutationReport {
    pub schema_version: u32,
    pub status: String,
    pub report_path: Option<String>,
    pub imported_at_unix: Option<u64>,
    pub producer: Option<String>,
    pub producer_version: Option<String>,
    pub files: Vec<MutationFile>,
    pub summary: MutationSummary,
    pub diagnostics: Vec<String>,
}
const SNAPSHOT: &str = ".archguard/mutation.json";
const DEFINITION: &str = "Killed percentage = Killed / (Killed + Survived + NoCoverage). Ignored and CompileError mutants are excluded. Timeout, Pending, RuntimeError, unmapped or changed sources make the percentage unavailable; this is not the producer's mutation score.";
const REVISION: &str = "Source contents are compared exactly with the producer's embedded original source. Test, dependency and tool-configuration freshness remains unverified. No mutation tests are executed. Locations/snippets use one-based UTF-16 columns and an exclusive end.";
fn empty(status: &str) -> MutationReport {
    MutationReport {
        schema_version: 1,
        status: status.into(),
        report_path: None,
        imported_at_unix: None,
        producer: None,
        producer_version: None,
        files: vec![],
        summary: MutationSummary::default(),
        diagnostics: vec![DEFINITION.into(), REVISION.into()],
    }
}
fn diagnostic(report: &mut MutationReport, message: impl Into<String>) {
    let m = message.into();
    if report.diagnostics.len() < 100 && !report.diagnostics.contains(&m) {
        report.diagnostics.push(m);
    }
}
pub fn mutation_fingerprint(root: &Path) -> String {
    evidence::fingerprint(root, SNAPSHOT)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Input {
    schema_version: String,
    thresholds: Thresholds,
    files: BTreeMap<String, InputFile>,
    framework: Option<Framework>,
}
#[derive(Deserialize)]
struct Thresholds {
    high: u8,
    low: u8,
}
#[derive(Deserialize)]
struct Framework {
    name: String,
    version: Option<String>,
}
#[derive(Deserialize)]
struct InputFile {
    language: String,
    source: String,
    mutants: Vec<InputMutant>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InputMutant {
    id: String,
    mutator_name: String,
    status: String,
    location: MutationLocation,
    description: Option<String>,
    replacement: Option<String>,
    duration: Option<f64>,
    #[serde(default)]
    covered_by: Vec<String>,
    #[serde(default)]
    killed_by: Vec<String>,
}
fn valid_status(status: &str) -> bool {
    matches!(
        status,
        "Killed"
            | "Survived"
            | "NoCoverage"
            | "CompileError"
            | "RuntimeError"
            | "Timeout"
            | "Ignored"
            | "Pending"
    )
}
fn valid_location(l: &MutationLocation) -> bool {
    l.start.line > 0
        && l.start.column > 0
        && l.end.line > 0
        && l.end.column > 0
        && (l.end.line, l.end.column) >= (l.start.line, l.start.column)
}
fn valid_duration(value: Option<f64>) -> bool {
    value.is_none_or(|n| n.is_finite() && n >= 0.0)
}
fn offset(source: &str, position: &MutationPosition) -> Result<usize, ArchguardError> {
    let mut base = 0;
    for (i, raw) in source.split('\n').enumerate() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if i + 1 == position.line as usize {
            let mut column = 1u32;
            for (offset, c) in line.char_indices() {
                if column == position.column {
                    return Ok(base + offset);
                }
                column += c.len_utf16() as u32;
            }
            if column == position.column {
                return Ok(base + line.len());
            }
            return Err(error(
                "mutation column lies outside source or inside a UTF-16 surrogate pair",
            ));
        }
        base += raw.len() + 1;
    }
    Err(error("mutation line lies outside reported source"))
}
fn original(source: &str, location: &MutationLocation) -> Result<(String, bool), ArchguardError> {
    if !valid_location(location) {
        return Err(error("invalid mutation range"));
    }
    let start = offset(source, &location.start)?;
    let end = offset(source, &location.end)?;
    let text = &source[start..end];
    let truncated = text.chars().count() > 512;
    Ok((text.chars().take(512).collect(), truncated))
}
fn summarize(report: &mut MutationReport) {
    let mut s = MutationSummary::default();
    for mutant in report.files.iter().flat_map(|file| &file.mutants) {
        s.total += 1;
        match mutant.status.as_str() {
            "Killed" => s.killed += 1,
            "Survived" => s.survived += 1,
            "NoCoverage" => s.no_coverage += 1,
            "Timeout" => s.timeouts += 1,
            "Pending" => s.pending += 1,
            "Ignored" => s.ignored += 1,
            "CompileError" => s.compile_errors += 1,
            "RuntimeError" => s.runtime_errors += 1,
            _ => {}
        }
    }
    let denominator = s.killed + s.survived + s.no_coverage;
    if denominator > 0
        && s.timeouts + s.pending + s.runtime_errors == 0
        && report
            .files
            .iter()
            .all(|file| file.status == "matches_reported_source")
    {
        s.killed_percent = Some(s.killed as f64 / denominator as f64 * 100.0);
    }
    report.status = if report
        .files
        .iter()
        .any(|f| f.status == "changed_since_report")
    {
        "stale"
    } else if s.total == 0
        || s.timeouts + s.pending + s.runtime_errors > 0
        || report.files.iter().any(|f| f.status == "unmapped")
    {
        "partial"
    } else {
        "ready"
    }
    .into();
    report.summary = s;
}
/// Import mutation-testing-elements schema 1/2; source files are read only after exact graph mapping.
pub fn import_mutations(
    root: &Path,
    config: &Config,
    graph: &Graph,
    report_path: &Path,
    source_root: Option<&Path>,
) -> Result<MutationReport, ArchguardError> {
    let input: Input =
        serde_json::from_slice(&evidence::read_input(root, report_path)?).map_err(error)?;
    let version: Vec<_> = input.schema_version.split('.').collect();
    if !matches!(version.first().copied(), Some("1" | "2"))
        || version.len() > 3
        || version.iter().any(|p| {
            p.is_empty()
                || !p.bytes().all(|c| c.is_ascii_digit())
                || (p.len() > 1 && p.starts_with('0'))
        })
        || input.thresholds.high > 100
        || input.thresholds.low > 100
        || input.thresholds.low > input.thresholds.high
    {
        return Err(error(
            "unsupported mutation schema version or invalid thresholds",
        ));
    }
    if input.files.len() > MAX_RECORDS
        || input.files.values().map(|f| f.mutants.len()).sum::<usize>() > MAX_RECORDS
    {
        return Err(error("mutation record limit exceeded"));
    }
    let mut sources = Sources::new(root, config, graph, source_root)?;
    let mut report = empty("ready");
    report.report_path = Some(report_path.to_string_lossy().into_owned());
    report.imported_at_unix = evidence::now();
    if let Some(f) = input.framework {
        if f.name.is_empty() {
            return Err(error("mutation framework name is empty"));
        }
        report.producer = Some(f.name);
        report.producer_version = f.version;
    }
    let mut paths = BTreeSet::new();
    for (path, file) in input.files {
        let path = sources.path(&path)?;
        if !paths.insert(path.clone())
            || file.language.is_empty()
            || file.source.len() as u64 > evidence::MAX_SOURCE_BYTES
        {
            return Err(error(
                "duplicate normalized source path, empty language or oversized mutation source",
            ));
        }
        let source_sha256 = hash(file.source.as_bytes());
        let (node, status) = match sources.get(&path) {
            Some(s) => (
                Some(s.node.clone()),
                if s.hash == source_sha256 {
                    "matches_reported_source"
                } else {
                    "changed_since_report"
                },
            ),
            None => (None, "unmapped"),
        };
        if status != "matches_reported_source" {
            diagnostic(
                &mut report,
                format!(
                    "{path}: {status}; mutations remain historical/unassigned and no current percentage is reported."
                ),
            );
        }
        let mut ids = BTreeSet::new();
        let mut mutants = Vec::new();
        for m in file.mutants {
            if m.id.is_empty()
                || !ids.insert(m.id.clone())
                || m.mutator_name.is_empty()
                || !valid_status(&m.status)
                || !valid_duration(m.duration)
                || m.covered_by.len() > MAX_RECORDS
                || m.killed_by.len() > MAX_RECORDS
            {
                return Err(error(
                    "invalid/duplicate mutation identity, status or duration",
                ));
            }
            let (original, original_truncated) = original(&file.source, &m.location)?;
            mutants.push(MutationRecord {
                id: m.id,
                mutator_name: m.mutator_name,
                status: m.status,
                location: m.location,
                description: m.description,
                replacement: m.replacement,
                original,
                original_truncated,
                duration_ms: m.duration,
                covered_by: m.covered_by,
                killed_by: m.killed_by,
            });
        }
        report.files.push(MutationFile {
            path,
            node,
            language: file.language,
            source_sha256,
            status: status.into(),
            mutants,
        });
    }
    summarize(&mut report);
    evidence::save(root, "mutation.json", &report)?;
    Ok(report)
}
pub fn read_mutations(root: &Path, config: &Config, graph: &Graph) -> MutationReport {
    let result = (|| -> Result<MutationReport, ArchguardError> {
        let bytes = match crate::project_files::read_repo_file(root, Path::new(SNAPSHOT), MAX_BYTES)
        {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(empty("not_imported")),
            Err(e) => return Err(e.into()),
        };
        let mut report: MutationReport = serde_json::from_slice(&bytes).map_err(error)?;
        let mut paths = BTreeSet::new();
        if report.schema_version != 1
            || !matches!(report.status.as_str(), "ready" | "partial" | "stale")
            || report.diagnostics.len() > 100
            || report.files.len() > MAX_RECORDS
            || report.files.iter().map(|f| f.mutants.len()).sum::<usize>() > MAX_RECORDS
            || report.files.iter().any(|f| {
                let mut ids = BTreeSet::new();
                !paths.insert(f.path.clone())
                    || !relative(&f.path).is_ok_and(|p| p == f.path)
                    || !valid_hash(&f.source_sha256)
                    || f.language.is_empty()
                    || !matches!(
                        f.status.as_str(),
                        "matches_reported_source" | "changed_since_report" | "unmapped"
                    )
                    || f.mutants.iter().any(|m| {
                        m.id.is_empty()
                            || !ids.insert(m.id.clone())
                            || m.mutator_name.is_empty()
                            || !valid_status(&m.status)
                            || !valid_location(&m.location)
                            || !valid_duration(m.duration_ms)
                            || m.original.chars().count() > 512
                            || m.covered_by.len() > MAX_RECORDS
                            || m.killed_by.len() > MAX_RECORDS
                    })
            })
        {
            return Err(error("invalid mutation snapshot"));
        }
        let mut sources = Sources::new(root, config, graph, None)?;
        for file in &mut report.files {
            match sources.get(&file.path) {
                Some(s) if s.hash == file.source_sha256 => {
                    for m in &file.mutants {
                        original(&s.text, &m.location)?;
                    }
                    file.node = Some(s.node.clone());
                    file.status = "matches_reported_source".into();
                }
                Some(s) => {
                    file.node = Some(s.node.clone());
                    file.status = "changed_since_report".into();
                }
                None => {
                    file.node = None;
                    file.status = "unmapped".into();
                }
            }
        }
        diagnostic(&mut report, DEFINITION);
        diagnostic(&mut report, REVISION);
        summarize(&mut report);
        Ok(report)
    })();
    match result {
        Ok(r) => r,
        Err(e) => {
            let mut r = empty("error");
            diagnostic(&mut r, e.to_string());
            r
        }
    }
}
