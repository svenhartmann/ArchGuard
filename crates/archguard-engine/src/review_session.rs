//! Explicit architecture reviews join existing observations without changing CI policy.
use crate::comparison;
use crate::config::LoadedConfig;
use crate::effective::AnalysisRun;
use crate::evidence_import as evidence;
use archguard_core::{ArchguardError, Finding, Graph, NodeKind};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

const LATEST: &str = ".archguard/review.json";
const DECISIONS: &str = ".archguard/review-decisions.json";
const MAX_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewFinding {
    pub id: String,
    pub fingerprint: String,
    pub category: String,
    pub severity: String,
    pub title: String,
    pub nodes: Vec<String>,
    pub change: String,
    pub source_status: String,
    pub evidence: Value,
    pub suggested_owners: Vec<String>,
    pub disposition: String,
    pub decision: Option<ReviewDecision>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewDecision {
    pub finding_id: String,
    pub finding_fingerprint: String,
    pub status: String,
    pub owner: String,
    pub reason: String,
    pub expires_on: Option<String>,
    #[serde(default)]
    pub links: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewDecisions {
    pub schema_version: u32,
    pub review_revision: String,
    #[serde(default)]
    pub decision_revision: Option<String>,
    pub decisions: Vec<ReviewDecision>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewReport {
    #[serde(skip)]
    recording_guard: Option<ReviewGuard>,
    pub schema_version: u32,
    /// Analysis identity, independent of review decisions.
    pub revision: String,
    pub input_fingerprint: String,
    pub decision_fingerprint: String,
    pub generated_at_unix: u64,
    pub project_root: String,
    /// Explicit policy selection, retained for reproducible viewer commands.
    #[serde(default)]
    pub config_path: Option<String>,
    pub base_ref: Option<String>,
    pub status: String,
    pub summary: Value,
    pub comparison: Option<Value>,
    pub changes: Option<Value>,
    pub impact: Value,
    pub findings: Vec<ReviewFinding>,
    pub evidence: Vec<Value>,
    pub gaps: Vec<Value>,
    pub decisions: ReviewDecisions,
    pub unmatched_decisions: Vec<ReviewDecision>,
    pub architecture: Value,
    pub limitations: Vec<String>,
}
#[derive(Debug, Clone)]
struct ReviewGuard {
    config_path: Option<PathBuf>,
    declared_inputs: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedReview {
    pub status: String,
    pub report: Option<ReviewReport>,
    pub diagnostics: Vec<String>,
}
fn error(message: impl std::fmt::Display) -> ArchguardError {
    ArchguardError::Analysis(format!("architecture review: {message}"))
}
fn hash(value: &impl Serialize) -> Result<String, ArchguardError> {
    Ok(evidence::hash(&serde_json::to_vec(value).map_err(error)?))
}
fn snapshot_fingerprint(root: &Path, name: &str) -> String {
    match crate::project_files::read_repo_file(root, Path::new(name), MAX_BYTES) {
        Ok(bytes) => evidence::hash(&bytes),
        Err(e) => format!("unavailable:{:?}", e.kind()),
    }
}
/// Expiry can change the live review even when no file was edited.
pub fn review_session_fingerprint(root: &Path, now: u64) -> String {
    format!(
        "{}:{}:{}",
        snapshot_fingerprint(root, LATEST),
        snapshot_fingerprint(root, DECISIONS),
        now / 86_400
    )
}
fn inputs(
    root: &Path,
    loaded: &LoadedConfig,
    base: Option<&str>,
) -> Result<String, ArchguardError> {
    let services = crate::services::analyze_services(root)?;
    let source = crate::change_context::input_revision(root, loaded)?;
    let repository = crate::ownership::repository_root(root)?;
    let mut ownership = BTreeMap::new();
    for path in [".github/CODEOWNERS", "CODEOWNERS", "docs/CODEOWNERS"] {
        ownership.insert(path, snapshot_fingerprint(&repository, path));
    }
    hash(&json!({
        "definition":"architecture-review/v3", "version":env!("CARGO_PKG_VERSION"),
        "source":source, "ownership":ownership, "services":services,
        "security":crate::security::security_fingerprint(root,&loaded.config)?,
        "security_snapshot":snapshot_fingerprint(root,".archguard/security.json"),
        "coverage":crate::test_coverage::test_coverage_fingerprint(root),
        "quality":crate::quality_findings::quality_findings_fingerprint(root),
        "tests":crate::test_evidence::test_evidence_fingerprint(root),
        "mutations":crate::mutation_report::mutation_fingerprint(root),
        "duplicates":crate::duplicates::duplication_fingerprint(root),
        "pattern_contracts":crate::patterns::pattern_inputs_fingerprint(root,&loaded.config),
        "contracts":crate::api_contracts::api_contracts_fingerprint(root),
        "base_commit":base.map(|b|crate::change_context::changed_files(root,b).map(|d|d.commit)).transpose()?
    }))
}

/// Generate one report. Tests, scanners and project builds are not launched.
/// Decisions and review records are written only through the explicit functions.
pub fn generate_review(
    root: &Path,
    config_path: Option<&Path>,
    base: Option<&str>,
    now: u64,
) -> Result<(ReviewReport, AnalysisRun), ArchguardError> {
    if base.is_some_and(|b| b.is_empty() || b.len() > 4096 || b.contains('\0')) {
        return Err(error(
            "base must be a nonempty Git reference of at most 4096 bytes",
        ));
    }
    let root = root.canonicalize()?;
    let declared = crate::config::load_config(config_path, &root)?;
    let before = inputs(&root, &declared, base)?;
    let decision_fingerprint = snapshot_fingerprint(&root, DECISIONS);
    let decisions = load_decisions(&root)?;
    let run =
        crate::effective::analyze_run_quiet(&root, &declared, crate::pipeline::Level::Module)?;
    let effective = run.effective_config();
    let mut findings = Vec::new();
    let comparison = base
        .map(|b| crate::comparison::compare_to_ref(&root, &effective, &run.graph, &run.report, b))
        .transpose()?;
    let new: BTreeSet<_> = comparison
        .as_ref()
        .map(|c| c.new_findings.iter().map(comparison::identity).collect())
        .unwrap_or_default();
    let sources = crate::review_sources::collect_review_sources(
        &root,
        &run.config,
        &run.graph,
        &run.config.review.git_since,
    )?;
    let owners = crate::ownership::analyze_ownership(
        &root,
        &run.graph,
        &sources,
        &run.config.ownership_rules,
    )?;
    let names: BTreeMap<_, _> = run
        .graph
        .nodes
        .iter()
        .map(|n| (n.id, n.name.as_str()))
        .collect();
    let canonical_edges: Vec<_> = run
        .graph
        .edges
        .iter()
        .map(|e| {
            json!({
                "from":names.get(&e.from),"to":names.get(&e.to),"count":e.count,"refs":e.refs
            })
        })
        .collect();
    let mut source_hashes = BTreeMap::new();
    for source in &sources.files {
        if let (Some(path), Some(name)) = (&source.path, names.get(&source.node_id)) {
            source_hashes.insert((*name).to_owned(), snapshot_fingerprint(&root, path));
        }
    }
    for finding in comparison::comparison_findings(&run.graph, &run.report, &run.config)? {
        let identity = comparison::identity(&finding);
        let change = if base.is_none() {
            "not_compared"
        } else if new.contains(&identity) {
            "new"
        } else {
            "existing"
        };
        let mut item = structural_finding(&finding, change)?;
        let related = |name: &str| {
            finding.nodes.iter().any(|n| {
                name == n
                    || name
                        .strip_prefix(n)
                        .is_some_and(|rest| rest.starts_with('/'))
            })
        };
        let edges: Vec<_> = canonical_edges
            .iter()
            .filter(|e| {
                related(e["from"].as_str().unwrap_or("")) || related(e["to"].as_str().unwrap_or(""))
            })
            .collect();
        let hashes: BTreeMap<_, _> = source_hashes
            .iter()
            .filter(|(name, _)| related(name))
            .collect();
        // Full stored edges and source contents bind acceptance: unchanged SCC
        // membership must not hide a denser cycle or changed source evidence.
        item.fingerprint = hash(
            &json!({"finding":item.evidence,"edges":edges,"sources":hashes,"policy":run.config}),
        )?;
        item.suggested_owners = owners
            .assignments
            .iter()
            .filter(|o| related(&o.name))
            .flat_map(|o| o.owners.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        findings.push(item);
    }
    if let Some(c) = &comparison {
        for f in &c.resolved_findings {
            findings.push(structural_finding(f, "not_observed_in_current")?);
        }
    }
    let coverage = crate::test_coverage::read_test_coverage(&root, &run.config, &run.graph);
    let quality = crate::quality_findings::read_quality_findings(&root, &run.config, &run.graph);
    let tests = crate::test_evidence::read_test_evidence(&root, &run.config, &run.graph);
    let security = crate::security::read_security(&root, &run.config);
    let mutations = crate::mutation_report::read_mutations(&root, &run.config, &run.graph);
    let duplicates = crate::duplicates::read_duplicates(&root, &run.config, &run.graph);
    let services = crate::services::analyze_services(&root)?;
    let contracts = crate::api_contracts::read_api_contracts(&root, &services);
    let mut evidence_rows = vec![];
    let mut gaps = vec![];
    let mut record = |source: &str,
                      status: &str,
                      unverified: bool,
                      count: usize,
                      diagnostics: &[String]| {
        evidence_rows.push(json!({"source":source,"status":status,"records":count,"producer_revision":if unverified {"unverified"} else {"analyzed_inputs"},"diagnostics":diagnostics}));
        if !matches!(
            status,
            "ready" | "complete" | "available" | "not_applicable"
        ) {
            gaps.push(json!({"source":source,"status":status,"reason":"Evidence is missing, incomplete, stale or unavailable; absence of findings is not a clean result."}));
        }
        if unverified && !matches!(status, "not_imported" | "not_scanned" | "error") {
            gaps.push(json!({"source":source,"status":"unverified","reason":"Producer revision, tool configuration and tests are not verified by this import."}));
        }
    };
    let semantic = run.graph.semantic.as_ref();
    record(
        "semantic",
        semantic.map_or("unavailable", |s| s.status.as_str()),
        false,
        run.graph.nodes.len(),
        &semantic.map(|s| s.diagnostics.clone()).unwrap_or_default(),
    );
    record(
        "coverage",
        &coverage.status,
        true,
        coverage.files.len(),
        &coverage.diagnostics,
    );
    record(
        "quality",
        &quality.status,
        true,
        quality.findings.len(),
        &quality.diagnostics,
    );
    record(
        "tests",
        &tests.status,
        true,
        tests.tests.len(),
        &tests.diagnostics,
    );
    record(
        "security",
        &security.status,
        false,
        security.findings.len(),
        &security.diagnostics,
    );
    record(
        "mutations",
        &mutations.status,
        true,
        mutations.files.len(),
        &mutations.diagnostics,
    );
    record(
        "contracts",
        &contracts.status,
        true,
        contracts.contracts.len(),
        &contracts.diagnostics,
    );
    record(
        "duplicates",
        &duplicates.status,
        true,
        duplicates.groups.len(),
        &duplicates.diagnostics,
    );
    for group in &duplicates.groups {
        findings.push(imported_finding("duplication",&group.id,"warning",
            &format!("Reported duplicate block: {} tokens at {} source locations",group.tokens,group.occurrences.len()),
            group.occurrences.iter().filter_map(|o|o.node.clone()).collect::<BTreeSet<_>>().into_iter().collect(),
            &group.status,json!({"group":group,"producer_version":duplicates.producer_version,"producer_revision":"unverified"}))?);
    }
    for f in &quality.findings {
        if f.suppressed || f.baseline_state.as_deref() == Some("absent") || f.level == "none" {
            continue;
        }
        findings.push(imported_finding(
            "quality",
            &f.id,
            &f.level,
            &f.message,
            f.node.iter().cloned().collect(),
            &f.status,
            serde_json::to_value(f).map_err(error)?,
        )?);
    }
    for f in &security.findings {
        findings.push(imported_finding(
            "security",
            &hash(&(
                f.id.as_str(),
                f.package.as_str(),
                f.installed_version.as_str(),
                f.ecosystem.as_str(),
                &f.targets,
            ))?,
            &f.severity,
            &f.title,
            vec![],
            &security.status,
            serde_json::to_value(f).map_err(error)?,
        )?);
    }
    for contract in &contracts.contracts {
        for change in &contract.changes {
            findings.push(imported_finding("contract",&hash(&(&contract.provider,&change.operation,&change.kind))?,&change.severity,&change.message,vec![],&contract.source_status,json!({"provider":contract.provider,"change":change,"comparison_status":contract.comparison_status,"compared_with":"previous_explicit_import"}))?);
        }
    }
    for test in &tests.tests {
        if !matches!(test.outcome.as_str(), "failed" | "error") {
            continue;
        }
        findings.push(imported_finding(
            "tests",
            &test.id,
            "error",
            &format!(
                "Reported test {}: {} / {}",
                test.outcome, test.suite, test.name
            ),
            test.mappings
                .iter()
                .filter_map(|m| m.node.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect(),
            &tests.status,
            json!({"test":test,"producer_revision":"unverified","report_path":tests.report_path}),
        )?);
    }
    for file in &mutations.files {
        for mutation in &file.mutants {
            if !matches!(mutation.status.as_str(), "Survived" | "NoCoverage") {
                continue;
            }
            findings.push(imported_finding(
                "mutations", &hash(&(&file.path,&mutation.id))?, "warning",
                &format!("Reported undetected mutation ({}) at {}:{}",mutation.status,file.path,mutation.location.start.line),
                file.node.iter().cloned().collect(), &file.status,
                json!({"path":file.path,"source_sha256":file.source_sha256,"mutation":mutation,"producer":mutations.producer,"producer_version":mutations.producer_version,"producer_revision":"unverified","interpretation":"Review the reported mutation and test assertions. An undetected mutation is not proof of a production bug; it may be equivalent."}),
            )?);
        }
    }
    // A report may already be stale when accepted. Its stale flag and imported
    // hashes do not change on a second edit; bind decisions to current mapped
    // source contents as well, without claiming that the producer was rerun.
    for finding in findings.iter_mut().filter(|f| f.category != "architecture") {
        let current_sources: BTreeMap<_, _> = finding
            .nodes
            .iter()
            .map(|name| (name, source_hashes.get(name)))
            .collect();
        finding.evidence["current_source_hashes"] =
            serde_json::to_value(current_sources).map_err(error)?;
        finding.fingerprint = hash(&(&finding.source_status, &finding.evidence))?;
    }
    let diff = base
        .map(|b| crate::change_context::changed_files(&root, b))
        .transpose()?;
    let mut targets = BTreeSet::new();
    let mut unresolved = Vec::new();
    if let Some(diff) = &diff {
        for (path, status) in &diff.files {
            let matches: Vec<_> = sources
                .files
                .iter()
                .filter(|f| f.path.as_deref() == Some(path))
                .filter_map(|f| names.get(&f.node_id).copied())
                .collect();
            if status != "D" && matches.len() == 1 {
                targets.insert(matches[0].to_owned());
            } else {
                unresolved.push(json!({"path":path,"status":status,"reason":if status=="D" {"deleted_source_former_callers_not_reconstructed"} else {"not_uniquely_mapped_source"}}));
            }
        }
    } else {
        for f in &findings {
            targets.extend(f.nodes.clone());
        }
    }
    let affected = affected_files(&run.graph, &targets);
    let target_list: Vec<_> = targets.iter().take(256).cloned().collect();
    let test_impact = crate::test_evidence::test_impact(&run.graph, &tests, &target_list);
    if targets.len() > 256 {
        gaps.push(json!({"source":"impact","status":"partial","reason":"Test recommendations are limited to the first 256 targets.","targets_total":targets.len()}));
    }
    if !unresolved.is_empty() {
        gaps.push(json!({"source":"changes","status":"partial","reason":"Some changed files have no current source identity. Former callers of deletions are not reconstructed.","files":unresolved}));
    }
    let architecture =
        crate::architecture::architecture_context(&root, &run.config, &run.graph, &run.report);
    let affected_components: Vec<_> = architecture
        .observed
        .target_model
        .as_ref()
        .map(|m| {
            m.components
                .iter()
                .filter(|c| c.nodes.iter().any(|n| affected.contains(n)))
                .map(|c| c.name.clone())
                .collect()
        })
        .unwrap_or_default();
    let mut architecture = serde_json::to_value(architecture).map_err(error)?;
    architecture["patterns"] =
        serde_json::to_value(crate::patterns::analyze_architecture_patterns(
            &root,
            &run.config,
            &run.graph,
            &crate::semantic_calls::analyze_calls(&root, &run.config),
            &services,
        ))
        .map_err(error)?;
    let changes = diff
        .as_ref()
        .map(|d| json!({"base_commit":d.commit,"files":d.files,"unresolved":unresolved}));
    findings.sort_by(|a, b| {
        priority(&a.severity)
            .cmp(&priority(&b.severity))
            .then_with(|| a.id.cmp(&b.id))
    });
    let input_fingerprint = inputs(&root, &effective, base)?;
    // Unanalyzed files (including an exported decision document) are context,
    // not analysis inputs. Adding a decision file cannot invalidate itself.
    let revision = hash(
        &json!({"inputs":input_fingerprint,"findings":findings,"owners":owners,"architecture":architecture}),
    )?;
    let selected_config = config_path.map(std::fs::canonicalize).transpose()?;
    let mut report = ReviewReport {
        recording_guard:Some(ReviewGuard{config_path:selected_config.clone(),declared_inputs:before.clone()}),
        schema_version:1,revision,input_fingerprint,decision_fingerprint,generated_at_unix:now,
        project_root:root.to_string_lossy().into_owned(),base_ref:base.map(str::to_owned),
        config_path:selected_config.map(|p|p.to_string_lossy().into_owned()),
        status:String::new(),summary:Value::Null,
        comparison:comparison.map(serde_json::to_value).transpose().map_err(error)?,changes,
        impact:json!({"selection":if base.is_some() {"changed_sources"} else {"finding_sources"},"targets":targets,"affected_files":affected,"affected_components":affected_components,"tests":test_impact,"safe_to_skip_other_tests":false}),
        findings,evidence:evidence_rows,gaps,decisions,unmatched_decisions:vec![],
        architecture,
        limitations:vec![
            "Review decisions do not change check/quality/security gates or cycle baselines.".into(),
            "The selected Git commit is compared directly with the working tree, including uncommitted sources. No merge base is inferred. Both revisions use current policy.".into(),
            "Not observed in current analysis is not proof of repair when coverage differs. Deleted files' former callers are not reconstructed.".into(),
            "Imported findings are not compared against the Git base. Contract changes use the previous explicit import. Producer revision remains unverified.".into(),
            "Static impact and producer-supplied test mappings cannot establish that other tests are safe to skip. No tests or project builds were executed.".into(),
            "Project text and decision links are untrusted data; links are not fetched. Saved reports preserve their original analysis and decision state.".into(),
        ],
    };
    evaluate_decisions(&mut report, now);
    if before
        != inputs(
            &root,
            &crate::config::load_config(config_path, &root)?,
            base,
        )?
        || report.decision_fingerprint != snapshot_fingerprint(&root, DECISIONS)
    {
        return Err(error(
            "inputs changed during review; retry before recording decisions",
        ));
    }
    Ok((report, run))
}
fn priority(severity: &str) -> u8 {
    match severity.to_ascii_lowercase().as_str() {
        "critical" | "high" | "error" => 0,
        "medium" | "warning" => 1,
        _ => 2,
    }
}
fn structural_finding(f: &Finding, change: &str) -> Result<ReviewFinding, ArchguardError> {
    let id = hash(&comparison::identity(f))?;
    let mut details = serde_json::to_value(f).map_err(error)?;
    details.as_object_mut().unwrap().remove("id");
    Ok(ReviewFinding {
        id,
        fingerprint: hash(&details)?,
        category: "architecture".into(),
        severity: serde_json::to_value(f.severity)
            .unwrap()
            .as_str()
            .unwrap()
            .into(),
        title: f
            .message
            .clone()
            .unwrap_or_else(|| format!("{:?}: {}", f.kind, f.nodes.join(" → "))),
        nodes: f.nodes.clone(),
        change: change.into(),
        source_status: "analyzed".into(),
        evidence: details,
        suggested_owners: vec![],
        disposition: "open".into(),
        decision: None,
    })
}
fn imported_finding(
    category: &str,
    id: &str,
    severity: &str,
    title: &str,
    nodes: Vec<String>,
    status: &str,
    details: Value,
) -> Result<ReviewFinding, ArchguardError> {
    Ok(ReviewFinding {
        id: hash(&(category, id))?,
        fingerprint: hash(&(status, &details))?,
        category: category.into(),
        severity: severity.to_lowercase(),
        title: title.into(),
        nodes,
        change: "not_compared".into(),
        source_status: status.into(),
        evidence: details,
        suggested_owners: vec![],
        disposition: "open".into(),
        decision: None,
    })
}
fn affected_files(graph: &Graph, targets: &BTreeSet<String>) -> BTreeSet<String> {
    let mut selected = BTreeSet::new();
    for node in &graph.nodes {
        if targets.iter().any(|t| {
            node.name == *t
                || node
                    .name
                    .strip_prefix(t)
                    .is_some_and(|s| s.starts_with('/'))
        }) {
            selected.insert(node.id);
        }
    }
    let mut reverse = BTreeMap::<_, Vec<_>>::new();
    for edge in &graph.edges {
        reverse.entry(edge.to).or_default().push(edge.from);
    }
    let mut stack: Vec<_> = selected.iter().copied().collect();
    while let Some(id) = stack.pop() {
        for from in reverse.get(&id).into_iter().flatten() {
            if selected.insert(*from) {
                stack.push(*from);
            }
        }
    }
    graph
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::File && !n.name.ends_with("/*") && selected.contains(&n.id))
        .map(|n| n.name.clone())
        .collect()
}
fn load_decisions(root: &Path) -> Result<ReviewDecisions, ArchguardError> {
    match crate::project_files::read_repo_file(root, Path::new(DECISIONS), MAX_BYTES) {
        Ok(bytes) => {
            let document: ReviewDecisions = serde_json::from_slice(&bytes).map_err(error)?;
            validate_decisions(&document)?;
            Ok(document)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ReviewDecisions {
            schema_version: 1,
            review_revision: String::new(),
            decision_revision: None,
            decisions: vec![],
        }),
        Err(e) => Err(e.into()),
    }
}
fn validate_decisions(document: &ReviewDecisions) -> Result<(), ArchguardError> {
    if document.schema_version != 1
        || !evidence::valid_hash(&document.review_revision)
        || document.decisions.len() > 10_000
    {
        return Err(error(
            "invalid decision document version, review revision or record count",
        ));
    }
    let mut ids = BTreeSet::new();
    for d in &document.decisions {
        if !evidence::valid_hash(&d.finding_id)
            || !evidence::valid_hash(&d.finding_fingerprint)
            || !ids.insert(&d.finding_id)
        {
            return Err(error("decision identities must be unique SHA-256 values"));
        }
        if !matches!(d.status.as_str(), "open" | "planned" | "accepted_risk")
            || d.owner.trim().is_empty()
            || d.owner.len() > 256
            || d.reason.trim().is_empty()
            || d.reason.len() > 8192
        {
            return Err(error(
                "every decision needs open/planned/accepted_risk status, owner and reason",
            ));
        }
        if d.links.len() > 20
            || d.links
                .iter()
                .any(|s| s.len() > 4096 || s.chars().any(char::is_control))
        {
            return Err(error("decision links exceed the supported size"));
        }
        if d.status == "accepted_risk" && d.expires_on.is_none() {
            return Err(error(
                "accepted risk requires expires_on (YYYY-MM-DD, inclusive UTC date)",
            ));
        }
        if let Some(date) = &d.expires_on {
            expiry_day(date)?;
        }
    }
    Ok(())
}
fn evaluate_decisions(report: &mut ReviewReport, now: u64) {
    let by_id: BTreeMap<_, _> = report
        .decisions
        .decisions
        .iter()
        .map(|d| (d.finding_id.as_str(), d))
        .collect();
    let mut observed = BTreeSet::new();
    for f in &mut report.findings {
        observed.insert(f.id.clone());
        f.decision = by_id.get(f.id.as_str()).map(|d| (*d).clone());
        f.disposition = if f.change == "not_observed_in_current" {
            "not_observed"
        } else if let Some(d) = &f.decision {
            if d.finding_fingerprint != f.fingerprint {
                "changed"
            } else if d
                .expires_on
                .as_ref()
                .is_some_and(|date| expiry_day(date).is_ok_and(|day| now / 86_400 > day))
            {
                "expired"
            } else {
                &d.status
            }
        } else {
            "open"
        }
        .into();
    }
    report.unmatched_decisions = report
        .decisions
        .decisions
        .iter()
        .filter(|d| !observed.contains(&d.finding_id))
        .cloned()
        .collect();
    let open = report
        .findings
        .iter()
        .filter(|f| !matches!(f.disposition.as_str(), "accepted_risk" | "not_observed"))
        .count();
    let accepted = report
        .findings
        .iter()
        .filter(|f| f.disposition == "accepted_risk")
        .count();
    report.status = if open > 0 {
        "needs_review"
    } else {
        "no_open_findings"
    }
    .into();
    report.summary = json!({"findings":report.findings.len(),"open":open,"accepted_risk":accepted,"expired":report.findings.iter().filter(|f|f.disposition=="expired").count(),"changed_decisions":report.findings.iter().filter(|f|f.disposition=="changed").count(),"not_observed":report.findings.iter().filter(|f|f.disposition=="not_observed").count(),"evidence_gaps":report.gaps.len(),"unmatched_decisions":report.unmatched_decisions.len(),"evidence_status":if report.gaps.is_empty() {"available"}else{"incomplete"}});
}
/// A strict Gregorian date, interpreted as an inclusive UTC day.
fn expiry_day(date: &str) -> Result<u64, ArchguardError> {
    if date.len() != 10
        || date.as_bytes()[4] != b'-'
        || date.as_bytes()[7] != b'-'
        || !date
            .bytes()
            .enumerate()
            .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit())
    {
        return Err(error("expiry must be YYYY-MM-DD"));
    }
    let year: u64 = date[..4].parse().map_err(error)?;
    let month: usize = date[5..7].parse().map_err(error)?;
    let day: u64 = date[8..].parse().map_err(error)?;
    let leap = |y: u64| y.is_multiple_of(4) && (!y.is_multiple_of(100) || y.is_multiple_of(400));
    let months = [
        31,
        if leap(year) { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if !(1970..=9999).contains(&year)
        || !(1..=12).contains(&month)
        || day == 0
        || day > months[month - 1]
    {
        return Err(error("expiry is not a valid date from 1970 to 9999"));
    }
    let prior = year - 1;
    let days = 365 * (year - 1970) + (prior / 4 - prior / 100 + prior / 400)
        - (1969 / 4 - 1969 / 100 + 1969 / 400);
    Ok(days + months[..month - 1].iter().sum::<u64>() + day - 1)
}

/// Replace explicit decisions only after validating the complete document.
pub fn import_review_decisions(
    root: &Path,
    report: &mut ReviewReport,
    path: &Path,
    now: u64,
) -> Result<(), ArchguardError> {
    let bytes = evidence::read_input(root, path)?;
    apply_review_decisions(root, report, &bytes, now)
}
/// Import already-read decision bytes with the same validation as the CLI.
pub fn apply_review_decisions(
    root: &Path,
    report: &mut ReviewReport,
    bytes: &[u8],
    now: u64,
) -> Result<(), ArchguardError> {
    assert_current(root, report)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(error("decision document exceeds 32 MiB"));
    }
    let document: ReviewDecisions = serde_json::from_slice(bytes).map_err(error)?;
    validate_decisions(&document)?;
    if document.review_revision != report.revision {
        return Err(error(
            "decision document belongs to a different review revision; regenerate and review the current evidence",
        ));
    }
    if document.decision_revision.as_deref() != Some(report.decision_fingerprint.as_str())
        && (!report
            .decision_fingerprint
            .starts_with("unavailable:NotFound")
            || document.decision_revision.is_some())
    {
        return Err(error(
            "decision export is based on an older decision revision; export again from the current review",
        ));
    }
    let findings: BTreeMap<_, _> = report.findings.iter().map(|f| (f.id.as_str(), f)).collect();
    for d in &document.decisions {
        let unchanged = report.decisions.decisions.contains(d);
        if let Some(f) = findings.get(d.finding_id.as_str()) {
            if !unchanged
                && (d.finding_fingerprint != f.fingerprint
                    || f.disposition == "not_observed"
                    || d.expires_on
                        .as_ref()
                        .is_some_and(|s| expiry_day(s).is_ok_and(|day| now / 86_400 > day)))
            {
                return Err(error(
                    "new decisions require current observed evidence and an unexpired date",
                ));
            }
        } else if !unchanged {
            return Err(error("decision refers to an unknown finding"));
        }
    }
    if snapshot_fingerprint(root, DECISIONS) != report.decision_fingerprint {
        return Err(error(
            "decisions changed since review generation; reload before importing",
        ));
    }
    let encoded = serde_json::to_vec_pretty(&document).map_err(error)?;
    crate::project_files::write_project_file(
        root,
        Path::new("review-decisions.json"),
        &encoded,
        true,
    )?;
    report.decision_fingerprint = evidence::hash(&encoded);
    report.decisions = document;
    evaluate_decisions(report, now);
    Ok(())
}
pub fn save_review(root: &Path, report: &ReviewReport) -> Result<(), ArchguardError> {
    assert_current(root, report)?;
    let bytes = serde_json::to_vec_pretty(report).map_err(error)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(error("review exceeds 32 MiB; narrow the project scope"));
    }
    let name = format!("reviews/{}.json", evidence::hash(&bytes));
    match crate::project_files::write_project_file(root, Path::new(&name), &bytes, false) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing = crate::project_files::read_repo_file(
                root,
                &Path::new(".archguard").join(&name),
                MAX_BYTES,
            )?;
            if existing != bytes {
                return Err(error(
                    "immutable review record already exists with different content",
                ));
            }
        }
        Err(e) => return Err(e.into()),
    }
    crate::project_files::write_project_file(root, Path::new("review.json"), &bytes, true)?;
    Ok(())
}
fn assert_current(root: &Path, report: &ReviewReport) -> Result<(), ArchguardError> {
    let guard = report.recording_guard.as_ref().ok_or_else(|| {
        error("generate a current review before recording it or importing decisions")
    })?;
    let root = root.canonicalize()?;
    let loaded = crate::config::load_config(guard.config_path.as_deref(), &root)?;
    if root.to_string_lossy() != report.project_root
        || inputs(&root, &loaded, report.base_ref.as_deref())? != guard.declared_inputs
    {
        return Err(error(
            "source, policy, base reference or evidence changed after review generation",
        ));
    }
    if snapshot_fingerprint(&root, DECISIONS) != report.decision_fingerprint {
        return Err(error(
            "decisions changed since review generation; reload before recording",
        ));
    }
    Ok(())
}
/// The supplied configuration must be the effective current analysis config.
pub fn read_saved_review(root: &Path, loaded: &LoadedConfig, now: u64) -> SavedReview {
    let read = || -> Result<SavedReview, ArchguardError> {
        let bytes = match crate::project_files::read_repo_file(root, Path::new(LATEST), MAX_BYTES) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(SavedReview {
                    status: "not_recorded".into(),
                    report: None,
                    diagnostics: vec![],
                });
            }
            Err(e) => return Err(e.into()),
        };
        let report: ReviewReport = serde_json::from_slice(&bytes).map_err(error)?;
        if report.schema_version != 1
            || !evidence::valid_hash(&report.revision)
            || !evidence::valid_hash(&report.input_fingerprint)
        {
            return Err(error("invalid saved review identity or version"));
        }
        let root = root.canonicalize()?;
        let mut diagnostics = vec![];
        if root.to_string_lossy() != report.project_root {
            diagnostics.push("Review was recorded for another project root.".into());
        }
        match inputs(&root, loaded, report.base_ref.as_deref()) {
            Ok(current) if current == report.input_fingerprint => {}
            Ok(_) => diagnostics.push(
                "Source, policy, base reference or evidence changed since this review.".into(),
            ),
            Err(e) => diagnostics.push(format!("Current inputs could not be verified: {e}")),
        }
        if snapshot_fingerprint(&root, DECISIONS) != report.decision_fingerprint {
            diagnostics.push("Decisions changed since this review.".into());
        }
        if report.findings.iter().any(|f| {
            f.disposition == "accepted_risk"
                && f.decision
                    .as_ref()
                    .and_then(|d| d.expires_on.as_ref())
                    .is_some_and(|date| expiry_day(date).is_ok_and(|day| now / 86_400 > day))
        }) {
            diagnostics.push("A recorded exception has expired; generate a new review.".into());
        }
        Ok(SavedReview {
            status: if diagnostics.is_empty() {
                "current"
            } else {
                "stale"
            }
            .into(),
            report: Some(report),
            diagnostics,
        })
    };
    read().unwrap_or_else(|e| SavedReview {
        status: "error".into(),
        report: None,
        diagnostics: vec![e.to_string()],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn an_outdated_import_cannot_reuse_an_accepted_fingerprint() {
        let ready = imported_finding(
            "security",
            "CVE-1",
            "high",
            "Advisory",
            vec![],
            "ready",
            json!({"id":"CVE-1"}),
        )
        .unwrap();
        let stale = imported_finding(
            "security",
            "CVE-1",
            "high",
            "Advisory",
            vec![],
            "stale",
            json!({"id":"CVE-1"}),
        )
        .unwrap();
        assert_eq!(ready.id, stale.id);
        assert_ne!(ready.fingerprint, stale.fingerprint);
    }
}
