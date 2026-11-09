//! The value types of the graph-DB mirror, shared by the `db` backend and
//! its callers whether or not the `db` feature is built.

use serde_json::Value as JsonValue;

/// Result of a read-only openCypher query against the graph-DB mirror
/// (Kap. 8): the returned column names, the rows as JSON values and whether
/// more rows existed than the requested `limit` allowed.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct QueryResult {
    /// Column names in return order, as reported by the database.
    pub columns: Vec<String>,
    /// One entry per returned row, one JSON value per column.
    pub rows: Vec<Vec<JsonValue>>,
    /// `true` if the database had more rows than `limit` permitted.
    pub truncated: bool,
}

/// One row of the T41 `snapshot` node table: the trend numbers the
/// engine derives from a [`Report`] (see `archguard_engine::snapshot_record`).
/// The store deliberately knows no engine types — this is the plain,
/// store-side transfer shape; `cycle_hashes` carries the stable per-cycle
/// hashes joined by `\n`. `id` is the primary key (`<ts>-<sha_or_notag>`):
/// recording the same id again replaces the row.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SnapshotRecord {
    /// Primary key of the row, e.g. `1730000000-abc1234`.
    pub id: String,
    /// Unix seconds at snapshot time.
    pub taken_at: i64,
    /// Short git SHA, or `notag`.
    pub git_sha: String,
    /// Optional `--tag` label.
    pub tag: Option<String>,
    /// Number of cyclic SCCs.
    pub cycles: i64,
    /// Size of the largest SCC.
    pub largest_scc: i64,
    /// Lakos ACD.
    pub acd: f64,
    /// Edge count.
    pub edges: i64,
    /// Stable cycle hashes joined by `\n` (empty string for none).
    pub cycle_hashes: String,
}
