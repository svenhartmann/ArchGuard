//! Shared class view types: selection key, kind filter, switches, ticket 03.

/// Row pitch and header height of a canvas box at zoom 1: the layout places rows by them and a
/// click on the canvas is matched against them.
pub const ROW: f32 = 16.0;
pub const HEAD: f32 = 3.0 * ROW;

/// What is selected: a `search_graph` node or a part (analysis building block) of an index/template.
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

impl Group {
    /// The kind combo's entry (`Group` order); an unknown index is "all".
    pub fn from_index(i: usize) -> Self {
        const ALL: [Group; 7] = [
            Group::All,
            Group::Services,
            Group::Indices,
            Group::Aliases,
            Group::Templates,
            Group::Analysis,
            Group::Processing,
        ];
        ALL.get(i).copied().unwrap_or_default()
    }
}

/// The four switches of the bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Switch {
    Services,
    Analysis,
    MultiFields,
    CollapseFields,
}

impl Switch {
    /// The switch of the bar's button `i`.
    pub fn from_index(i: usize) -> Option<Self> {
        [
            Switch::Services,
            Switch::Analysis,
            Switch::MultiFields,
            Switch::CollapseFields,
        ]
        .get(i)
        .copied()
    }
}

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
