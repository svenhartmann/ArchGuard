//! Explicit SARIF imports; no analyzer execution or remote artifact fetching.
use crate::pipeline::NamedGraph;
use archguard_core::{ArchguardError, Config, Graph};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

const SNAPSHOT: &str = ".archguard/quality.json";
const MAX_BYTES: u64 = 32 * 1024 * 1024;
const MAX_SOURCE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_FINDINGS: usize = 100_000;
const REVISION_NOTE: &str = "Report/source revision at import is unverified. Hashes detect later changes to mapped source files, not analyzer configuration, tests or dependency changes. Ready means this SARIF import was processed, not that the project has no quality problems.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualityFinding {
    pub id: String,
    pub tool: String,
    pub rule_id: String,
    pub level: String,
    pub message: String,
    pub path: Option<String>,
    pub node: Option<String>,
    pub line: Option<u32>,
    pub column: Option<u32>,
    pub source_sha256: Option<String>,
    pub status: String,
    pub help_uri: Option<String>,
    pub suppressed: bool,
    /// SARIF producer comparison state; `absent` is historical, not active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_state: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QualityReport {
    pub schema_version: u32,
    pub status: String,
    pub report_path: Option<String>,
    pub imported_at_unix: Option<u64>,
    pub tools: Vec<String>,
    pub findings: Vec<QualityFinding>,
    pub diagnostics: Vec<String>,
}
impl QualityReport {
    fn empty(status: &str) -> Self {
        Self {
            schema_version: 1,
            status: status.into(),
            report_path: None,
            imported_at_unix: None,
            tools: vec![],
            findings: vec![],
            diagnostics: vec![REVISION_NOTE.into()],
        }
    }
    fn diagnostic(&mut self, message: impl Into<String>) {
        if self.diagnostics.len() < 100 {
            self.diagnostics.push(message.into());
        }
    }
    fn partial(&mut self, message: impl Into<String>) {
        self.status = "partial".into();
        self.diagnostic(message);
    }
}
fn error(message: impl std::fmt::Display) -> ArchguardError {
    ArchguardError::Analysis(format!("quality findings: {message}"))
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn valid_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
fn valid_level(value: &str) -> bool {
    matches!(value, "error" | "warning" | "note" | "none")
}
fn valid_baseline(value: &str) -> bool {
    matches!(value, "new" | "unchanged" | "updated" | "absent")
}
fn safe_help(value: &str) -> bool {
    (value.starts_with("https://") || value.starts_with("http://"))
        && !value.chars().any(|c| c.is_control() || c == '\\')
}

/// A change to the saved report invalidates the live-viewer cache.
pub fn quality_findings_fingerprint(root: &Path) -> String {
    match crate::project_files::read_repo_file(root, Path::new(SNAPSHOT), MAX_BYTES) {
        Ok(bytes) => hash(&bytes),
        Err(e) => format!("unavailable:{:?}", e.kind()),
    }
}
fn source_paths<'a>(
    root: &Path,
    config: &Config,
    graph: impl Into<NamedGraph<'a>>,
) -> Result<BTreeMap<String, String>, ArchguardError> {
    let graph = graph.into();
    let nodes: BTreeMap<_, _> = graph
        .graph
        .nodes
        .iter()
        .map(|node| (node.id, &node.name))
        .collect();
    let mut candidates = BTreeMap::<String, Vec<String>>::new();
    for source in crate::review_sources::collect_source_evidence(root, config, graph)? {
        if let (Some(path), Some(node)) = (source.path, nodes.get(&source.node_id)) {
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

/// Read normalized evidence and recheck mapped source files, never execute tools.
/// Pass `(graph, names)` from the run that built `graph` to skip re-extraction.
pub fn read_quality_findings<'a>(
    root: &Path,
    config: &Config,
    graph: impl Into<NamedGraph<'a>>,
) -> QualityReport {
    let bytes = match crate::project_files::read_repo_file(root, Path::new(SNAPSHOT), MAX_BYTES) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return QualityReport::empty("not_imported");
        }
        Err(e) => {
            let mut report = QualityReport::empty("error");
            report.diagnostic(format!("Cannot read {SNAPSHOT}: {e}"));
            return report;
        }
    };
    let mut report: QualityReport = match serde_json::from_slice(&bytes) {
        Ok(report) => report,
        Err(e) => {
            let mut report = QualityReport::empty("error");
            report.diagnostic(format!("Invalid quality snapshot: {e}"));
            return report;
        }
    };
    let mut ids = BTreeSet::new();
    if report.schema_version != 1
        || !matches!(report.status.as_str(), "ready" | "partial")
        || report.findings.len() > MAX_FINDINGS
        || report.diagnostics.len() > 100
        || report.tools.len() > 1000
        || report.tools.iter().any(String::is_empty)
        || report.findings.iter().any(|f| {
            !ids.insert(&f.id)
                || !valid_hash(&f.id)
                || !report.tools.contains(&f.tool)
                || !valid_level(&f.level)
                || f.line == Some(0)
                || f.column == Some(0)
                || f.baseline_state
                    .as_deref()
                    .is_some_and(|s| !valid_baseline(s))
                || f.help_uri.as_deref().is_some_and(|uri| !safe_help(uri))
                || match f.status.as_str() {
                    "unchanged_since_import" => {
                        f.path
                            .as_deref()
                            .is_none_or(|p| relative(p).as_deref() != Ok(p))
                            || f.node.as_deref().is_none_or(str::is_empty)
                            || f.source_sha256.as_deref().is_none_or(|s| !valid_hash(s))
                    }
                    "unmapped" => {
                        f.node.is_some()
                            || f.source_sha256.is_some()
                            || f.path
                                .as_deref()
                                .is_some_and(|p| relative(p).as_deref() != Ok(p))
                    }
                    _ => true,
                }
        })
    {
        let mut report = QualityReport::empty("error");
        report.diagnostic("Unsupported or inconsistent quality snapshot; import SARIF again.");
        return report;
    }
    if !report.diagnostics.iter().any(|d| d == REVISION_NOTE) {
        report.diagnostic(REVISION_NOTE);
    }
    let paths = match source_paths(root, config, graph) {
        Ok(paths) => paths,
        Err(e) => {
            report.status = "error".into();
            report.findings.clear();
            report.diagnostic(e.to_string());
            return report;
        }
    };
    let mut hashes = BTreeMap::<String, Option<String>>::new();
    let mut stale = 0;
    for finding in &mut report.findings {
        if finding.status != "unchanged_since_import" {
            continue;
        }
        let path = finding.path.as_ref().expect("validated mapped path");
        let current = hashes.entry(path.clone()).or_insert_with(|| {
            crate::project_files::read_repo_file(root, Path::new(path), MAX_SOURCE_BYTES)
                .ok()
                .map(|bytes| hash(&bytes))
        });
        if paths.get(path) != finding.node.as_ref()
            || current.as_ref() != finding.source_sha256.as_ref()
        {
            finding.status = "changed_since_import".into();
            stale += 1;
        }
    }
    if stale > 0 {
        report.status = "stale".into();
        report.diagnostic(format!("{stale} findings refer to source files changed, removed or outside the current analysis scope since import."));
    }
    report
}

/// Replace the saved import only after the entire bounded report is validated.
pub fn import_quality_findings(
    root: &Path,
    config: &Config,
    graph: &Graph,
    report_path: &Path,
    source_root: Option<&Path>,
) -> Result<QualityReport, ArchguardError> {
    let input = if report_path.is_absolute() {
        report_path.to_path_buf()
    } else {
        root.join(report_path)
    };
    let file = fs::File::open(input)?;
    if !file.metadata()?.is_file() || file.metadata()?.len() > MAX_BYTES {
        return Err(error(
            "SARIF input must be a regular file of at most 32 MiB",
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(error("SARIF input exceeds 32 MiB"));
    }
    let json: Value = serde_json::from_slice(&bytes).map_err(error)?;
    if json["version"] != "2.1.0" {
        return Err(error("only SARIF version 2.1.0 is supported"));
    }
    let runs = json
        .get("runs")
        .and_then(Value::as_array)
        .ok_or_else(|| error("runs must be an array"))?;
    if runs.len() > 1000 {
        return Err(error("SARIF contains more than 1000 runs"));
    }
    let canonical = root.canonicalize()?;
    let prefix = source_root
        .map(|path| {
            let path = if path.is_absolute() {
                path.to_path_buf()
            } else {
                root.join(path)
            }
            .canonicalize()?;
            if !path.is_dir() {
                return Err(error("source root must be a directory"));
            }
            let relative = path
                .strip_prefix(&canonical)
                .map_err(|_| error("source root must stay inside the project"))?;
            Ok(relative.to_string_lossy().replace('\\', "/"))
        })
        .transpose()?;
    let mut roots = vec![canonical.to_string_lossy().replace('\\', "/")];
    if root.is_absolute() {
        roots.push(root.to_string_lossy().replace('\\', "/"));
    }
    let paths = source_paths(root, config, graph)?;
    let mut hashes = BTreeMap::<String, Option<String>>::new();
    let mut report = QualityReport::empty("ready");
    report.report_path = Some(report_path.to_string_lossy().into_owned());
    report.imported_at_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs());
    if runs.is_empty() {
        report.partial("No analyzer runs are present; no quality assessment was imported.");
    }
    for (run_index, run) in runs.iter().enumerate() {
        let driver = run
            .pointer("/tool/driver")
            .filter(|v| v.is_object())
            .ok_or_else(|| error("run has no tool.driver object"))?;
        let tool = driver["name"]
            .as_str()
            .filter(|name| !name.is_empty())
            .ok_or_else(|| error("tool.driver.name is required"))?;
        if !report.tools.iter().any(|name| name == tool) {
            report.tools.push(tool.into());
        }
        check_invocations(run, tool, &mut report)?;
        let results = match run.get("results") {
            Some(Value::Array(results)) => results,
            None | Some(Value::Null) => {
                report.partial(format!("{tool}: results are missing or null; analysis did not report a complete result set."));
                continue;
            }
            _ => return Err(error("run.results must be an array or null")),
        };
        if report.findings.len().saturating_add(results.len()) > MAX_FINDINGS {
            return Err(error("SARIF exceeds 100000 results"));
        }
        for (result_index, result) in results.iter().enumerate() {
            if !result.is_object() {
                return Err(error("each SARIF result must be an object"));
            }
            let rule = resolve_rule(driver, result, &mut report);
            let rule_id = result["ruleId"]
                .as_str()
                .or_else(|| result.pointer("/rule/id").and_then(Value::as_str))
                .or_else(|| rule.and_then(|r| r["id"].as_str()))
                .unwrap_or("")
                .to_string();
            let message = result
                .get("message")
                .ok_or_else(|| error("result.message is required"))?;
            let message = message_text(message, rule, driver)?;
            let level = resolve_level(run, result, rule)?;
            let help_uri = rule
                .and_then(|r| r["helpUri"].as_str())
                .filter(|s| safe_help(s))
                .map(String::from);
            let suppressed = match result.get("suppressions") {
                None | Some(Value::Null) => false,
                Some(Value::Array(suppressions)) => {
                    if suppressions
                        .iter()
                        .any(|s| s.get("status").is_none() || s["status"] == "underReview")
                    {
                        report.partial(format!("{tool}/{rule_id}: suppression request not accepted; finding remains active unless another suppression is accepted."));
                    }
                    suppressions.iter().any(|s| s["status"] == "accepted")
                }
                _ => return Err(error("result.suppressions must be an array or null")),
            };
            let locations = match result.get("locations") {
                None => &[][..],
                Some(Value::Array(locations)) => locations.as_slice(),
                _ => return Err(error("result.locations must be an array")),
            };
            if locations.len() > 1 {
                report.partial(format!("{tool}/{rule_id}: only the first primary physical location is mapped; {} additional locations remain outside this normalized view.",locations.len()-1));
            }
            let physical = locations.first().and_then(|v| v.get("physicalLocation"));
            let line = positive(
                physical.and_then(|v| v.pointer("/region/startLine")),
                "startLine",
            )?;
            let column = positive(
                physical.and_then(|v| v.pointer("/region/startColumn")),
                "startColumn",
            )?;
            let baseline_state = result
                .get("baselineState")
                .map(|value| {
                    value
                        .as_str()
                        .filter(|s| valid_baseline(s))
                        .map(String::from)
                        .ok_or_else(|| error("invalid result.baselineState"))
                })
                .transpose()?;
            let mut finding = QualityFinding {
                id: String::new(),
                tool: tool.into(),
                rule_id,
                level,
                message,
                path: None,
                node: None,
                line,
                column,
                source_sha256: None,
                status: "unmapped".into(),
                help_uri,
                suppressed,
                baseline_state,
            };
            let mapping = physical
                .and_then(|v| v.get("artifactLocation"))
                .ok_or_else(|| "no primary source artifact location".to_string())
                .and_then(|location| artifact_uri(run, location, 0, &mut BTreeSet::new()))
                .and_then(|(uri, bases)| {
                    // Preserve a safe source identifier even when the file is
                    // missing or excluded; node/hash still require exact mapping.
                    finding.path = relative(&uri).ok();
                    let candidates =
                        mapping_candidates(&uri, &bases, &roots, prefix.as_deref(), config)?;
                    let matches: Vec<_> = candidates
                        .into_iter()
                        .filter(|path| paths.contains_key(path))
                        .collect();
                    if matches.len() != 1 {
                        return Err(format!("{} matching analyzed source files", matches.len()));
                    }
                    Ok(matches[0].clone())
                });
            match mapping {
                Ok(path) => {
                    let source_hash = hashes.entry(path.clone()).or_insert_with(|| {
                        crate::project_files::read_repo_file(
                            root,
                            Path::new(&path),
                            MAX_SOURCE_BYTES,
                        )
                        .ok()
                        .map(|bytes| hash(&bytes))
                    });
                    if let Some(source_hash) = source_hash {
                        finding.node = Some(paths[&path].clone());
                        finding.path = Some(path);
                        finding.source_sha256 = Some(source_hash.clone());
                        finding.status = "unchanged_since_import".into();
                    } else {
                        report.partial(format!("{tool}/{}: mapped source cannot be read safely; finding remains unmapped.",finding.rule_id));
                    }
                }
                Err(reason) => report.partial(format!(
                    "{tool}/{}: {reason}; finding remains unmapped.",
                    finding.rule_id
                )),
            }
            // Preserve duplicate results with deterministic report-position IDs.
            finding.id =
                hash(&serde_json::to_vec(&(run_index, result_index, &finding)).map_err(error)?);
            report.findings.push(finding);
        }
    }
    let bytes = serde_json::to_vec_pretty(&report).map_err(error)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(error("normalized quality snapshot exceeds 32 MiB"));
    }
    crate::project_files::write_project_file(root, Path::new("quality.json"), &bytes, true)?;
    Ok(report)
}

fn check_invocations(
    run: &Value,
    tool: &str,
    report: &mut QualityReport,
) -> Result<(), ArchguardError> {
    let Some(invocations) = run.get("invocations") else {
        return Ok(());
    };
    let invocations = invocations
        .as_array()
        .ok_or_else(|| error("run.invocations must be an array"))?;
    for invocation in invocations {
        if invocation
            .get("executionSuccessful")
            .is_some_and(|value| !value.is_boolean())
        {
            return Err(error("invocation.executionSuccessful must be a boolean"));
        }
        if invocation["executionSuccessful"] == false {
            report.partial(format!(
                "{tool}: analyzer execution failed; imported findings are incomplete."
            ));
        }
        for key in [
            "toolExecutionNotifications",
            "toolConfigurationNotifications",
        ] {
            if let Some(notifications) = invocation.get(key) {
                let notifications = notifications
                    .as_array()
                    .ok_or_else(|| error("invocation notifications must be arrays"))?;
                for notification in notifications {
                    if notification["level"] == "error" {
                        let message = notification
                            .pointer("/message/text")
                            .and_then(Value::as_str)
                            .unwrap_or("analyzer error notification");
                        report.partial(format!("{tool}: {message}"));
                    }
                }
            }
        }
    }
    Ok(())
}
fn resolve_rule<'a>(
    driver: &'a Value,
    result: &Value,
    report: &mut QualityReport,
) -> Option<&'a Value> {
    if result.pointer("/rule/toolComponent").is_some() {
        report.partial("Rule references to tool extensions are not interpreted; explicit result levels/messages remain available.");
        return None;
    }
    let rules = driver["rules"].as_array()?;
    let id = result["ruleId"]
        .as_str()
        .or_else(|| result.pointer("/rule/id").and_then(Value::as_str));
    let index = result
        .get("ruleIndex")
        .or_else(|| result.pointer("/rule/index"));
    if let Some(index) = index {
        if let Some(rule) = index
            .as_u64()
            .and_then(|i| usize::try_from(i).ok())
            .and_then(|i| rules.get(i))
            && id.is_none_or(|id| rule["id"] == id)
        {
            return Some(rule);
        }
        report.partial(
            "A rule index is invalid or disagrees with its rule ID; indexed metadata was not used.",
        );
    }
    id.and_then(|id| rules.iter().find(|r| r["id"] == id))
}
fn message_text(
    message: &Value,
    rule: Option<&Value>,
    driver: &Value,
) -> Result<String, ArchguardError> {
    let text = message["text"]
        .as_str()
        .or_else(|| message["markdown"].as_str())
        .or_else(|| {
            let id = message["id"].as_str()?;
            rule.and_then(|r| r.get("messageStrings"))
                .and_then(|strings| strings.get(id))
                .or_else(|| {
                    driver
                        .get("globalMessageStrings")
                        .and_then(|strings| strings.get(id))
                })
                .and_then(|value| {
                    value["text"]
                        .as_str()
                        .or_else(|| value["markdown"].as_str())
                })
        })
        .ok_or_else(|| error("result message has no resolvable text, markdown or message ID"))?;
    let Some(arguments) = message.get("arguments") else {
        return Ok(text.into());
    };
    let arguments = arguments
        .as_array()
        .ok_or_else(|| error("message.arguments must be an array"))?;
    let arguments: Vec<_> = arguments
        .iter()
        .map(|argument| {
            argument
                .as_str()
                .ok_or_else(|| error("message arguments must be strings"))
        })
        .collect::<Result<_, _>>()?;
    let mut output = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if (c == '{' || c == '}') && chars.peek() == Some(&c) {
            output.push(c);
            chars.next();
            continue;
        }
        if c != '{' {
            output.push(c);
            continue;
        }
        let mut token = String::new();
        let mut closed = false;
        for next in chars.by_ref() {
            if next == '}' {
                closed = true;
                break;
            }
            token.push(next);
        }
        if closed && token.bytes().all(|b| b.is_ascii_digit()) && !token.is_empty() {
            let index = token
                .parse::<usize>()
                .map_err(|_| error("message argument index is too large"))?;
            output.push_str(
                arguments
                    .get(index)
                    .ok_or_else(|| error("message argument is missing"))?,
            );
        } else {
            output.push('{');
            output.push_str(&token);
            if closed {
                output.push('}');
            }
        }
    }
    Ok(output)
}
fn resolve_level(
    run: &Value,
    result: &Value,
    rule: Option<&Value>,
) -> Result<String, ArchguardError> {
    let kind = match result.get("kind") {
        None => "fail",
        Some(value) => value
            .as_str()
            .filter(|kind| {
                matches!(
                    *kind,
                    "fail" | "pass" | "open" | "informational" | "notApplicable" | "review"
                )
            })
            .ok_or_else(|| error("invalid result.kind"))?,
    };
    if result.get("level").is_some_and(|value| !value.is_string()) {
        return Err(error("result.level must be a string"));
    }
    let mut level = result["level"].as_str();
    if level.is_none() && kind != "fail" {
        level = Some("none");
    }
    if level.is_none()
        && let Some(invocation) = result
            .pointer("/provenance/invocationIndex")
            .and_then(Value::as_u64)
            .and_then(|i| usize::try_from(i).ok())
            .and_then(|i| run["invocations"].get(i))
        && let (Some(rule), Some(overrides)) =
            (rule, invocation["ruleConfigurationOverrides"].as_array())
    {
        level = overrides
            .iter()
            .find(|item| {
                item.pointer("/descriptor/id") == rule.get("id")
                    || item
                        .pointer("/descriptor/index")
                        .and_then(Value::as_u64)
                        .and_then(|i| usize::try_from(i).ok())
                        .and_then(|i| {
                            run.pointer("/tool/driver/rules")
                                .and_then(|rules| rules.get(i))
                        })
                        == Some(rule)
            })
            .and_then(|item| item.pointer("/configuration/level"))
            .and_then(Value::as_str);
    }
    let level = level
        .or_else(|| {
            rule.and_then(|r| r.pointer("/defaultConfiguration/level"))
                .and_then(Value::as_str)
        })
        .unwrap_or("warning");
    if !valid_level(level) || (kind != "fail" && level != "none") {
        return Err(error("invalid SARIF result level/kind combination"));
    }
    Ok(level.into())
}
fn positive(value: Option<&Value>, name: &str) -> Result<Option<u32>, ArchguardError> {
    value
        .map(|v| {
            v.as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .filter(|n| *n > 0)
                .ok_or_else(|| error(format!("{name} must be a positive 32-bit integer")))
        })
        .transpose()
}

// URI/path values are identifiers. Only a mapped, root-relative source is read.
fn uri_path(uri: &str) -> Result<String, String> {
    if uri.contains(['?', '#', '\\']) {
        return Err("artifact URI queries, fragments or backslashes are unsupported".into());
    }
    let uri = if let Some(path) = uri.strip_prefix("file://localhost/") {
        format!("/{path}")
    } else if let Some(path) = uri.strip_prefix("file:///") {
        format!("/{path}")
    } else if uri.contains(':') {
        return Err("artifact is not a supported local file URI".into());
    } else {
        uri.into()
    };
    let mut output = Vec::new();
    let bytes = uri.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return Err("invalid percent escape in artifact URI".into());
            }
            let hex =
                std::str::from_utf8(&bytes[i + 1..i + 3]).map_err(|_| "invalid percent escape")?;
            output.push(u8::from_str_radix(hex, 16).map_err(|_| "invalid percent escape")?);
            i += 3;
        } else {
            output.push(bytes[i]);
            i += 1;
        }
    }
    let value = String::from_utf8(output).map_err(|_| "artifact URI is not UTF-8")?;
    clean_path(&value)
}
fn clean_path(value: &str) -> Result<String, String> {
    if value.chars().any(char::is_control)
        || value.contains('\\')
        || value.split('/').any(|p| p == "..")
    {
        return Err("artifact path contains traversal or control characters".into());
    }
    let prefix = if value.starts_with('/') { "/" } else { "" };
    let tail = if value.ends_with('/') && !value.is_empty() {
        "/"
    } else {
        ""
    };
    let parts = value
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect::<Vec<_>>()
        .join("/");
    Ok(if parts.is_empty() {
        prefix.into()
    } else {
        format!("{prefix}{parts}{tail}")
    })
}
fn relative(value: &str) -> Result<String, String> {
    let value = clean_path(value)?;
    if value.starts_with('/')
        || value.is_empty()
        || value.split('/').next().is_some_and(|p| p.contains(':'))
    {
        return Err("expected a nonempty project-relative source path".into());
    }
    Ok(value.trim_end_matches('/').into())
}
fn artifact_uri(
    run: &Value,
    location: &Value,
    depth: usize,
    seen: &mut BTreeSet<String>,
) -> Result<(String, Vec<String>), String> {
    if depth > 20 {
        return Err("artifact URI indirection exceeds 20 steps".into());
    }
    if !location.is_object() {
        return Err("artifactLocation must be an object".into());
    }
    let indexed = if let Some(index) = location.get("index") {
        Some(
            index
                .as_u64()
                .and_then(|i| usize::try_from(i).ok())
                .and_then(|i| run["artifacts"].get(i))
                .and_then(|a| a.get("location"))
                .ok_or("artifact index is out of range")?,
        )
    } else {
        None
    };
    let uri = location["uri"]
        .as_str()
        .or_else(|| indexed.and_then(|a| a["uri"].as_str()))
        .ok_or("artifact URI is missing")?;
    if let (Some(explicit), Some(indexed)) = (
        location["uri"].as_str(),
        indexed.and_then(|a| a["uri"].as_str()),
    ) && uri_path(explicit)? != uri_path(indexed)?
    {
        return Err("artifact URI disagrees with its artifact index".into());
    }
    if let (Some(explicit), Some(indexed)) = (
        location["uriBaseId"].as_str(),
        indexed.and_then(|a| a["uriBaseId"].as_str()),
    ) && explicit != indexed
    {
        return Err("artifact URI base disagrees with its artifact index".into());
    }
    let mut path = uri_path(uri)?;
    let mut bases = Vec::new();
    if let Some(base) = location["uriBaseId"]
        .as_str()
        .or_else(|| indexed.and_then(|a| a["uriBaseId"].as_str()))
    {
        if !seen.insert(base.into()) {
            return Err("artifact URI base cycle".into());
        }
        let base_location = run["originalUriBaseIds"]
            .get(base)
            .ok_or("artifact URI base is unknown")?;
        let (parent, parents) = artifact_uri(run, base_location, depth + 1, seen)?;
        bases.extend(parents);
        bases.push(parent.clone());
        if !path.starts_with('/') {
            let directory = if parent.ends_with('/') {
                parent.as_str()
            } else {
                parent
                    .rsplit_once('/')
                    .map(|(dir, _)| &parent[..dir.len() + 1])
                    .unwrap_or("")
            };
            path = clean_path(&format!("{directory}{path}"))?;
        }
        seen.remove(base);
    }
    Ok((path, bases))
}
fn mapping_candidates(
    path: &str,
    bases: &[String],
    roots: &[String],
    prefix: Option<&str>,
    config: &Config,
) -> Result<BTreeSet<String>, String> {
    let mut names = BTreeSet::new();
    if let Ok(name) = relative(path) {
        names.insert(name);
    }
    for base in bases.iter().chain(roots) {
        let base = base.trim_end_matches('/');
        if !base.is_empty()
            && let Some(name) = path.strip_prefix(&(base.to_owned() + "/"))
        {
            names.insert(relative(name)?);
        }
    }
    let mut candidates = BTreeSet::new();
    for name in names {
        if let Some(prefix) = prefix {
            candidates.insert(relative(
                format!("{prefix}/{name}").trim_start_matches('/'),
            )?);
        } else {
            candidates.insert(name.clone());
            for source_root in &config.project.source_roots {
                if source_root == "." || source_root.is_empty() {
                    continue;
                }
                if let Ok(source_root) = relative(source_root) {
                    candidates.insert(relative(&format!("{source_root}/{name}"))?);
                }
            }
        }
    }
    Ok(candidates)
}
