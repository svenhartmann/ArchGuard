//! A relational sub-item: per datastore the services with database name and tables (ADR 0012),
//! the shared datastores as warnings, and the tables no single datastore can be named for.
//! Everything is read from `ServiceTopology.datastores`; nothing is recomputed.

use archguard_engine::core::{ServiceTopology, SharedDatastore, SharedDatastoreScope};

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

fn table(t: &ServiceTopology, service: &str, name: &str) -> Table {
    let e = t
        .datastores
        .tables
        .iter()
        .find(|x| x.service == service && x.name == name)
        .and_then(|x| x.evidence.first());
    let file = e.map(|e| e.file.clone()).unwrap_or_default();
    Table {
        name: name.into(),
        line: e.and_then(|e| e.line).unwrap_or(0),
        clickable: !file.is_empty() && !file.contains(':'),
        file,
    }
}

fn svc(t: &ServiceTopology, service: &str, database: String, names: &[String]) -> Svc {
    let mut names = names.to_vec();
    names.sort();
    Svc {
        name: name(t, service).into(),
        database,
        tables: names.iter().map(|n| table(t, service, n)).collect(),
    }
}

fn warn(t: &ServiceTopology, s: &SharedDatastore) -> Warn {
    let mut names: Vec<&str> = s.services.iter().map(|id| name(t, id)).collect();
    names.sort();
    let services = names.join(", ");
    match s.scope {
        SharedDatastoreScope::Table => Warn::Table {
            tables: s.tables.join(", "),
            services,
        },
        SharedDatastoreScope::Database => Warn::Database {
            database: s.database.clone().unwrap_or_default(),
            services,
        },
        SharedDatastoreScope::Server => Warn::Server { services },
    }
}

/// The datastores of `engine` with their services and warnings.
pub fn blocks(t: &ServiceTopology, engine: &str) -> Vec<Block> {
    t.datastores
        .nodes
        .iter()
        .filter(|n| engine_of(&n.engine) == engine)
        .map(|n| Block {
            id: n.id.clone(),
            services: used(t, &n.id)
                .map(|u| {
                    svc(
                        t,
                        &u.service,
                        u.database.clone().unwrap_or_default(),
                        &u.tables,
                    )
                })
                .collect(),
            warnings: t
                .datastores
                .shared
                .iter()
                .filter(|s| s.datastore == n.id)
                .map(|s| warn(t, s))
                .collect(),
        })
        .collect()
}

/// One display line of the sub-item, flat for the view.
#[derive(Debug, PartialEq, Eq)]
pub struct Line {
    /// 0 datastore, 1 service, 2 table, 3 shared table, 4 shared database, 5 shared server.
    pub kind: i32,
    pub text: String,
    pub detail: String,
    pub file: String,
    pub line: i32,
    pub clickable: bool,
}

fn line(kind: i32, text: &str, detail: &str) -> Line {
    Line {
        kind,
        text: text.into(),
        detail: detail.into(),
        file: String::new(),
        line: 0,
        clickable: false,
    }
}

fn service_lines(s: &Svc, out: &mut Vec<Line>) {
    out.push(line(1, &s.name, &s.database));
    out.extend(s.tables.iter().map(|t| Line {
        file: t.file.clone(),
        line: t.line as i32,
        clickable: t.clickable,
        ..line(2, &t.name, "")
    }));
}

/// The lines of the active sub-item: the engine's datastores, or the tables without datastore.
pub fn lines(t: &ServiceTopology, engine: &Engine) -> Vec<Line> {
    let mut out = Vec::new();
    if engine.orphan {
        orphans(t).iter().for_each(|s| service_lines(s, &mut out));
        return out;
    }
    for b in blocks(t, &engine.name) {
        out.push(line(0, &b.id, ""));
        b.services.iter().for_each(|s| service_lines(s, &mut out));
        out.extend(b.warnings.iter().map(|w| match w {
            Warn::Table { tables, services } => line(3, tables, services),
            Warn::Database { database, services } => line(4, database, services),
            Warn::Server { services } => line(5, "", services),
        }));
    }
    out
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
pub fn orphans(t: &ServiceTopology) -> Vec<Svc> {
    let mut ids: Vec<&str> = t
        .datastores
        .tables
        .iter()
        .map(|x| x.service.as_str())
        .collect();
    ids.dedup();
    let mut seen: Vec<&str> = Vec::new();
    let mut out = Vec::new();
    for id in ids {
        let placed = t
            .datastores
            .usages
            .iter()
            .any(|u| u.service == id && !u.tables.is_empty());
        if placed || seen.contains(&id) {
            continue;
        }
        seen.push(id);
        let names: Vec<String> = t
            .datastores
            .tables
            .iter()
            .filter(|x| x.service == id)
            .map(|x| x.name.clone())
            .collect();
        out.push(svc(t, id, String::new(), &names));
    }
    out
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
        assert_eq!(
            (orders.name.as_str(), orders.database.as_str()),
            ("orders", "shop")
        );
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
        assert!(
            !billing.tables[0].clickable,
            "a project prefix is not an editor path"
        );
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
    fn lines_flatten_the_blocks_for_the_view() {
        let t = topology(SharedDatastoreScope::Table);
        let e = Engine {
            name: "postgresql".into(),
            count: 1,
            orphan: false,
        };
        let kinds: Vec<_> = lines(&t, &e).iter().map(|l| l.kind).collect();
        assert_eq!(kinds, [0, 1, 2, 2, 1, 2, 3]);
        let l = &lines(&t, &e)[3];
        assert_eq!(
            (l.file.as_str(), l.line, l.clickable),
            ("orders/Order.java", 7, true)
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
