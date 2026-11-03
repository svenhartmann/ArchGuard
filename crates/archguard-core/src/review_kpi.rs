//! Value types for the review KPIs of the architecture review 2.0
//!.
//!
//! These are pure data: the producing analyses live in
//! `archguard-analysis` (`pc`, `sdp`, `zones`, `tangles`, `layer_drift`,
//! `health`, `leaky`) and the engine glue in `archguard-engine`.
//! Every type is `#[serde(default)]` so a payload written before a field
//! existed still loads, and unknown measurements serialize as `null`
//! (`Option`) — never as an invented zero.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Propagation cost of one cyclic SCC: the SCC itself plus every node
/// reachable from it in the condensation DAG, divided by the node total.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PropagationCost {
    /// Stable finding id of the cycle, e.g. `scc-3`.
    pub scc_id: String,
    /// Number of nodes inside the SCC.
    pub size: usize,
    /// Nodes affected: SCC members plus transitive successors.
    pub affected: usize,
    /// Total number of nodes the ratio is taken over.
    pub total: usize,
    /// `affected / total` (`0.0` for an empty graph).
    pub pc: f64,
}

/// Share of nodes that live inside a cyclic SCC of size > 1.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tangledness {
    /// Nodes inside any SCC with more than one member.
    pub tangled_nodes: usize,
    /// All nodes the quote is taken over.
    pub total: usize,
    /// `tangled_nodes / total` (`0.0` for an empty graph).
    pub quote: f64,
}

/// Coupling concentration: inequality of the fan-in distribution.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HubGini {
    /// Gini coefficient of the fan-in distribution (`0.0` equal,
    /// approaching `1.0` single hub). `0.0` for a graph without nodes.
    pub gini: f64,
    /// Share of all edges that end in the five biggest hubs.
    pub top_share: f64,
}

/// One Stable Dependencies Principle violation: a module edge pointing
/// from a more stable to a less stable module (Martin's SDP).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SdpViolation {
    /// Source module of the offending edge.
    pub from: String,
    /// Target module of the offending edge.
    pub to: String,
    /// Instability of `from`; `None` when not measurable.
    pub i_from: Option<f64>,
    /// Instability of `to`; `None` when not measurable.
    pub i_to: Option<f64>,
    /// Human-readable import evidence (`file:line`).
    pub evidence: Vec<String>,
}

/// Distribution of module boundaries over the Martin A-I zones.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ZoneStats {
    /// Boundaries with low abstraction and low instability (far from the
    /// main sequence towards `A+I < 1`).
    pub zone_of_pain: usize,
    /// Boundaries with high abstraction and high instability.
    pub zone_of_uselessness: usize,
    /// Boundaries close to the main sequence (`D' = |A + I − 1|` small).
    pub main_sequence: usize,
    /// Boundaries without measurable A/I (never guessed into a zone).
    pub unmeasured: usize,
}

/// Divergence between declared and derived layers.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayerDrift {
    /// Share of modules with the same assignment in both models;
    /// `None` when no comparison is possible (e.g. nothing declared).
    pub agreement: Option<f64>,
    /// Modules only the declared model assigns.
    pub declared_only: Vec<String>,
    /// Modules only the derived model assigns.
    pub derived_only: Vec<String>,
    /// Modules assigned to different layers by the two models.
    pub violated: Vec<String>,
}

/// Density of technical-debt markers (TODO/FIXME/HACK/XXX).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DebtDensity {
    /// Total number of markers found.
    pub markers: usize,
    /// Total source lines of the scanned files.
    pub loc: usize,
    /// `markers / (loc / 1000)`; `None` when `loc` is unknown or zero.
    pub per_kloc: Option<f64>,
    /// Marker count per kind (lowercase, e.g. `todo`, `fixme`).
    pub by_kind: BTreeMap<String, usize>,
}

/// Size of the publicly reachable symbol surface of a module.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ApiSurface {
    /// Exported symbols across the module.
    pub exported_symbols: usize,
    /// Files the symbols are spread over.
    pub files: usize,
    /// `exported_symbols / files`; `None` when `files` is zero.
    pub per_file: Option<f64>,
    /// Hottest files by exported symbol count, deterministically sorted.
    pub top: Vec<(String, usize)>,
}

/// Risk-weighted coverage gap of one file: hotspot risk times the
/// uncovered share.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RiskCoverageGap {
    /// Source-root-relative path of the file.
    pub path: String,
    /// Hotspot score (higher is riskier).
    pub hotspot: f64,
    /// Line coverage in `0.0..=1.0`; `None` when coverage is unavailable.
    pub coverage: Option<f64>,
    /// `hotspot * (1 − coverage)`; `None` when coverage is unknown —
    /// never computed against an invented zero.
    pub gap: Option<f64>,
}

/// Ampel level of the composite health index.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum HealthLevel {
    /// Score ≥ 80.
    Green,
    /// Score ≥ 50.
    Yellow,
    /// Score < 50.
    #[default]
    Red,
}

/// Composite architecture health index (0–100) with its parts.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HealthIndex {
    /// Weighted score in `0.0..=100.0`; `None` when no part is measurable.
    pub score: Option<f64>,
    /// Ampel derived from `score`; `None` together with `score`.
    pub level: Option<HealthLevel>,
    /// Delta against the previous snapshot; `None` without a previous one.
    pub trend: Option<f64>,
    /// Normalized contribution per part KPI (raw part value, `0.0..=1.0`).
    pub parts: BTreeMap<String, f64>,
}

/// Weights for the composite health index (config section in R30).
///
/// Parts not listed keep weight `1.0`; the index normalizes by the sum of
/// the weights of the *measurable* parts.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HealthWeights {
    /// Part name → weight (negative weights are rejected by the gate).
    pub weights: BTreeMap<String, f64>,
}

impl HealthWeights {
    /// Weight of one part; unlisted parts weigh `1.0`.
    pub fn weight(&self, part: &str) -> f64 {
        self.weights.get(part).copied().unwrap_or(1.0)
    }
}

/// The four tracked technical-debt marker kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MarkerKind {
    Todo,
    Fixme,
    Hack,
    Xxx,
}

impl MarkerKind {
    /// The marker keyword as written in source.
    pub fn keyword(self) -> &'static str {
        match self {
            MarkerKind::Todo => "TODO",
            MarkerKind::Fixme => "FIXME",
            MarkerKind::Hack => "HACK",
            MarkerKind::Xxx => "XXX",
        }
    }
}

/// One technical-debt marker occurrence in one file, produced by the
/// scanner in `archguard-extract` (`debt.rs`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DebtMarker {
    pub kind: MarkerKind,
    /// 1-based line of the comment carrying the marker.
    pub line: u32,
    /// Source-root-relative path of the scanned file.
    pub file: String,
}

/// Exported/internal symbol split of one source file.
///
/// Produced by the language scanners in `archguard-extract` (WP-02) and
/// consumed by the leaky-encapsulation analysis in `archguard-analysis`;
/// it lives here because both L1 crates share it without depending on
/// each other.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Visibility {
    /// Source-root-relative path of the file.
    pub file: String,
    /// Names reachable from outside the file.
    pub exported: std::collections::BTreeSet<String>,
    /// Names hidden behind a language-private marker.
    pub internal: std::collections::BTreeSet<String>,
}

/// One leaky-encapsulation finding: a file reaching a symbol that its
/// own file does not export.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Leak {
    /// File holding the reference.
    pub from_file: String,
    /// Referenced symbol.
    pub symbol: String,
    /// File defining the symbol.
    pub in_file: String,
    /// Human-readable evidence (`file:line`).
    pub evidence: Vec<String>,
}

/// One resolved external dependency (R22).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalDependency {
    /// Ecosystem key: `cargo`, `npm`, `pypi`, `composer`, `maven`.
    pub ecosystem: String,
    /// Package name as written in the lockfile.
    pub name: String,
    /// Resolved version; `None` when the manifest carries none.
    pub version: Option<String>,
    /// `Some(true)` when imported metadata marks the version outdated,
    /// `Some(false)` when current, `None` without metadata.
    pub stale: Option<bool>,
}

/// Inventory of external dependencies across all ecosystems (R22).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DependencyInventory {
    /// Total number of distinct external dependencies.
    pub total: usize,
    /// Distinct dependency count per ecosystem key.
    pub by_ecosystem: BTreeMap<String, usize>,
    /// Outdated dependencies; `None` when no registry metadata was
    /// imported (not zero — not measured).
    pub stale_count: Option<usize>,
    /// All dependencies, sorted by `(ecosystem, name, version)`.
    pub dependencies: Vec<ExternalDependency>,
    /// Coverage notes: manifests read, manifests skipped and why.
    pub notes: Vec<String>,
}

/// All review KPIs in one document, embedded into the analysis payload
/// as the optional `review_kpis` field of [`Graph`](crate::model::Graph) (R23).
/// Every field is `#[serde(default)]` so payloads written before a
/// field existed still load; `None` means "not measured", never "clean".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReviewKpis {
    /// Propagation cost per module (R04).
    pub propagation_cost: Vec<PropagationCost>,
    /// Stable-dependency-principle violations (R05).
    pub sdp_violations: Vec<SdpViolation>,
    /// Martin A/I zone distribution (R06); `None` without abstraction data.
    pub zones: Option<ZoneStats>,
    /// Tangledness (R07); `None` when not computable.
    pub tangledness: Option<Tangledness>,
    /// Hub concentration (R08); `None` when not computable.
    pub hub_gini: Option<HubGini>,
    /// Declared-vs-derived layer agreement (R09); `None` without declared layers.
    pub layer_drift: Option<LayerDrift>,
    /// Debt-marker density (R11/R12); `None` when no scan ran.
    pub debt: Option<DebtDensity>,
    /// Aggregated exported API surface (R10–R16); empty when no file
    /// exports a symbol.
    pub api_surface: Vec<ApiSurface>,
    /// Leaky encapsulation findings (R17).
    pub leaks: Vec<Leak>,
    /// Risk-weighted coverage gaps, top-N (R21).
    pub risk_gaps: Vec<RiskCoverageGap>,
    /// External dependency inventory (R22); `None` when no manifest was found.
    pub dependencies: Option<DependencyInventory>,
    /// Composite health index (R09); `None` when no part was measurable.
    pub health: Option<HealthIndex>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn propagation_cost_round_trips() {
        let v = PropagationCost {
            scc_id: "scc-3".to_string(),
            size: 3,
            affected: 7,
            total: 10,
            pc: 0.7,
        };
        let json = serde_json::to_string(&v).expect("serialization");
        let back: PropagationCost = serde_json::from_str(&json).expect("deserialization");
        assert_eq!(v, back);
        assert_eq!(back.pc, 0.7);
    }

    #[test]
    fn every_kpi_type_deserializes_from_an_empty_object() {
        // `#[serde(default)]` throughout: a payload without any key loads
        // into the zero value instead of failing.
        assert_eq!(
            serde_json::from_str::<PropagationCost>("{}").unwrap(),
            PropagationCost::default()
        );
        assert_eq!(
            serde_json::from_str::<Tangledness>("{}").unwrap(),
            Tangledness::default()
        );
        assert_eq!(
            serde_json::from_str::<HubGini>("{}").unwrap(),
            HubGini::default()
        );
        assert_eq!(
            serde_json::from_str::<SdpViolation>("{}").unwrap(),
            SdpViolation::default()
        );
        assert_eq!(
            serde_json::from_str::<ZoneStats>("{}").unwrap(),
            ZoneStats::default()
        );
        assert_eq!(
            serde_json::from_str::<LayerDrift>("{}").unwrap(),
            LayerDrift::default()
        );
        assert_eq!(
            serde_json::from_str::<DebtDensity>("{}").unwrap(),
            DebtDensity::default()
        );
        assert_eq!(
            serde_json::from_str::<ApiSurface>("{}").unwrap(),
            ApiSurface::default()
        );
        assert_eq!(
            serde_json::from_str::<RiskCoverageGap>("{}").unwrap(),
            RiskCoverageGap::default()
        );
        assert_eq!(
            serde_json::from_str::<HealthIndex>("{}").unwrap(),
            HealthIndex::default()
        );
        assert_eq!(serde_json::from_str::<Leak>("{}").unwrap(), Leak::default());
    }

    #[test]
    fn unknown_measurements_are_null_not_zero() {
        // SDP: an unmeasurable instability must stay visibly unknown.
        let v = SdpViolation {
            from: "app".to_string(),
            to: "core".to_string(),
            i_from: None,
            i_to: Some(0.8),
            evidence: vec!["src/app/a.ts:3".to_string()],
        };
        let value = serde_json::to_value(&v).expect("serialization");
        assert!(value["i_from"].is_null());
        assert_eq!(value["i_to"], 0.8);
        let back: SdpViolation = serde_json::from_value(value).expect("deserialization");
        assert_eq!(v, back);

        // Coverage gap without coverage data keeps both fields null.
        let gap = RiskCoverageGap {
            path: "src/app/a.ts".to_string(),
            hotspot: 80.0,
            coverage: None,
            gap: None,
        };
        let value = serde_json::to_value(&gap).expect("serialization");
        assert!(value["coverage"].is_null());
        assert!(value["gap"].is_null());
        assert_eq!(
            serde_json::from_value::<RiskCoverageGap>(value).expect("deserialization"),
            gap
        );
    }

    #[test]
    fn health_level_uses_lowercase_names_and_defaults_to_red() {
        for (level, name) in [
            (HealthLevel::Green, "green"),
            (HealthLevel::Yellow, "yellow"),
            (HealthLevel::Red, "red"),
        ] {
            let json = serde_json::to_string(&level).expect("serialization");
            assert_eq!(json, format!("\"{name}\""));
            let back: HealthLevel = serde_json::from_str(&json).expect("deserialization");
            assert_eq!(back, level);
        }
        assert_eq!(HealthLevel::default(), HealthLevel::Red);
    }

    #[test]
    fn health_index_round_trips_with_parts_and_trend() {
        let mut parts = BTreeMap::new();
        parts.insert("sdp".to_string(), 0.9);
        parts.insert("tangledness".to_string(), 0.4);
        let v = HealthIndex {
            score: Some(72.5),
            level: Some(HealthLevel::Yellow),
            trend: Some(-3.5),
            parts,
        };
        let json = serde_json::to_string(&v).expect("serialization");
        let back: HealthIndex = serde_json::from_str(&json).expect("deserialization");
        assert_eq!(v, back);
        // Without a previous snapshot the trend is null, not 0.
        let v = HealthIndex {
            score: Some(81.0),
            level: Some(HealthLevel::Green),
            trend: None,
            parts: BTreeMap::new(),
        };
        let value = serde_json::to_value(&v).expect("serialization");
        assert!(value["trend"].is_null());
    }

    #[test]
    fn health_weights_default_unlisted_parts_to_one() {
        let weights = HealthWeights::default();
        assert_eq!(weights.weight("anything"), 1.0);
        let weights = HealthWeights {
            weights: [("sdp".to_string(), 2.0)].into_iter().collect(),
        };
        assert_eq!(weights.weight("sdp"), 2.0);
        assert_eq!(weights.weight("other"), 1.0);
        let json = serde_json::to_string(&weights).expect("serialization");
        let back: HealthWeights = serde_json::from_str(&json).expect("deserialization");
        assert_eq!(weights, back);
    }

    #[test]
    fn debt_density_carries_kind_breakdown() {
        let mut by_kind = BTreeMap::new();
        by_kind.insert("todo".to_string(), 4usize);
        by_kind.insert("fixme".to_string(), 1usize);
        let v = DebtDensity {
            markers: 5,
            loc: 2000,
            per_kloc: Some(2.5),
            by_kind,
        };
        let json = serde_json::to_string(&v).expect("serialization");
        let back: DebtDensity = serde_json::from_str(&json).expect("deserialization");
        assert_eq!(v, back);
        assert_eq!(back.by_kind["todo"], 4);
        // A file-less scan cannot produce a per-kloc rate.
        let empty = DebtDensity::default();
        assert_eq!(empty.per_kloc, None);
    }

    #[test]
    fn api_surface_and_leak_round_trip() {
        let v = ApiSurface {
            exported_symbols: 12,
            files: 4,
            per_file: Some(3.0),
            top: vec![("src/app/a.ts".to_string(), 7)],
        };
        let json = serde_json::to_string(&v).expect("serialization");
        assert_eq!(
            serde_json::from_str::<ApiSurface>(&json).expect("deserialization"),
            v
        );
        let leak = Leak {
            from_file: "src/app/a.ts".to_string(),
            symbol: "Secret".to_string(),
            in_file: "src/domain/s.ts".to_string(),
            evidence: vec!["src/app/a.ts:9".to_string()],
        };
        let json = serde_json::to_string(&leak).expect("serialization");
        assert_eq!(
            serde_json::from_str::<Leak>(&json).expect("deserialization"),
            leak
        );
    }

    #[test]
    fn empty_review_kpis_round_trips_and_legacy_payloads_load() {
        let json = serde_json::to_string(&ReviewKpis::default()).expect("serialization");
        let back: ReviewKpis = serde_json::from_str(&json).expect("deserialization");
        assert_eq!(back, ReviewKpis::default());
        // A payload written before some of these fields existed.
        let legacy: ReviewKpis =
            serde_json::from_str(r#"{"propagation_cost":[]}"#).expect("missing fields default");
        assert_eq!(legacy, ReviewKpis::default());
    }

    #[test]
    fn zone_stats_round_trips() {
        let v = ZoneStats {
            zone_of_pain: 2,
            zone_of_uselessness: 1,
            main_sequence: 8,
            unmeasured: 3,
        };
        let json = serde_json::to_string(&v).expect("serialization");
        assert_eq!(
            serde_json::from_str::<ZoneStats>(&json).expect("deserialization"),
            v
        );
    }
}
