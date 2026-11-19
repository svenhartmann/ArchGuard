//! Own Mermaid in a chapter as a catalogue diagram, so the editor draws it
//! like the generated ones: flowcharts (`flowchart`, `graph`) and C4
//! (`C4Context`, `C4Container`, …) with their nodes, boxes and arrows, and
//! sequences (`sequenceDiagram`, read in `sequence`).
//! shortcut: subgraphs, `&` chains, styles and other diagram types are not
//! read (they stay source text); add them when a chapter needs them.

use std::ops::Deref;

use archguard_engine::core::{
    Diagram, DiagramElement, DiagramKind, DiagramRelation, ElementKind, RelationKind,
};

mod sequence;
pub use sequence::Item;

/// Own Mermaid as read: the diagram, and for a sequence what lies between
/// its messages (blocks, notes).
pub struct Own {
    pub diagram: Diagram,
    pub items: Vec<Item>,
}

impl Deref for Own {
    type Target = Diagram;
    fn deref(&self) -> &Diagram {
        &self.diagram
    }
}

/// The diagram `source` (the inside of a ```` ```mermaid ```` block)
/// draws, if it is a kind this reads and has at least one element.
pub fn diagram(source: &str) -> Option<Own> {
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
    let mut items = Vec::new();
    if word == "sequenceDiagram" {
        items = sequence::read(lines, &mut d, label)?;
    } else if matches!(word, "flowchart" | "graph") {
        for line in lines {
            for statement in split_unquoted(line, ';') {
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
    (!d.elements.is_empty()).then_some(Own { diagram: d, items })
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
    "accTitle",
    "accDescr",
];

/// One flowchart statement: a node, or nodes joined by links.
fn flow_statement(s: &str, d: &mut Diagram) {
    let first = s.split([' ', ':']).next().unwrap_or_default();
    if s.is_empty() || SKIPPED.contains(&first) {
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
    // `-` and `.` join word characters (`api-gateway`); `-->` ends the id.
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    let end = s
        .char_indices()
        .find(|&(i, c)| {
            let joins = matches!(c, '-' | '.')
                && word(s[..i].chars().last())
                && word(s[i + 1..].chars().next());
            !(word(Some(c)) || joins)
        })
        .map_or(s.len(), |(i, _)| i);
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
        // The shape closes with its opening reversed: `[(` with `)]`.
        let closing: String = rest[..open]
            .chars()
            .rev()
            .map(|c| match c {
                '[' | '>' => ']',
                '(' => ')',
                '{' => '}',
                c => c,
            })
            .collect();
        let inner = &rest[open..];
        let (close, after) = if let Some(quoted) = inner.strip_prefix('"') {
            let close = quoted.find('"').map(|i| i + 2)?;
            let after = inner[close..].trim_start_matches([']', ')', '}', '/', '\\']);
            (close, after)
        } else if let Some(close) = inner.find(&closing) {
            (close, &inner[close + closing.len()..])
        } else {
            // A trapezoid mixes its slashes: `[/text\]`.
            let last = closing.chars().last()?;
            let close = inner.find(last)?;
            (close, &inner[close + last.len_utf8()..])
        };
        name = Some(label(inner[..close].trim_end_matches(['/', '\\'])));
        rest = after;
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
    let first = line.split([' ', ':']).next().unwrap_or_default();
    if matches!(first, "title" | "accTitle" | "accDescr") {
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
    // A boundary or deployment node holds what follows up to its `}`.
    if line.trim_end().ends_with('{') {
        groups.push(arg(1).or_else(|| arg(0)).unwrap_or_default());
        return;
    }
    if name.contains("Boundary") {
        return;
    }
    if name == "Rel" || name == "BiRel" || name.starts_with("Rel_") {
        if let (Some(from), Some(to)) = (arg(0), arg(1)) {
            let text = match (arg(2), arg(3)) {
                (Some(l), Some(t)) => Some(format!("{l} [{t}]")),
                (l, _) => l,
            };
            if name == "Rel_Back" {
                relation(d, &to, &from, text, false);
            } else {
                relation(d, &from, &to, text.clone(), false);
                if name == "BiRel" {
                    relation(d, &to, &from, text, false);
                }
            }
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
    split_unquoted(args, ',').into_iter().map(label).collect()
}

/// `s` split at `sep` outside double quotes.
fn split_unquoted(s: &str, sep: char) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut from, mut quoted) = (0, false);
    for (i, c) in s.char_indices() {
        if c == '"' {
            quoted = !quoted;
        } else if c == sep && !quoted {
            out.push(&s[from..i]);
            from = i + c.len_utf8();
        }
    }
    out.push(&s[from..]);
    out
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
                ("mail", "shop", Some("Sendet"), RelationKind::Dependency),
            ]
        );
    }

    /// Ids with hyphens and dots, `;` inside quotes, words after brackets
    /// in a label and accessibility lines read as Mermaid reads them.
    #[test]
    fn ids_quotes_and_labels_read_as_mermaid_does() {
        let d = diagram(
            "flowchart LR\n  accTitle: Shop\n  accDescr: Wer was nutzt\n  api-gateway --> orders.v2\n  A[\"a; b\"] --> B; B-->C\n  D[Shop (alt) v2] --> E\n",
        )
        .expect("a diagram");
        let ids: Vec<&str> = d.elements.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["api-gateway", "orders.v2", "A", "B", "C", "D", "E"]);
        assert_eq!(d.elements[2].name, "a; b");
        assert_eq!(d.elements[5].name, "Shop (alt) v2");
        let links: Vec<(&str, &str)> = d
            .relations
            .iter()
            .map(|r| (r.from.as_str(), r.to.as_str()))
            .collect();
        assert_eq!(
            links,
            [
                ("api-gateway", "orders.v2"),
                ("A", "B"),
                ("B", "C"),
                ("D", "E")
            ]
        );
    }

    /// `Rel_Back` points back, `BiRel` both ways; deployment nodes group
    /// what they hold; a title is no element.
    #[test]
    fn c4_relations_point_as_written_and_nodes_group() {
        let d = diagram(
            "C4Deployment\n  title MongoDb (prod)\n  Deployment_Node(dc, \"Rechenzentrum\") {\n    Container(app, \"App\")\n  }\n  System_Boundary(b, \"Shop\") {\n    Deployment_Node(k8s, \"Cluster\") {\n      ContainerDb(db, \"DB\")\n    }\n    Container(web, \"Web\")\n  }\n  Rel_Back(app, db, \"liest\")\n  BiRel(app, web, \"spricht\")\n",
        )
        .expect("a diagram");
        let groups: Vec<(&str, Option<&str>)> = d
            .elements
            .iter()
            .map(|e| (e.id.as_str(), e.group.as_deref()))
            .collect();
        assert_eq!(
            groups,
            [
                ("app", Some("Rechenzentrum")),
                ("db", Some("Cluster")),
                ("web", Some("Shop")),
            ]
        );
        let links: Vec<(&str, &str)> = d
            .relations
            .iter()
            .map(|r| (r.from.as_str(), r.to.as_str()))
            .collect();
        assert_eq!(links, [("db", "app"), ("app", "web"), ("web", "app")]);
    }

    /// A label keeps brackets of another kind than its shape's.
    #[test]
    fn labels_keep_their_own_brackets() {
        let d = diagram("flowchart LR\n  A[Shop (alt)] --> B(Liste [neu])\n  C[(Daten (roh))]\n")
            .expect("a diagram");
        assert_eq!(
            names(&d),
            [
                ("A", "Shop (alt)", ElementKind::Package),
                ("B", "Liste [neu]", ElementKind::Package),
                ("C", "Daten (roh)", ElementKind::Database),
            ]
        );
        assert_eq!(links(&d), [("A", "B", None, RelationKind::Dependency)]);
    }

    /// A real chapter's sequence: declared participants, autonumber,
    /// `break` and `alt`/`else` nested inside `alt`.
    const SEARCH: &str = "sequenceDiagram
    autonumber
    participant F as Frontend
    participant A as APIM
    participant G as GLS
    participant E as Elasticsearch
    F->>A: Suchrequest mit Identität
    A->>A: Tokenprüfung laut API-Konzept
    A->>G: Suchrequest und search-authorization
    break Authentifizierung fehlgeschlagen
        G-->>A: HTTP 401
        A-->>F: Fehlerantwort
    end
    G->>G: Suchparameter validieren
    alt Mindestens eine berechtigte Teilabfrage
        G->>E: POST /_msearch/template mit geprüftem Bearer-Token
        alt Auswertbare Multi-Search-Antwort
            E-->>G: Teilantworten
        else Gesamtfehler
            G->>G: HTTP 401 oder 503 vorbereiten
        end
    else Keine berechtigte Teilabfrage
        G->>G: Leere Suchantwort erzeugen
    end
    G-->>A: HTTP-Antwort
    A-->>F: Such- oder Fehlerantwort
";

    #[test]
    fn a_sequence_has_participants_numbered_messages_and_nested_blocks() {
        let d = diagram(SEARCH).expect("a sequence");
        assert_eq!(d.kind, DiagramKind::Sequence);
        assert_eq!(
            names(&d),
            [
                ("F", "Frontend", ElementKind::Package),
                ("A", "APIM", ElementKind::Package),
                ("G", "GLS", ElementKind::Package),
                ("E", "Elasticsearch", ElementKind::Package),
            ]
        );
        assert_eq!(d.relations.len(), 12);
        let l = links(&d);
        assert_eq!(
            l[0],
            (
                "F",
                "A",
                Some("1. Suchrequest mit Identität"),
                RelationKind::Dependency
            )
        );
        assert_eq!(l[1].0, l[1].1, "a message to oneself");
        assert_eq!(
            l[3],
            ("G", "A", Some("4. HTTP 401"), RelationKind::Reference),
            "dashed"
        );
        assert_eq!(l[11].2, Some("12. Such- oder Fehlerantwort"));
        use Item::*;
        let open = |s: &str| Open(s.to_string());
        let divider = |s: &str| Divider(s.to_string());
        assert_eq!(
            d.items,
            [
                Message,
                Message,
                Message,
                open("break [Authentifizierung fehlgeschlagen]"),
                Message,
                Message,
                End,
                Message,
                open("alt [Mindestens eine berechtigte Teilabfrage]"),
                Message,
                open("alt [Auswertbare Multi-Search-Antwort]"),
                Message,
                divider("[Gesamtfehler]"),
                Message,
                End,
                divider("[Keine berechtigte Teilabfrage]"),
                Message,
                End,
                Message,
                Message,
            ]
        );
    }

    /// Implicit participants, actors, every arrow, notes and the lines
    /// read without being drawn.
    #[test]
    fn sequence_arrows_actors_and_notes_read_as_mermaid_does() {
        let d = diagram(
            "sequenceDiagram\n  title Bestellung\n  actor K as Kunde\n  K->>+S: a\n  S-->>-K: b\n  K->S: c\n  K-->S: d\n  K-xS: e\n  K--xS: f\n  K-)Q: g\n  K--)Q\n  activate S\n  deactivate S\n  Note over K,S: Text\n  Note left of Q: links\n  loop Jede Minute\n    opt\n    rect rgb(0,0,0)\n    par\n      S->>Q: h\n    and\n      S->>K: i\n    end\n    end\n    end\n  end\n  critical Verbindung\n  option Zeitüberschreitung\n  end\n",
        )
        .expect("a sequence");
        assert_eq!(
            names(&d),
            [
                ("K", "Kunde", ElementKind::Person),
                ("S", "S", ElementKind::Package),
                ("Q", "Q", ElementKind::Package),
            ]
        );
        let kinds: Vec<(RelationKind, bool)> =
            d.relations.iter().map(|r| (r.kind, r.error)).collect();
        use RelationKind::*;
        assert_eq!(
            kinds,
            [
                (Dependency, false),
                (Reference, false),
                (Dependency, false),
                (Reference, false),
                (Dependency, true),
                (Reference, true),
                (Event, false),
                (Event, false),
                (Dependency, false),
                (Dependency, false),
            ]
        );
        assert_eq!(d.relations[7].label, None);
        assert_eq!(
            d.items[8..10],
            [
                Item::Note {
                    from: 0,
                    to: 1,
                    side: 0,
                    text: "Text".into()
                },
                Item::Note {
                    from: 2,
                    to: 2,
                    side: -1,
                    text: "links".into()
                },
            ]
        );
        let opened: Vec<&str> = d
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Open(l) => Some(l.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            opened,
            [
                "loop [Jede Minute]",
                "opt",
                "",
                "par",
                "critical [Verbindung]"
            ]
        );
    }

    /// A construct this does not draw keeps the block as source text, as
    /// does a block that never closes or closes twice.
    #[test]
    fn an_unknown_sequence_line_falls_back_to_code() {
        let code = |s: &str| diagram(&format!("sequenceDiagram\n{s}")).is_none();
        assert!(code(
            "  box Aqua Gruppe\n  participant A\n  end\n  A->>B: x\n"
        ));
        assert!(code("  create participant C\n  A->>C: x\n"));
        assert!(code("  loop x\n  A->>B: x\n"));
        assert!(code("  A->>B: x\n  end\n"));
        assert!(code("  A<<->>B: x\n"));
        assert!(!code("  A->>B: x\n"));
    }

    #[test]
    fn other_diagram_kinds_and_empty_ones_are_not_read() {
        assert!(diagram("classDiagram\n  A <|-- B\n").is_none());
        assert!(diagram("flowchart LR\n").is_none());
        assert!(diagram("").is_none());
    }
}
