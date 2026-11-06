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
