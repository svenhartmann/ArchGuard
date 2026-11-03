//! JUnit observations and explicit per-test source associations, never test execution.
use crate::evidence_import::{
    self as evidence, MAX_BYTES, MAX_RECORDS, Sources, error, relative, valid_hash,
};
use archguard_core::{ArchguardError, Config, Graph};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestSourceMapping {
    pub path: String,
    pub node: Option<String>,
    pub lines: Vec<u32>,
    pub source_sha256: Option<String>,
    pub status: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestCaseEvidence {
    pub id: String,
    pub suite: String,
    pub name: String,
    pub class_name: Option<String>,
    pub duration_seconds: Option<f64>,
    pub outcome: String,
    pub message: Option<String>,
    pub mappings: Vec<TestSourceMapping>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestEvidenceReport {
    pub schema_version: u32,
    pub status: String,
    pub report_path: Option<String>,
    pub mapping_path: Option<String>,
    pub imported_at_unix: Option<u64>,
    pub tests: Vec<TestCaseEvidence>,
    pub diagnostics: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestImpactMatch {
    pub test_id: String,
    pub name: String,
    pub suite: String,
    pub class_name: Option<String>,
    pub outcome: String,
    pub duration_seconds: Option<f64>,
    pub reason: String,
    pub matched_sources: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TestImpactReport {
    pub status: String,
    pub targets: Vec<String>,
    pub unresolved_targets: Vec<String>,
    pub selected: Vec<TestImpactMatch>,
    pub total_tests: usize,
    pub unmapped_tests: usize,
    pub stale_tests: usize,
    pub diagnostics: Vec<String>,
}
const SNAPSHOT: &str = ".archguard/tests.json";
const REVISION: &str = "Test report/source revision at import is unverified. Hashes detect later changes to mapped source files, not tests, dependencies or configuration. JUnit records alone do not identify exercised production code; explicit mappings are producer-supplied associations, not proof that tests detect regressions.";
fn empty(status: &str) -> TestEvidenceReport {
    TestEvidenceReport {
        schema_version: 1,
        status: status.into(),
        report_path: None,
        mapping_path: None,
        imported_at_unix: None,
        tests: vec![],
        diagnostics: vec![REVISION.into()],
    }
}
fn diagnostic(report: &mut TestEvidenceReport, value: impl Into<String>) {
    let v = value.into();
    if report.diagnostics.len() < 100 && !report.diagnostics.contains(&v) {
        report.diagnostics.push(v);
    }
}
fn valid_outcome(value: &str) -> bool {
    matches!(value, "passed" | "failed" | "error" | "skipped" | "unknown")
}
fn valid_duration(value: Option<f64>) -> bool {
    value.is_none_or(|n| n.is_finite() && n >= 0.0)
}
fn update_status(report: &mut TestEvidenceReport) {
    report.status = if report
        .tests
        .iter()
        .flat_map(|t| &t.mappings)
        .any(|m| m.status == "changed_since_import")
    {
        "stale"
    } else if report.tests.is_empty()
        || report.tests.iter().any(|t| {
            t.outcome == "unknown"
                || t.mappings.is_empty()
                || t.mappings.iter().any(|m| m.status == "unmapped")
        })
    {
        "partial"
    } else {
        "ready"
    }
    .into();
}
pub fn test_evidence_fingerprint(root: &Path) -> String {
    evidence::fingerprint(root, SNAPSHOT)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MappingInput {
    schema_version: u32,
    tests: Vec<MappingTest>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MappingTest {
    test_id: Option<String>,
    name: Option<String>,
    class_name: Option<String>,
    suite: Option<String>,
    sources: Vec<MappingSource>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MappingSource {
    path: String,
    #[serde(default)]
    lines: Vec<u32>,
}
/// JUnit XML and optional ArchGuard mapping JSON are explicit read-only inputs.
pub fn import_test_evidence(
    root: &Path,
    config: &Config,
    graph: &Graph,
    report_path: &Path,
    mapping_path: Option<&Path>,
    source_root: Option<&Path>,
) -> Result<TestEvidenceReport, ArchguardError> {
    let bytes = evidence::read_input(root, report_path)?;
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
    if !matches!(
        document.root_element().tag_name().name(),
        "testsuite" | "testsuites"
    ) {
        return Err(error("expected JUnit testsuite or testsuites root"));
    }
    let mut report = empty("ready");
    report.report_path = Some(report_path.to_string_lossy().into_owned());
    report.imported_at_unix = evidence::now();
    report.mapping_path = mapping_path.map(|p| p.to_string_lossy().into_owned());
    let mut occurrences = BTreeMap::new();
    for case in document
        .descendants()
        .filter(|n| n.has_tag_name("testcase"))
    {
        if report.tests.len() >= MAX_RECORDS {
            return Err(error("JUnit testcase limit exceeded"));
        }
        if !case.parent().is_some_and(|p| p.has_tag_name("testsuite")) {
            return Err(error("JUnit testcase must belong directly to a testsuite"));
        }
        let name = case
            .attribute("name")
            .filter(|n| !n.is_empty())
            .ok_or_else(|| error("JUnit testcase requires a nonempty name"))?
            .to_owned();
        let class_name = case.attribute("classname").map(str::to_owned);
        let mut suites = case
            .ancestors()
            .filter(|n| n.has_tag_name("testsuite"))
            .map(|n| n.attribute("name").unwrap_or(""))
            .collect::<Vec<_>>();
        suites.reverse();
        let suite = suites.join("/");
        let duration_seconds = case
            .attribute("time")
            .map(|t| t.parse::<f64>().map_err(error))
            .transpose()?;
        if !valid_duration(duration_seconds) {
            return Err(error("JUnit time must be finite, nonnegative seconds"));
        }
        let failures = case
            .children()
            .filter(|n| {
                n.has_tag_name("error") || n.has_tag_name("failure") || n.has_tag_name("skipped")
            })
            .collect::<Vec<_>>();
        let outcome = if failures.iter().any(|n| n.has_tag_name("error")) {
            "error"
        } else if failures.iter().any(|n| n.has_tag_name("failure")) {
            "failed"
        } else if failures.iter().any(|n| n.has_tag_name("skipped")) {
            "skipped"
        } else {
            match case.attribute("status") {
                None | Some("run" | "passed" | "success") => "passed",
                Some("notrun" | "disabled" | "skipped") => "skipped",
                Some("failed") => "failed",
                Some("error") => "error",
                _ => "unknown",
            }
        };
        let message = (!failures.is_empty()).then(|| {
            failures
                .iter()
                .map(|n| {
                    [n.attribute("message").unwrap_or(""), n.text().unwrap_or("")]
                        .into_iter()
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join(" · ")
                })
                .collect::<Vec<_>>()
                .join("\n")
        });
        let key = serde_json::to_string(&(&suite, &class_name, &name)).map_err(error)?;
        let occurrence = occurrences.entry(key.clone()).or_insert(0usize);
        let id = evidence::hash(format!("{key}:{occurrence}").as_bytes());
        *occurrence += 1;
        report.tests.push(TestCaseEvidence {
            id,
            suite,
            name,
            class_name,
            duration_seconds,
            outcome: outcome.into(),
            message,
            mappings: vec![],
        });
    }
    if let Some(mapping_path) = mapping_path {
        let input: MappingInput =
            serde_json::from_slice(&evidence::read_input(root, mapping_path)?).map_err(error)?;
        if input.schema_version != 1
            || input.tests.len() > MAX_RECORDS
            || input.tests.iter().map(|t| t.sources.len()).sum::<usize>() > MAX_RECORDS
        {
            return Err(error("unsupported or oversized test mapping document"));
        }
        let mut sources = Sources::new(root, config, graph, source_root)?;
        let mut assigned = BTreeSet::new();
        for entry in input.tests {
            if entry.sources.is_empty()
                || (entry.test_id.is_none() && entry.name.is_none())
                || (entry.test_id.is_some()
                    && (entry.name.is_some()
                        || entry.class_name.is_some()
                        || entry.suite.is_some()))
            {
                return Err(error(
                    "mapping requires test_id alone or a name with optional class_name/suite, and nonempty sources",
                ));
            }
            let matches = report
                .tests
                .iter()
                .enumerate()
                .filter(|(_, t)| {
                    entry.test_id.as_ref().is_none_or(|id| &t.id == id)
                        && entry.name.as_ref().is_none_or(|n| &t.name == n)
                        && entry
                            .class_name
                            .as_ref()
                            .is_none_or(|c| t.class_name.as_ref() == Some(c))
                        && entry.suite.as_ref().is_none_or(|s| &t.suite == s)
                })
                .map(|(i, _)| i)
                .collect::<Vec<_>>();
            if matches.len() != 1 || !assigned.insert(matches[0]) {
                return Err(error(
                    "mapping must select exactly one previously unassigned JUnit testcase",
                ));
            }
            let test = &mut report.tests[matches[0]];
            let mut paths = BTreeSet::new();
            for mut source in entry.sources {
                let path = sources.path(&source.path)?;
                if !paths.insert(path.clone())
                    || source.lines.len() > 500_000
                    || source.lines.contains(&0)
                {
                    return Err(error("duplicate source path or invalid mapping lines"));
                }
                source.lines.sort_unstable();
                source.lines.dedup();
                let (node, source_sha256, status) = if let Some(current) = sources.get(&path) {
                    if source.lines.iter().any(|n| *n > current.lines) {
                        return Err(error("mapped test line lies outside current source"));
                    }
                    (
                        Some(current.node.clone()),
                        Some(current.hash.clone()),
                        "unchanged_since_import",
                    )
                } else {
                    (None, None, "unmapped")
                };
                test.mappings.push(TestSourceMapping {
                    path,
                    node,
                    lines: source.lines,
                    source_sha256,
                    status: status.into(),
                });
            }
        }
    } else if source_root.is_some() {
        return Err(error("--source-root requires a test mapping document"));
    }
    if report.tests.iter().any(|t| t.mappings.is_empty()) {
        diagnostic(
            &mut report,
            "Some testcases have no explicit source mapping. Their impact is unknown; a JUnit classname is never treated as a production-code dependency.",
        );
    }
    if report.tests.is_empty() {
        diagnostic(
            &mut report,
            "No individual JUnit testcases were supplied. Suite aggregate counts are not substituted for test identities.",
        );
    }
    update_status(&mut report);
    evidence::save(root, "tests.json", &report)?;
    Ok(report)
}
pub fn read_test_evidence(root: &Path, config: &Config, graph: &Graph) -> TestEvidenceReport {
    let result = (|| -> Result<TestEvidenceReport, ArchguardError> {
        let bytes = match crate::project_files::read_repo_file(root, Path::new(SNAPSHOT), MAX_BYTES)
        {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(empty("not_imported")),
            Err(e) => return Err(e.into()),
        };
        let mut report: TestEvidenceReport = serde_json::from_slice(&bytes).map_err(error)?;
        let mut ids = BTreeSet::new();
        if report.schema_version != 1
            || !matches!(report.status.as_str(), "ready" | "partial")
            || report.tests.len() > MAX_RECORDS
            || report.diagnostics.len() > 100
            || report.tests.iter().map(|t| t.mappings.len()).sum::<usize>() > MAX_RECORDS
            || report.tests.iter().any(|t| {
                let mut paths = BTreeSet::new();
                !valid_hash(&t.id)
                    || !ids.insert(t.id.clone())
                    || t.name.is_empty()
                    || !valid_duration(t.duration_seconds)
                    || !valid_outcome(&t.outcome)
                    || t.mappings.iter().any(|m| {
                        !paths.insert(m.path.clone())
                            || !relative(&m.path).is_ok_and(|p| p == m.path)
                            || m.lines.len() > 500_000
                            || m.lines.contains(&0)
                            || m.lines.windows(2).any(|w| w[0] >= w[1])
                            || match m.status.as_str() {
                                "unchanged_since_import" => {
                                    m.node.as_deref().is_none_or(str::is_empty)
                                        || m.source_sha256.as_deref().is_none_or(|h| !valid_hash(h))
                                }
                                "unmapped" => m.node.is_some() || m.source_sha256.is_some(),
                                _ => true,
                            }
                    })
            })
        {
            return Err(error("invalid test-evidence snapshot"));
        }
        let mut sources = Sources::new(root, config, graph, None)?;
        for test in &mut report.tests {
            for m in &mut test.mappings {
                if m.status == "unchanged_since_import"
                    && !sources.get(&m.path).is_some_and(|s| {
                        Some(&s.node) == m.node.as_ref()
                            && Some(&s.hash) == m.source_sha256.as_ref()
                            && m.lines.iter().all(|line| *line <= s.lines)
                    })
                {
                    m.status = "changed_since_import".into();
                }
            }
        }
        diagnostic(&mut report, REVISION);
        update_status(&mut report);
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
/// Recommendations from explicit mappings plus reverse static dependencies. Never a test-skip proof.
pub fn test_impact(
    graph: &Graph,
    report: &TestEvidenceReport,
    targets: &[String],
) -> TestImpactReport {
    let mut result=TestImpactReport {status:"partial".into(),targets:targets.to_vec(),unresolved_targets:vec![],selected:vec![],total_tests:report.tests.len(),unmapped_tests:0,stale_tests:0,diagnostics:vec!["These are test recommendations from producer mappings and static dependencies. It is not safe to omit other tests; dynamic calls, missing mappings and unobserved behavior may be absent.".into(),REVISION.into()]};
    if !matches!(report.status.as_str(), "ready" | "partial" | "stale") {
        result.status = "unavailable".into();
        return result;
    }
    if targets.len() > 256 {
        result.status = "unavailable".into();
        result
            .diagnostics
            .push("At most 256 exact targets are supported.".into());
        return result;
    }
    let names: BTreeMap<_, _> = graph
        .nodes
        .iter()
        .map(|n| (n.name.as_str(), n.id))
        .collect();
    let mut changed = BTreeSet::new();
    let mut children = BTreeMap::new();
    for n in &graph.nodes {
        if let Some(p) = n.parent {
            children.entry(p).or_insert_with(Vec::new).push(n.id);
        }
    }
    for target in targets {
        let mut ids = BTreeSet::new();
        if let Some(id) = names.get(target.as_str()) {
            ids.insert(*id);
        }
        for m in report
            .tests
            .iter()
            .flat_map(|t| &t.mappings)
            .filter(|m| &m.path == target)
        {
            if let Some(id) = m.node.as_ref().and_then(|n| names.get(n.as_str())) {
                ids.insert(*id);
            }
        }
        if ids.len() != 1 {
            result.unresolved_targets.push(target.clone());
            continue;
        }
        let mut stack = ids.into_iter().collect::<Vec<_>>();
        while let Some(id) = stack.pop() {
            if changed.insert(id) {
                stack.extend(children.get(&id).into_iter().flatten().copied());
            }
        }
    }
    let mut reverse = BTreeMap::new();
    for e in &graph.edges {
        reverse
            .entry(e.to)
            .or_insert_with(BTreeSet::new)
            .insert(e.from);
    }
    let mut affected = changed.clone();
    let mut stack = changed.iter().copied().collect::<Vec<_>>();
    while let Some(id) = stack.pop() {
        for next in reverse.get(&id).into_iter().flatten() {
            if affected.insert(*next) {
                stack.push(*next);
            }
        }
    }
    for test in &report.tests {
        if test.mappings.is_empty() || test.mappings.iter().all(|m| m.status == "unmapped") {
            result.unmapped_tests += 1;
        }
        if test
            .mappings
            .iter()
            .any(|m| m.status == "changed_since_import")
        {
            result.stale_tests += 1;
        }
        let mut matched = BTreeSet::new();
        let mut direct = false;
        for m in test
            .mappings
            .iter()
            .filter(|m| m.status == "unchanged_since_import")
        {
            if let Some(id) = m.node.as_ref().and_then(|n| names.get(n.as_str()))
                && affected.contains(id)
            {
                matched.insert(m.path.clone());
                direct |= changed.contains(id);
            }
        }
        if !matched.is_empty() {
            result.selected.push(TestImpactMatch {
                test_id: test.id.clone(),
                name: test.name.clone(),
                suite: test.suite.clone(),
                class_name: test.class_name.clone(),
                outcome: test.outcome.clone(),
                duration_seconds: test.duration_seconds,
                reason: if direct {
                    "mapped_changed_source"
                } else {
                    "mapped_static_dependent"
                }
                .into(),
                matched_sources: matched.into_iter().collect(),
            });
        }
    }
    result.selected.sort_by(|a, b| {
        a.reason
            .cmp(&b.reason)
            .then(a.name.cmp(&b.name))
            .then(a.test_id.cmp(&b.test_id))
    });
    result.status = if targets.is_empty() {
        "not_requested"
    } else if result.unresolved_targets.is_empty()
        && report.status == "ready"
        && graph.semantic.as_ref().is_some_and(|s| s.status == "ready")
    {
        "ready"
    } else {
        "partial"
    }
    .into();
    result
}
