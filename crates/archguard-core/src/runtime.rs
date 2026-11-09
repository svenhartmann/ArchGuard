//! Runtime profile imported from OTLP JSON Lines (ADR 0004). It describes a
//! sample of analysis runs: never complete traffic, never proof that a pattern
//! is absent. Query texts keep placeholders only; literal values are replaced.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RuntimeProfile {
    pub schema_version: u32,
    /// Project-relative input files in read order.
    pub inputs: Vec<String>,
    /// Unique, valid spans imported.
    pub spans: u64,
    pub traces: u64,
    /// First span start and last span end, Unix nanoseconds.
    pub window_unix_ns: Option<[u64; 2]>,
    /// Distinct `archguard.sampling` resource values; empty when unknown.
    pub sampling: Vec<String>,
    pub endpoints: Vec<RuntimeEndpoint>,
    pub statements: Vec<RuntimeStatement>,
    pub diagnostics: Vec<RuntimeDiagnostic>,
    /// False when input was skipped (unreadable lines, limits, refused files).
    pub complete: bool,
    pub note: String,
}

/// Requests handled by one entry span name (SERVER or CONSUMER) of a service.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeEndpoint {
    pub service: String,
    /// `METHOD route`, `rpc.service/rpc.method`, or the span name.
    pub name: String,
    pub requests: u64,
    /// Entry spans with ERROR status.
    pub errors: u64,
    pub duration_ms: RuntimeStats,
    /// Database calls per request.
    pub db_calls: RuntimeStats,
    /// Summed client-side duration of the requests' database calls. Parallel
    /// calls can make it exceed the request time.
    pub db_time_ms: f64,
    pub example: RuntimeEvidence,
}

/// Nearest-rank statistics over the observed values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct RuntimeStats {
    pub mean: f64,
    pub p50: f64,
    pub p95: f64,
    pub max: f64,
}

/// Database calls grouped by service, database system and statement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeStatement {
    pub service: String,
    pub db_system: Option<String>,
    /// `db.query.summary`, else `archguard.db.statement_hash`, else the normalized text.
    pub key: String,
    /// Normalized query text (literals replaced by `?`), when recorded.
    pub text: Option<String>,
    pub calls: u64,
    pub errors: u64,
    /// Client-side span duration: execution, network and pool wait.
    pub exec_ms: RuntimeStats,
    /// Sum of `archguard.db.read_ms` (row reading); `None` when no probe recorded it.
    pub read_ms_total: Option<f64>,
    /// Most calls of this statement within one request (or one trace outside requests).
    pub max_calls_per_request: u64,
    pub call_sites: Vec<RuntimeCallSite>,
    pub example: RuntimeEvidence,
}

/// The application code that issued a database call, as recorded on the span.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCallSite {
    pub function: Option<String>,
    pub file: Option<String>,
    pub line: Option<u32>,
    /// `archguard.callsite.source` (`stack`, `tag`, `none`) when a probe set it.
    pub source: Option<String>,
    pub calls: u64,
}

/// Where a value can be checked: input file, 1-based line, trace and span id.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeEvidence {
    pub file: String,
    pub line: u32,
    pub trace_id: String,
    pub span_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeDiagnostic {
    pub file: String,
    pub line: Option<u32>,
    pub message: String,
}
