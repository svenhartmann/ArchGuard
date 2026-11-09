//! ADR 0004 step 1: runtime profile from OTLP JSON Lines (`fixtures/runtime/shop.jsonl`).
use archguard_engine::{core::RuntimeProfile, runtime_profile};
use std::fs;

const SHOP: &str = include_str!("../../../fixtures/runtime/shop.jsonl");

fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (path, text) in files {
        let p = dir.path().join(path);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }
    dir
}

fn shop() -> RuntimeProfile {
    let dir = project(&[(".archguard/runtime/run-1/traces-0.jsonl", SHOP)]);
    runtime_profile(dir.path()).expect("runtime input present")
}

fn endpoint<'a>(
    p: &'a RuntimeProfile,
    service: &str,
) -> &'a archguard_engine::core::RuntimeEndpoint {
    p.endpoints
        .iter()
        .find(|e| e.service == service && e.name == "GET /orders/{id}")
        .unwrap_or_else(|| panic!("endpoint of {service}: {:#?}", p.endpoints))
}

fn statement<'a>(
    p: &'a RuntimeProfile,
    prefix: &str,
) -> &'a archguard_engine::core::RuntimeStatement {
    p.statements
        .iter()
        .find(|s| s.key.starts_with(prefix))
        .unwrap_or_else(|| panic!("statement {prefix}: {:#?}", p.statements))
}

#[test]
fn no_runtime_input_means_no_profile() {
    let dir = project(&[("src/main.rs", "fn main() {}")]);
    assert!(runtime_profile(dir.path()).is_none());
}

#[test]
fn counts_spans_traces_and_reports_the_broken_line() {
    let p = shop();
    assert_eq!(p.inputs, vec![".archguard/runtime/run-1/traces-0.jsonl"]);
    assert_eq!((p.spans, p.traces), (14, 3));
    assert_eq!(p.sampling, vec!["always_on"]);
    assert!(
        !p.complete,
        "an unreadable line makes the profile incomplete"
    );
    let broken = p
        .diagnostics
        .iter()
        .find(|d| d.line == Some(2))
        .expect("diagnostic for line 2");
    assert_eq!(broken.file, ".archguard/runtime/run-1/traces-0.jsonl");
    assert!(p.note.contains("sample"), "{}", p.note);
}

#[test]
fn endpoints_count_requests_db_calls_and_db_time() {
    let p = shop();
    let orders = endpoint(&p, "orders");
    assert_eq!((orders.requests, orders.errors), (2, 0));
    // trace A: 1 order + 3 line queries (nested EF/provider pair counted once); trace B: 1 + 2.
    assert_eq!((orders.db_calls.mean, orders.db_calls.max), (3.5, 4.0));
    assert_eq!(
        (orders.duration_ms.p50, orders.duration_ms.max),
        (40.0, 70.0)
    );
    assert_eq!(orders.db_time_ms, 40.0);
    let web = endpoint(&p, "web");
    assert_eq!((web.requests, web.db_calls.max), (1, 0.0));
}

#[test]
fn statements_group_by_normalized_text_with_call_sites() {
    let p = shop();
    let order = statement(&p, "SELECT * FROM orders WHERE id = @p0");
    assert_eq!((order.calls, order.max_calls_per_request), (2, 1));
    assert_eq!(order.db_system.as_deref(), Some("postgresql"));
    assert_eq!(order.call_sites.len(), 1, "old and new code.* names agree");
    let site = &order.call_sites[0];
    assert_eq!(
        site.function.as_deref(),
        Some("Shop.Orders.OrderService.Load")
    );
    assert_eq!((site.line, site.calls), (Some(31), 2));

    let lines = statement(&p, "SELECT * FROM order_lines WHERE order_id = ?");
    assert_eq!(
        (lines.calls, lines.errors, lines.max_calls_per_request),
        (5, 1, 3)
    );
    assert_eq!(lines.exec_ms.max, 5.0);
    assert_eq!(lines.read_ms_total, Some(1.5));
    assert_eq!(lines.call_sites.len(), 1);
    assert_eq!(
        lines.call_sites[0].function.as_deref(),
        Some("Shop.Orders.OrderService.LoadLines")
    );
    assert_eq!(
        lines.call_sites[0].calls, 5,
        "the provider span inherits the EF span's call site"
    );
}

#[test]
fn db_calls_outside_a_request_and_nested_spans_are_reported() {
    let p = shop();
    let cleanup = statement(&p, "DELETE FROM carts");
    assert_eq!(cleanup.calls, 1);
    let messages: Vec<&str> = p.diagnostics.iter().map(|d| d.message.as_str()).collect();
    assert!(
        messages.iter().any(|m| m.contains("outside a request")),
        "{messages:#?}"
    );
    assert!(
        messages.iter().any(|m| m.contains("nested")),
        "{messages:#?}"
    );
}

#[test]
fn literal_values_never_reach_the_profile() {
    let json = serde_json::to_string(&shop()).unwrap();
    for secret in ["secret-user", "2026-01-01", "order_id = 7", "order_id = 8"] {
        assert!(!json.contains(secret), "{secret} leaked");
    }
}

#[test]
fn legacy_single_document_is_read_too() {
    let first = SHOP.lines().next().unwrap();
    let dir = project(&[(".archguard/otel-traces.json", first)]);
    let p = runtime_profile(dir.path()).unwrap();
    assert_eq!(p.inputs, vec![".archguard/otel-traces.json"]);
    assert_eq!(endpoint(&p, "orders").requests, 1);
    assert!(p.complete);
}

#[cfg(unix)]
#[test]
fn symlinked_inputs_are_not_followed() {
    let outside = project(&[("traces.jsonl", SHOP)]);
    let dir = project(&[("src/main.rs", "")]);
    let runtime = dir.path().join(".archguard/runtime");
    fs::create_dir_all(&runtime).unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("traces.jsonl"),
        runtime.join("link.jsonl"),
    )
    .unwrap();
    let p = runtime_profile(dir.path()).unwrap();
    assert_eq!(p.spans, 0);
    assert!(p.diagnostics.iter().any(|d| d.file.ends_with("link.jsonl")));
}
