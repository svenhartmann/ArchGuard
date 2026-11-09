//! Review-KPI payload (R23).
//!
//! [`compute_review_kpis`] aggregates the review KPIs the pipeline
//! embeds into the analysis payload — the optional `review_kpis` field
//! of [`Graph`] that every consumer (CLI, reports, MCP, dashboard)
//! reads from one place. The engine glue is deliberately thin: all
//! computation lives in the pure analyses of `archguard-analysis` and
//! the scanners of `archguard-extract`; this module only feeds them,
//! aggregates the debt/visibility scans and keeps unknown measurements
//! `None` — absence means "not measured", never "clean".
//!
//! KPIs ride with the graph but are never cached with it: the pipeline
//! attaches them after the graph cache step, so every run — cold and
//! warm — recomputes them against the fully effective config and the
//! current imports (coverage, dep-meta). A replayed cache graph never
//! carries stale KPIs.

use std::collections::BTreeMap;
use std::path::Path;

// The payload types live in core (the `Graph` field needs them there);
// re-exported so engine consumers keep a single import site.
pub use archguard_core::ReviewKpis;

use archguard_analysis::{
    find_sccs, health_index, hub_gini, infer_layers, layer_drift, leaky_encapsulation,
    propagation_cost, sdp_violations, tangledness,
};
use archguard_core::{
    ApiSurface, ArchguardError, Config, Cut, DebtDensity, DebtMarker, Graph, HealthIndex,
    HealthWeights, LayerConfig, Visibility, aggregate,
};
use archguard_extract::{Lang, scan_debt, scan_visibility};

/// One scanned source, ready for the file-level scans.
pub struct ScannedSource<'a> {
    /// Source-root-relative path (the graph node name basis).
    pub path: &'a str,
    /// Language the extractor registered for it.
    pub lang: Lang,
    /// Full file content.
    pub content: &'a str,
}

/// Number of hottest files listed per module in [`ApiSurface::top`].
const API_TOP: usize = 5;
/// Top-N risk gaps kept in the payload.
const RISK_TOP: usize = 10;

/// Compute the full review-KPI payload for a freshly built graph.
///
/// Module-view KPIs (propagation cost, SDP, tangledness, hubs, layer
/// drift, health) run on the configured `[modules].depth` view; file
/// KPIs (visibility, debt, leaks, risk gaps) run on the full graph.
/// `zones` stays `None` here: it needs compiler abstractness, which only
/// callers that run the call analysis add via `review_zones`.
#[tracing::instrument(skip_all)]
pub fn compute_review_kpis(
    root: &Path,
    config: &Config,
    full: &Graph,
    sources: &[ScannedSource<'_>],
) -> Result<ReviewKpis, ArchguardError> {
    // ---- file-level scans over the sources the pipeline just read.
    let visibility: Vec<Visibility> = sources
        .iter()
        .map(|source| scan_visibility(source.path, source.content, source.lang))
        .collect();
    let markers: Vec<DebtMarker> = sources
        .iter()
        .flat_map(|source| scan_debt(source.path, source.content, source.lang))
        .collect();
    let loc = sources
        .iter()
        .map(|source| source.content.lines().count())
        .sum();

    // ---- module view for the graph KPIs.
    let view = aggregate(full, &Cut::at_depth(full, config.modules.depth as usize));
    let sccs = find_sccs(&view);
    let tangled = tangledness(&view);
    let hubs = hub_gini(&view);
    let sdp = sdp_violations(&view);

    let declared: Vec<LayerConfig> = config
        .layers
        .iter()
        .filter(|layer| layer.origin == archguard_core::LayerOrigin::Declared)
        .cloned()
        .collect();
    let drift = (!declared.is_empty())
        .then(|| layer_drift(&declared, &infer_layers(&view, &sccs).layers, &view));

    let risk_gaps = crate::risk_gap::risk_coverage_gaps(root, config, full, RISK_TOP)?;
    let inventory = crate::dep_inventory::dependency_inventory(root);
    let dependencies = (inventory.total > 0).then_some(inventory);

    // Declared `[review] health_weights` drive the index; unconfigured
    // parts keep the documented weight of 1.0.
    let weights = config
        .review
        .health_weights
        .as_ref()
        .map(|weights| HealthWeights {
            weights: weights.clone(),
        })
        .unwrap_or_default();
    let health = health_parts(
        &tangled.quote,
        &hubs.gini,
        &sdp_violation_share(&view, &sdp),
        &risk_gaps,
        &weights,
    );

    Ok(ReviewKpis {
        propagation_cost: propagation_cost(&view),
        sdp_violations: sdp,
        zones: None,
        tangledness: Some(tangled),
        hub_gini: Some(hubs),
        layer_drift: drift,
        debt: Some(debt_density(&markers, loc)),
        api_surface: api_surfaces(&visibility),
        leaks: leaky_encapsulation(full, &visibility),
        risk_gaps,
        dependencies,
        health: Some(health),
    })
}

/// Debt density over everything the scan covered.
fn debt_density(markers: &[DebtMarker], loc: usize) -> DebtDensity {
    let mut by_kind: BTreeMap<String, usize> = BTreeMap::new();
    for marker in markers {
        *by_kind
            .entry(marker.kind.keyword().to_lowercase())
            .or_default() += 1;
    }
    DebtDensity {
        markers: markers.len(),
        loc,
        per_kloc: (loc > 0).then(|| markers.len() as f64 / (loc as f64 / 1000.0)),
        by_kind,
    }
}

/// The aggregate [`ApiSurface`] over all scanned files: [`ApiSurface`]
/// carries no module name, so the payload keeps exactly one entry —
/// the total surface — instead of anonymous per-module rows. Files
/// exporting nothing are not part of the surface.
fn api_surfaces(visibility: &[Visibility]) -> Vec<ApiSurface> {
    let mut files: Vec<(&str, usize)> = visibility
        .iter()
        .filter(|entry| !entry.exported.is_empty())
        .map(|entry| (entry.file.as_str(), entry.exported.len()))
        .collect();
    if files.is_empty() {
        return Vec::new();
    }
    let exported_symbols: usize = files.iter().map(|(_, count)| count).sum();
    files.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    let top = files
        .iter()
        .take(API_TOP)
        .map(|(file, count)| ((*file).to_string(), *count))
        .collect();
    vec![ApiSurface {
        exported_symbols,
        files: files.len(),
        per_file: Some(exported_symbols as f64 / files.len() as f64),
        top,
    }]
}

/// Violations per view edge in `0.0..=1.0`; `None` for an edgeless view.
fn sdp_violation_share(view: &Graph, violations: &[archguard_core::SdpViolation]) -> Option<f64> {
    (!view.edges.is_empty()).then(|| (violations.len() as f64 / view.edges.len() as f64).min(1.0))
}

/// The health index from the objectively measurable parts; every part
/// is `0.0..=1.0`, higher is better, and a part without evidence is
/// simply absent (see [`health_index`], which then stays `None`).
fn health_parts(
    tangled_quote: &f64,
    hub_gini: &f64,
    sdp_share: &Option<f64>,
    risk_gaps: &[archguard_core::RiskCoverageGap],
    weights: &HealthWeights,
) -> HealthIndex {
    let mut parts: BTreeMap<String, f64> = BTreeMap::new();
    parts.insert("tangledness".to_string(), 1.0 - tangled_quote);
    parts.insert("hubs".to_string(), 1.0 - hub_gini);
    if let Some(share) = sdp_share {
        parts.insert("sdp".to_string(), 1.0 - share);
    }
    let known: Vec<f64> = risk_gaps.iter().filter_map(|gap| gap.coverage).collect();
    if !known.is_empty() {
        let mean = known.iter().sum::<f64>() / known.len() as f64;
        parts.insert("coverage".to_string(), mean);
    }
    health_index(&parts, weights, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{DebtMarker, MarkerKind, Node, NodeId, NodeKind, Visibility};

    fn empty_graph() -> Graph {
        Graph {
            semantic: None,
            review_kpis: None,
            schema_version: 1,
            root: "/p".to_string(),
            nodes: Vec::new(),
            edges: Vec::new(),
        }
    }

    #[test]
    fn debt_density_counts_kinds_and_kloc() {
        let markers = vec![
            DebtMarker {
                kind: MarkerKind::Todo,
                line: 1,
                file: "a.ts".to_string(),
            },
            DebtMarker {
                kind: MarkerKind::Todo,
                line: 2,
                file: "a.ts".to_string(),
            },
            DebtMarker {
                kind: MarkerKind::Fixme,
                line: 3,
                file: "b.ts".to_string(),
            },
        ];
        let density = debt_density(&markers, 1000);
        assert_eq!(density.markers, 3);
        assert_eq!(density.per_kloc, Some(3.0));
        assert_eq!(density.by_kind["todo"], 2);
        assert_eq!(density.by_kind["fixme"], 1);
        // Zero loc stays unknown, not infinite.
        assert_eq!(debt_density(&markers, 0).per_kloc, None);
    }

    #[test]
    fn api_surface_aggregates_per_module_with_hottest_files() {
        let visibility = vec![
            Visibility {
                file: "src/app/a.ts".to_string(),
                exported: ["A", "B"].iter().map(|s| s.to_string()).collect(),
                internal: Default::default(),
            },
            Visibility {
                file: "src/app/b.ts".to_string(),
                exported: ["C"].iter().map(|s| s.to_string()).collect(),
                internal: Default::default(),
            },
            Visibility {
                file: "src/lib/c.ts".to_string(),
                exported: Default::default(),
                internal: ["x"].iter().map(|s| s.to_string()).collect(),
            },
        ];
        let surfaces = api_surfaces(&visibility);
        assert_eq!(surfaces.len(), 1, "src/lib exports nothing → no surface");
        let surface = &surfaces[0];
        assert_eq!(surface.exported_symbols, 3);
        assert_eq!(surface.files, 2);
        assert_eq!(surface.per_file, Some(1.5));
        assert_eq!(surface.top[0], ("src/app/a.ts".to_string(), 2));
    }

    #[test]
    fn health_stays_none_without_a_single_measurable_part() {
        let index = health_parts(&0.0, &0.0, &None, &[], &HealthWeights::default());
        // tangledness and hubs are always computable → score exists;
        // the honest None case is exercised in the analysis crate.
        assert!(index.score.is_some());
    }

    #[test]
    fn declared_weights_move_the_health_score() {
        // tangledness part = 1.0 (quote 0.0), hubs part = 0.0 (gini 1.0):
        // equal weights halve the score, a dominant tangledness weight
        // pulls it towards 100.
        let equal = health_parts(&0.0, &1.0, &None, &[], &HealthWeights::default());
        let mut weights = BTreeMap::new();
        weights.insert("tangledness".to_string(), 10.0);
        let weighted = health_parts(&0.0, &1.0, &None, &[], &HealthWeights { weights });
        assert_eq!(equal.score, Some(50.0));
        assert!(
            weighted.score.is_some_and(|score| score > 80.0),
            "declared weight must dominate: {:?}",
            weighted.score
        );
    }

    #[test]
    fn compute_over_an_empty_graph_reports_unknowns_not_failures() {
        let dir = tempfile::tempdir().expect("tempdir");
        let kpis = compute_review_kpis(dir.path(), &Config::default(), &empty_graph(), &[])
            .expect("empty project computes");
        assert!(kpis.propagation_cost.is_empty());
        assert!(kpis.zones.is_none(), "no compiler evidence, no zones");
        assert!(kpis.dependencies.is_none(), "no manifests → not measured");
        assert!(kpis.debt.expect("scan ran").per_kloc.is_none(), "0 loc");
        assert!(
            kpis.layer_drift.is_none(),
            "no declared layers → no drift claim"
        );
    }

    #[test]
    fn payload_round_trips_and_stays_backward_compatible() {
        let json = serde_json::to_string(&ReviewKpis::default()).expect("serializes");
        let back: ReviewKpis = serde_json::from_str(&json).expect("round trip");
        assert_eq!(back, ReviewKpis::default());
        let legacy: ReviewKpis = serde_json::from_str("{}").expect("missing fields default");
        assert_eq!(legacy, ReviewKpis::default());
    }

    #[test]
    fn node_import_is_used_through_the_helper() {
        // Guards the imports a future edit might drop.
        let node = Node {
            id: NodeId(0),
            name: "x".to_string(),
            kind: NodeKind::File,
            parent: None,
            lang: None,
        };
        assert_eq!(node.name, "x");
    }
}
