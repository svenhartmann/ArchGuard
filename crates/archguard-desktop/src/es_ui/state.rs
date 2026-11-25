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
    // Unused until ticket 12.
    #[allow(dead_code)]
    pub(super) selected: Option<Key>,
    /// Class view: whole cluster instead of the neighborhood.
    #[allow(dead_code)]
    pub(super) cluster: bool,
    #[allow(dead_code)]
    pub(super) switches: Switches,
    /// Class view: list search text.
    #[allow(dead_code)]
    pub(super) query: String,
    /// Class view: list kind filter.
    #[allow(dead_code)]
    pub(super) group: Group,
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
        switches: Switches::default(),
        query: String::new(),
        group: Group::All,
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
    pub(super) fn focus_on(&mut self, node: usize) {
        if node < self.graph.nodes.len() {
            self.focus = Some(node);
            self.problem.clear();
        }
    }

    /// "Show all": no focus.
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

// Unused until ticket 12.
#[allow(dead_code)]
impl EsState {
    pub(super) fn select(&mut self, _key: Option<Key>) {}

    pub(super) fn toggle_cluster(&mut self) {}

    pub(super) fn set_switch(&mut self, _switch: Switch, _on: bool) {}

    pub(super) fn set_query(&mut self, _query: &str) {}

    pub(super) fn set_group(&mut self, _group: Group) {}
}
