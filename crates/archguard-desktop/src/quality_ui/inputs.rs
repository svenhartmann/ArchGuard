//! The tab's data, from the same engine calls as the CLI's report payload
//! (`review_cmd::html_page_with_services`, `insights`).

use std::collections::BTreeMap;
use std::path::Path;

use archguard_engine::core::{Config, Graph, NodeKind, ReviewSources};
use archguard_engine::{
    CodeMetricsReport, CohesionReport, DuplicationReport, KnowledgeReport, MeasurementReport,
    MutationReport, QualityHistoryReport, TestEvidenceReport,
};

use super::rows::{NodeInfo, Source};

pub struct Inputs {
    pub source: Source,
    pub code: CodeMetricsReport,
    /// `None` when Git could not be read (the CLI fails the page then).
    pub knowledge: Option<KnowledgeReport>,
    pub measurements: MeasurementReport,
    pub cohesion: CohesionReport,
    pub history: QualityHistoryReport,
    pub tests: TestEvidenceReport,
    pub mutations: MutationReport,
    pub duplicates: DuplicationReport,
}

/// Nodes, file edges and scanned paths of the original graph.
pub fn source(graph: &Graph, sources: &ReviewSources) -> Source {
    let index: BTreeMap<_, usize> = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id, i))
        .collect();
    Source {
        nodes: graph
            .nodes
            .iter()
            .map(|n| NodeInfo {
                name: n.name.clone(),
                parent: n.parent.and_then(|p| index.get(&p).copied()),
                module: n.kind == NodeKind::Module,
            })
            .collect(),
        edges: graph
            .edges
            .iter()
            .filter_map(|e| Some((*index.get(&e.from)?, *index.get(&e.to)?)))
            .collect(),
        files: sources
            .files
            .iter()
            .filter_map(|f| Some((*index.get(&f.node_id)?, f.path.clone()?)))
            .collect(),
    }
}

impl Inputs {
    /// Runs the call analysis and reads Git and imported evidence; slow
    /// (seconds), so the window calls it on a worker thread.
    pub fn compute(root: &Path, graph: &Graph, config: &Config, sources: &ReviewSources) -> Self {
        let calls = archguard_engine::analyze_calls(root, config);
        let options = archguard_engine::CoChangeOptions {
            since: config.review.git_since.clone(),
            ..Default::default()
        };
        let co_change = archguard_engine::co_change(root, config, graph, &options).ok();
        Inputs {
            source: source(graph, sources),
            code: archguard_engine::code_metrics(&calls),
            knowledge: archguard_engine::knowledge_metrics(root, config, graph).ok(),
            measurements: archguard_engine::read_measurement_runs(root),
            cohesion: archguard_engine::analyze_cohesion(graph, co_change.as_ref()),
            history: archguard_engine::read_quality_history(root),
            tests: archguard_engine::read_test_evidence(root, config, graph),
            mutations: archguard_engine::read_mutations(root, config, graph),
            duplicates: archguard_engine::read_duplicates(root, config, graph),
        }
    }
}
