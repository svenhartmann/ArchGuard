#![cfg_attr(not(test), allow(dead_code))]
// Unused until ticket 12.
//! List model (groups, search, kind filter), ticket 06.

use archguard_engine::SearchGraph;
use archguard_engine::core::ServiceTopology;

use super::types::{Group, Key};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GroupTitle {
    Services,
    Indices(String),
    IndicesUndeclared,
    Aliases,
    Templates,
    Analysis,
    Processing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListRow {
    pub key: Key,
    pub label: String,
    pub side: String,
    pub writes: usize,
    pub reads: usize,
    pub warn: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListGroup {
    pub title: GroupTitle,
    pub rows: Vec<ListRow>,
}

pub fn list(_t: &ServiceTopology, _g: &SearchGraph, _query: &str, _group: Group) -> Vec<ListGroup> {
    Vec::new()
}

pub fn members(_groups: &[ListGroup]) -> Vec<Key> {
    Vec::new()
}
