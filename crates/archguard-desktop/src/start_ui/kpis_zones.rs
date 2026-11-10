//! The A/I zone scatter of the glance profile: the port of
//! `renderDashZones` (viewer_dash_zones.js). Points are the node-scope
//! abstraction metrics of the engine's structural performance report
//! (`analyze_performance`, as the CLI's report embeds them); the zone
//! counts are `review_zones` over the same report. Unmeasured boundaries
//! are never plotted.

use archguard_engine::core::{
    AbstractionMetric, CallAnalysis, Graph, PerformanceReport, ReviewKpis,
};

use crate::context::Loaded;
use crate::format::Locale;
use crate::model_sync::sync_rows;
use crate::ui::{StartAiPoint, StartGlance};

/// Mirrored from archguard-analysis `zones.rs` (as the web does): the
/// ±1/3 main-sequence corridor and its rounding tolerance.
const MAIN_SEQUENCE_MAX_DISTANCE: f64 = 1.0 / 3.0;
const DISTANCE_TOLERANCE: f64 = 1e-9;

/// `StartAiPoint.zone` values.
pub mod zone {
    pub const PAIN: i32 = 1;
    pub const USELESS: i32 = 2;
    pub const MAIN: i32 = 3;
}

/// What the performance report needs besides the call analysis, copied
/// from the document on the UI thread.
#[derive(Debug, Clone)]
pub struct Inputs {
    root: std::path::PathBuf,
    config: archguard_engine::core::Config,
    graph: Graph,
    view: Graph,
    sources: archguard_engine::core::ReviewSources,
}

impl Inputs {
    pub fn of(data: &Loaded) -> Self {
        Inputs {
            root: data.root.to_path_buf(),
            config: data.run.config.clone(),
            graph: data.run.graph.clone(),
            view: data.run.view.clone(),
            sources: data.sources.clone(),
        }
    }

    /// The report the CLI's `review_cmd.rs` builds for the viewer; `None`
    /// when the analysis fails (the profile then says not measured).
    pub fn measure(&self, calls: &CallAnalysis) -> Option<(Graph, PerformanceReport)> {
        let _span = archguard_engine::self_trace::tracing::info_span!("measure").entered();
        let ids = archguard_engine::analysis::match_review_entry_points(
            &self.graph,
            &self.sources,
            &self.config.review.entry_points,
        )
        .ok()?;
        let report = archguard_engine::analyze_performance(
            &self.root,
            &self.config,
            &self.graph,
            &self.view,
            &self.sources,
            calls,
            &ids,
        )
        .ok()?;
        Some((self.view.clone(), report))
    }
}

/// The measured report into the review KPIs, as the CLI's report does:
/// zone counts from `review_zones` (the pipeline leaves them unmeasured);
/// returns the plottable points. No report leaves both unmeasured.
pub fn apply(
    kpis: Option<&mut ReviewKpis>,
    measured: Option<(Graph, PerformanceReport)>,
) -> Vec<AbstractionMetric> {
    let Some((view, report)) = measured else {
        return Vec::new();
    };
    if let Some(k) = kpis {
        k.zones = archguard_engine::review_zones(&view, &report);
    }
    points(&report)
}

/// `dashZoneOf`: the zone of a boundary from its A and I; `D'` is
/// recomputed, never read from the metric.
pub fn zone_of(abstractness: f64, instability: f64) -> i32 {
    let sum = abstractness + instability;
    if (sum - 1.0).abs() <= MAIN_SEQUENCE_MAX_DISTANCE + DISTANCE_TOLERANCE {
        zone::MAIN
    } else if sum < 1.0 {
        zone::PAIN
    } else {
        zone::USELESS
    }
}

/// The plottable metrics: node scope with a finite A and I.
pub fn points(report: &PerformanceReport) -> Vec<AbstractionMetric> {
    report
        .abstractions
        .iter()
        .filter(|m| {
            m.scope == "node"
                && m.abstractness.is_some_and(f64::is_finite)
                && m.instability.is_finite()
        })
        .cloned()
        .collect()
}

/// Fill the scatter's points and the legend counts.
pub fn fill(gl: &mut StartGlance, old: &StartGlance, points: &[AbstractionMetric], l: Locale) {
    let rows: Vec<StartAiPoint> = points
        .iter()
        .filter_map(|m| {
            let (a, i) = (m.abstractness?, m.instability);
            let d = (a + i - 1.0).abs();
            Some(StartAiPoint {
                name: m.name.as_str().into(),
                x: i as f32,
                y: a as f32,
                zone: zone_of(a, i),
                tip: format!(
                    "{}\nA={} · I={} · D'={}",
                    m.name,
                    l.dec(a, 2),
                    l.dec(i, 2),
                    l.dec(d, 2)
                )
                .into(),
            })
        })
        .collect();
    let count =
        |z: i32| i32::try_from(rows.iter().filter(|p| p.zone == z).count()).unwrap_or(i32::MAX);
    gl.ai_pain = count(zone::PAIN);
    gl.ai_useless = count(zone::USELESS);
    gl.ai_main = count(zone::MAIN);
    gl.points = sync_rows(&old.points, rows);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metric(name: &str, scope: &str, a: Option<f64>, i: f64) -> AbstractionMetric {
        AbstractionMetric {
            scope: scope.into(),
            name: name.into(),
            nodes: vec![name.into()],
            types: 1,
            abstract_types: 0,
            fan_in: 0,
            fan_out: 0,
            instability: i,
            abstractness: a,
            distance: None,
            coverage: if a.is_some() {
                "complete"
            } else {
                "unavailable"
            }
            .into(),
            reason: None,
        }
    }

    /// The corridor of `zones.rs`: its edge stays on the main sequence.
    #[test]
    fn zones_follow_the_main_sequence_corridor() {
        assert_eq!(zone_of(0.0, 1.0), zone::MAIN);
        assert_eq!(zone_of(1.0, 0.0), zone::MAIN);
        assert_eq!(zone_of(0.0, 2.0 / 3.0), zone::MAIN);
        assert_eq!(zone_of(0.0, 0.0), zone::PAIN);
        assert_eq!(zone_of(1.0, 1.0), zone::USELESS);
    }

    /// Only measured node boundaries become points.
    #[test]
    fn unmeasured_and_cycle_metrics_are_never_plotted() {
        let report = PerformanceReport {
            abstractions: vec![
                metric("a", "node", Some(0.5), 0.25),
                metric("b", "node", None, 0.5),
                metric("scc-1", "scc", Some(0.0), 0.0),
                metric("c", "node", Some(f64::NAN), 0.5),
            ],
            ..Default::default()
        };
        let names: Vec<String> = points(&report).into_iter().map(|m| m.name).collect();
        assert_eq!(names, ["a"]);
    }

    /// Each point carries I as x, A as y, its zone and tip; the legend
    /// counts the points per zone.
    #[test]
    fn points_carry_position_zone_and_tip() {
        let mut gl = StartGlance::default();
        let pts = [
            metric("core", "node", Some(0.5), 0.25),
            metric("db", "node", Some(0.0), 0.0),
            metric("api", "node", Some(1.0), 1.0),
        ];
        fill(&mut gl, &StartGlance::default(), &pts, Locale::En);
        let p = |i: usize| gl.points.row_data(i).unwrap();
        assert_eq!(gl.points.row_count(), 3);
        assert_eq!((p(0).name.as_str(), p(0).x, p(0).y), ("core", 0.25, 0.5));
        assert_eq!(p(0).zone, zone::MAIN);
        assert_eq!(p(0).tip, "core\nA=0.50 · I=0.25 · D'=0.25");
        assert_eq!((p(1).zone, p(2).zone), (zone::PAIN, zone::USELESS));
        assert_eq!((gl.ai_pain, gl.ai_useless, gl.ai_main), (1, 1, 1));
    }

    /// The report fills the zone counts and keeps the measured points.
    #[test]
    fn the_report_measures_zones_and_points() {
        let mut kpis = ReviewKpis::default();
        let report = PerformanceReport {
            abstractions: vec![
                metric("a", "node", Some(0.0), 1.0),
                metric("b", "node", None, 0.5),
            ],
            ..Default::default()
        };
        let view = Graph {
            schema_version: 1,
            root: String::new(),
            nodes: Vec::new(),
            edges: Vec::new(),
            semantic: None,
            review_kpis: None,
        };
        let points = apply(Some(&mut kpis), Some((view, report)));
        let zones = kpis.zones.expect("measured");
        assert_eq!((zones.main_sequence, zones.unmeasured), (1, 1));
        assert_eq!(points.len(), 1);
        assert!(apply(None, None).is_empty());
    }

    use slint::Model;
}
