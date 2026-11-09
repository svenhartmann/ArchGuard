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
