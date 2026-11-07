//! The arc42 diagram catalogue (ADR 0006): one neutral model per diagram
//! that CLI, web and desktop draw or write as Mermaid or PlantUML, and the
//! `[arc42]` section that chooses which diagrams the document shows.

use serde::{Deserialize, Serialize};

/// `[arc42]` in `archguard.toml`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Arc42Config {
    /// Text format of the diagrams written into `arc42.md`.
    pub diagram_format: DiagramFormat,
    /// Catalogue ids left out of the document; all others are shown.
    pub hidden_diagrams: Vec<String>,
}

impl Arc42Config {
    pub fn shows(&self, id: &str) -> bool {
        !self.hidden_diagrams.iter().any(|h| h == id)
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElementKind {
    Package,
    /// The remainder box for blocks a diagram leaves out.
    Remainder,
    Container,
    Database,
    /// A service of another deployment configuration.
    External,
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
}
