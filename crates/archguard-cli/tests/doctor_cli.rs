//! The doctor lists which external packages the current OS needs for full features.
use serde_json::Value;
use std::process::Command;

fn run(args: &[&str]) -> std::process::Output {
    Command::new(assert_cmd::cargo::cargo_bin!("archguard"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn doctor_json_reports_every_required_tool_with_status_and_hint() {
    let out = run(&["doctor", "--format", "json"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["os"], std::env::consts::OS);
    let tools = value["tools"].as_array().expect("tools array");
    let names: Vec<_> = tools
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name"))
        .collect();
    for required in ["git", "java", "dotnet", "node", "python3", "rust-analyzer"] {
        assert!(names.contains(&required), "{required} must be reported");
    }
    for tool in tools {
        let status = tool["status"].as_str().expect("status");
        assert!(matches!(status, "available" | "missing"), "{tool:?}");
        assert!(!tool["feature"].as_str().expect("feature").is_empty());
        if status == "available" {
            let version = tool["version"].as_str().expect("version");
            assert!(
                !version.is_empty(),
                "an available tool reports its version: {tool:?}"
            );
            assert!(
                !version.starts_with("Picked up "),
                "launcher noise must not masquerade as the version: {tool:?}"
            );
        } else {
            let hint = tool["hint"].as_str().expect("missing tool needs a hint");
            assert!(!hint.is_empty(), "{tool:?}");
        }
    }
    // The test hosts all provide git; a missing git would mask real regressions.
    let git = tools.iter().find(|t| t["name"] == "git").unwrap();
    assert_eq!(git["status"], "available");
}

#[test]
fn doctor_text_exits_zero_even_with_missing_optional_tools() {
    let out = run(&["doctor"]);
    assert!(
        out.status.success(),
        "doctor must never fail the shell: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    for required in ["git", "java", "dotnet", "node", "python3", "rust-analyzer"] {
        assert!(text.contains(required), "{required} missing from: {text}");
    }
}
