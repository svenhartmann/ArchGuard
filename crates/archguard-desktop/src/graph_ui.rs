//! Graph view: tree, network and dependency displays. A document can show
//! several Graphs; each tab has its own [`Graph`] (`instances`), all
//! share the document's package tree.

mod data;

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, SharedString, VecModel};

use crate::documents::{self, with_store};
use crate::instances::{self, Instance};
use crate::panes::Layout;
use crate::panes_ui;
use crate::state::{self, GraphState};
use crate::tab_support::{self, index, with_ui};
use crate::tree::{EdgeMode, Sort, Tree};
use crate::ui::*;

/// One Graph instance: its state and the document's cycle findings on
/// its tree.
pub struct Graph {
    pub g: GraphState,
    cycles: Vec<CycleItem>,
}

thread_local! {
    /// Per pane: the data of the Graph it shows.
    static PANES: Rc<VecModel<GraphData>> = Rc::new(VecModel::default());
    /// The header search field's text. The field belongs to the window, not
    /// to a document; a switch searches the new document for it.
    static SEARCH: RefCell<SharedString> = RefCell::default();
}

impl Instance for Graph {
    fn show(&mut self, ui: &AppWindow, pane: usize, id: u32) {
        let row = data::graph_data(ui, &self.g, &self.cycles, id);
        PANES.with(|m| instances::set_row(m, pane, row));
    }
}

fn node_of_row(g: &GraphState, r: i32) -> Option<usize> {
    index(r).and_then(|r| g.view.rows.get(r)).map(|r| r.node)
}

/// Change Graph `id` with `f` and show it; its tab follows a new scope or
/// display.
fn act(ui: &AppWindow, id: i32, f: impl FnOnce(&mut GraphState)) {
    let Some(id) = index(id).and_then(|i| u32::try_from(i).ok()) else {
        return;
    };
    let layout = panes_ui::layout();
    let changed = instances::update(ui, &layout, id, |s: &mut Graph| {
        let before = (s.g.scope, s.g.display);
        f(&mut s.g);
        let name = |n: usize| s.g.tree.nodes[n].name.clone();
        ((s.g.scope, s.g.display) != before).then(|| (s.g.scope.map(name), s.g.display))
    })
    .flatten();
    if let Some((node, display)) = changed {
        panes_ui::describe(ui, id, node, display);
    }
}

/// A Graph callback with the instance id and one argument.
fn on<A: 'static>(
    ui: &AppWindow,
    f: impl Fn(&mut GraphState, A) + 'static,
) -> impl Fn(i32, A) + 'static {
    let w = ui.as_weak();
    move |id, a| {
        if let Some(ui) = w.upgrade() {
            act(&ui, id, |g| f(g, a));
        }
    }
}

/// Register the Graph view's callbacks once at start-up.
pub fn wire(ui: &AppWindow) {
    ui.set_graph_panes(PANES.with(|m| m.clone()).into());
    panes_ui::on_reveal("graph", |ui, id, node| {
        act(ui, id as i32, |g| g.reveal(node));
    });
    panes_ui::on_sync("graph", sync);
    let select = on(ui, |g, r: i32| g.select_row(index(r)));
    ui.on_select_row(link(ui, select));
    let reveal = on(ui, |g, n: i32| g.reveal(index(n).unwrap_or(0)));
    ui.on_reveal(link(ui, reveal));
    ui.on_toggle_row(on(ui, |g, r: i32| g.toggle_row(index(r).unwrap_or(0))));
    let f = on(ui, |g, ()| g.expand_all());
    ui.on_expand_all(move |id| f(id, ()));
    let f = on(ui, |g, ()| g.collapse_all());
    ui.on_collapse_all(move |id| f(id, ()));
    let f = on(ui, |g, ()| g.reset());
    ui.on_reset_tree(move |id| f(id, ()));
    ui.on_open_node(on(ui, |g, n: i32| g.open(index(n).unwrap_or(0))));
    ui.on_open_row(on(ui, |g, r: i32| {
        if let Some(node) = node_of_row(g, r) {
            g.open(node);
        }
    }));
    ui.on_set_edge_mode(on(ui, |g, m: i32| g.mode = edge_mode(m)));
    ui.on_set_sort(on(ui, |g, by_deps: bool| {
        g.set_sort(if by_deps {
            Sort::Connections
        } else {
            Sort::Alphabetical
        });
    }));
    ui.on_set_display(on(ui, |g, d: i32| g.display = d));
    ui.on_set_explorer_tab(on(ui, |g, t: i32| g.explorer_tab = t));
    wire_search(ui);
}

/// A selection callback: after `f`, the Graph's selected node is revealed
/// in the pane beside it.
fn link(ui: &AppWindow, f: impl Fn(i32, i32) + 'static) -> impl Fn(i32, i32) + 'static {
    let w = ui.as_weak();
    move |id, arg| {
        f(id, arg);
        let Some(ui) = w.upgrade() else { return };
        let Ok(tab) = u32::try_from(id) else { return };
        if let Some(node) = instances::with(tab, |s: &mut Graph| s.g.selected).flatten() {
            panes_ui::link_from(&ui, tab, node);
        }
    }
}

fn wire_search(ui: &AppWindow) {
    ui.on_search(with_ui(ui, |ui, q: SharedString| {
        SEARCH.set(q);
        // Every Graph shows the hits in its explorer's first tab.
        instances::each(|s: &mut Graph| s.g.explorer_tab = 0);
        panes_ui::sync(ui);
        push_hits(ui);
    }));
    ui.on_reveal_hit(with_ui(ui, |ui, n: i32| {
        if let Some(n) = index(n) {
            panes_ui::reveal(ui, n);
        }
    }));
}

/// Hits of the header search in the active document.
fn push_hits(ui: &AppWindow) {
    let q = SEARCH.with_borrow(Clone::clone);
    let hits = documents::with_active_state(|t: &mut Rc<Tree>| {
        state::search(t, &q, 300)
            .into_iter()
            .map(|h| Hit {
                node: h.node as i32,
                label: h.label.into(),
                scope: h.scope.into(),
            })
            .collect::<Vec<_>>()
    });
    ui.set_searching(!q.trim().is_empty());
    ui.set_hits(tab_support::model(hits.unwrap_or_default()));
}

fn edge_mode(m: i32) -> EdgeMode {
    match m {
        0 => EdgeMode::Incoming,
        1 => EdgeMode::Outgoing,
        2 => EdgeMode::Both,
        4 => EdgeMode::Cycles,
        _ => EdgeMode::All,
    }
}

/// Build, drop and show the active document's Graphs for `layout`.
fn sync(ui: &AppWindow, layout: &Layout) -> Vec<u32> {
    let Some(analysis) = with_store(|s| s.active()?.analysis.clone()) else {
        return Vec::new();
    };
    let data = analysis.loaded();
    let tree = tab_support::tree(&data);
    let run = data.run;
    instances::sync(ui, layout, "graph", &tree, |tab, node| {
        let mut g = GraphState::with_review(
            tree.clone(),
            data.sources.clone(),
            run.config.review.entry_points.clone(),
        );
        g.display = tab.display;
        if let Some(n) = node {
            g.open(n);
        }
        let cycles = g.cycles(&run.report.findings);
        let cycles = cycles.into_iter().map(|c| CycleItem {
            first_node: c.first_node as i32,
            label: c.label.into(),
            size: c.size as i32,
        });
        Some(Graph {
            cycles: cycles.collect(),
            g,
        })
    })
}

/// After the active document's analysis is there or it is shown again:
/// the header search's hits (the Graphs follow `panes_ui::sync`).
pub fn show(ui: &AppWindow) {
    push_hits(ui);
}

/// The tree node named `name` in the active document (snapshot hooks).
#[cfg(feature = "snapshot")]
pub fn node_named(name: &str) -> Option<usize> {
    documents::with_active_state(|t: &mut Rc<Tree>| instances::node_named(t, name)).flatten()
}
