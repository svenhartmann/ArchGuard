//! A comparison of two immutable source snapshots under one analysis policy.
use crate::model::{Finding, Graph, Ref, Summary};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchitectureEdgeChange {
    pub from: String,
    pub to: String,
    pub before: Vec<Ref>,
    pub after: Vec<Ref>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchitectureDiffDetails {
    pub module_depth: u32,
    pub base_graph: Graph,
    pub target_graph: Graph,
    pub base_summary: Summary,
    pub target_summary: Summary,
    pub added_nodes: Vec<String>,
    pub removed_nodes: Vec<String>,
    pub added_edges: Vec<ArchitectureEdgeChange>,
    pub removed_edges: Vec<ArchitectureEdgeChange>,
    pub changed_edges: Vec<ArchitectureEdgeChange>,
    pub new_findings: Vec<Finding>,
    pub resolved_findings: Vec<Finding>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchArchitectureDiff {
    pub definition: String,
    pub target: String,
    pub base_ref: String,
    pub base_commit: String,
    pub target_ref: String,
    pub target_commit: String,
    pub policy_source: String,
    #[serde(flatten)]
    pub details: ArchitectureDiffDetails,
    pub source_diff: String,
    pub limitations: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagedArchitectureDiff {
    pub definition: String,
    pub target: String,
    pub base_ref: String,
    pub base_commit: String,
    pub index_fingerprint: String,
    pub policy_source: String,
    pub module_depth: u32,
    pub base_graph: Graph,
    pub staged_graph: Graph,
    pub base_summary: Summary,
    pub staged_summary: Summary,
    pub added_nodes: Vec<String>,
    pub removed_nodes: Vec<String>,
    pub added_edges: Vec<ArchitectureEdgeChange>,
    pub removed_edges: Vec<ArchitectureEdgeChange>,
    pub changed_edges: Vec<ArchitectureEdgeChange>,
    pub new_findings: Vec<Finding>,
    pub resolved_findings: Vec<Finding>,
    pub source_diff: String,
    pub limitations: Vec<String>,
}
