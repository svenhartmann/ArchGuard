//! `archguard-store` — L1: graph-DB mirror and read-only openCypher queries
//! with a query guard. Lands in T31–T34.
//!
//! The database backend `LadybugStore` sits behind the cargo feature `db`
//! (pinned `lbug = "=0.22.1"`); the value types and the query sanitizer do
//! not. `LadybugStore` is the only backend, so it carries `sync`, `query`
//! and `record_snapshot` as inherent methods instead of behind a trait.

mod api;
mod sanitize;

pub use api::{QueryResult, SnapshotRecord};

pub use sanitize::{MAX_STATEMENT_BYTES, sanitize_query};

#[cfg(feature = "db")]
mod db;

#[cfg(feature = "db")]
pub use db::{LadybugStore, MAX_CELL_BYTES, MAX_QUERY_ROWS, QUERY_TIMEOUT_MS};
