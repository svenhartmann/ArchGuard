//! L2: the Elasticsearch map of a project (ADR 0023, 0025): sources (Terraform, search templates,
//! code), their merge, findings and the graph. One format is one module in `sources/` and one row
//! in `sources::SOURCES`.
mod analysis;
mod context;
mod findings;
mod graph;
mod merge;
mod sources;

pub use analysis::{ChainRef, SearchAnalysisPart, search_analysis_parts, search_builtin};
pub use context::Reader;
use context::SearchContext;
pub use findings::{is_detail_gap, is_elasticsearch_gap, search_artifact_covers};
pub use graph::{SearchEdge, SearchGraph, SearchNode, search_graph, short_scope};
pub use merge::merge_set;

use archguard_core::ServiceTopology;
use archguard_flow_common::support::Discovery;
use std::path::Path;

/// Runs every source over the report's services and merges the result into `report`.
/// Called after the deployments and tables are known; no earlier step reads the Elasticsearch map.
pub fn analyze(root: &Path, discovery: &Discovery, read: &Reader, report: &mut ServiceTopology) {
    let ctx = SearchContext {
        root,
        discovery,
        topology: report,
        read,
    };
    let outputs = sources::SOURCES.iter().map(|source| source(&ctx)).collect();
    merge::apply(report, outputs);
}
