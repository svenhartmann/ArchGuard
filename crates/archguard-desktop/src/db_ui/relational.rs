//! A relational sub-item: per datastore the services with database name and tables (ADR 0012),
//! the shared datastores as warnings, and the tables no single datastore can be named for.
//! Everything is read from `ServiceTopology.datastores`; nothing is recomputed.

use archguard_engine::core::{ServiceTopology, SharedDatastoreScope};

use super::model::{Engine, engine_of, name, used};

/// A table with its evidence; a file carrying a `<project>:` prefix is not clickable.
#[derive(Debug, PartialEq, Eq)]
pub struct Table {
    pub name: String,
    pub file: String,
    pub line: u32,
    pub clickable: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Svc {
    pub name: String,
    pub database: String,
    pub tables: Vec<Table>,
}

/// A shared datastore: the tables, the database name, or just the server.
#[derive(Debug, PartialEq, Eq)]
pub enum Warn {
    Table { tables: String, services: String },
    Database { database: String, services: String },
    Server { services: String },
}

#[derive(Debug, PartialEq, Eq)]
pub struct Block {
    pub id: String,
    pub services: Vec<Svc>,
    pub warnings: Vec<Warn>,
}

/// The datastores of `engine` with their services and warnings.
pub fn blocks(_t: &ServiceTopology, _engine: &str) -> Vec<Block> {
    let _ = (engine_of(&None), name, used);
    let _ = SharedDatastoreScope::Table;
    Vec::new()
}

/// The sub-item "tables without datastore", when there are such tables.
pub fn sub_item(t: &ServiceTopology) -> Option<Engine> {
    let count = orphans(t).iter().map(|s| s.tables.len()).sum();
    (count > 0).then(|| Engine {
        name: String::new(),
        count,
        orphan: true,
    })
}

/// Services with tables whose datastore is not unique: their tables, no database.
pub fn orphans(_t: &ServiceTopology) -> Vec<Svc> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use archguard_engine::core::{
        Datastore, DatastoreUsage, Service, ServiceEvidence, ServiceTable, SharedDatastore,
    };

    use super::*;

    fn ev(file: &str, line: u32) -> ServiceEvidence {
        ServiceEvidence {
            file: file.into(),
            pointer: String::new(),
            line: Some(line),
            description: String::new(),
        }
    }

    fn svc(id: &str) -> Service {
        Service {
            id: id.into(),
            name: id.rsplit("::").next().unwrap().into(),
            compose_file: "compose.yaml".into(),
            build_context: None,
            networks: vec![],
            profiles: vec![],
            project: None,
        }
    }

    fn table(service: &str, name: &str, file: &str, line: u32) -> ServiceTable {
        ServiceTable {
            service: service.into(),
            name: name.into(),
            evidence: vec![ev(file, line)],
        }
    }

    fn usage(service: &str, ds: &str, db: Option<&str>, tables: &[&str]) -> DatastoreUsage {
        DatastoreUsage {
            service: service.into(),
            datastore: ds.into(),
            database: db.map(Into::into),
            evidence: vec![],
            tables: tables.iter().map(|t| t.to_string()).collect(),
        }
    }

    fn shared(scope: SharedDatastoreScope, db: Option<&str>, tables: &[&str]) -> SharedDatastore {
        SharedDatastore {
            datastore: "pg".into(),
            scope,
            database: db.map(Into::into),
            services: vec!["c::orders".into(), "c::billing".into()],
            tables: tables.iter().map(|t| t.to_string()).collect(),
            disjoint_tables: false,
        }
    }

    fn topology(scope: SharedDatastoreScope) -> ServiceTopology {
        let mut t = ServiceTopology {
            services: vec![svc("c::orders"), svc("c::billing")],
            ..Default::default()
        };
        t.datastores.nodes = vec![Datastore {
            id: "pg".into(),
            engine: Some("postgresql".into()),
            service: None,
            host: None,
            port: None,
            evidence: vec![],
        }];
        t.datastores.usages = vec![
            usage("c::orders", "pg", Some("shop"), &["orders", "lines"]),
            usage("c::billing", "pg", Some("shop"), &["orders"]),
        ];
        t.datastores.tables = vec![
            table("c::orders", "orders", "orders/Order.java", 7),
            table("c::orders", "lines", "orders/Line.java", 3),
            table("c::billing", "orders", "other:billing/Bill.java", 9),
        ];
        let tables: &[&str] = if scope == SharedDatastoreScope::Table {
            &["orders"]
        } else {
            &[]
        };
        t.datastores.shared = vec![shared(scope, Some("shop"), tables)];
        t
    }

    #[test]
    fn a_datastore_lists_its_services_with_database_and_tables_with_evidence() {
        let b = blocks(&topology(SharedDatastoreScope::Table), "postgresql");
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].id, "pg");
        let [orders, billing] = &b[0].services[..] else {
            panic!("two services: {:?}", b[0].services)
        };
        assert_eq!((orders.name.as_str(), orders.database.as_str()), ("orders", "shop"));
        let names: Vec<_> = orders.tables.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["lines", "orders"]);
        assert_eq!(
            orders.tables[1],
            Table {
                name: "orders".into(),
                file: "orders/Order.java".into(),
                line: 7,
                clickable: true
            }
        );
        assert_eq!(billing.name, "billing");
        assert_eq!(billing.tables.len(), 1);
        assert!(!billing.tables[0].clickable, "a project prefix is not an editor path");
    }

    #[test]
    fn the_scope_gives_the_warning() {
        let w = |s| blocks(&topology(s), "postgresql").remove(0).warnings;
        assert_eq!(
            w(SharedDatastoreScope::Table),
            [Warn::Table {
                tables: "orders".into(),
                services: "billing, orders".into()
            }]
        );
        assert_eq!(
            w(SharedDatastoreScope::Database),
            [Warn::Database {
                database: "shop".into(),
                services: "billing, orders".into()
            }]
        );
        assert_eq!(
            w(SharedDatastoreScope::Server),
            [Warn::Server {
                services: "billing, orders".into()
            }]
        );
    }

    #[test]
    fn other_engines_give_no_block() {
        assert!(blocks(&topology(SharedDatastoreScope::Table), "redis").is_empty());
    }

    #[test]
    fn tables_of_a_service_with_several_datastores_get_their_own_sub_item() {
        let mut t = topology(SharedDatastoreScope::Server);
        assert!(orphans(&t).is_empty());
        assert_eq!(sub_item(&t), None, "no sub-item without such tables");
        // `orders` uses a second datastore: its usages carry no tables.
        t.datastores.usages[0].tables.clear();
        t.datastores
            .usages
            .push(usage("c::orders", "pg2", None, &[]));
        let o = orphans(&t);
        assert_eq!(o.len(), 1);
        assert_eq!(o[0].name, "orders");
        let names: Vec<_> = o[0].tables.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["lines", "orders"]);
        let e = sub_item(&t).unwrap();
        assert!(e.orphan);
        assert_eq!(e.count, 2);
    }
}
