//! The content of an index, template or ghost detail page as plain data (no Slint types; the
//! conversion comes with ticket 43). Pure function of the topology; coverage is the engine's
//! `search_artifact_covers`.
// Unused until ticket 43 wires the page into the view.
#![cfg_attr(not(test), allow(dead_code))]

use archguard_engine::SearchNode;
use archguard_engine::core::ServiceTopology;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct IndexPage {
    pub overview: Overview,
    pub mapping: Vec<MappingRow>,
    pub settings: Vec<(String, String)>,
    pub analysis: Vec<AnalysisRow>,
    /// Why mapping and settings are empty; `None` when a detail was read.
    pub note: Option<DetailNote>,
    pub links: Vec<Link>,
    pub usages: Vec<UsageRow>,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Overview {
    /// File and line of the declaration; a ghost has none.
    pub declaration: Option<(String, Option<u32>)>,
    pub scope: String,
    pub address: String,
    pub origins: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct MappingRow {
    pub depth: usize,
    /// Last path segment.
    pub name: String,
    pub kind: String,
    pub analyzer: Option<String>,
    pub search_analyzer: Option<String>,
    pub normalizer: Option<String>,
    /// `object` or `nested`: its children follow.
    pub parent: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct AnalysisRow {
    pub kind: String,
    pub name: String,
    pub definition: String,
}

/// Translation key of the hint shown instead of mapping and settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailNote {
    NotRead,
    NotDeclared,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Link {
    pub kind: String,
    pub name: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct UsageRow {
    pub service: String,
    pub access: String,
    pub file: String,
    pub line: Option<u32>,
    /// False for a usage of another project of a set (`<project>:` prefix).
    pub clickable: bool,
}

pub fn index_page(_t: &ServiceTopology, _node: &SearchNode) -> IndexPage {
    IndexPage::default()
}
