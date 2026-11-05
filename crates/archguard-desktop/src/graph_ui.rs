//! Graph tab: tree, network and dependency displays of one
//! [`GraphState`] per document (in [`documents`]).

// The helpers all tabs share. They belong to the crate root, but `main.rs`
// is the window shell's; declared in `context.rs` they would close the
// cycle context -> tab_support -> documents -> context.
#[path = "tab_support.rs"]
pub mod tab_support;

use std::cell::RefCell;
use std::collections::BTreeSet;

use archguard_engine::core::ReviewPriority;
use slint::{ModelRc, SharedString};

use crate::context::Loaded;
use crate::documents;
use crate::network;
use crate::state::{self, GraphState};
use crate::tree::{EdgeMode, Sort};
use crate::ui::*;
use tab_support::{Actions, TabState, index};

impl TabState for GraphState {
    fn push(&mut self, ui: &AppWindow) {
        push_graph(ui, self);
    }
}

fn node_of_row(g: &GraphState, r: i32) -> Option<usize> {
    index(r).and_then(|r| g.view.rows.get(r)).map(|r| r.node)
}

/// Register the Graph tab's callbacks once at start-up.
pub fn wire(ui: &AppWindow) {
    let a = Actions::<GraphState>::new(ui);
    ui.on_select_row(a.on(|g, _, r: i32| g.select_row(index(r))));
    ui.on_toggle_row(a.on(|g, _, r: i32| g.toggle_row(index(r).unwrap_or(0))));
    ui.on_expand_all(a.on0(|g, _| g.expand_all()));
    ui.on_collapse_all(a.on0(|g, _| g.collapse_all()));
    ui.on_reset_tree(a.on0(|g, _| g.reset()));
    ui.on_reveal(a.on(|g, _, n: i32| g.reveal(index(n).unwrap_or(0))));
    ui.on_open_node(a.on(|g, _, n: i32| g.open(index(n).unwrap_or(0))));
    ui.on_open_row(a.on(|g, _, r: i32| {
        if let Some(node) = node_of_row(g, r) {
            g.open(node);
        }
    }));
    ui.on_set_edge_mode(a.on(|g, _, m: i32| g.mode = edge_mode(m)));
    ui.on_set_sort(a.on(|g, _, by_deps: bool| {
        g.set_sort(if by_deps {
            Sort::Connections
        } else {
            Sort::Alphabetical
        });
    }));
    wire_search(ui);
}

thread_local! {
    /// The header search field's text. The field belongs to the window, not
    /// to a document; a switch searches the new document for it.
    static SEARCH: RefCell<SharedString> = RefCell::default();
}

fn wire_search(ui: &AppWindow) {
    let w = ui.as_weak();
    ui.on_search(move |q| {
        let Some(ui) = w.upgrade() else { return };
        SEARCH.set(q);
        push_hits(&ui);
    });
}

/// Hits of the header search in the active document.
fn push_hits(ui: &AppWindow) {
    let q = SEARCH.with_borrow(Clone::clone);
    let hits = documents::with_active_state(|g: &mut GraphState| {
        g.search(&q, 300)
            .into_iter()
            .map(|h| Hit {
                node: h.node as i32,
                label: h.label.into(),
                scope: h.scope.into(),
            })
            .collect::<Vec<_>>()
    });
    ui.set_searching(!q.trim().is_empty());
    ui.set_hits(vec_model(hits.unwrap_or_default()));
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

fn mode_index(m: EdgeMode) -> i32 {
    match m {
        EdgeMode::Incoming => 0,
        EdgeMode::Outgoing => 1,
        EdgeMode::Both => 2,
        EdgeMode::All => 3,
        EdgeMode::Cycles => 4,
    }
}

/// For `documents_ui`, which imports the list model from here.
pub use tab_support::model as vec_model;

/// Show the active document's Graph tab, building its state on first use.
pub fn show(ui: &AppWindow, data: &Loaded) {
    let run = data.run;
    let build = || {
        GraphState::with_review(
            tab_support::tree(data),
            data.sources.clone(),
            run.config.review.entry_points.clone(),
        )
    };
    documents::with_active_state_or(build, |g: &mut GraphState| {
        ui.set_node_total(g.tree.nodes.len() as i32);
        ui.set_cycles(vec_model(g.cycles(&run.report.findings).into_iter().map(
            |c| CycleItem {
                first_node: c.first_node as i32,
                label: c.label.into(),
                size: c.size as i32,
            },
        )));
        push_graph(ui, g);
    });
    push_hits(ui);
}

/// The tree node named `name` in the active document (snapshot hooks).
#[cfg(feature = "snapshot")]
pub fn node_named(name: &str) -> Option<usize> {
    documents::with_active_state(|g: &mut GraphState| {
        g.tree.nodes.iter().position(|n| n.name == name)
    })
    .flatten()
}

fn push_graph(ui: &AppWindow, g: &GraphState) {
    let v = &g.view;
    let selected = g.selected_row();
    let covered = selected.map(|s| v.selection(s)).unwrap_or_default();
    ui.set_rows(tree_rows(g, &covered));
    ui.set_links(vec_model(v.draw(selected, g.mode).into_iter().map(|d| {
        TreeLink {
            path: d.path.into(),
            kind: d.kind as i32,
            from: d.from as i32,
            to: d.to as i32,
            right: d.right,
        }
    })));
    ui.set_deps(vec_model(v.shown(selected, g.mode).into_iter().map(
        |(i, lane)| {
            let l = &v.links[i];
            let kind = v.link_kind(i, lane, selected, g.mode) as i32;
            let name = |row: usize| g.tree.nodes[v.rows[row].node].name.clone();
            DepCard {
                from: name(l.from).into(),
                to: name(l.to).into(),
                weight: l.weight.min(i32::MAX as u64) as i32,
                kind,
                refs: l
                    .refs
                    .iter()
                    .map(|r| format!("{}:{}  {}", r.file, r.line, r.raw.trim()))
                    .collect::<Vec<_>>()
                    .join("\n")
                    .into(),
                ref_count: l.refs.len() as i32,
            }
        },
    )));
    push_network(ui, g, selected);
    ui.set_selected_row(selected.map_or(-1, |s| s as i32));
    ui.set_edge_mode(mode_index(g.mode));
    ui.set_sort_by_deps(g.sort == Sort::Connections);
    ui.set_dep_count(v.links.len() as i32);
    ui.set_cycle_groups(v.cycle_groups as i32);
    ui.set_acd(format!("{:.2}", v.acd(&g.tree)).into());
    let (cin, cout, cboth, call) = g.counts();
    ui.set_count_in(cin as i32);
    ui.set_count_out(cout as i32);
    ui.set_count_both(cboth as i32);
    ui.set_count_all(call as i32);
    ui.set_scope_nodes(v.scope_rows().len() as i32);
    push_selection(ui, g);
}

/// The tree rows for the window; a compressed chain reads as its path.
fn tree_rows(g: &GraphState, covered: &BTreeSet<usize>) -> ModelRc<TreeRow> {
    let v = &g.view;
    vec_model(v.rows.iter().enumerate().map(|(i, r)| {
        let n = &g.tree.nodes[r.node];
        TreeRow {
            row: i as i32,
            node: r.node as i32,
            label: r
                .chain
                .iter()
                .map(|&c| g.tree.nodes[c].label.as_str())
                .collect::<Vec<_>>()
                .join("/")
                .into(),
            depth: r.depth as i32,
            expandable: r.expandable,
            expanded: r.expanded,
            file: n.file,
            in_count: v.incoming[i] as i32,
            out_count: v.outgoing[i] as i32,
            in_cycle: v.in_cycle[i],
            covered: covered.contains(&i),
            priority: priority_index(g.priorities[i]),
        }
    }))
}

/// The network display shows the rows that are not expanded: exactly the
/// rows that carry links.
fn push_network(ui: &AppWindow, g: &GraphState, selected: Option<usize>) {
    let v = &g.view;
    let leaves: Vec<usize> = (0..v.rows.len()).filter(|&r| !v.rows[r].expanded).collect();
    let mut slot = vec![usize::MAX; v.rows.len()];
    for (i, &r) in leaves.iter().enumerate() {
        slot[r] = i;
    }
    let net = network::place(leaves.len());
    ui.set_net_cards(vec_model(leaves.iter().zip(&net.cards).map(|(&r, c)| {
        let node = v.rows[r].node;
        let (files, packages) = g.tree.counts(node);
        NetCard {
            row: r as i32,
            label: g.tree.nodes[node].label.clone().into(),
            file: g.tree.nodes[node].file,
            files: files as i32,
            packages: packages as i32,
            priority: priority_index(g.priorities[r]),
            in_cycle: v.in_cycle[r],
            x: c.x,
            y: c.y,
        }
    })));
    ui.set_net_arrows(vec_model(v.shown(selected, g.mode).into_iter().filter_map(
        |(i, lane)| {
            let l = &v.links[i];
            let (a, b) = (slot[l.from], slot[l.to]);
            (a != usize::MAX && b != usize::MAX).then(|| {
                let arrow = network::arrow(&net.cards[a], &net.cards[b], l.weight);
                NetArrow {
                    path: arrow.path.into(),
                    width: arrow.width,
                    kind: v.link_kind(i, lane, selected, g.mode) as i32,
                }
            })
        },
    )));
    ui.set_net_w(net.width);
    ui.set_net_h(net.height);
}

fn priority_index(p: Option<ReviewPriority>) -> i32 {
    match p {
        Some(ReviewPriority::High) => 1,
        Some(ReviewPriority::Medium) => 2,
        _ => 0,
    }
}

fn push_selection(ui: &AppWindow, g: &GraphState) {
    let mut crumbs = vec![Crumb {
        node: -1,
        label: ui.get_project(),
    }];
    crumbs.extend(g.crumbs().into_iter().map(|n| Crumb {
        node: n as i32,
        label: g.tree.nodes[n].label.clone().into(),
    }));
    ui.set_crumbs(vec_model(crumbs));

    let node = g.selected.map(|n| &g.tree.nodes[n]);
    ui.set_reference(node.map(|n| n.name.clone()).unwrap_or_default().into());
    ui.set_selected_label(node.map(|n| n.label.clone()).unwrap_or_default().into());
    let (inc, out) = g.neighbors();
    let neighbors = |list: Vec<state::Neighbor>| {
        vec_model(list.into_iter().map(|n| Neighbor {
            row: n.row as i32,
            label: g.tree.nodes[g.view.rows[n.row].node].label.clone().into(),
            weight: n.weight.min(i32::MAX as u64) as i32,
        }))
    };
    ui.set_incoming(neighbors(inc));
    ui.set_outgoing(neighbors(out));
}
