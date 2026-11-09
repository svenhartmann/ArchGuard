//! `archguard runtime` (ADR 0004): runtime profile from `.archguard/runtime/**/*.jsonl`.
use serde_json::Value;
use std::{fs, process::Command};

const SHOP: &str = include_str!("../../../fixtures/runtime/shop.jsonl");

fn project(with_runtime: bool) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    // Runtime import is independent of the import-analysis configuration.
    fs::write(
        dir.path().join("archguard.toml"),
        "not a valid TOML document = [",
    )
    .unwrap();
    if with_runtime {
        let run = dir.path().join(".archguard/runtime/run-1");
        fs::create_dir_all(&run).unwrap();
        fs::write(run.join("traces-0.jsonl"), SHOP).unwrap();
    }
    dir
}

fn archguard(dir: &tempfile::TempDir, format: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_archguard"))
        .args(["runtime", "--format", format])
        .arg(dir.path())
        .output()
        .unwrap()
}

#[test]
fn runtime_json_is_the_profile() {
    let dir = project(true);
    let output = archguard(&dir, "json");
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["spans"], 14);
    assert_eq!(json["complete"], false);
    let lines = json["statements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["key"].as_str().unwrap().contains("order_lines"))
        .unwrap();
    assert_eq!(lines["max_calls_per_request"], 3);
}

#[test]
fn runtime_text_lists_endpoints_statements_and_call_sites() {
    let dir = project(true);
    let output = archguard(&dir, "text");
    assert_eq!(output.status.code(), Some(0));
    let text = String::from_utf8(output.stdout).unwrap();
    for expected in [
        "14 spans in 3 traces",
        "sampling always_on",
        "incomplete",
        "orders · GET /orders/{id}",
        "SELECT * FROM order_lines WHERE order_id = ?",
        "max 3 per request",
        "Shop.Orders.OrderService.LoadLines (/_/src/Orders/OrderService.cs:42)",
        "line 2",
        "sample",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in:\n{text}");
    }
}

#[test]
fn runtime_without_input_says_where_it_looked() {
    let dir = project(false);
    let output = archguard(&dir, "text");
    assert_eq!(output.status.code(), Some(0));
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains(".archguard/runtime"), "{text}");
    let json: Value = serde_json::from_slice(&archguard(&dir, "json").stdout).unwrap();
    assert!(json.is_null());
}
