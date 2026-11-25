//! Shared class view types: selection key, kind filter, switches, ticket 03.

/// What is selected: a `search_graph` node or a part (analysis building block) of an index/template.
// Unused until ticket 12.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Key {
    Node(usize),
    Part {
        owner: usize,
        kind: String,
        name: String,
    },
}

/// The list's kind filter.
// Unused until ticket 12.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Group {
    #[default]
    All,
    Services,
    Indices,
    Aliases,
    Templates,
    Analysis,
    Processing,
}

/// The four switches of the bar.
// Unused until ticket 12.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Switch {
    Services,
    Analysis,
    MultiFields,
    CollapseFields,
}

// Unused until ticket 12.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Switches {
    pub services: bool,
    pub analysis: bool,
    pub multi_fields: bool,
    pub collapse_fields: bool,
}

impl Default for Switches {
    fn default() -> Self {
        Self {
            services: true,
            analysis: true,
            multi_fields: true,
            collapse_fields: false,
        }
    }
}
