#![cfg_attr(not(test), allow(dead_code))]
// Unused until ticket 12.
//! Diagram graph (boxes, edges, neighborhood, cluster), ticket 04.
use archguard_engine::{SearchGraph, core::ServiceTopology};

use super::types::{Key, Switches};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BoxKind {
    Service,
    Index,
    Template,
    Alias,
    /// Every column-0 kind except services (pipeline, ILM, transform, watch, ...).
    Processing,
    Analyzer,
    Normalizer,
    Tokenizer,
    Filter,
    CharFilter,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DBox {
    pub key: Key,
    pub kind: BoxKind,
    pub ghost: bool,
    /// `SearchNode.warn`: engine findings only.
    pub warn: bool,
    pub selected: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EdgeStyle {
    Write,
    Read,
    Unknown,
    Uses,
    Alias,
    Covers,
    Pipeline,
    Lifecycle,
    Reference,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DEdge {
    pub from: Key,
    pub to: Key,
    pub style: EdgeStyle,
    pub label: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Diagram {
    pub boxes: Vec<DBox>,
    pub edges: Vec<DEdge>,
}

pub fn diagram(
    _t: &ServiceTopology,
    _g: &SearchGraph,
    _selected: &Key,
    _cluster: Option<&[Key]>,
    _sw: &Switches,
) -> Diagram {
    Diagram::default()
}
