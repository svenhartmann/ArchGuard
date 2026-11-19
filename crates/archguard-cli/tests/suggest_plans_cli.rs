//! `archguard suggest --plans`: repair plans per cycle finding, text and JSON.

use std::fs;
use std::process::Output;

use assert_cmd::Command;
use serde_json::Value;
use tempfile::{TempDir, tempdir};

/// a → b → c → a and a ⇄ c: the cut c → a breaks both cycles.
fn fixture() -> TempDir {
    let dir = tempdir().expect("temp dir");
    let files = [
        (
            "src/a.ts",
            "import { b } from \"./b\";\nimport { c } from \"./c\";\nexport const a = b + c;\n",
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
    for (rel, content) in files {
        let path = dir.path().join(rel);
        fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
        fs::write(path, content).expect("write");
    }
    dir
}

fn archguard(dir: &TempDir, args: &[&str]) -> Output {
    Command::cargo_bin("archguard")
        .expect("archguard binary")
        .current_dir(dir.path())
        .args(args)
        .output()
        .expect("run archguard")
}

#[test]
fn plans_print_as_text() {
    let dir = fixture();
    let out = archguard(&dir, &["suggest", "--plans"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("scc-1"), "{text}");
    assert!(text.contains("option 1: cycle removed"), "{text}");
    assert!(text.contains("cut c.ts -> a.ts"), "{text}");
    assert!(text.contains("search complete"), "{text}");
}

#[test]
fn plans_print_as_json_and_unknown_cycles_fail() {
    let dir = fixture();
    let out = archguard(
        &dir,
        &["suggest", "--plans", "--cycle", "scc-1", "--format", "json"],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let value: Value = serde_json::from_slice(&out.stdout).expect("JSON");
    let first = &value[0];
    assert_eq!(first["id"], "scc-1");
    assert_eq!(first["search"]["complete"], true);
    let plan = &first["plans"][0];
    assert_eq!(plan["complete"], true);
    assert_eq!(plan["ops"][0]["cut_edge"]["from"], "c.ts");
    let unknown = archguard(&dir, &["suggest", "--plans", "--cycle", "scc-9"]);
    assert_eq!(unknown.status.code(), Some(2));
}
