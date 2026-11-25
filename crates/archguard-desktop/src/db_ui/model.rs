//! The tab's data: the engines of the topology as sub-items and the list rows of an engine.

use archguard_engine::core::ServiceTopology;

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

pub fn engines(_: &ServiceTopology) -> Vec<Engine> {
    Vec::new()
}

pub fn rows(_: &ServiceTopology, _engine: &str) -> Vec<Row> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use archguard_engine::core::{Datastore, DatastoreUsage, SearchArtifact, ServiceTopology};

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

    fn topology() -> ServiceTopology {
        let mut t = ServiceTopology::default();
        t.elasticsearch.artifacts = vec![artifact()];
        t.datastores.nodes = vec![
            store("pg", Some("postgresql"), Some("db.local"), Some(5432)),
            store("cache", Some("redis"), None, None),
            store("docs", Some("mongodb"), Some("mongo.local"), None),
            store("mystery", None, None, None),
        ];
        t.datastores.usages = vec![
            usage("orders", "pg", Some("shop")),
            usage("billing", "pg", None),
            usage("orders", "cache", None),
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
        let t = topology();
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
        let docs = rows(&t, "mongodb");
        assert_eq!(docs[0].endpoint, "mongo.local");
        let unknown = rows(&t, "unknown");
        assert_eq!(unknown[0].id, "mystery");
        assert_eq!(unknown[0].endpoint, "");
    }
}
