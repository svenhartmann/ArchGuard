//! The request list and the selected request's timeline as Slint values.
use std::collections::HashMap;

use archguard_engine::core::{RuntimeRequest, RuntimeSpan};
use slint::{ModelRc, SharedString, VecModel};

use super::summary::{file_name, number, short_function};
use crate::ui::{RtDetail, RtRequest, RtSpanRow};

/// `HH:MM:SS.mmm` of a Unix time in nanoseconds, UTC.
pub fn utc_time(unix_ns: u64) -> String {
    let ms = unix_ns / 1_000_000;
    let day = (ms / 1000) % 86_400;
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        day / 3600,
        (day / 60) % 60,
        day % 60,
        ms % 1000
    )
}

/// A database call's identity within one request: statement and call site.
type CallKey<'a> = (&'a str, Option<&'a str>, Option<u32>);

fn call_key(s: &RuntimeSpan) -> Option<CallKey<'_>> {
    Some((s.db.as_deref()?, s.function.as_deref(), s.line))
}

/// Calls per (statement, call site) within the request.
fn repeats(r: &RuntimeRequest) -> HashMap<CallKey<'_>, u64> {
    let mut counts = HashMap::new();
    for key in r.spans.iter().filter_map(call_key) {
        *counts.entry(key).or_insert(0) += 1;
    }
    counts
}

pub fn request(r: &RuntimeRequest, threshold: u64, pressed: bool) -> RtRequest {
    let repeated = repeats(r).into_values().max().unwrap_or(0);
    RtRequest {
        time: utc_time(r.start_unix_ns).into(),
        service: r.service.as_str().into(),
        name: r.name.as_str().into(),
        duration: number(r.duration_ms),
        db_calls: i32::try_from(r.db_calls).unwrap_or(i32::MAX),
        repeated: i32::try_from(repeated).unwrap_or(i32::MAX),
        flagged: repeated >= threshold,
        error: r.error,
        pressed,
    }
}

fn site(s: &RuntimeSpan) -> String {
    let function = s.function.as_deref().map(short_function);
    match (function, s.file.as_deref(), s.line) {
        (Some(f), Some(file), Some(line)) => format!("{f} ({}:{line})", file_name(file)),
        (Some(f), _, _) => f,
        (None, Some(file), Some(line)) => format!("{}:{line}", file_name(file)),
        _ => String::new(),
    }
}

/// Time of the request not covered by its direct child spans: the
/// application's own code and waiting that no span recorded.
pub fn own_ms(r: &RuntimeRequest) -> f64 {
    let mut spans: Vec<(f64, f64)> = r
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
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (mut covered, mut end) = (0.0, 0.0_f64);
    for (a, b) in spans {
        if b > end {
            covered += b - a.max(end);
            end = b;
        }
    }
    (r.duration_ms - covered).max(0.0)
}

/// A method span names its caller and the calling line; any other span the
/// code that issued it (its own call site).
fn place(s: &RuntimeSpan) -> (String, bool) {
    match (s.db.is_none(), s.caller.as_deref()) {
        (true, Some(caller)) => {
            let caller = short_function(caller);
            let at = match (s.file.as_deref(), s.line) {
                (Some(file), Some(line)) => format!(" ({}:{line})", file_name(file)),
                _ => String::new(),
            };
            (format!("{caller}{at}"), true)
        }
        // A method span without a recorded caller (one the framework called)
        // would only repeat its own name.
        (true, None)
            if s.function.as_deref().map(short_function).as_deref() == Some(s.name.as_str()) =>
        {
            (String::new(), false)
        }
        _ => (site(s), false),
    }
}

fn span_row(s: &RuntimeSpan, total: f64, repeated: bool) -> RtSpanRow {
    let share = |v: f64| if total > 0.0 { (v / total) as f32 } else { 0.0 };
    let (site, called_from) = place(s);
    let kind = if s.db.is_some() {
        0
    } else if matches!(s.kind.as_str(), "client" | "producer") {
        1
    } else {
        2
    };
    RtSpanRow {
        depth: i32::try_from(s.depth).unwrap_or(i32::MAX),
        label: s.db.as_deref().unwrap_or(&s.name).into(),
        site: site.into(),
        called_from,
        self_time: number(s.self_ms),
        has_self: s.self_ms + 0.0005 < s.duration_ms,
        x: share(s.offset_ms),
        w: share(s.duration_ms),
        duration: number(s.duration_ms),
        kind,
        error: s.error,
        repeated,
    }
}

pub fn detail(r: &RuntimeRequest, threshold: u64) -> RtDetail {
    let counts = repeats(r);
    let (flame, flame_depth) = super::time::flame(r);
    let rows: Vec<RtSpanRow> = r
        .spans
        .iter()
        .map(|s| {
            let repeated = call_key(s).is_some_and(|k| counts[&k] >= threshold);
            span_row(s, r.duration_ms, repeated)
        })
        .collect();
    RtDetail {
        has: true,
        title: format!("{} · {}", r.service, r.name).into(),
        time: utc_time(r.start_unix_ns).into(),
        duration: number(r.duration_ms),
        db_calls: i32::try_from(r.db_calls).unwrap_or(i32::MAX),
        db_time: number(r.db_time_ms),
        own: number(own_ms(r)),
        threshold: i32::try_from(threshold).unwrap_or(i32::MAX),
        trace: SharedString::from(r.trace_id.as_str()),
        truncated: r.spans_truncated,
        spans: ModelRc::new(VecModel::from(rows)),
        flame,
        flame_depth,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(depth: u32, offset: f64, duration: f64, db: Option<&str>, line: u32) -> RuntimeSpan {
        RuntimeSpan {
            depth,
            offset_ms: offset,
            duration_ms: duration,
            self_ms: duration,
            kind: "client".into(),
            name: "q".into(),
            error: false,
            db: db.map(Into::into),
            function: Some("Shop.Orders.OrderService.LoadLines".into()),
            file: Some("/_/src/Orders/OrderService.cs".into()),
            line: Some(line),
            caller: None,
            section: None,
            category: if db.is_some() { "database" } else { "other" }.into(),
        }
    }

    fn request_of(spans: Vec<RuntimeSpan>) -> RuntimeRequest {
        RuntimeRequest {
            service: "orders".into(),
            name: "GET /orders/{id}".into(),
            trace_id: "a".repeat(32),
            span_id: "1".repeat(16),
            start_unix_ns: 1_700_000_045_123_000_000,
            duration_ms: 70.0,
            error: false,
            db_calls: spans.len() as u64,
            db_time_ms: spans.iter().map(|s| s.duration_ms).sum(),
            spans,
            spans_truncated: false,
            evidence: Default::default(),
        }
    }

    #[test]
    fn utc_time_of_day_with_milliseconds() {
        assert_eq!(utc_time(1_700_000_045_123_000_000), "22:14:05.123");
    }

    #[test]
    fn own_time_is_the_part_no_child_span_covers() {
        let r = request_of(vec![
            span(1, 5.0, 10.0, Some("a"), 31),
            span(1, 10.0, 10.0, Some("b"), 42),
            span(2, 11.0, 2.0, None, 1),
            span(1, 60.0, 20.0, Some("c"), 50),
        ]);
        // Covered: 5..20 and 60..70 (clipped to the request) = 25 ms.
        assert_eq!(own_ms(&r), 45.0);
    }

    #[test]
    fn repeated_statements_from_one_site_are_flagged() {
        let three = (0..3).map(|i| span(1, 10.0 + i as f64 * 6.0, 5.0, Some("lines"), 42));
        let r = request_of(
            std::iter::once(span(1, 5.0, 4.0, Some("order"), 31))
                .chain(three)
                .collect(),
        );
        let row = request(&r, 3, false);
        assert_eq!((row.repeated, row.flagged), (3, true));
        assert!(!request(&r, 4, false).flagged);
        let d = detail(&r, 3);
        let rows: Vec<bool> = (0..4)
            .map(|i| slint::Model::row_data(&d.spans, i).unwrap().repeated)
            .collect();
        assert_eq!(rows, vec![false, true, true, true]);
        let first = slint::Model::row_data(&d.spans, 0).unwrap();
        assert_eq!(first.site, "OrderService.LoadLines (OrderService.cs:31)");
        assert!((first.x - 5.0 / 70.0).abs() < 1e-6);
    }

    #[test]
    fn method_spans_show_their_caller_and_self_time() {
        let mut service = span(1, 0.0, 40.0, None, 16);
        service.kind = "internal".into();
        service.name = "OrderService.GetSummaryAsync".into();
        service.function = Some("Shop.Orders.OrderService.GetSummaryAsync".into());
        service.file = Some("Orders/OrdersController.cs".into());
        service.caller = Some("Shop.Orders.OrdersController.GetSummary".into());
        service.self_ms = 4.5;
        let row = span_row(&service, 70.0, false);
        assert!(row.called_from);
        assert_eq!(
            row.site,
            "OrdersController.GetSummary (OrdersController.cs:16)"
        );
        assert_eq!((row.self_time.as_str(), row.has_self), ("4.5", true));
        let db = span(2, 1.0, 5.0, Some("SELECT 1"), 31);
        let row = span_row(&db, 70.0, false);
        assert!(!row.called_from && !row.has_self);
        let mut controller = span(1, 0.0, 40.0, None, 9);
        controller.kind = "internal".into();
        controller.name = "OrdersController.GetSummary".into();
        controller.function = Some("Shop.Orders.OrdersController.GetSummary".into());
        assert_eq!(
            span_row(&controller, 70.0, false).site,
            "",
            "no caller, no repeated name"
        );
        assert_eq!(row.site, "OrderService.LoadLines (OrderService.cs:31)");
    }
}
