//! Static service topology. Configured endpoints are evidence of configuration,
//! never evidence that a request ran or that an endpoint exists.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
    /// Elasticsearch artifacts, index usages and findings read statically.
    #[serde(default, skip_serializing_if = "ElasticsearchModel::is_empty")]
    pub elasticsearch: ElasticsearchModel,
}

/// Static Elasticsearch view: declared artifacts, which services use which
/// index, and the findings derived from comparing the two.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElasticsearchModel {
    pub artifacts: Vec<SearchArtifact>,
    pub usages: Vec<SearchIndexUsage>,
    pub findings: Vec<SearchFinding>,
    /// Project names of a project set; empty outside a set. `projects[0]` is
    /// the primary project.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub projects: Vec<String>,
}

impl ElasticsearchModel {
    pub fn is_empty(&self) -> bool {
        self.artifacts.is_empty()
            && self.usages.is_empty()
            && self.findings.is_empty()
            && self.projects.is_empty()
    }

    /// The primary project of a project set, if any.
    pub fn primary(&self) -> Option<&str> {
        self.projects.first().map(String::as_str)
    }
}

/// A declared Elasticsearch object. `kind` is one of `index`, `alias`,
/// `index_template`, `component_template`, `ilm_policy`, `ingest_pipeline`,
/// `transform`, `watch`, `enrich_policy`, `snapshot_lifecycle`,
/// `stored_script`, `search_template`, `data_view`, `alerting_rule`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchArtifact {
    pub kind: String,
    pub name: Option<String>,
    pub address: String,
    pub scope: String,
    pub patterns: Vec<String>,
    pub references: Vec<SearchReference>,
    pub evidence: Vec<ServiceEvidence>,
    /// Projects of a project set that declare this artifact; empty outside a set.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub origins: Vec<String>,
    /// Mapping, settings and analysis of an index or template; each attribute is read only when literal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<SearchIndexDetail>,
}

/// Mapping, settings and analysis of an index or template (ADR 0023). An
/// attribute that is not fully literal is not read; the others are.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchIndexDetail {
    /// Dotted paths, parents before children; a multi-field is `a.raw`; an
    /// object without `type` is `object`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<SearchField>,
    /// Keys without the `index.` prefix, values as written.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub settings: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub analysis: Vec<SearchAnalysis>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchField {
    pub path: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analyzer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_analyzer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normalizer: Option<String>,
}

/// One analysis component; `kind` is `analyzer`, `tokenizer`, `filter`,
/// `char_filter` or `normalizer`, `definition` is compact JSON.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchAnalysis {
    pub kind: String,
    pub name: String,
    pub definition: String,
}

impl SearchArtifact {
    /// Declared in a project set, but not in the primary project.
    pub fn foreign(&self, primary: &str) -> bool {
        !self.origins.is_empty() && !self.origins.iter().any(|o| o == primary)
    }
}

/// Target kind and name an artifact points at, e.g. `ingest_pipeline`/`p1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchReference {
    pub kind: String,
    pub name: String,
}

/// A service's use of an index; `access` is `write`, `read` or `unknown`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchIndexUsage {
    pub service: String,
    pub index: String,
    pub access: String,
    pub evidence: Vec<ServiceEvidence>,
}

/// `kind` is `undeclared_pipeline`, `undeclared_index`, `shared_write_index`,
/// `uncovered_pattern` or `alert_undeclared_index`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchFinding {
    pub kind: String,
    pub subject: String,
    pub services: Vec<String>,
    pub evidence: Vec<ServiceEvidence>,
}

#[cfg(test)]
mod elasticsearch_tests {
    use super::*;

    #[test]
    fn default_topology_omits_elasticsearch_and_filled_model_round_trips() {
        let t = ServiceTopology::default();
        assert!(!serde_json::to_string(&t).unwrap().contains("elasticsearch"));
        let mut t = t;
        t.elasticsearch.artifacts.push(SearchArtifact {
            kind: "index_template".into(),
            name: Some("t1".into()),
            address: "es_index_template.t1".into(),
            scope: "infra".into(),
            patterns: vec!["logs-*".into()],
            references: vec![SearchReference {
                kind: "ingest_pipeline".into(),
                name: "p1".into(),
            }],
            evidence: vec![],
            origins: vec![],
            detail: None,
        });
        t.elasticsearch.usages.push(SearchIndexUsage {
            service: "api".into(),
            index: "logs-a".into(),
            access: "write".into(),
            evidence: vec![],
        });
        t.elasticsearch.findings.push(SearchFinding {
            kind: "undeclared_index".into(),
            subject: "logs-a".into(),
            services: vec!["api".into()],
            evidence: vec![],
        });
        let json = serde_json::to_string(&t).unwrap();
        assert!(json.contains("elasticsearch"));
        assert_eq!(serde_json::from_str::<ServiceTopology>(&json).unwrap(), t);
        assert!(!json.contains("origins"));
    }

    fn art(origins: &[&str]) -> SearchArtifact {
        SearchArtifact {
            kind: "index".into(),
            name: Some("audit".into()),
            address: "a".into(),
            scope: "s".into(),
            patterns: vec![],
            references: vec![],
            evidence: vec![],
            origins: origins.iter().map(|o| o.to_string()).collect(),
            detail: None,
        }
    }

    #[test]
    fn origins_and_projects_round_trip_and_are_skipped_when_empty() {
        let mut m = ElasticsearchModel::default();
        m.artifacts.push(art(&["infra", "adapter"]));
        m.projects = vec!["infra".into(), "adapter".into()];
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("\"origins\":[\"infra\",\"adapter\"]"));
        assert!(json.contains("\"projects\":[\"infra\",\"adapter\"]"));
        assert_eq!(
            serde_json::from_str::<ElasticsearchModel>(&json).unwrap(),
            m
        );
        let mut bare = m.clone();
        bare.projects.clear();
        assert!(
            !serde_json::to_string(&bare)
                .unwrap()
                .contains("\"projects\"")
        );
        let plain = serde_json::to_string(&art(&[])).unwrap();
        assert!(!plain.contains("origins"));
        assert!(
            serde_json::from_str::<SearchArtifact>(&plain)
                .unwrap()
                .origins
                .is_empty()
        );
        let only_projects = ElasticsearchModel {
            projects: vec!["a".into()],
            ..Default::default()
        };
        assert!(!only_projects.is_empty());
    }

    #[test]
    fn detail_is_skipped_when_none_and_round_trips() {
        let mut a = art(&[]);
        assert!(!serde_json::to_string(&a).unwrap().contains("detail"));
        a.detail = Some(SearchIndexDetail {
            fields: vec![SearchField {
                path: "a.b".into(),
                kind: "keyword".into(),
                ..Default::default()
            }],
            settings: [("number_of_shards".to_string(), "1".to_string())].into(),
            analysis: vec![],
        });
        let json = serde_json::to_string(&a).unwrap();
        assert!(json.contains("\"detail\""));
        assert!(!json.contains("analysis"));
        assert!(!json.contains("analyzer"));
        assert_eq!(serde_json::from_str::<SearchArtifact>(&json).unwrap(), a);
    }

    #[test]
    fn foreign_and_primary() {
        assert!(!art(&[]).foreign("infra"));
        assert!(!art(&["infra", "adapter"]).foreign("infra"));
        assert!(art(&["adapter"]).foreign("infra"));
        let mut m = ElasticsearchModel::default();
        assert_eq!(m.primary(), None);
        m.projects = vec!["infra".into(), "adapter".into()];
        assert_eq!(m.primary(), Some("infra"));
    }
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
/// configuration, not of runtime use. Tables come from the services' own source
/// (ADR 0012); queries are never analyzed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceDatastores {
    pub nodes: Vec<Datastore>,
    pub usages: Vec<DatastoreUsage>,
    /// Datastores that two or more services point at.
    pub shared: Vec<SharedDatastore>,
    /// Coverage note; empty when no datastore was found.
    #[serde(default)]
    pub note: String,
    /// Tables a service's own source maps or migrates (entities, migrations, SQL).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tables: Vec<ServiceTable>,
}

/// A table named literally in a service's source; one entry per service and table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceTable {
    pub service: String,
    /// Lowercase table name without schema or quotes.
    pub name: String,
    pub evidence: Vec<ServiceEvidence>,
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
    /// The service's tables (see `ServiceDatastores::tables`) when this is its only
    /// usage of a datastore with tables; otherwise empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tables: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SharedDatastoreScope {
    /// Same datastore and the same database name.
    Database,
    /// Same datastore server; database names differ or are unknown.
    Server,
    /// Same database, and two or more of the services name the same table.
    Table,
}

/// Two or more services configured against the same datastore. A shared table is claimed
/// only when the services' source names it literally (ADR 0012).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SharedDatastore {
    pub datastore: String,
    pub scope: SharedDatastoreScope,
    /// The shared database name for `Database` and `Table`; `None` for `Server`.
    pub database: Option<String>,
    pub services: Vec<String>,
    /// For `Table`: the tables two or more of the services name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tables: Vec<String>,
    /// Same database, every service's tables are known and none is named twice.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disjoint_tables: bool,
}
