//! T33 — the `graph_query` tool (Kap. 10.2): one read-only openCypher
//! statement against the graph-DB mirror, run through the query guard
//! (T32) and the shared response size guard (T27, [`tools::guarded`]).
//!
//! Compiled only with the `db` build feature; without it the tool method
//! in [`crate`] is cfg'd away and the tool is not registered at all.

use std::path::Path;

use rmcp::ErrorData;
// `use rmcp::schemars;` (not the derive directly): the JsonSchema derive
// macro expands to `schemars::…` paths, so the module name must be in scope.
use archguard_engine::store::{GraphStore, LadybugStore, sanitize_query};
use rmcp::schemars;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::server::DEFAULT_LIMIT;
use crate::tools::{guarded, invalid_params, required_str, resolve_file_in_root};

/// Default location of the mirror relative to the server root — the same
/// path the CLI's `db sync` writes to by default (Kap. 8).
const DEFAULT_DB_PATH: &str = ".archguard/graph.db";

/// Input of the `graph_query` tool: `{ cypher, limit?, db? }`. The
/// optional `db` path is an addition beyond Kap. 10.2 that lets callers
/// (and the tests) point at a mirror other than
/// `<root>/.archguard/graph.db`.
#[derive(Debug, Clone, Default, Deserialize, schemars::JsonSchema)]
#[serde(default)]
pub struct GraphQueryParams {
    /// Read-only openCypher statement; must start with MATCH or RETURN.
    /// Statements additionally obey a server-owned size budget and a
    /// denylist of unbounded list/string functions (F05).
    pub cypher: Option<String>,
    /// Row cap; defaults to [`DEFAULT_LIMIT`]. The store applies its
    /// server-owned budgets independently (F05): rows are capped at
    /// `MAX_QUERY_ROWS`, cells at `MAX_CELL_BYTES` bytes, and every
    /// statement runs under a query timeout — a larger caller value
    /// never lifts them, it only lowers them.
    pub limit: Option<usize>,
    /// Database path; defaults to `<root>/.archguard/graph.db`. Relative
    /// paths resolve against the server root and must stay inside it (F02).
    /// The mirror must already exist (created by `archguard db sync`) —
    /// a query never creates a database, schema or files (F04).
    pub db: Option<String>,
}

/// Body of the `graph_query` tool: query guard → read-only open → query →
/// size guard. A statement the guard rejects is `invalid_params`, a
/// missing database or database failure an `internal_error`.
pub fn graph_query(root: &Path, params: GraphQueryParams) -> Result<String, ErrorData> {
    let cypher = required_str(&params.cypher, "cypher")?;
    let limit = params.limit.unwrap_or(DEFAULT_LIMIT);
    // Guard check only — the raw statement goes to the store, which
    // re-runs the guard with `limit + 1` and needs that spare row to
    // flag `truncated` (a pre-capped `LIMIT n` would hide the note).
    sanitize_query(cypher, limit).map_err(|err| invalid_params(format!("{err}")))?;
    // F02: the mirror path (parameter or the in-root default) goes through
    // the server-root policy as an *existing* file — nothing is created
    // before or after. F04: the open is read-only (no schema ensure, no
    // kuzu aux files); a missing mirror is an error, never a creation.
    let db_arg = params.db.as_deref().unwrap_or(DEFAULT_DB_PATH);
    let db_path = resolve_file_in_root(root, Path::new(db_arg), false)?;
    let store = LadybugStore::open_for_query(&db_path)
        .map_err(|err| ErrorData::internal_error(format!("graph db error: {err}"), None))?;
    let result = store
        .query(cypher, limit)
        .map_err(|err| ErrorData::internal_error(format!("graph db query failed: {err}"), None))?;
    let doc: Value = json!({
        "columns": result.columns,
        "rows": result.rows,
        "truncated": result.truncated,
    });
    Ok(guarded(&doc).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::model::ErrorCode;
    use tempfile::tempdir;

    #[test]
    fn graph_query_requires_cypher() {
        let error = graph_query(Path::new("."), GraphQueryParams::default()).unwrap_err();
        assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
        assert!(error.message.contains("cypher"), "{}", error.message);
    }

    #[test]
    fn graph_query_rejects_write_statements_through_the_guard() {
        let params = GraphQueryParams {
            cypher: Some("CREATE (m:module {name: 'x'})".to_string()),
            ..Default::default()
        };
        let error = graph_query(Path::new("."), params).unwrap_err();
        assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
        assert!(
            error.message.contains("query rejected"),
            "{}",
            error.message
        );
    }

    /// Creates an (empty) mirror at `path` through the sanctioned write
    /// path so the read-only query open (F04) has something to open.
    fn create_mirror(path: &std::path::Path) {
        LadybugStore::open(path).expect("create mirror");
    }

    #[test]
    fn graph_query_answers_from_a_synced_but_unused_mirror() {
        // F04: the query open no longer creates the mirror — `db sync`
        // (here: `LadybugStore::open`) does. A schema-complete, data-empty
        // mirror answers with the columns of the statement, no rows, not
        // truncated.
        let base = tempdir().expect("temp dir");
        let root = base.path().join("root");
        std::fs::create_dir_all(&root).expect("mkdir root");
        create_mirror(&root.join("graph.db"));
        let params = GraphQueryParams {
            cypher: Some("MATCH (m:module) RETURN m.name".to_string()),
            db: Some(root.join("graph.db").display().to_string()),
            ..Default::default()
        };
        let answer: Value = serde_json::from_str(&graph_query(&root, params).expect("query ok"))
            .expect("tool answer is JSON");
        assert_eq!(answer["columns"], json!(["m.name"]));
        assert_eq!(answer["rows"], json!([]));
        assert_eq!(answer["truncated"], json!(false));
    }

    /// F05: an extreme caller `limit` (usize::MAX) must stay a valid
    /// request — the store clamps to its server budget and the enforced
    /// LIMIT literal remains a valid int64.
    #[test]
    fn graph_query_survives_an_extreme_limit() {
        let base = tempdir().expect("temp dir");
        let root = base.path().join("root");
        std::fs::create_dir_all(&root).expect("mkdir root");
        create_mirror(&root.join("graph.db"));
        let params = GraphQueryParams {
            cypher: Some("MATCH (m:module) RETURN m.name".to_string()),
            limit: Some(usize::MAX),
            db: Some(root.join("graph.db").display().to_string()),
        };
        let answer: Value =
            serde_json::from_str(&graph_query(&root, params).expect("extreme limit ok"))
                .expect("tool answer is JSON");
        assert_eq!(answer["columns"], json!(["m.name"]));
    }

    /// F05 (adversarial review): when the answer exceeds the MCP
    /// response budget, the shrink must hit the `rows` data list — never
    /// the `columns` definitions, which clients zip `rows` against.
    /// serde_json iterates object keys alphabetically, so without the
    /// explicit `rows` preference `guarded` would have emptied `columns`
    /// while keeping every oversized row.
    #[test]
    fn graph_query_oversized_answer_shrinks_rows_never_columns() {
        use crate::tools::MAX_RESPONSE_BYTES;
        let base = tempdir().expect("temp dir");
        let root = base.path().join("root");
        std::fs::create_dir_all(&root).expect("mkdir root");
        create_mirror(&root.join("graph.db"));
        // Two budget-sized cells: the single row alone is larger than
        // the response budget, so the only answer inside it is no rows
        // — but the column definitions must survive untouched.
        let big = "a".repeat(2048);
        let params = GraphQueryParams {
            cypher: Some(format!("RETURN '{big}' AS a, '{big}' AS b")),
            limit: None,
            db: Some(root.join("graph.db").display().to_string()),
        };
        let raw = graph_query(&root, params).expect("oversized answer ok");
        assert!(
            raw.len() <= MAX_RESPONSE_BYTES,
            "guarded must cap the total, got {}",
            raw.len()
        );
        let answer: Value = serde_json::from_str(&raw).expect("tool answer is JSON");
        assert_eq!(answer["columns"], json!(["a", "b"]), "columns never shrink");
        assert_eq!(answer["rows"], json!([]), "rows is the list that shrinks");
        assert_eq!(answer["truncated"], json!(true), "and it says so");
    }

    /// F02 regression (kept meaningful under F04): an absolute `db`
    /// outside the server root is rejected by the path policy — even when
    /// the database really exists there.
    #[test]
    fn graph_query_rejects_databases_outside_the_root() {
        let base = tempdir().expect("temp dir");
        let root = base.path().join("root");
        let outside = base.path().join("outside");
        std::fs::create_dir_all(&root).expect("mkdir root");
        std::fs::create_dir_all(&outside).expect("mkdir outside");
        let db = outside.join("graph.db");
        create_mirror(&db);
        let params = GraphQueryParams {
            cypher: Some("MATCH (m:module) RETURN m.name".to_string()),
            db: Some(db.display().to_string()),
            ..Default::default()
        };
        let error = graph_query(&root, params).expect_err("dbs outside the root must be rejected");
        assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
        assert!(
            error.message.contains("outside the server root"),
            "{}",
            error.message
        );
    }

    /// F02 regression: a `../` climb and a symlinked database target are
    /// rejected; under F04 the (read-only) open would not create anything
    /// even if the path passed — so the assertions additionally prove that
    /// no outside file is touched or conjured up.
    #[test]
    fn graph_query_rejects_escaping_and_symlinked_database_paths() {
        let base = tempdir().expect("temp dir");
        let root = base.path().join("root");
        let outside = base.path().join("outside");
        std::fs::create_dir_all(&root).expect("mkdir root");
        std::fs::create_dir_all(&outside).expect("mkdir outside");

        // Existing sibling db outside: rejected via the canonical check.
        create_mirror(&outside.join("graph.db"));
        let climbing = GraphQueryParams {
            cypher: Some("MATCH (m:module) RETURN m.name".to_string()),
            db: Some("../outside/graph.db".to_string()),
            ..Default::default()
        };
        let error = graph_query(&root, climbing).expect_err("`../` dbs must be rejected");
        assert!(
            error.message.contains("outside the server root"),
            "{}",
            error.message
        );

        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.join("mirror.db"), root.join("link.db"))
            .expect("symlink");
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(outside.join("mirror.db"), root.join("link.db"))
            .expect("symlink");
        let via_link = GraphQueryParams {
            cypher: Some("MATCH (m:module) RETURN m.name".to_string()),
            db: Some(root.join("link.db").display().to_string()),
            ..Default::default()
        };
        let error = graph_query(&root, via_link).expect_err("symlinked dbs must be rejected");
        assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
        assert!(
            !outside.join("mirror.db").exists(),
            "a rejected graph_query must not create through the symlink"
        );
    }

    /// F04: a query never creates a mirror — a `db` below a missing
    /// directory, or a plain absent file inside the root, is an error and
    /// nothing is conjured up.
    #[test]
    fn graph_query_does_not_create_missing_mirrors() {
        let base = tempdir().expect("temp dir");
        let root = base.path().join("root");
        std::fs::create_dir_all(&root).expect("mkdir root");

        let below_missing = GraphQueryParams {
            cypher: Some("MATCH (m:module) RETURN m.name".to_string()),
            db: Some("missing/graph.db".to_string()),
            ..Default::default()
        };
        let error = graph_query(&root, below_missing).expect_err("a missing db must be rejected");
        assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
        assert!(
            !root.join("missing").exists(),
            "a rejected graph_query must not create parent directories"
        );

        let absent = GraphQueryParams {
            cypher: Some("MATCH (m:module) RETURN m.name".to_string()),
            db: Some("graph.db".to_string()),
            ..Default::default()
        };
        let error = graph_query(&root, absent).expect_err("an absent db must be rejected");
        assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
        assert!(!root.join("graph.db").exists(), "db must not be created");
    }

    /// F02 guard (F04-adapted): the documented default
    /// `<root>/.archguard/graph.db` keeps working — and, if it is missing,
    /// the read-path policy rejects it instead of the open creating it.
    #[test]
    fn graph_query_default_mirror_path_still_works() {
        let base = tempdir().expect("temp dir");
        let root = base.path().join("root");
        std::fs::create_dir_all(root.join(".archguard")).expect("mkdir .archguard");

        let missing_params = GraphQueryParams {
            cypher: Some("MATCH (m:module) RETURN m.name".to_string()),
            db: None,
            ..Default::default()
        };
        let error = graph_query(&root, missing_params)
            .expect_err("a missing default mirror must be reported, not created");
        // The path policy requires an existing file for reads, so the
        // miss surfaces there — before any open could create anything.
        assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
        assert!(error.message.contains("not readable"), "{}", error.message);
        assert!(
            !root.join(".archguard/graph.db").exists(),
            "the default mirror must not be conjured up"
        );

        create_mirror(&root.join(".archguard/graph.db"));
        let params = GraphQueryParams {
            cypher: Some("MATCH (m:module) RETURN m.name".to_string()),
            db: None,
            ..Default::default()
        };
        let answer: Value =
            serde_json::from_str(&graph_query(&root, params).expect("default path query ok"))
                .expect("tool answer is JSON");
        assert_eq!(answer["columns"], json!(["m.name"]));
    }
}
