//! The Runtime tab's "Time" page and the flame graph: shares and numbers
//! for `ui/runtime_time.slint`. Categories come from the engine
//! (`RuntimeSpan::category`, `RuntimeBreakdown::parts`).
use std::rc::Rc;

use archguard_engine::core::{RuntimeProfile, RuntimeRequest};
use slint::{ModelRc, VecModel};

use super::summary::{number, short_function, span_label};
use crate::ui::{RtBreakdown, RtFlame, RtHot, RtPart, RtSampled, RtSampledRow};

/// The engine's categories in the order of `RtTime` in the Slint file.
const CATEGORIES: [&str; 9] = [
    "database",
    "outgoing",
    "controller",
    "service",
    "repository",
    "component",
    "method",
    "other",
    "unattributed",
];

/// Sampled methods shown per request; the CLI JSON holds up to 50.
const SAMPLED_ROWS: usize = 12;

fn index(category: &str) -> i32 {
    CATEGORIES.iter().position(|c| *c == category).unwrap_or(7) as i32
}

fn list<T: Clone + 'static>(rows: Vec<T>) -> ModelRc<T> {
    ModelRc::from(Rc::new(VecModel::from(rows)))
}

fn share(v: f64, max: f64) -> f32 {
    if max > 0.0 { (v / max) as f32 } else { 0.0 }
}

/// One stacked bar per endpoint, scaled to the slowest endpoint's mean.
pub fn breakdown(p: &RuntimeProfile) -> ModelRc<RtBreakdown> {
    let max = p.breakdown.iter().map(|b| b.mean_ms).fold(0.0, f64::max);
    let mut rows: Vec<_> = p.breakdown.iter().collect();
    rows.sort_by(|a, b| b.mean_ms.total_cmp(&a.mean_ms));
    list(
        rows.into_iter()
            .map(|b| RtBreakdown {
                service: b.service.as_str().into(),
                name: b.name.as_str().into(),
                requests: i32::try_from(b.requests).unwrap_or(i32::MAX),
                mean: number(b.mean_ms),
                parts: list(
                    b.parts
                        .iter()
                        .map(|x| RtPart {
                            category: index(&x.category),
                            width: share(x.mean_ms, max),
                            ms: number(x.mean_ms),
                        })
                        .collect(),
                ),
            })
            .collect(),
    )
}

/// Methods by self time, bars scaled to the first.
pub fn hot(p: &RuntimeProfile) -> ModelRc<RtHot> {
    let max = p.hot_methods.first().map_or(0.0, |m| m.self_ms);
    list(
        p.hot_methods
            .iter()
            .map(|m| RtHot {
                name: short_function(&m.name).into(),
                category: index(m.kind.as_deref().unwrap_or("other")),
                calls: i32::try_from(m.calls).unwrap_or(i32::MAX),
                self_ms: number(m.self_ms),
                total_ms: number(m.total_ms),
                share: share(m.self_ms, max),
            })
            .collect(),
    )
}

/// Profiler samples of one request by method; empty when none were recorded.
pub fn sampled(r: Option<&RuntimeRequest>) -> RtSampled {
    let Some((r, s)) = r.and_then(|r| Some((r, r.samples.as_ref()?))) else {
        return RtSampled::default();
    };
    let total = s.samples as f64;
    let count = |n: u64| i32::try_from(n).unwrap_or(i32::MAX);
    RtSampled {
        has: true,
        title: format!("{} · {}", r.service, r.name).into(),
        samples: count(s.samples),
        interval: number(s.interval_ms),
        clock: s.clock.as_str().into(),
        lower_bound: number(total * s.interval_ms),
        methods: list(
            s.methods
                .iter()
                .take(SAMPLED_ROWS)
                .map(|m| RtSampledRow {
                    name: short_function(&m.function).into(),
                    self_samples: count(m.self_samples),
                    total_samples: count(m.total_samples),
                    self_share: share(m.self_samples as f64, total),
                    total_share: share(m.total_samples as f64, total),
                })
                .collect(),
        ),
    }
}

/// The request's spans as flame graph blocks and the deepest level.
pub fn flame(r: &RuntimeRequest) -> (ModelRc<RtFlame>, i32) {
    let total = r.duration_ms;
    let blocks: Vec<RtFlame> = r
        .spans
        .iter()
        .map(|s| RtFlame {
            depth: i32::try_from(s.depth).unwrap_or(i32::MAX),
            label: span_label(s).into(),
            x: share(s.offset_ms, total),
            w: share(s.duration_ms, total),
            category: index(&s.category),
        })
        .collect();
    let depth = blocks.iter().map(|b| b.depth).max().unwrap_or(0);
    (list(blocks), depth)
}

#[cfg(test)]
mod tests {
    use slint::Model;

    use super::*;

    fn fixture() -> RuntimeProfile {
        load("shop-dotnet.jsonl")
    }

    fn load(name: &str) -> RuntimeProfile {
        let dir = tempfile::tempdir().unwrap();
        let run = dir.path().join(".archguard/runtime/run");
        std::fs::create_dir_all(&run).unwrap();
        let path = format!(
            "{}/../../fixtures/runtime/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        std::fs::copy(path, run.join("traces-0001.jsonl")).unwrap();
        archguard_engine::runtime_profile(dir.path(), None).unwrap()
    }

    #[test]
    fn bars_are_scaled_to_the_slowest_endpoint() {
        let p = fixture();
        let bars = breakdown(&p);
        let first = bars.row_data(0).unwrap();
        let total: f32 = first.parts.iter().map(|x| x.width).sum();
        assert!(
            (total - 1.0).abs() < 0.01,
            "the slowest bar fills the track: {total}"
        );
        for b in bars.iter() {
            let width: f32 = b.parts.iter().map(|x| x.width).sum();
            assert!(width <= 1.01, "{width}");
        }
    }

    #[test]
    fn hot_methods_and_flame_use_the_engine_categories() {
        let p = fixture();
        let rows = hot(&p);
        let first = rows.row_data(0).unwrap();
        assert_eq!(
            (first.name.as_str(), first.category, first.share),
            ("OrderRepository.GetOrderAsync", 4, 1.0)
        );
        let summary = p
            .requests
            .iter()
            .find(|r| r.name.contains("summary"))
            .unwrap();
        let (blocks, depth) = flame(summary);
        assert_eq!(depth, 4, "controller, service, repository, database");
        let db = blocks.iter().filter(|b| b.category == 0).count();
        assert_eq!(db, 7);
    }

    /// The Time page's sampled methods of the selected request, with the
    /// numbers the honesty rules ask for: count, interval, clock.
    #[test]
    fn sampled_methods_of_the_selected_request() {
        let p = load("shop-java-profile.jsonl");
        let r = p
            .requests
            .iter()
            .find(|r| r.name.contains("simulation"))
            .unwrap();
        let m = sampled(Some(r));
        assert!(m.has);
        assert_eq!(
            (m.samples, m.interval.as_str(), m.clock.as_str()),
            (35, "10", "cpu")
        );
        assert_eq!(m.lower_bound.as_str(), "350");
        let first = m.methods.row_data(0).unwrap();
        assert_eq!(first.name.as_str(), "PriceCalculator.simulatedTotal");
        assert_eq!((first.self_samples, first.total_samples), (20, 35));
        assert!((first.self_share - 20.0 / 35.0).abs() < 1e-6);
        assert!((first.total_share - 1.0).abs() < 1e-6);
        let none = sampled(fixture().requests.first());
        assert!(!none.has && none.methods.row_count() == 0);
        assert!(!sampled(None).has);
    }
}
