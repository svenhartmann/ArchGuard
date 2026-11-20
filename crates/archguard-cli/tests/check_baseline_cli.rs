//! `archguard check --baseline` with the stock rules: the baseline accepts
//! only the cycle finding, the SCC-size gate keeps failing until the cycle
//! is fixed; and `check` rejects formats it cannot render.
//! The default baseline file is `[rules].baseline` under the analysed
//! project root, wherever the command runs from.

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use tempfile::tempdir;

fn archguard(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::cargo_bin("archguard")
        .expect("binary")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("runs")
}

fn write(dir: &Path, rel: &str, content: &str) {
    let path = dir.join(rel);
    fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
    fs::write(path, content).expect("write");
}

/// A three-module cycle a → b → c → a under `src/`, no config file.
fn cyclic(dir: &Path) {
    write(
        dir,
        "src/a.ts",
        "import { b } from \"./b\";\nexport const a = b;\n",
    );
    write(
        dir,
        "src/b.ts",
        "import { c } from \"./c\";\nexport const b = c;\n",
    );
    write(
        dir,
        "src/c.ts",
        "import { a } from \"./a\";\nexport const c = a;\n",
    );
}

fn report(out: &std::process::Output) -> String {
    format!(
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Lets a baselined cycle pass: the stock `max_scc_size = 1` gate is
/// independent of the baseline, so a known cycle still fails `check`.
const TOLERATE_SCC_SIZE: &str = "[rules]\nmax_scc_size = 100\n";

#[test]
fn baselined_cycle_still_fails_the_default_scc_size_gate() {
    let dir = tempdir().expect("tempdir");
    cyclic(dir.path());
    let out = archguard(dir.path(), &["baseline", "write"]);
    assert_eq!(out.status.code(), Some(0), "{}", report(&out));
    let out = archguard(dir.path(), &["check", "--baseline"]);
    assert_eq!(out.status.code(), Some(1), "{}", report(&out));
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("metric-max-scc-size"),
        "{}",
        report(&out)
    );
}

#[test]
fn check_rejects_the_pr_comment_format() {
    let dir = tempdir().expect("tempdir");
    cyclic(dir.path());
    let out = archguard(dir.path(), &["--format", "pr-comment", "check"]);
    assert_eq!(out.status.code(), Some(2), "{}", report(&out));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("check supports text, json and sarif"),
        "{}",
        report(&out)
    );
}

#[test]
fn baseline_write_of_a_subproject_lands_in_that_project() {
    let repo = tempdir().expect("tempdir");
    let api = repo.path().join("api");
    cyclic(&api);
    write(&api, "archguard.toml", TOLERATE_SCC_SIZE);
    let out = archguard(repo.path(), &["baseline", "write", "api"]);
    assert_eq!(out.status.code(), Some(0), "{}", report(&out));
    assert!(
        api.join(".archguard/baseline.json").is_file(),
        "{}",
        report(&out)
    );
    assert!(!repo.path().join(".archguard/baseline.json").exists());

    // Found from inside the project and from the repo root alike.
    let out = archguard(&api, &["check", ".", "--baseline"]);
    assert_eq!(out.status.code(), Some(0), "from api: {}", report(&out));
    let out = archguard(repo.path(), &["check", "api", "--baseline"]);
    assert_eq!(out.status.code(), Some(0), "from repo: {}", report(&out));
    let out = archguard(repo.path(), &["baseline", "diff", "api"]);
    assert_eq!(out.status.code(), Some(0), "diff: {}", report(&out));
}

#[test]
fn configured_rules_baseline_is_the_default_file() {
    let dir = tempdir().expect("tempdir");
    cyclic(dir.path());
    write(
        dir.path(),
        "archguard.toml",
        &format!("{TOLERATE_SCC_SIZE}baseline = \".archguard/accepted.json\"\n"),
    );
    let out = archguard(dir.path(), &["baseline", "write"]);
    assert_eq!(out.status.code(), Some(0), "{}", report(&out));
    assert!(
        dir.path().join(".archguard/accepted.json").is_file(),
        "{}",
        report(&out)
    );
    assert!(!dir.path().join(".archguard/baseline.json").exists());

    let out = archguard(dir.path(), &["check", "--baseline"]);
    assert_eq!(out.status.code(), Some(0), "check: {}", report(&out));
    let out = archguard(dir.path(), &["baseline", "diff"]);
    assert_eq!(out.status.code(), Some(0), "diff: {}", report(&out));
}

/// SARIF agrees with the exit code: a baselined cycle is a note, not an
/// error, while the cycle stays visible.
#[test]
fn sarif_reports_baselined_cycles_as_notes() {
    let dir = tempdir().expect("tempdir");
    cyclic(dir.path());
    write(dir.path(), "archguard.toml", TOLERATE_SCC_SIZE);
    let out = archguard(dir.path(), &["baseline", "write"]);
    assert_eq!(out.status.code(), Some(0), "{}", report(&out));
    let out = archguard(dir.path(), &["--format", "sarif", "check", "--baseline"]);
    assert_eq!(out.status.code(), Some(0), "{}", report(&out));
    let sarif: serde_json::Value = serde_json::from_slice(&out.stdout).expect("sarif json");
    let cycles: Vec<&serde_json::Value> = sarif["runs"][0]["results"]
        .as_array()
        .expect("results")
        .iter()
        .filter(|r| r["ruleId"] == "archguard/cycle")
        .collect();
    assert!(!cycles.is_empty(), "{}", report(&out));
    assert!(
        cycles.iter().all(|r| r["level"] == "note"),
        "{}",
        report(&out)
    );
}

/// A hostile `[rules].baseline` must not let `baseline write` (no file
/// argument) replace a file outside the project; `outside` keeps its bytes.
fn assert_write_refused(dir: &Path, outside: &Path) {
    let out = archguard(dir, &["baseline", "write"]);
    assert_ne!(out.status.code(), Some(0), "{}", report(&out));
    assert_eq!(
        fs::read_to_string(outside).expect("outside file"),
        "keep",
        "{}",
        report(&out)
    );
}

#[test]
fn configured_baseline_escaping_the_root_is_refused() {
    let base = tempdir().expect("tempdir");
    let dir = base.path().join("repo");
    cyclic(&dir);
    write(base.path(), "victim.json", "keep");
    write(
        &dir,
        "archguard.toml",
        &format!("{TOLERATE_SCC_SIZE}baseline = \"../victim.json\"\n"),
    );
    assert_write_refused(&dir, &base.path().join("victim.json"));

    // An explicit operator file argument stays unrestricted.
    let out = archguard(&dir, &["baseline", "write", ".", "../chosen.json"]);
    assert_eq!(out.status.code(), Some(0), "{}", report(&out));
    assert!(
        base.path().join("chosen.json").is_file(),
        "{}",
        report(&out)
    );
}

#[test]
fn configured_absolute_baseline_is_refused() {
    let base = tempdir().expect("tempdir");
    let dir = base.path().join("repo");
    cyclic(&dir);
    let victim = base.path().join("victim.json");
    write(base.path(), "victim.json", "keep");
    write(
        &dir,
        "archguard.toml",
        &format!(
            "{TOLERATE_SCC_SIZE}baseline = {:?}\n",
            victim.display().to_string()
        ),
    );
    assert_write_refused(&dir, &victim);
}

#[cfg(unix)]
#[test]
fn symlinked_archguard_directory_does_not_redirect_the_baseline() {
    let base = tempdir().expect("tempdir");
    let dir = base.path().join("repo");
    cyclic(&dir);
    let elsewhere = base.path().join("elsewhere");
    write(&elsewhere, "baseline.json", "keep");
    std::os::unix::fs::symlink(&elsewhere, dir.join(".archguard")).expect("symlink");
    assert_write_refused(&dir, &elsewhere.join("baseline.json"));

    // Nor is a baseline read through the link.
    write(
        &elsewhere,
        "baseline.json",
        "{\"schema_version\":1,\"hashes\":[]}",
    );
    for args in [&["baseline", "diff"][..], &["check", "--baseline"][..]] {
        let out = archguard(&dir, args);
        assert_eq!(out.status.code(), Some(3), "{args:?}: {}", report(&out));
    }
}
