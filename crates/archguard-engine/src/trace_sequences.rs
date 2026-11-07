//! Request sequences from an OTLP trace export (ADR 0006): the matched
//! service-to-service span pairs of one trace in start order. Only bounded,
//! low-cardinality attributes name an operation (HTTP route, RPC method);
//! URLs, queries, span names and message destinations are never copied.

use archguard_core::{ServiceDependencyKind as Kind, TraceSequence, TraceStep};
use std::collections::BTreeMap;

/// Traces kept for diagrams: the ones with most steps, then the earliest.
const MAX_SEQUENCES: usize = 5;
const MAX_STEPS: usize = 40;
const MAX_OPERATION: usize = 120;

/// One matched pair, as the trace import correlates it.
pub(crate) struct Step {
    pub trace: String,
    pub from: String,
    pub to: String,
    pub kind: Kind,
    pub start: u64,
    pub end: u64,
    pub error: bool,
    pub operation: Option<String>,
}

/// What a span names as its operation, when the value is safe to show;
/// `attribute` reads one string attribute of the span.
pub(crate) fn operation(attribute: impl Fn(&str) -> Option<String>) -> Option<String> {
    let text = |key: &str| {
        attribute(key)
            .filter(|v| !v.is_empty() && v.len() <= MAX_OPERATION && !v.contains(['?', '#', '@']))
    };
    let method = text("http.request.method").or_else(|| text("http.method"));
    if let Some(route) = text("http.route") {
        return Some(method.map_or(route.clone(), |m| format!("{m} {route}")));
    }
    let (service, rpc) = (text("rpc.service")?, text("rpc.method")?);
    Some(format!("{service}/{rpc}"))
}

pub(crate) fn sequences(steps: Vec<Step>) -> Vec<TraceSequence> {
    let mut by_trace: BTreeMap<String, Vec<Step>> = BTreeMap::new();
    for s in steps {
        by_trace.entry(s.trace.clone()).or_default().push(s);
    }
    let mut out: Vec<TraceSequence> = by_trace
        .into_iter()
        .map(|(trace, mut steps)| {
            steps.sort_by(|a, b| (a.start, &a.from, &a.to).cmp(&(b.start, &b.from, &b.to)));
            steps.truncate(MAX_STEPS);
            sequence(trace, &steps)
        })
        .collect();
    out.sort_by(|a, b| {
        (b.steps.len(), a.start_unix_ns, &a.trace).cmp(&(a.steps.len(), b.start_unix_ns, &b.trace))
    });
    out.truncate(MAX_SEQUENCES);
    out
}

fn sequence(trace: String, steps: &[Step]) -> TraceSequence {
    let start = steps.iter().map(|s| s.start).min().unwrap_or(0);
    let end = steps.iter().map(|s| s.end).max().unwrap_or(start);
    TraceSequence {
        trace,
        start_unix_ns: start,
        duration_us: (end - start) / 1000,
        steps: steps
            .iter()
            .map(|s| TraceStep {
                from: s.from.clone(),
                to: s.to.clone(),
                kind: s.kind,
                offset_us: (s.start - start) / 1000,
                duration_us: (s.end - s.start) / 1000,
                error: s.error,
                operation: s.operation.clone(),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(trace: &str, from: &str, start: u64) -> Step {
        Step {
            trace: trace.into(),
            from: from.into(),
            to: "b".into(),
            kind: Kind::HttpCall,
            start,
            end: start + 2_000_000,
            error: false,
            operation: None,
        }
    }

    #[test]
    fn steps_are_ordered_per_trace_and_the_longest_traces_are_kept() {
        let mut steps = vec![step("t1", "c", 5_000_000), step("t1", "a", 1_000_000)];
        steps.extend((0..6).map(|i| step(&format!("x{i}"), "a", 1)));
        let out = sequences(steps);
        assert_eq!(out.len(), MAX_SEQUENCES);
        let t1 = &out[0];
        assert_eq!(t1.trace, "t1");
        assert_eq!(
            (t1.steps[0].from.as_str(), t1.steps[1].offset_us),
            ("a", 4000)
        );
        assert_eq!(t1.duration_us, 6000);
    }

    #[test]
    fn only_safe_attributes_name_an_operation() {
        let of = |pairs: &'static [(&str, &str)]| {
            operation(move |key| {
                pairs
                    .iter()
                    .find(|(k, _)| *k == key)
                    .map(|(_, v)| v.to_string())
            })
        };
        assert_eq!(
            of(&[
                ("http.request.method", "GET"),
                ("http.route", "/orders/{id}")
            ])
            .as_deref(),
            Some("GET /orders/{id}")
        );
        assert_eq!(
            of(&[
                ("url.full", "https://u:p@h/x?token=1"),
                ("http.route", "/x?token=1")
            ]),
            None
        );
        assert_eq!(
            of(&[("rpc.service", "Shop"), ("rpc.method", "Buy")]).as_deref(),
            Some("Shop/Buy")
        );
    }
}
