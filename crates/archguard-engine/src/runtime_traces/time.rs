//! Where the time goes (ADR 0004): per endpoint, the mean self time of a
//! request's spans by category, and the methods with the most self time.
//! Built from the kept requests, so it covers at most the newest 200.
use archguard_core::{RuntimeBreakdown, RuntimeMethod, RuntimeRequest, RuntimeTimePart};
use std::collections::BTreeMap;

/// Categories in display order.
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
/// Hot methods kept.
const MAX_METHODS: usize = 50;

fn round(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// The category a span's self time counts for.
pub(super) fn category(db: bool, kind: &str, section: Option<&str>) -> &'static str {
    if db {
        return "database";
    }
    if matches!(kind, "client" | "producer") {
        return "outgoing";
    }
    match section {
        Some("controller") => "controller",
        Some("service") => "service",
        Some("repository") => "repository",
        Some("component") => "component",
        Some("method") => "method",
        _ => "other",
    }
}

/// Request time that no direct child span covers.
fn unattributed(r: &RuntimeRequest) -> f64 {
    let mut windows: Vec<(f64, f64)> = r
        .spans
        .iter()
        .filter(|s| s.depth == 1)
        .map(|s| {
            (
                s.offset_ms.max(0.0),
                (s.offset_ms + s.duration_ms).min(r.duration_ms),
            )
        })
        .filter(|(a, b)| b > a)
        .collect();
    windows.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (mut covered, mut reached) = (0.0, f64::MIN);
    for (a, b) in windows {
        if b > reached {
            covered += b - a.max(reached);
            reached = b;
        }
    }
    (r.duration_ms - covered).max(0.0)
}

/// Mean milliseconds per request by category, per endpoint.
pub(super) fn breakdown(requests: &[RuntimeRequest]) -> Vec<RuntimeBreakdown> {
    let mut groups: BTreeMap<(&str, &str), Vec<&RuntimeRequest>> = BTreeMap::new();
    for r in requests {
        groups.entry((&r.service, &r.name)).or_default().push(r);
    }
    groups
        .into_iter()
        .map(|((service, name), rs)| {
            let n = rs.len() as f64;
            let mut sums: BTreeMap<&str, f64> = BTreeMap::new();
            for r in &rs {
                *sums.entry("unattributed").or_default() += unattributed(r);
                for s in &r.spans {
                    *sums.entry(s.category.as_str()).or_default() += s.self_ms;
                }
            }
            let parts = CATEGORIES
                .iter()
                .filter_map(|c| sums.get(c).filter(|v| **v > 0.0).map(|v| (c, v)))
                .map(|(c, v)| RuntimeTimePart {
                    category: (*c).into(),
                    mean_ms: round(v / n),
                })
                .collect();
            RuntimeBreakdown {
                service: service.into(),
                name: name.into(),
                requests: rs.len() as u64,
                mean_ms: round(rs.iter().map(|r| r.duration_ms).sum::<f64>() / n),
                parts,
            }
        })
        .collect()
}

/// Method spans summed by service and name, most self time first.
pub(super) fn hot_methods(requests: &[RuntimeRequest]) -> Vec<RuntimeMethod> {
    let mut methods: BTreeMap<(&str, &str), RuntimeMethod> = BTreeMap::new();
    for r in requests {
        for s in r
            .spans
            .iter()
            .filter(|s| s.db.is_none() && s.kind == "internal")
        {
            let m = methods
                .entry((&r.service, &s.name))
                .or_insert_with(|| RuntimeMethod {
                    service: r.service.clone(),
                    name: s.name.clone(),
                    function: s.function.clone(),
                    kind: s.section.clone(),
                    calls: 0,
                    total_ms: 0.0,
                    self_ms: 0.0,
                });
            m.calls += 1;
            m.total_ms += s.duration_ms;
            m.self_ms += s.self_ms;
        }
    }
    let mut out: Vec<RuntimeMethod> = methods
        .into_values()
        .map(|mut m| {
            m.total_ms = round(m.total_ms);
            m.self_ms = round(m.self_ms);
            m
        })
        .collect();
    out.sort_by(|a, b| {
        b.self_ms
            .total_cmp(&a.self_ms)
            .then_with(|| a.name.cmp(&b.name))
    });
    out.truncate(MAX_METHODS);
    out
}
