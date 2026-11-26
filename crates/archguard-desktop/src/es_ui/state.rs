//! The Elasticsearch tab's state: the topology's search graph, the kind filter, the focus.

use std::collections::BTreeSet;
use std::path::PathBuf;

use archguard_engine::{SearchGraph, search_graph};

use super::class::types::{Group, Key, Switch, Switches};
use crate::services_ui::search::KINDS;
use crate::tab_support::Topology;

/// Kinds shown until the user toggles them; ghosts are always shown.
const DEFAULT: [&str; 3] = ["service", "index", "alias"];

pub struct EsState {
    pub(super) topology: Topology,
    pub(super) graph: SearchGraph,
    /// Kinds flipped against [`DEFAULT`].
    pub(super) toggled: BTreeSet<String>,
    /// Node index of the focused node.
    pub(super) focus: Option<usize>,
    pub(super) zoom: f32,
    pub(super) viewport: f32,
    /// The document's root: usages open their file below it.
    pub(super) root: PathBuf,
    /// The editor's error of the last usage click.
    pub(super) problem: String,
    /// Class view: the selected box.
    pub(super) selected: Option<Key>,
    /// Class view: whole cluster instead of the neighborhood.
    pub(super) cluster: bool,
    /// Class view: the selected field, as index key and field path.
    pub(super) field: Option<(Key, String)>,
    pub(super) switches: Switches,
    /// Class view: list search text.
    pub(super) query: String,
    /// Class view: list kind filter.
    pub(super) group: Group,
    /// Class view, set by the last `class::view::build`: the key of every list row (flat) and
    /// canvas box, and the canvas size at zoom 1 (what the callbacks' indices mean).
    pub(super) row_keys: Vec<Key>,
    pub(super) box_keys: Vec<Key>,
    pub(super) canvas: (f32, f32),
    /// Class view, set with `box_keys`: per canvas box the (top, path) of its field lines.
    pub(super) box_rows: Vec<Vec<(f32, String)>>,
    /// Class view, set by the last build: the shown selection (what a details click refers to).
    pub(super) shown: Option<Key>,
    /// Class view: counts up with every change of the selection or scope (the canvas refits).
    pub(super) fit_tick: i32,
}

pub fn build_from(topology: Topology) -> EsState {
    let graph = (*topology).as_ref().map(search_graph).unwrap_or_default();
    EsState {
        topology,
        graph,
        toggled: BTreeSet::new(),
        focus: None,
        zoom: 1.0,
        viewport: 0.0,
        root: PathBuf::new(),
        problem: String::new(),
        selected: None,
        cluster: false,
        field: None,
        switches: Switches::default(),
        query: String::new(),
        group: Group::All,
        row_keys: Vec::new(),
        box_keys: Vec::new(),
        canvas: (0.0, 0.0),
        box_rows: Vec::new(),
        shown: None,
        fit_tick: 0,
    }
}

impl EsState {
    /// The kinds of the graph: services, then the artifact kinds in the Services
    /// tab's order, unknown kinds by first appearance.
    pub(super) fn kinds(&self) -> Vec<String> {
        let present: BTreeSet<&str> = self.graph.nodes.iter().map(|n| n.kind.as_str()).collect();
        let mut out: Vec<&str> = ["service"].into_iter().chain(KINDS).collect();
        out.retain(|k| present.contains(k));
        for n in &self.graph.nodes {
            if !out.contains(&n.kind.as_str()) {
                out.push(&n.kind);
            }
        }
        out.into_iter().map(String::from).collect()
    }

    pub(super) fn kind_shown(&self, kind: &str) -> bool {
        DEFAULT.contains(&kind) != self.toggled.contains(kind)
    }

    #[cfg_attr(not(test), allow(dead_code))] // Removed in ticket 14.
    pub(super) fn toggle_kind(&mut self, i: usize) {
        let Some(kind) = self.kinds().into_iter().nth(i) else {
            return;
        };
        if !self.toggled.remove(&kind) {
            self.toggled.insert(kind);
        }
        self.clear_focus();
    }

    /// A group row (no node) has no focus.
    #[cfg_attr(not(test), allow(dead_code))] // Removed in ticket 14.
    pub(super) fn focus_on(&mut self, node: usize) {
        if node < self.graph.nodes.len() {
            self.focus = Some(node);
            self.problem.clear();
        }
    }

    /// "Show all": no focus.
    #[cfg_attr(not(test), allow(dead_code))] // Removed in ticket 14.
    pub(super) fn clear_focus(&mut self) {
        self.focus = None;
        self.problem.clear();
    }

    pub(super) fn zoom_by(&mut self, factor: f32) {
        self.zoom = if factor == 0.0 {
            1.0
        } else {
            (self.zoom * factor).clamp(0.01, 3.0)
        };
    }
}

impl EsState {
    pub(super) fn select(&mut self, key: Option<Key>) {
        self.selected = key;
        self.problem.clear();
        self.fit_tick += 1;
    }

    /// Field `path` of the index `key` is selected (the index is selected with it).
    pub(super) fn set_field(&mut self, _key: Key, _path: &str) {}

    /// Back to the index selection.
    pub(super) fn clear_field(&mut self) {}

    /// A field of the shown index was clicked in the details.
    pub(super) fn select_field(&mut self, _path: &str) {}

    pub(super) fn toggle_cluster(&mut self) {
        self.cluster = !self.cluster;
        self.fit_tick += 1;
    }

    pub(super) fn set_switch(&mut self, switch: Switch, on: bool) {
        let s = &mut self.switches;
        match switch {
            Switch::Services => s.services = on,
            Switch::Analysis => s.analysis = on,
            Switch::MultiFields => s.multi_fields = on,
            Switch::CollapseFields => s.collapse_fields = on,
        }
        self.fit_tick += 1;
    }

    /// The list row `i` (flat) is selected; an index out of range is ignored.
    pub(super) fn pick_row(&mut self, i: usize) {
        if let Some(key) = self.row_keys.get(i).cloned() {
            self.select(Some(key));
        }
    }

    /// The canvas box `i` is selected; an index out of range is ignored.
    pub(super) fn pick_box(&mut self, i: usize, _y: f32) {
        if let Some(key) = self.box_keys.get(i).cloned() {
            self.select(Some(key));
        }
    }

    /// A usage was clicked: open its file in the editor, an error goes into `problem`.
    pub(super) fn open_usage(&mut self, file: &str, line: u32) {
        self.problem = crate::editor::open(&self.root, file, line)
            .err()
            .unwrap_or_default();
    }

    pub(super) fn set_query(&mut self, query: &str) {
        self.query = query.to_string();
    }

    pub(super) fn set_group(&mut self, group: Group) {
        self.group = group;
    }
}
