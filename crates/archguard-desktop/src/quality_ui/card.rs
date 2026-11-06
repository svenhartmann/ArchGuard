//! One record of a section as the window shows it: the web viewer's
//! `.engineering-card` with paragraphs, `<details>` and "Locate" buttons.

use std::collections::BTreeSet;

use crate::tab_support::Texts;

/// How an item renders (`QualityItem.kind` in `quality_cards.slint`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text = 0,
    Muted = 1,
    /// Small muted text (`<small class="muted">`).
    Small = 2,
    /// Monospace path (`.path`, `.inspector-full-name`).
    Path = 3,
    /// Preformatted (`<pre>`).
    Pre = 4,
    /// Highlighted verdict (`.detail-verdict`).
    Verdict = 5,
    /// Sub-heading (`<h4>`).
    Heading = 6,
    /// "Locate in graph" button for `node`.
    Locate = 7,
    /// Collapsible `<details>`: `text` is the summary, `lines` the body.
    Details = 8,
    /// Share bar (`.knowledge-bar`), `value` in 0..=1.
    Bar = 9,
    /// The card's metric grid (`.quality-metric-grid`).
    Metrics = 10,
    /// The cohesion card's group buttons.
    Groups = 11,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub kind: Kind,
    pub text: String,
    pub node: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub kind: Kind,
    pub text: String,
    pub node: String,
    pub value: f32,
    pub lines: Vec<Line>,
}

/// A cohesion group button.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub index: usize,
    pub label: String,
    pub files: String,
    pub active: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Card {
    /// Identity for callbacks (package node, class id).
    pub key: String,
    pub title: String,
    /// Full name shown on hover.
    pub full: String,
    /// Heading button (cohesion "Inspect groups" / "Selected").
    pub action: String,
    pub pressed: bool,
    pub items: Vec<Item>,
    pub metrics: Vec<String>,
    pub groups: Vec<Group>,
    pub more_groups: bool,
}

/// Web texts are shortened at 4096 characters (`engineeringText`).
const MAX_TEXT: usize = 4096;

pub fn short(text: &str) -> String {
    match text.char_indices().nth(MAX_TEXT) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}

impl Card {
    /// The card as one tooltip text: paragraphs apart, a `<details>`
    /// summary above its lines; buttons and bars have no text to show.
    pub fn plain_text(&self) -> String {
        let paragraph = |i: &Item| match i.kind {
            Kind::Locate | Kind::Bar | Kind::Metrics | Kind::Groups => None,
            Kind::Details => Some(
                std::iter::once(i.text.as_str())
                    .chain(i.lines.iter().map(|l| l.text.as_str()))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            _ => Some(i.text.clone()),
        };
        let parts: Vec<String> = self.items.iter().filter_map(paragraph).collect();
        parts.join("\n\n")
    }
}

impl Line {
    pub fn new(kind: Kind, text: impl AsRef<str>) -> Self {
        Line {
            kind,
            text: short(text.as_ref()),
            node: String::new(),
        }
    }
}

/// Card builder; `nodes` are the source graph's names ("Locate" needs one).
pub struct Builder<'a> {
    pub lang: &'a Texts,
    pub nodes: &'a BTreeSet<String>,
    pub card: Card,
}

impl<'a> Builder<'a> {
    pub fn new(lang: &'a Texts, nodes: &'a BTreeSet<String>, title: impl Into<String>) -> Self {
        let title = title.into();
        Builder {
            lang,
            nodes,
            card: Card {
                full: title.clone(),
                title,
                ..Card::default()
            },
        }
    }

    pub fn push(&mut self, kind: Kind, text: impl AsRef<str>) -> &mut Self {
        let text = text.as_ref();
        self.card.items.push(Item {
            kind,
            text: short(text),
            node: String::new(),
            value: 0.0,
            lines: Vec::new(),
        });
        if text.chars().count() > MAX_TEXT {
            let note = self
                .lang
                .t("Text shortened; full evidence is included in the export.");
            self.push(Kind::Small, note);
        }
        self
    }

    pub fn details(&mut self, summary: impl Into<String>, lines: Vec<Line>) -> &mut Self {
        self.card.items.push(Item {
            kind: Kind::Details,
            text: summary.into(),
            node: String::new(),
            value: 0.0,
            lines,
        });
        self
    }

    pub fn bar(&mut self, value: f64) -> &mut Self {
        self.push(Kind::Bar, "");
        if let Some(item) = self.card.items.last_mut() {
            item.value = value.clamp(0.0, 1.0) as f32;
        }
        self
    }

    /// `engineeringLocate`: only for nodes of the source graph.
    pub fn locate(&mut self, node: Option<&str>) -> &mut Self {
        if let Some(line) = locate(self.lang, self.nodes, node) {
            self.card.items.push(Item {
                kind: Kind::Locate,
                text: line.text,
                node: line.node,
                value: 0.0,
                lines: Vec::new(),
            });
        }
        self
    }

    pub fn done(self) -> Card {
        self.card
    }
}

/// A "Locate in graph" line, if `node` is in the source graph.
pub fn locate(lang: &Texts, nodes: &BTreeSet<String>, node: Option<&str>) -> Option<Line> {
    let node = node.filter(|n| nodes.contains(*n))?;
    Some(Line {
        kind: Kind::Locate,
        text: lang.t("Locate in graph").into(),
        node: node.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The status line's "?" shows the notes as one tooltip text, as the
    /// web's `bindExplanation` joins `#engineering-note`'s children.
    #[test]
    fn plain_text_joins_paragraphs_and_keeps_details_lines() {
        let lang = Texts::default();
        let nodes = BTreeSet::from(["a".to_string()]);
        let mut b = Builder::new(&lang, &nodes, "");
        b.push(Kind::Muted, "Original analysis.")
            .push(Kind::Text, "Classes × metrics.")
            .details(
                "Coverage",
                vec![Line::new(Kind::Muted, "one"), Line::new(Kind::Text, "two")],
            )
            .push(Kind::Pre, "archguard quality import")
            .locate(Some("a"))
            .bar(0.5);
        assert_eq!(
            b.done().plain_text(),
            "Original analysis.\n\nClasses × metrics.\n\nCoverage\none\ntwo\n\narchguard quality import"
        );
    }
}
