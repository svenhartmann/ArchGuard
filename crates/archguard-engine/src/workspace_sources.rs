//! Project-qualified source and call evidence across declared workspace boundaries.
use archguard_core::{
    ArchguardError, CallAnalysis, Config, ConfigSource, Graph, GraphBuilder, NodeKind, Ref,
    ServiceProject, WorkspaceReport,
};
use archguard_extract::{Lang, SourceFile, TargetSpec, extractor_for, strip_root};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    path::{Path, PathBuf},
};

const MAX_PROJECTS: usize = 64;
const MAX_PROJECT_FILES: usize = 2_000;
const MAX_FILES: usize = 10_000;
const MAX_SOURCE_BYTES: usize = 64 * 1024 * 1024;
const MAX_JAVA_SCOPES: usize = 24;
const MAX_JAVA_VISITS: usize = 8_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceSourceFile {
    pub id: String,
    pub project: String,
    pub name: String,
    pub path: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceProjectCoverage {
    pub project: String,
    pub status: String,
    pub files: usize,
    pub call_status: String,
    pub diagnostics: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceTestMapping {
    pub node: Option<String>,
    pub path: String,
    pub status: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceTestEvidence {
    pub id: String,
    pub project: String,
    pub suite: String,
    pub name: String,
    pub outcome: String,
    pub duration_seconds: Option<f64>,
    pub mappings: Vec<WorkspaceTestMapping>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceSourceReport {
    pub schema_version: u32,
    pub status: String,
    pub graph: Graph,
    pub files: Vec<WorkspaceSourceFile>,
    pub calls: CallAnalysis,
    pub projects: Vec<WorkspaceProjectCoverage>,
    pub tests: Vec<WorkspaceTestEvidence>,
    pub cycles: Vec<Vec<String>>,
    pub diagnostics: Vec<String>,
    pub limitations: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceImpactedTest {
    pub test_id: String,
    pub project: String,
    pub name: String,
    pub suite: String,
    pub outcome: String,
    pub duration_seconds: Option<f64>,
    pub reason: String,
    pub matched_sources: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceTestImpact {
    pub status: String,
    pub targets: Vec<String>,
    pub unresolved_targets: Vec<String>,
    pub affected_files: Vec<String>,
    pub affected_projects: Vec<String>,
    pub selected: Vec<WorkspaceImpactedTest>,
    pub unknown_tests: usize,
    pub safe_to_skip_other_tests: bool,
    pub diagnostics: Vec<String>,
}

pub fn analyze_workspace_sources(
    root: &Path,
    workspace: &WorkspaceReport,
) -> Result<WorkspaceSourceReport, ArchguardError> {
    let root = root.canonicalize()?;
    let (inputs, mut diagnostics) = gather(&root, workspace)?;
    let mut result=WorkspaceSourceReport {
        schema_version: 1,
        status: workspace.status.clone(),
        graph: GraphBuilder::new().build(root.display().to_string()),
        files: vec![],
        calls: CallAnalysis { java_release: 0, ..Default::default() },
        projects: vec![],
        tests: vec![],
        cycles: vec![],
        diagnostics: vec![],
        limitations: vec![
            "Source identities include the manifest project ID. Import graphs are built in isolated project scopes; deployment/API configuration is not a source call.".into(),
            "Cross-project compiler calls currently cover Java. Other languages retain isolated import/call evidence; no cross-project method call is guessed from an import.".into(),
            "Java production visibility uses direct compile/provided/optional dependencies and transitive compile dependencies. Test dependencies are included only for src/test sources; dependency test sources are never added. Maven profiles, exclusions and full build-system resolution are not executed.".into(),
            "Graph cycles are strongly connected file groups, not ordered paths. Declared project cycles and observed source cycles are different measurements.".into(),
            "Test suggestions use explicit imported mappings and static reverse dependency reachability. Missing/dynamic relationships and unmapped/stale tests prevent any guarantee that other tests can be skipped.".into(),
            format!("Work limits: {MAX_PROJECTS} projects, {MAX_PROJECT_FILES} files/project, {MAX_FILES} files total, 64 MiB source text, {MAX_JAVA_SCOPES} Java scopes and {MAX_JAVA_VISITS} Java source visits; omissions are partial."),
        ],
    };
    let mut builder = GraphBuilder::new();
    let mut global_files = BTreeMap::<String, WorkspaceSourceFile>::new();
    // Build each graph alone. An unrelated project's duplicate qualified class
    // name cannot participate in import resolution or compiler attribution.
    for input in &inputs {
        let (graph, warnings) =
            crate::pipeline::build_graph(&input.root, &input.files, &input.config)?;
        let mut by_name = BTreeMap::<String, Vec<&NamedSource>>::new();
        for file in &input.named {
            by_name.entry(file.name.clone()).or_default().push(file);
        }
        let mut local_to_global = BTreeMap::new();
        let mut project_diagnostics = input.diagnostics.clone();
        let mut project_partial = !input.diagnostics.is_empty();
        for node in &graph.nodes {
            if node.kind != NodeKind::File || node.name.ends_with("/*") {
                continue;
            }
            let Some(candidates) = by_name.get(&node.name) else {
                continue;
            };
            if candidates.len() != 1 {
                project_partial = true;
                project_diagnostics.push(format!("Ambiguous source identity {} belongs to {} files; no merged file or edges were published.",node.name,candidates.len()));
                continue;
            }
            let source = candidates[0];
            let id = qualify(&input.project.id, &node.name);
            let index = builder.ensure_path(&id, node.lang.as_deref());
            local_to_global.insert(node.id, index);
            global_files.insert(
                source.repo_path.clone(),
                WorkspaceSourceFile {
                    id: id.clone(),
                    project: input.project.id.clone(),
                    name: node.name.clone(),
                    path: source.repo_path.clone(),
                },
            );
        }
        for edge in &graph.edges {
            let (Some(from), Some(to)) = (
                local_to_global.get(&edge.from),
                local_to_global.get(&edge.to),
            ) else {
                continue;
            };
            for reference in &edge.refs {
                let source = input.named.iter().find(|file| {
                    file.stripped == reference.file || file.relative == reference.file
                });
                let mut reference = reference.clone();
                if let Some(source) = source {
                    reference.file = source.repo_path.clone();
                }
                builder.edge(*from, *to, reference)?;
            }
        }
        if !warnings.is_empty() {
            project_diagnostics.push(format!("{} imports were unresolved in the isolated project; supported explicit cross-project imports are resolved separately.",warnings.len()));
        }
        let tests = crate::test_evidence::read_test_evidence(&input.root, &input.config, &graph);
        if !matches!(tests.status.as_str(), "ready" | "not_imported") {
            project_diagnostics.push(format!(
                "Imported tests: {}. {}",
                tests.status,
                tests.diagnostics.join(" ")
            ));
        }
        for test in tests.tests {
            result.tests.push(WorkspaceTestEvidence {
                id: qualify(&input.project.id, &test.id),
                project: input.project.id.clone(),
                suite: test.suite,
                name: test.name,
                outcome: test.outcome,
                duration_seconds: test.duration_seconds,
                mappings: test
                    .mappings
                    .into_iter()
                    .map(|mapping| WorkspaceTestMapping {
                        node: mapping
                            .node
                            .as_ref()
                            .map(|name| qualify(&input.project.id, name))
                            .filter(|id| global_files.values().any(|file| &file.id == id)),
                        path: repo_path(&input.project.path, &mapping.path),
                        status: mapping.status,
                    })
                    .collect(),
            });
        }
        result.projects.push(WorkspaceProjectCoverage {
            project: input.project.id.clone(),
            status: if project_partial { "partial" } else { "ready" }.into(),
            files: input.files.len(),
            call_status: "not_applicable".into(),
            diagnostics: project_diagnostics,
        });
    }
    let mut indexes = BTreeMap::new();
    for file in global_files.values() {
        indexes.insert(file.id.clone(), builder.ensure_path(&file.id, None));
    }
    let by_project: BTreeMap<_, _> = inputs
        .iter()
        .map(|input| (input.project.id.clone(), input))
        .collect();
    // Explicit Java class imports require one visible exact file; imports never
    // turn into fabricated method calls.
    for input in &inputs {
        for source in &input.named {
            let Some(from) = global_files.get(&source.repo_path) else {
                continue;
            };
            if source.language != Lang::Java {
                continue;
            }
            let visible = visibility(workspace, &input.project.id, is_test(&source.relative));
            for import in &source.imports {
                let TargetSpec::Symbol {
                    path,
                    stem: Some(stem),
                } = &import.target
                else {
                    continue;
                };
                let name = format!("{path}/{stem}.java");
                let candidates: Vec<_> = global_files
                    .values()
                    .filter(|file| file.name == name && visible.contains(&file.project))
                    .collect();
                if candidates.len() == 1 && candidates[0].project != input.project.id {
                    builder.edge(
                        indexes[&from.id],
                        indexes[&candidates[0].id],
                        Ref {
                            file: source.repo_path.clone(),
                            line: import.line,
                            raw: import.raw.clone(),
                        },
                    )?;
                }
            }
        }
    }
    let mut java_scopes = 0;
    let mut java_visits = 0;
    for input in &inputs {
        let java_count = input
            .files
            .iter()
            .filter(|file| file.lang == Lang::Java)
            .count();
        let mut reports = Vec::new();
        if java_count > 0 {
            let visible = visibility(workspace, &input.project.id, true);
            let mut closure = Vec::new();
            let mut names = BTreeMap::<String, usize>::new();
            for id in &visible {
                let Some(dependency) = by_project.get(id) else {
                    continue;
                };
                for (file, named) in dependency.files.iter().zip(&dependency.named) {
                    if file.lang != Lang::Java
                        || (id != &input.project.id && is_test(&file.rel_path))
                    {
                        continue;
                    }
                    closure.push(SourceFile {
                        path: file.path.clone(),
                        rel_path: named.repo_path.clone(),
                        lang: file.lang,
                    });
                    *names.entry(named.name.clone()).or_default() += 1;
                }
            }
            if names.values().any(|count| *count > 1) {
                diagnostics.push(format!("{}: duplicate Java source identities in the declared closure; compiler scope not combined.",input.project.id));
                // Retain the independent project's own calls; no ambiguous
                // dependency target enters its compiler scope.
                closure = input
                    .files
                    .iter()
                    .zip(&input.named)
                    .filter(|(file, _)| file.lang == Lang::Java)
                    .map(|(file, named)| SourceFile {
                        path: file.path.clone(),
                        rel_path: named.repo_path.clone(),
                        lang: file.lang,
                    })
                    .collect();
            }
            if java_scopes >= MAX_JAVA_SCOPES
                || closure.len() > MAX_PROJECT_FILES
                || java_visits + closure.len() > MAX_JAVA_VISITS
            {
                diagnostics.push(format!("{}: Java compiler work limit reached; call coverage unavailable for this project.",input.project.id));
                reports.push((
                    CallAnalysis {
                        status: "unavailable".into(),
                        files: java_count,
                        diagnostics: vec!["Workspace compiler work limit reached.".into()],
                        ..Default::default()
                    },
                    true,
                ));
            } else {
                java_scopes += 1;
                java_visits += closure.len();
                let mut config = input.config.clone();
                config.project.source_roots = vec![];
                config.project.languages = vec!["java".into()];
                config.semantic.java.classpath = config
                    .semantic
                    .java
                    .classpath
                    .iter()
                    .map(|path| input.root.join(path).display().to_string())
                    .collect();
                reports.push((
                    crate::java_calls::analyze_java_workspace_calls(&root, &config, &closure),
                    true,
                ));
            }
        }
        if input.files.iter().any(|file| file.lang != Lang::Java) {
            let mut config = input.config.clone();
            for file in input.files.iter().filter(|file| file.lang == Lang::Java) {
                config.project.exclude.push(escape_glob(&file.rel_path));
            }
            if input.diagnostics.is_empty() {
                reports.push((
                    crate::semantic_calls::analyze_calls(&input.root, &config),
                    false,
                ));
            } else {
                reports.push((CallAnalysis {status:"unavailable".into(),diagnostics:vec!["Project source boundary is incomplete; non-Java compiler analysis was not run.".into()],..Default::default()},false));
            }
            if workspace
                .dependencies
                .iter()
                .any(|dependency| dependency.from == input.project.id)
            {
                diagnostics.push(format!("{}: cross-project compiler calls are not implemented for its non-Java sources; isolated calls remain available.",input.project.id));
            }
        }
        let mut statuses = Vec::new();
        for (analysis, repo_relative) in reports {
            statuses.push(analysis.status.clone());
            let coverage = result
                .projects
                .iter_mut()
                .find(|coverage| coverage.project == input.project.id)
                .unwrap();
            coverage
                .diagnostics
                .extend(analysis.diagnostics.iter().take(20).cloned());
            merge_calls(
                &mut result.calls,
                analysis,
                input,
                workspace,
                &global_files,
                repo_relative,
            );
        }
        let coverage = result
            .projects
            .iter_mut()
            .find(|coverage| coverage.project == input.project.id)
            .unwrap();
        coverage.call_status = if statuses.is_empty() {
            "not_applicable"
        } else if statuses.iter().all(|s| s == "ready") {
            "ready"
        } else {
            "partial"
        }
        .into();
        if coverage.call_status == "partial" {
            coverage.status = "partial".into();
        }
    }
    // Semantic edges retain the real source call/inheritance sites.
    let class_files: BTreeMap<_, _> = result
        .calls
        .classes
        .iter()
        .map(|class| (class.id.clone(), class.graph_node.clone()))
        .collect();
    let mut projected_calls = 0;
    let mut projected_supertypes = 0;
    for call in &result.calls.calls {
        if call.resolution != "project" {
            continue;
        }
        if let (Some(from), Some(to)) = (
            class_files.get(&call.from_class),
            call.to_class.as_ref().and_then(|id| class_files.get(id)),
        ) && let (Some(from), Some(to)) = (indexes.get(from), indexes.get(to))
        {
            builder.edge(
                *from,
                *to,
                Ref {
                    file: call.file.clone(),
                    line: call.line,
                    raw: format!("{} [compiler-resolved {} call]", call.code, call.dispatch),
                },
            )?;
            if from != to {
                projected_calls += 1;
            }
        }
    }
    for class in &result.calls.classes {
        for target in &class.supertypes {
            if let (Some(from), Some(to)) = (
                indexes.get(&class.graph_node),
                class_files.get(target).and_then(|file| indexes.get(file)),
            ) {
                builder.edge(
                    *from,
                    *to,
                    Ref {
                        file: class.file.clone(),
                        line: class.line,
                        raw: format!(
                            "{} extends/implements {} [compiler-resolved type]",
                            class.name, target
                        ),
                    },
                )?;
                if from != to {
                    projected_supertypes += 1;
                }
            }
        }
    }
    let mut projected_type_uses = 0;
    let mut skipped_type_uses = 0;
    if let Some(details) = &result.calls.details {
        let source_nodes: BTreeMap<_, _> = global_files
            .values()
            .map(|f| (f.path.as_str(), f.id.as_str()))
            .collect();
        let target_classes: BTreeMap<_, _> = result
            .calls
            .classes
            .iter()
            .map(|c| (c.id.as_str(), c))
            .collect();
        for used in details
            .type_uses
            .iter()
            .filter(|u| u.resolution == "project")
        {
            let endpoints = (|| {
                let to = target_classes.get(used.to_class.as_deref()?)?;
                if to
                    .declaration_files
                    .as_ref()
                    .is_some_and(|files| files.len() != 1)
                {
                    return None;
                }
                Some((
                    indexes.get(*source_nodes.get(used.file.as_str())?)?,
                    indexes.get(&to.graph_node)?,
                ))
            })();
            if let Some((from, to)) = endpoints {
                builder.edge(
                    *from,
                    *to,
                    Ref {
                        file: used.file.clone(),
                        line: used.line,
                        raw: format!("[semantic type:{}] {}", used.role, used.code),
                    },
                )?;
                if from != to {
                    projected_type_uses += 1;
                }
            } else {
                skipped_type_uses += 1;
            }
        }
    }
    let cohesion = crate::class_cohesion(&result.calls);
    if let Some(details) = &mut result.calls.details {
        details.cohesion = cohesion;
    }
    result.graph = builder.build(root.display().to_string());
    let graph_names: BTreeMap<_, _> = result
        .graph
        .nodes
        .iter()
        .map(|node| (node.id, &node.name))
        .collect();
    result.cycles = archguard_analysis::find_sccs(&result.graph)
        .into_iter()
        .map(|scc| {
            scc.node_ids
                .iter()
                .map(|id| graph_names[id].clone())
                .collect()
        })
        .collect();
    result.files = global_files.into_values().collect();
    result.files.sort_by(|a, b| a.id.cmp(&b.id));
    result.calls.classes.sort_by(|a, b| a.id.cmp(&b.id));
    result.calls.classes.dedup_by(|a, b| a.id == b.id);
    result.calls.methods.sort_by(|a, b| a.id.cmp(&b.id));
    result.calls.methods.dedup_by(|a, b| a.id == b.id);
    result.calls.calls.sort_by(|a, b| {
        (
            &a.from_class,
            &a.from_method,
            &a.to_class,
            &a.to_method,
            &a.file,
            a.line,
            a.column,
        )
            .cmp(&(
                &b.from_class,
                &b.from_method,
                &b.to_class,
                &b.to_method,
                &b.file,
                b.line,
                b.column,
            ))
    });
    result.calls.calls.dedup();
    result.calls.files = result.files.len();
    result.calls.status = if result
        .projects
        .iter()
        .all(|coverage| coverage.call_status == "not_applicable")
    {
        "not_applicable"
    } else if diagnostics.is_empty()
        && result
            .projects
            .iter()
            .all(|coverage| matches!(coverage.call_status.as_str(), "ready" | "not_applicable"))
    {
        "ready"
    } else {
        "partial"
    }
    .into();
    result.calls.diagnostics = diagnostics.iter().take(100).cloned().collect();
    result.status = if workspace.status == "partial"
        || !diagnostics.is_empty()
        || result
            .projects
            .iter()
            .any(|coverage| coverage.status == "partial")
    {
        "partial"
    } else if result.files.is_empty() {
        "not_applicable"
    } else {
        "ready"
    }
    .into();
    diagnostics.truncate(200);
    result.diagnostics = diagnostics;
    result.graph.semantic = Some(archguard_core::SemanticDependencyCoverage {
        status: result.calls.status.clone(),
        analyzers: result.calls.analyzers.clone(),
        projected_calls,
        projected_supertypes,
        projected_type_uses,
        skipped_type_uses,
        skipped_project_calls: 0,
        diagnostics: result.calls.diagnostics.clone(),
    });
    Ok(result)
}

pub fn workspace_test_impact(
    report: &WorkspaceSourceReport,
    targets: &[String],
) -> WorkspaceTestImpact {
    let mut direct = BTreeSet::new();
    let mut unresolved = Vec::new();
    for target in targets {
        let matches: Vec<_> = report
            .files
            .iter()
            .filter(|file| file.id == *target || file.path == *target || file.project == *target)
            .collect();
        if matches.is_empty() {
            unresolved.push(target.clone());
        } else {
            direct.extend(matches.iter().map(|file| file.id.clone()));
        }
    }
    let names: BTreeMap<_, _> = report
        .graph
        .nodes
        .iter()
        .map(|node| (node.id, node.name.clone()))
        .collect();
    let mut reverse = BTreeMap::<String, Vec<String>>::new();
    for edge in &report.graph.edges {
        if let (Some(from), Some(to)) = (names.get(&edge.from), names.get(&edge.to)) {
            reverse.entry(to.clone()).or_default().push(from.clone());
        }
    }
    let mut affected = direct.clone();
    let mut queue: VecDeque<_> = direct.iter().cloned().collect();
    while let Some(target) = queue.pop_front() {
        for from in reverse.get(&target).into_iter().flatten() {
            if affected.insert(from.clone()) {
                queue.push_back(from.clone());
            }
        }
    }
    let mut selected = Vec::new();
    let mut unknown = 0;
    for test in &report.tests {
        if test.mappings.is_empty()
            || test
                .mappings
                .iter()
                .any(|mapping| mapping.node.is_none() || mapping.status != "unchanged_since_import")
        {
            unknown += 1;
        }
        let matches: Vec<_> = test
            .mappings
            .iter()
            .filter(|mapping| mapping.status == "unchanged_since_import")
            .filter_map(|mapping| mapping.node.as_ref())
            .filter(|node| affected.contains(*node))
            .cloned()
            .collect();
        if matches.is_empty() {
            continue;
        }
        let reason = if matches.iter().any(|node| direct.contains(node)) {
            "mapped_changed_source"
        } else {
            "mapped_static_dependent"
        };
        selected.push(WorkspaceImpactedTest {
            test_id: test.id.clone(),
            project: test.project.clone(),
            name: test.name.clone(),
            suite: test.suite.clone(),
            outcome: test.outcome.clone(),
            duration_seconds: test.duration_seconds,
            reason: reason.into(),
            matched_sources: matches,
        });
    }
    let affected_projects: Vec<_> = report
        .files
        .iter()
        .filter(|file| affected.contains(&file.id))
        .map(|file| file.project.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let missing_test_projects: Vec<_> = affected_projects
        .iter()
        .filter(|project| !report.tests.iter().any(|test| &test.project == *project))
        .cloned()
        .collect();
    let mut diagnostics=vec!["Imported mappings are unverified producer associations. Static reachability is incomplete for dynamic or unsupported dependencies; no other test is guaranteed safe to omit.".into()];
    if !missing_test_projects.is_empty() {
        diagnostics.push(format!(
            "No imported test evidence for affected projects: {}.",
            missing_test_projects.join(", ")
        ));
    }
    WorkspaceTestImpact {
        status: if report.status == "ready"
            && unresolved.is_empty()
            && unknown == 0
            && !report.tests.is_empty()
            && missing_test_projects.is_empty()
        {
            "ready"
        } else {
            "partial"
        }
        .into(),
        targets: targets.to_vec(),
        unresolved_targets: unresolved,
        affected_files: affected.iter().cloned().collect(),
        affected_projects,
        selected,
        unknown_tests: unknown,
        safe_to_skip_other_tests: false,
        diagnostics,
    }
}

pub fn workspace_sources_fingerprint(
    root: &Path,
    workspace: &WorkspaceReport,
) -> Result<String, ArchguardError> {
    let (inputs, diagnostics) = gather(&root.canonicalize()?, workspace)?;
    let mut hash = Sha256::new();
    hash.update(serde_json::to_vec(workspace).unwrap());
    hash.update(serde_json::to_vec(&diagnostics).unwrap());
    for input in inputs {
        hash.update(input.project.id.as_bytes());
        hash.update(serde_json::to_vec(&input.config).unwrap());
        for file in &input.named {
            hash.update(file.repo_path.as_bytes());
            hash.update(file.hash.as_bytes());
        }
        hash.update(crate::test_evidence::test_evidence_fingerprint(&input.root).as_bytes());
        if input.diagnostics.is_empty() {
            for fingerprint in
                crate::semantic_calls::semantic_fingerprints(&input.root, &input.config)
            {
                match fingerprint {
                    Ok(bytes) => hash.update(bytes),
                    Err(message) => hash.update(message.as_bytes()),
                }
            }
        }
    }
    Ok(format!("{:x}", hash.finalize()))
}

struct NamedSource {
    relative: String,
    stripped: String,
    repo_path: String,
    name: String,
    hash: String,
    language: Lang,
    imports: Vec<archguard_extract::RawImport>,
}
struct Input {
    project: ServiceProject,
    root: PathBuf,
    config: Config,
    files: Vec<SourceFile>,
    named: Vec<NamedSource>,
    diagnostics: Vec<String>,
}
fn qualify(project: &str, id: &str) -> String {
    // A manifest directory may itself contain "::" on Unix. Escape the
    // namespace delimiter and escape character so project/source pairs remain
    // injective while ordinary project IDs stay readable and unchanged.
    let project = project.replace('%', "%25").replace("::", "%3A%3A");
    format!("{project}::{id}")
}
fn repo_path(project: &str, path: &str) -> String {
    if project == "." {
        path.into()
    } else {
        format!("{project}/{path}")
    }
}
fn escape_glob(value: &str) -> String {
    value
        .chars()
        .flat_map(|c| {
            if "?*[]{}\\".contains(c) {
                vec!['\\', c]
            } else {
                vec![c]
            }
        })
        .collect()
}
fn is_test(path: &str) -> bool {
    path.starts_with("src/test/") || path.contains("/src/test/")
}
fn gather(
    root: &Path,
    workspace: &WorkspaceReport,
) -> Result<(Vec<Input>, Vec<String>), ArchguardError> {
    let mut inputs = Vec::new();
    let mut diagnostics = Vec::new();
    let mut total_files = 0;
    let mut total_bytes = 0;
    if workspace.projects.len() > MAX_PROJECTS {
        diagnostics.push(format!(
            "Project limit reached: analyzing the first {MAX_PROJECTS} of {} projects.",
            workspace.projects.len()
        ));
    }
    for project in workspace.projects.iter().take(MAX_PROJECTS) {
        if workspace
            .projects
            .iter()
            .filter(|other| other.path == project.path)
            .count()
            != 1
        {
            diagnostics.push(format!("{}: multiple manifests share this source root; ownership is ambiguous, project sources omitted.",project.id));
            continue;
        }
        let path = root.join(&project.path).canonicalize()?;
        if !path.starts_with(root) {
            diagnostics.push(format!(
                "{}: project path escapes repository; sources omitted.",
                project.id
            ));
            continue;
        }
        let mut loaded = crate::config::load_config(None, &path)?;
        if loaded.sources.get("project") == Some(&ConfigSource::Default) {
            let (roots, languages) = crate::effective::derive_project_hints(&path)?;
            loaded.config.project.source_roots = roots;
            loaded.config.project.languages = languages;
        }
        let mut config = loaded.config;
        for other in &workspace.projects {
            if other.id == project.id {
                continue;
            }
            let prefix = if project.path == "." {
                Some(other.path.as_str())
            } else {
                other.path.strip_prefix(&(project.path.clone() + "/"))
            };
            if let Some(prefix) = prefix.filter(|prefix| !prefix.is_empty() && *prefix != ".") {
                config
                    .project
                    .exclude
                    .push(format!("{}/**", escape_glob(prefix)));
            }
        }
        for pattern in [
            "**/node_modules/**",
            "**/target/**",
            "**/bin/**",
            "**/obj/**",
            "**/vendor/**",
        ] {
            config.project.exclude.push(pattern.into());
        }
        let found = crate::pipeline::walk_files(&path, &config)?;
        let mut input = Input {
            project: project.clone(),
            root: path,
            config,
            files: vec![],
            named: vec![],
            diagnostics: vec![],
        };
        for file in found {
            if input.files.len() >= MAX_PROJECT_FILES || total_files >= MAX_FILES {
                if !input
                    .diagnostics
                    .iter()
                    .any(|s| s.contains("Source file limit"))
                {
                    input
                        .diagnostics
                        .push("Source file limit reached; remaining files omitted.".into());
                }
                input
                    .config
                    .project
                    .exclude
                    .push(escape_glob(&file.rel_path));
                continue;
            }
            let bytes = match crate::security::read_repo_file(
                &input.root,
                Path::new(&file.rel_path),
                8 * 1024 * 1024,
            ) {
                Ok(bytes) => bytes,
                Err(error) => {
                    input.diagnostics.push(format!(
                        "{}: unreadable, non-regular or oversized source omitted: {error}",
                        file.rel_path
                    ));
                    input
                        .config
                        .project
                        .exclude
                        .push(escape_glob(&file.rel_path));
                    continue;
                }
            };
            if total_bytes + bytes.len() > MAX_SOURCE_BYTES {
                input
                    .diagnostics
                    .push("Source byte limit reached; remaining files omitted.".into());
                input
                    .config
                    .project
                    .exclude
                    .push(escape_glob(&file.rel_path));
                continue;
            }
            let source = std::str::from_utf8(&bytes).map_err(|_| {
                ArchguardError::Analysis(format!("Source {} is not UTF-8", file.rel_path))
            })?;
            let Some(extractor) = extractor_for(file.lang) else {
                continue;
            };
            let stripped = strip_root(&file.rel_path, &input.config.project.source_roots);
            let extraction = extractor.extract(&stripped, source);
            let name = crate::pipeline::file_node_name(&stripped, &extraction.declarations);
            input.named.push(NamedSource {
                relative: file.rel_path.clone(),
                stripped,
                repo_path: repo_path(&project.path, &file.rel_path),
                name,
                hash: format!("{:x}", Sha256::digest(&bytes)),
                language: file.lang,
                imports: extraction.imports,
            });
            input.files.push(file);
            total_files += 1;
            total_bytes += bytes.len();
        }
        diagnostics.extend(
            input
                .diagnostics
                .iter()
                .map(|message| format!("{}: {message}", project.id)),
        );
        inputs.push(input);
    }
    Ok((inputs, diagnostics))
}
fn visibility(workspace: &WorkspaceReport, project: &str, tests: bool) -> BTreeSet<String> {
    let mut visible = BTreeSet::from([project.to_owned()]);
    let mut queue = VecDeque::new();
    for edge in workspace
        .dependencies
        .iter()
        .filter(|edge| edge.from == project)
    {
        if (matches!(edge.scope.as_str(), "compile" | "provided" | "optional")
            || (tests && edge.scope == "test"))
            && visible.insert(edge.to.clone())
        {
            queue.push_back(edge.to.clone());
        }
    }
    while let Some(project) = queue.pop_front() {
        for edge in workspace
            .dependencies
            .iter()
            .filter(|edge| edge.from == project && edge.scope == "compile")
        {
            if visible.insert(edge.to.clone()) {
                queue.push_back(edge.to.clone());
            }
        }
    }
    visible
}

fn merge_calls(
    target: &mut CallAnalysis,
    analysis: CallAnalysis,
    input: &Input,
    workspace: &WorkspaceReport,
    files: &BTreeMap<String, WorkspaceSourceFile>,
    repo_relative: bool,
) {
    let locate = |file: &str| -> Option<&WorkspaceSourceFile> {
        if repo_relative {
            files.get(file)
        } else {
            let found: Vec<_> = input
                .named
                .iter()
                .filter(|source| source.relative == file || source.stripped == file)
                .collect();
            if found.len() == 1 {
                files.get(&found[0].repo_path)
            } else {
                None
            }
        }
    };
    let mut classes = BTreeMap::<String, (&archguard_core::CallClass, &WorkspaceSourceFile)>::new();
    let mut duplicate = BTreeSet::new();
    for class in &analysis.classes {
        if let Some(file) = locate(&class.file)
            && classes.insert(class.id.clone(), (class, file)).is_some()
        {
            duplicate.insert(class.id.clone());
        }
    }
    for id in duplicate {
        classes.remove(&id);
    }
    let methods: BTreeMap<_, _> = analysis
        .methods
        .iter()
        .filter_map(|method| {
            classes
                .get(&method.class_id)
                .map(|(_, file)| (method.id.clone(), qualify(&file.project, &method.id)))
        })
        .collect();
    if let Some(details) = analysis.details {
        let combined = target.details.get_or_insert_with(Default::default);
        combined.enumeration_complete = Some(
            combined.enumeration_complete.unwrap_or(true)
                && details.enumeration_complete == Some(true),
        );
        for mut metric in details.method_metrics {
            let Some(id) = methods.get(&metric.method_id) else {
                continue;
            };
            if !analysis
                .methods
                .iter()
                .find(|m| m.id == metric.method_id)
                .and_then(|m| classes.get(&m.class_id))
                .is_some_and(|(_, f)| f.project == input.project.id)
            {
                continue;
            }
            metric.method_id = id.clone();
            if !combined
                .method_metrics
                .iter()
                .any(|m| m.method_id == metric.method_id)
            {
                combined.method_metrics.push(metric);
            }
        }
        for mut parent in details.inheritance {
            let Some((_, from)) = classes
                .get(&parent.class_id)
                .filter(|(_, f)| f.project == input.project.id)
            else {
                continue;
            };
            parent.class_id = qualify(&from.project, &parent.class_id);
            if let Some(base) = parent.base_class.as_ref().and_then(|id| classes.get(id)) {
                let visible = visibility(workspace, &from.project, is_test(&from.path));
                if visible.contains(&base.1.project) {
                    parent.base_class = Some(qualify(&base.1.project, &base.0.id));
                } else {
                    parent.base_class = None;
                    parent.depth = None;
                    parent.status = "partial".into();
                }
            }
            if !combined
                .inheritance
                .iter()
                .any(|p| p.class_id == parent.class_id)
            {
                combined.inheritance.push(parent);
            }
        }
        let field_ids: BTreeMap<_, _> = details
            .class_states
            .iter()
            .filter_map(|state| classes.get(&state.class_id).map(|(_, file)| (state, file)))
            .flat_map(|(state, file)| {
                state
                    .fields
                    .iter()
                    .map(move |field| (field.id.clone(), qualify(&file.project, &field.id)))
            })
            .collect();
        for mut used in details.type_uses {
            let Some((_, from)) = classes.get(&used.from_class) else {
                continue;
            };
            if from.project != input.project.id {
                continue;
            }
            let visible = visibility(workspace, &from.project, is_test(&from.path));
            let mapped = used
                .to_class
                .as_ref()
                .and_then(|id| classes.get(id))
                .filter(|(_, file)| visible.contains(&file.project));
            used.from_class = qualify(&from.project, &used.from_class);
            used.from_method = used
                .from_method
                .as_ref()
                .and_then(|m| methods.get(m))
                .cloned();
            if let Some((class, to)) = mapped {
                used.to_class = Some(qualify(&to.project, &class.id));
            } else if used.resolution == "project" {
                used.to_class = None;
                used.resolution = "unresolved".into();
            }
            used.file = locate(&used.file).unwrap_or(from).path.clone();
            combined.type_uses.push(used);
        }
        for mut state in details.class_states {
            let Some((_, file)) = classes.get(&state.class_id) else {
                continue;
            };
            if file.project != input.project.id {
                continue;
            }
            state.class_id = qualify(&file.project, &state.class_id);
            for field in &mut state.fields {
                field.id = qualify(&file.project, &field.id);
                field.file = locate(&field.file).unwrap_or(file).path.clone();
            }
            state.accesses.retain_mut(|access| {
                let (Some(method), Some(field)) =
                    (methods.get(&access.method), field_ids.get(&access.field))
                else {
                    return false;
                };
                access.method = method.clone();
                access.field = field.clone();
                access.file = locate(&access.file).unwrap_or(file).path.clone();
                true
            });
            if let Some(existing) = combined
                .class_states
                .iter_mut()
                .find(|s| s.class_id == state.class_id)
            {
                if state.status != "ready" {
                    existing.status = "partial".into();
                }
                existing.fields.extend(state.fields);
                existing.accesses.extend(state.accesses);
                let mut fields = BTreeSet::new();
                existing
                    .fields
                    .retain(|f| fields.insert((f.id.clone(), f.file.clone())));
                let mut accesses = BTreeSet::new();
                existing.accesses.retain(|a| {
                    accesses.insert((a.method.clone(), a.field.clone(), a.file.clone(), a.line))
                });
            } else {
                combined.class_states.push(state);
            }
        }
    }
    for (class, file) in classes.values() {
        let mut value = (*class).clone();
        value.id = qualify(&file.project, &class.id);
        value.package = qualify(&file.project, &class.package);
        value.file = file.path.clone();
        value.graph_node = file.id.clone();
        value.supertypes = class
            .supertypes
            .iter()
            .filter_map(|id| classes.get(id))
            .filter(|(_, parent)| {
                visibility(workspace, &file.project, is_test(&file.path)).contains(&parent.project)
            })
            .map(|(parent, parent_file)| qualify(&parent_file.project, &parent.id))
            .collect();
        if let Some(sites) = &mut value.supertype_sites {
            for site in sites.iter_mut() {
                if let Some((parent, parent_file)) = classes.get(&site.target) {
                    site.target = qualify(&parent_file.project, &parent.id);
                }
                if let Some(site_file) = locate(&site.file) {
                    site.file = site_file.path.clone();
                }
            }
        }
        if let Some(declarations) = &mut value.declaration_files {
            *declarations = declarations
                .iter()
                .filter_map(|path| locate(path))
                .map(|file| file.path.clone())
                .collect();
        }
        if let Some(index) = target
            .classes
            .iter()
            .position(|existing| existing.id == value.id)
        {
            // The declaration's own project has its full declared visibility;
            // a consumer's smaller closure must not replace that metadata.
            if file.project == input.project.id {
                target.classes[index] = value;
            }
        } else {
            target.classes.push(value);
        }
    }
    for method in analysis.methods {
        let Some((_, file)) = classes.get(&method.class_id) else {
            continue;
        };
        let mut value = method.clone();
        value.id = qualify(&file.project, &method.id);
        value.class_id = qualify(&file.project, &method.class_id);
        value.file = locate(&method.file).unwrap_or(file).path.clone();
        if let Some(index) = target
            .methods
            .iter()
            .position(|existing| existing.id == value.id)
        {
            if file.project == input.project.id {
                target.methods[index] = value;
            }
        } else {
            target.methods.push(value);
        }
    }
    for mut call in analysis.calls {
        let Some((_, from)) = classes.get(&call.from_class) else {
            continue;
        };
        if from.project != input.project.id {
            continue;
        }
        let visible = visibility(workspace, &from.project, is_test(&from.path));
        call.from_class = qualify(&from.project, &call.from_class);
        call.from_method = call
            .from_method
            .as_ref()
            .and_then(|id| methods.get(id))
            .cloned();
        let mapped = call
            .to_class
            .as_ref()
            .and_then(|id| classes.get(id))
            .filter(|(_, file)| visible.contains(&file.project));
        if let Some((class, to)) = mapped {
            call.to_class = Some(qualify(&to.project, &class.id));
            call.to_method = call
                .to_method
                .as_ref()
                .and_then(|id| methods.get(id))
                .cloned();
        } else if call.resolution == "project" {
            call.to_class = None;
            call.to_method = None;
            call.resolution = "unresolved".into();
        }
        call.file = locate(&call.file).unwrap_or(from).path.clone();
        target.calls.push(call);
    }
    for mut analyzer in analysis.analyzers {
        analyzer.project = Some(input.project.id.clone());
        target.analyzers.push(analyzer);
    }
}
