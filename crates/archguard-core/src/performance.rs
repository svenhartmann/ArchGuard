//! Structural performance indicators and compiler-backed abstraction evidence.
use crate::model::{NodeId, Ref};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PerformanceConfig {
    /// Exact external module prefixes to flag for review, not measured costs.
    pub heavy_imports: Vec<String>,
    pub min_barrel_exports: u32,
    pub min_barrel_ratio: f64,
}
impl Default for PerformanceConfig {
    fn default() -> Self {
        Self {
            heavy_imports: [
                "pandas",
                "numpy",
                "tensorflow",
                "torch",
                "scipy",
                "@tensorflow/tfjs",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            min_barrel_exports: 10,
            min_barrel_ratio: 0.8,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceCostFinding {
    /// `heavy_import` or `barrel`; a source-backed review hint, not a runtime measurement.
    pub kind: String,
    pub node_id: NodeId,
    pub name: String,
    pub file: String,
    pub line: u32,
    pub raw: String,
    pub library: Option<String>,
    pub reexports: u32,
    pub code_lines: u32,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntryClosure {
    pub node_id: NodeId,
    pub name: String,
    /// Distinct source files reachable from the entry (entry file included).
    pub reachable_files: u32,
    pub lines: Option<u64>,
    /// Number of SCC components along the longest reachable condensation path.
    pub longest_component_path: u32,
    pub cyclic_components: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CyclicBuildGroup {
    pub nodes: Vec<String>,
    pub source_files: u32,
    pub lines: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AbstractionMetric {
    /// `node` or `scc`; SCC metrics aggregate types and boundary edges.
    pub scope: String,
    pub name: String,
    pub nodes: Vec<String>,
    pub types: u32,
    pub abstract_types: u32,
    pub fan_in: u32,
    pub fan_out: u32,
    pub instability: f64,
    pub abstractness: Option<f64>,
    /// Main Sequence Distance, `abs(A + I - 1)`.
    pub distance: Option<f64>,
    /// `complete` or `unavailable`; partial compiler data never yields A/D.
    pub coverage: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContractCandidate {
    pub from_class: String,
    pub to_class: String,
    pub from_node: String,
    pub to_node: String,
    pub interfaces: Vec<String>,
    pub call_sites: u32,
    pub evidence: Vec<Ref>,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PerformanceReport {
    pub schema_version: u32,
    pub source_findings: Vec<SourceCostFinding>,
    pub entries: Vec<EntryClosure>,
    pub cyclic_groups: Vec<CyclicBuildGroup>,
    pub abstractions: Vec<AbstractionMetric>,
    pub contract_candidates: Vec<ContractCandidate>,
    pub contract_candidates_truncated: bool,
    pub diagnostics: Vec<String>,
    pub limitations: Vec<String>,
}
