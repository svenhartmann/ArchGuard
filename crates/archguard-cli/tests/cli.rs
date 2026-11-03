//! T24+T25 integration tests: the `archguard` binary end-to-end via `assert_cmd`.
//!
//! Every test runs in its own tempdir with the fixture as the project
//! root, so no ambient `archguard.toml` can influence the run and no
//! assertion depends on an absolute path.

use std::fs;
use std::path::Path;
use std::process::Output;

use assert_cmd::Command;
use serde_json::Value;
use tempfile::{TempDir, tempdir};

#[test]
fn metric_gates_fail_even_when_cycles_are_allowed_or_baselined() {
    let dir = fixture(&CYCLIC);
    fs::write(
        dir.path().join("archguard.toml"),
        "[rules]\ndeny_cycles = 'off'\nmax_scc_size = 3\nmax_acd = 0.1",
    )
    .unwrap();
    let failed = archguard(&dir, &["check", "--format", "json"]);
    assert_eq!(failed.status.code(), Some(1));
    let value: Value = serde_json::from_slice(&failed.stdout).unwrap();
    assert!(
        value["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["id"] == "metric-max-acd")
    );
    assert_eq!(
        archguard(&dir, &["baseline", "write"]).status.code(),
        Some(0)
    );
    assert_eq!(
        archguard(&dir, &["check", "--baseline"]).status.code(),
        Some(1)
    );
    fs::write(
        dir.path().join("archguard.toml"),
        "[rules]\ndeny_cycles = 'off'\nmax_scc_size = 3\nmax_acd = 3.0",
    )
    .unwrap();
    assert_eq!(archguard(&dir, &["check"]).status.code(), Some(0));
}

#[test]
fn file_cycle_gate_survives_collapsed_view_and_baseline() {
    let dir = fixture(&[
        ("src/app/a.ts", "import './b';"),
        ("src/app/b.ts", "import './a';"),
    ]);
    let config = "derive_layers = false\n[modules]\ndepth = 0\n[rules]\ndeny_cycles = 'file'\nmax_scc_size = 100\n";
    fs::write(dir.path().join("archguard.toml"), config).unwrap();
    let out = archguard(&dir, &["check", "--format", "json"]);
    assert_eq!(out.status.code(), Some(1), "{}", stderr(&out));
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["findings"][0]["id"], "file-scc-1");
    assert_eq!(report["summary"]["edges"], 0);
    assert_eq!(
        archguard(&dir, &["baseline", "write"]).status.code(),
        Some(0)
    );
    let accepted = archguard(&dir, &["check", "--baseline", "--format", "json"]);
    assert_eq!(accepted.status.code(), Some(0));
    assert_eq!(
        serde_json::from_slice::<Value>(&accepted.stdout).unwrap()["summary"]["status"],
        "pass"
    );
    fs::write(
        dir.path().join("archguard.toml"),
        config.replace("'file'", "'off'"),
    )
    .unwrap();
    assert_eq!(archguard(&dir, &["check"]).status.code(), Some(0));
}

/// A → B → C → A via relative imports (same fixture as the engine's
/// `analyze_pipeline` test): exactly one cycle.
const CYCLIC: [(&str, &str); 3] = [
    (
        "src/a.ts",
        "import { b } from \"./b\";\nexport const a = b + 1;\n",
    ),
    (
        "src/b.ts",
        "import { c } from \"./c\";\nexport const b = c + 1;\n",
    ),
    (
        "src/c.ts",
        "import { a } from \"./a\";\nexport const c = a + 1;\n",
    ),
];

/// A single one-way import: no cycle.
const ACYCLIC: [(&str, &str); 2] = [
    (
        "src/a.ts",
        "import { b } from \"./b\";\nexport const a = b + 1;\n",
    ),
    ("src/b.ts", "export const b = 1;\n"),
];

/// Write `files` (root-relative paths) into a fresh tempdir.
fn fixture(files: &[(&str, &str)]) -> TempDir {
    let dir = tempdir().expect("temp dir");
    for (rel, content) in files {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().expect("parent dir")).expect("create dirs");
        fs::write(&path, content).expect("write fixture file");
    }
    dir
}

/// Run the `archguard` binary with `args` inside `dir`.
fn archguard(dir: &TempDir, args: &[&str]) -> Output {
    Command::cargo_bin("archguard")
        .expect("archguard binary")
        .current_dir(dir.path())
        .args(args)
        .output()
        .expect("run archguard")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("exit code")
}

#[test]
fn scan_writes_graph_json_and_prints_summary() {
    let dir = fixture(&CYCLIC);
    let out = archguard(&dir, &["scan", "--out", "graph.json"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let json = fs::read_to_string(dir.path().join("graph.json")).expect("graph.json exists");
    let value: Value = serde_json::from_str(&json).expect("graph.json is valid JSON");
    assert_eq!(value["nodes"].as_array().expect("nodes array").len(), 3);
    assert_eq!(value["edges"].as_array().expect("edges array").len(), 3);
    let stdout = stdout(&out);
    assert!(stdout.contains("modules"), "summary missing: {stdout}");
    assert!(stdout.contains("a.ts"), "summary misses members: {stdout}");
}

#[test]
fn scan_default_out_writes_dot_archguard_graph_json() {
    let dir = fixture(&ACYCLIC);
    let out = archguard(&dir, &["scan"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert!(
        dir.path().join(".archguard/graph.json").is_file(),
        "default --out"
    );
}

#[test]
fn scan_quiet_prints_no_summary_but_still_writes_graph() {
    let dir = fixture(&CYCLIC);
    let out = archguard(&dir, &["--quiet", "scan", "--out", "graph.json"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert_eq!(stdout(&out), "");
    assert!(dir.path().join("graph.json").is_file());
}

#[test]
fn scan_json_format_prints_report_as_json() {
    let dir = fixture(&CYCLIC);
    let out = archguard(&dir, &["--format", "json", "scan", "--out", "graph.json"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let value: Value = serde_json::from_str(&stdout(&out)).expect("summary is valid JSON");
    assert!(value["summary"].is_object(), "report: {value}");
}

#[test]
fn scan_bad_path_exits_3() {
    let dir = fixture(&ACYCLIC);
    let out = archguard(&dir, &["scan", "no-such-directory"]);
    assert_eq!(code(&out), 3);
}

#[test]
fn cycles_text_prints_the_cycle() {
    let dir = fixture(&CYCLIC);
    let out = archguard(&dir, &["cycles"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let stdout = stdout(&out);
    assert!(stdout.contains("cycle"), "no cycle in: {stdout}");
    assert!(stdout.contains("members:"), "no member list in: {stdout}");
    assert!(stdout.contains("a.ts"), "no cycle member in: {stdout}");
}

#[test]
fn cycles_prints_nothing_on_acyclic_fixture() {
    let dir = fixture(&ACYCLIC);
    let out = archguard(&dir, &["cycles"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert!(!stdout(&out).contains("members:"), "unexpected cycle");
}

#[test]
fn cycles_fail_on_found_exits_1_when_cycles_exist() {
    let dir = fixture(&CYCLIC);
    let out = archguard(&dir, &["cycles", "--fail-on-found"]);
    assert_eq!(code(&out), 1);
}

#[test]
fn cycles_fail_on_found_exits_0_on_acyclic_fixture() {
    let dir = fixture(&ACYCLIC);
    let out = archguard(&dir, &["cycles", "--fail-on-found"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
}

#[test]
fn cycles_json_format_parses_as_json() {
    let dir = fixture(&CYCLIC);
    let out = archguard(&dir, &["--format", "json", "cycles"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let value: Value = serde_json::from_str(&stdout(&out)).expect("cycles output is valid JSON");
    let findings = value["findings"].as_array().expect("findings array");
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0]["kind"], "cycle");
}

#[test]
fn cycles_mermaid_format_emits_flowchart() {
    let dir = fixture(&CYCLIC);
    let out = archguard(&dir, &["--format", "mermaid", "cycles"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("flowchart TD"),
        "not mermaid: {}",
        stdout(&out)
    );
}

#[test]
fn cycles_max_cycles_caps_the_output() {
    let dir = fixture(&CYCLIC);
    let out = archguard(&dir, &["cycles", "--max-cycles", "0"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let capped = stdout(&out);
    assert!(
        !capped.contains("members:"),
        "cycle block not capped: {capped}"
    );
    assert!(
        capped.contains("0 of 1 cycles shown"),
        "no cap note: {capped}"
    );
    // Without the cap the block is there.
    let uncapped = archguard(&dir, &["cycles"]);
    assert!(stdout(&uncapped).contains("members:"));
}

#[test]
fn cycles_quiet_prints_nothing_but_keeps_exit_code() {
    let dir = fixture(&CYCLIC);
    let out = archguard(&dir, &["--quiet", "cycles", "--fail-on-found"]);
    assert_eq!(code(&out), 1);
    assert_eq!(stdout(&out), "");
}

#[test]
fn unknown_subcommand_exits_2() {
    let dir = fixture(&ACYCLIC);
    let out = archguard(&dir, &["frobnicate"]);
    assert_eq!(code(&out), 2);
}

#[test]
fn missing_config_file_exits_3() {
    let dir = fixture(&ACYCLIC);
    let out = archguard(&dir, &["--config", "missing.toml", "scan"]);
    assert_eq!(code(&out), 3);
    assert!(stderr(&out).contains("config"), "stderr: {}", stderr(&out));
}

// ------------------------------------------------------------------ T25

/// Declares the cycle ban explicitly so the cycle finding is an Error
/// (Kap. 11) and `check` fails on the cyclic fixture.
const DENY_CYCLES: (&str, &str) = ("archguard.toml", "[rules]\ndeny_cycles = \"module\"\n");

/// A second, independent two-node cycle x ↔ y (added to `CYCLIC` to
/// produce a NEW cycle after a baseline was written).
const EXTRA_CYCLE: [(&str, &str); 2] = [
    (
        "src/x.ts",
        "import { y } from \"./y\";\nexport const x = y;\n",
    ),
    (
        "src/y.ts",
        "import { x } from \"./x\";\nexport const y = x;\n",
    ),
];

/// `CYCLIC` plus the explicit `[rules] deny_cycles = "module"` config.
fn cyclic_denying() -> Vec<(&'static str, &'static str)> {
    let mut files: Vec<(&str, &str)> = CYCLIC.to_vec();
    files.push(DENY_CYCLES);
    files
}

#[test]
fn check_exits_1_on_cyclic_fixture_with_deny_cycles() {
    let dir = fixture(&cyclic_denying());
    let out = archguard(&dir, &["check"]);
    assert_eq!(code(&out), 1, "stderr: {}", stderr(&out));
    let stdout = stdout(&out);
    assert!(stdout.contains("cycle"), "no cycle in: {stdout}");
    assert!(stdout.contains("scc-1"), "no finding id in: {stdout}");
}

#[test]
fn check_exits_0_on_acyclic_fixture() {
    let dir = fixture(&ACYCLIC);
    let out = archguard(&dir, &["check"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
}

#[test]
fn check_sarif_format_parses_with_run_results() {
    let dir = fixture(&cyclic_denying());
    let out = archguard(&dir, &["--format", "sarif", "check"]);
    assert_eq!(code(&out), 1, "stderr: {}", stderr(&out));
    let value: Value = serde_json::from_str(&stdout(&out)).expect("sarif is valid JSON");
    assert_eq!(value["version"], "2.1.0");
    let runs = value["runs"].as_array().expect("runs array");
    assert_eq!(runs.len(), 1);
    let results = runs[0]["results"].as_array().expect("results array");
    assert_eq!(results.len(), 2);
    assert!(
        results
            .iter()
            .any(|result| result["ruleId"] == "archguard/cycle")
    );
    assert!(
        results
            .iter()
            .any(|result| result["ruleId"] == "archguard/metric-threshold")
    );
    assert_eq!(results[0]["level"], "error");
}

#[test]
fn check_baseline_accepts_known_cycles_and_blocks_new_ones() {
    let dir = fixture(&cyclic_denying());
    fs::write(
        dir.path().join("archguard.toml"),
        "[rules]\nmax_scc_size = 100",
    )
    .unwrap();
    let write = archguard(&dir, &["baseline", "write"]);
    assert_eq!(code(&write), 0, "stderr: {}", stderr(&write));
    // The known cycle is baselined → gate passes.
    let out = archguard(&dir, &["--quiet", "check", "--baseline"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    // A new cycle that is not in the baseline fails the gate again.
    for (rel, content) in EXTRA_CYCLE {
        fs::write(dir.path().join(rel), content).expect("write extra cycle");
    }
    let out = archguard(&dir, &["--quiet", "check", "--baseline"]);
    assert_eq!(code(&out), 1);
    // A missing baseline is an IO error (exit 3), not a usage error.
    let out = archguard(&dir, &["check", "--baseline", "nope.json"]);
    assert_eq!(code(&out), 3);
}

#[test]
fn init_writes_derived_config_with_marker() {
    let dir = fixture(&ACYCLIC);
    let out = archguard(&dir, &["init"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let toml = fs::read_to_string(dir.path().join(".archguard/archguard.toml"))
        .expect("archguard.toml exists");
    assert!(
        toml.contains("# abgeleitet"),
        "no derived marker in: {toml}"
    );
}

#[test]
fn init_refuses_to_overwrite_existing_config() {
    let dir = fixture(&ACYCLIC);
    let first = archguard(&dir, &["init"]);
    assert_eq!(code(&first), 0, "stderr: {}", stderr(&first));
    let second = archguard(&dir, &["init"]);
    assert_eq!(code(&second), 3);
    assert!(
        stderr(&second).contains("refusing to overwrite"),
        "stderr: {}",
        stderr(&second)
    );
}

#[test]
fn init_defaults_writes_stock_config_without_marker() {
    let dir = fixture(&ACYCLIC);
    let out = archguard(&dir, &["init", "--defaults"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let toml = fs::read_to_string(dir.path().join(".archguard/archguard.toml"))
        .expect("archguard.toml exists");
    assert!(
        !toml.contains("abgeleitet"),
        "defaults file carries derived marker: {toml}"
    );
    assert!(toml.contains("deny_cycles"), "not the stock config: {toml}");
}

#[test]
fn baseline_write_then_diff_roundtrips_clean() {
    let dir = fixture(&CYCLIC);
    let write = archguard(&dir, &["baseline", "write"]);
    assert_eq!(code(&write), 0, "stderr: {}", stderr(&write));
    assert!(
        dir.path().join(".archguard/baseline.json").is_file(),
        "default file"
    );
    let json = fs::read_to_string(dir.path().join(".archguard/baseline.json")).expect("baseline");
    let value: Value = serde_json::from_str(&json).expect("baseline is valid JSON");
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["hashes"].as_array().expect("hashes").len(), 1);
    let diff = archguard(&dir, &["baseline", "diff"]);
    assert_eq!(code(&diff), 0, "stderr: {}", stderr(&diff));
    assert!(
        stdout(&diff).contains("baseline matches"),
        "stdout: {}",
        stdout(&diff)
    );
}

#[test]
fn baseline_diff_flags_new_cycle_with_exit_1() {
    let dir = fixture(&CYCLIC);
    let write = archguard(&dir, &["baseline", "write"]);
    assert_eq!(code(&write), 0, "stderr: {}", stderr(&write));
    for (rel, content) in EXTRA_CYCLE {
        fs::write(dir.path().join(rel), content).expect("write extra cycle");
    }
    let diff = archguard(&dir, &["baseline", "diff"]);
    assert_eq!(code(&diff), 1, "stderr: {}", stderr(&diff));
    let stdout = stdout(&diff);
    assert!(stdout.contains("ADDED"), "no ADDED line in: {stdout}");
    // A missing baseline file is an IO error (exit 3).
    let missing = archguard(&dir, &["baseline", "diff", ".", "nope.json"]);
    assert_eq!(code(&missing), 3);
}

#[test]
fn metrics_prints_sorted_table_and_caps_with_top() {
    let dir = fixture(&CYCLIC);
    let out = archguard(&dir, &["metrics"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("MODULE"), "no table header in: {text}");
    assert!(text.contains("a.ts"), "no module row in: {text}");
    // R28: the KPI table follows the module table; count only the
    // module-table part and require the KPI block separately.
    let table = text
        .split("Review KPIs (delta")
        .next()
        .expect("module table");
    assert_eq!(
        table.trim_end().lines().count(),
        4,
        "one header + three rows: {text}"
    );
    assert!(
        text.contains("Review KPIs (delta vs. newest snapshot):"),
        "no KPI block in: {text}"
    );
    let capped = archguard(&dir, &["metrics", "--top", "2"]);
    assert_eq!(code(&capped), 0, "stderr: {}", stderr(&capped));
    assert_eq!(
        stdout(&capped)
            .split("Review KPIs (delta")
            .next()
            .expect("module table")
            .trim_end()
            .lines()
            .count(),
        3,
        "header + two rows"
    );
    // --module filters to one row; an unknown name is a usage error.
    let single = archguard(&dir, &["metrics", "--module", "a.ts"]);
    assert_eq!(
        stdout(&single)
            .split("Review KPIs (delta")
            .next()
            .expect("module table")
            .trim_end()
            .lines()
            .count(),
        2
    );
    let unknown = archguard(&dir, &["metrics", "--module", "nope.ts"]);
    assert_eq!(code(&unknown), 2);
}

#[test]
fn metrics_json_format_parses_as_array() {
    let dir = fixture(&CYCLIC);
    let out = archguard(&dir, &["metrics", "--format", "json"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let value: Value = serde_json::from_str(&stdout(&out)).expect("metrics is valid JSON");
    let rows = value.as_array().expect("top-level array");
    assert_eq!(rows.len(), 3);
    assert!(rows[0].get("instability").is_some(), "row: {}", rows[0]);
}

#[test]
fn graph_default_and_dot_format_emit_digraph() {
    let dir = fixture(&CYCLIC);
    let out = archguard(&dir, &["graph"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("digraph"),
        "not dot: {}",
        stdout(&out)
    );
    let explicit = archguard(&dir, &["graph", "--format", "dot"]);
    assert_eq!(code(&explicit), 0, "stderr: {}", stderr(&explicit));
    assert!(stdout(&explicit).contains("digraph"));
    // Cyclic edges of the scc are highlighted.
    assert!(stdout(&explicit).contains("color=red"), "no red edges");
    // A format graph does not speak is a usage error.
    let bad = archguard(&dir, &["graph", "--format", "text"]);
    assert_eq!(code(&bad), 2);
}

#[test]
fn graph_mermaid_and_json_formats_work() {
    let dir = fixture(&CYCLIC);
    let mermaid = archguard(&dir, &["graph", "--format", "mermaid"]);
    assert_eq!(code(&mermaid), 0, "stderr: {}", stderr(&mermaid));
    assert!(
        stdout(&mermaid).contains("flowchart TD"),
        "not mermaid: {}",
        stdout(&mermaid)
    );
    let json = archguard(&dir, &["graph", "--format", "json"]);
    assert_eq!(code(&json), 0, "stderr: {}", stderr(&json));
    let value: Value = serde_json::from_str(&stdout(&json)).expect("graph json is valid JSON");
    assert_eq!(value["nodes"].as_array().expect("nodes").len(), 3);
    // --focus narrows to the neighbourhood, --depth 0 to the node alone.
    let focused = archguard(&dir, &["graph", "--focus", "a.ts", "--depth", "0"]);
    assert_eq!(code(&focused), 0, "stderr: {}", stderr(&focused));
    let focused = stdout(&focused);
    assert!(focused.contains("a_ts"), "focus lost: {focused}");
    assert!(!focused.contains("c_ts"), "depth 0 leaked: {focused}");
}

#[test]
fn graph_html_format_emits_self_contained_page() {
    let dir = fixture(&CYCLIC);
    let out = archguard(&dir, &["graph", "--format", "html"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let page = stdout(&out);
    assert!(page.starts_with("<!DOCTYPE html>"), "not a page: {page}");
    assert!(
        page.contains("https://cdn.jsdelivr.net/npm/force-graph@1.52.0/dist/force-graph.min.js"),
        "pinned CDN script missing: {page}"
    );
    assert!(
        page.contains(r#"type="application/json""#),
        "JSON payload marker missing: {page}"
    );
}

/// Recursively copy the checked-in fixture directory `fixtures/<name>`
/// into a fresh tempdir, so repo-root fixtures stay pristine and config
/// discovery cannot leak from the repository.
fn fixture_dir(name: &str) -> TempDir {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/archguard-cli sits two levels below the repo root")
        .join("fixtures")
        .join(name);
    fn copy(from: &Path, to: &Path) {
        fs::create_dir_all(to).expect("create copy dir");
        for entry in fs::read_dir(from).expect("read fixture dir") {
            let entry = entry.expect("dir entry");
            let target = to.join(entry.file_name());
            if entry.file_type().expect("file type").is_dir() {
                copy(&entry.path(), &target);
            } else {
                fs::copy(entry.path(), &target).expect("copy fixture file");
            }
        }
    }
    let dir = tempdir().expect("temp dir");
    copy(&root, dir.path());
    dir
}

#[test]
fn ts_xss_fixture_payloads_stay_literal_inert_text_in_the_html_viewer() {
    // F01 regression: `fixtures/ts-xss` carries HTML payloads inside
    // import comments (an <img onerror> probe and a </script> closer
    // probe; the extractor keeps whole lines verbatim in `raw`). The
    // viewer must show them as literal text — the payload text travels
    // into the JSON block for the text-node tooltips — without ever
    // breaking out of that block or materialising as an element or
    // event handler. The behavioral half (force-graph receives text-only
    // label ELEMENTS from nodeLabel/linkLabel) is verified by the
    // headless DOM harness in scripts/viewer/test_viewer_state.js.
    let dir = fixture_dir("ts-xss");
    let out = archguard(&dir, &["graph", "--format", "html"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let page = stdout(&out);

    const OPEN: &str = r#"<script id="archguard-data" type="application/json">"#;
    let start = page.find(OPEN).expect("payload script") + OPEN.len();
    let end = page[start..]
        .find("</script>")
        .expect("payload script must close");
    let payload = &page[start..start + end];

    // Untrusted source text survives verbatim INSIDE the payload block —
    // this is the exact string the tooltips must display as literal text.
    assert!(
        payload.contains("<img src=x onerror=alert(1)>"),
        "payload must carry the literal import comment: {payload}"
    );
    // The script-closer probe is escaped at the embedding boundary and
    // stayed inside the block (it did not terminate the script element).
    assert!(
        payload.contains(r"<\/script>"),
        "the </script> probe must be escaped in place: {payload}"
    );
    // Exactly the two real scripts exist (payload, app). The optional network
    // renderer loads dynamically; the probe must not create a third closer.
    assert_eq!(
        page.matches("</script>").count(),
        2,
        "script closers changed"
    );
    assert_eq!(
        page.lines()
            .filter(|line| line.starts_with("<script"))
            .count(),
        2,
        "only the payload and application scripts may be embedded"
    );
    assert!(
        !page.contains("<script src="),
        "no parser-blocking CDN script"
    );
    // Outside the payload block the probe text is absent entirely, so no
    // injected element or handler can exist. The trusted lazy loader does use
    // script.onerror; that API name is not the untrusted fixture's handler.
    let outside = format!(
        "{}{}",
        &page[..start - OPEN.len()],
        &page[start + end + "</script>".len()..]
    );
    assert!(
        !outside.contains("<img"),
        "injected element outside payload"
    );
    assert!(
        !outside.contains("onerror=alert(1)") && !outside.contains("alert(1)"),
        "injected handler outside payload"
    );
    // Both label accessors hand force-graph the text-only element builder
    // (tooltipEl → createTextNode), never raw HTML strings (innerHTML).
    assert!(
        page.contains(".nodeLabel(n => tooltipEl("),
        "nodeLabel must build a text-only element"
    );
    assert!(
        page.contains(".linkLabel(l => tooltipEl("),
        "linkLabel must build a text-only element"
    );
    assert!(
        page.contains("document.createTextNode(String(line))"),
        "tooltip lines must go through text nodes"
    );
}

#[test]
fn graph_graphml_and_gexf_formats_emit_xml() {
    let dir = fixture(&CYCLIC);
    let graphml = archguard(&dir, &["graph", "--format", "graphml"]);
    assert_eq!(code(&graphml), 0, "stderr: {}", stderr(&graphml));
    let out = stdout(&graphml);
    assert!(out.starts_with("<?xml"), "not XML: {out}");
    assert!(out.contains("<graphml"));
    let gexf = archguard(&dir, &["graph", "--format", "gexf"]);
    assert_eq!(code(&gexf), 0, "stderr: {}", stderr(&gexf));
    let out = stdout(&gexf);
    assert!(out.starts_with("<?xml"));
    assert!(out.contains("<gexf"));
    // The graph-only formats stay a usage error (exit 2) as global flags.
    // (`html` moved out of this set with T40: it is a valid `cycles`
    // format now — see cycles_format_html_opens_the_cycles_panel.)
    let bad = archguard(&dir, &["--format", "graphml", "cycles"]);
    assert_eq!(code(&bad), 2);
    let bad = archguard(&dir, &["--format", "graphml", "check"]);
    assert_eq!(code(&bad), 2);
}

#[test]
fn suggest_prints_cuts_and_caps_with_max() {
    let dir = fixture(&CYCLIC);
    let out = archguard(&dir, &["suggest"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("scc-1"), "no cycle id in: {text}");
    assert!(text.contains("a.ts -> b.ts"), "no cut in: {text}");
    let capped = archguard(&dir, &["suggest", "--cycle", "scc-1", "--max", "1"]);
    assert_eq!(code(&capped), 0, "stderr: {}", stderr(&capped));
    assert_eq!(capped_cuts_lines(&stdout(&capped)), 1, "cut not capped");
    let unknown = archguard(&dir, &["suggest", "--cycle", "scc-99"]);
    assert_eq!(code(&unknown), 2);
}

/// Count the indented cut lines of `suggest` text output.
fn capped_cuts_lines(text: &str) -> usize {
    text.lines().filter(|line| line.starts_with("  ")).count()
}

#[test]
fn suggest_json_format_parses_as_array() {
    let dir = fixture(&CYCLIC);
    let out = archguard(&dir, &["suggest", "--format", "json"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let value: Value = serde_json::from_str(&stdout(&out)).expect("suggest is valid JSON");
    let cycles = value.as_array().expect("top-level array");
    assert_eq!(cycles.len(), 1);
    assert_eq!(cycles[0]["id"], "scc-1");
    assert!(!cycles[0]["cuts"].as_array().expect("cuts").is_empty());
}

#[test]
fn config_text_shows_per_section_provenance() {
    let dir = fixture(&ACYCLIC);
    let out = archguard(&dir, &["config"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let stdout = stdout(&out);
    assert!(stdout.contains("provenance"), "no header in: {stdout}");
    assert!(stdout.contains("rules"), "no section row in: {stdout}");
    assert!(
        stdout.contains("declared") || stdout.contains("derived") || stdout.contains("default"),
        "no source names in: {stdout}"
    );
}

#[test]
fn config_json_format_parses_with_config_object() {
    let dir = fixture(&ACYCLIC);
    let out = archguard(&dir, &["config", "--format", "json"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let value: Value = serde_json::from_str(&stdout(&out)).expect("config is valid JSON");
    let config = value["config"].as_object().expect("config object");
    assert!(config.contains_key("rules"), "sections: {config:?}");
    assert_eq!(config["rules"], "default", "explicit config above was none");
}

// ---------------------------------------------------------------------------
// T40: dsm, simulate, graph --expand
// ---------------------------------------------------------------------------

/// Two modules with one file each and a single one-way import. The
/// declared `[modules] depth = 0` keeps the module nodes (`a`, `b`)
/// visible, so `--expand a` has something to zoom into.
const EXPANDABLE: [(&str, &str); 3] = [
    ("archguard.toml", "[modules]\ndepth = 0\n"),
    (
        "src/a/alpha.ts",
        "import { beta } from \"../b/beta\";\nexport function alpha(input: string): string { return beta(input); }\n",
    ),
    (
        "src/b/beta.ts",
        "export function beta(input: string): string { return input; }\n",
    ),
];

/// Sorted node names of a `graph --format json` payload.
fn node_names(value: &Value) -> Vec<String> {
    value["nodes"]
        .as_array()
        .expect("nodes array")
        .iter()
        .map(|node| node["name"].as_str().expect("string name").to_string())
        .collect()
}

/// The edges of a `graph --format json` payload as (from, to) names.
fn edge_pairs(value: &Value) -> Vec<(String, String)> {
    let names: Vec<&str> = value["nodes"]
        .as_array()
        .expect("nodes array")
        .iter()
        .map(|node| node["name"].as_str().expect("string name"))
        .collect();
    value["edges"]
        .as_array()
        .expect("edges array")
        .iter()
        .map(|edge| {
            let from = edge["from"].as_u64().expect("numeric from") as usize;
            let to = edge["to"].as_u64().expect("numeric to") as usize;
            (names[from].to_string(), names[to].to_string())
        })
        .collect()
}

#[test]
fn graph_expand_zooms_a_module_into_its_children() {
    let dir = fixture(&EXPANDABLE);
    let plain = archguard(&dir, &["graph", "--format", "json"]);
    assert_eq!(code(&plain), 0, "stderr: {}", stderr(&plain));
    let plain: Value = serde_json::from_str(&stdout(&plain)).expect("graph is valid JSON");
    let plain_names = node_names(&plain);
    assert!(
        plain_names.contains(&"a".to_string()),
        "raw: {plain_names:?}"
    );
    assert!(
        plain_names.contains(&"a/alpha.ts".to_string()),
        "raw: {plain_names:?}"
    );

    let expanded = archguard(&dir, &["graph", "--format", "json", "--expand", "a"]);
    assert_eq!(code(&expanded), 0, "stderr: {}", stderr(&expanded));
    let expanded: Value =
        serde_json::from_str(&stdout(&expanded)).expect("expanded graph is valid JSON");
    assert_eq!(
        node_names(&expanded),
        vec!["a/alpha.ts".to_string(), "b".to_string()],
        "the module a is replaced by its child, b stays aggregated"
    );
    assert_eq!(
        edge_pairs(&expanded),
        vec![("a/alpha.ts".to_string(), "b".to_string())],
        "the import re-projects onto the revealed child"
    );

    let unknown = archguard(&dir, &["graph", "--format", "json", "--expand", "ghost"]);
    assert_eq!(code(&unknown), 2, "unknown --expand name must fail");
    assert!(
        stderr(&unknown).contains("unknown module 'ghost' for '--expand'"),
        "stderr: {}",
        stderr(&unknown)
    );
}

#[test]
fn dsm_json_marks_every_cycle_cell_of_the_fixture() {
    let dir = fixture(&CYCLIC);
    let out = archguard(&dir, &["dsm", "--format", "json"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let value: Value = serde_json::from_str(&stdout(&out)).expect("dsm is valid JSON");
    assert_eq!(
        value["order"],
        serde_json::json!(["a.ts", "b.ts", "c.ts"]),
        "the single cycle keeps its members in order"
    );
    assert_eq!(
        value["cells"],
        serde_json::json!([[0, 1, 1, true], [1, 2, 1, true], [2, 0, 1, true]]),
        "every cell of the a→b→c→a ring is a weighted cyclic cell"
    );
}

#[test]
fn dsm_within_and_expand_narrow_the_view() {
    let dir = fixture(&EXPANDABLE);
    let within = archguard(&dir, &["dsm", "--format", "json", "--within", "a"]);
    assert_eq!(code(&within), 0, "stderr: {}", stderr(&within));
    let within: Value = serde_json::from_str(&stdout(&within)).expect("dsm is valid JSON");
    assert_eq!(
        within["order"],
        serde_json::json!(["a/alpha.ts"]),
        "within a shows exactly its child"
    );
    assert_eq!(
        within["cells"],
        serde_json::json!([]),
        "no edges inside one node"
    );

    let expanded = archguard(&dir, &["dsm", "--format", "json", "--expand", "a"]);
    assert_eq!(code(&expanded), 0, "stderr: {}", stderr(&expanded));
    let expanded: Value = serde_json::from_str(&stdout(&expanded)).expect("dsm is valid JSON");
    assert_eq!(
        expanded["order"],
        serde_json::json!(["b", "a/alpha.ts"]),
        "sinks first: nothing depends on b"
    );
    assert_eq!(
        expanded["cells"],
        serde_json::json!([[1, 0, 1, false]]),
        "the one-way import is a non-cyclic cell"
    );

    let unknown = archguard(&dir, &["dsm", "--within", "ghost"]);
    assert_eq!(code(&unknown), 2, "unknown --within name must fail");
    assert!(
        stderr(&unknown).contains("unknown module 'ghost' for '--within'"),
        "stderr: {}",
        stderr(&unknown)
    );
}

#[test]
fn simulate_ops_file_resolves_the_fixture_cycle() {
    let dir = fixture(&CYCLIC);
    fs::write(
        dir.path().join("ops.json"),
        "[{\"cut_edge\":{\"from\":\"a.ts\",\"to\":\"b.ts\"}}]\n",
    )
    .expect("write ops file");
    let out = archguard(&dir, &["simulate", "--ops", "ops.json", "--format", "json"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let delta: Value = serde_json::from_str(&stdout(&out)).expect("delta is valid JSON");
    assert_eq!(delta["resolved_findings"], serde_json::json!(["scc-1"]));
    assert_eq!(delta["introduced_findings"], serde_json::json!([]));
    assert_eq!(delta["scc_count_before"], 1);
    assert_eq!(delta["scc_count_after"], 0, "the cut must clear the cycle");
    assert!(
        delta["acd_after"].as_f64().expect("acd_after")
            < delta["acd_before"].as_f64().expect("acd_before"),
        "cutting a cycle edge cannot raise the ACD: {delta}"
    );
}

#[test]
fn simulate_cut_flag_resolves_the_fixture_cycle() {
    let dir = fixture(&CYCLIC);
    let out = archguard(&dir, &["simulate", "--cut", "a.ts,b.ts"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let text = stdout(&out);
    assert!(
        text.contains("scc-1"),
        "resolved finding missing in: {text}"
    );
    assert!(text.contains("SCCs: 1 -> 0"), "scc line wrong in: {text}");
    assert!(text.contains("ACD:"), "acd line missing in: {text}");
}

#[test]
fn simulate_without_any_op_is_a_usage_error() {
    let dir = fixture(&CYCLIC);
    let out = archguard(&dir, &["simulate"]);
    assert_eq!(code(&out), 2, "simulate without ops must fail");
    assert!(stderr(&out).contains("no ops"), "stderr: {}", stderr(&out));
}

/// One fabricated history file (T41): `name` inside
/// `.archguard/history/`, `row` the raw snapshot JSON the engine reads.
fn write_history(dir: &TempDir, name: &str, row: serde_json::Value) {
    let history = dir.path().join(".archguard/history");
    fs::create_dir_all(&history).expect("history dir");
    fs::write(history.join(name), row.to_string()).expect("write snapshot file");
}

/// A snapshot row with the numbers the trend table must render and diff.
fn snapshot_row(
    timestamp: u64,
    git_sha: &str,
    tag: Option<&str>,
    sccs: u32,
    largest_scc: u32,
    acd: f64,
    edges: u32,
) -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "timestamp": timestamp,
        "git_sha": git_sha,
        "tag": tag,
        "nodes": 5,
        "edges": edges,
        "sccs": sccs,
        "largest_scc": largest_scc,
        "acd": acd,
        "nccd": acd / 2.0,
        "cycle_hashes": [],
    })
}

#[test]
fn snapshot_appends_history_and_trend_lists_both_rows() {
    let dir = fixture(&CYCLIC);
    let out = archguard(&dir, &["snapshot"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("snapshot written to"),
        "stdout: {}",
        stdout(&out)
    );
    let out = archguard(&dir, &["snapshot", "--tag", "v1"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let history = dir.path().join(".archguard/history");
    let files = fs::read_dir(&history).expect("history dir").count();
    assert_eq!(files, 2, "one file per snapshot in {history:?}");
    let out = archguard(&dir, &["trend", "--format", "json"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let doc: Value = serde_json::from_str(&stdout(&out)).expect("trend json");
    assert_eq!(doc["count"], 2);
    let rows = doc["snapshots"].as_array().expect("snapshots array");
    assert_eq!(rows.len(), 2);
    // Outside any git repository the SHA degrades to the literal `notag`.
    assert_eq!(rows[0]["git_sha"], "notag");
    assert_eq!(rows[0]["tag"], Value::Null, "first snapshot is untagged");
    assert_eq!(rows[1]["tag"], "v1", "second snapshot keeps its tag");
    assert!(
        rows[0]["timestamp"].as_u64().expect("ts") <= rows[1]["timestamp"].as_u64().expect("ts"),
        "rows are oldest first"
    );
    assert_eq!(rows[0]["cycles"], 1, "the fixture cycle is counted");
}

#[test]
fn trend_table_shows_sha_tag_and_delta_signs() {
    let dir = fixture(&ACYCLIC);
    write_history(
        &dir,
        "100-abc1111.json",
        snapshot_row(100, "abc1111", None, 4, 3, 0.5, 10),
    );
    write_history(
        &dir,
        "200-def2222.json",
        snapshot_row(200, "def2222", Some("v1"), 2, 3, 0.25, 12),
    );
    let out = archguard(&dir, &["trend"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let text = stdout(&out);
    assert!(
        text.contains("DATE") && text.contains("REF"),
        "header missing: {text}"
    );
    assert!(text.contains("abc1111"), "row shows its sha: {text}");
    assert!(text.contains("v1"), "row shows its tag: {text}");
    let first = text
        .lines()
        .find(|line| line.contains("abc1111"))
        .expect("first row");
    let second = text
        .lines()
        .find(|line| line.contains("v1"))
        .expect("second row");
    assert!(!first.contains('('), "first row shows bare values: {first}");
    assert!(second.contains("(-2)"), "cycles fell by two: {second}");
    assert!(second.contains("(±0)"), "largest scc unchanged: {second}");
    assert!(second.contains("(+2)"), "edges grew by two: {second}");
    assert!(second.contains("(-0.250)"), "acd fell: {second}");
}

#[test]
fn trend_last_quiet_and_empty_history() {
    let dir = fixture(&ACYCLIC);
    let out = archguard(&dir, &["trend"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert!(
        stdout(&out).contains("no snapshots"),
        "empty history hint: {}",
        stdout(&out)
    );
    write_history(
        &dir,
        "100-aaa1111.json",
        snapshot_row(100, "aaa1111", None, 1, 2, 0.5, 3),
    );
    write_history(
        &dir,
        "200-bbb2222.json",
        snapshot_row(200, "bbb2222", None, 1, 2, 0.5, 4),
    );
    let out = archguard(&dir, &["trend", "--last", "1"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let text = stdout(&out);
    assert!(text.contains("bbb2222"), "newest row survives: {text}");
    assert!(!text.contains("aaa1111"), "older row is cut: {text}");
    let out = archguard(&dir, &["trend", "--quiet"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    assert!(stdout(&out).is_empty(), "--quiet prints nothing");
}

#[test]
fn trend_rejects_foreign_formats_with_usage_exit() {
    let dir = fixture(&ACYCLIC);
    let out = archguard(&dir, &["trend", "--format", "dot"]);
    assert_eq!(code(&out), 2, "foreign format must be a usage error");
    assert!(
        stderr(&out).contains("trend supports table and json"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn cycles_format_html_opens_the_cycles_panel() {
    let dir = fixture(&CYCLIC);
    let out = archguard(&dir, &["cycles", "--format", "html"]);
    assert_eq!(code(&out), 0, "stderr: {}", stderr(&out));
    let page = stdout(&out);
    assert!(page.starts_with("<!DOCTYPE html>"), "not a page: {page}");
    assert!(
        page.contains("https://cdn.jsdelivr.net/npm/force-graph@1.52.0/dist/force-graph.min.js"),
        "pinned CDN script missing"
    );
    assert!(
        page.contains(r#""initialTab":"cycles""#),
        "the payload must ask for the cycles tab"
    );
}

#[test]
fn project_typescript_compiler_runs_only_after_explicit_trust() {
    let dir = tempdir().unwrap();
    let marker = dir.path().join("compiler-ran");
    let compiler = dir.path().join("node_modules/typescript/lib");
    fs::create_dir_all(&compiler).unwrap();
    fs::write(
        dir.path().join("node_modules/typescript/package.json"),
        r#"{"name":"typescript","version":"5.8.2"}"#,
    )
    .unwrap();
    fs::write(
        compiler.join("typescript.js"),
        format!(
            "require('fs').writeFileSync({},'ran');\n",
            serde_json::to_string(marker.to_str().unwrap()).unwrap()
        ),
    )
    .unwrap();
    fs::write(dir.path().join("main.ts"), "export const a = 1;\n").unwrap();

    let untrusted = Command::cargo_bin("archguard")
        .unwrap()
        .current_dir(dir.path())
        .env_remove("ARCHGUARD_TRUST_PROJECT_COMPILER")
        .args(["calls", ".", "--format", "json"])
        .output()
        .unwrap();
    assert!(!marker.exists(), "scanning must not run repository code");
    assert!(
        stdout(&untrusted).contains("ARCHGUARD_TRUST_PROJECT_COMPILER"),
        "the diagnostic names the opt-in: {}",
        stdout(&untrusted)
    );

    // Without Node there is nothing to run; the opt-in below would prove nothing.
    if std::process::Command::new("node")
        .arg("--version")
        .output()
        .is_err()
    {
        return;
    }
    Command::cargo_bin("archguard")
        .unwrap()
        .current_dir(dir.path())
        .env("ARCHGUARD_TRUST_PROJECT_COMPILER", "1")
        .args(["calls", ".", "--format", "json"])
        .output()
        .unwrap();
    assert!(
        marker.exists(),
        "the opt-in must reach the installed compiler"
    );
}

#[test]
fn layer_rules_see_dependencies_between_workspace_crates() {
    let dir = tempdir().unwrap();
    for (name, content) in [
        ("core/Cargo.toml", "[package]\nname = \"app-core\"\n"),
        ("core/src/lib.rs", "pub struct Thing;\n"),
        (
            "cli/Cargo.toml",
            "[package]\nname = \"app-cli\"\n[dependencies]\napp-core = { path = \"../core\" }\n",
        ),
        ("cli/src/lib.rs", "use app_core::Thing;\n"),
    ] {
        let path = dir.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
    let policy = |allowed: &str| {
        fs::write(
            dir.path().join("archguard.toml"),
            format!(
                "derive_layers = false\n[project]\nsource_roots = []\n[[layers]]\nname = \"core\"\ncontains = [\"core/**\"]\nmay_depend_on = []\n[[layers]]\nname = \"cli\"\ncontains = [\"cli/**\"]\nmay_depend_on = {allowed}\n"
            ),
        )
        .unwrap();
        archguard(&dir, &["check", "--format", "json"])
            .status
            .code()
    };
    assert_eq!(policy("[\"core\"]"), Some(0));
    assert_eq!(policy("[]"), Some(1), "the forbidden crate edge must fail");
}
