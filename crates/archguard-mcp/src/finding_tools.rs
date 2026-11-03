//! Current review observations as a repair queue, with revision-bound detail pages.
use archguard_engine::core::{Graph, NodeKind};
use archguard_engine::{AnalysisRun, ReviewFinding, ReviewReport};
use rmcp::{ErrorData, schemars};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ArchitectureFindingsParams {
    #[serde(flatten)]
    pub page: crate::engineering_tools::EngineeringParams,
    /// Exact category: architecture, quality, security, duplication, contract, tests or mutations.
    pub category: Option<String>,
    /// Exact structural kind: cycle, layer_violation, layer_unranked, metric_threshold, dependency_rule, ownership_rule, orphan or target_architecture.
    pub kind: Option<String>,
    /// Exact severity: error, warning, info, critical, high, medium, low, note or unknown.
    pub severity: Option<String>,
    /// Revision from the previous list/detail page. Restart after source, policy or evidence changes.
    pub expected_revision: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct ArchitectureFindingParams {
    /// Project directory inside the server root; use the same path as the list call.
    pub path: Option<String>,
    /// Exact stable id returned by architecture_findings, not a transient scc-N identifier.
    pub finding_id: String,
    /// First detail record; follow next_offset with expected_revision.
    pub offset: Option<usize>,
    /// 1..=200 detail records, further bounded by 4 KiB.
    pub limit: Option<usize>,
    /// Revision from architecture_findings or the preceding detail page.
    pub expected_revision: Option<String>,
}

const KINDS: &[&str] = &[
    "cycle",
    "layer_violation",
    "layer_unranked",
    "metric_threshold",
    "dependency_rule",
    "ownership_rule",
    "orphan",
    "target_architecture",
];
const CATEGORIES: &[&str] = &[
    "architecture",
    "quality",
    "security",
    "duplication",
    "contract",
    "tests",
    "mutations",
];
const SEVERITIES: &[&str] = &[
    "error", "warning", "info", "critical", "high", "medium", "low", "note", "unknown",
];

pub fn validate(p: &ArchitectureFindingsParams) -> Result<(), ErrorData> {
    crate::engineering_tools::validate(&p.page)?;
    for (name, value, allowed) in [
        ("kind", &p.kind, KINDS),
        ("category", &p.category, CATEGORIES),
        ("severity", &p.severity, SEVERITIES),
    ] {
        if value.as_deref().is_some_and(|v| !allowed.contains(&v)) {
            return Err(crate::tools::invalid_params(format!(
                "{name} must be one of {}",
                allowed.join(", ")
            )));
        }
    }
    validate_revision(p.expected_revision.as_deref())
}
fn validate_revision(value: Option<&str>) -> Result<(), ErrorData> {
    if value.is_some_and(|v| v.is_empty() || v.len() > 256 || v.contains('\0')) {
        return Err(crate::tools::invalid_params(
            "expected_revision must be the revision returned by architecture_findings or architecture_finding",
        ));
    }
    Ok(())
}
impl ArchitectureFindingParams {
    fn page(&self) -> crate::engineering_tools::EngineeringParams {
        crate::engineering_tools::EngineeringParams {
            path: self.path.clone(),
            offset: self.offset,
            limit: self.limit,
            query: None,
        }
    }
    pub fn validate(&self) -> Result<(), ErrorData> {
        crate::engineering_tools::validate(&self.page())?;
        if self.finding_id.len() != 64 || !self.finding_id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(crate::tools::invalid_params(
                "finding_id must be the 64-character id returned by architecture_findings",
            ));
        }
        validate_revision(self.expected_revision.as_deref())
    }
}

/// Reuse the CLI review engine, including stable identities and evidence freshness.
/// No review or decision file is recorded by these tools.
pub fn analyze(
    server_root: &Path,
    path: Option<&str>,
) -> Result<(ReviewReport, AnalysisRun, String), ErrorData> {
    let root = crate::tools::resolve_root(server_root, path)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(crate::server::insight_error)?
        .as_secs();
    let (report, run) = archguard_engine::generate_review(&root, None, None, now)
        .map_err(crate::server::insight_error)?;
    // Review revision deliberately excludes decisions. Paging includes their
    // fingerprint and expiry day so disposition changes cannot mix pages.
    let revision = format!(
        "{}:{}:{}",
        report.revision,
        report.decision_fingerprint,
        now / 86_400
    );
    Ok((report, run, revision))
}
fn ensure_revision(revision: &str, expected: Option<&str>) -> Result<(), ErrorData> {
    if expected.is_some_and(|v| v != revision) {
        return Err(crate::tools::invalid_params(
            "Findings changed; restart at offset 0 without expected_revision and use the returned revision for subsequent pages.",
        ));
    }
    Ok(())
}
fn metadata(report: &ReviewReport, revision: &str) -> Value {
    let mut categories = BTreeMap::<&str, usize>::new();
    let mut severities = BTreeMap::<&str, usize>::new();
    for f in &report.findings {
        *categories.entry(&f.category).or_default() += 1;
        *severities.entry(&f.severity).or_default() += 1;
    }
    let evidence: BTreeMap<_, _> = report
        .evidence
        .iter()
        .filter_map(|e| Some((e["source"].as_str()?, e["status"].as_str()?)))
        .collect();
    json!({"revision":revision,"findings_total":report.findings.len(),"categories":categories,"severities":severities,"evidence_status":evidence,"evidence_gaps_total":report.gaps.len(),
        "interpretation":"Current observations, not proof of completeness or repair. Imported tools and tests were not rerun. Pattern and cohesion candidates are available through architecture_patterns and package_cohesion; project/service cycles through workspace_context and service_topology."})
}
fn summary(f: &ReviewFinding) -> Value {
    let title: String = f.title.chars().take(180).collect();
    json!({"kind":"finding","id":f.id,"fingerprint":f.fingerprint,"category":f.category,"finding_kind":f.evidence.get("kind"),"severity":f.severity,"title":title,"title_truncated":title!=f.title,"source_status":f.source_status,"disposition":f.disposition,"nodes_total":f.nodes.len(),"sample_nodes":f.nodes.iter().filter(|n|n.len()<=256).take(3).collect::<Vec<_>>()})
}
pub fn list_value(
    report: &ReviewReport,
    revision: &str,
    p: &ArchitectureFindingsParams,
) -> Result<Value, ErrorData> {
    validate(p)?;
    ensure_revision(revision, p.expected_revision.as_deref())?;
    let query = p.page.query.as_deref().unwrap_or("").trim().to_lowercase();
    let rows = report.findings.iter().filter(|f| {
        p.category.as_ref().is_none_or(|v| v == &f.category)
            && p.kind.as_deref().is_none_or(|v| f.evidence["kind"] == v)
            && p.severity.as_ref().is_none_or(|v| v == &f.severity)
            && (query.is_empty()
                || [&f.id, &f.title, &f.category]
                    .into_iter()
                    .chain(f.nodes.iter())
                    .any(|s| s.to_lowercase().contains(&query)))
    });
    crate::engineering_tools::page(
        &report.status,
        metadata(report, revision),
        rows,
        &p.page,
        summary,
    )
}

/// Split nested evidence instead of dropping a large finding wholesale. JSON
/// pointers retain association; long scalar text is returned in ordered chunks.
fn evidence_rows(rows: &mut Vec<Value>, pointer: &str, value: &Value) {
    if value.to_string().len() <= 1200 {
        rows.push(json!({"kind":"evidence","pointer":pointer,"value":value}));
        return;
    }
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                evidence_rows(
                    rows,
                    &format!("{pointer}/{}", key.replace('~', "~0").replace('/', "~1")),
                    child,
                );
            }
        }
        Value::Array(array) => {
            for (index, child) in array.iter().enumerate() {
                evidence_rows(rows, &format!("{pointer}/{index}"), child);
            }
        }
        Value::String(text) => {
            let chars: Vec<_> = text.chars().collect();
            let total = chars.len().div_ceil(200);
            for (index, chunk) in chars.chunks(200).enumerate() {
                rows.push(json!({"kind":"evidence_text","pointer":pointer,"part":index,"parts":total,"text":chunk.iter().collect::<String>()}));
            }
        }
        _ => rows.push(json!({"kind":"evidence","pointer":pointer,"value":value})),
    }
}
fn remediation(f: &ReviewFinding) -> &'static str {
    match f.evidence["kind"].as_str().unwrap_or(&f.category) {
        "cycle" | "layer_unranked" => {
            "Trace the recorded dependency path. Consider extracting shared types, depending on an interface, or moving a responsibility across the boundary. Suggested cuts are graph simulations, not source patches; preserve behavior and verify callers/tests."
        }
        "dependency_rule" | "layer_violation" | "target_architecture" => {
            "Read the declared boundary and concrete source references. Remove the forbidden dependency through a permitted interface or relocate the responsibility. Preserve the policy unless changing it is explicitly part of the task."
        }
        "metric_threshold" => {
            "Inspect the measured module and configured threshold. Reduce the responsible coupling or split responsibilities, then measure again; a metric alone does not select a valid code change."
        }
        "ownership_rule" => {
            "Inspect the ownership assignment and the source dependency crossing the configured boundary. Coordinate the responsibility or interface change with the recorded owners."
        }
        "orphan" => {
            "Check entry points, reflection, configuration and runtime consumers before deleting or relocating an apparently unused module."
        }
        "duplication" => {
            "Compare every recorded occurrence and its current source. Extract shared behavior only where the responsibilities agree; update and test every affected caller."
        }
        "mutations" | "tests" => {
            "Inspect the recorded failure or surviving mutation, current source and assertions. Repair the behavior or test gap and rerun the producing tests; import fresh evidence before verification."
        }
        "contract" => {
            "Check the recorded API change and declared consumers. Preserve compatibility or update affected consumers and contract tests; refresh the imported contract comparison."
        }
        "security" => {
            "Inspect the advisory and affected dependency paths. Apply an appropriate dependency update and run compatibility tests and a fresh security scan."
        }
        _ => {
            "Inspect the exact producer rule and source evidence. Fix the current implementation and rerun the producing check and relevant tests; refresh imported evidence before verification."
        }
    }
}
pub fn detail_value(
    report: &ReviewReport,
    run: &AnalysisRun,
    revision: &str,
    p: &ArchitectureFindingParams,
) -> Result<Value, ErrorData> {
    p.validate()?;
    ensure_revision(revision, p.expected_revision.as_deref())?;
    let mut meta = metadata(report, revision);
    meta["finding_id"] = p.finding_id.clone().into();
    let Some(f) = report.findings.iter().find(|f| f.id == p.finding_id) else {
        return crate::engineering_tools::page(
            "not_found",
            meta,
            std::iter::empty::<Value>(),
            &p.page(),
            |v| v,
        );
    };
    let mut rows = vec![
        summary(f),
        json!({"kind":"remediation","approach":remediation(f),"source_status":f.source_status,"automatic_fix":false}),
    ];
    evidence_rows(&mut rows, "/title", &json!(f.title));
    for node in &f.nodes {
        rows.push(json!({"kind":"finding_node","node":node}));
    }
    evidence_rows(&mut rows, "/evidence", &f.evidence);
    if let Some(decision) = &f.decision {
        evidence_rows(&mut rows, "/decision", &json!(decision));
    }
    for owner in &f.suggested_owners {
        rows.push(json!({"kind":"suggested_owner","owner":owner}));
    }
    let inside = |name: &str| {
        f.nodes
            .iter()
            .any(|n| name == n || name.strip_prefix(n).is_some_and(|s| s.starts_with('/')))
    };
    let selected: BTreeSet<_> = run
        .graph
        .nodes
        .iter()
        .filter(|n| inside(&n.name))
        .map(|n| n.id)
        .collect();
    let by_id: BTreeMap<_, _> = run.graph.nodes.iter().map(|n| (n.id, n)).collect();
    let paths =
        archguard_engine::source_reference_paths(Path::new(&report.project_root), &run.config)
            .map_err(crate::server::insight_error)?;
    if f.evidence["kind"] == "cycle" {
        let files: BTreeSet<_> = run
            .graph
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::File && selected.contains(&n.id))
            .map(|n| n.id)
            .collect();
        let mut subgraph: Graph = run.graph.clone();
        subgraph.nodes.retain(|n| files.contains(&n.id));
        subgraph
            .edges
            .retain(|e| files.contains(&e.from) && files.contains(&e.to));
        let level = if f.nodes.iter().all(|name| {
            run.graph
                .nodes
                .iter()
                .any(|n| n.name == *name && n.kind == NodeKind::File)
        }) {
            "file"
        } else {
            "module"
        };
        rows.push(json!({"kind":"cycle_scope","level":level,"file_cycle_groups":archguard_engine::analysis::find_sccs(&subgraph).len(),"interpretation":"A package cycle need not be a closed file cycle. Counts refer to the recorded graph and its semantic coverage."}));
    }
    for (edge_index, edge) in run.graph.edges.iter().enumerate() {
        let (left, right) = (selected.contains(&edge.from), selected.contains(&edge.to));
        if !left && !right {
            continue;
        }
        let (Some(from), Some(to)) = (by_id.get(&edge.from), by_id.get(&edge.to)) else {
            continue;
        };
        rows.push(json!({"kind":"source_dependency","from":from.name,"to":to.name,"count":edge.count,"references_total":edge.refs.len(),"relation":if left && right {"internal"} else if left {"outgoing"} else {"incoming"}}));
        for (reference_index, reference) in edge.refs.iter().enumerate() {
            let mut row = json!({"from":from.name,"to":to.name,"reference":reference,"physical_path":paths.get(&reference.file)});
            if row.to_string().len() > 1600 {
                evidence_rows(
                    &mut rows,
                    &format!("/source_dependencies/{edge_index}/references/{reference_index}"),
                    &row,
                );
            } else {
                row["kind"] = "source_reference".into();
                rows.push(row);
            }
        }
    }
    for (index, evidence) in report.evidence.iter().enumerate() {
        evidence_rows(&mut rows, &format!("/coverage/{index}"), evidence);
    }
    for (index, gap) in report.gaps.iter().enumerate() {
        evidence_rows(&mut rows, &format!("/evidence_gap/{index}"), gap);
    }
    let targets: Vec<_> = run
        .graph
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::File && selected.contains(&n.id))
        .map(|n| n.name.clone())
        .collect();
    rows.push(json!({"kind":"next_tool","purpose":"Read architecture intent and rules","name":"architecture_context","arguments":{"path":p.path}}));
    // Keep one target per call so long identifiers cannot crowd out a whole plan.
    for target in targets.iter().take(16) {
        rows.push(json!({"kind":"next_tool","purpose":"Inspect impact before editing this source","name":"change_context","arguments":{"path":p.path,"targets":[target],"depth":1},"targets_total":targets.len(),"targets_suggested":targets.len().min(16)}));
    }
    rows.push(json!({"kind":"verification","name":"architecture_findings","arguments":{"path":p.path},"instruction":"After editing, start a fresh list without expected_revision. Check remaining and newly introduced findings and evidence coverage. Run relevant builds/tests; an absent finding alone is not proof of repair."}));
    rows.push(json!({"kind":"verification","name":"check","arguments":{"path":p.path},"instruction":"Recheck current configured architecture gates. Refresh imported producer reports separately."}));
    crate::engineering_tools::page(&report.status, meta, rows.iter(), &p.page(), Clone::clone)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, ReviewReport, AnalysisRun) {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in [
            ("src/a/A.ts", "import {b} from '../b/B';\nexport const a=b;"),
            ("src/b/B.ts", "import {a} from '../a/A';\nexport const b=a;"),
        ] {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        std::fs::write(dir.path().join("archguard.toml"), "derive_layers=false\n[project]\nsource_roots=['src']\nlanguages=['ts']\n[modules]\ndepth=1\n").unwrap();
        let (report, run) =
            archguard_engine::generate_review(dir.path(), None, None, 1_793_232_000).unwrap();
        (dir, report, run)
    }

    #[test]
    fn all_finding_kinds_and_categories_remain_pageable_even_with_large_titles() {
        let (_dir, mut report, _) = fixture();
        let original = report.findings[0].clone();
        report.findings = (0..80)
            .map(|index| {
                let mut f = original.clone();
                f.id = format!("{index:064x}");
                f.category = CATEGORIES[index % CATEGORIES.len()].into();
                f.evidence["kind"] = KINDS[index % KINDS.len()].into();
                f.title = "Untrusted <instruction> Ä🦀\n".repeat(500);
                f.nodes = vec!["long-name".repeat(500), "real/file.ts".into()];
                f.source_status = "stale".into();
                f
            })
            .collect();
        let mut params = ArchitectureFindingsParams::default();
        let mut seen = BTreeSet::new();
        loop {
            let page = list_value(&report, "revision", &params).unwrap();
            assert!(page.to_string().len() <= 4096);
            assert_eq!(page["metadata"]["revision"], "revision");
            assert_eq!(page["metadata"]["findings_total"], 80);
            for row in page["items"].as_array().unwrap() {
                assert_ne!(row["omitted"], true);
                assert_eq!(row["source_status"], "stale");
                assert_eq!(row["title_truncated"], true);
                assert!(seen.insert(row["id"].as_str().unwrap().to_owned()));
            }
            let Some(next) = page["next_offset"].as_u64() else {
                break;
            };
            assert!(next as usize > params.page.offset.unwrap_or(0));
            params.page.offset = Some(next as usize);
            params.expected_revision = Some("revision".into());
        }
        assert_eq!(seen.len(), 80);
        params.page.offset = None;
        params.page.query = Some("not present".into());
        let empty = list_value(&report, "revision", &params).unwrap();
        assert_eq!(empty["total"], 0);
        assert_eq!(empty["metadata"]["findings_total"], 80);
        assert!(empty["metadata"]["evidence_gaps_total"].as_u64().unwrap() > 0);
        assert!(list_value(&report, "changed", &params).is_err());
    }

    #[test]
    fn long_reference_chunks_preserve_exact_text_and_distinct_source_associations() {
        let (_dir, report, mut run) = fixture();
        let finding = report
            .findings
            .iter()
            .find(|f| f.evidence["kind"] == "cycle")
            .unwrap();
        let originals: Vec<_> = run
            .graph
            .edges
            .iter_mut()
            .enumerate()
            .map(|(index, edge)| {
                let text = format!("{index}: {}", "🦀\n\u{0001}<instruction>".repeat(500));
                edge.refs[0].raw = text.clone();
                text
            })
            .collect();
        let mut params = ArchitectureFindingParams {
            finding_id: finding.id.clone(),
            ..Default::default()
        };
        let mut chunks = BTreeMap::<String, Vec<(u64, String)>>::new();
        loop {
            let page = detail_value(&report, &run, "revision", &params).unwrap();
            assert!(page.to_string().len() <= 4096);
            assert_eq!(page["metadata"]["finding_id"], finding.id);
            for row in page["items"].as_array().unwrap() {
                assert_ne!(row["omitted"], true);
                if row["kind"] == "evidence_text" {
                    chunks
                        .entry(row["pointer"].as_str().unwrap().into())
                        .or_default()
                        .push((
                            row["part"].as_u64().unwrap(),
                            row["text"].as_str().unwrap().into(),
                        ));
                }
            }
            let Some(next) = page["next_offset"].as_u64() else {
                break;
            };
            assert!(next as usize > params.offset.unwrap_or(0));
            params.offset = Some(next as usize);
        }
        for (index, original) in originals.iter().enumerate() {
            let pointer = format!("/source_dependencies/{index}/references/0/reference/raw");
            let parts = chunks
                .get_mut(&pointer)
                .expect("each source reference needs a distinct pointer");
            parts.sort_by_key(|(part, _)| *part);
            assert_eq!(
                &parts
                    .iter()
                    .map(|(_, text)| text.as_str())
                    .collect::<String>(),
                original
            );
        }
    }
}
