//! A Graph instance's [`GraphData`] for its pane.

use std::collections::BTreeSet;

use archguard_engine::core::ReviewPriority;
use slint::ModelRc;

use crate::kpi;
use crate::model_sync::sync_rows;
use crate::network;
use crate::state::{self, GraphState};
use crate::tab_support::Texts;
use crate::tree::Sort;
use crate::ui::*;

fn mode_index(m: crate::tree::EdgeMode) -> i32 {
    use crate::tree::EdgeMode;
    match m {
        EdgeMode::Incoming => 0,
        EdgeMode::Outgoing => 1,
        EdgeMode::Both => 2,
        EdgeMode::All => 3,
        EdgeMode::Cycles => 4,
    }
}

/// Display indices (`GraphState::display`).
const TREE: i32 = 0;
const NETWORK: i32 = 1;
const DEPENDENCIES: i32 = 2;

/// Everything the Graph view of tab `id` shows of `g`, in the models of
/// `old` (what its pane showed before), changed in place. The links,
/// network and dependency list are computed only for their display.
pub fn graph_data(
    ui: &AppWindow,
    g: &GraphState,
    cycles: &[CycleItem],
    id: u32,
    old: &GraphData,
) -> GraphData {
    let v = &g.view;
    let selected = g.selected_row();
    let covered = selected.map(|s| v.selection(s)).unwrap_or_default();
    let texts = Texts::of(ui);
    let (cin, cout, cboth, call) = g.counts();
    let node = g.selected.map(|n| &g.tree.nodes[n]);
    let (incoming, outgoing) = neighbors(g, old);
    let net = network_data(g, selected, old);
    let links = (g.display == TREE).then(|| v.draw(selected, g.mode));
    GraphData {
        id: id as i32,
        rows: tree_rows(g, &covered, &old.rows),
        links: sync_rows(
            &old.links,
            links.into_iter().flatten().map(|d| TreeLink {
                path: d.path.into(),
                kind: d.kind as i32,
                from: d.from as i32,
                to: d.to as i32,
                right: d.right,
            }),
        ),
        selected_row: selected.map_or(-1, |s| s as i32),
        selected_node: g.selected.map_or(-1, |n| n as i32),
        crumbs: crumbs(ui, g, &old.crumbs),
        dep_count: v.links.len() as i32,
        kpis: sync_rows(
            &old.kpis,
            [
                kpi::SHOWN_MODULES.item(&texts, v.scope_rows().len()),
                kpi::SHOWN_DEPENDENCIES.item(&texts, v.links.len()),
                kpi::SHOWN_CYCLE_GROUPS.item(&texts, v.cycle_groups),
                kpi::SHOWN_ACD.item(&texts, kpi::Value::Real(v.acd(&g.tree))),
            ],
        ),
        count_in: cin as i32,
        count_out: cout as i32,
        count_both: cboth as i32,
        count_all: call as i32,
        reference: node.map(|n| n.name.clone()).unwrap_or_default().into(),
        selected_label: node.map(|n| n.label.clone()).unwrap_or_default().into(),
        incoming,
        outgoing,
        deps: deps(g, selected, &old.deps),
        net_cards: net.0,
        net_arrows: net.1,
        net_w: net.2,
        net_h: net.3,
        edge_mode: mode_index(g.mode),
        sort_by_deps: g.sort == Sort::Connections,
        display: g.display,
        explorer_tab: g.explorer_tab,
        cycles: sync_rows(&old.cycles, cycles.iter().cloned()),
        node_total: g.tree.nodes.len() as i32,
    }
}

fn deps(g: &GraphState, selected: Option<usize>, old: &ModelRc<DepCard>) -> ModelRc<DepCard> {
    let v = &g.view;
    let shown = (g.display == DEPENDENCIES).then(|| v.shown(selected, g.mode));
    sync_rows(
        old,
        shown.into_iter().flatten().map(|(i, lane)| {
            let l = &v.links[i];
            let name = |row: usize| g.tree.nodes[v.rows[row].node].name.clone();
            DepCard {
                from: name(l.from).into(),
                to: name(l.to).into(),
                weight: l.weight.min(i32::MAX as u64) as i32,
                kind: v.link_kind(i, lane, selected, g.mode) as i32,
                refs: l
                    .refs
                    .iter()
                    .map(|r| format!("{}:{}  {}", r.file, r.line, r.raw.trim()))
                    .collect::<Vec<_>>()
                    .join("\n")
                    .into(),
                ref_count: l.refs.len() as i32,
            }
        }),
    )
}

/// The tree rows for the window; a compressed chain reads as its path.
fn tree_rows(
    g: &GraphState,
    covered: &BTreeSet<usize>,
    old: &ModelRc<TreeRow>,
) -> ModelRc<TreeRow> {
    let v = &g.view;
    sync_rows(
        old,
        v.rows.iter().enumerate().map(|(i, r)| {
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
        }),
    )
}

/// Up to this many arrows, every arrow shows its reference count.
const NET_LABELS: usize = 24;

type Network = (ModelRc<NetCard>, ModelRc<NetArrow>, f32, f32);

/// Shown links between network cards: (from slot, to slot, weight, kind).
fn network_links(
    g: &GraphState,
    selected: Option<usize>,
    slot: &[usize],
) -> Vec<(usize, usize, u64, i32)> {
    let v = &g.view;
    v.shown(selected, g.mode)
        .into_iter()
        .filter_map(|(i, lane)| {
            let l = &v.links[i];
            let (a, b) = (slot[l.from], slot[l.to]);
            let kind = v.link_kind(i, lane, selected, g.mode) as i32;
            (a != usize::MAX && b != usize::MAX && a != b).then_some((a, b, l.weight, kind))
        })
        .collect()
}

/// The network display shows the rows that are not expanded: exactly the
/// rows that carry links. Empty while another display is shown.
fn network_data(g: &GraphState, selected: Option<usize>, old: &GraphData) -> Network {
    let v = &g.view;
    let shown = g.display == NETWORK;
    let leaves: Vec<usize> = (0..v.rows.len())
        .filter(|&r| shown && !v.rows[r].expanded)
        .collect();
    let mut slot = vec![usize::MAX; v.rows.len()];
    for (i, &r) in leaves.iter().enumerate() {
        slot[r] = i;
    }
    let links = if shown {
        network_links(g, selected, &slot)
    } else {
        Vec::new()
    };
    let ends: Vec<_> = links.iter().map(|&(a, b, w, _)| (a, b, w)).collect();
    // The gaps between the cards widen to the lanes the arrows need.
    let (net, routes) = network::layout(leaves.len(), &ends);
    let cards = sync_rows(
        &old.net_cards,
        leaves.iter().zip(&net.cards).map(|(&r, c)| {
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
        }),
    );
    // Counts on every arrow while few are shown, else on the highlighted ones.
    let few = links.len() <= NET_LABELS;
    let arrows = sync_rows(
        &old.net_arrows,
        routes
            .into_iter()
            .zip(&links)
            .map(|(arrow, &(_, _, weight, kind))| NetArrow {
                path: arrow.path.into(),
                width: arrow.width,
                kind,
                count: weight.min(i32::MAX as u64) as i32,
                labeled: few || kind != 0,
                lx: arrow.label.0,
                ly: arrow.label.1,
            }),
    );
    (cards, arrows, net.width, net.height)
}

fn priority_index(p: Option<ReviewPriority>) -> i32 {
    match p {
        Some(ReviewPriority::High) => 1,
        Some(ReviewPriority::Medium) => 2,
        _ => 0,
    }
}

fn crumbs(ui: &AppWindow, g: &GraphState, old: &ModelRc<Crumb>) -> ModelRc<Crumb> {
    let mut crumbs = vec![Crumb {
        node: -1,
        label: ui.get_project(),
    }];
    crumbs.extend(g.crumbs().into_iter().map(|n| Crumb {
        node: n as i32,
        label: g.tree.nodes[n].label.clone().into(),
    }));
    sync_rows(old, crumbs)
}

fn neighbors(g: &GraphState, old: &GraphData) -> (ModelRc<Neighbor>, ModelRc<Neighbor>) {
    let (inc, out) = g.neighbors();
    let list = |old: &ModelRc<Neighbor>, list: Vec<state::Neighbor>| {
        sync_rows(
            old,
            list.into_iter().map(|n| Neighbor {
                row: n.row as i32,
                label: g.tree.nodes[g.view.rows[n.row].node].label.clone().into(),
                weight: n.weight.min(i32::MAX as u64) as i32,
            }),
        )
    };
    (list(&old.incoming, inc), list(&old.outgoing, out))
}
