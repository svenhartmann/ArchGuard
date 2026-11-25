//! The tab's data: the engines of the topology as sub-items and the list rows of an engine.

use std::collections::BTreeMap;

use archguard_engine::core::{DatastoreUsage, ServiceTopology};

/// One sub-item: the raw engine name and what it holds.
#[derive(Debug, PartialEq, Eq)]
pub struct Engine {
    pub name: String,
    pub count: usize,
}

/// One datastore of a list sub-item.
#[derive(Debug, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub endpoint: String,
    pub services: String,
    pub database: String,
}

const ES: &str = "elasticsearch";
const UNKNOWN: &str = "unknown";
/// Engines without tables (as `archguard-engine`'s service tables); the other named ones are relational.
const NOT_RELATIONAL: [&str; 6] = ["mongodb", "redis", "cassandra", ES, "neo4j", "couchdb"];

fn engine_of(engine: &Option<String>) -> &str {
    engine.as_deref().unwrap_or(UNKNOWN)
}

/// Elasticsearch, then the relational engines, then the others (each by name), unknown last.
fn rank(name: &str) -> u8 {
    match name {
        ES => 0,
        UNKNOWN => 3,
        n if NOT_RELATIONAL.contains(&n) => 2,
        _ => 1,
    }
}

/// One sub-item per engine with content: the Elasticsearch model counts its artifacts,
/// every engine its datastores.
pub fn engines(t: &ServiceTopology) -> Vec<Engine> {
    let mut count: BTreeMap<&str, usize> = BTreeMap::new();
    for n in &t.datastores.nodes {
        *count.entry(engine_of(&n.engine)).or_default() += 1;
    }
    if !t.elasticsearch.is_empty() {
        count.insert(ES, t.elasticsearch.artifacts.len());
    }
    let mut out: Vec<Engine> = count
        .into_iter()
        .map(|(name, count)| Engine {
            name: name.into(),
            count,
        })
        .collect();
    out.sort_by_key(|e| rank(&e.name));
    out
}

fn joined<'a>(items: impl Iterator<Item = &'a str>) -> String {
    let mut seen: Vec<&str> = Vec::new();
    for i in items {
        if !seen.contains(&i) {
            seen.push(i);
        }
    }
    seen.join(", ")
}

fn used<'a>(t: &'a ServiceTopology, id: &'a str) -> impl Iterator<Item = &'a DatastoreUsage> {
    t.datastores
        .usages
        .iter()
        .filter(move |u| u.datastore == id)
}

/// The service's name, else its id (as the Services view).
fn name<'a>(t: &'a ServiceTopology, id: &'a str) -> &'a str {
    t.services
        .iter()
        .find(|s| s.id == id)
        .map_or(id, |s| s.name.as_str())
}

/// The datastores of `engine`: id, `host:port`, the services using it and its database names.
pub fn rows(t: &ServiceTopology, engine: &str) -> Vec<Row> {
    t.datastores
        .nodes
        .iter()
        .filter(|n| engine_of(&n.engine) == engine)
        .map(|n| {
            let host = n.host.as_deref().unwrap_or("");
            let endpoint = match n.port {
                Some(p) if !host.is_empty() => format!("{host}:{p}"),
                Some(p) => format!(":{p}"),
                None => host.to_string(),
            };
            Row {
                id: n.id.clone(),
                endpoint,
                services: joined(used(t, &n.id).map(|u| name(t, &u.service))),
                database: joined(used(t, &n.id).filter_map(|u| u.database.as_deref())),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use archguard_engine::core::{
        Datastore, DatastoreUsage, SearchArtifact, Service, ServiceTopology,
    };

    use super::*;

    fn store(id: &str, engine: Option<&str>, host: Option<&str>, port: Option<u16>) -> Datastore {
        Datastore {
            id: id.into(),
            engine: engine.map(Into::into),
            service: None,
            host: host.map(Into::into),
            port,
            evidence: vec![],
        }
    }

    fn usage(service: &str, datastore: &str, database: Option<&str>) -> DatastoreUsage {
        DatastoreUsage {
            service: service.into(),
            datastore: datastore.into(),
            database: database.map(Into::into),
            evidence: vec![],
            tables: vec![],
        }
    }

    fn artifact() -> SearchArtifact {
        SearchArtifact {
            kind: "index".into(),
            name: Some("x".into()),
            address: "i.x".into(),
            scope: String::new(),
            patterns: vec![],
            references: vec![],
            evidence: vec![],
            origins: vec![],
        }
    }

    fn service(id: &str, name: &str) -> Service {
        Service {
            id: id.into(),
            name: name.into(),
            compose_file: "compose.yaml".into(),
            build_context: None,
            networks: vec![],
            profiles: vec![],
            project: None,
        }
    }

    fn topology() -> ServiceTopology {
        let mut t = ServiceTopology::default();
        t.services = vec![
            service("compose.yaml::orders", "orders"),
            service("compose.yaml::billing", "billing"),
        ];
        t.elasticsearch.artifacts = vec![artifact()];
        t.datastores.nodes = vec![
            store("pg", Some("postgresql"), Some("db.local"), Some(5432)),
            store("cache", Some("redis"), None, None),
            store("docs", Some("mongodb"), Some("mongo.local"), None),
            store("mystery", None, None, None),
        ];
        t.datastores.usages = vec![
            usage("compose.yaml::orders", "pg", Some("shop")),
            usage("compose.yaml::billing", "pg", None),
            usage("compose.yaml::orders", "cache", None),
        ];
        t
    }

    fn names(t: &ServiceTopology) -> Vec<(String, usize)> {
        engines(t).into_iter().map(|e| (e.name, e.count)).collect()
    }

    #[test]
    fn engines_come_elasticsearch_first_then_relational_then_the_rest_unknown_last() {
        let want = [
            ("elasticsearch", 1),
            ("postgresql", 1),
            ("mongodb", 1),
            ("redis", 1),
            ("unknown", 1),
        ];
        let want: Vec<_> = want.iter().map(|(n, c)| (n.to_string(), *c)).collect();
        assert_eq!(names(&topology()), want);
    }

    #[test]
    fn an_empty_topology_has_no_sub_items() {
        assert!(engines(&ServiceTopology::default()).is_empty());
    }

    #[test]
    fn a_datastore_elasticsearch_alone_gives_the_sub_item() {
        let mut t = ServiceTopology::default();
        t.datastores.nodes = vec![store("es", Some("elasticsearch"), None, None)];
        assert_eq!(names(&t), [("elasticsearch".to_string(), 1)]);
    }

    #[test]
    fn list_rows_carry_id_endpoint_services_and_database() {
        let mut t = topology();
        let pg = rows(&t, "postgresql");
        assert_eq!(
            pg,
            [Row {
                id: "pg".into(),
                endpoint: "db.local:5432".into(),
                services: "orders, billing".into(),
                database: "shop".into(),
            }]
        );
        // A datastore running in the topology has no host: the address stays empty, not its id.
        let mut running = store("compose.yaml::redis", Some("redis"), None, None);
        running.service = Some("compose.yaml::redis".into());
        t.datastores.nodes.push(running);
        let redis = rows(&t, "redis");
        assert_eq!(redis[1].endpoint, "");
        assert_eq!(redis[0].services, "orders");
        let docs = rows(&t, "mongodb");
        assert_eq!(docs[0].endpoint, "mongo.local");
        let unknown = rows(&t, "unknown");
        assert_eq!(unknown[0].id, "mystery");
        assert_eq!(unknown[0].endpoint, "");
    }
}
