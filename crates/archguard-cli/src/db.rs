//! T33 — the `db` subcommand (Kap. 8): `db sync` mirrors the effective
//! analysis into the graph-DB mirror, `db query` runs a read-only
//! openCypher statement through the query guard against it. Since T41
//! `sync` additionally records the summary as a `snapshot` node — the
//! trend time series, queryable via openCypher.
//!
//! The subcommand is parsed either way so `--help` stays stable; without
//! the `db` build feature the handler prints the build hint and exits
//! with the Kap. 9 usage code (2) instead of touching the database.

use std::path::Path;

use crate::args::DbAction;
#[cfg(not(feature = "db"))]
use crate::output::EXIT_USAGE;

/// The exact hint of a slim build without the `db` feature (T33).
#[cfg(not(feature = "db"))]
const NO_DB_MSG: &str =
    "built without the 'db' feature — install archguard-full or build with --features db";

#[cfg(feature = "db")]
use crate::args::Format;
#[cfg(feature = "db")]
use crate::output::{EXIT_OK, analysis_error, emit, usage};
#[cfg(feature = "db")]
use archguard_engine::store::{GraphStore, LadybugStore, QueryResult, sanitize_query};
#[cfg(feature = "db")]
use archguard_engine::{analyze_run, load_config, snapshot_record};
#[cfg(feature = "db")]
use serde_json::{Value, json};

/// Entry point of `Command::Db`, dispatched in `run` before the pipeline:
/// `query` never analyses, `sync` loads the config and analyses inside.
pub(crate) fn dispatch(action: &DbAction, cli: &crate::args::Cli, root: &Path) -> i32 {
    #[cfg(not(feature = "db"))]
    {
        let _ = (action, cli, root);
        eprintln!("{NO_DB_MSG}");
        EXIT_USAGE
    }
    #[cfg(feature = "db")]
    match action {
        DbAction::Sync { db, .. } => sync(cli, root, db),
        DbAction::Query {
            cypher,
            db,
            limit,
            format,
        } => query(cypher, db, *limit, *format),
    }
}

/// `db sync`: the same effective pipeline as the other commands (see
/// `run`), then an idempotent [`GraphStore::sync`] into `--db`.
#[cfg(feature = "db")]
fn sync(cli: &crate::args::Cli, root: &Path, db_path: &Path) -> i32 {
    let loaded = match load_config(cli.config.as_deref(), root) {
        Ok(loaded) => loaded,
        Err(err) => return analysis_error(err),
    };
    let run = match analyze_run(root, &loaded, cli.command.level()) {
        Ok(run) => run,
        Err(err) => return analysis_error(err),
    };
    let (graph, report) = (run.graph, run.report);
    // `.archguard/graph.db` under a fresh root: the parent must exist.
    if let Some(parent) = db_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        && let Err(err) = std::fs::create_dir_all(parent)
    {
        return analysis_error(err);
    }
    let mut store = match LadybugStore::open(db_path) {
        Ok(store) => store,
        Err(err) => return analysis_error(err),
    };
    if let Err(err) = store.sync(&graph, &report) {
        return analysis_error(err);
    }
    // T41: the mirrored analysis also becomes one row of the Cypher-
    // queryable time series. `sync` never deletes the `snapshot` table,
    // so every sync appends; the id `<ts>-<sha>` upserts (replaces) on
    // collision. A DB failure is an analysis error (exit 3).
    if let Err(err) = store.record_snapshot(&snapshot_record(root, &report, None)) {
        return analysis_error(err);
    }
    if !cli.quiet {
        println!(
            "synced {} modules and {} edges to {}",
            graph.nodes.len(),
            graph.edges.len(),
            db_path.display()
        );
    }
    EXIT_OK
}

/// `db query`: the entry point runs the statement through the query guard
/// itself (Kap. 8.3) — a rejected statement is a usage error (exit 2), a
/// missing or broken database an analysis error (exit 3). Only the *check*
/// happens here; the raw statement goes to the store, which re-runs the
/// guard with `limit + 1` and needs that spare row to flag `truncated` (a
/// pre-capped `LIMIT n` would silently suppress the note). The database
/// opens read-only (F04): a query never creates the mirror — `db sync`
/// does.
#[cfg(feature = "db")]
fn query(cypher: &str, db_path: &Path, limit: usize, format: Format) -> i32 {
    if let Err(err) = sanitize_query(cypher, limit) {
        return usage(err);
    }
    let store = match LadybugStore::open_for_query(db_path) {
        Ok(store) => store,
        Err(err) => return analysis_error(err),
    };
    let result = match store.query(cypher, limit) {
        Ok(result) => result,
        Err(err) => return analysis_error(err),
    };
    if format == Format::Json {
        emit(
            &json!({
                "columns": result.columns,
                "rows": result.rows,
                "truncated": result.truncated,
            })
            .to_string(),
        );
    } else {
        print_table(&result);
    }
    EXIT_OK
}

/// Fixed-width table: header row, dash rule, one row per result row, and
/// a note when the limit cut further rows off. Hand-rolled — no table
/// crate joins the dependency graph for this.
#[cfg(feature = "db")]
fn print_table(result: &QueryResult) {
    let rows: Vec<Vec<String>> = result
        .rows
        .iter()
        .map(|row| row.iter().map(render_cell).collect())
        .collect();
    let mut widths: Vec<usize> = result
        .columns
        .iter()
        .map(|column| column.chars().count())
        .collect();
    for row in &rows {
        for (index, cell) in row.iter().enumerate() {
            if let Some(width) = widths.get_mut(index) {
                *width = (*width).max(cell.chars().count());
            }
        }
    }
    let line = |cells: &[String]| {
        let mut out = String::new();
        for (index, cell) in cells.iter().enumerate() {
            if index > 0 {
                out.push_str("  ");
            }
            let width = widths.get(index).copied().unwrap_or(0);
            out.push_str(&format!("{cell:<width$}"));
        }
        out.trim_end().to_string()
    };
    if !result.columns.is_empty() {
        println!("{}", line(&result.columns));
        println!(
            "{}",
            widths
                .iter()
                .map(|width| "-".repeat(*width))
                .collect::<Vec<_>>()
                .join("  ")
        );
    }
    for row in &rows {
        println!("{}", line(row));
    }
    if result.truncated {
        println!("[truncated: further rows existed; raise --limit]");
    }
}

/// One cell as text: strings unquoted, other JSON values in JSON notation.
#[cfg(feature = "db")]
fn render_cell(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}
