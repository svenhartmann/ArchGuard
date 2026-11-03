//! `archguard snapshot` / `archguard trend` (T41, Kap. 9): persist the
//! current analysis summary and print the development over time.
//!
//! The heavy lifting lives in [`archguard_engine::write_snapshot`] and
//! [`archguard_engine::read_trend`] (history files under
//! `.archguard/history/`, see the engine module docs); this module is the
//! Kap. 9 surface: exit codes, the fixed-width trend table and its
//! `+`/`-` deltas against the previous row.

use std::path::Path;

use archguard_engine::core::Report;
use archguard_engine::{
    LoadedConfig, SnapshotRow, read_trend, snapshot_scope, write_snapshot_with_scope,
};

use crate::args::{Cli, Format};
use crate::output::{EXIT_OK, analysis_error, emit, format_label, to_pretty_json, usage};

/// Handler of `Command::Snapshot`, called after the normal analysis
/// pipeline: writes one history file and prints its path (unless
/// `--quiet`). An IO or write failure is an analysis error (exit 3).
pub(crate) fn snapshot(
    cli: &Cli,
    root: &Path,
    tag: Option<&str>,
    report: &Report,
    loaded: &LoadedConfig,
) -> i32 {
    match write_snapshot_with_scope(root, report, tag, Some(&snapshot_scope(&loaded.config))) {
        Ok(path) => {
            if !cli.quiet {
                emit(&format!("snapshot written to {}", path.display()));
            }
            EXIT_OK
        }
        Err(err) => analysis_error(err),
    }
}

/// Handler of `Command::Trend`, dispatched in `run` before the config
/// load: reading the history needs no analysis and no config. `last == 0`
/// means "all rows" (see [`archguard_engine::read_trend`]); the
/// subcommand's own `--format` supports `table` (default) and `json`,
/// foreign values are a usage error (exit 2), like `dsm`.
pub(crate) fn trend(cli: &Cli, root: &Path, last: usize, format: Format) -> i32 {
    if !matches!(format, Format::Table | Format::Json) {
        return usage(format!(
            "invalid value '{}' for '--format': trend supports table and json",
            format_label(format)
        ));
    }
    let rows = read_trend(root, last);
    if cli.quiet {
        return EXIT_OK;
    }
    match format {
        Format::Json => {
            let value = serde_json::json!({
                "count": rows.len(),
                "snapshots": serde_json::to_value(&rows)
                    .expect("snapshot rows are plain data and always serializable"),
            });
            emit(&to_pretty_json(&value));
        }
        _ => print_table(&rows),
    }
    EXIT_OK
}

/// Fixed-width trend table (Kap. 9): date, sha/tag, cycles, largest SCC,
/// ACD, edges — the last five columns carry the `+`/`-` delta against the
/// previous row; the oldest row shows bare values, a delta of zero reads
/// `(±0)` / `(±0.000)`. Hand-rolled like `db query`'s table — no table
/// crate joins the dependency graph for this.
fn print_table(rows: &[SnapshotRow]) {
    if rows.is_empty() {
        emit("no snapshots in .archguard/history — run `archguard snapshot` first");
        return;
    }
    let headers = ["DATE", "REF", "CYCLES", "LARGEST", "ACD", "EDGES"];
    let mut cells: Vec<Vec<String>> = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        // `read_trend` returns ascending rows, so the row before is the
        // predecessor in time.
        let prev = if index > 0 {
            Some(&rows[index - 1])
        } else {
            None
        };
        cells.push(vec![
            format_timestamp(row.timestamp),
            row.tag.clone().unwrap_or_else(|| row.git_sha.clone()),
            delta_u32(row.cycles, prev.map(|p| p.cycles)),
            delta_u32(row.largest_scc, prev.map(|p| p.largest_scc)),
            delta_f64(row.acd, prev.map(|p| p.acd)),
            delta_u32(row.edges, prev.map(|p| p.edges)),
        ]);
    }
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in &cells {
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
    println!(
        "{}",
        line(&headers.iter().map(|h| h.to_string()).collect::<Vec<_>>())
    );
    println!(
        "{}",
        widths
            .iter()
            .map(|width| "-".repeat(*width))
            .collect::<Vec<_>>()
            .join("  ")
    );
    for row in &cells {
        println!("{}", line(row));
    }
}

/// Unsigned column with delta against the previous row: `7`, `7 (±0)`,
/// `5 (-2)` or `9 (+2)`; the first row has no predecessor and prints bare.
fn delta_u32(value: u32, prev: Option<u32>) -> String {
    match prev {
        None => value.to_string(),
        Some(prev) => {
            let delta = i64::from(value) - i64::from(prev);
            if delta == 0 {
                format!("{value} (±0)")
            } else if delta > 0 {
                format!("{value} (+{delta})")
            } else {
                format!("{value} ({delta})")
            }
        }
    }
}

/// ACD column with delta, three decimal places like `metrics`; a delta of
/// exactly zero reads `(±0.000)` instead of a signed `+0.000`.
fn delta_f64(value: f64, prev: Option<f64>) -> String {
    match prev {
        None => format!("{value:.3}"),
        Some(prev) if value == prev => format!("{value:.3} (±0.000)"),
        Some(prev) => format!("{value:.3} ({:+.3})", value - prev),
    }
}

/// Unix seconds to `YYYY-MM-DD HH:MM` in UTC — pure integer arithmetic
/// (Hinnant's `civil_from_days`), no datetime crate for this column.
fn format_timestamp(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let rest = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        rest / 3_600,
        (rest % 3_600) / 60
    )
}

/// Days since 1970-01-01 to the proleptic Gregorian date (Howard Hinnant's
/// algorithm), as `(year, month, day)`.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // day of era, [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of year, [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11], March-based
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_u32_renders_sign_and_bare_first_row() {
        assert_eq!(delta_u32(4, None), "4");
        assert_eq!(delta_u32(2, Some(4)), "2 (-2)");
        assert_eq!(delta_u32(6, Some(4)), "6 (+2)");
        assert_eq!(delta_u32(4, Some(4)), "4 (±0)");
    }

    #[test]
    fn delta_f64_uses_three_decimals() {
        assert_eq!(delta_f64(0.5, None), "0.500");
        assert_eq!(delta_f64(0.25, Some(0.5)), "0.250 (-0.250)");
        assert_eq!(delta_f64(0.75, Some(0.5)), "0.750 (+0.250)");
        assert_eq!(delta_f64(0.5, Some(0.5)), "0.500 (±0.000)");
    }

    #[test]
    fn format_timestamp_is_utc() {
        // 1_700_000_000 s = 2023-11-14 22:13:20 UTC.
        assert_eq!(format_timestamp(1_700_000_000), "2023-11-14 22:13");
        assert_eq!(format_timestamp(0), "1970-01-01 00:00");
        // Leap day and the year boundary around it.
        assert_eq!(format_timestamp(1_709_208_000), "2024-02-29 12:00");
    }
}
