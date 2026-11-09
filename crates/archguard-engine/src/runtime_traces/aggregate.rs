//! Spans → endpoints and statements. A database call belongs to the nearest
//! SERVER or CONSUMER span of the same service above it. When a database span
//! wraps another one (an ORM span around the driver span), only the innermost
//! is counted; it inherits the call site of the wrapping spans.
use super::index::{Index, evidence, ms};
use super::span::{Site, Span};
use archguard_core::{
    RuntimeCallSite, RuntimeDiagnostic, RuntimeEndpoint, RuntimeEvidence, RuntimeProfile,
    RuntimeStatement, RuntimeStats,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};

#[derive(Default)]
struct EndpointAcc {
    durations: Vec<f64>,
    db_calls: Vec<f64>,
    db_time: f64,
    errors: u64,
    example: Option<usize>,
}

/// A request: the entry span, or the trace for calls outside a request.
type Request = (usize, String);

/// Calls from one call site: first recorded source, total, per request.
#[derive(Default)]
struct SiteAcc {
    source: Option<String>,
    calls: u64,
    per_request: HashMap<Request, u64>,
}

/// Call site identity (function, file, line) → its calls.
type Sites = BTreeMap<(Option<String>, Option<String>, Option<u32>), SiteAcc>;

#[derive(Default)]
struct StatementAcc {
    text: Option<String>,
    durations: Vec<f64>,
    errors: u64,
    read: Option<f64>,
    per_request: HashMap<Request, u64>,
    sites: Sites,
    example: Option<usize>,
}

fn stats(mut values: Vec<f64>) -> RuntimeStats {
    if values.is_empty() {
        return RuntimeStats::default();
    }
    values.sort_by(f64::total_cmp);
    let rank =
        |q: f64| values[((q * values.len() as f64).ceil() as usize).clamp(1, values.len()) - 1];
    let round = |v: f64| (v * 1000.0).round() / 1000.0;
    RuntimeStats {
        mean: round(values.iter().sum::<f64>() / values.len() as f64),
        p50: round(rank(0.5)),
        p95: round(rank(0.95)),
        max: round(values[values.len() - 1]),
    }
}

type StatementKey = (String, Option<String>, String);

/// Database calls per statement, per entry span (calls, time) and outside requests.
#[derive(Default)]
struct DbCalls {
    statements: BTreeMap<StatementKey, StatementAcc>,
    per_entry: HashMap<usize, (u64, f64)>,
    outside: usize,
    first_outside: Option<usize>,
}

impl StatementAcc {
    fn add(&mut self, i: usize, span: &Span, entry: Option<usize>, site: Option<Site>) {
        let Some(db) = &span.db else { return };
        self.text = self.text.take().or_else(|| db.text.clone());
        self.durations.push(ms(span));
        self.errors += u64::from(span.error);
        if let Some(r) = db.read_ms {
            self.read = Some(self.read.unwrap_or(0.0) + r);
        }
        let request =
            entry.map_or_else(|| (usize::MAX, span.trace.clone()), |e| (e, String::new()));
        *self.per_request.entry(request.clone()).or_default() += 1;
        if let Some(site) = site {
            let slot = self
                .sites
                .entry((site.function, site.file, site.line))
                .or_default();
            slot.source = slot.source.take().or(site.source);
            slot.calls += 1;
            *slot.per_request.entry(request).or_default() += 1;
        }
        self.example.get_or_insert(i);
    }
}

fn db_calls(index: &Index) -> DbCalls {
    let mut out = DbCalls::default();
    for (i, span) in index.spans.iter().enumerate() {
        let Some(db) = &span.db else { continue };
        if index.wrappers.contains(&i) {
            continue;
        }
        let entry = index.entry(span);
        match entry {
            Some(e) => {
                let slot = out.per_entry.entry(e).or_default();
                slot.0 += 1;
                slot.1 += ms(span);
            }
            None => {
                out.outside += 1;
                out.first_outside.get_or_insert(i);
            }
        }
        let key = (span.service.clone(), db.system.clone(), db.key.clone());
        out.statements
            .entry(key)
            .or_default()
            .add(i, span, entry, index.site(span));
    }
    out
}

fn endpoints(
    spans: &[Span],
    per_entry: &HashMap<usize, (u64, f64)>,
    inputs: &[String],
) -> Vec<RuntimeEndpoint> {
    let mut endpoints: BTreeMap<(String, String), EndpointAcc> = BTreeMap::new();
    for (i, span) in spans.iter().enumerate() {
        let Some(name) = &span.entry else { continue };
        let acc = endpoints
            .entry((span.service.clone(), name.clone()))
            .or_default();
        let (calls, time) = per_entry.get(&i).copied().unwrap_or_default();
        acc.durations.push(ms(span));
        acc.db_calls.push(calls as f64);
        acc.db_time += time;
        acc.errors += u64::from(span.error);
        acc.example.get_or_insert(i);
    }
    endpoints
        .into_iter()
        .map(|((service, name), acc)| RuntimeEndpoint {
            service,
            name,
            requests: acc.durations.len() as u64,
            errors: acc.errors,
            duration_ms: stats(acc.durations),
            db_calls: stats(acc.db_calls),
            db_time_ms: (acc.db_time * 1000.0).round() / 1000.0,
            example: evidence(spans, inputs, acc.example.unwrap_or_default()),
        })
        .collect()
}

/// Fills endpoints, statements, counts and diagnostics of `profile` from `spans`.
pub(super) fn fill(spans: &[Span], profile: &mut RuntimeProfile) {
    let index = Index::new(spans);
    let calls = db_calls(&index);
    let inputs = profile.inputs.clone();
    profile.endpoints = endpoints(spans, &calls.per_entry, &inputs);
    profile.statements = calls
        .statements
        .into_iter()
        .map(|(key, acc)| statement(key, acc, spans, &inputs))
        .collect();
    (profile.requests, profile.requests_total) = super::requests::requests(&index, &inputs);
    counts(spans, profile);
    let example = calls.first_outside.map(|i| evidence(spans, &inputs, i));
    notes(&index, calls.outside, example, profile);
}

fn statement(
    (service, db_system, key): StatementKey,
    acc: StatementAcc,
    spans: &[Span],
    inputs: &[String],
) -> RuntimeStatement {
    let mut call_sites: Vec<RuntimeCallSite> = acc
        .sites
        .into_iter()
        .map(|((function, file, line), site)| RuntimeCallSite {
            function,
            file,
            line,
            source: site.source,
            calls: site.calls,
            max_calls_per_request: site.per_request.into_values().max().unwrap_or(0),
        })
        .collect();
    call_sites.sort_by(|a, b| {
        b.calls
            .cmp(&a.calls)
            .then_with(|| a.function.cmp(&b.function))
    });
    RuntimeStatement {
        service,
        db_system,
        key,
        text: acc.text,
        calls: acc.durations.len() as u64,
        errors: acc.errors,
        exec_ms: stats(acc.durations),
        read_ms_total: acc.read.map(|r| (r * 1000.0).round() / 1000.0),
        max_calls_per_request: acc.per_request.values().copied().max().unwrap_or(0),
        call_sites,
        example: evidence(spans, inputs, acc.example.unwrap_or_default()),
    }
}

fn counts(spans: &[Span], profile: &mut RuntimeProfile) {
    profile.spans = spans.len() as u64;
    profile.traces = spans
        .iter()
        .map(|s| s.trace.as_str())
        .collect::<BTreeSet<_>>()
        .len() as u64;
    let start = spans.iter().map(|s| s.start).min();
    let end = spans.iter().map(|s| s.end).max();
    profile.window_unix_ns = start.zip(end).map(|(s, e)| [s, e]);
}

fn notes(
    index: &Index,
    outside: usize,
    example: Option<RuntimeEvidence>,
    profile: &mut RuntimeProfile,
) {
    let note = |message: String, example: Option<&RuntimeEvidence>| RuntimeDiagnostic {
        file: example.map(|e| e.file.clone()).unwrap_or_default(),
        line: example.map(|e| e.line),
        message,
    };
    if !index.wrappers.is_empty() {
        let first = index
            .wrappers
            .iter()
            .min()
            .map(|i| evidence(index.spans, &profile.inputs, *i));
        profile.diagnostics.push(note(
            format!(
                "{} database spans are nested around another database span (for example an ORM span around the driver span); only the innermost span is counted.",
                index.wrappers.len()
            ),
            first.as_ref(),
        ));
    }
    if outside > 0 {
        profile.diagnostics.push(note(
            format!(
                "{outside} database calls ran outside a request (no SERVER or CONSUMER span of the same service above them, for example background jobs or lost context); they count for statements, not for endpoints."
            ),
            example.as_ref(),
        ));
    }
}
