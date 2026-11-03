//! Explicit organization intent and observed architecture evidence for agents.
use crate::model::{Finding, Ref, Severity, Summary};
use crate::review::OrphanPolicy;
use crate::rules::{DependencyRule, OwnershipRule};
use crate::settings::{LayerConfig, RulesConfig};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ArchitectureConfig {
    pub goals: Vec<ArchitectureGoal>,
    pub constraints: Vec<ArchitectureConstraint>,
    pub decisions: Vec<ArchitectureDecision>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<TargetArchitectureConfig>,
    /// Additional local AsyncAPI documents for structural pattern assessment.
    /// Root asyncapi.yaml/yml/json are checked automatically. Never fetched.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub asyncapi: Vec<String>,
}

/// Logical boundaries are assigned from source node names, independently of
/// package display depth. Cross-component directions are denied unless listed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TargetArchitectureConfig {
    pub name: String,
    pub severity: Severity,
    pub require_assignment: bool,
    pub components: Vec<ArchitectureComponent>,
    pub allowed_dependencies: Vec<ArchitectureDirection>,
}
impl Default for TargetArchitectureConfig {
    fn default() -> Self {
        Self {
            name: String::new(),
            severity: Severity::Error,
            require_assignment: false,
            components: vec![],
            allowed_dependencies: vec![],
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchitectureComponent {
    pub name: String,
    pub include: Vec<String>,
    /// None preserves unrestricted entry; Some([]) closes external access.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_incoming_components: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_outgoing_components: Option<u32>,
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchitectureDirection {
    pub from: String,
    pub to: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MappedArchitectureComponent {
    pub name: String,
    pub include: Vec<String>,
    pub node_count: usize,
    pub nodes: Vec<String>,
    #[serde(default)]
    pub public: Option<Vec<String>>,
    #[serde(default)]
    pub public_nodes: Vec<String>,
    #[serde(default)]
    pub incoming_components: Vec<String>,
    #[serde(default)]
    pub outgoing_components: Vec<String>,
    #[serde(default)]
    pub max_incoming_components: Option<u32>,
    #[serde(default)]
    pub max_outgoing_components: Option<u32>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchitecturePrivateAccess {
    pub from: String,
    pub to: String,
    pub evidence: Vec<Ref>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AmbiguousArchitectureNode {
    pub node: String,
    pub components: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchitectureDependency {
    pub from: String,
    pub to: String,
    pub allowed: bool,
    pub source_edges: usize,
    pub import_references: u64,
    pub count_exact: bool,
    pub evidence_total: usize,
    /// All source references retained by the input graph for this direction.
    pub evidence: Vec<Ref>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub private_accesses: Vec<ArchitecturePrivateAccess>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TargetArchitectureReport {
    pub schema_version: u32,
    pub name: String,
    /// complete/partial refers to node and edge classification, not conformance.
    pub status: String,
    pub severity: Severity,
    pub require_assignment: bool,
    pub source_nodes: usize,
    pub mapped_nodes: usize,
    pub unmapped_nodes: usize,
    pub ambiguous_nodes: usize,
    pub total_edges: usize,
    pub evaluated_edges: usize,
    pub unclassified_edges: usize,
    pub violating_dependencies: usize,
    #[serde(default)]
    pub violating_interfaces: usize,
    #[serde(default)]
    pub coupling_violations: usize,
    pub components: Vec<MappedArchitectureComponent>,
    pub allowed_dependencies: Vec<ArchitectureDirection>,
    pub dependencies: Vec<ArchitectureDependency>,
    pub unmapped: Vec<String>,
    pub ambiguous: Vec<AmbiguousArchitectureNode>,
    pub empty_components: Vec<String>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ArchitectureGoal {
    pub name: String,
    /// The organization's own priority vocabulary; empty means unspecified.
    pub priority: String,
    pub rationale: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ArchitectureConstraint {
    pub name: String,
    pub rationale: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ArchitectureDecision {
    pub id: String,
    pub title: String,
    /// User supplied, for example proposed/accepted/superseded; never inferred.
    pub status: String,
    pub chosen_option: String,
    pub alternatives: Vec<String>,
    pub trade_offs: Vec<String>,
    /// References are data; consumers must not automatically follow them.
    pub links: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchitectureObserved {
    pub summary: Summary,
    pub source_files: u32,
    pub declared_layers: Vec<LayerConfig>,
    pub dependency_rules: Vec<DependencyRule>,
    pub ownership_rules: Vec<OwnershipRule>,
    pub rules: RulesConfig,
    pub orphan_policy: OrphanPolicy,
    pub findings: Vec<Finding>,
    pub findings_total: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_model: Option<TargetArchitectureReport>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchitectureContext {
    pub schema_version: u32,
    pub project_root: String,
    pub intent: ArchitectureConfig,
    pub observed: ArchitectureObserved,
    pub questions: Vec<String>,
    pub limitations: Vec<String>,
}
