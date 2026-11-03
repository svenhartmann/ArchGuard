//! Evidence and results for explainable architecture review priorities.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::model::{NodeId, NodeKind};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OrphanPolicy {
    #[default]
    Off,
    Info,
    Warning,
    Error,
}

/// Optional review settings. Candidates are not proof of dead code.
///
/// The KPI gates (R30) are `None` = off by default: an unconfigured gate
/// never fails a run. No `Eq` — the gate bounds are finite floats.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReviewConfig {
    pub git_since: String,
    pub entry_points: Vec<String>,
    /// Opt-in file-level gate. A candidate is not proof of dead code.
    pub orphans: OrphanPolicy,
    /// Weights for the composite health index (part name → weight);
    /// unlisted parts weigh `1.0` (see [`crate::review_kpi::HealthWeights`]).
    /// Negative or non-finite weights are rejected at config load.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_weights: Option<BTreeMap<String, f64>>,
    /// Gate: maximum number of SDP violations. `None` = gate off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_sdp_violations: Option<u32>,
    /// Gate: maximum tangledness quote in `0.0..=1.0`. `None` = off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tangledness: Option<f64>,
    /// Gate: maximum propagation cost in `0.0..=1.0`. `None` = off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_propagation_cost: Option<f64>,
    /// Gate: `accept-risk` decisions must carry an ADR link.
    pub adr_required_on_accept_risk: bool,
}

impl Default for ReviewConfig {
    fn default() -> Self {
        Self {
            git_since: "180.days".into(),
            orphans: OrphanPolicy::Off,
            entry_points: [
                "**/main.*",
                "**/index.*",
                "**/lib.rs",
                "**/__main__.py",
                "**/Main.java",
                "**/Program.cs",
                "**/cli/**",
                "**/bin/**",
                "**/test/**",
                "**/tests/**",
                "**/__tests__/**",
                "**/test_*",
                "**/*_test.*",
                "**/*.test.*",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
            health_weights: None,
            max_sdp_violations: None,
            max_tangledness: None,
            max_propagation_cost: None,
            adr_required_on_accept_risk: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitStatus {
    Available,
    #[default]
    Unavailable,
}

/// History coverage. A truncated or shallow history supplies lower bounds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitEvidence {
    pub status: GitStatus,
    pub since: String,
    pub shallow: bool,
    pub truncated: bool,
    pub scanned_commits: u32,
    pub message: Option<String>,
}

impl Default for GitEvidence {
    fn default() -> Self {
        Self {
            status: GitStatus::Unavailable,
            since: "180.days".into(),
            shallow: false,
            truncated: false,
            scanned_commits: 0,
            message: Some("No Git history supplied".into()),
        }
    }
}

/// Measurements for one real source file. `None` means unknown, never zero.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceEvidence {
    pub node_id: NodeId,
    pub path: Option<String>,
    pub lines: Option<u64>,
    /// Number of commits touching this file, not lines changed.
    pub changes: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewSources {
    pub files: Vec<SourceEvidence>,
    pub git: GitEvidence,
    pub mapped_files: u32,
    pub total_files: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewPriority {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrphanCandidate {
    Isolated,
    Unreferenced,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewScore {
    pub cycle: u32,
    pub coupling: u32,
    pub impact: u32,
    pub churn: u32,
}

/// All graph metrics refer to the supplied analysis scope, not hidden runtime calls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleReview {
    pub node_id: NodeId,
    pub source_id: Option<NodeId>,
    pub name: String,
    pub kind: NodeKind,
    pub rank: u32,
    pub score: u32,
    pub priority: ReviewPriority,
    pub fan_in: u32,
    pub fan_out: u32,
    pub instability: f64,
    /// Zero for acyclic nodes; one for a self-cycle.
    pub scc_size: u32,
    /// Distinct transitively reachable nodes, excluding self.
    pub dependencies: u32,
    /// Distinct nodes transitively depending on this node, excluding self.
    pub dependents: u32,
    pub file_count: u32,
    pub mapped_files: u32,
    pub lines: Option<u64>,
    /// Sum of per-file commit touches, not distinct package commits.
    pub changes: Option<u64>,
    pub entry_point: bool,
    pub orphan: Option<OrphanCandidate>,
    pub components: ReviewScore,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReviewSummary {
    pub scope_nodes: u32,
    pub scope_edges: u32,
    pub ccd: u64,
    pub acd: f64,
    pub nccd: f64,
    pub cyclic_nodes: u32,
    pub largest_scc: u32,
    pub high_priority: u32,
    pub orphan_candidates: u32,
    pub source_files: u32,
    pub mapped_files: u32,
    pub lines: Option<u64>,
    pub changes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReviewReport {
    pub schema_version: u32,
    pub score_version: String,
    pub summary: ReviewSummary,
    pub nodes: Vec<ModuleReview>,
    pub git: GitEvidence,
    pub entry_points: Vec<String>,
    pub entry_point_ids: Vec<NodeId>,
    pub limitations: Vec<String>,
}
