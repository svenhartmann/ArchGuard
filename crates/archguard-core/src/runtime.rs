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
    /// Single requests with their span tree, newest first, at most
    /// `MAX_REQUESTS` of `requests_total`.
    pub requests: Vec<RuntimeRequest>,
    pub requests_total: u64,
    /// The `[runtime]` thresholds the findings used.
    #[serde(default)]
    pub thresholds: RuntimeConfig,
    /// Threshold findings (`[runtime]` in archguard.toml), most severe kinds first.
    pub findings: Vec<RuntimeFinding>,
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
    /// Most calls from this site within one request (or one trace outside requests).
    pub max_calls_per_request: u64,
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

/// `[runtime]` in archguard.toml: thresholds of the runtime findings. A
/// finding fires when the observed value reaches the threshold. The defaults
/// are starting values, not calibrated on measured projects.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RuntimeConfig {
    /// `n_plus_one`: calls of one statement from one call site within one request.
    pub n_plus_one_min_calls: u64,
    /// `chatty_endpoint`: database calls per request at the 95th percentile.
    pub chatty_endpoint_db_calls: f64,
    /// `slow_statement`: client-side execution time at the 95th percentile, ms.
    pub slow_statement_ms: f64,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            n_plus_one_min_calls: 5,
            chatty_endpoint_db_calls: 20.0,
            slow_statement_ms: 100.0,
        }
    }
}

/// A runtime value that reached its `[runtime]` threshold in the sample.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeFinding {
    /// `n_plus_one`, `chatty_endpoint` or `slow_statement`.
    pub kind: String,
    pub service: String,
    /// Statement key or endpoint name.
    pub subject: String,
    pub value: f64,
    pub threshold: f64,
    /// The issuing code for `n_plus_one`, when the span recorded it.
    pub call_site: Option<RuntimeCallSite>,
    pub message: String,
    pub evidence: RuntimeEvidence,
}

/// One request (an entry span) with the spans of its own service below it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeRequest {
    pub service: String,
    pub name: String,
    pub trace_id: String,
    pub span_id: String,
    pub start_unix_ns: u64,
    pub duration_ms: f64,
    pub error: bool,
    pub db_calls: u64,
    pub db_time_ms: f64,
    /// Pre-order, children by start time; a database span wrapping a driver
    /// span is folded into it, as when counting.
    pub spans: Vec<RuntimeSpan>,
    /// More spans than kept.
    pub spans_truncated: bool,
    pub evidence: RuntimeEvidence,
}

/// A span inside a request, placed relative to the request's start.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeSpan {
    /// 1 for direct children of the request.
    pub depth: u32,
    pub offset_ms: f64,
    pub duration_ms: f64,
    /// `server`, `client`, `internal`, `producer`, `consumer` or `unspecified`.
    pub kind: String,
    pub name: String,
    pub error: bool,
    /// Statement key of a database call.
    pub db: Option<String>,
    pub function: Option<String>,
    pub file: Option<String>,
    pub line: Option<u32>,
}
