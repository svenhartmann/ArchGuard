//! Graph, finding, report and error types every other module of this crate builds on.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::calls::SemanticDependencyCoverage;
use crate::review_kpi::ReviewKpis;

/// Internal, interning-assigned integer id of a [`Node`].
///
/// Ids are an implementation detail: at the outside (CLI, MCP, reports)
/// nodes are always addressed by their normalized module path name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(pub u32);

/// What kind of tree node this is: a directory/package/namespace module or
/// a source file (also the package pseudo-file `…/*` for symbolic imports).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    Module,
    File,
}

/// A node of the containment tree.
///
/// `name` is a normalized module path (`a/b/c.ts`); module nodes are
/// directories/packages/namespaces, file nodes are source files or the
/// package pseudo-file named `…/*`. `parent` builds the containment tree
/// (`None` at tree roots); `lang` is set on file nodes only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub name: String,
    pub kind: NodeKind,
    pub parent: Option<NodeId>,
    /// Omitted in canonical JSON when `None` (Kap. 5 example).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
}

/// One concrete dependency site backing an [`Edge`]: an import or a
/// compiler-resolved call/inheritance reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ref {
    pub file: String,
    pub line: u32,
    pub raw: String,
}

/// A dependency edge, always between `file` nodes; `refs` are the
/// deduplicated, merged concrete source references proving it.
///
/// `count` is the *total* number of deduplicated source references the
/// edge represents, kept separate from `refs` because views cap `refs`
/// to a small evidence sample ([`view::aggregate`]) while the total
/// must survive every aggregation. The builder and every aggregation
/// set it exactly; `None` marks a legacy graph (or hand-built edge)
/// written before this field existed, whose truncated total cannot be
/// recovered — [`Edge::weight`] is then only a lower bound and must
/// never be presented as an exact count.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub from: NodeId,
    pub to: NodeId,
    pub refs: Vec<Ref>,
    /// Total dependency-reference count; omitted in JSON when `None` and
    /// absent in legacy JSON (deserializes back to `None`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u64>,
}

impl Edge {
    /// Number of dependency references this edge represents.
    ///
    /// Exact whenever `count` is set (every graph produced by
    /// [`GraphBuilder::build`](graph::GraphBuilder::build) or
    /// [`view::aggregate`], and every simulation merge, which all
    /// deduplicate before counting). For edges without `count` this
    /// falls back to the sample length and is a **lower bound** only:
    /// the true total was truncated and cannot be recovered, so it is
    /// never invented here.
    pub fn weight(&self) -> u64 {
        self.count.unwrap_or(self.refs.len() as u64)
    }
}

/// The canonical analysis graph (Kap. 5): containment tree plus edges.
///
/// `Eq` is deliberately not derived: the optional [`ReviewKpis`] field
/// carries `f64` measurements, for which a reflexive equality would be
/// a lie (`NaN != NaN`). `PartialEq` compares graphs exactly.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Graph {
    pub schema_version: u32,
    pub root: String,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    /// Original compiler coverage; absent in legacy/import-only graphs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic: Option<SemanticDependencyCoverage>,
    /// Review KPIs wired in by the engine pipeline (R23); absent in
    /// graphs produced before the review feature — absence means "not
    /// measured", never "clean".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_kpis: Option<ReviewKpis>,
}

impl Graph {
    /// Name of the node with this id, `None` if the id is unknown.
    pub fn node_name(&self, id: NodeId) -> Option<&str> {
        self.nodes
            .iter()
            .find(|n| n.id == id)
            .map(|n| n.name.as_str())
    }

    /// All edges starting at `id`, in graph (stable) order.
    pub fn out_edges(&self, id: NodeId) -> impl Iterator<Item = &Edge> + '_ {
        self.edges.iter().filter(move |e| e.from == id)
    }

    /// Direct children of `id` in the containment tree, sorted by id.
    /// Empty for an unknown id or a childless node.
    pub fn children_of(&self, id: NodeId) -> Vec<NodeId> {
        let mut children: Vec<NodeId> = self
            .nodes
            .iter()
            .filter(|n| n.parent == Some(id))
            .map(|n| n.id)
            .collect();
        children.sort();
        children
    }

    /// All ancestors of `id` in the containment tree, nearest first.
    /// Empty for an unknown id or a tree root. Malformed parent cycles
    /// terminate at the first repeated node instead of looping.
    pub fn ancestors_of(&self, id: NodeId) -> Vec<NodeId> {
        let mut ancestors = Vec::new();
        let mut seen = BTreeSet::new();
        let mut current = self
            .nodes
            .iter()
            .find(|n| n.id == id)
            .and_then(|n| n.parent);
        while let Some(parent) = current {
            if !seen.insert(parent) {
                break;
            }
            ancestors.push(parent);
            current = self
                .nodes
                .iter()
                .find(|n| n.id == parent)
                .and_then(|n| n.parent);
        }
        ancestors
    }
}

// ------------------------------------------------- analysis vocabulary (Kap. 5)

/// How severe a [`Finding`] is.
///
/// The derived `Ord` order is the report sort order of Kap. 5: findings are
/// emitted sorted by severity first, hence `Error < Warning < Info`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Info,
}

/// The kind of analysis that produced a [`Finding`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    Cycle,
    LayerViolation,
    MetricThreshold,
    /// A module inside a cycle cannot be ranked into a layer (Kap. 11).
    LayerUnranked,
    DependencyRule,
    OwnershipRule,
    Orphan,
    TargetArchitecture,
}

/// A recommendation to break a cycle: cutting the dependency `from` → `to`
/// separates the sampled cycles. `score` ranks the candidates (higher is
/// better cut), `reason` is the human-facing justification (Kap. 5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SuggestedCut {
    pub from: String,
    pub to: String,
    pub score: f32,
    pub reason: String,
}

/// One analysis result (Kap. 5): a cycle, a layer violation or a violated
/// metric threshold. Involved nodes are always named by their
/// source-root-relative path, never by internal [`NodeId`]s.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub kind: FindingKind,
    /// Stable finding id, e.g. `scc-3`.
    pub id: String,
    pub severity: Severity,
    /// Optional actionable explanation; absent in legacy reports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// Source-backed evidence, omitted when no source location is known.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<Ref>,
    /// Involved node names, stably sorted.
    pub nodes: Vec<String>,
    pub edge_count: u32,
    /// Number of concrete cycles enumerated for this finding (capped, Kap. 7).
    pub cycles_sampled: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sampling: Option<CycleSampling>,
    pub suggested_cuts: Vec<SuggestedCut>,
}

/// Sampling is root-limited evidence, never proof that an SCC is acyclic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CycleSampling {
    pub searched_from: Option<String>,
    pub explored_edges: usize,
    pub edge_budget: usize,
    pub budget_exhausted: bool,
    pub length_limited: bool,
    pub cycle_limit_reached: bool,
    pub root_search_complete: bool,
    /// False because cycles not containing `searched_from` are not searched.
    pub complete: bool,
}

/// Dependency metrics of a single module, keyed by module name in
/// [`Report::metrics`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleMetrics {
    pub fan_in: u32,
    pub fan_out: u32,
    /// `fan_out / (fan_in + fan_out)`; `0.0` for a module without edges.
    pub instability: f32,
    pub in_cycle: bool,
}

/// Outcome of the quality gates for a complete run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Pass,
    Fail,
}

/// Run-wide counts shown on top of a [`Report`] (Kap. 5).
///
/// The Lakos fields (`ccd`, `acd`, `nccd`, Kap. 7) carry
/// `#[serde(default)]` so reports written before they existed still load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub nodes: u32,
    pub edges: u32,
    pub sccs: u32,
    pub largest_scc: u32,
    pub layer_violations: u32,
    pub status: Status,
    /// Lakos CCD: the sum over all nodes of the size of each node's
    /// *Depends-Upon* set (the node itself included).
    #[serde(default)]
    pub ccd: u64,
    /// Lakos ACD: `ccd / node_count` (`0.0` for the empty graph).
    #[serde(default)]
    pub acd: f64,
    /// Lakos NCCD: `ccd` normalized by the CCD of a balanced tree of the
    /// same node count (see the analysis crate; `0.0` when the
    /// normalizer is not positive).
    #[serde(default)]
    pub nccd: f64,
}

/// Where the value of a configuration section came from (Kap. 11:
/// `deklariert ⇐ abgeleitet ⇐ Default`), shown per section in the
/// [`Report`]'s `config` block (Kap. 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConfigSource {
    /// Written by the user in `archguard.toml`.
    Declared,
    /// Derived from the analysed project.
    Derived,
    /// Neither declared nor derivable — the documented default.
    Default,
    /// Section with values of more than one origin.
    Mixed,
}

/// Result of a complete run (Kap. 5): the [`Summary`], every [`Finding`] and
/// the per-module [`ModuleMetrics`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub schema_version: u32,
    pub summary: Summary,
    /// Section name → provenance (Kap. 5); `BTreeMap` keeps the report
    /// deterministic, an empty block serializes as `{}`.
    #[serde(default)]
    pub config: BTreeMap<String, ConfigSource>,
    /// Emitted in stable order: by severity, then by id (Kap. 5).
    pub findings: Vec<Finding>,
    /// Module name → metrics; `BTreeMap` keeps the report deterministic.
    pub metrics: BTreeMap<String, ModuleMetrics>,
    /// Coverage inherited from the graph used to compute this report.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic: Option<SemanticDependencyCoverage>,
    /// Review KPIs inherited from the graph used to compute this report
    /// (R23/R26); absent for reports built without the KPI pass, so old
    /// payloads stay byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_kpis: Option<ReviewKpis>,
}

// ----------------------------------------------------------------- errors

/// The single error type shared by every layer of archguard: fallible APIs
/// return `Result<_, ArchguardError>`.
#[derive(Debug, thiserror::Error)]
pub enum ArchguardError {
    /// Filesystem or process I/O failed.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// A `archguard.toml` (or baseline file) could not be read or parsed.
    #[error("config error: {0}")]
    Config(String),
    /// An analysis could not be completed.
    #[error("analysis error: {0}")]
    Analysis(String),
}
