//! Box contents (header, compartments, rows, chips), ticket 05.
// Unused until ticket 12.
#![cfg_attr(not(test), allow(dead_code))]

use archguard_engine::SearchGraph;
use archguard_engine::core::ServiceTopology;

use super::types::{Key, Switches};

/// Compartment titles; Slint translates them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Title {
    Fields,
    Settings,
    Analysis,
    Chain,
    Parameters,
    UsedBy,
    Patterns,
    PointsTo,
    Access,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Prefix {
    #[default]
    None,
    /// `+`, a top-level field.
    Field,
    /// `›`, a multi-field.
    Multi,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    CharFilter,
    Tokenizer,
    Filter,
}

/// Engine class of a chain member (`ChainRef.class`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChainClass {
    Declared,
    Builtin,
    Undeclared,
    Inline,
}

/// `summary` > 0: that many collapsed fields; < 0: that many more rows. A hint row has the
/// identifier `not_read` or `not_declared` as `text`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Line {
    pub depth: usize,
    pub prefix: Prefix,
    pub text: String,
    pub kind_text: String,
    pub note: String,
    pub summary: Option<isize>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chip {
    pub text: String,
    pub stage: Stage,
    pub class: ChainClass,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Compartment {
    pub title: Title,
    pub lines: Vec<Line>,
    pub chips: Vec<Chip>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BoxContent {
    pub stereotype: String,
    pub name: String,
    pub root: String,
    pub ghost: bool,
    pub compartments: Vec<Compartment>,
}

pub fn content(_t: &ServiceTopology, _g: &SearchGraph, _key: &Key, _sw: &Switches) -> BoxContent {
    BoxContent::default()
}
