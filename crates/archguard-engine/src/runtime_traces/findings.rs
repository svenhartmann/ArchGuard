//! Threshold findings of the runtime profile (`[runtime]` in archguard.toml).
//! A finding says that a value in the sample reached its threshold; it does
//! not say how often this happens in production.
use archguard_core::{
    RuntimeCallSite, RuntimeConfig, RuntimeFinding, RuntimeProfile, RuntimeStatement,
};

pub(super) fn findings(p: &RuntimeProfile, c: &RuntimeConfig) -> Vec<RuntimeFinding> {
    let mut out = n_plus_one(p, c);
    out.extend(chatty_endpoints(p, c));
    out.extend(slow_statements(p, c));
    out
}

fn by_value(mut found: Vec<RuntimeFinding>) -> Vec<RuntimeFinding> {
    found.sort_by(|a, b| {
        b.value
            .total_cmp(&a.value)
            .then_with(|| a.subject.cmp(&b.subject))
    });
    found
}

fn place(site: Option<&RuntimeCallSite>) -> String {
    match site {
        Some(s) => match (&s.function, &s.file, s.line) {
            (Some(f), Some(file), Some(line)) => format!("{f} ({file}:{line})"),
            (Some(f), _, _) => f.clone(),
            (None, Some(file), line) => format!("{file}:{}", line.unwrap_or(0)),
            _ => "an unknown call site".into(),
        },
        None => "an unknown call site (no code.* attributes recorded)".into(),
    }
}

fn repetition(
    s: &RuntimeStatement,
    site: Option<&RuntimeCallSite>,
    count: u64,
    k: u64,
) -> RuntimeFinding {
    RuntimeFinding {
        kind: "n_plus_one".into(),
        service: s.service.clone(),
        subject: s.key.clone(),
        value: count as f64,
        threshold: k as f64,
        call_site: site.cloned(),
        message: format!(
            "{count} calls of the same statement from {} within one request (threshold {k}): likely one query per item of a loop. Load the rows with one query (join, IN list or batch).",
            place(site)
        ),
        evidence: s.example.clone(),
    }
}

fn n_plus_one(p: &RuntimeProfile, c: &RuntimeConfig) -> Vec<RuntimeFinding> {
    let k = c.n_plus_one_min_calls;
    let mut found = Vec::new();
    for s in &p.statements {
        if s.call_sites.is_empty() {
            if s.max_calls_per_request >= k {
                found.push(repetition(s, None, s.max_calls_per_request, k));
            }
            continue;
        }
        for site in s
            .call_sites
            .iter()
            .filter(|site| site.max_calls_per_request >= k)
        {
            found.push(repetition(s, Some(site), site.max_calls_per_request, k));
        }
    }
    by_value(found)
}

fn chatty_endpoints(p: &RuntimeProfile, c: &RuntimeConfig) -> Vec<RuntimeFinding> {
    let found = p
        .endpoints
        .iter()
        .filter(|e| e.db_calls.p95 >= c.chatty_endpoint_db_calls)
        .map(|e| RuntimeFinding {
            kind: "chatty_endpoint".into(),
            service: e.service.clone(),
            subject: e.name.clone(),
            value: e.db_calls.p95,
            threshold: c.chatty_endpoint_db_calls,
            call_site: None,
            message: format!(
                "{} database calls per request at the 95th percentile over {} requests (threshold {}); max {}.",
                e.db_calls.p95, e.requests, c.chatty_endpoint_db_calls, e.db_calls.max
            ),
            evidence: e.example.clone(),
        })
        .collect();
    by_value(found)
}

fn slow_statements(p: &RuntimeProfile, c: &RuntimeConfig) -> Vec<RuntimeFinding> {
    let found = p
        .statements
        .iter()
        .filter(|s| s.exec_ms.p95 >= c.slow_statement_ms)
        .map(|s| RuntimeFinding {
            kind: "slow_statement".into(),
            service: s.service.clone(),
            subject: s.key.clone(),
            value: s.exec_ms.p95,
            threshold: c.slow_statement_ms,
            call_site: s.call_sites.first().cloned(),
            message: format!(
                "{} ms at the 95th percentile over {} calls (threshold {} ms); client-side time including network and pool wait.",
                s.exec_ms.p95, s.calls, c.slow_statement_ms
            ),
            evidence: s.example.clone(),
        })
        .collect();
    by_value(found)
}
