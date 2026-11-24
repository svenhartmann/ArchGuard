//! The Elasticsearch tab's state: the topology's search graph, the kind filter, the focus.

use std::collections::BTreeSet;

use archguard_engine::{SearchGraph, search_graph};

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
}

pub fn build_from(topology: Topology) -> EsState {
    let graph = (*topology)
        .as_ref()
        .map(search_graph)
        .unwrap_or_default();
    EsState {
        topology,
        graph,
        toggled: BTreeSet::new(),
        focus: None,
        zoom: 1.0,
        viewport: 0.0,
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
        self.focus = None;
    }

    /// A group row (no node) has no focus.
    pub(super) fn focus_on(&mut self, node: usize) {
        if node < self.graph.nodes.len() {
            self.focus = Some(node);
        }
    }

    pub(super) fn zoom_by(&mut self, factor: f32) {
        self.zoom = if factor == 0.0 {
            1.0
        } else {
            (self.zoom * factor).clamp(0.01, 3.0)
        };
    }
}
