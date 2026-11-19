//! Own Mermaid `sequenceDiagram`: participants, messages in order and the
//! blocks around them (`alt`/`else`, `opt`, `loop`, `break`, `par`/`and`,
//! `critical`/`option`, `rect`), plus notes. `activate`, `deactivate`,
//! titles and the `+`/`-` on arrows are read but not drawn. Any other line
//! that is no message (`box`, `create`, `link`, …) leaves the whole block as
//! source text.

use archguard_engine::core::{
    Diagram, DiagramElement, DiagramKind, DiagramRelation, ElementKind, RelationKind,
};

/// What a sequence shows below its participants, top to bottom.
#[derive(Debug, PartialEq)]
pub enum Item {
    /// The next message of `Diagram::relations`.
    Message,
    /// A block opens, its label as Mermaid shows it: `alt [condition]`.
    Open(String),
    /// `else`, `and`, `option`: a divider in the open block, `[condition]`.
    Divider(String),
    End,
    /// A note beside or over participants `from..=to` (indices); `side`
    /// -1 left of, 0 over, 1 right of.
    Note {
        from: usize,
        to: usize,
        side: i8,
        text: String,
    },
}

const BLOCKS: &[&str] = &["alt", "opt", "loop", "break", "par", "critical", "rect"];
const DIVIDERS: &[&str] = &["else", "and", "option"];
const IGNORED: &[&str] = &["activate", "deactivate", "title", "accTitle", "accDescr"];
/// Longest first: `-->>` is no `-->` followed by `>`.
const ARROWS: &[&str] = &["-->>", "->>", "--x", "--)", "-->", "-x", "-)", "->"];

/// Reads the lines after `sequenceDiagram` into `d`; `None` for a line it
/// does not know or blocks that do not close. `label` cleans shown text.
pub fn read<'a>(
    lines: impl Iterator<Item = &'a str>,
    d: &mut Diagram,
    label: fn(&str) -> String,
) -> Option<Vec<Item>> {
    d.kind = DiagramKind::Sequence;
    let (mut items, mut number) = (Vec::new(), None);
    for line in lines {
        let (word, rest) = line
            .split_once(char::is_whitespace)
            .map_or((line, ""), |(w, r)| (w, r.trim()));
        let item = match word {
            "autonumber" => {
                number = Some(0);
                continue;
            }
            "participant" | "actor" => {
                declare(d, word, rest, label);
                continue;
            }
            w if IGNORED.contains(&w) => continue,
            // A `rect` is a coloured band without a label.
            "rect" => Item::Open(String::new()),
            w if BLOCKS.contains(&w) => Item::Open(bracketed(w, rest)),
            w if DIVIDERS.contains(&w) => Item::Divider(bracketed("", rest)),
            "end" => Item::End,
            "Note" | "note" => note(d, rest, label)?,
            _ => message(d, line, &mut number, label)?,
        };
        items.push(item);
    }
    balanced(&items).then_some(items)
}

/// Every block ends once, dividers only inside one.
fn balanced(items: &[Item]) -> bool {
    let mut open = 0usize;
    for item in items {
        match item {
            Item::Open(_) => open += 1,
            Item::End if open == 0 => return false,
            Item::End => open -= 1,
            Item::Divider(_) if open == 0 => return false,
            _ => {}
        }
    }
    open == 0
}

/// `participant A as Label` or `actor A`.
fn declare(d: &mut Diagram, word: &str, rest: &str, label: fn(&str) -> String) {
    let (id, name) = rest.split_once(" as ").unwrap_or((rest, rest));
    let kind = if word == "actor" {
        ElementKind::Person
    } else {
        ElementKind::Package
    };
    at(d, id.trim(), Some((label(name), kind)));
}

/// `alt [condition]`; `[condition]` without a word, the word alone
/// without a condition.
fn bracketed(word: &str, condition: &str) -> String {
    match (word, condition) {
        (w, "") => w.to_string(),
        ("", c) => format!("[{c}]"),
        (w, c) => format!("{w} [{c}]"),
    }
}

/// The index of participant `id`, added on first use; a declaration names it.
fn at(d: &mut Diagram, id: &str, declared: Option<(String, ElementKind)>) -> usize {
    let i = d
        .elements
        .iter()
        .position(|e| e.id == id)
        .unwrap_or_else(|| {
            d.elements.push(DiagramElement {
                id: id.into(),
                name: id.into(),
                kind: ElementKind::Package,
                technology: None,
                detail: None,
                group: None,
                cycle: false,
            });
            d.elements.len() - 1
        });
    if let Some((name, kind)) = declared {
        (d.elements[i].name, d.elements[i].kind) = (name, kind);
    }
    i
}

/// `A->>B: text`: dashed arrows (`--`) are references, `)` async events,
/// `x` lost messages (drawn as errors, as the generated `-x`).
fn message(
    d: &mut Diagram,
    line: &str,
    number: &mut Option<usize>,
    label: fn(&str) -> String,
) -> Option<Item> {
    let (head, text) = line.split_once(':').unwrap_or((line, ""));
    let (pos, arrow) = head.char_indices().find_map(|(i, _)| {
        ARROWS
            .iter()
            .find(|a| head[i..].starts_with(**a))
            .map(|a| (i, *a))
    })?;
    let from = head[..pos].trim();
    let to = head[pos + arrow.len()..]
        .trim_start_matches(['+', '-'])
        .trim();
    let id =
        |s: &str| !s.is_empty() && !s.contains(|c: char| c.is_whitespace() || "<>".contains(c));
    if !id(from) || !id(to) {
        return None;
    }
    at(d, from, None);
    at(d, to, None);
    let mut text = label(text);
    if let Some(n) = number {
        *n += 1;
        text = format!("{n}. {text}").trim_end().to_string();
    }
    let kind = if arrow.ends_with(')') {
        RelationKind::Event
    } else if arrow.starts_with("--") {
        RelationKind::Reference
    } else {
        RelationKind::Dependency
    };
    d.relations.push(DiagramRelation {
        from: from.into(),
        to: to.into(),
        kind,
        count: 0,
        cycle: false,
        label: (!text.is_empty()).then_some(text),
        error: arrow.ends_with('x'),
        caller: None,
        reply: false,
    });
    Some(Item::Message)
}

/// `left of A: text`, `right of A: text`, `over A,B: text`.
fn note(d: &mut Diagram, rest: &str, label: fn(&str) -> String) -> Option<Item> {
    let (place, text) = rest.split_once(':')?;
    let place = place.trim();
    let (side, who) = if let Some(w) = place.strip_prefix("over ") {
        (0, w)
    } else if let Some(w) = place.strip_prefix("left of ") {
        (-1, w)
    } else {
        (1, place.strip_prefix("right of ")?)
    };
    let ids: Vec<usize> = who.split(',').map(|id| at(d, id.trim(), None)).collect();
    Some(Item::Note {
        from: *ids.iter().min()?,
        to: *ids.iter().max()?,
        side,
        text: label(text),
    })
}
