//! `archguard-store` — L1: `GraphStore` trait, graph-DB mirror and read-only
//! openCypher queries with a query guard. Lands in T31–T34.
//!
//! The database backend sits behind the cargo feature `db`; the trait and the
//! in-memory store do not (ARCHITECTURE.md Kap. 8.1: trait instead of a
//! direct dependency, pinned `lbug = "=0.22.1"` behind the optional feature).

mod api;
mod sanitize;

pub use api::{GraphStore, NullStore, QueryResult, SnapshotRecord};

pub use sanitize::{MAX_STATEMENT_BYTES, sanitize_query};

#[cfg(feature = "db")]
mod db;

#[cfg(feature = "db")]
pub use db::{LadybugStore, MAX_CELL_BYTES, MAX_QUERY_ROWS, QUERY_TIMEOUT_MS};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_store_sync_is_a_noop_ok() {
        let mut store = NullStore;
        let graph = archguard_core::Graph {
            semantic: None,
            review_kpis: None,
            schema_version: 1,
            root: "/abs".to_string(),
            nodes: Vec::new(),
            edges: Vec::new(),
        };
        let report = archguard_core::Report {
            semantic: None,
            review_kpis: None,
            schema_version: 1,
            summary: archguard_core::Summary {
                nodes: 0,
                edges: 0,
                sccs: 0,
                largest_scc: 0,
                layer_violations: 0,
                status: archguard_core::Status::Pass,
                ccd: 0,
                acd: 0.0,
                nccd: 0.0,
            },
            config: Default::default(),
            findings: Vec::new(),
            metrics: Default::default(),
        };
        assert!(store.sync(&graph, &report).is_ok());
    }

    #[test]
    fn null_store_record_snapshot_is_a_noop_ok() {
        let mut store = NullStore;
        let record = SnapshotRecord {
            id: "1730000000-notag".to_string(),
            taken_at: 1_730_000_000,
            git_sha: "notag".to_string(),
            tag: Some("baseline".to_string()),
            cycles: 2,
            largest_scc: 3,
            acd: 1.5,
            edges: 7,
            cycle_hashes: "aa\nbb".to_string(),
        };
        // The default trait implementation absorbs the record silently.
        assert!(store.record_snapshot(&record).is_ok());
    }

    #[test]
    fn null_store_query_is_always_empty() {
        let store = NullStore;
        let result = store.query("MATCH (m:module) RETURN m.name", 10).unwrap();
        assert_eq!(result.columns, Vec::<String>::new());
        assert!(result.rows.is_empty());
        assert!(!result.truncated);
        // Limit 0 behaves the same — the store never holds rows.
        assert_eq!(
            store.query("MATCH (m:module) RETURN m.name", 0).unwrap(),
            result
        );
    }
}
