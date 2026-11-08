//! Chapter 3: a system context derived from the code (ADR 0006). Users
//! reach the system through its applications (manifest projects of kind
//! `application`); around it the programs it starts, the hosts it
//! addresses over HTTP and the datastores it is configured for.

use archguard_core::{
    Diagram, DiagramElement, DiagramKind, DiagramRelation, ElementKind, RelationKind,
    ServiceTopology,
};

use crate::arc42_blocks::Arc42Context;

/// One element around the system: name, kind, technology, relation, files.
type Outside = (String, ElementKind, String, RelationKind, usize);

/// Programs, hosts and datastores around the system.
fn outside(facts: &Arc42Context, services: &ServiceTopology) -> Vec<Outside> {
    let programs = facts.programs.iter().map(|(n, f)| {
        (
            n.clone(),
            ElementKind::External,
            "Programm".into(),
            RelationKind::Runs,
            *f,
        )
    });
    let hosts = facts.hosts.iter().map(|(h, f)| {
        (
            h.clone(),
            ElementKind::External,
            "HTTP".into(),
            RelationKind::Http,
            *f,
        )
    });
    let stores = services.datastores.nodes.iter().map(|d| {
        let name = d.host.clone().unwrap_or_else(|| d.id.clone());
        let engine = d.engine.clone().unwrap_or_else(|| "Datenbank".into());
        (name, ElementKind::Database, engine, RelationKind::Data, 1)
    });
    programs.chain(hosts).chain(stores).collect()
}

/// How users reach the system: its applications, if the manifests name any.
fn entries(services: &ServiceTopology) -> Vec<&str> {
    services
        .projects
        .iter()
        .filter(|p| p.kind == "application")
        .map(|p| p.name.as_str())
        .collect()
}

pub fn diagram(system: &str, facts: &Arc42Context, services: &ServiceTopology) -> Option<Diagram> {
    let around = outside(facts, services);
    if around.is_empty() {
        return None;
    }
    let apps = entries(services).join(", ");
    let reach = if apps.is_empty() {
        "nutzt".to_string()
    } else {
        format!("nutzt über {apps}")
    };
    let mut elements = vec![
        element(0, "Nutzer", ElementKind::Person, None, None),
        element(
            1,
            system,
            ElementKind::Container,
            Some("System"),
            (!apps.is_empty()).then(|| apps.clone()),
        ),
    ];
    let mut relations = vec![relation(0, 1, RelationKind::Reference, 1, Some(reach))];
    for (name, kind, tech, rel, files) in around {
        let i = elements.len();
        elements.push(element(i, &name, kind, Some(&tech), None));
        relations.push(relation(1, i, rel, files, None));
    }
    Some(Diagram {
        id: "context".into(),
        chapter: 3,
        title: "Kontext (aus dem Code abgeleitet)".into(),
        kind: DiagramKind::Context,
        note: NOTE.into(),
        unit: "Dateien".into(),
        elements,
        relations,
        shown: true,
    })
}

const NOTE: &str = "Quelle: Anwendungen aus den Manifesten (Einstiege der Nutzer), im Quelltext gestartete Programme (z. B. Command::new, subprocess, exec.Command, ProcessBuilder, Process.Start, spawn), angesprochene HTTP-Hosts (nur in Dateien mit HTTP-Client) und konfigurierte Datenbanken. Tests und fremder Code sind ausgenommen; Zahlen sind Dateien. Statisch: weder Vollständigkeit noch tatsächliche Nutzung ist belegt.";

fn element(
    i: usize,
    name: &str,
    kind: ElementKind,
    technology: Option<&str>,
    detail: Option<String>,
) -> DiagramElement {
    DiagramElement {
        id: format!("e{i}"),
        name: name.into(),
        kind,
        technology: technology.map(str::to_owned),
        detail,
        group: None,
        cycle: false,
    }
}

fn relation(
    from: usize,
    to: usize,
    kind: RelationKind,
    count: usize,
    label: Option<String>,
) -> DiagramRelation {
    DiagramRelation {
        from: format!("e{from}"),
        to: format!("e{to}"),
        kind,
        count,
        cycle: false,
        label,
        error: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn users_the_system_and_what_it_starts_and_reaches() {
        let facts = Arc42Context {
            programs: vec![("git".into(), 3), ("node".into(), 1)],
            hosts: vec![("api.github.com".into(), 1)],
        };
        let services: ServiceTopology = serde_json::from_value(serde_json::json!({
            "schema_version":1,"root":"p","monorepo":false,"monorepo_reason":"","compose_files":[],"services":[],
            "projects":[{"id":"a","name":"shop-cli","path":".","manifest":"Cargo.toml","ecosystem":"cargo","kind":"application","runtime":null,"evidence":[]},
                        {"id":"b","name":"shop-core","path":"core","manifest":"core/Cargo.toml","ecosystem":"cargo","kind":"library","runtime":null,"evidence":[]}],
            "dependencies":[],"cycles":[],"diagnostics":[],"complete":true
        }))
        .unwrap();
        let d = diagram("shop", &facts, &services).unwrap();
        assert_eq!((d.chapter, d.kind), (3, DiagramKind::Context));
        let names: Vec<&str> = d.elements.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Nutzer", "shop", "git", "node", "api.github.com"]);
        assert_eq!(
            d.relations[0].label.as_deref(),
            Some("nutzt über shop-cli"),
            "only applications are entries"
        );
        assert_eq!(
            (d.relations[1].kind, d.relations[1].count),
            (RelationKind::Runs, 3)
        );
        let m = super::super::render::mermaid(&d);
        assert!(
            m.starts_with("C4Context\n") && m.contains("Person(e0, \"Nutzer\""),
            "{m}"
        );
        assert!(m.contains("Rel(e0, e1, \"nutzt über shop-cli\""), "{m}");
        assert!(
            diagram(
                "shop",
                &Arc42Context::default(),
                &ServiceTopology::default()
            )
            .is_none()
        );
    }
}
