//! An input that cannot be read is a gap, never a silent skip: the flow is incomplete.
//! Covers the `package.json` probes of `ui_flow_nestjs` and `ui_flow_express`.

use archguard_flow_node::{ui_flow_express, ui_flow_nestjs};

/// `FF FE` announces UTF-16, the single byte after it is half a unit: `InvalidData`.
const BROKEN_UTF16: &[u8] = &[0xFF, 0xFE, 0x41];

fn unread_package_json() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("package.json"), BROKEN_UTF16).unwrap();
    dir
}

fn assert_gap(flow: &archguard_core::UiFlow) {
    assert!(
        !flow.complete,
        "an unreadable package.json must not leave the flow complete"
    );
    assert!(
        flow.diagnostics.iter().any(|d| d.file == "package.json"),
        "no gap names package.json: {:?}",
        flow.diagnostics
    );
}

#[test]
fn nestjs_unreadable_package_json_is_a_gap() {
    assert_gap(&ui_flow_nestjs::extract(unread_package_json().path()));
}

#[test]
fn express_unreadable_package_json_is_a_gap() {
    assert_gap(&ui_flow_express::extract(unread_package_json().path()));
}
