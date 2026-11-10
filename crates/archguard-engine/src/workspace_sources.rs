//! Project-qualified source and call evidence across declared workspace boundaries.
use crate::test_evidence::{TestCaseEvidence, TestSourceMapping};
use archguard_core::{
    ArchguardError, CallAnalysis, CallClass, CallMethod, CallSupertypeSite, ClassInheritance,
    ClassState, Config, ConfigSource, Graph, GraphBuilder, MethodCall, Node, NodeId, NodeKind, Ref,
    SemanticDetails, SemanticTypeUse, ServiceProject, SourceMethodMetrics, WorkspaceReport,
};
use archguard_extract::{Lang, RawImport, SourceFile, TargetSpec, extractor_for, strip_root};
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

const READY: &str = "ready";
const PARTIAL: &str = "partial";
const NOT_APPLICABLE: &str = "not_applicable";
const UNAVAILABLE: &str = "unavailable";
const PROJECT_RESOLUTION: &str = "project";
const UNRESOLVED: &str = "unresolved";

/// Workspace graph node per qualified source file ID.
type FileIndexes = BTreeMap<String, NodeId>;

pub fn analyze_workspace_sources(
    root: &Path,
    workspace: &WorkspaceReport,
) -> Result<WorkspaceSourceReport, ArchguardError> {
    let root = root.canonicalize()?;
    let (inputs, diagnostics) = gather(&root, workspace)?;
    let mut run = SourceRun {
        root: &root,
        workspace,
        result: WorkspaceSourceReport::empty(&root, workspace),
        builder: GraphBuilder::new(),
        files: BTreeMap::new(),
        diagnostics,
        java: JavaBudget::default(),
    };
    // Build each graph alone. An unrelated project's duplicate qualified class
    // name cannot participate in import resolution or compiler attribution.
    for input in &inputs {
        run.isolated_project(input)?;
    }
    let indexes = run.file_indexes();
    // Explicit Java class imports require one visible exact file; imports never
    // turn into fabricated method calls.
    for input in &inputs {
        run.java_imports(input, &indexes)?;
    }
    let by_project: BTreeMap<_, _> = inputs
        .iter()
        .map(|input| (input.project.id.clone(), input))
        .collect();
    for input in &inputs {
        run.project_calls(input, &by_project);
    }
    let semantic = run.semantic_edges(&indexes)?;
    Ok(run.finish(semantic))
}

impl WorkspaceSourceReport {
    fn empty(root: &Path, workspace: &WorkspaceReport) -> Self {
        Self {
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
        }
    }

    fn calls_status(&self, diagnostics: &[String]) -> &'static str {
        let statuses = || {
            self.projects
                .iter()
                .map(|coverage| coverage.call_status.as_str())
        };
        if statuses().all(|status| status == NOT_APPLICABLE) {
            NOT_APPLICABLE
        } else if diagnostics.is_empty()
            && statuses().all(|status| matches!(status, READY | NOT_APPLICABLE))
        {
            READY
        } else {
            PARTIAL
        }
    }

    fn source_status(&self, workspace: &WorkspaceReport, diagnostics: &[String]) -> &'static str {
        if workspace.status == PARTIAL
            || !diagnostics.is_empty()
            || self
                .projects
                .iter()
                .any(|coverage| coverage.status == PARTIAL)
        {
            PARTIAL
        } else if self.files.is_empty() {
            NOT_APPLICABLE
        } else {
            READY
        }
    }
}

/// Shared Java compiler budget across all project scopes of one workspace run.
#[derive(Default)]
struct JavaBudget {
    scopes: usize,
    visits: usize,
}

impl JavaBudget {
    /// Reserves one compiler scope of `files` source visits, if the limits allow it.
    fn admit(&mut self, files: usize) -> bool {
        if self.scopes >= MAX_JAVA_SCOPES
            || files > MAX_PROJECT_FILES
            || self.visits + files > MAX_JAVA_VISITS
        {
            return false;
        }
        self.scopes += 1;
        self.visits += files;
        true
    }
}

/// Graph edges projected from compiler evidence.
struct SemanticCounts {
    calls: usize,
    supertypes: usize,
    type_uses: usize,
    skipped_type_uses: usize,
}

impl SemanticCounts {
    fn coverage(&self, calls: &CallAnalysis) -> archguard_core::SemanticDependencyCoverage {
        archguard_core::SemanticDependencyCoverage {
            status: calls.status.clone(),
            analyzers: calls.analyzers.clone(),
            projected_calls: self.calls,
            projected_supertypes: self.supertypes,
            projected_type_uses: self.type_uses,
            skipped_type_uses: self.skipped_type_uses,
            skipped_project_calls: 0,
            diagnostics: calls.diagnostics.clone(),
        }
    }
}

/// One workspace source analysis: the report under construction, the shared
/// graph and the qualified files per repository path.
struct SourceRun<'a> {
    root: &'a Path,
    workspace: &'a WorkspaceReport,
    result: WorkspaceSourceReport,
    builder: GraphBuilder,
    files: BTreeMap<String, WorkspaceSourceFile>,
    diagnostics: Vec<String>,
    java: JavaBudget,
}

impl SourceRun<'_> {
    fn isolated_project(&mut self, input: &Input) -> Result<(), ArchguardError> {
        let (graph, warnings) =
            crate::pipeline::build_graph(&input.root, &input.files, &input.config)?;
        let mut coverage = WorkspaceProjectCoverage {
            project: input.project.id.clone(),
            status: if input.diagnostics.is_empty() {
                READY
            } else {
                PARTIAL
            }
            .into(),
            files: input.files.len(),
            call_status: NOT_APPLICABLE.into(),
            diagnostics: input.diagnostics.clone(),
        };
        let local_to_global = self.register_files(input, &graph, &mut coverage);
        self.isolated_edges(input, &graph, &local_to_global)?;
        if !warnings.is_empty() {
            coverage.diagnostics.push(format!("{} imports were unresolved in the isolated project; supported explicit cross-project imports are resolved separately.",warnings.len()));
        }
        self.imported_tests(input, &graph, &mut coverage.diagnostics);
        self.result.projects.push(coverage);
        Ok(())
    }

    /// Publishes each graph file with exactly one source; returns local to
    /// workspace node IDs.
    fn register_files(
        &mut self,
        input: &Input,
        graph: &Graph,
        coverage: &mut WorkspaceProjectCoverage,
    ) -> BTreeMap<NodeId, NodeId> {
        let mut by_name = BTreeMap::<&str, Vec<&NamedSource>>::new();
        for file in &input.named {
            by_name.entry(file.name.as_str()).or_default().push(file);
        }
        let mut local_to_global = BTreeMap::new();
        let file_nodes = graph
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::File && !node.name.ends_with("/*"));
        for node in file_nodes {
            match by_name.get(node.name.as_str()).map(Vec::as_slice) {
                None => {}
                Some([source]) => {
                    let index = self.register_file(&input.project.id, node, source);
                    local_to_global.insert(node.id, index);
                }
                Some(candidates) => {
                    coverage.status = PARTIAL.into();
                    coverage.diagnostics.push(format!("Ambiguous source identity {} belongs to {} files; no merged file or edges were published.",node.name,candidates.len()));
                }
            }
        }
        local_to_global
    }

    fn register_file(&mut self, project: &str, node: &Node, source: &NamedSource) -> NodeId {
        let id = qualify(project, &node.name);
        let index = self.builder.ensure_path(&id, node.lang.as_deref());
        self.files.insert(
            source.repo_path.clone(),
            WorkspaceSourceFile {
                id,
                project: project.to_owned(),
                name: node.name.clone(),
                path: source.repo_path.clone(),
            },
        );
        index
    }

    fn isolated_edges(
        &mut self,
        input: &Input,
        graph: &Graph,
        local_to_global: &BTreeMap<NodeId, NodeId>,
    ) -> Result<(), ArchguardError> {
        for edge in &graph.edges {
            let (Some(from), Some(to)) = (
                local_to_global.get(&edge.from),
                local_to_global.get(&edge.to),
            ) else {
                continue;
            };
            for reference in &edge.refs {
                self.builder
                    .edge(*from, *to, input.repo_reference(reference))?;
            }
        }
        Ok(())
    }

    fn imported_tests(&mut self, input: &Input, graph: &Graph, diagnostics: &mut Vec<String>) {
        let tests = crate::test_evidence::read_test_evidence(&input.root, &input.config, graph);
        if !matches!(tests.status.as_str(), "ready" | "not_imported") {
            diagnostics.push(format!(
                "Imported tests: {}. {}",
                tests.status,
                tests.diagnostics.join(" ")
            ));
        }
        for test in tests.tests {
            let evidence = self.test_evidence(&input.project, test);
            self.result.tests.push(evidence);
        }
    }

    fn test_evidence(
        &self,
        project: &ServiceProject,
        test: TestCaseEvidence,
    ) -> WorkspaceTestEvidence {
        WorkspaceTestEvidence {
            id: qualify(&project.id, &test.id),
            project: project.id.clone(),
            suite: test.suite,
            name: test.name,
            outcome: test.outcome,
            duration_seconds: test.duration_seconds,
            mappings: test
                .mappings
                .into_iter()
                .map(|mapping| self.test_mapping(project, mapping))
                .collect(),
        }
    }

    /// Keeps a mapped node only when it names a file published so far.
    fn test_mapping(
        &self,
        project: &ServiceProject,
        mapping: TestSourceMapping,
    ) -> WorkspaceTestMapping {
        WorkspaceTestMapping {
            node: mapping
                .node
                .as_ref()
                .map(|name| qualify(&project.id, name))
                .filter(|id| self.files.values().any(|file| &file.id == id)),
            path: repo_path(&project.path, &mapping.path),
            status: mapping.status,
        }
    }

    fn file_indexes(&mut self) -> FileIndexes {
        let mut indexes = BTreeMap::new();
        for file in self.files.values() {
            indexes.insert(file.id.clone(), self.builder.ensure_path(&file.id, None));
        }
        indexes
    }

    fn java_imports(&mut self, input: &Input, indexes: &FileIndexes) -> Result<(), ArchguardError> {
        let java = input
            .named
            .iter()
            .filter(|source| source.language == Lang::Java);
        for source in java {
            let Some(from) = self.files.get(&source.repo_path) else {
                continue;
            };
            let from = indexes[&from.id];
            let visible = visibility(self.workspace, &input.project.id, is_test(&source.relative));
            for import in &source.imports {
                let Some(to) = self.import_target(import, &visible, &input.project.id, indexes)
                else {
                    continue;
                };
                let reference = Ref {
                    file: source.repo_path.clone(),
                    line: import.line,
                    raw: import.raw.clone(),
                };
                self.builder.edge(from, to, reference)?;
            }
        }
        Ok(())
    }

    /// The single visible file of another project that declares the imported class.
    fn import_target(
        &self,
        import: &RawImport,
        visible: &BTreeSet<String>,
        project: &str,
        indexes: &FileIndexes,
    ) -> Option<NodeId> {
        let TargetSpec::Symbol {
            path,
            stem: Some(stem),
        } = &import.target
        else {
            return None;
        };
        let name = format!("{path}/{stem}.java");
        let mut candidates = self
            .files
            .values()
            .filter(|file| file.name == name && visible.contains(&file.project));
        let only = candidates.next()?;
        (candidates.next().is_none() && only.project != project).then(|| indexes[&only.id])
    }

    fn project_calls(&mut self, input: &Input, by_project: &BTreeMap<String, &Input>) {
        let java_count = input
            .files
            .iter()
            .filter(|file| file.lang == Lang::Java)
            .count();
        let mut reports = Vec::new();
        if java_count > 0 {
            reports.push((self.java_calls(input, java_count, by_project), true));
        }
        if input.files.iter().any(|file| file.lang != Lang::Java) {
            reports.push((self.other_calls(input), false));
        }
        let index = self
            .result
            .projects
            .iter()
            .position(|coverage| coverage.project == input.project.id)
            .unwrap();
        let mut statuses = Vec::new();
        for (analysis, repo_relative) in reports {
            statuses.push(analysis.status.clone());
            self.result.projects[index]
                .diagnostics
                .extend(analysis.diagnostics.iter().take(20).cloned());
            merge_calls(
                &mut self.result.calls,
                analysis,
                input,
                self.workspace,
                &self.files,
                repo_relative,
            );
        }
        let coverage = &mut self.result.projects[index];
        coverage.call_status = call_status(&statuses).into();
        if coverage.call_status == PARTIAL {
            coverage.status = PARTIAL.into();
        }
    }

    fn java_calls(
        &mut self,
        input: &Input,
        java_count: usize,
        by_project: &BTreeMap<String, &Input>,
    ) -> CallAnalysis {
        let closure = self.java_closure(input, by_project);
        if !self.java.admit(closure.len()) {
            self.diagnostics.push(format!(
                "{}: Java compiler work limit reached; call coverage unavailable for this project.",
                input.project.id
            ));
            return CallAnalysis {
                status: UNAVAILABLE.into(),
                files: java_count,
                diagnostics: vec!["Workspace compiler work limit reached.".into()],
                ..Default::default()
            };
        }
        crate::java_calls::analyze_java_workspace_calls(self.root, &input.java_config(), &closure)
    }

    /// Java sources of the project and its visible dependencies, without
    /// dependency test sources.
    fn java_closure(
        &mut self,
        input: &Input,
        by_project: &BTreeMap<String, &Input>,
    ) -> Vec<SourceFile> {
        let mut closure = Vec::new();
        let mut names = BTreeMap::<&str, usize>::new();
        for id in &visibility(self.workspace, &input.project.id, true) {
            let Some(dependency) = by_project.get(id) else {
                continue;
            };
            let own = id == &input.project.id;
            for (file, named) in dependency.files.iter().zip(&dependency.named) {
                if file.lang == Lang::Java && (own || !is_test(&file.rel_path)) {
                    closure.push(named.java_source(file));
                    *names.entry(named.name.as_str()).or_default() += 1;
                }
            }
        }
        if names.values().any(|count| *count > 1) {
            self.diagnostics.push(format!("{}: duplicate Java source identities in the declared closure; compiler scope not combined.",input.project.id));
            // Retain the independent project's own calls; no ambiguous
            // dependency target enters its compiler scope.
            return input.java_sources();
        }
        closure
    }

    fn other_calls(&mut self, input: &Input) -> CallAnalysis {
        let analysis = if input.diagnostics.is_empty() {
            let mut config = input.config.clone();
            for file in input.files.iter().filter(|file| file.lang == Lang::Java) {
                config.project.exclude.push(escape_glob(&file.rel_path));
            }
            crate::semantic_calls::analyze_calls(&input.root, &config)
        } else {
            CallAnalysis {status:UNAVAILABLE.into(),diagnostics:vec!["Project source boundary is incomplete; non-Java compiler analysis was not run.".into()],..Default::default()}
        };
        if self
            .workspace
            .dependencies
            .iter()
            .any(|dependency| dependency.from == input.project.id)
        {
            self.diagnostics.push(format!("{}: cross-project compiler calls are not implemented for its non-Java sources; isolated calls remain available.",input.project.id));
        }
        analysis
    }

    /// Semantic edges retain the real source call/inheritance sites.
    fn semantic_edges(&mut self, indexes: &FileIndexes) -> Result<SemanticCounts, ArchguardError> {
        let class_files: BTreeMap<_, _> = self
            .result
            .calls
            .classes
            .iter()
            .map(|class| (class.id.clone(), class.graph_node.clone()))
            .collect();
        let calls = self.call_edges(&class_files, indexes)?;
        let supertypes = self.supertype_edges(&class_files, indexes)?;
        let (type_uses, skipped_type_uses) = self.type_use_edges(indexes)?;
        Ok(SemanticCounts {
            calls,
            supertypes,
            type_uses,
            skipped_type_uses,
        })
    }

    fn call_edges(
        &mut self,
        class_files: &BTreeMap<String, String>,
        indexes: &FileIndexes,
    ) -> Result<usize, ArchguardError> {
        let mut projected = 0;
        let calls = self
            .result
            .calls
            .calls
            .iter()
            .filter(|call| call.resolution == PROJECT_RESOLUTION);
        for call in calls {
            let from = class_files
                .get(&call.from_class)
                .and_then(|file| indexes.get(file));
            let to = call
                .to_class
                .as_ref()
                .and_then(|id| class_files.get(id))
                .and_then(|file| indexes.get(file));
            let (Some(from), Some(to)) = (from, to) else {
                continue;
            };
            let reference = Ref {
                file: call.file.clone(),
                line: call.line,
                raw: format!("{} [compiler-resolved {} call]", call.code, call.dispatch),
            };
            projected += link(&mut self.builder, *from, *to, reference)?;
        }
        Ok(projected)
    }

    fn supertype_edges(
        &mut self,
        class_files: &BTreeMap<String, String>,
        indexes: &FileIndexes,
    ) -> Result<usize, ArchguardError> {
        let mut projected = 0;
        for class in &self.result.calls.classes {
            for target in &class.supertypes {
                let (Some(from), Some(to)) = (
                    indexes.get(&class.graph_node),
                    class_files.get(target).and_then(|file| indexes.get(file)),
                ) else {
                    continue;
                };
                let raw = format!(
                    "{} extends/implements {} [compiler-resolved type]",
                    class.name, target
                );
                let reference = Ref {
                    file: class.file.clone(),
                    line: class.line,
                    raw,
                };
                projected += link(&mut self.builder, *from, *to, reference)?;
            }
        }
        Ok(projected)
    }

    /// Returns projected and skipped project-resolved type uses.
    fn type_use_edges(&mut self, indexes: &FileIndexes) -> Result<(usize, usize), ArchguardError> {
        let Some(details) = &self.result.calls.details else {
            return Ok((0, 0));
        };
        let source_nodes: BTreeMap<_, _> = self
            .files
            .values()
            .map(|f| (f.path.as_str(), f.id.as_str()))
            .collect();
        let target_classes: BTreeMap<_, _> = self
            .result
            .calls
            .classes
            .iter()
            .map(|c| (c.id.as_str(), c))
            .collect();
        let (mut projected, mut skipped) = (0, 0);
        let uses = details
            .type_uses
            .iter()
            .filter(|u| u.resolution == PROJECT_RESOLUTION);
        for used in uses {
            let Some((from, to)) =
                type_use_endpoints(used, &target_classes, &source_nodes, indexes)
            else {
                skipped += 1;
                continue;
            };
            let reference = Ref {
                file: used.file.clone(),
                line: used.line,
                raw: format!("[semantic type:{}] {}", used.role, used.code),
            };
            projected += link(&mut self.builder, from, to, reference)?;
        }
        Ok((projected, skipped))
    }

    fn finish(self, semantic: SemanticCounts) -> WorkspaceSourceReport {
        let mut result = self.result;
        let cohesion = crate::class_cohesion(&result.calls);
        if let Some(details) = &mut result.calls.details {
            details.cohesion = cohesion;
        }
        result.graph = self.builder.build(self.root.display().to_string());
        result.cycles = source_cycles(&result.graph);
        result.files = self.files.into_values().collect();
        result.files.sort_by(|a, b| a.id.cmp(&b.id));
        sort_calls(&mut result.calls);
        result.calls.files = result.files.len();
        let mut diagnostics = self.diagnostics;
        result.calls.status = result.calls_status(&diagnostics).into();
        result.calls.diagnostics = diagnostics.iter().take(100).cloned().collect();
        result.status = result.source_status(self.workspace, &diagnostics).into();
        diagnostics.truncate(200);
        result.diagnostics = diagnostics;
        result.graph.semantic = Some(semantic.coverage(&result.calls));
        result
    }
}

fn call_status(statuses: &[String]) -> &'static str {
    if statuses.is_empty() {
        NOT_APPLICABLE
    } else if statuses.iter().all(|s| s == READY) {
        READY
    } else {
        PARTIAL
    }
}

/// Adds one graph edge; counts it when it links two different files.
fn link(
    builder: &mut GraphBuilder,
    from: NodeId,
    to: NodeId,
    reference: Ref,
) -> Result<usize, ArchguardError> {
    builder.edge(from, to, reference)?;
    Ok(usize::from(from != to))
}

/// Source and target file nodes of a type use whose target has one declaration file.
fn type_use_endpoints(
    used: &SemanticTypeUse,
    target_classes: &BTreeMap<&str, &CallClass>,
    source_nodes: &BTreeMap<&str, &str>,
    indexes: &FileIndexes,
) -> Option<(NodeId, NodeId)> {
    let to = target_classes.get(used.to_class.as_deref()?)?;
    if to
        .declaration_files
        .as_ref()
        .is_some_and(|files| files.len() != 1)
    {
        return None;
    }
    Some((
        *indexes.get(*source_nodes.get(used.file.as_str())?)?,
        *indexes.get(&to.graph_node)?,
    ))
}

fn source_cycles(graph: &Graph) -> Vec<Vec<String>> {
    let names: BTreeMap<_, _> = graph
        .nodes
        .iter()
        .map(|node| (node.id, &node.name))
        .collect();
    archguard_analysis::find_sccs(graph)
        .into_iter()
        .map(|scc| scc.node_ids.iter().map(|id| names[id].clone()).collect())
        .collect()
}

fn sort_calls(calls: &mut CallAnalysis) {
    calls.classes.sort_by(|a, b| a.id.cmp(&b.id));
    calls.classes.dedup_by(|a, b| a.id == b.id);
    calls.methods.sort_by(|a, b| a.id.cmp(&b.id));
    calls.methods.dedup_by(|a, b| a.id == b.id);
    calls.calls.sort_by(|a, b| {
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
    calls.calls.dedup();
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
impl NamedSource {
    /// Compiler input with the repository-relative path as its identity.
    fn java_source(&self, file: &SourceFile) -> SourceFile {
        SourceFile {
            path: file.path.clone(),
            rel_path: self.repo_path.clone(),
            lang: file.lang,
            project: file.project.clone(),
        }
    }
}
impl Input {
    fn java_sources(&self) -> Vec<SourceFile> {
        self.files
            .iter()
            .zip(&self.named)
            .filter(|(file, _)| file.lang == Lang::Java)
            .map(|(file, named)| named.java_source(file))
            .collect()
    }
    fn java_config(&self) -> Config {
        let mut config = self.config.clone();
        config.project.source_roots = vec![];
        config.project.languages = vec!["java".into()];
        config.semantic.java.classpath = config
            .semantic
            .java
            .classpath
            .iter()
            .map(|path| self.root.join(path).display().to_string())
            .collect();
        config
    }
    /// Rewrites a project-local reference to its repository path.
    fn repo_reference(&self, reference: &Ref) -> Ref {
        let source = self
            .named
            .iter()
            .find(|file| file.stripped == reference.file || file.relative == reference.file);
        let mut reference = reference.clone();
        if let Some(source) = source {
            reference.file = source.repo_path.clone();
        }
        reference
    }
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
            let bytes = match crate::project_files::read_repo_file(
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
            let name = crate::pipeline::file_node_name(&file, &stripped, &extraction.declarations);
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
    let CallAnalysis {
        details,
        analyzers,
        classes,
        methods,
        calls,
        ..
    } = analysis;
    let mut merge = CallMerge {
        project: &input.project.id,
        named: &input.named,
        workspace,
        files,
        repo_relative,
        sources: &methods,
        classes: BTreeMap::new(),
        methods: BTreeMap::new(),
    };
    merge.index(&classes);
    if let Some(details) = details {
        merge.details(target.details.get_or_insert_with(Default::default), details);
    }
    for (class, file) in merge.classes.values() {
        // The declaration's own project has its full declared visibility;
        // a consumer's smaller closure must not replace that metadata.
        let value = merge.qualified_class(class, file);
        upsert(
            &mut target.classes,
            value,
            file.project == *merge.project,
            |c| &c.id,
        );
    }
    merge.methods_into(&mut target.methods);
    target.calls.extend(
        calls
            .into_iter()
            .filter_map(|call| merge.qualified_call(call)),
    );
    for mut analyzer in analyzers {
        analyzer.project = Some(input.project.id.clone());
        target.analyzers.push(analyzer);
    }
}

/// Replaces an entry with the same ID only for its own project; adds new IDs.
fn upsert<T>(items: &mut Vec<T>, value: T, own: bool, id: fn(&T) -> &String) {
    if let Some(index) = items.iter().position(|existing| id(existing) == id(&value)) {
        if own {
            items[index] = value;
        }
    } else {
        items.push(value);
    }
}

/// Folds a class state reported by another compiler scope into the existing one.
fn merge_class_state(existing: &mut ClassState, state: ClassState) {
    if state.status != READY {
        existing.status = PARTIAL.into();
    }
    existing.fields.extend(state.fields);
    existing.accesses.extend(state.accesses);
    let mut fields = BTreeSet::new();
    existing
        .fields
        .retain(|f| fields.insert((f.id.clone(), f.file.clone())));
    let mut accesses = BTreeSet::new();
    existing
        .accesses
        .retain(|a| accesses.insert((a.method.clone(), a.field.clone(), a.file.clone(), a.line)));
}

/// Qualifies one compiler scope's evidence with the project of each located class.
struct CallMerge<'a> {
    project: &'a str,
    named: &'a [NamedSource],
    workspace: &'a WorkspaceReport,
    files: &'a BTreeMap<String, WorkspaceSourceFile>,
    repo_relative: bool,
    sources: &'a [CallMethod],
    classes: BTreeMap<String, (&'a CallClass, &'a WorkspaceSourceFile)>,
    methods: BTreeMap<String, String>,
}

impl<'a> CallMerge<'a> {
    /// Locates each class in exactly one file and qualifies its methods.
    fn index(&mut self, classes: &'a [CallClass]) {
        let mut duplicate = BTreeSet::new();
        for class in classes {
            if let Some(file) = self.locate(&class.file)
                && self
                    .classes
                    .insert(class.id.clone(), (class, file))
                    .is_some()
            {
                duplicate.insert(class.id.clone());
            }
        }
        for id in duplicate {
            self.classes.remove(&id);
        }
        self.methods = self
            .sources
            .iter()
            .filter_map(|method| {
                self.classes
                    .get(&method.class_id)
                    .map(|(_, file)| (method.id.clone(), qualify(&file.project, &method.id)))
            })
            .collect();
    }

    fn locate(&self, file: &str) -> Option<&'a WorkspaceSourceFile> {
        if self.repo_relative {
            return self.files.get(file);
        }
        let mut found = self
            .named
            .iter()
            .filter(|source| source.relative == file || source.stripped == file);
        match (found.next(), found.next()) {
            (Some(source), None) => self.files.get(&source.repo_path),
            _ => None,
        }
    }

    /// Repository path of a reported file, or the class file when it is not located.
    fn path_or(&self, file: &str, fallback: &WorkspaceSourceFile) -> String {
        self.locate(file).unwrap_or(fallback).path.clone()
    }

    fn visible(&self, file: &WorkspaceSourceFile) -> BTreeSet<String> {
        visibility(self.workspace, &file.project, is_test(&file.path))
    }

    /// File of a located class that belongs to the merged project.
    fn own_class(&self, id: &str) -> Option<&'a WorkspaceSourceFile> {
        self.classes
            .get(id)
            .map(|(_, file)| *file)
            .filter(|file| file.project == self.project)
    }

    /// Qualified ID of a located target class in a visible project.
    fn visible_class(&self, id: Option<&String>, visible: &BTreeSet<String>) -> Option<String> {
        id.and_then(|id| self.classes.get(id))
            .filter(|(_, file)| visible.contains(&file.project))
            .map(|(class, file)| qualify(&file.project, &class.id))
    }

    fn qualified_method(&self, id: Option<&String>) -> Option<String> {
        id.and_then(|id| self.methods.get(id)).cloned()
    }

    fn details(&self, combined: &mut SemanticDetails, details: SemanticDetails) {
        combined.enumeration_complete = Some(
            combined.enumeration_complete.unwrap_or(true)
                && details.enumeration_complete == Some(true),
        );
        self.method_metrics(combined, details.method_metrics);
        self.inheritance(combined, details.inheritance);
        let field_ids = self.field_ids(&details.class_states);
        let uses = details.type_uses.into_iter();
        combined
            .type_uses
            .extend(uses.filter_map(|used| self.qualified_type_use(used)));
        self.class_states(combined, details.class_states, &field_ids);
    }

    fn method_metrics(&self, combined: &mut SemanticDetails, metrics: Vec<SourceMethodMetrics>) {
        for mut metric in metrics {
            let Some(id) = self.methods.get(&metric.method_id) else {
                continue;
            };
            let own = self
                .sources
                .iter()
                .find(|m| m.id == metric.method_id)
                .is_some_and(|m| self.own_class(&m.class_id).is_some());
            if !own {
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
    }

    fn inheritance(&self, combined: &mut SemanticDetails, inheritance: Vec<ClassInheritance>) {
        for mut parent in inheritance {
            let Some(from) = self.own_class(&parent.class_id) else {
                continue;
            };
            parent.class_id = qualify(&from.project, &parent.class_id);
            if let Some((base, file)) = parent
                .base_class
                .as_ref()
                .and_then(|id| self.classes.get(id))
            {
                if self.visible(from).contains(&file.project) {
                    parent.base_class = Some(qualify(&file.project, &base.id));
                } else {
                    parent.base_class = None;
                    parent.depth = None;
                    parent.status = PARTIAL.into();
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
    }

    /// Qualified field IDs of all located classes, including dependency classes.
    fn field_ids(&self, states: &[ClassState]) -> BTreeMap<String, String> {
        states
            .iter()
            .filter_map(|state| {
                self.classes
                    .get(&state.class_id)
                    .map(|(_, file)| (state, file))
            })
            .flat_map(|(state, file)| {
                state
                    .fields
                    .iter()
                    .map(move |field| (field.id.clone(), qualify(&file.project, &field.id)))
            })
            .collect()
    }

    fn qualified_type_use(&self, mut used: SemanticTypeUse) -> Option<SemanticTypeUse> {
        let from = self.own_class(&used.from_class)?;
        let mapped = self.visible_class(used.to_class.as_ref(), &self.visible(from));
        used.from_class = qualify(&from.project, &used.from_class);
        used.from_method = self.qualified_method(used.from_method.as_ref());
        if mapped.is_some() {
            used.to_class = mapped;
        } else if used.resolution == PROJECT_RESOLUTION {
            used.to_class = None;
            used.resolution = UNRESOLVED.into();
        }
        used.file = self.path_or(&used.file, from);
        Some(used)
    }

    fn class_states(
        &self,
        combined: &mut SemanticDetails,
        states: Vec<ClassState>,
        field_ids: &BTreeMap<String, String>,
    ) {
        for state in states {
            let Some(file) = self.own_class(&state.class_id) else {
                continue;
            };
            let state = self.qualified_state(state, file, field_ids);
            match combined
                .class_states
                .iter_mut()
                .find(|s| s.class_id == state.class_id)
            {
                Some(existing) => merge_class_state(existing, state),
                None => combined.class_states.push(state),
            }
        }
    }

    /// Qualifies fields and keeps only accesses whose method and field are located.
    fn qualified_state(
        &self,
        mut state: ClassState,
        file: &WorkspaceSourceFile,
        field_ids: &BTreeMap<String, String>,
    ) -> ClassState {
        state.class_id = qualify(&file.project, &state.class_id);
        for field in &mut state.fields {
            field.id = qualify(&file.project, &field.id);
            field.file = self.path_or(&field.file, file);
        }
        state.accesses.retain_mut(|access| {
            let (Some(method), Some(field)) = (
                self.methods.get(&access.method),
                field_ids.get(&access.field),
            ) else {
                return false;
            };
            access.method = method.clone();
            access.field = field.clone();
            access.file = self.path_or(&access.file, file);
            true
        });
        state
    }

    fn qualified_class(&self, class: &CallClass, file: &WorkspaceSourceFile) -> CallClass {
        let mut value = class.clone();
        value.id = qualify(&file.project, &class.id);
        value.package = qualify(&file.project, &class.package);
        value.file = file.path.clone();
        value.graph_node = file.id.clone();
        value.supertypes = class
            .supertypes
            .iter()
            .filter_map(|id| self.classes.get(id))
            .filter(|(_, parent)| self.visible(file).contains(&parent.project))
            .map(|(parent, parent_file)| qualify(&parent_file.project, &parent.id))
            .collect();
        for site in value.supertype_sites.iter_mut().flatten() {
            self.qualify_site(site);
        }
        if let Some(declarations) = &mut value.declaration_files {
            *declarations = declarations
                .iter()
                .filter_map(|path| self.locate(path))
                .map(|file| file.path.clone())
                .collect();
        }
        value
    }

    fn qualify_site(&self, site: &mut CallSupertypeSite) {
        if let Some((parent, parent_file)) = self.classes.get(&site.target) {
            site.target = qualify(&parent_file.project, &parent.id);
        }
        if let Some(site_file) = self.locate(&site.file) {
            site.file = site_file.path.clone();
        }
    }

    fn methods_into(&self, target: &mut Vec<CallMethod>) {
        for method in self.sources {
            let Some((_, file)) = self.classes.get(&method.class_id) else {
                continue;
            };
            let mut value = method.clone();
            value.id = qualify(&file.project, &method.id);
            value.class_id = qualify(&file.project, &method.class_id);
            value.file = self.path_or(&method.file, file);
            upsert(target, value, file.project == self.project, |m| &m.id);
        }
    }

    fn qualified_call(&self, mut call: MethodCall) -> Option<MethodCall> {
        let from = self.own_class(&call.from_class)?;
        let visible = self.visible(from);
        call.from_class = qualify(&from.project, &call.from_class);
        call.from_method = self.qualified_method(call.from_method.as_ref());
        if let Some(to_class) = self.visible_class(call.to_class.as_ref(), &visible) {
            call.to_class = Some(to_class);
            call.to_method = self.qualified_method(call.to_method.as_ref());
        } else if call.resolution == PROJECT_RESOLUTION {
            call.to_class = None;
            call.to_method = None;
            call.resolution = UNRESOLVED.into();
        }
        call.file = self.path_or(&call.file, from);
        Some(call)
    }
}
