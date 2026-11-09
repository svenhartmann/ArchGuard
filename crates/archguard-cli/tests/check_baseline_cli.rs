//! `archguard check --baseline` with the stock rules: a baselined cycle
//! is accepted together with the SCC-size gate for the same members, a
//! new cycle still fails; and `check` rejects formats it cannot render.

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

#[test]
fn baselined_cycle_passes_with_the_default_scc_size_gate() {
    let dir = tempdir().expect("tempdir");
    cyclic(dir.path());
    let out = archguard(dir.path(), &["check"]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "cycle fails unbaselined: {}",
        report(&out)
    );

    let out = archguard(dir.path(), &["baseline", "write"]);
    assert_eq!(out.status.code(), Some(0), "{}", report(&out));
    let out = archguard(dir.path(), &["check", "--baseline"]);
    assert_eq!(out.status.code(), Some(0), "baselined: {}", report(&out));

    // A new cycle is neither in the baseline nor accepted by size.
    write(
        dir.path(),
        "src/x.ts",
        "import { y } from \"./y\";\nexport const x = y;\n",
    );
    write(
        dir.path(),
        "src/y.ts",
        "import { x } from \"./x\";\nexport const y = x;\n",
    );
    let out = archguard(dir.path(), &["check", "--baseline"]);
    assert_eq!(out.status.code(), Some(1), "new cycle: {}", report(&out));
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
