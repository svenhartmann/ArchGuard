//! The Elasticsearch tab's state: the topology's search graph and the class view's selection.

use std::path::PathBuf;

use archguard_engine::{SearchGraph, search_graph};

use super::class::types::{Group, HEAD, Key, ROW, Switch, Switches};
use crate::tab_support::Topology;

pub struct EsState {
    pub(super) topology: Topology,
    pub(super) graph: SearchGraph,
    pub(super) zoom: f32,
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
        zoom: 1.0,
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
        self.field = None;
        self.problem.clear();
        self.fit_tick += 1;
    }

    /// Field `path` of the index `key` is selected (the index is selected with it).
    pub(super) fn set_field(&mut self, key: Key, path: &str) {
        if self.selected.as_ref() != Some(&key) {
            self.select(Some(key.clone()));
        }
        self.field = Some((key, path.to_string()));
    }

    /// Back to the index selection.
    pub(super) fn clear_field(&mut self) {
        self.field = None;
    }

    /// A field of the shown index was clicked in the details.
    pub(super) fn select_field(&mut self, path: &str) {
        if let Some(key) = self.shown.clone() {
            self.set_field(key, path);
        }
    }

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

    /// The canvas box `i` is clicked `y` px below its top (zoom 1): a field row selects that
    /// field of the index, the header of the index with a field in focus returns to the index,
    /// else the box is selected. An index out of range is ignored.
    pub(super) fn pick_box(&mut self, i: usize, y: f32) {
        let Some(key) = self.box_keys.get(i).cloned() else {
            return;
        };
        let hit = (self.box_rows.get(i).into_iter().flatten())
            .find(|(top, _)| (*top..*top + ROW).contains(&y))
            .map(|(_, path)| path.clone());
        match hit {
            Some(path) => self.set_field(key, &path),
            None if self.field.as_ref().is_some_and(|(k, _)| *k == key) && y < HEAD => {
                self.clear_field();
            }
            None => self.select(Some(key)),
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
