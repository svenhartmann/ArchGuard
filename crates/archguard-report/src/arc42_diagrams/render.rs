//! Mermaid and PlantUML text of a catalogue diagram: package diagrams as
//! flowchart / packages, C4 diagrams with Mermaid's C4 syntax and the
//! C4-PlantUML standard library (as the web viewer's export).

use archguard_core::{
    Diagram, DiagramElement, DiagramKind, DiagramRelation, ElementKind, RelationKind,
};
use std::fmt::Write;

/// The relation's label in the document's language.
fn label(r: &DiagramRelation, unit: &str) -> String {
    match r.kind {
        RelationKind::Dependency | RelationKind::Call => counted(r.count, unit),
        RelationKind::Startup => "benötigt Start von".into(),
        RelationKind::Http => "HTTP".into(),
        RelationKind::Grpc => "gRPC".into(),
        RelationKind::Event => "Event".into(),
        RelationKind::Reference => "Konfigurationsverweis".into(),
        RelationKind::Data => "nutzt Datenbank".into(),
    }
}

/// `n` with its unit, singular for one.
pub fn counted(n: usize, unit: &str) -> String {
    let one = match unit {
        "Quellverbindungen" => "Quellverbindung",
        "Aufrufstellen" => "Aufrufstelle",
        "Konfigurationsbelege" => "Konfigurationsbeleg",
        other => other,
    };
    format!("{n} {}", if n == 1 { one } else { unit })
}

/// Groups in first-use order, each with its elements; ungrouped last.
fn groups(d: &Diagram) -> Vec<(Option<&str>, Vec<&DiagramElement>)> {
    let mut out: Vec<(Option<&str>, Vec<&DiagramElement>)> = Vec::new();
    for e in &d.elements {
        let g = e.group.as_deref();
        match out.iter_mut().find(|(k, _)| *k == g) {
            Some((_, list)) => list.push(e),
            None => out.push((g, vec![e])),
        }
    }
    out.sort_by_key(|(g, _)| g.is_none());
    out
}

/// Mermaid label text: anything beyond plain characters as an entity.
fn entity(text: &str) -> String {
    text.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || " /._-".contains(ch) {
                ch.to_string()
            } else {
                format!("#{};", ch as u32)
            }
        })
        .collect()
}

/// A quoted C4 macro argument; quotes would end it.
fn arg(text: &str) -> String {
    format!("\"{}\"", text.replace('"', "'"))
}

pub fn mermaid(d: &Diagram) -> String {
    match d.kind {
        DiagramKind::Packages => mermaid_flowchart(d),
        DiagramKind::Deployment => c4(d, C4Text::Mermaid("C4Deployment")),
        DiagramKind::Communication => c4(d, C4Text::Mermaid("C4Container")),
    }
}

pub fn plantuml(d: &Diagram) -> String {
    match d.kind {
        DiagramKind::Packages => plantuml_packages(d),
        DiagramKind::Deployment => c4(d, C4Text::Plantuml("C4_Deployment")),
        DiagramKind::Communication => c4(d, C4Text::Plantuml("C4_Container")),
    }
}

fn mermaid_flowchart(d: &Diagram) -> String {
    let mut out = String::from("flowchart LR\n");
    for (n, (group, elements)) in groups(d).into_iter().enumerate() {
        let indent = if group.is_some() { "    " } else { "  " };
        if let Some(g) = group {
            let _ = writeln!(out, "  subgraph g{n}[\"{}\"]", entity(g));
        }
        for e in elements {
            let _ = writeln!(out, "{indent}{}[\"{}\"]", e.id, entity(&e.name));
        }
        if group.is_some() {
            out.push_str("  end\n");
        }
    }
    for r in &d.relations {
        let _ = writeln!(out, "  {} -->|{}| {}", r.from, r.count, r.to);
    }
    let cyclic: Vec<&str> = d
        .elements
        .iter()
        .filter(|e| e.cycle)
        .map(|e| e.id.as_str())
        .collect();
    if !cyclic.is_empty() {
        let _ = writeln!(
            out,
            "  classDef cycle stroke:#d33,stroke-width:2px\n  class {} cycle",
            cyclic.join(",")
        );
    }
    out
}

fn plantuml_packages(d: &Diagram) -> String {
    let mut out = String::from("@startuml\nskinparam shadowing false\n");
    for (group, elements) in groups(d) {
        if let Some(g) = group {
            let _ = writeln!(out, "rectangle {} {{", arg(g));
        }
        for e in elements {
            let shape = if e.kind == ElementKind::Remainder {
                "rectangle"
            } else {
                "package"
            };
            let color = if e.cycle { " #FFE4E6" } else { "" };
            let _ = writeln!(out, "{shape} {} as {}{color}", arg(&e.name), e.id);
        }
        if group.is_some() {
            out.push_str("}\n");
        }
    }
    for r in &d.relations {
        let color = if r.cycle { "#d33" } else { "#607796" };
        let _ = writeln!(
            out,
            "{} -[{color}]-> {} : {}",
            r.from,
            r.to,
            label(r, &d.unit)
        );
    }
    out.push_str("@enduml\n");
    out
}

enum C4Text {
    /// Mermaid diagram type.
    Mermaid(&'static str),
    /// C4-PlantUML library.
    Plantuml(&'static str),
}

/// C4 elements in their deployment nodes, then the relations. Mermaid and
/// C4-PlantUML share the macro names.
fn c4(d: &Diagram, text: C4Text) -> String {
    let mut out = match text {
        C4Text::Mermaid(kind) => format!("{kind}\n  title {}\n", d.title.replace('"', "'")),
        C4Text::Plantuml(lib) => format!("@startuml\n!include <C4/{lib}>\ntitle {}\n", d.title),
    };
    for (n, (group, elements)) in groups(d).into_iter().enumerate() {
        let indent = if group.is_some() { "    " } else { "  " };
        if let Some(g) = group {
            let _ = writeln!(
                out,
                "  Deployment_Node(g{n}, {}, \"Docker Compose\") {{",
                arg(g)
            );
        }
        for e in elements {
            let _ = writeln!(out, "{indent}{}", c4_element(e));
        }
        if group.is_some() {
            out.push_str("  }\n");
        }
    }
    for r in &d.relations {
        let evidence = counted(r.count, &d.unit);
        let _ = writeln!(
            out,
            "  Rel({}, {}, {}, {})",
            r.from,
            r.to,
            arg(&label(r, &d.unit)),
            arg(&evidence)
        );
    }
    if matches!(text, C4Text::Plantuml(_)) {
        out.push_str("@enduml\n");
    }
    out
}

fn c4_element(e: &DiagramElement) -> String {
    let shape = match e.kind {
        ElementKind::Database => "ContainerDb",
        ElementKind::External => "Container_Ext",
        _ => "Container",
    };
    format!(
        "{shape}({}, {}, {}, {})",
        e.id,
        arg(&e.name),
        arg(e.technology.as_deref().unwrap_or("Dienst")),
        arg(e.detail.as_deref().unwrap_or(""))
    )
}
