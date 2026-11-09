//! Single requests with their span tree (ADR 0004, step 2b): what one REST
//! call did inside its application. Spans of other services belong to their
//! own requests; a database span wrapping a driver span is folded into it.
use super::index::{Index, evidence, ms};
use super::span::Span;
use archguard_core::{RuntimeRequest, RuntimeSpan};

/// Requests kept, newest first; `requests_total` counts all.
const MAX_REQUESTS: usize = 200;
/// Spans kept per request; counts cover all of them.
const MAX_SPANS: usize = 500;
/// Nesting followed below a request, against parent cycles in bad input.
const MAX_DEPTH: u32 = 64;

fn round(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

fn kind(k: u8) -> &'static str {
    match k {
        1 => "internal",
        2 => "server",
        3 => "client",
        4 => "producer",
        5 => "consumer",
        _ => "unspecified",
    }
}

/// The newest requests and the number of all requests.
pub(super) fn requests(index: &Index, inputs: &[String]) -> (Vec<RuntimeRequest>, u64) {
    let mut entries: Vec<usize> = (0..index.spans.len())
        .filter(|&i| index.spans[i].entry.is_some())
        .collect();
    let total = entries.len() as u64;
    entries.sort_by(|&a, &b| {
        let (x, y) = (&index.spans[a], &index.spans[b]);
        y.start.cmp(&x.start).then(a.cmp(&b))
    });
    entries.truncate(MAX_REQUESTS);
    let requests = entries
        .into_iter()
        .map(|e| request(index, inputs, e))
        .collect();
    (requests, total)
}

/// Spans collected below one request, with totals over all of them.
#[derive(Default)]
struct Walk {
    spans: Vec<RuntimeSpan>,
    truncated: bool,
    db_calls: u64,
    db_time: f64,
}

fn request(index: &Index, inputs: &[String], e: usize) -> RuntimeRequest {
    let root = &index.spans[e];
    let mut walk = Walk::default();
    descend(index, root, e, 1, &mut walk);
    RuntimeRequest {
        service: root.service.clone(),
        name: root.entry.clone().unwrap_or_default(),
        trace_id: root.trace.clone(),
        span_id: root.id.clone(),
        start_unix_ns: root.start,
        duration_ms: round(ms(root)),
        error: root.error,
        db_calls: walk.db_calls,
        db_time_ms: round(walk.db_time),
        spans: walk.spans,
        spans_truncated: walk.truncated,
        evidence: evidence(index.spans, inputs, e),
    }
}

/// Children of `parent` in start order, depth first. Other services and
/// nested requests are left out; wrappers are skipped but descended into.
fn descend(index: &Index, root: &Span, parent: usize, depth: u32, walk: &mut Walk) {
    if depth > MAX_DEPTH {
        walk.truncated = true;
        return;
    }
    let mut children = index.children(parent).to_vec();
    children.sort_by_key(|&c| (index.spans[c].start, c));
    for c in children {
        let span = &index.spans[c];
        if span.service != root.service || span.entry.is_some() {
            continue;
        }
        if index.wrappers.contains(&c) {
            descend(index, root, c, depth, walk);
            continue;
        }
        if span.db.is_some() {
            walk.db_calls += 1;
            walk.db_time += ms(span);
        }
        if walk.spans.len() < MAX_SPANS {
            walk.spans.push(node(index, root, span, depth));
        } else {
            walk.truncated = true;
        }
        descend(index, root, c, depth + 1, walk);
    }
}

fn node(index: &Index, root: &Span, span: &Span, depth: u32) -> RuntimeSpan {
    let site = if span.db.is_some() {
        index.site(span)
    } else {
        span.site.clone()
    };
    let (function, file, line) = site.map_or((None, None, None), |s| (s.function, s.file, s.line));
    RuntimeSpan {
        depth,
        offset_ms: round(span.start.saturating_sub(root.start) as f64 / 1_000_000.0),
        duration_ms: round(ms(span)),
        kind: kind(span.kind).into(),
        name: span.name.clone(),
        error: span.error,
        db: span.db.as_ref().map(|d| d.key.clone()),
        function,
        file,
        line,
    }
}
