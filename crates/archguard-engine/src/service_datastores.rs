//! Datastore references from static configuration: well-known datastore images,
//! connection URLs and conventional host/database settings. Only engine, host, port
//! and database name are kept; credentials, query strings and raw values never are.
use super::service_datastore_settings::{Reference, parse_url, references};
use super::service_deployments::{Deployment, proof};
use super::support::diagnostic;
use archguard_core::{
    Datastore, DatastoreUsage, ServiceEvidence, ServiceTopology, SharedDatastore,
    SharedDatastoreScope as Scope,
};
use std::collections::{BTreeMap, BTreeSet};

/// A Spring connection property of one service.
pub(super) struct SpringDatasource {
    pub service: String,
    pub value: String,
    pub evidence: ServiceEvidence,
}

const NOTE: &str = "Datastore references come from static configuration (images, connection URLs, host/database settings); they do not prove runtime use. A shared database means the same configured database name, a shared server only the same configured server. Tables and queries are not analyzed.";
const IMAGE: &str = "Well-known datastore image; the datastore runs as this service.";
const URL: &str = "Literal datastore connection URL; host, port and database name only. Credentials and options omitted.";
const SETTING: &str = "Conventional datastore host/database setting; credentials are not read.";
pub(super) const SPRING: &str =
    "Spring datasource property; profiles are combined. Credentials and options omitted.";

/// The engine of a container image, matched on its repository name.
fn image_engine(image: &str) -> Option<&'static str> {
    let image = image.to_ascii_lowercase();
    let path = image.split(['@']).next().unwrap_or("");
    let name = path.rsplit('/').next().unwrap_or("");
    let name = name.split(':').next().unwrap_or("");
    if let Some((_, engine)) = IMAGES.iter().find(|(image, _)| *image == name) {
        return Some(engine);
    }
    if path.contains("mssql") {
        return Some("sqlserver");
    }
    (path.contains("oracle") && !path.contains("java")).then_some("oracle")
}

/// Repository names of well-known datastore images and their engines.
const IMAGES: [(&str, &str); 20] = [
    ("postgres", "postgresql"),
    ("postgresql", "postgresql"),
    ("postgis", "postgresql"),
    ("timescaledb", "postgresql"),
    ("mysql", "mysql"),
    ("mysql-server", "mysql"),
    ("mariadb", "mariadb"),
    ("mongo", "mongodb"),
    ("mongodb", "mongodb"),
    ("mongodb-community-server", "mongodb"),
    ("redis", "redis"),
    ("redis-stack", "redis"),
    ("redis-stack-server", "redis"),
    ("valkey", "redis"),
    ("cassandra", "cassandra"),
    ("elasticsearch", "elasticsearch"),
    ("opensearch", "elasticsearch"),
    ("neo4j", "neo4j"),
    ("couchdb", "couchdb"),
    ("cockroach", "cockroachdb"),
];

fn default_port(engine: &str) -> Option<u16> {
    Some(match engine {
        "postgresql" | "cockroachdb" => 5432,
        "mysql" | "mariadb" => 3306,
        "mongodb" => 27017,
        "redis" => 6379,
        "sqlserver" => 1433,
        "oracle" => 1521,
        "cassandra" => 9042,
        _ => return None,
    })
}

fn loopback(host: &str) -> bool {
    matches!(host, "localhost" | "0.0.0.0" | "::1") || host.starts_with("127.")
}

/// Datastore nodes and usages collected before the shared groups are derived.
struct Collector<'a> {
    report: &'a mut ServiceTopology,
    nodes: BTreeMap<String, Datastore>,
    usages: BTreeMap<(String, String, Option<String>), BTreeSet<ServiceEvidence>>,
}

impl Collector<'_> {
    fn node(&mut self, id: &str, engine: Option<&str>) -> &mut Datastore {
        let node = self
            .nodes
            .entry(id.to_owned())
            .or_insert_with(|| Datastore {
                id: id.to_owned(),
                engine: None,
                service: None,
                host: None,
                port: None,
                evidence: Vec::new(),
            });
        if node.engine.is_none() {
            node.engine = engine.map(str::to_owned);
        }
        node
    }

    /// The datastore a reference of `service` names: a unique workload in its scope,
    /// otherwise the configured external host.
    fn resolve(
        &mut self,
        service: &str,
        file: &str,
        reference: &Reference,
        candidates: &[&Deployment],
    ) -> Option<String> {
        let name = service.rsplit([':']).next().unwrap_or(service);
        let Some(host) = &reference.host else {
            diagnostic(
                self.report,
                file,
                Some(name),
                "Datastore reference has an interpolated, multi-host or unsupported host; the datastore is unknown.",
            );
            return None;
        };
        if loopback(host) {
            diagnostic(
                self.report,
                file,
                Some(name),
                "Datastore reference names a loopback host; it is not attributed to a shared datastore.",
            );
            return None;
        }
        let targets: Vec<_> = candidates
            .iter()
            .filter(|d| d.aliases.contains(host))
            .collect();
        match targets.as_slice() {
            [target] if target.id == service => None,
            [target] => {
                let id = target.id.clone();
                let node = self.node(&id, reference.engine);
                node.service = Some(id.clone());
                Some(id)
            }
            [] => {
                let port = reference
                    .port
                    .or_else(|| reference.engine.and_then(default_port));
                let id = match port {
                    Some(port) => format!("external:{host}:{port}"),
                    None => format!("external:{host}"),
                };
                let node = self.node(&id, reference.engine);
                node.host = Some(host.clone());
                node.port = port;
                Some(id)
            }
            _ => {
                diagnostic(
                    self.report,
                    file,
                    Some(name),
                    "Datastore host is ambiguous in this deployment scope; no target was guessed.",
                );
                None
            }
        }
    }

    fn usage(
        &mut self,
        service: &str,
        datastore: String,
        database: Option<String>,
        evidence: Vec<ServiceEvidence>,
    ) {
        if let Some(node) = self.nodes.get_mut(&datastore)
            && node.service.is_none()
        {
            node.evidence.extend(evidence.iter().cloned());
        }
        self.usages
            .entry((service.to_owned(), datastore, database))
            .or_default()
            .extend(evidence);
    }

    fn deployment(&mut self, d: &Deployment, all: &[Deployment]) {
        let scope: Vec<&Deployment> = all.iter().filter(|t| t.scope == d.scope).collect();
        for (reference, keys) in references(&d.datastore_env) {
            let Some(datastore) = self.resolve(&d.id, &d.file, &reference, &scope) else {
                continue;
            };
            let mut evidence = Vec::new();
            for key in keys {
                match d.datastore_env.get(&key) {
                    Some((_, origin)) if !origin.is_empty() => evidence.extend(origin.clone()),
                    _ => {
                        let text = if parse_url(&d.datastore_env[&key].0).is_some() {
                            URL
                        } else {
                            SETTING
                        };
                        evidence.push(proof(&d.file, &d.pointer(&key), text));
                    }
                }
            }
            self.usage(&d.id, datastore, reference.database, evidence);
        }
    }

    fn spring(&mut self, source: &SpringDatasource, all: &[Deployment]) {
        let Some(reference) = parse_url(&source.value) else {
            return;
        };
        let scope: Vec<&Deployment> = match all.iter().find(|d| d.id == source.service) {
            Some(own) => all.iter().filter(|t| t.scope == own.scope).collect(),
            None => all.iter().collect(),
        };
        let file = source.evidence.file.clone();
        if let Some(datastore) = self.resolve(&source.service, &file, &reference, &scope) {
            self.usage(
                &source.service,
                datastore,
                reference.database,
                vec![source.evidence.clone()],
            );
        }
    }
}

/// Adds datastore nodes, service usages and shared datastores to the topology.
pub(super) fn augment(
    report: &mut ServiceTopology,
    deployments: &[Deployment],
    spring: &[SpringDatasource],
) {
    let mut collector = Collector {
        report,
        nodes: BTreeMap::new(),
        usages: BTreeMap::new(),
    };
    let mut stores = BTreeSet::new();
    for d in deployments {
        let Some((image, engine)) = d
            .images
            .iter()
            .find_map(|(pointer, image)| Some((pointer, image_engine(image)?)))
        else {
            continue;
        };
        stores.insert(d.id.clone());
        let node = collector.node(&d.id, Some(engine));
        node.service = Some(d.id.clone());
        node.evidence.push(proof(&d.file, image, IMAGE));
    }
    // A datastore's own settings (e.g. POSTGRES_DB) declare it; they are no usage.
    for d in deployments.iter().filter(|d| !stores.contains(&d.id)) {
        collector.deployment(d, deployments);
    }
    for source in spring.iter().filter(|s| !stores.contains(&s.service)) {
        collector.spring(source, deployments);
    }
    let Collector {
        report,
        nodes,
        usages,
    } = collector;
    report.datastores.usages = merge_unknown_databases(usages);
    report.datastores.nodes = nodes
        .into_values()
        .map(|mut node| {
            node.evidence.sort();
            node.evidence.dedup();
            node
        })
        .collect();
    report.datastores.shared = shared(&report.datastores.usages);
    if !report.datastores.nodes.is_empty() {
        report.datastores.note = NOTE.into();
    }
}

/// A usage without a database name adds nothing when the same service names a
/// database on that datastore; its evidence joins those usages.
fn merge_unknown_databases(
    usages: BTreeMap<(String, String, Option<String>), BTreeSet<ServiceEvidence>>,
) -> Vec<DatastoreUsage> {
    let mut unknown = BTreeMap::new();
    let mut out: Vec<DatastoreUsage> = Vec::new();
    for ((service, datastore, database), evidence) in usages {
        if database.is_none() {
            unknown.insert((service.clone(), datastore.clone()), evidence.clone());
        }
        out.push(DatastoreUsage {
            service,
            datastore,
            database,
            evidence: evidence.into_iter().collect(),
        });
    }
    let named: BTreeSet<(String, String)> = out
        .iter()
        .filter(|u| u.database.is_some())
        .map(|u| (u.service.clone(), u.datastore.clone()))
        .collect();
    out.retain(|u| {
        u.database.is_some() || !named.contains(&(u.service.clone(), u.datastore.clone()))
    });
    for usage in &mut out {
        if let Some(extra) = unknown.get(&(usage.service.clone(), usage.datastore.clone()))
            && usage.database.is_some()
        {
            usage.evidence.extend(extra.iter().cloned());
            usage.evidence.sort();
            usage.evidence.dedup();
        }
    }
    out
}

/// Same datastore and database name: shared database. Two or more services on one
/// datastore that do not all share one known database: shared server.
fn shared(usages: &[DatastoreUsage]) -> Vec<SharedDatastore> {
    let mut by_store: BTreeMap<&str, BTreeMap<Option<&str>, BTreeSet<&str>>> = BTreeMap::new();
    for usage in usages {
        by_store
            .entry(&usage.datastore)
            .or_default()
            .entry(usage.database.as_deref())
            .or_default()
            .insert(&usage.service);
    }
    let mut out = Vec::new();
    for (datastore, databases) in by_store {
        let all: BTreeSet<&str> = databases.values().flatten().copied().collect();
        for (name, services) in &databases {
            if let Some(name) = name
                && services.len() >= 2
            {
                out.push(SharedDatastore {
                    datastore: datastore.into(),
                    scope: Scope::Database,
                    database: Some((*name).into()),
                    services: services.iter().map(|s| (*s).into()).collect(),
                });
            }
        }
        let one_database = databases.len() == 1 && databases.keys().all(Option::is_some);
        if all.len() >= 2 && !one_database {
            out.push(SharedDatastore {
                datastore: datastore.into(),
                scope: Scope::Server,
                database: None,
                services: all.into_iter().map(str::to_owned).collect(),
            });
        }
    }
    out
}
