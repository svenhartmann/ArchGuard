#![cfg_attr(not(test), allow(dead_code))]
// Unused until ticket 12.
//! Detail model (index, building block, other), ticket 07. Reuses `index_page` for the overview,
//! links, usage and the "not read" note; the analysis parts come from the engine.

use archguard_engine::core::ServiceTopology;
use archguard_engine::{SearchFieldRole, SearchGraph};

use super::types::Key;
use crate::es_ui::index_page::IndexPage;
use crate::ui::SvcSearchDetail;

#[derive(Debug, PartialEq)]
pub enum Details {
    Index(IndexDetails),
    Part(PartDetails),
    Other(SvcSearchDetail),
}

#[derive(Debug, PartialEq)]
pub struct IndexDetails {
    pub page: IndexPage,
    pub fields: Vec<FieldRow>,
    pub chains: Vec<ChainTable>,
    pub hints: Vec<Hint>,
    pub writes: Vec<String>,
    pub reads: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct FieldRow {
    pub path: String,
    pub role: SearchFieldRole,
    pub kind: String,
    pub analyzer: Option<String>,
    pub search_analyzer: Option<String>,
    pub normalizer: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ChainTable {
    pub analyzer: String,
    /// `index`, `search`, `normalizer`: how fields use it.
    pub roles: Vec<String>,
    pub stages: Vec<StageRow>,
}

/// `stage` is `char_filter`, `tokenizer` or `filter`; `n` counts filters from 1, else 0; `class` is
/// `declared`, `builtin`, `undeclared` or `inline`.
#[derive(Debug, PartialEq, Eq)]
pub struct StageRow {
    pub stage: &'static str,
    pub n: usize,
    pub name: String,
    pub class: &'static str,
    pub params: Vec<(String, String)>,
}

/// Neutral note: a field's index and search analyzer differ.
#[derive(Debug, PartialEq, Eq)]
pub struct Hint {
    pub field: String,
    pub index: String,
    pub search: String,
    pub tokenizer_differs: bool,
    pub only_index: Vec<String>,
    pub only_search: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct PartDetails {
    pub kind: String,
    pub kind_type: Option<String>,
    pub name: String,
    pub owner: String,
    pub declaration: Option<(String, Option<u32>)>,
    pub stages: Vec<StageRow>,
    pub params: Vec<(String, String)>,
    pub used_by: Vec<String>,
    pub definition: Vec<(String, String)>,
}

pub fn details(_t: &ServiceTopology, _g: &SearchGraph, _key: &Key) -> Details {
    Details::Other(SvcSearchDetail::default())
}
