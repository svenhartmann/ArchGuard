//! C4 diagrams from the static service topology: one deployment diagram
//! per Compose or manifest file (chapter 7) and the communication between
//! services (chapter 6). Configured links are configuration evidence, not
//! measured traffic.

use archguard_core::{
    Diagram, DiagramElement, DiagramKind, DiagramRelation, ElementKind, RelationKind, Service,
    ServiceDependencyKind, ServiceTopology,
};
use std::collections::{BTreeMap, BTreeSet};

pub fn diagrams(t: &ServiceTopology) -> Vec<Diagram> {
    let files: BTreeSet<&str> = t
        .services
        .iter()
        .map(|s| s.compose_file.as_str())
        .filter(|f| !f.is_empty())
        .collect();
    let mut out: Vec<Diagram> = files.into_iter().map(|f| deployment(t, f)).collect();
    out.extend(communication(t));
    out
}

fn kind_of(k: ServiceDependencyKind) -> RelationKind {
    match k {
        ServiceDependencyKind::Startup => RelationKind::Startup,
        ServiceDependencyKind::ConfiguredHttp | ServiceDependencyKind::HttpCall => {
            RelationKind::Http
        }
        ServiceDependencyKind::GrpcCall => RelationKind::Grpc,
        ServiceDependencyKind::Event => RelationKind::Event,
        ServiceDependencyKind::ConfiguredReference => RelationKind::Reference,
    }
}

/// Collects elements by service or datastore id, in first-use order.
struct Builder<'a> {
    t: &'a ServiceTopology,
    elements: Vec<DiagramElement>,
    ids: BTreeMap<String, String>,
    relations: BTreeMap<(String, String, RelationKind), usize>,
}

impl<'a> Builder<'a> {
    fn new(t: &'a ServiceTopology) -> Self {
        Builder {
            t,
            elements: Vec::new(),
            ids: BTreeMap::new(),
            relations: BTreeMap::new(),
        }
    }

    /// The element of service `id`; `group` is its deployment node, or
    /// `None` for a service drawn outside it.
    fn service(&mut self, s: &Service, group: Option<&str>, external: bool) -> String {
        if let Some(id) = self.ids.get(&s.id) {
            return id.clone();
        }
        let store = self
            .t
            .datastores
            .nodes
            .iter()
            .find(|d| d.service.as_deref() == Some(&s.id));
        let kind = match (store, external) {
            (Some(_), _) => ElementKind::Database,
            (None, true) => ElementKind::External,
            (None, false) => ElementKind::Container,
        };
        let project = s
            .project
            .as_ref()
            .and_then(|p| self.t.projects.iter().find(|q| &q.id == p));
        let technology = store
            .and_then(|d| d.engine.clone())
            .or_else(|| project.map(|p| p.runtime.clone().unwrap_or_else(|| p.ecosystem.clone())));
        // Compose puts every service in "default"; it tells nothing.
        let networks = s.networks.iter().filter(|n| n.as_str() != "default");
        let detail = networks
            .chain(&s.profiles)
            .cloned()
            .collect::<Vec<_>>()
            .join(" · ");
        self.push(
            &s.id,
            DiagramElement {
                id: String::new(),
                name: s.name.clone(),
                kind,
                technology,
                detail: (!detail.is_empty()).then_some(detail),
                group: group.map(str::to_owned),
                cycle: false,
            },
        )
    }

    /// The element of datastore `id`: its service's element, else its own.
    fn datastore(&mut self, id: &str, group: Option<&str>) -> Option<String> {
        let d = self.t.datastores.nodes.iter().find(|d| d.id == id)?;
        if let Some(s) = d
            .service
            .as_ref()
            .and_then(|s| self.t.services.iter().find(|x| &x.id == s))
        {
            let inside = group.is_some_and(|g| g == s.compose_file);
            return Some(self.service(s, inside.then_some(s.compose_file.as_str()), false));
        }
        let name = d.host.clone().unwrap_or_else(|| d.id.clone());
        Some(self.push(
            id,
            DiagramElement {
                id: String::new(),
                name,
                kind: ElementKind::Database,
                technology: d.engine.clone(),
                detail: d.port.map(|p| format!("Port {p}")),
                group: None,
                cycle: false,
            },
        ))
    }

    fn push(&mut self, key: &str, mut e: DiagramElement) -> String {
        let id = format!("e{}", self.elements.len());
        e.id = id.clone();
        self.elements.push(e);
        self.ids.insert(key.to_owned(), id.clone());
        id
    }

    fn relate(&mut self, from: String, to: String, kind: RelationKind, count: usize) {
        *self.relations.entry((from, to, kind)).or_default() += count.max(1);
    }

    fn finish(
        self,
        id: String,
        chapter: u8,
        title: String,
        kind: DiagramKind,
        note: &str,
    ) -> Diagram {
        Diagram {
            id,
            chapter,
            title,
            kind,
            note: note.into(),
            unit: "Konfigurationsbelege".into(),
            elements: self.elements.clone(),
            relations: self
                .relations
                .iter()
                // A datastore usage is read from the reference; draw it once.
                .filter(|((from, to, kind), _)| {
                    *kind != RelationKind::Reference
                        || !self.relations.contains_key(&(
                            from.clone(),
                            to.clone(),
                            RelationKind::Data,
                        ))
                })
                .map(|(key, &count)| (key.clone(), count))
                .map(|((from, to, kind), count)| DiagramRelation {
                    from,
                    to,
                    kind,
                    count,
                    cycle: false,
                    label: None,
                    error: false,
                })
                .collect(),
            shown: true,
        }
    }
}

/// Chapter 7: the services of `file` in one deployment node, the
/// datastores they use and the services of other files they reach.
fn deployment(t: &ServiceTopology, file: &str) -> Diagram {
    let mut b = Builder::new(t);
    let local: Vec<&Service> = t
        .services
        .iter()
        .filter(|s| s.compose_file == file)
        .collect();
    for s in &local {
        b.service(s, Some(file), false);
    }
    let inside: BTreeSet<&str> = local.iter().map(|s| s.id.as_str()).collect();
    for d in t
        .dependencies
        .iter()
        .filter(|d| inside.contains(d.from.as_str()))
    {
        let Some(to) = t.services.iter().find(|s| s.id == d.to) else {
            continue;
        };
        let from = b.service(
            local.iter().find(|s| s.id == d.from).unwrap(),
            Some(file),
            false,
        );
        let local_target = to.compose_file == file;
        let to = b.service(to, local_target.then_some(file), !local_target);
        b.relate(from, to, kind_of(d.kind), d.evidence.len());
    }
    for u in t
        .datastores
        .usages
        .iter()
        .filter(|u| inside.contains(u.service.as_str()))
    {
        let from = b.ids[&u.service].clone();
        if let Some(to) = b.datastore(&u.datastore, Some(file)) {
            b.relate(from, to, RelationKind::Data, u.evidence.len());
        }
    }
    b.finish(
        format!("deployment-{file}"),
        7,
        format!("C4 · Deployment · {file}"),
        DiagramKind::Deployment,
        "Quelle: deklarierte Deployment-Konfiguration. Container sind die dort deklarierten Dienste; Laufzeithosts, Skalierung und tatsächlicher Betrieb sind nicht bekannt.",
    )
}

/// Chapter 6: services that call, notify or reference each other, and the
/// datastores they use; startup order is left to chapter 7.
fn communication(t: &ServiceTopology) -> Option<Diagram> {
    let mut b = Builder::new(t);
    for d in t
        .dependencies
        .iter()
        .filter(|d| d.kind != ServiceDependencyKind::Startup)
    {
        let find = |id: &str| t.services.iter().find(|s| s.id == id);
        let (Some(from), Some(to)) = (find(&d.from), find(&d.to)) else {
            continue;
        };
        let (from, to) = (b.service(from, None, false), b.service(to, None, false));
        b.relate(from, to, kind_of(d.kind), d.evidence.len());
    }
    for u in &t.datastores.usages {
        let Some(s) = t.services.iter().find(|s| s.id == u.service) else {
            continue;
        };
        let from = b.service(s, None, false);
        if let Some(to) = b.datastore(&u.datastore, None) {
            b.relate(from, to, RelationKind::Data, u.evidence.len());
        }
    }
    if b.relations.is_empty() {
        return None;
    }
    Some(b.finish(
        "runtime-services".into(),
        6,
        "C4 · Kommunikation zwischen Diensten".into(),
        DiagramKind::Communication,
        "Quelle: konfigurierte und im Code erkannte HTTP-, gRPC- und Event-Verbindungen sowie Datenbanknutzung. Statisch erfasst; keine gemessene Kommunikation und keine Reihenfolge.",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// orders → billing (HTTP), orders → db (startup and data), billing in
    /// another file reaches shipping.
    fn topology() -> ServiceTopology {
        let svc = |id: &str, file: &str| {
            serde_json::json!({"id":id,"name":id,"compose_file":file,"build_context":null,
                "networks":["shop"],"profiles":[],"project":null})
        };
        let dep = |from: &str, to: &str, kind: &str| {
            serde_json::json!({"from":from,"to":to,"kind":kind,"evidence":[
                {"file":"compose.yaml","pointer":"/x","line":null,"description":"d"}]})
        };
        serde_json::from_value(serde_json::json!({
            "schema_version":1,"root":"shop","monorepo":false,"monorepo_reason":"",
            "compose_files":["compose.yaml","other.yaml"],
            "services":[svc("orders","compose.yaml"),svc("db","compose.yaml"),svc("shipping","other.yaml")],
            "projects":[],
            "dependencies":[dep("orders","db","startup"),dep("orders","shipping","configured_http")],
            "cycles":[],"diagnostics":[],"complete":true,
            "datastores":{"nodes":[{"id":"pg","engine":"PostgreSQL","service":"db","host":null,"port":null,"evidence":[]}],
                "usages":[{"service":"orders","datastore":"pg","database":"shop","evidence":[]}],
                "shared":[],"note":""}
        }))
        .unwrap()
    }

    #[test]
    fn one_deployment_per_file_with_containers_databases_and_external_targets() {
        let all = diagrams(&topology());
        let ids: Vec<&str> = all.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "deployment-compose.yaml",
                "deployment-other.yaml",
                "runtime-services"
            ]
        );
        let d = &all[0];
        assert_eq!((d.chapter, d.kind), (7, DiagramKind::Deployment));
        let kind = |name: &str| {
            d.elements
                .iter()
                .find(|e| e.name == name)
                .map(|e| (e.kind, e.group.clone()))
        };
        let file = Some("compose.yaml".to_string());
        assert_eq!(kind("orders"), Some((ElementKind::Container, file.clone())));
        assert_eq!(kind("db"), Some((ElementKind::Database, file)));
        assert_eq!(kind("shipping"), Some((ElementKind::External, None)));
        let kinds: Vec<RelationKind> = d.relations.iter().map(|r| r.kind).collect();
        assert!(kinds.contains(&RelationKind::Startup) && kinds.contains(&RelationKind::Http));
        assert!(kinds.contains(&RelationKind::Data));
        let db = d.elements.iter().find(|e| e.name == "db").unwrap();
        assert_eq!(db.technology.as_deref(), Some("PostgreSQL"));
    }

    #[test]
    fn communication_leaves_startup_order_to_the_deployment_view() {
        let all = diagrams(&topology());
        let c = all.iter().find(|d| d.id == "runtime-services").unwrap();
        assert_eq!((c.chapter, c.kind), (6, DiagramKind::Communication));
        assert!(c.relations.iter().all(|r| r.kind != RelationKind::Startup));
        assert_eq!(c.relations.len(), 2, "{:?}", c.relations);
        assert!(c.elements.iter().all(|e| e.group.is_none()));
    }

    #[test]
    fn a_datastore_reference_is_drawn_once_as_data_use() {
        let mut t = topology();
        let mut reference = t.dependencies[1].clone();
        (reference.to, reference.kind) = ("db".into(), ServiceDependencyKind::ConfiguredReference);
        t.dependencies.push(reference);
        let d = &diagrams(&t)[0];
        let to_db: Vec<RelationKind> = d
            .relations
            .iter()
            .filter(|r| r.to == "e1")
            .map(|r| r.kind)
            .collect();
        assert_eq!(to_db, [RelationKind::Startup, RelationKind::Data]);
    }

    #[test]
    fn no_services_no_diagrams() {
        assert!(diagrams(&ServiceTopology::default()).is_empty());
    }
}
