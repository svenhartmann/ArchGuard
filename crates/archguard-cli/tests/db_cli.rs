//! T33 — `db sync` + `db query` against the `rs-clean` fixture
//! (Kap. 8/9). The gate is per test, not per file: `cargo test
//! --features db` runs the round trip against a `db`-built binary, the
//! default run checks that the slim binary refuses with the build hint
//! and exit code 2. A file-level `#![cfg]` would let the wrong half run
//! against the wrong binary.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use assert_cmd::Command;
#[cfg(feature = "db")]
use serde_json::Value;
use tempfile::{TempDir, tempdir};

/// The exact slim-build hint (T33) — duplicated on purpose so a silent
/// wording change fails the test.
#[cfg(not(feature = "db"))]
const NO_DB_MSG: &str =
    "built without the 'db' feature — install archguard-full or build with --features db";

/// Workspace fixture, addressed relative to this crate's manifest.
fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/rs-clean")
        .canonicalize()
        .expect("rs-clean fixture")
}

/// Recursive copy — `sync`/`analyze` may write a `.archguard/` cache into
/// the project dir, which must never land in the shared fixture.
fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("create dir");
    for entry in fs::read_dir(from).expect("read dir") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("copy file");
        }
    }
}

/// A private copy of the fixture the database run can write into.
fn rs_clean_project() -> TempDir {
    let dir = tempdir().expect("temp dir");
    copy_tree(&fixture_dir(), dir.path());
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

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Sync a copy of `rs-clean` into a fresh mirror, then query it.
/// Asserts exit codes, the sync summary and the row content of both
/// output formats.
#[cfg(feature = "db")]
#[test]
fn db_sync_then_query_returns_the_fixture_modules() {
    let dir = rs_clean_project();
    let db = dir.path().join("graph.db");

    let sync = archguard(
        &dir,
        &["db", "sync", ".", "--db", db.to_str().expect("utf-8 path")],
    );
    assert!(
        sync.status.code() == Some(0),
        "sync failed: {}",
        stderr(&sync)
    );
    // The fixture's graph: 11 module nodes (6 files and 5 directories) and
    // 7 edges. Rust imports resolve to files, so no `…/*` pseudo-file appears.
    assert!(
        stdout(&sync).starts_with("synced 11 modules and 7 edges to "),
        "unexpected sync summary: {}",
        stdout(&sync)
    );

    let query = archguard(
        &dir,
        &[
            "db",
            "query",
            "MATCH (m:module) RETURN m.name ORDER BY m.name LIMIT 20",
            "--db",
            db.to_str().expect("utf-8 path"),
            "--limit",
            "20",
            "--format",
            "json",
        ],
    );
    assert!(
        query.status.code() == Some(0),
        "query failed: {}",
        stderr(&query)
    );
    let answer: Value = serde_json::from_str(&stdout(&query)).expect("json output");
    assert_eq!(answer["columns"], serde_json::json!(["m.name"]));
    assert_eq!(answer["truncated"], serde_json::json!(false));
    let cells: Vec<String> = answer["rows"]
        .as_array()
        .expect("rows array")
        .iter()
        .flat_map(|row| {
            row.as_array()
                .expect("row array")
                .iter()
                .map(|cell| cell.as_str().expect("string cell").to_string())
                .collect::<Vec<_>>()
        })
        .collect();
    for expected in [
        "alpha/gamma/mod.rs",
        "alpha/mod.rs",
        "beta/delta/mod.rs",
        "main.rs",
    ] {
        assert!(
            cells.iter().any(|cell| cell == expected),
            "{expected} missing from {cells:?}"
        );
    }
}

/// The table format prints the header, a dash rule and the cells
/// unquoted; a rejected statement is a usage error (exit 2).
#[cfg(feature = "db")]
#[test]
fn db_query_table_format_and_guard_rejection() {
    let dir = rs_clean_project();
    let db = dir.path().join("graph.db");
    let sync = archguard(
        &dir,
        &[
            "db",
            "sync",
            ".",
            "--db",
            db.to_str().expect("utf-8 path"),
            "--quiet",
        ],
    );
    assert!(
        sync.status.code() == Some(0),
        "sync failed: {}",
        stderr(&sync)
    );
    assert!(
        stdout(&sync).is_empty(),
        "--quiet must silence sync: {}",
        stdout(&sync)
    );

    let query = archguard(
        &dir,
        &[
            "db",
            "query",
            "MATCH (m:module) RETURN m.name ORDER BY m.name",
            "--db",
            db.to_str().expect("utf-8 path"),
            "--limit",
            "3",
            "--format",
            "table",
        ],
    );
    assert!(
        query.status.code() == Some(0),
        "query failed: {}",
        stderr(&query)
    );
    let out = stdout(&query);
    let mut lines = out.lines();
    assert_eq!(lines.next(), Some("m.name"), "table:\n{out}");
    // Sorted first three rows; column width = widest cell
    // ("alpha/gamma/mod.rs", 18 chars).
    assert_eq!(lines.next(), Some("-".repeat(18).as_str()), "table:\n{out}");
    assert_eq!(lines.next(), Some("alpha"), "table:\n{out}");
    assert_eq!(lines.next(), Some("alpha/gamma"), "table:\n{out}");
    assert_eq!(lines.next(), Some("alpha/gamma/mod.rs"), "table:\n{out}");
    // --limit 3 of the fixture's 11 modules: the truncation note must show.
    assert!(
        out.contains("[truncated: further rows existed; raise --limit]"),
        "table:\n{out}"
    );

    let rejected = archguard(
        &dir,
        &[
            "db",
            "query",
            "MATCH (m:module) DELETE m",
            "--db",
            db.to_str().expect("utf-8 path"),
        ],
    );
    assert_eq!(
        rejected.status.code(),
        Some(2),
        "stderr: {}",
        stderr(&rejected)
    );
    assert!(
        stderr(&rejected).contains("query rejected"),
        "stderr: {}",
        stderr(&rejected)
    );
}

/// T41: `db sync` records the summary as a `snapshot` node, so the time
/// series is queryable via openCypher. The fixture is acyclic and the
/// temp copy lives outside any git repository — `notag` and zeros are
/// the expected row.
#[cfg(feature = "db")]
#[test]
fn db_sync_records_a_queryable_snapshot_row() {
    let dir = rs_clean_project();
    let db = dir.path().join("graph.db");
    let sync = archguard(
        &dir,
        &[
            "db",
            "sync",
            ".",
            "--db",
            db.to_str().expect("utf-8 path"),
            "--quiet",
        ],
    );
    assert!(
        sync.status.code() == Some(0),
        "sync failed: {}",
        stderr(&sync)
    );

    let query = archguard(
        &dir,
        &[
            "db",
            "query",
            "MATCH (s:snapshot) RETURN s.git_sha, s.tag, s.cycles, s.largest_scc, s.taken_at LIMIT 5",
            "--db",
            db.to_str().expect("utf-8 path"),
            "--limit",
            "5",
            "--format",
            "json",
        ],
    );
    assert!(
        query.status.code() == Some(0),
        "query failed: {}",
        stderr(&query)
    );
    let answer: Value = serde_json::from_str(&stdout(&query)).expect("json output");
    let rows = answer["rows"].as_array().expect("rows array");
    assert_eq!(rows.len(), 1, "one sync writes exactly one snapshot row");
    let row = rows[0].as_array().expect("row array");
    assert_eq!(row[0], Value::String("notag".to_string()), "no git → notag");
    assert_eq!(row[1], Value::Null, "db sync records no tag");
    assert_eq!(row[2], serde_json::json!(0), "acyclic fixture: 0 cycles");
    assert_eq!(row[3], serde_json::json!(0), "acyclic fixture: largest 0");
    assert!(
        row[4].as_i64().is_some_and(|taken_at| taken_at > 0),
        "taken_at must be a positive unix second, got {row:?}"
    );
}

/// F04: `db query` opens the mirror read-only — a missing database is an
/// analysis error (exit 3) pointing at `db sync`, and the query call
/// creates no database file.
#[cfg(feature = "db")]
#[test]
fn db_query_on_a_missing_mirror_fails_without_creating_it() {
    let dir = rs_clean_project();
    let db = dir.path().join("graph.db");

    let query = archguard(
        &dir,
        &[
            "db",
            "query",
            "MATCH (m:module) RETURN m.name",
            "--db",
            db.to_str().expect("utf-8 path"),
        ],
    );
    assert_eq!(query.status.code(), Some(3), "stderr: {}", stderr(&query));
    assert!(
        stderr(&query).contains("does not exist"),
        "stderr: {}",
        stderr(&query)
    );
    assert!(!db.exists(), "a failed query must not create {:?}", db);
}

/// F05: `--limit` with the maximum usize stays a valid request — the
/// store clamps to its server-owned budget and the output stays sane.
#[cfg(feature = "db")]
#[test]
fn db_query_with_an_extreme_limit_succeeds() {
    let dir = rs_clean_project();
    let db = dir.path().join("graph.db");
    let sync = archguard(
        &dir,
        &[
            "db",
            "sync",
            ".",
            "--db",
            db.to_str().expect("utf-8 path"),
            "--quiet",
        ],
    );
    assert!(sync.status.code() == Some(0), "sync: {}", stderr(&sync));

    let query = archguard(
        &dir,
        &[
            "db",
            "query",
            "MATCH (m:module) RETURN m.name",
            "--db",
            db.to_str().expect("utf-8 path"),
            "--limit",
            "18446744073709551615",
            "--format",
            "json",
        ],
    );
    assert!(query.status.code() == Some(0), "stderr: {}", stderr(&query));
    let answer: Value = serde_json::from_str(&stdout(&query)).expect("json output");
    assert_eq!(answer["columns"], serde_json::json!(["m.name"]));
    assert_eq!(answer["truncated"], serde_json::json!(false));
}

/// Without the `db` feature the subcommand still parses (stable
/// `--help`) but every action ends with the build hint and exit 2.
#[cfg(not(feature = "db"))]
#[test]
fn db_subcommand_without_the_feature_exits_with_the_build_hint() {
    let dir = rs_clean_project();
    for action in [
        vec!["db", "sync"],
        vec!["db", "query", "MATCH (m:module) RETURN m.name"],
    ] {
        let out = archguard(&dir, &action);
        assert_eq!(out.status.code(), Some(2), "args: {action:?}");
        assert_eq!(stderr(&out), format!("{NO_DB_MSG}\n"), "args: {action:?}");
        assert!(stdout(&out).is_empty(), "args: {action:?}");
    }
}
