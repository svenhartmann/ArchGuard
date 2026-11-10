//! The arc42 diagram catalogue (ADR 0006): one neutral model per diagram
//! that CLI, web and desktop draw or write as Mermaid or PlantUML, and the
//! `[arc42]` section that chooses which diagrams the document shows.

use serde::{Deserialize, Serialize};

/// The language of generated arc42 text (ADR 0008).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DocLang {
    #[default]
    En,
    De,
}

impl DocLang {
    /// The text for this language.
    pub fn pick<T>(self, en: T, de: T) -> T {
        match self {
            DocLang::En => en,
            DocLang::De => de,
        }
    }
}

/// `[arc42]` in `archguard.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Arc42Config {
    /// Text format of the diagrams written into `arc42.md`.
    pub diagram_format: DiagramFormat,
    /// Catalogue ids left out of the document; all others are shown.
    pub hidden_diagrams: Vec<String>,
    /// Package map levels put into the document, as `<map>@<package>`
    /// (`building-blocks@app/core`); levels are left out unless listed.
    pub shown_diagrams: Vec<String>,
    /// The language of the generated text, set by the caller (the desktop:
    /// its window language); never read from or written to the file
    /// (ADR 0008).
    #[serde(skip)]
    pub lang: DocLang,
}

impl Arc42Config {
    pub fn shows(&self, id: &str) -> bool {
        if Self::level_of(id).is_some() {
            self.shown_diagrams.iter().any(|s| s == id)
        } else {
            !self.hidden_diagrams.iter().any(|h| h == id)
        }
    }

    /// The map id and package of a package map level id.
    pub fn level_of(id: &str) -> Option<(&str, &str)> {
        id.split_once('@')
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagramFormat {
    #[default]
    Mermaid,
    Plantuml,
}

/// What a diagram shows, which decides its notation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagramKind {
    /// Packages and their relations (flowchart / package diagram).
    Packages,
    /// C4 deployment: containers inside one deployment configuration.
    Deployment,
    /// C4 container view of how services talk to each other.
    Communication,
    /// C4 system context: users, the system, what it starts and reaches.
    Context,
    /// One recorded request: participants and messages in time order.
    Sequence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElementKind {
    Package,
    /// The remainder box for blocks a diagram leaves out.
    Remainder,
    Container,
    Database,
    /// A service of another deployment configuration, or an external
    /// program or system in the context diagram.
    External,
    /// Who uses the system (context diagram).
    Person,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    /// Source dependency between packages.
    Dependency,
    /// Static call sites between packages.
    Call,
    Startup,
    Http,
    Grpc,
    Event,
    /// A configured reference (URL, host name) without a known protocol.
    Reference,
    /// A service uses a datastore.
    Data,
    /// The system starts a program.
    Runs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagramElement {
    pub id: String,
    pub name: String,
    pub kind: ElementKind,
    /// C4 technology, e.g. "node", "PostgreSQL".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub technology: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Box the element is drawn in: a layer or a deployment node.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Part of a cycle.
    #[serde(default)]
    pub cycle: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagramRelation {
    /// Element ids.
    pub from: String,
    pub to: String,
    pub kind: RelationKind,
    /// Evidence count (source connections, call sites, config entries).
    pub count: usize,
    #[serde(default)]
    pub cycle: bool,
    /// Text on the arrow when the kind alone does not say enough
    /// (sequence messages: order, operation, duration).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// A recorded error on this message.
    #[serde(default)]
    pub error: bool,
    /// Sequences: the message (index into `relations`) this one is sent
    /// within; `None`: sent by the initiator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller: Option<usize>,
    /// Sequences: the explicit answer to `caller`, which then needs no
    /// drawn return of its own.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reply: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagram {
    /// Stable catalogue id, as listed in `hidden_diagrams`.
    pub id: String,
    pub chapter: u8,
    pub title: String,
    pub kind: DiagramKind,
    /// What the diagram is based on and what it does not show.
    pub note: String,
    /// What the counts on relations count.
    pub unit: String,
    pub elements: Vec<DiagramElement>,
    pub relations: Vec<DiagramRelation>,
    /// Shown in the document under the current `[arc42]` choice.
    pub shown: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SequenceStep {
    /// Index into `relations`.
    Call(usize),
    /// The return of that call to its caller.
    Return(usize),
}

/// Messages in order, with a return for each call once the calls made
/// within it (`caller`) are done. Events and explicit replies get none.
pub fn sequence_steps(relations: &[DiagramRelation]) -> Vec<SequenceStep> {
    let (mut out, mut open): (Vec<SequenceStep>, Vec<usize>) = (Vec::new(), Vec::new());
    for (i, r) in relations.iter().enumerate() {
        // The nearest enclosing message still open: a collapsed scenario
        // can send from a call that already returned. Callers only point
        // back, so the walk ends.
        let mut within = r.caller.filter(|&c| c < i);
        while let Some(c) = within
            && !open.contains(&c)
        {
            within = relations[c].caller.filter(|&p| p < c);
        }
        while let Some(&top) = open.last()
            && Some(top) != within
        {
            open.pop();
            if relations[top].kind != RelationKind::Event {
                out.push(SequenceStep::Return(top));
            }
        }
        out.push(SequenceStep::Call(i));
        if !r.reply {
            open.push(i);
        } else if within.is_some() && within == r.caller {
            open.pop();
        }
    }
    let rest = open.into_iter().rev();
    out.extend(
        rest.filter(|&i| relations[i].kind != RelationKind::Event)
            .map(SequenceStep::Return),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arc42_section_parses_format_and_hidden_ids() {
        let c: Arc42Config =
            toml::from_str("diagram_format = \"plantuml\"\nhidden_diagrams = [\"cycles\"]")
                .unwrap();
        assert_eq!(c.diagram_format, DiagramFormat::Plantuml);
        assert!(!c.shows("cycles") && c.shows("building-blocks"));
        assert_eq!(
            Arc42Config::default().diagram_format,
            DiagramFormat::Mermaid
        );
        assert!(toml::from_str::<Arc42Config>("format = \"x\"").is_err());
    }

    #[test]
    fn package_map_levels_are_shown_only_when_listed() {
        let c: Arc42Config =
            toml::from_str("shown_diagrams = [\"building-blocks@app/core\"]").unwrap();
        assert!(c.shows("building-blocks@app/core"));
        assert!(!c.shows("building-blocks@app/web"), "levels are opt-in");
        assert!(
            c.shows("deployment-stacks/a/compose.yaml"),
            "a slash is no level"
        );
        assert_eq!(
            Arc42Config::level_of("cycles@app/core"),
            Some(("cycles", "app/core"))
        );
        assert_eq!(Arc42Config::level_of("cycles"), None);
    }
}

#[cfg(test)]
mod sequence_tests {
    use super::*;
    use SequenceStep::{Call, Return};

    fn rel(from: &str, to: &str, caller: Option<usize>) -> DiagramRelation {
        DiagramRelation {
            from: from.into(),
            to: to.into(),
            kind: RelationKind::Http,
            count: 1,
            cycle: false,
            label: None,
            error: false,
            caller,
            reply: false,
        }
    }

    /// service-shop: catalog asks pricing, pricing asks promotions,
    /// promotions asks catalog again; only then catalog asks inventory.
    #[test]
    fn a_call_returns_to_the_activation_that_made_it() {
        let r = [
            rel("user", "gateway", None),
            rel("gateway", "catalog", Some(0)),
            rel("catalog", "pricing", Some(1)),
            rel("pricing", "promotions", Some(2)),
            rel("promotions", "catalog", Some(3)),
            rel("catalog", "inventory", Some(1)),
        ];
        let want = [
            Call(0),
            Call(1),
            Call(2),
            Call(3),
            Call(4),
            Return(4),
            Return(3),
            Return(2),
            Call(5),
            Return(5),
            Return(1),
            Return(0),
        ];
        assert_eq!(sequence_steps(&r), want);
    }

    #[test]
    fn an_explicit_reply_closes_its_call_without_a_drawn_return() {
        let reply = |from, to, caller| DiagramRelation {
            reply: true,
            error: true,
            ..rel(from, to, Some(caller))
        };
        let r = [
            rel("user", "a", None),
            rel("a", "b", Some(0)),
            reply("b", "a", 1),
            reply("a", "user", 0),
        ];
        assert_eq!(sequence_steps(&r), [Call(0), Call(1), Call(2), Call(3)]);
    }

    #[test]
    fn an_event_never_returns_but_calls_made_on_it_do() {
        let r = [
            DiagramRelation {
                kind: RelationKind::Event,
                ..rel("a", "b", None)
            },
            rel("b", "c", Some(0)),
        ];
        assert_eq!(sequence_steps(&r), [Call(0), Call(1), Return(1)]);
    }
}
