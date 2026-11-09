//! Static service topology. Configured endpoints are evidence of configuration,
//! never evidence that a request ran or that an endpoint exists.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceTopology {
    pub schema_version: u32,
    pub root: String,
    pub monorepo: bool,
    pub monorepo_reason: String,
    pub compose_files: Vec<String>,
    pub services: Vec<Service>,
    /// Buildable projects discovered from static manifests, including non-service tools/libraries.
    #[serde(default)]
    pub projects: Vec<ServiceProject>,
    pub dependencies: Vec<ServiceDependency>,
    pub cycles: Vec<ServiceCycle>,
    pub diagnostics: Vec<ServiceDiagnostic>,
    /// Statically configured datastores, which services point at them, and shared use.
    #[serde(default)]
    pub datastores: ServiceDatastores,
    /// Whether all discovered configuration was covered by this static reader.
    /// Even true does not establish runtime completeness or reachability.
    pub complete: bool,
    /// Recorded requests from OTLP traces, as ordered service-to-service
    /// steps (ADR 0006); empty without `.archguard/otel-traces.json`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub traces: Vec<TraceSequence>,
    /// HTTP routes the services' source handles, with the calls each makes
    /// in source order (static; ADR 0006).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub routes: Vec<ServiceRoute>,
    /// Timeouts, retries, circuit breakers and fallbacks found in a
    /// service's source or the client code it imports.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resilience: Vec<ServiceResilience>,
}

/// One HTTP route of a service, read from its source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceRoute {
    pub service: String,
    pub method: String,
    pub path: String,
    pub file: String,
    pub line: u32,
    /// Calls to other services, in source order.
    pub steps: Vec<RouteStep>,
    /// Error responses the route raises itself (status and message).
    pub failures: Vec<RouteFailure>,
    /// The handler catches errors itself (`catch`, `except`, `rescue`,
    /// `recover()`, `err != nil`); errors from its calls do not simply
    /// pass through.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub catches: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteStep {
    /// Service id of the target.
    pub to: String,
    pub method: String,
    /// Path without query.
    pub path: String,
    pub line: u32,
    /// Started together with other steps (`Promise.all`, `asyncio.gather`, …).
    pub parallel: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteFailure {
    pub status: u16,
    pub message: String,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceResilience {
    pub service: String,
    /// `timeout`, `retry`, `circuit_breaker` or `fallback`.
    pub kind: String,
    pub detail: String,
    pub file: String,
    pub line: u32,
}

/// One recorded trace: the matched service-to-service spans in start order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceSequence {
    pub trace: String,
    pub start_unix_ns: u64,
    /// From the first step's start to the last step's end.
    pub duration_us: u64,
    pub steps: Vec<TraceStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceStep {
    /// Repository service ids, as in `dependencies`.
    pub from: String,
    pub to: String,
    /// `HttpCall`, `GrpcCall` or `Event`.
    pub kind: ServiceDependencyKind,
    /// Start relative to the trace's first step.
    pub offset_us: u64,
    /// Client span duration (consumer span for events).
    pub duration_us: u64,
    pub error: bool,
    /// Route, RPC method or message destination; never a URL or a query.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Service {
    pub id: String,
    pub name: String,
    pub compose_file: String,
    pub build_context: Option<String>,
    pub networks: Vec<String>,
    pub profiles: Vec<String>,
    /// Stable manifest identity; Compose declarations retain their deployment identity.
    #[serde(default)]
    pub project: Option<String>,
}

/// A manifest-defined project; application classification is static evidence, not a running service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceProject {
    pub id: String,
    pub name: String,
    pub path: String,
    pub manifest: String,
    pub ecosystem: String,
    /// application, library, tooling, workspace or unknown.
    pub kind: String,
    pub runtime: Option<String>,
    pub evidence: Vec<ServiceEvidence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceDependencyKind {
    Startup,
    ConfiguredHttp,
    HttpCall,
    GrpcCall,
    Event,
    ConfiguredReference,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceDependency {
    pub from: String,
    pub to: String,
    pub kind: ServiceDependencyKind,
    pub evidence: Vec<ServiceEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ServiceEvidence {
    pub file: String,
    /// Pointer into the parsed Compose document (including merged YAML keys).
    pub pointer: String,
    pub line: Option<u32>,
    /// Sanitized explanation; no raw environment values or URL credentials.
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceCycle {
    pub kind: ServiceDependencyKind,
    /// A strongly connected component, not an ordered cycle path.
    pub nodes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ServiceDiagnostic {
    pub file: String,
    pub service: Option<String>,
    pub message: String,
}

/// Datastore references read from static configuration. A reference is evidence of
/// configuration, not of runtime use; tables and queries are never analyzed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceDatastores {
    pub nodes: Vec<Datastore>,
    pub usages: Vec<DatastoreUsage>,
    /// Datastores that two or more services point at.
    pub shared: Vec<SharedDatastore>,
    /// Coverage note; empty when no datastore was found.
    #[serde(default)]
    pub note: String,
}

/// A database server, cache or broker used as a store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Datastore {
    /// The service id when it runs in the topology, otherwise `external:<host>[:<port>]`.
    pub id: String,
    /// postgresql, mysql, mongodb, redis, ... or `None` when unknown.
    pub engine: Option<String>,
    /// The topology service running this datastore, if any.
    pub service: Option<String>,
    /// Configured host name of a datastore outside the topology.
    pub host: Option<String>,
    pub port: Option<u16>,
    pub evidence: Vec<ServiceEvidence>,
}

/// A service configured to use a datastore. Credentials and raw values are never kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatastoreUsage {
    pub service: String,
    pub datastore: String,
    /// Database or schema name when configured literally.
    pub database: Option<String>,
    pub evidence: Vec<ServiceEvidence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SharedDatastoreScope {
    /// Same datastore and the same database name.
    Database,
    /// Same datastore server; database names differ or are unknown.
    Server,
}

/// Two or more services configured against the same datastore. Shared tables are never claimed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedDatastore {
    pub datastore: String,
    pub scope: SharedDatastoreScope,
    /// The shared database name for `Database`; `None` for `Server`.
    pub database: Option<String>,
    pub services: Vec<String>,
}
