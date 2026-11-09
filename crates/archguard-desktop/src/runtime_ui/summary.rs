//! The Runtime tab's summary: the runtime profile (ADR 0004) over all
//! requests as one [`RtSummary`] value. Numbers are formatted here, labels live in
//! `ui/runtime_summary.slint`.

use std::rc::Rc;

use archguard_engine::core::{
    RuntimeCallSite, RuntimeEndpoint, RuntimeFinding, RuntimeProfile, RuntimeStatement,
};
use slint::{ModelRc, SharedString, VecModel};

use crate::ui::{RtEndpoint, RtFinding, RtSite, RtStatement, RtSummary};

/// Rows per list; the CLI (`archguard runtime --format json`) has all.
const MAX_ROWS: usize = 50;

fn list<T: Clone + 'static>(rows: impl Iterator<Item = T>) -> ModelRc<T> {
    ModelRc::from(Rc::new(VecModel::from(
        rows.take(MAX_ROWS).collect::<Vec<_>>(),
    )))
}

/// `12.5`, `40`: one decimal only where it carries information.
pub(super) fn number(v: f64) -> SharedString {
    if (v - v.round()).abs() < 0.05 {
        format!("{v:.0}").into()
    } else {
        format!("{v:.1}").into()
    }
}

fn place(site: &RuntimeCallSite) -> String {
    let function = site.function.as_deref().map(short_function);
    match (function, &site.file, site.line) {
        (Some(f), Some(file), Some(line)) => format!("{f} ({}:{line})", file_name(file)),
        (Some(f), _, _) => f,
        (None, Some(file), Some(line)) => format!("{}:{line}", file_name(file)),
        (None, Some(file), None) => file_name(file).to_string(),
        (None, None, _) => String::new(),
    }
}

/// `Shop.Orders.OrderService.LoadLines` → `OrderService.LoadLines`.
pub(super) fn short_function(f: &str) -> String {
    let parts: Vec<&str> = f.rsplitn(3, '.').collect();
    match parts.as_slice() {
        [method, class, _] => format!("{class}.{method}"),
        _ => f.to_string(),
    }
}

pub(super) fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn finding(f: &RuntimeFinding) -> RtFinding {
    RtFinding {
        kind: match f.kind.as_str() {
            "n_plus_one" => 0,
            "chatty_endpoint" => 1,
            _ => 2,
        },
        service: f.service.as_str().into(),
        subject: f.subject.as_str().into(),
        value: number(f.value),
        threshold: number(f.threshold),
        site: f.call_site.as_ref().map(place).unwrap_or_default().into(),
    }
}

fn endpoint(e: &RuntimeEndpoint) -> RtEndpoint {
    RtEndpoint {
        service: e.service.as_str().into(),
        name: e.name.as_str().into(),
        requests: i32::try_from(e.requests).unwrap_or(i32::MAX),
        errors: i32::try_from(e.errors).unwrap_or(i32::MAX),
        p50: number(e.duration_ms.p50),
        p95: number(e.duration_ms.p95),
        db_mean: number(e.db_calls.mean),
        db_max: number(e.db_calls.max),
        db_time: number(e.db_time_ms),
    }
}

fn statement(s: &RuntimeStatement) -> RtStatement {
    RtStatement {
        service: s.service.as_str().into(),
        system: s.db_system.as_deref().unwrap_or_default().into(),
        text: s.text.as_deref().unwrap_or(&s.key).into(),
        calls: i32::try_from(s.calls).unwrap_or(i32::MAX),
        errors: i32::try_from(s.errors).unwrap_or(i32::MAX),
        max_per_request: i32::try_from(s.max_calls_per_request).unwrap_or(i32::MAX),
        mean: number(s.exec_ms.mean),
        max: number(s.exec_ms.max),
        read: s.read_ms_total.map(number).unwrap_or_default(),
        sites: list(s.call_sites.iter().map(|c| RtSite {
            place: place(c).into(),
            calls: i32::try_from(c.calls).unwrap_or(i32::MAX),
        })),
    }
}

fn total_ms(s: &RuntimeStatement) -> f64 {
    s.exec_ms.mean * s.calls as f64
}

/// The view's model; `available` is false without recorded runtime input.
pub fn model(profile: Option<&RuntimeProfile>) -> RtSummary {
    let Some(p) = profile else {
        return RtSummary::default();
    };
    let mut endpoints: Vec<_> = p.endpoints.iter().collect();
    endpoints.sort_by(|a, b| {
        b.db_time_ms
            .total_cmp(&a.db_time_ms)
            .then(b.requests.cmp(&a.requests))
    });
    let mut statements: Vec<_> = p.statements.iter().collect();
    statements.sort_by(|a, b| {
        total_ms(b)
            .total_cmp(&total_ms(a))
            .then(b.calls.cmp(&a.calls))
    });
    RtSummary {
        available: true,
        spans: i32::try_from(p.spans).unwrap_or(i32::MAX),
        traces: i32::try_from(p.traces).unwrap_or(i32::MAX),
        sampling: p.sampling.join(", ").into(),
        complete: p.complete,
        findings: list(p.findings.iter().map(finding)),
        endpoints: list(endpoints.into_iter().map(endpoint)),
        statements: list(statements.into_iter().map(statement)),
        diagnostics: list(p.diagnostics.iter().map(|d| {
            let at = match d.line {
                Some(line) => format!("{} line {line}: ", d.file),
                None if d.file.is_empty() => String::new(),
                None => format!("{}: ", d.file),
            };
            SharedString::from(format!("{at}{}", d.message))
        })),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use slint::Model;

    use super::*;

    fn shop(config: &str) -> RuntimeProfile {
        let dir = tempfile::tempdir().unwrap();
        let run = dir.path().join(".archguard/runtime/run-1");
        fs::create_dir_all(&run).unwrap();
        let fixture = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/runtime/shop.jsonl"
        );
        fs::copy(fixture, run.join("traces-0.jsonl")).unwrap();
        fs::write(dir.path().join("archguard.toml"), config).unwrap();
        archguard_engine::runtime_profile(dir.path(), None).unwrap()
    }

    #[test]
    fn without_input_the_view_says_so() {
        assert!(!model(None).available);
    }

    #[test]
    fn statements_by_db_time_with_short_call_sites() {
        let m = model(Some(&shop("")));
        assert!(m.available);
        assert_eq!((m.spans, m.traces, m.complete), (14, 3, false));
        let first = m.statements.row_data(0).unwrap();
        assert_eq!(first.text, "SELECT * FROM order_lines WHERE order_id = ?");
        assert_eq!((first.calls, first.max_per_request), (5, 3));
        assert_eq!(first.read, "1.5");
        let site = first.sites.row_data(0).unwrap();
        assert_eq!(site.place, "OrderService.LoadLines (OrderService.cs:42)");
        let endpoint = m.endpoints.row_data(0).unwrap();
        assert_eq!(
            (endpoint.service.as_str(), endpoint.p95.as_str()),
            ("orders", "70")
        );
        assert_eq!(endpoint.db_mean, "3.5");
        assert_eq!(m.findings.row_count(), 0);
        assert_eq!(m.diagnostics.row_count(), 3);
    }

    #[test]
    fn findings_carry_kind_values_and_call_site() {
        let m = model(Some(&shop(
            "[runtime]\nn_plus_one_min_calls = 3\nslow_statement_ms = 20\n",
        )));
        let kinds: Vec<i32> = m.findings.iter().map(|f| f.kind).collect();
        assert_eq!(kinds, vec![0, 2]);
        let n1 = m.findings.row_data(0).unwrap();
        assert_eq!((n1.value.as_str(), n1.threshold.as_str()), ("3", "3"));
        assert_eq!(n1.site, "OrderService.LoadLines (OrderService.cs:42)");
    }
}
