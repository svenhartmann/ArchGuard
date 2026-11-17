//! Own Mermaid in a chapter as a catalogue diagram, so the editor draws it
//! like the generated ones: flowcharts (`flowchart`, `graph`) and C4
//! (`C4Context`, `C4Container`, …) with their nodes, boxes and arrows.
//! shortcut: subgraphs, `&` chains, styles and other diagram types are not
//! read (they stay source text); add them when a chapter needs them.

use archguard_engine::core::{
    Diagram, DiagramElement, DiagramKind, DiagramRelation, ElementKind, RelationKind,
};

/// The diagram `source` (the inside of a ```` ```mermaid ```` block)
/// draws, if it is a kind this reads and has at least one element.
pub fn diagram(source: &str) -> Option<Diagram> {
    let mut lines = source
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("%%"));
    let head = lines.next()?;
    let mut d = Diagram {
        // Not empty: the view takes an empty id for a diagram it lacks.
        id: "mermaid".into(),
        chapter: 0,
        title: String::new(),
        kind: DiagramKind::Packages,
        note: String::new(),
        unit: String::new(),
        elements: Vec::new(),
        relations: Vec::new(),
        shown: true,
    };
    let word = head.split_whitespace().next().unwrap_or_default();
    if matches!(word, "flowchart" | "graph") {
        for line in lines {
            for statement in line.split(';') {
                flow_statement(statement.trim(), &mut d);
            }
        }
    } else if word.starts_with("C4") {
        let mut groups: Vec<String> = Vec::new();
        for line in lines {
            c4_line(line, &mut groups, &mut d);
        }
    } else {
        return None;
    }
    (!d.elements.is_empty()).then_some(d)
}

fn element(d: &mut Diagram, id: &str, name: Option<String>, kind: ElementKind) {
    match d.elements.iter_mut().find(|e| e.id == id) {
        Some(e) => {
            if let Some(name) = name {
                e.name = name;
                e.kind = kind;
            }
        }
        None => d.elements.push(DiagramElement {
            id: id.into(),
            name: name.unwrap_or_else(|| id.into()),
            kind,
            technology: None,
            detail: None,
            group: None,
            cycle: false,
        }),
    }
}

fn relation(d: &mut Diagram, from: &str, to: &str, label: Option<String>, dotted: bool) {
    d.relations.push(DiagramRelation {
        from: from.into(),
        to: to.into(),
        kind: if dotted {
            RelationKind::Reference
        } else {
            RelationKind::Dependency
        },
        count: 0,
        cycle: false,
        label: label.filter(|l| !l.is_empty()),
        error: false,
        caller: None,
        reply: false,
    });
}

/// Label text as shown: quotes off, line breaks as spaces.
fn label(text: &str) -> String {
    let text = text.trim().trim_matches('"');
    ["<br/>", "<br />", "<br>"]
        .iter()
        .fold(text.to_string(), |t, br| t.replace(br, " "))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

const SKIPPED: &[&str] = &[
    "subgraph",
    "end",
    "classDef",
    "class",
    "style",
    "linkStyle",
    "click",
    "direction",
];

/// One flowchart statement: a node, or nodes joined by links.
fn flow_statement(s: &str, d: &mut Diagram) {
    if s.is_empty() || SKIPPED.contains(&s.split_whitespace().next().unwrap_or_default()) {
        return;
    }
    let Some((mut from, mut rest)) = node(s, d) else {
        return;
    };
    while let Some((text, dotted, after)) = link(rest) {
        let Some((to, after)) = node(after, d) else {
            return;
        };
        relation(d, &from, &to, text, dotted);
        (from, rest) = (to, after);
    }
}

/// A node at the start of `s` (its id, with its shape and label if
/// given), and the text after it.
fn node<'a>(s: &'a str, d: &mut Diagram) -> Option<(String, &'a str)> {
    let s = s.trim_start();
    let end = s
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(s.len());
    if end == 0 {
        return None;
    }
    let (id, mut rest) = s.split_at(end);
    let mut name = None;
    let mut kind = ElementKind::Package;
    if rest.starts_with(['[', '(', '{', '>']) {
        if rest.starts_with("[(") {
            kind = ElementKind::Database;
        }
        let open = rest.len()
            - rest
                .trim_start_matches(['[', '(', '{', '>', '/', '\\'])
                .len();
        let inner = &rest[open..];
        let close = if let Some(quoted) = inner.strip_prefix('"') {
            quoted.find('"').map(|i| i + 2)?
        } else {
            inner.find([']', ')', '}'])?
        };
        name = Some(label(inner[..close].trim_end_matches(['/', '\\'])));
        let after = &inner[close..];
        rest = after.trim_start_matches([']', ')', '}', '/', '\\']);
    }
    if let Some(class) = rest.strip_prefix(":::") {
        rest = class.trim_start_matches(|c: char| c.is_alphanumeric() || c == '_' || c == '-');
    }
    element(d, id, name, kind);
    Some((id.to_string(), rest))
}

/// A link at the start of `s`: its text, whether it is dotted, and the
/// text after it. Arrows `-->`, `---`, `-.->`, `==>`, `<-->` with an
/// optional `|text|`, or text inside them (`-- text -->`, `-. text .->`).
fn link(s: &str) -> Option<(Option<String>, bool, &str)> {
    let s = s.trim_start();
    let arrow = |s: &str| s.len() - s.trim_start_matches(['-', '=', '.', '<', '>']).len();
    let n = arrow(s);
    if n < 2 {
        return None;
    }
    let (head, mut rest) = s.split_at(n);
    let mut dotted = head.contains('.');
    let mut text = None;
    if matches!(head, "--" | "-." | "==") {
        // Text inside the arrow, up to its end.
        let end = ["-->", "---", ".->", ".-", "==>", "==="]
            .iter()
            .filter_map(|a| rest.find(a))
            .min()?;
        text = Some(label(&rest[..end]));
        let after = &rest[end..];
        let close = arrow(after);
        dotted |= after[..close].contains('.');
        rest = &after[close..];
    }
    let rest = rest.trim_start();
    if let Some(piped) = rest.strip_prefix('|') {
        let end = piped.find('|')?;
        return Some((Some(label(&piped[..end])), dotted, &piped[end + 1..]));
    }
    Some((text, dotted, rest))
}

/// One C4 line: an element, a relation, or a boundary that groups the
/// elements up to its `}`.
fn c4_line(line: &str, groups: &mut Vec<String>, d: &mut Diagram) {
    if line.starts_with('}') {
        groups.pop();
        return;
    }
    let Some((name, args)) = line.split_once('(') else {
        return;
    };
    let Some(args) = args.rfind(')').map(|i| &args[..i]) else {
        return;
    };
    let args: Vec<String> = split_args(args)
        .into_iter()
        .filter(|a| !a.starts_with('$'))
        .collect();
    let arg = |i: usize| args.get(i).cloned().filter(|a| !a.is_empty());
    let name = name.trim();
    if name.contains("Boundary") {
        if line.trim_end().ends_with('{') {
            groups.push(arg(1).or_else(|| arg(0)).unwrap_or_default());
        }
        return;
    }
    if name == "Rel" || name == "BiRel" || name.starts_with("Rel_") {
        if let (Some(from), Some(to)) = (arg(0), arg(1)) {
            let text = match (arg(2), arg(3)) {
                (Some(l), Some(t)) => Some(format!("{l} [{t}]")),
                (l, _) => l,
            };
            relation(d, &from, &to, text, false);
        }
        return;
    }
    let kind = if name.starts_with("Person") {
        ElementKind::Person
    } else if name.contains("Db") {
        ElementKind::Database
    } else if name.ends_with("_Ext") {
        ElementKind::External
    } else if name.starts_with("Container") {
        ElementKind::Container
    } else if name.starts_with("System") || name.starts_with("Component") {
        ElementKind::Package
    } else {
        return;
    };
    let Some(id) = arg(0) else { return };
    element(d, &id, Some(arg(1).unwrap_or_else(|| id.clone())), kind);
    // Containers and components have a technology before the description.
    let (technology, detail) = if name.starts_with("Container") || name.starts_with("Component") {
        (arg(2), arg(3))
    } else {
        (None, arg(2))
    };
    if let Some(e) = d.elements.iter_mut().find(|e| e.id == id) {
        e.technology = technology;
        e.detail = detail;
        e.group = groups.last().cloned();
    }
}

/// Arguments split at commas outside quotes, trimmed and unquoted.
fn split_args(args: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut quoted = false;
    for c in args.chars() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(String::new()),
            _ => out.last_mut().expect("one").push(c),
        }
    }
    out.iter().map(|a| label(a)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(d: &Diagram) -> Vec<(&str, &str, ElementKind)> {
        d.elements
            .iter()
            .map(|e| (e.id.as_str(), e.name.as_str(), e.kind))
            .collect()
    }

    fn links(d: &Diagram) -> Vec<(&str, &str, Option<&str>, RelationKind)> {
        d.relations
            .iter()
            .map(|r| (r.from.as_str(), r.to.as_str(), r.label.as_deref(), r.kind))
            .collect()
    }

    #[test]
    fn a_flowchart_has_its_nodes_shapes_and_links() {
        let d = diagram(
            "flowchart LR\n  %% comment\n  K[Kunde] --> W[\"Web und<br/>Shop\"]\n  W -->|bestellt| O(Bestellung)\n  O --> DB[(Datenbank)]\n  O -.-> M{Mail}\n  W -- liest --> K\n  O -. meldet .-> L\n  A --> B --> C; style A fill:#fff\n  subgraph x\n  end\n",
        )
        .expect("a diagram");
        assert_eq!(
            names(&d),
            [
                ("K", "Kunde", ElementKind::Package),
                ("W", "Web und Shop", ElementKind::Package),
                ("O", "Bestellung", ElementKind::Package),
                ("DB", "Datenbank", ElementKind::Database),
                ("M", "Mail", ElementKind::Package),
                ("L", "L", ElementKind::Package),
                ("A", "A", ElementKind::Package),
                ("B", "B", ElementKind::Package),
                ("C", "C", ElementKind::Package),
            ]
        );
        assert_eq!(
            links(&d),
            [
                ("K", "W", None, RelationKind::Dependency),
                ("W", "O", Some("bestellt"), RelationKind::Dependency),
                ("O", "DB", None, RelationKind::Dependency),
                ("O", "M", None, RelationKind::Reference),
                ("W", "K", Some("liest"), RelationKind::Dependency),
                ("O", "L", Some("meldet"), RelationKind::Reference),
                ("A", "B", None, RelationKind::Dependency),
                ("B", "C", None, RelationKind::Dependency),
            ]
        );
    }

    #[test]
    fn a_c4_diagram_has_people_systems_relations_and_boundaries() {
        let d = diagram(
            "C4Context\n  title Shop\n  Person(kunde, \"Kunde\", \"Kauft ein\")\n  Enterprise_Boundary(b, \"Shop\") {\n    System(shop, \"Shop\", \"Verkauft, liefert\")\n    ContainerDb(db, \"Datenbank\", \"PostgreSQL\", \"Bestellungen\")\n  }\n  System_Ext(mail, \"Mail\")\n  Rel(kunde, shop, \"Bestellt\", \"HTTPS\")\n  BiRel(shop, mail, \"Sendet\")\n  UpdateRelStyle(kunde, shop, $offsetX=\"-30\")\n",
        )
        .expect("a diagram");
        assert_eq!(
            names(&d),
            [
                ("kunde", "Kunde", ElementKind::Person),
                ("shop", "Shop", ElementKind::Package),
                ("db", "Datenbank", ElementKind::Database),
                ("mail", "Mail", ElementKind::External),
            ]
        );
        assert_eq!(d.elements[1].detail.as_deref(), Some("Verkauft, liefert"));
        assert_eq!(d.elements[2].technology.as_deref(), Some("PostgreSQL"));
        assert_eq!(d.elements[2].group.as_deref(), Some("Shop"));
        assert_eq!(d.elements[3].group, None);
        assert_eq!(
            links(&d),
            [
                (
                    "kunde",
                    "shop",
                    Some("Bestellt [HTTPS]"),
                    RelationKind::Dependency
                ),
                ("shop", "mail", Some("Sendet"), RelationKind::Dependency),
            ]
        );
    }

    #[test]
    fn other_diagram_kinds_and_empty_ones_are_not_read() {
        assert!(diagram("sequenceDiagram\n  A->>B: hi\n").is_none());
        assert!(diagram("flowchart LR\n").is_none());
        assert!(diagram("").is_none());
    }
}
