//! The `db` backend: [`LadybugStore`], the file-based graph-DB mirror on top
//! of the pinned `lbug = "=0.22.1"` crate (LadybugDB, the maintained Kuzu fork;
//! storage format 49, see ARCHITECTURE.md Kap. 8.1/8.2).
//!
//! lbug's `Connection` borrows the `Database`, so the store keeps only the
//! `Database` and creates a fresh `Connection` per operation.

use std::collections::{BTreeSet, HashSet};
use std::path::Path;

use archguard_core::{
    ArchguardError, Finding, FindingKind, Graph, ModuleMetrics, Report, Severity,
};
use lbug::{Connection, Database, LogicalType, SystemConfig, Value};
use serde_json::{Number, Value as JsonValue};

use crate::api::{GraphStore, QueryResult, SnapshotRecord};

/// Schema of Kap. 8.2, in creation order (node tables before their rel
/// tables). `(table name, CREATE statement)`.
const SCHEMA_STATEMENTS: &[(&str, &str)] = &[
    (
        "module",
        "CREATE NODE TABLE module(name STRING PRIMARY KEY, lang STRING, layer STRING, \
         fan_in INT64, fan_out INT64, instability DOUBLE, in_cycle BOOLEAN);",
    ),
    (
        "depends_on",
        "CREATE REL TABLE depends_on(FROM module TO module, ref_count INT64, cyclic BOOLEAN);",
    ),
    (
        "finding",
        "CREATE NODE TABLE finding(id STRING PRIMARY KEY, kind STRING, severity STRING, \
         detail STRING);",
    ),
    (
        "involves",
        "CREATE REL TABLE involves(FROM finding TO module);",
    ),
    // T41 trend time series (Kap. 9 `snapshot`/`trend`): one row per
    // recorded snapshot, filled by `record_snapshot` — deliberately not
    // part of `sync`'s delete list, so the history survives a re-sync.
    // `cycle_hashes` stores the stable per-cycle hashes joined by '\n'.
    (
        "snapshot",
        "CREATE NODE TABLE snapshot(id STRING PRIMARY KEY, taken_at INT64, git_sha STRING, \
         tag STRING, cycles INT64, largest_scc INT64, acd DOUBLE, edges INT64, \
         cycle_hashes STRING);",
    ),
];

/// Map a lbug error onto the shared error type. [`ArchguardError`] has no
/// dedicated storage variant (core is frozen), so database failures surface
/// as analysis errors carrying the lbug message.
fn db_error(err: lbug::Error) -> ArchguardError {
    ArchguardError::Analysis(format!("graph db error: {err}"))
}

/// Like [`db_error`] for `Database::new`: when lbug cannot read the file's
/// format — a mirror written by the former `kuzu = "=0.11.3"` backend, or
/// by an lbug release with another storage version — the message names the
/// remedy. The file is never removed here: its `snapshot` rows are the T41
/// trend history, so deleting it stays the user's decision.
fn open_error(path: &Path, err: lbug::Error) -> ArchguardError {
    let message = err.to_string();
    if message.contains("not a valid Lbug database file") || message.contains("different version") {
        return ArchguardError::Analysis(format!(
            "graph db error: {message} {} uses a format this LadybugDB build cannot read \
             (e.g. written by the former Kuzu 0.11.3 backend); delete it together with its \
             `.wal` file and run `archguard db sync` again (its recorded trend snapshots are lost)",
            path.display()
        ));
    }
    db_error(err)
}

/// Server-owned maximum rows for a single query (F05). A caller-supplied
/// `limit` — CLI `--limit`, MCP `limit`, or `usize::MAX` — can only lower
/// this, never raise it; reaching the budget itself reports `truncated`.
pub const MAX_QUERY_ROWS: usize = 1_000;

/// Server-owned serialization budget per result cell (F05). The budget
/// covers string bytes **and** every converted node (each scalar, list
/// item, map entry and property costs at least one budget unit), so
/// neither a huge scalar nor a million-item list converts beyond it —
/// exhausted budget renders as a visible `…` and the walk stops there.
/// Column definitions are never dropped, and the MCP response guard caps
/// the total on top. Note what the budget does *not* cover: the engine
/// materializes the full result before this conversion ever runs (the
/// timeout interrupts only at lbug's coarse check points), so the query
/// guard additionally blocks the known list/string amplifier functions
/// and deployment-level resource limits remain the backstop.
pub const MAX_CELL_BYTES: usize = 2_048;

/// lbug execution timeout for queries (F05). The row/cell budgets bound
/// memory and serialization; only the engine's own timeout stops
/// expensive *execution* (row truncation is not a CPU limit).
pub const QUERY_TIMEOUT_MS: u64 = 5_000;

/// Nesting bound for JSON conversion of container cells (F05); deeper
/// structures from a hostile database become a `…` marker.
const MAX_JSON_DEPTH: usize = 16;

/// A [`GraphStore`] mirroring graph and report into a LadybugDB database
/// file (Kap. 8.2 schema). Open an existing or new database with
/// [`LadybugStore::open`]; `sync` clears all rows and bulk-inserts — the
/// T41 `snapshot` table excepted —, `query` runs read-only openCypher and
/// [`GraphStore::record_snapshot`] upserts one trend row.
pub struct LadybugStore {
    /// lbug's `Connection` borrows the `Database`, so only the `Database` is
    /// stored and a fresh `Connection` is created per operation.
    db: Database,
    /// lbug execution timeout applied to every `query` (F05);
    /// [`LadybugStore::set_query_timeout_ms`] adjusts it (tests).
    query_timeout_ms: u64,
}

impl LadybugStore {
    /// Opens the database at `path`, creating it (and the Kap. 8.2 schema)
    /// if missing.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ArchguardError> {
        Self::open_with(path.as_ref(), SystemConfig::default())
    }

    /// Like [`LadybugStore::open`], with an explicit lbug `SystemConfig`.
    pub fn open_with(path: &Path, config: SystemConfig) -> Result<Self, ArchguardError> {
        let db = Database::new(path, config).map_err(|err| open_error(path, err))?;
        let store = Self {
            db,
            query_timeout_ms: QUERY_TIMEOUT_MS,
        };
        store.ensure_schema()?;
        Ok(store)
    }

    /// Open an **existing** database at `path` for read-only queries
    /// (F04): lbug runs in `read_only` mode and the Kap. 8.2 schema is
    /// *not* ensured — a query call never creates a database, tables or
    /// auxiliary files. A missing path is an error pointing at
    /// `db sync`; creation and syncing stay with [`LadybugStore::open`].
    pub fn open_for_query(path: impl AsRef<Path>) -> Result<Self, ArchguardError> {
        let path = path.as_ref();
        if !path.exists() {
            return Err(ArchguardError::Analysis(format!(
                "graph db {} does not exist; run `archguard db sync` first",
                path.display()
            )));
        }
        let db = Database::new(path, SystemConfig::default().read_only(true))
            .map_err(|err| open_error(path, err))?;
        Ok(Self {
            db,
            query_timeout_ms: QUERY_TIMEOUT_MS,
        })
    }

    /// Adjust the lbug execution timeout used by subsequent `query` calls
    /// (F05). `0` disables it — for trusted in-process paths only; CLI and
    /// MCP keep [`QUERY_TIMEOUT_MS`].
    pub fn set_query_timeout_ms(&mut self, timeout_ms: u64) {
        self.query_timeout_ms = timeout_ms;
    }

    /// Creates every Kap. 8.2 table that does not exist yet.
    fn ensure_schema(&self) -> Result<(), ArchguardError> {
        let conn = Connection::new(&self.db).map_err(db_error)?;
        let mut result = conn
            .query("CALL SHOW_TABLES() RETURN name;")
            .map_err(db_error)?;
        let mut existing = HashSet::new();
        for row in result.by_ref() {
            if let Some(Value::String(name)) = row.first() {
                existing.insert(name.to_lowercase());
            }
        }
        for (table, ddl) in SCHEMA_STATEMENTS {
            if !existing.contains(*table) {
                conn.query(ddl).map_err(db_error)?;
            }
        }
        Ok(())
    }

    /// Inserts (or replaces the row values of) one module node. `metrics`
    /// supplies fan-in/out, instability and `in_cycle` when the report has
    /// metrics for this name.
    fn execute_module(
        &self,
        conn: &Connection<'_>,
        stmt: &mut lbug::PreparedStatement,
        name: &str,
        lang: Option<&str>,
        metrics: Option<&ModuleMetrics>,
    ) -> Result<(), ArchguardError> {
        conn.execute(
            stmt,
            vec![
                ("name", Value::String(name.to_string())),
                (
                    "lang",
                    lang.map_or_else(
                        || null_value(LogicalType::String),
                        |lang| Value::String(lang.to_string()),
                    ),
                ),
                ("layer", null_value(LogicalType::String)),
                (
                    "fan_in",
                    Value::Int64(metrics.map_or(0, |m| i64::from(m.fan_in))),
                ),
                (
                    "fan_out",
                    Value::Int64(metrics.map_or(0, |m| i64::from(m.fan_out))),
                ),
                (
                    "instability",
                    Value::Double(metrics.map_or(0.0, |m| f64::from(m.instability))),
                ),
                ("in_cycle", Value::Bool(metrics.is_some_and(|m| m.in_cycle))),
            ],
        )
        .map_err(db_error)?;
        Ok(())
    }
}

impl GraphStore for LadybugStore {
    /// Idempotent full refresh (Kap. 8.2): delete all rows of the mirror
    /// tables (the T41 `snapshot` table is deliberately not among them),
    /// then bulk-insert modules, `depends_on` edges (`ref_count` = the
    /// [`archguard_core::Edge::weight`] total, F12 — never the capped
    /// sample; `cyclic` = both endpoints in one cycle finding), findings
    /// and `involves` rels.
    ///
    /// Every graph node — file nodes included — lands in the `module` table,
    /// because Kap. 8.2 keeps a single node table for the analysed zoom
    /// level; `lang` carries the file language. Findings may name modules
    /// that the graph does not contain (e.g. another zoom level): placeholder
    /// module rows are inserted so `involves` stays complete.
    fn sync(&mut self, g: &Graph, r: &Report) -> Result<(), ArchguardError> {
        let conn = Connection::new(&self.db).map_err(db_error)?;
        for stmt in [
            "MATCH ()-[d:depends_on]->() DELETE d;",
            "MATCH ()-[i:involves]->() DELETE i;",
            "MATCH (m:module) DELETE m;",
            "MATCH (f:finding) DELETE f;",
        ] {
            conn.query(stmt).map_err(db_error)?;
        }

        let module_stmt = "CREATE (m:module {name: $name, lang: $lang, layer: $layer, \
                           fan_in: $fan_in, fan_out: $fan_out, instability: $instability, \
                           in_cycle: $in_cycle});";
        let mut module_stmt = conn.prepare(module_stmt).map_err(db_error)?;

        let graph_names: HashSet<&str> = g.nodes.iter().map(|n| n.name.as_str()).collect();
        for node in &g.nodes {
            self.execute_module(
                &conn,
                &mut module_stmt,
                &node.name,
                node.lang.as_deref(),
                r.metrics.get(&node.name),
            )?;
        }

        // An edge lies on a cycle exactly when both endpoints are in the same
        // cycle finding (same SCC ⇒ a back-path exists).
        let cycle_pairs = cyclic_edge_pairs(r);
        let mut edge_stmt = conn
            .prepare(
                "MATCH (a:module {name: $from}), (b:module {name: $to}) \
                 CREATE (a)-[:depends_on {ref_count: $ref_count, cyclic: $cyclic}]->(b);",
            )
            .map_err(db_error)?;
        for edge in &g.edges {
            let from = g.node_name(edge.from).ok_or_else(|| {
                ArchguardError::Analysis(format!("edge references unknown node {:?}", edge.from))
            })?;
            let to = g.node_name(edge.to).ok_or_else(|| {
                ArchguardError::Analysis(format!("edge references unknown node {:?}", edge.to))
            })?;
            let cyclic = {
                let pair = if from <= to {
                    (from.to_string(), to.to_string())
                } else {
                    (to.to_string(), from.to_string())
                };
                cycle_pairs.contains(&pair)
            };
            conn.execute(
                &mut edge_stmt,
                vec![
                    ("from", Value::String(from.to_string())),
                    ("to", Value::String(to.to_string())),
                    ("ref_count", Value::Int64(edge.weight() as i64)),
                    ("cyclic", Value::Bool(cyclic)),
                ],
            )
            .map_err(db_error)?;
        }

        let mut finding_stmt = conn
            .prepare(
                "CREATE (f:finding {id: $id, kind: $kind, severity: $severity, \
                 detail: $detail});",
            )
            .map_err(db_error)?;
        for finding in &r.findings {
            conn.execute(
                &mut finding_stmt,
                vec![
                    ("id", Value::String(finding.id.clone())),
                    ("kind", Value::String(kind_name(finding.kind).to_string())),
                    (
                        "severity",
                        Value::String(severity_name(finding.severity).to_string()),
                    ),
                    ("detail", Value::String(finding_detail(finding))),
                ],
            )
            .map_err(db_error)?;
        }

        // Modules named by findings but absent from the graph.
        let mut placeholders: BTreeSet<&str> = BTreeSet::new();
        for finding in &r.findings {
            for name in &finding.nodes {
                if !graph_names.contains(name.as_str()) {
                    placeholders.insert(name.as_str());
                }
            }
        }
        for name in placeholders {
            self.execute_module(&conn, &mut module_stmt, name, None, r.metrics.get(name))?;
        }

        let mut involves_stmt = conn
            .prepare(
                "MATCH (f:finding {id: $id}), (m:module {name: $name}) \
                 CREATE (f)-[:involves]->(m);",
            )
            .map_err(db_error)?;
        for finding in &r.findings {
            for name in &finding.nodes {
                conn.execute(
                    &mut involves_stmt,
                    vec![
                        ("id", Value::String(finding.id.clone())),
                        ("name", Value::String(name.clone())),
                    ],
                )
                .map_err(db_error)?;
            }
        }
        Ok(())
    }

    /// Runs `cypher` and returns at most `min(limit, MAX_QUERY_ROWS)`
    /// rows; `truncated` is set when more rows existed **or** the
    /// server-owned row budget was the binding cap (F05). Cells are
    /// serialized within [`MAX_CELL_BYTES`] and the statement runs under
    /// the connection's [`QUERY_TIMEOUT_MS`] execution timeout. The
    /// CLI/MCP path additionally guards the statement (T33/T32/F04).
    /// Edge: a caller whose *own* `LIMIT` literal already equals the cap
    /// leaves no spare row for the probe, so further existing rows stay
    /// unflagged — `truncated` reports the server budget, not the
    /// statement's own limit.
    fn query(&self, cypher: &str, limit: usize) -> Result<QueryResult, ArchguardError> {
        // F05: caller `limit` can only lower the server-owned budget,
        // never raise it; zero stays zero. The enforced LIMIT still gets
        // one spare row so `truncated` can be detected below; i64::MAX
        // keeps the appended literal a valid int64 even for limit =
        // usize::MAX.
        let effective = limit.min(MAX_QUERY_ROWS);
        let max_limit = effective.saturating_add(1).min(i64::MAX as usize);
        let cypher = crate::sanitize::sanitize_query(cypher, max_limit)?;
        let conn = Connection::new(&self.db).map_err(db_error)?;
        if self.query_timeout_ms > 0 {
            conn.set_query_timeout(self.query_timeout_ms);
        }
        let mut result = conn.query(&cypher).map_err(|err| {
            // lbug stops an expired query with its interrupt exception,
            // whose message is the exact sentence "Interrupted."
            // (lbug-src interrupt.h). Match that verbatim — a looser
            // case-insensitive "interrupt" substring would also label
            // genuine IO errors ("Interrupted system call", EINTR) as
            // timeouts and send the caller chasing the wrong remediation.
            if err.to_string().contains("Interrupted.") {
                ArchguardError::Analysis(format!(
                    "query timed out after {} ms; narrow the statement",
                    self.query_timeout_ms
                ))
            } else {
                db_error(err)
            }
        })?;
        let columns = result.get_column_names();
        let mut rows: Vec<Vec<JsonValue>> = Vec::new();
        let mut truncated = false;
        for row in result.by_ref().take(effective.saturating_add(1)) {
            if rows.len() == effective {
                truncated = true;
                break;
            }
            rows.push(row.into_iter().map(json_cell).collect());
        }
        Ok(QueryResult {
            columns,
            rows,
            truncated,
        })
    }

    /// Upsert one trend row into the T41 `snapshot` table: delete the row
    /// with the same primary key, then create the new one — a plain
    /// replace, so re-recording an id is idempotent. `tag` stays NULL when
    /// absent. `sync` never touches this table.
    fn record_snapshot(&mut self, record: &SnapshotRecord) -> Result<(), ArchguardError> {
        let conn = Connection::new(&self.db).map_err(db_error)?;
        let mut delete = conn
            .prepare("MATCH (s:snapshot {id: $id}) DELETE s;")
            .map_err(db_error)?;
        conn.execute(&mut delete, vec![("id", Value::String(record.id.clone()))])
            .map_err(db_error)?;
        let mut insert = conn
            .prepare(
                "CREATE (s:snapshot {id: $id, taken_at: $taken_at, git_sha: $git_sha, \
                 tag: $tag, cycles: $cycles, largest_scc: $largest_scc, acd: $acd, \
                 edges: $edges, cycle_hashes: $cycle_hashes});",
            )
            .map_err(db_error)?;
        conn.execute(
            &mut insert,
            vec![
                ("id", Value::String(record.id.clone())),
                ("taken_at", Value::Int64(record.taken_at)),
                ("git_sha", Value::String(record.git_sha.clone())),
                (
                    "tag",
                    record.tag.as_ref().map_or_else(
                        || null_value(LogicalType::String),
                        |tag| Value::String(tag.clone()),
                    ),
                ),
                ("cycles", Value::Int64(record.cycles)),
                ("largest_scc", Value::Int64(record.largest_scc)),
                ("acd", Value::Double(record.acd)),
                ("edges", Value::Int64(record.edges)),
                ("cycle_hashes", Value::String(record.cycle_hashes.clone())),
            ],
        )
        .map_err(db_error)?;
        Ok(())
    }
}

/// `Value::Null` for a nullable column of the given type.
fn null_value(logical_type: LogicalType) -> Value {
    Value::Null(logical_type)
}

/// Unordered node-name pairs that lie inside one cycle finding.
fn cyclic_edge_pairs(r: &Report) -> HashSet<(String, String)> {
    let mut pairs = HashSet::new();
    for finding in &r.findings {
        if finding.kind != FindingKind::Cycle {
            continue;
        }
        for a in &finding.nodes {
            for b in &finding.nodes {
                if a != b {
                    pairs.insert(if a <= b {
                        (a.clone(), b.clone())
                    } else {
                        (b.clone(), a.clone())
                    });
                }
            }
        }
    }
    pairs
}

/// Stable, machine-readable finding kind name (matches the core serde names).
fn kind_name(kind: FindingKind) -> &'static str {
    match kind {
        FindingKind::Cycle => "cycle",
        FindingKind::LayerViolation => "layer_violation",
        FindingKind::MetricThreshold => "metric_threshold",
        FindingKind::LayerUnranked => "layer_unranked",
        FindingKind::DependencyRule => "dependency_rule",
        FindingKind::OwnershipRule => "ownership_rule",
        FindingKind::Orphan => "orphan",
        FindingKind::TargetArchitecture => "target_architecture",
    }
}

/// Stable severity name (matches the core serde names).
fn severity_name(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Info => "info",
    }
}

/// Existing kinds keep their legacy one-line summary. The new logical target
/// kind stores Finding JSON so its declaration message and source evidence
/// survive synchronization without changing the database schema.
fn finding_detail(finding: &Finding) -> String {
    if finding.kind == FindingKind::TargetArchitecture {
        return serde_json::to_string(finding).expect("target architecture finding serializes");
    }
    format!(
        "nodes=[{}]; edge_count={}; cycles_sampled={}",
        finding.nodes.join(", "),
        finding.edge_count,
        finding.cycles_sampled
    )
}

/// Convert one result cell with the F05 byte budget: at most
/// [`MAX_CELL_BYTES`] worth of string bytes per cell (scalars and every
/// nested container share one budget), exhausted budget renders as `…`.
/// Column definitions live outside the cell, so they survive untouched.
fn json_cell(value: Value) -> JsonValue {
    let mut budget = MAX_CELL_BYTES;
    json_value(value, &mut budget, 0)
}

/// Converts a lbug `Value` into a `serde_json::Value`. Complex values
/// (nodes, rels, structs, maps) become JSON objects, temporal and other
/// exotic values fall back to their `Display` rendering as strings —
/// all string bytes come out of the shared `budget` (F05), and nesting
/// deeper than [`MAX_JSON_DEPTH`] degrades to `…`.
fn json_value(value: Value, budget: &mut usize, depth: usize) -> JsonValue {
    if depth >= MAX_JSON_DEPTH {
        return JsonValue::String("…".to_string());
    }
    // F05 (review): the budget must gate conversion work, not just
    // string bytes — every converted node pays at least one unit, and a
    // spent budget stops the walk without touching further children
    // (a million-integer list converts ~MAX_CELL_BYTES items, not a
    // million).
    if *budget == 0 {
        return JsonValue::String("…".to_string());
    }
    *budget -= 1;
    match value {
        Value::Null(_) => JsonValue::Null,
        Value::Bool(b) => JsonValue::Bool(b),
        Value::Int8(v) => JsonValue::from(i64::from(v)),
        Value::Int16(v) => JsonValue::from(i64::from(v)),
        Value::Int32(v) => JsonValue::from(i64::from(v)),
        Value::Int64(v) => JsonValue::from(v),
        Value::UInt8(v) => JsonValue::from(u64::from(v)),
        Value::UInt16(v) => JsonValue::from(u64::from(v)),
        Value::UInt32(v) => JsonValue::from(u64::from(v)),
        Value::UInt64(v) => JsonValue::from(v),
        Value::Float(v) => float_value(f64::from(v)),
        Value::Double(v) => float_value(v),
        Value::String(s) => string_cell(s, budget),
        Value::List(_, items) | Value::Array(_, items) => {
            JsonValue::Array(budgeted_items(items.into_iter(), budget, depth))
        }
        Value::Struct(fields) => object_value(&fields, budget, depth),
        Value::Node(node) => object_value(node.get_properties(), budget, depth),
        Value::Rel(rel) => object_value(rel.get_properties(), budget, depth),
        // A traversed path: expose its nodes and rels like nested lists —
        // never the full `Display` rendering of the whole sequence (F05).
        Value::RecursiveRel { nodes, rels } => JsonValue::Array(budgeted_items(
            nodes
                .into_iter()
                .map(Value::Node)
                .chain(rels.into_iter().map(Value::Rel)),
            budget,
            depth,
        )),
        Value::Map(_, entries) => {
            let mut map = serde_json::Map::new();
            for (k, v) in entries {
                if *budget == 0 {
                    map.insert("…".to_string(), JsonValue::String("…".to_string()));
                    break;
                }
                let key = json_value(k, budget, depth + 1).to_string();
                let value = json_value(v, budget, depth + 1);
                map.insert(key, value);
            }
            JsonValue::Object(map)
        }
        // Blobs render as hex (4–5 chars per byte through Debug), so cut
        // the raw bytes to the budget before rendering — a planted 256 MB
        // blob must not become a gigabyte-scale string first (F05).
        Value::Blob(mut bytes) => {
            if bytes.len() > *budget {
                bytes.truncate(*budget);
                *budget = 0;
                JsonValue::String(format!("{bytes:x?}…"))
            } else {
                *budget -= bytes.len();
                JsonValue::String(format!("{bytes:x?}"))
            }
        }
        // Dates, timestamps, intervals, UUIDs, decimals, internal ids,
        // unions: stable string rendering beats dropping the value.
        other => string_cell(other.to_string(), budget),
    }
}

/// Budgeted string: full copy while the budget lasts, otherwise cut at a
/// char boundary and closed with the `…` marker (F05 — an oversized cell
/// says so instead of silently bloating the response).
fn string_cell(mut s: String, budget: &mut usize) -> JsonValue {
    if s.len() <= *budget {
        *budget -= s.len();
        return JsonValue::String(s);
    }
    let mut cut = *budget;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    s.truncate(cut);
    s.push('…');
    *budget = 0;
    JsonValue::String(s)
}

/// Non-finite floats have no JSON number; they degrade to their `Display`.
fn float_value(v: f64) -> JsonValue {
    Number::from_f64(v).map_or_else(|| JsonValue::String(v.to_string()), JsonValue::Number)
}

/// Budgeted conversion of a container's items: converts while the
/// budget lasts and closes the list with a single `…` marker once it is
/// spent — the remaining (possibly millions of) items are never
/// visited, cloned or collected (F05, review follow-up).
fn budgeted_items<I>(items: I, budget: &mut usize, depth: usize) -> Vec<JsonValue>
where
    I: Iterator<Item = Value>,
{
    let mut out = Vec::new();
    for item in items {
        if *budget == 0 {
            out.push(JsonValue::String("…".to_string()));
            break;
        }
        out.push(json_value(item, budget, depth + 1));
    }
    out
}

/// JSON object from lbug property pairs, sharing the cell `budget`;
/// like [`budgeted_items`], a spent budget stops the walk before the
/// next `value.clone()` (F05).
fn object_value(properties: &[(String, Value)], budget: &mut usize, depth: usize) -> JsonValue {
    let mut map = serde_json::Map::new();
    for (key, value) in properties {
        if *budget == 0 {
            map.insert("…".to_string(), JsonValue::String("…".to_string()));
            break;
        }
        let key = string_cell(key.clone(), budget);
        let value = json_value(value.clone(), budget, depth + 1);
        map.insert(key.to_string(), value);
    }
    JsonValue::Object(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{
        Edge, ModuleMetrics, Node, NodeId, NodeKind, Ref, Status, SuggestedCut, Summary,
    };
    use std::collections::BTreeMap;

    /// Kap. 5-shaped sample: modules `app`, `app/billing`, `domain`, files
    /// `app/billing/invoice.ts` and `domain/money.ts`, joined by a cycle
    /// (both directions, one ref each).
    fn sample_graph() -> Graph {
        let file = |id: u32, name: &str, parent: u32| Node {
            id: NodeId(id),
            name: name.to_string(),
            kind: NodeKind::File,
            parent: Some(NodeId(parent)),
            lang: Some("ts".to_string()),
        };
        let module = |id: u32, name: &str, parent: Option<u32>| Node {
            id: NodeId(id),
            name: name.to_string(),
            kind: NodeKind::Module,
            parent: parent.map(NodeId),
            lang: None,
        };
        Graph {
            review_kpis: None,
            semantic: None,
            schema_version: 1,
            root: "/abs/projekt".to_string(),
            nodes: vec![
                module(0, "app", None),
                module(1, "app/billing", Some(0)),
                file(2, "app/billing/invoice.ts", 1),
                module(3, "domain", None),
                file(4, "domain/money.ts", 3),
            ],
            edges: vec![
                Edge {
                    from: NodeId(2),
                    to: NodeId(4),
                    refs: vec![Ref {
                        file: "src/app/billing/invoice.ts".to_string(),
                        line: 4,
                        raw: "import { Money } from \"../../domain/money\";".to_string(),
                    }],
                    // Builder-shaped: the total equals the (complete) sample.
                    count: Some(1),
                },
                Edge {
                    from: NodeId(4),
                    to: NodeId(2),
                    refs: vec![Ref {
                        file: "src/domain/money.ts".to_string(),
                        line: 7,
                        raw: "import { Invoice } from \"../app/billing/invoice\";".to_string(),
                    }],
                    count: Some(1),
                },
            ],
        }
    }

    /// One cycle finding over both files plus `app/tax` (absent from the
    /// graph → placeholder module), one layer violation, per-module metrics.
    fn sample_report() -> Report {
        Report {
            semantic: None,
            schema_version: 1,
            summary: Summary {
                nodes: 5,
                edges: 2,
                sccs: 1,
                largest_scc: 2,
                layer_violations: 1,
                status: Status::Fail,
                ccd: 0,
                acd: 0.0,
                nccd: 0.0,
            },
            config: BTreeMap::new(),
            findings: vec![
                Finding {
                    message: None,
                    evidence: Vec::new(),
                    kind: FindingKind::Cycle,
                    id: "scc-1".to_string(),
                    severity: Severity::Error,
                    nodes: vec![
                        "app/billing/invoice.ts".to_string(),
                        "app/tax".to_string(),
                        "domain/money.ts".to_string(),
                    ],
                    edge_count: 2,
                    cycles_sampled: 1,
                    sampling: None,
                    suggested_cuts: vec![SuggestedCut {
                        from: "domain/money.ts".to_string(),
                        to: "app/billing/invoice.ts".to_string(),
                        score: 0.8,
                        reason: "Eine Import-Stelle.".to_string(),
                    }],
                },
                Finding {
                    message: None,
                    evidence: Vec::new(),
                    kind: FindingKind::LayerViolation,
                    id: "lv-1".to_string(),
                    severity: Severity::Warning,
                    nodes: vec!["domain/money.ts".to_string()],
                    edge_count: 1,
                    cycles_sampled: 0,
                    sampling: None,
                    suggested_cuts: Vec::new(),
                },
            ],
            metrics: [
                (
                    "app/billing/invoice.ts".to_string(),
                    ModuleMetrics {
                        fan_in: 1,
                        fan_out: 1,
                        instability: 0.5,
                        in_cycle: true,
                    },
                ),
                (
                    "domain/money.ts".to_string(),
                    ModuleMetrics {
                        fan_in: 1,
                        fan_out: 1,
                        instability: 0.5,
                        in_cycle: true,
                    },
                ),
                (
                    "app/billing".to_string(),
                    ModuleMetrics {
                        fan_in: 0,
                        fan_out: 0,
                        instability: 0.0,
                        in_cycle: false,
                    },
                ),
            ]
            .into_iter()
            .collect(),
            review_kpis: None,
        }
    }

    /// Opens a fresh store in a tempdir; the caller keeps the tempdir alive.
    fn open_store(dir: &Path) -> LadybugStore {
        LadybugStore::open(dir.join("graph.db")).expect("open store")
    }

    fn rows(store: &LadybugStore, cypher: &str) -> Vec<Vec<JsonValue>> {
        store.query(cypher, usize::MAX).expect("query").rows
    }

    #[test]
    fn target_architecture_finding_keeps_type_and_source_evidence_after_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let graph = sample_graph();
        let mut report = sample_report();
        let finding = Finding {
            kind: FindingKind::TargetArchitecture,
            id: "target:boundary".into(),
            severity: Severity::Error,
            message: Some("Target architecture does not allow Experience → Storage.".into()),
            nodes: vec!["app/billing/invoice.ts".into(), "domain/money.ts".into()],
            edge_count: 1,
            cycles_sampled: 0,
            sampling: None,
            suggested_cuts: vec![],
            evidence: graph.edges[0].refs.clone(),
        };
        report.findings = vec![finding.clone()];
        {
            let mut store = open_store(dir.path());
            store.sync(&graph, &report).unwrap();
        }
        let store = LadybugStore::open_for_query(dir.path().join("graph.db")).unwrap();
        let result = store
            .query(
                "MATCH (f:finding {id: 'target:boundary'}) RETURN f.kind, f.severity, f.detail;",
                10,
            )
            .unwrap();
        assert!(!result.truncated);
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0][0], "target_architecture");
        assert_eq!(result.rows[0][1], "error");
        let restored: Finding = serde_json::from_str(result.rows[0][2].as_str().unwrap()).expect(
            "target finding detail preserves the complete finding and its evidence as JSON",
        );
        assert_eq!(restored, finding);
        assert_eq!(
            rows(
                &store,
                "MATCH (f:finding {kind: 'target_architecture'})-[:involves]->(m:module) RETURN m.name ORDER BY m.name;"
            ),
            vec![
                vec![JsonValue::from("app/billing/invoice.ts")],
                vec![JsonValue::from("domain/money.ts")]
            ]
        );
        assert_eq!(
            rows(&store, "MATCH ()-[d:depends_on]->() RETURN d.cyclic;"),
            vec![vec![JsonValue::Bool(false)], vec![JsonValue::Bool(false)]],
            "a forbidden target direction is not automatically a cycle"
        );
    }

    /// F04 regression: the query open is strictly read-only. A missing
    /// database is an error that creates nothing; a side-effecting second
    /// statement is rejected by the guard *before* execution and leaves no
    /// EXPORT target behind; literal words that look like keywords are
    /// ordinary data; and a read-only store cannot be written to.
    #[test]
    fn query_open_is_read_only_and_creates_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("graph.db");

        // (1) No mirror yet: querying fails and must not conjure one up.
        let missing = LadybugStore::open_for_query(&db_path);
        let Err(err) = missing else {
            panic!("querying a missing database must fail, not create it");
        };
        assert!(err.to_string().contains("does not exist"), "{err}");
        assert!(
            !db_path.exists(),
            "a failed query open must not create {:?}",
            db_path
        );

        // Create the mirror through the sanctioned write path.
        let mut store = LadybugStore::open(&db_path).expect("open for write");
        store.sync(&sample_graph(), &sample_report()).expect("sync");
        drop(store);

        let store = LadybugStore::open_for_query(&db_path).expect("read-only open");

        // (2) The task's acceptance case: rejected before execution — the
        // EXPORT target must not appear.
        let export = dir.path().join("pwn");
        let err = store
            .query(
                &format!("RETURN 1 LIMIT 1; EXPORT DATABASE '{}'", export.display()),
                10,
            )
            .expect_err("EXPORT DATABASE must be rejected");
        assert!(err.to_string().contains("query rejected"), "{err}");
        assert!(
            !export.exists(),
            "a rejected statement must create nothing at {:?}",
            export
        );

        // (2b) F04 review F1: the `--`/`//` divergence payload — with the
        // old comment model kuzu ran the hidden EXPORT; now the guard
        // rejects it before execution and creates nothing.
        let pwn = dir.path().join("pwn2");
        let err = store
            .query(
                &format!("RETURN 1 -- 0; EXPORT DATABASE '{}';\n// x", pwn.display()),
                10,
            )
            .expect_err("payload hidden behind `--` must be rejected");
        assert!(err.to_string().contains("query rejected"), "{err}");
        assert!(!pwn.exists(), "rejected payload must create nothing");

        // (3) Literal words are data, not keywords — through real lbug.
        let result = store
            .query(
                "MATCH (m:module) WHERE m.name = 'delete me' RETURN m.name",
                10,
            )
            .expect("literal words pass the guard");
        assert!(result.rows.is_empty(), "{:?}", result.rows);
        let result = store
            .query("RETURN 'delete me' AS msg", 10)
            .expect("literal return passes the guard");
        assert_eq!(
            result.rows[0][0],
            JsonValue::String("delete me".to_string())
        );

        // (4) lbug's comment grammar (`//` … LF/EOF, non-nested `/* */`)
        // behaves identically on both sides of the guard: the statement
        // runs and the enforced cap is live — `truncated` only fires
        // because the appended LIMIT survives behind the comment tail
        // (F04 review, F2).
        let result = store
            .query(
                "MATCH (m:module) /* all of them */ RETURN m.name ORDER BY m.name // tail",
                2,
            )
            .expect("lbug comments pass guard and engine");
        assert_eq!(result.rows.len(), 2, "{:?}", result.rows);
        assert!(result.truncated, "the appended LIMIT must be the cap");

        // (5) The read-only open cannot mutate: syncing through it fails.
        let mut read_only = LadybugStore::open_for_query(&db_path).expect("ro open again");
        let err = read_only
            .sync(&sample_graph(), &sample_report())
            .expect_err("sync through a read-only open must fail");
        assert!(!err.to_string().is_empty(), "{err}");
    }

    /// F05 acceptance: server-owned row, cell and time budgets that no
    /// caller value can lift — zero, huge and `usize::MAX` limits behave
    /// safely, one huge scalar stays within the documented per-cell
    /// budget without dropping the column, and an expensive query times
    /// out while a subsequent valid request succeeds.
    #[test]
    fn query_budgets_bound_rows_cells_and_time() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = open_store(dir.path());
        // Sample graph plus 1995 extra modules: real-world size for the
        // budget assertions, and enough cross-product work that a 1 ms
        // timeout must interrupt.
        let mut graph = sample_graph();
        for id in 10..2005u32 {
            graph.nodes.push(Node {
                id: NodeId(id),
                name: format!("m{id}"),
                kind: NodeKind::Module,
                parent: None,
                lang: None,
            });
        }
        store.sync(&graph, &sample_report()).expect("sync");

        // (1) Zero limit: no rows, but honest truncation when data exists.
        let result = store
            .query("MATCH (m:module) RETURN m.name", 0)
            .expect("zero limit is valid");
        assert!(result.rows.is_empty(), "{:?}", result.rows);
        assert!(result.truncated, "two thousand modules existed");

        // (2) `usize::MAX` must not materialize the whole cartesian
        // product (2000 × 2000 rows) — the server-owned cap stops at
        // MAX_QUERY_ROWS and flags truncation.
        let result = store
            .query(
                "MATCH (a:module),(b:module) RETURN a.name, b.name",
                usize::MAX,
            )
            .expect("usize::MAX limit is safe");
        assert_eq!(
            result.rows.len(),
            crate::db::MAX_QUERY_ROWS,
            "materialization must stop at the server budget"
        );
        assert!(result.truncated, "the budget itself truncates");

        // (3) One huge scalar: bounded to the documented cell budget,
        // column definition intact, marker says honest goodbye. The
        // literal stays below the statement-size cap (F05 review) — the
        // engine still hands over the full string before the cut.
        let payload = "x".repeat(60 * 1024);
        let result = store
            .query(&format!("RETURN '{payload}' AS big"), 1)
            .expect("huge scalar passes the guard");
        assert_eq!(result.columns, vec!["big".to_string()], "columns kept");
        let cell = result.rows[0][0].as_str().expect("string cell");
        assert!(
            cell.len() <= crate::db::MAX_CELL_BYTES + "[…]".len() * 4,
            "cell must stay within the budget, got {}",
            cell.len()
        );
        assert!(cell.ends_with('…'), "truncation must be visible: {cell}");

        // (4) Execution timeout: a 1 ms budget kills the running query
        // with a clear error — the ORDER BY forces lbug to materialize
        // the full 8e9-row triple cross product before LIMIT, so the
        // engine cannot finish before the deadline — and the store stays
        // usable afterwards.
        store.set_query_timeout_ms(1);
        let err = store
            .query(
                "MATCH (a:module),(b:module),(c:module) \
                 RETURN a.name, b.name, c.name \
                 ORDER BY a.name, b.name, c.name",
                1_000,
            )
            .expect_err("1 ms budget must interrupt the expensive query");
        assert!(err.to_string().contains("timed out"), "{err}");
        store.set_query_timeout_ms(crate::db::QUERY_TIMEOUT_MS);
        let result = store
            .query("MATCH (m:module) RETURN m.name LIMIT 10", 10)
            .expect("the next valid request succeeds");
        assert_eq!(result.rows.len(), 10);
    }

    /// F05 (adversarial review): the cell budget must bound **conversion
    /// work**, not only serialized string bytes — a million-item list
    /// converts at most ~MAX_CELL_BYTES items, huge blobs are cut before
    /// their hex rendering allocates, and a spent budget ends the walk
    /// without visiting or cloning the remaining children.
    #[test]
    fn json_cell_budget_bounds_conversion_not_only_output_bytes() {
        // Wide list of integers: each item pays ≥1 budget unit, so the
        // walk stops around MAX_CELL_BYTES items and closes with one `…`
        // instead of collecting a million values.
        let wide = Value::List(
            LogicalType::Int64,
            (0..1_000_000_i64).map(Value::Int64).collect(),
        );
        let cell = json_cell(wide);
        let items = cell.as_array().expect("array cell");
        assert!(
            items.len() <= MAX_CELL_BYTES + 8,
            "conversion must stop at the budget, got {} items",
            items.len()
        );
        assert_eq!(
            items.last().and_then(JsonValue::as_str),
            Some("…"),
            "the cut must be visible"
        );

        // Wide struct: same bound, and the marker appears as a key.
        let fields = (0..1_000_000)
            .map(|i| (format!("k{i}"), Value::Int64(i)))
            .collect();
        let cell = json_cell(Value::Struct(fields));
        let object = cell.as_object().expect("object cell");
        assert!(
            object.len() <= MAX_CELL_BYTES + 8,
            "struct conversion must stop at the budget, got {}",
            object.len()
        );
        assert!(object.contains_key("…"), "the cut must be visible");

        // A megabyte blob must never be rendered whole: the raw bytes
        // are cut to the budget before hex formatting (≈4–5 chars per
        // byte through Debug, hence the factor).
        let blob = json_cell(Value::Blob(vec![0xAB_u8; 1 << 20]));
        let text = blob.as_str().expect("blob string");
        assert!(
            text.len() <= MAX_CELL_BYTES * 6 + 16,
            "blob rendering must be bounded, got {}",
            text.len()
        );
        assert!(text.ends_with('…'), "the cut must be visible");

        // A megabyte scalar string: one budget-sized cell with marker.
        let huge = json_cell(Value::String("x".repeat(1 << 20)));
        let text = huge.as_str().expect("string cell");
        assert!(text.len() <= MAX_CELL_BYTES + 4, "got {}", text.len());
        assert!(text.ends_with('…'));
    }

    /// A `graph.db` that lbug cannot read — above all a mirror written by
    /// the former `kuzu = "=0.11.3"` backend, which lbug rejects as "not a
    /// valid Lbug database file" — must fail with a remedy instead of the
    /// bare engine message, and must not be overwritten (its `snapshot`
    /// rows are the T41 history).
    #[test]
    fn unreadable_database_file_names_the_remedy_and_stays_untouched() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("graph.db");
        let foreign = b"KUZU\x27\0\0\0 not a ladybug file".repeat(64);
        std::fs::write(&db_path, &foreign).expect("write foreign file");

        let Err(write_err) = LadybugStore::open(&db_path) else {
            panic!("opening a foreign file for sync must fail");
        };
        let Err(read_err) = LadybugStore::open_for_query(&db_path) else {
            panic!("opening a foreign file for queries must fail");
        };
        for err in [write_err.to_string(), read_err.to_string()] {
            assert!(err.contains("not a valid Lbug database file"), "{err}");
            assert!(err.contains("Kuzu"), "{err}");
            assert!(err.contains("archguard db sync"), "{err}");
        }
        assert_eq!(
            std::fs::read(&db_path).expect("file still there"),
            foreign,
            "a failed open must leave the file as it was"
        );
    }

    #[test]
    fn sync_twice_is_idempotent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = open_store(dir.path());
        let (graph, report) = (sample_graph(), sample_report());

        store.sync(&graph, &report).expect("first sync");
        let first_modules = rows(&store, "MATCH (m:module) RETURN m.name ORDER BY m.name;");
        let first_edges = rows(
            &store,
            "MATCH ()-[d:depends_on]->() RETURN d.ref_count, d.cyclic;",
        );
        let first_findings = rows(&store, "MATCH (f:finding) RETURN f.id ORDER BY f.id;");
        let first_involves = rows(&store, "MATCH ()-[i:involves]->() RETURN i;");

        store.sync(&graph, &report).expect("second sync");
        assert_eq!(
            rows(&store, "MATCH (m:module) RETURN m.name ORDER BY m.name;"),
            first_modules
        );
        assert_eq!(
            rows(
                &store,
                "MATCH ()-[d:depends_on]->() RETURN d.ref_count, d.cyclic;",
            ),
            first_edges
        );
        assert_eq!(
            rows(&store, "MATCH (f:finding) RETURN f.id ORDER BY f.id;"),
            first_findings
        );
        assert_eq!(
            rows(&store, "MATCH ()-[i:involves]->() RETURN i;"),
            first_involves
        );

        // Sanity on the mirrored cardinality: 5 graph nodes + 1 placeholder
        // (app/tax), 2 depends_on, 2 findings, 3 + 1 involves.
        assert_eq!(first_modules.len(), 6);
        assert_eq!(first_edges.len(), 2);
        assert_eq!(first_findings.len(), 2);
        assert_eq!(first_involves.len(), 4);

        // Re-opening the existing file keeps the rows and the schema
        // (ensure_schema must tolerate the tables already existing).
        drop(store);
        let reopened = open_store(dir.path());
        assert_eq!(
            rows(&reopened, "MATCH (m:module) RETURN m.name ORDER BY m.name;"),
            first_modules
        );
    }

    #[test]
    fn depends_on_ref_count_is_the_f12_total_not_the_capped_sample() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = open_store(dir.path());
        let file = |id: u32, name: &str| Node {
            id: NodeId(id),
            name: name.to_string(),
            kind: NodeKind::File,
            parent: None,
            lang: Some("ts".to_string()),
        };
        let sample = |file_name: &str| -> Vec<Ref> {
            (0..10)
                .map(|line| Ref {
                    file: file_name.to_string(),
                    line: line + 1,
                    raw: "import …".to_string(),
                })
                .collect()
        };
        let graph = Graph {
            semantic: None,
            review_kpis: None,
            schema_version: 1,
            root: "/abs/projekt".to_string(),
            nodes: vec![file(0, "a"), file(1, "b")],
            edges: vec![
                // a → b: the sample holds 10 of 100 import references.
                Edge {
                    from: NodeId(0),
                    to: NodeId(1),
                    refs: sample("a"),
                    count: Some(100),
                },
                // b → a: legacy edge without total — the sample length is
                // the documented lower bound, so 10 lands in the column.
                Edge {
                    from: NodeId(1),
                    to: NodeId(0),
                    refs: sample("b"),
                    count: None,
                },
            ],
        };
        store.sync(&graph, &sample_report()).expect("sync");

        let result = rows(
            &store,
            "MATCH (a:module)-[d:depends_on]->(b:module) \
             RETURN a.name, b.name, d.ref_count ORDER BY a.name;",
        );
        assert_eq!(
            result,
            vec![
                vec![
                    JsonValue::String("a".to_string()),
                    JsonValue::String("b".to_string()),
                    JsonValue::from(100i64),
                ],
                vec![
                    JsonValue::String("b".to_string()),
                    JsonValue::String("a".to_string()),
                    JsonValue::from(10i64),
                ],
            ]
        );
    }

    #[test]
    fn match_returns_expected_rows() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = open_store(dir.path());
        store.sync(&sample_graph(), &sample_report()).expect("sync");

        let result = store
            .query(
                "MATCH (a:module {name: \"app/billing/invoice.ts\"})\
                 -[d:depends_on]->(b:module) RETURN b.name, d.ref_count, d.cyclic;",
                10,
            )
            .expect("query");
        assert_eq!(
            result.rows,
            vec![
                // one outgoing edge invoice.ts → money.ts, 1 ref, on the cycle
                vec![
                    JsonValue::String("domain/money.ts".to_string()),
                    JsonValue::from(1i64),
                    JsonValue::Bool(true),
                ],
            ]
        );
        assert!(!result.truncated);
        assert_eq!(result.columns, vec!["b.name", "d.ref_count", "d.cyclic"]);

        // Metrics and null handling on module nodes.
        let result = store
            .query(
                "MATCH (m:module {name: \"domain/money.ts\"}) \
                 RETURN m.name, m.lang, m.fan_in, m.instability, m.in_cycle;",
                10,
            )
            .expect("query");
        assert_eq!(
            result.rows,
            vec![vec![
                JsonValue::String("domain/money.ts".to_string()),
                JsonValue::String("ts".to_string()),
                JsonValue::from(1i64),
                JsonValue::from(0.5f64),
                JsonValue::Bool(true),
            ]]
        );
        let lang = store
            .query("MATCH (m:module {name: \"app\"}) RETURN m.lang;", 10)
            .expect("query");
        assert_eq!(lang.rows, vec![vec![JsonValue::Null]]);

        // Findings + involves, as in the Kap. 8.2 example query.
        let cycles = store
            .query(
                "MATCH (f:finding {kind: \"cycle\"})-[:involves]->(m:module) \
                 RETURN f.id, m.name ORDER BY m.name;",
                10,
            )
            .expect("query");
        assert_eq!(
            cycles.rows,
            vec![
                vec![
                    JsonValue::String("scc-1".to_string()),
                    JsonValue::String("app/billing/invoice.ts".to_string()),
                ],
                vec![
                    JsonValue::String("scc-1".to_string()),
                    JsonValue::String("app/tax".to_string()),
                ],
                vec![
                    JsonValue::String("scc-1".to_string()),
                    JsonValue::String("domain/money.ts".to_string()),
                ],
            ]
        );
    }

    #[test]
    fn query_enforces_limit_and_flags_truncated() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = open_store(dir.path());
        store.sync(&sample_graph(), &sample_report()).expect("sync");

        let result = store
            .query("MATCH (m:module) RETURN m.name ORDER BY m.name;", 2)
            .expect("query");
        assert_eq!(result.rows.len(), 2);
        assert!(result.truncated);

        let uncut = store
            .query("MATCH (m:module) RETURN m.name ORDER BY m.name;", 100)
            .expect("query");
        assert_eq!(uncut.rows.len(), 6);
        assert!(!uncut.truncated);

        let empty = store
            .query("MATCH (m:module) RETURN m.name ORDER BY m.name;", 0)
            .expect("query");
        assert!(empty.rows.is_empty());
        assert!(empty.truncated);
    }

    /// T41: the `snapshot` table upserts on the primary key and survives
    /// the full refresh of `sync` (it is not in the delete list).
    #[test]
    fn record_snapshot_upserts_and_survives_sync() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = open_store(dir.path());
        store.sync(&sample_graph(), &sample_report()).expect("sync");

        let record = SnapshotRecord {
            id: "1730000000-abc1234".to_string(),
            taken_at: 1_730_000_000,
            git_sha: "abc1234".to_string(),
            tag: Some("baseline".to_string()),
            cycles: 2,
            largest_scc: 3,
            acd: 1.25,
            edges: 7,
            cycle_hashes: "aa\nbb".to_string(),
        };
        let all = "MATCH (s:snapshot) RETURN s.id, s.taken_at, s.git_sha, s.tag, s.cycles, \
                   s.largest_scc, s.acd, s.edges, s.cycle_hashes;";

        store.record_snapshot(&record).expect("record");
        store
            .record_snapshot(&record)
            .expect("record again (upsert)");
        assert_eq!(
            rows(&store, all),
            vec![vec![
                JsonValue::String("1730000000-abc1234".to_string()),
                JsonValue::from(1_730_000_000_i64),
                JsonValue::String("abc1234".to_string()),
                JsonValue::String("baseline".to_string()),
                JsonValue::from(2i64),
                JsonValue::from(3i64),
                JsonValue::from(1.25f64),
                JsonValue::from(7i64),
                JsonValue::String("aa\nbb".to_string()),
            ]],
            "recording the same id twice keeps exactly one row"
        );

        // Same id, changed values: the row is replaced, absent tag → NULL.
        store
            .record_snapshot(&SnapshotRecord {
                cycles: 1,
                largest_scc: 2,
                acd: 0.5,
                tag: None,
                ..record.clone()
            })
            .expect("replace");
        let replaced = rows(&store, all);
        assert_eq!(replaced.len(), 1);
        assert_eq!(replaced[0][4], JsonValue::from(1i64));
        assert_eq!(replaced[0][5], JsonValue::from(2i64));
        assert_eq!(replaced[0][6], JsonValue::from(0.5f64));
        assert_eq!(replaced[0][3], JsonValue::Null);

        // A second snapshot becomes a second row (time series).
        store
            .record_snapshot(&SnapshotRecord {
                id: "1730000600-abc1234".to_string(),
                taken_at: 1_730_000_600,
                ..record.clone()
            })
            .expect("second snapshot");
        assert_eq!(rows(&store, all).len(), 2);

        // sync wipes the mirror tables but never the trend rows.
        store
            .sync(&sample_graph(), &sample_report())
            .expect("re-sync");
        assert_eq!(
            rows(&store, all).len(),
            2,
            "snapshot rows survive the re-sync"
        );
        assert_eq!(
            rows(&store, "MATCH (m:module) RETURN m.name;").len(),
            6,
            "the mirror itself was refreshed"
        );
    }
}
