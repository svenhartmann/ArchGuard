//! `archguard check` with the `[review]` KPI gates (R30/R31): an
//! unconfigured check passes the cyclic-but-tolerated fixture, each
//! breached gate adds an error finding and flips the exit to 1 — in text
//! and sarif alike.

use std::fs;

use assert_cmd::Command;
use tempfile::tempdir;

/// Two modules in a cycle; the `[rules]` config tolerates it so only the
/// KPI gates can fail the run.
fn fixture(extra: &str) -> tempfile::TempDir {
    let dir = tempdir().expect("tempdir");
    fs::write(
        dir.path().join("archguard.toml"),
        format!("[rules]\ndeny_cycles = 'off'\nmax_scc_size = 100\n{extra}"),
    )
    .expect("config");
    fs::create_dir(dir.path().join("src")).expect("src");
    fs::write(dir.path().join("src/a.ts"), "import './b';\n").expect("a");
    fs::write(dir.path().join("src/b.ts"), "import './a';\n").expect("b");
    dir
}

fn check(dir: &tempfile::TempDir, args: &[&str]) -> std::process::Output {
    Command::cargo_bin("archguard")
        .expect("binary")
        .args(["check"])
        .args(args)
        .current_dir(dir.path())
        .output()
        .expect("runs")
}

#[test]
fn configured_gates_pass_when_the_values_stay_below() {
    // Both nodes are tangled (quote 1.0) and affected (pc 1.0) — a gate
    // of 1.0 is the highest allowed value, so the run still passes.
    let dir = fixture(
        "[review]\nmax_tangledness = 1.0\nmax_propagation_cost = 1.0\nmax_sdp_violations = 0\n",
    );
    let output = check(&dir, &[]);
    assert!(
        output.status.success(),
        "{:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains("metric-max-tangledness"), "{stdout}");
}

#[test]
fn breached_tangledness_gate_exits_1_with_the_finding() {
    let dir = fixture("[review]\nmax_tangledness = 0.4\n");
    let output = check(&dir, &[]);
    assert_eq!(
        output.status.code(),
        Some(1),
        "gate breach must fail the check"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("metric-max-tangledness"), "{stdout}");
    assert!(stdout.contains("Tangledness"), "{stdout}");
    // Meldung mit Belegen: the aggregate numbers are part of the message.
    assert!(stdout.contains("nodes live in cyclic SCCs"), "{stdout}");
}

#[test]
fn breached_propagation_cost_gate_names_the_scc() {
    let dir = fixture("[review]\nmax_propagation_cost = 0.5\n");
    let output = check(&dir, &[]);
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("metric-max-propagation-cost-scc-"),
        "{stdout}"
    );
    assert!(stdout.contains("Propagation cost"), "{stdout}");
}

#[test]
fn sarif_carries_the_gate_as_metric_threshold_rule() {
    let dir = fixture("[review]\nmax_tangledness = 0.4\n");
    let output = check(&dir, &["--format", "sarif"]);
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let doc: serde_json::Value = serde_json::from_str(&stdout).expect("valid sarif");
    let results = doc["runs"][0]["results"].as_array().expect("results");
    let gate = results
        .iter()
        .find(|result| result["ruleId"] == "archguard/metric-threshold")
        .unwrap_or_else(|| panic!("gate finding missing in: {stdout}"));
    assert_eq!(gate["level"], "error");
    let text = gate["message"]["text"].as_str().expect("message text");
    assert!(text.contains("Tangledness"), "{text}");
    let rule_ids: Vec<&str> = doc["runs"][0]["tool"]["driver"]["rules"]
        .as_array()
        .expect("rules")
        .iter()
        .map(|rule| rule["id"].as_str().expect("id"))
        .collect();
    assert!(
        rule_ids.contains(&"archguard/metric-threshold"),
        "{rule_ids:?}"
    );
}
