//! Golden tests for the checked-in fixtures under `fixtures/<name>` at the
//! repository root (task T28).
//!
//! Each test copies its fixture into a fresh tempdir, runs
//! `archguard check --format json` (or, for the T40 DSM golden,
//! `archguard dsm --format json`) inside the copy (so config discovery can
//! never leak from the repository root and the checked-in fixtures stay
//! pristine), normalizes the parsed stdout JSON, and asserts full JSON
//! equality against `tests/goldens/<name>.json`.
//!
//! The R43 goldens extend the same pattern to the review surface: the
//! PR-comment Markdown golden compares against `tests/goldens/<name>.md`
//! (tempdir paths normalized to `<ROOT>`), and the decision-register list
//! additionally normalizes every `ts` value to `<TS>` so the wall clock
//! never enters a golden.
//!
//! Normalization: any string containing the tempdir path (or its
//! canonicalized form, as macOS turns `/var/…` into `/private/var/…`) is
//! replaced with `<ROOT>`, and any object key named `root` has its value
//! replaced likewise. Compiler version strings are replaced by
//! `<COMPILER_VERSION>`; coverage status, counts and evidence stay exact.
//! Java and C# fixtures require the same installed Java 17+ / .NET 8+
//! toolchains as CI. Missing-backend behavior has separate regressions.
//! Goldens are regenerated through the very same code path with
//! `UPDATE_GOLDENS=1 cargo test -p archguard-cli`.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use assert_cmd::Command;
use serde_json::Value;
use tempfile::{TempDir, tempdir};

/// Placeholder substituted for any machine-dependent root path.
const ROOT_PLACEHOLDER: &str = "<ROOT>";

/// Repository root (the directory holding `Cargo.toml` and `fixtures/`).
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/archguard-cli sits two levels below the repo root")
        .to_path_buf()
}

/// Recursively copy `from` into `to` (which must not exist yet).
fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("create copy dir");
    for entry in fs::read_dir(from).expect("read dir") {
        let entry = entry.expect("dir entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("copy file");
        }
    }
}

/// Recursively replace root-dependent strings in `value`.
fn normalize(value: &mut Value, needles: &[String]) {
    match value {
        Value::Object(map) => {
            for (key, inner) in map.iter_mut() {
                if key == "root" {
                    *inner = Value::String(ROOT_PLACEHOLDER.to_string());
                } else {
                    normalize(inner, needles);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                normalize(item, needles);
            }
        }
        Value::String(text) if needles.iter().any(|needle| text.contains(needle)) => {
            *text = ROOT_PLACEHOLDER.to_string();
        }
        _ => {}
    }
}

/// Copy fixture `name` into a tempdir and run `check --format json` inside
/// it, returning the run output, the tempdir and the parsed, normalized
/// stdout JSON. The tests reject all stderr: unresolved imports would
/// silently drop an edge and weaken the golden.
fn check_fixture(name: &str) -> (Output, TempDir, Value) {
    run_fixture(name, &["check", "--format", "json"])
}

/// [`check_fixture`] with an arbitrary argument list (used by the T40
/// `dsm --format json` golden).
fn run_fixture(name: &str, args: &[&str]) -> (Output, TempDir, Value) {
    let source = repo_root().join("fixtures").join(name);
    assert!(
        source.is_dir(),
        "missing fixture directory {}",
        source.display()
    );
    let dir = tempdir().expect("temp dir");
    copy_dir(&source, &dir.path().join(name));
    let run_dir = dir.path().join(name);

    let out = Command::cargo_bin("archguard")
        .expect("archguard binary")
        .current_dir(&run_dir)
        .args(args)
        .output()
        .expect("run archguard");

    let mut needles = vec![run_dir.display().to_string()];
    if let Ok(canonical) = run_dir.canonicalize() {
        needles.push(canonical.display().to_string());
    }

    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut report: Value =
        serde_json::from_str(&stdout).expect("command prints valid JSON on stdout");
    normalize(&mut report, &needles);
    if let Some(analyzers) = report
        .get_mut("semantic")
        .and_then(|coverage| coverage.get_mut("analyzers"))
        .and_then(Value::as_array_mut)
    {
        for analyzer in analyzers {
            if analyzer["version"].is_string() {
                analyzer["version"] = Value::String("<COMPILER_VERSION>".into());
            }
        }
    }
    (out, dir, report)
}

/// Compare `actual` against the golden for `name`. With `UPDATE_GOLDENS=1`
/// the golden is rewritten (through the same normalization) instead.
fn assert_golden(name: &str, actual: &Value) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("goldens")
        .join(format!("{name}.json"));
    if env::var_os("UPDATE_GOLDENS").is_some() {
        fs::create_dir_all(path.parent().expect("goldens dir")).expect("create goldens dir");
        let pretty = serde_json::to_string_pretty(actual).expect("serialize");
        fs::write(&path, pretty + "\n").expect("write golden");
    }
    let text = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "missing golden {} (generate with UPDATE_GOLDENS=1 cargo test -p archguard-cli): {error}",
            path.display()
        )
    });
    let golden: Value = serde_json::from_str(&text).expect("golden is valid JSON");
    if &golden != actual {
        panic!(
            "golden mismatch for fixture {name}\n\
             --- expected (golden {}) ---\n{}\n\
             --- actual ---\n{}\n",
            path.display(),
            serde_json::to_string_pretty(&golden).expect("re-serialize golden"),
            serde_json::to_string_pretty(actual).expect("re-serialize actual"),
        );
    }
}

/// Assert the exit code and that stderr is empty: every intended import
/// must resolve, and coverage is carried by the JSON report itself.
fn assert_clean_run(out: &Output, expected_code: i32) {
    assert_eq!(
        out.status.code(),
        Some(expected_code),
        "unexpected exit code"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.is_empty(), "unexpected stderr: {stderr}");
}

/// Compare `actual` against the checked-in Markdown golden
/// `tests/goldens/<name>.md` (the R43 PR-comment golden: Markdown, not
/// JSON, so it is not a [`assert_golden`] candidate). With
/// `UPDATE_GOLDENS=1` the file is rewritten instead.
fn assert_markdown_golden(name: &str, actual: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("goldens")
        .join(format!("{name}.md"));
    if env::var_os("UPDATE_GOLDENS").is_some() {
        fs::create_dir_all(path.parent().expect("goldens dir")).expect("create goldens dir");
        fs::write(&path, actual).expect("write markdown golden");
    }
    let golden = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "missing golden {} (generate with UPDATE_GOLDENS=1 cargo test -p archguard-cli --test goldens): {error}",
            path.display()
        )
    });
    assert_eq!(
        golden, actual,
        "markdown golden mismatch for {name}\n--- expected ---\n{golden}\n--- actual ---\n{actual}\n"
    );
}

/// Recursively replace every string value of an object key named `ts`
/// with `<TS>` — the register's wall-clock timestamps are the only
/// machine-dependent part of `review-decision list --format json`.
fn normalize_ts(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, inner) in map.iter_mut() {
                if key == "ts" {
                    *inner = Value::String("<TS>".to_string());
                } else {
                    normalize_ts(inner);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                normalize_ts(item);
            }
        }
        _ => {}
    }
}

/// Run `git` inside `dir` with an isolated config, as the PR-comment
/// regression in `pr_comment_cli.rs` does.
fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("HOME", dir)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Findings of `report` with the given kind and severity.
fn findings(report: &Value, kind: &str, severity: &str) -> Vec<Value> {
    report["findings"]
        .as_array()
        .expect("findings is an array")
        .iter()
        .filter(|finding| finding["kind"] == kind && finding["severity"] == severity)
        .cloned()
        .collect()
}

#[test]
fn ts_cyclic_two_independent_cycles() {
    let (out, _dir, report) = check_fixture("ts-cyclic");
    assert_clean_run(&out, 1);
    assert_eq!(report["summary"]["sccs"], 2, "exactly two SCCs expected");
    let cycles = findings(&report, "cycle", "error");
    assert_eq!(cycles.len(), 2, "two Error cycle findings expected");
    let names: Vec<Vec<String>> = cycles
        .iter()
        .map(|finding| {
            finding["nodes"]
                .as_array()
                .expect("nodes array")
                .iter()
                .map(|node| node.as_str().expect("string node").to_string())
                .collect()
        })
        .collect();
    assert!(
        names.contains(&vec!["a/alpha.ts".to_string(), "b/beta.ts".to_string()]),
        "a↔b cycle missing: {names:?}"
    );
    assert!(
        names.contains(&vec![
            "c/gamma.ts".to_string(),
            "d/delta.ts".to_string(),
            "e/epsilon.ts".to_string(),
        ]),
        "c→d→e→c cycle missing: {names:?}"
    );
    assert_golden("ts-cyclic", &report);
}

#[test]
fn py_layers_single_declared_layer_violation() {
    let (out, _dir, report) = check_fixture("py-layers");
    assert_clean_run(&out, 1);
    assert_eq!(report["summary"]["sccs"], 0, "fixture must stay acyclic");
    assert_eq!(report["config"]["layers"], "declared");
    let violations = findings(&report, "layer_violation", "error");
    assert_eq!(
        violations.len(),
        1,
        "exactly one Error layer violation expected, got {:?}",
        report["findings"]
    );
    assert_eq!(
        violations[0]["nodes"],
        serde_json::json!(["domain/ledger.py", "app/service.py"]),
        "the deliberate domain→app import"
    );
    assert_eq!(
        findings(&report, "cycle", "error").len(),
        0,
        "no cycle findings expected"
    );
    assert_golden("py-layers", &report);
}

#[test]
fn java_packages_package_cycle() {
    let (out, _dir, report) = check_fixture("java-packages");
    assert_clean_run(&out, 1);
    let cycles = findings(&report, "cycle", "error");
    assert_eq!(cycles.len(), 1, "one package cycle expected");
    assert_eq!(
        cycles[0]["nodes"],
        serde_json::json!(["blue/Blue.java", "red/Red.java"]),
        "red and blue must import each other"
    );
    assert_golden("java-packages", &report);
}

#[test]
fn cs_namespaces_namespace_cycle_and_dir_mismatch() {
    let (out, _dir, report) = check_fixture("cs-namespaces");
    assert_clean_run(&out, 1);
    let cycles = findings(&report, "cycle", "error");
    assert_eq!(cycles.len(), 1, "one namespace cycle expected");
    assert_eq!(
        cycles[0]["nodes"],
        serde_json::json!(["Cyan/Cyan.cs", "Magenta/Magenta.cs"]),
        "Cyan and Magenta must import each other"
    );
    // The deliberate flaw: src/Elsewhere/Stray.cs declares `namespace Cyan`
    // and is therefore named under the Cyan module, not its directory.
    assert!(
        report["metrics"].get("Cyan/Stray.cs").is_some(),
        "Stray.cs must be named by its namespace, not its directory: {}",
        report["metrics"]
    );
    assert!(report["metrics"].get("Elsewhere/Stray.cs").is_none());
    assert_golden("cs-namespaces", &report);
}

#[test]
fn php_psr4_namespace_cycle() {
    let (out, _dir, report) = check_fixture("php-psr4");
    assert_clean_run(&out, 1);
    let cycles = findings(&report, "cycle", "error");
    assert_eq!(cycles.len(), 1, "one namespace cycle expected");
    assert_eq!(
        cycles[0]["nodes"],
        serde_json::json!(["App/One/One.php", "App/Two/Two.php"]),
        "App\\One and App\\Two must import each other"
    );
    assert_golden("php-psr4", &report);
}

#[test]
fn rs_clean_acyclic() {
    let (out, _dir, report) = check_fixture("rs-clean");
    assert_clean_run(&out, 0);
    assert_eq!(report["summary"]["sccs"], 0);
    assert!(
        report["findings"].as_array().expect("findings").is_empty(),
        "clean fixture must report no findings: {}",
        report["findings"]
    );
    assert!(
        report["summary"]["edges"].as_u64().expect("edges") > 0,
        "the edges must actually resolve: {:?}",
        report["summary"]
    );
    assert_eq!(
        report["summary"]["nodes"], 6,
        "six real file/module targets, no synthetic import placeholders"
    );
    assert_eq!(report["summary"]["edges"], 7);
    assert!(
        report["metrics"]
            .as_object()
            .unwrap()
            .keys()
            .all(|name| !name.ends_with("/*"))
    );
    assert_golden("rs-clean", &report);
}

#[test]
fn ts_derive_zero_config_derivation() {
    let (out, _dir, report) = check_fixture("ts-derive");
    assert_clean_run(&out, 0);
    // No archguard.toml exists in this fixture: project, module depth and
    // layers all come from derivation.
    assert_eq!(report["config"]["project"], "derived");
    assert_eq!(report["config"]["modules"], "derived");
    assert_eq!(report["config"]["layers"], "derived");
    assert_eq!(
        findings(&report, "cycle", "error").len(),
        0,
        "acyclic fixture must have no cycle findings"
    );
    let errors = report["findings"]
        .as_array()
        .expect("findings")
        .iter()
        .filter(|finding| finding["severity"] == "error")
        .count();
    assert_eq!(
        errors, 0,
        "zero Error findings expected: {}",
        report["findings"]
    );
    assert_eq!(report["summary"]["status"], "pass");
    assert_golden("ts-derive", &report);
}

#[test]
fn ts_cyclic_dsm_json_matrix() {
    // T40: the DSM of the effective (derived-depth) view — sinks first,
    // the two cycles as blocks of cyclic cells. Order and cells are
    // deterministic by construction (dsm_layout breaks ties on names,
    // to_dsm_json sorts its cells).
    let (out, _dir, matrix) = run_fixture("ts-cyclic", &["dsm", "--format", "json"]);
    assert_clean_run(&out, 0);
    assert!(
        matrix["order"].as_array().expect("order").len() > 1,
        "the DSM must show more than one module: {matrix}"
    );
    let cyclic = matrix["cells"]
        .as_array()
        .expect("cells")
        .iter()
        .filter(|cell| cell[3] == serde_json::json!(true))
        .count();
    assert!(
        cyclic > 0,
        "the two fixture cycles must mark cyclic cells: {matrix}"
    );
    assert_golden("ts-cyclic-dsm", &matrix);
}

#[test]
fn review_kpis_sdp_leaks_debt_and_health_are_measured() {
    // R43: the fixture is engineered so the headline KPIs are all
    // measured — a stable module importing an unstable one (SDP), an
    // import reaching an internal symbol (leak), debt markers in
    // comments, and a real cycle behind the tangledness/health numbers.
    let (out, _dir, report) = check_fixture("review-kpis");
    assert_clean_run(&out, 1);
    assert_eq!(report["summary"]["sccs"], 1, "the ping↔pong cycle");
    let kpis = &report["review_kpis"];

    let sdp = kpis["sdp_violations"]
        .as_array()
        .expect("sdp_violations is an array");
    assert_eq!(
        sdp.len(),
        1,
        "the one stable→unstable edge must violate the SDP: {sdp:?}"
    );
    assert_eq!(sdp[0]["from"], "stable/store.ts");
    assert_eq!(sdp[0]["to"], "unstable/flow.ts");

    let leaks = kpis["leaks"].as_array().expect("leaks is an array");
    assert_eq!(
        leaks.len(),
        1,
        "app.ts must leak the internal `mask` of hidden.ts: {leaks:?}"
    );
    assert_eq!(leaks[0]["from_file"], "app.ts");
    assert_eq!(leaks[0]["in_file"], "internal/hidden.ts");
    assert_eq!(leaks[0]["symbol"], "mask");

    let debt = &kpis["debt"];
    assert_eq!(
        debt["markers"].as_u64().expect("markers"),
        5,
        "one TODO, two FIXME, one HACK, one XXX: {debt}"
    );
    assert_eq!(debt["by_kind"]["todo"], 1);
    assert_eq!(debt["by_kind"]["fixme"], 2);
    assert_eq!(debt["by_kind"]["hack"], 1);
    assert_eq!(debt["by_kind"]["xxx"], 1);
    assert!(
        debt["per_kloc"].as_f64().is_some_and(|v| v > 0.0),
        "density is measured, not null: {debt}"
    );

    assert_eq!(kpis["tangledness"]["tangled_nodes"], 2);
    assert_eq!(kpis["tangledness"]["total"], 11);
    assert!(
        kpis["health"]["score"].as_f64().is_some(),
        "health is measured: {}",
        kpis["health"]
    );
    assert_eq!(
        kpis["propagation_cost"]
            .as_array()
            .expect("propagation_cost")
            .len(),
        1,
        "the ping/pong SCC drives one propagation cost"
    );
    assert_golden("review-kpis", &report);
}

#[test]
fn review_kpis_pr_comment_markdown() {
    // R43: the Markdown comment over the same fixture — git-initialized
    // like the pr_comment_cli regression, with a decision register so the
    // accepted-risks section renders both the ADR link and its absence.
    let source = repo_root().join("fixtures").join("review-kpis");
    let dir = tempdir().expect("temp dir");
    let run_dir = dir.path().join("review-kpis");
    copy_dir(&source, &run_dir);
    git(&run_dir, &["init", "--quiet", "--template=", "."]);
    git(&run_dir, &["add", "-A"]);
    git(
        &run_dir,
        &[
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "user.name=test",
            "commit",
            "--quiet",
            "-m",
            "base",
        ],
    );
    for args in [
        &[
            "review-decision",
            "record",
            "--finding",
            "scc-1",
            "--decision",
            "accept-risk",
            "--adr",
            "docs/adr/001.md",
            "--note",
            "zyklus bewusst",
        ][..],
        &[
            "review-decision",
            "record",
            "--finding",
            "scc-2",
            "--decision",
            "accept-risk",
        ][..],
        &[
            "review-decision",
            "record",
            "--finding",
            "scc-3",
            "--decision",
            "defer",
            "--note",
            "später auflösen",
        ][..],
    ] {
        let recorded = Command::cargo_bin("archguard")
            .expect("archguard binary")
            .current_dir(&run_dir)
            .args(args)
            .output()
            .expect("record runs");
        assert!(recorded.status.success(), "record {args:?} failed");
    }

    let out = Command::cargo_bin("archguard")
        .expect("archguard binary")
        .current_dir(&run_dir)
        .args(["review", "--format", "pr-comment", "--base", "HEAD"])
        .output()
        .expect("review runs");
    assert_clean_run(&out, 0);
    let mut markdown = String::from_utf8_lossy(&out.stdout).into_owned();
    for needle in [
        run_dir.display().to_string(),
        run_dir
            .canonicalize()
            .expect("canonical run dir")
            .display()
            .to_string(),
    ] {
        markdown = markdown.replace(&needle, ROOT_PLACEHOLDER);
    }
    // The register renders exactly the two accept-risk decisions, sorted
    // by finding; the deferred one is not PR material.
    assert!(
        markdown.contains("`scc-1` [ADR](docs/adr/001.md)"),
        "{markdown}"
    );
    assert!(markdown.contains("`scc-2` (kein ADR-Link)"), "{markdown}");
    assert!(
        !markdown.contains("scc-3"),
        "defer is not PR material:\n{markdown}"
    );
    assert_markdown_golden("review-kpis-pr", &markdown);
}

#[test]
fn review_decision_list_json() {
    // R43: the decision register list — two records through the real CLI,
    // listed as JSON with the wall-clock `ts` normalized to `<TS>` so the
    // golden is reproducible.
    let dir = tempdir().expect("temp dir");
    for args in [
        &[
            "review-decision",
            "record",
            "--finding",
            "scc-1",
            "--decision",
            "accept-risk",
            "--adr",
            "docs/adr/001.md",
            "--note",
            "zyklus bewusst",
        ][..],
        &[
            "review-decision",
            "record",
            "--finding",
            "scc-2",
            "--decision",
            "defer",
            "--note",
            "später auflösen",
        ][..],
    ] {
        let recorded = Command::cargo_bin("archguard")
            .expect("archguard binary")
            .current_dir(dir.path())
            .args(args)
            .output()
            .expect("record runs");
        assert!(recorded.status.success(), "record {args:?} failed");
    }

    let out = Command::cargo_bin("archguard")
        .expect("archguard binary")
        .current_dir(dir.path())
        .args(["review-decision", "list", "--format", "json"])
        .output()
        .expect("list runs");
    assert_clean_run(&out, 0);
    let mut register: Value =
        serde_json::from_slice(&out.stdout).expect("list prints valid JSON on stdout");
    normalize_ts(&mut register);
    let decisions = register["decisions"].as_array().expect("decisions");
    assert_eq!(decisions.len(), 2, "two distinct findings, no upsert");
    assert_eq!(decisions[0]["id"], "scc-1");
    assert_eq!(decisions[1]["id"], "scc-2");
    assert_golden("review-decision-list", &register);
}
