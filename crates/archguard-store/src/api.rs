//! The storage contract: the [`GraphStore`] trait, its value types and the no-op store.

use archguard_core::{ArchguardError, Graph, Report};
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

/// The graph-DB mirror behind a trait, so the backend stays exchangeable
/// (Kap. 8.1): `sync` mirrors a [`Graph`] plus [`Report`] into the database,
/// `query` runs a read-only openCypher statement.
///
/// `query` is read-only by contract; the CLI/MCP entry points additionally
/// run the statement through the query guard (T33) before calling it.
pub trait GraphStore {
    /// Mirror `g` and `r` into the store. Implementations are idempotent:
    /// a second `sync` with the same inputs yields the same state. `sync`
    /// never touches the T41 `snapshot` table — the time series survives a
    /// re-sync.
    fn sync(&mut self, g: &Graph, r: &Report) -> Result<(), ArchguardError>;

    /// Run a read-only openCypher `cypher` statement, returning at most
    /// `limit` rows and flagging `truncated` if there were more. Callers
    /// that only read must open the file backend via
    /// `LadybugStore::open_for_query` (db feature, F04) so a query
    /// creates no database, schema or files.
    fn query(&self, cypher: &str, limit: usize) -> Result<QueryResult, ArchguardError>;

    /// Upsert one [`SnapshotRecord`] into the `snapshot` table (T41), so
    /// the trend time series becomes Cypher-queryable. The default is a
    /// no-op: stores without a time series (like [`NullStore`]) keep
    /// working, only the Ladybug backend persists.
    fn record_snapshot(&mut self, _record: &SnapshotRecord) -> Result<(), ArchguardError> {
        Ok(())
    }
}

/// A [`GraphStore`] that keeps no state: `sync` does nothing, `query` always
/// returns an empty [`QueryResult`]. Used by the slim build (no `db` feature)
/// so callers do not need to branch on the feature themselves.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NullStore;

impl GraphStore for NullStore {
    fn sync(&mut self, _g: &Graph, _r: &Report) -> Result<(), ArchguardError> {
        Ok(())
    }

    fn query(&self, _cypher: &str, _limit: usize) -> Result<QueryResult, ArchguardError> {
        Ok(QueryResult::default())
    }
}
