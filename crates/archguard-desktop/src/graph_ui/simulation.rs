//! Manual simulation in the Graph view: proposals from a Graph's
//! selection, the actions of simulation.slint and what a Graph shows of the
//! document's [`Simulation`] (`crate::tree::simulation`).

use std::path::Path;

use archguard_engine::analysis::{DEFAULT_MAX_EXAMINED, SimOp, plan_cycle_repairs};
use archguard_engine::core::Graph;
use slint::{ModelRc, SharedString, VecModel};

use crate::documents::{self, with_store};
use crate::format::Locale;
use crate::model_sync::sync_rows;
use crate::panes_ui;
use crate::state::GraphState;
use crate::tree::Tree;
use crate::tree::scenario::{Ask, Scenarios, Status};
pub use crate::tree::simulation::Simulation;
use crate::ui::{AppWindow, CutItem, CyclePlans, PlanItem, ScenarioRow, SimData, SimOpRow};

/// What Graph `g` proposes for `action` (`cut-out`/`cut-in` with the
/// neighbour's row, `move` with the target module), from its selection;
/// empty for nothing.
pub fn proposal(g: &GraphState, action: &str, arg: &str) -> Vec<SimOp> {
    // Cuts from the cycle planner: `from\nto` pairs by name, no selection;
    // a repair plan has several.
    if action == "plan" {
        let names: Vec<&str> = arg.lines().collect();
        return names
            .as_chunks::<2>()
            .0
            .iter()
            .map(|[from, to]| SimOp::CutEdge {
                from: (*from).into(),
                to: (*to).into(),
            })
            .collect();
    }
    single(g, action, arg).into_iter().collect()
}

/// The repair plans for the cycle group `members` of `full`, as the
/// explorer shows them; a failure shows as their error.
pub fn cycle_plans(full: &Graph, members: &[String]) -> CyclePlans {
    let r = match plan_cycle_repairs(full, members, DEFAULT_MAX_EXAMINED) {
        Ok(r) => r,
        Err(e) => {
            return CyclePlans {
                error: e.to_string().into(),
                complete: true,
                ..Default::default()
            };
        }
    };
    let int = |n: usize| i32::try_from(n).unwrap_or(i32::MAX);
    let plans = r.plans.iter().map(|p| {
        let pairs = p.ops.iter().filter_map(|op| match op {
            SimOp::CutEdge { from, to } => Some((from.as_str(), to.as_str())),
            _ => None,
        });
        let cuts = pairs
            .clone()
            .zip(&p.op_references)
            .map(|((from, to), &refs)| CutItem {
                from: from.into(),
                to: to.into(),
                refs: refs.min(i32::MAX as u64) as i32,
                ..Default::default()
            });
        let arg: Vec<&str> = pairs.flat_map(|(from, to)| [from, to]).collect();
        PlanItem {
            cuts: ModelRc::new(VecModel::from(cuts.collect::<Vec<_>>())),
            complete: p.complete,
            files: int(p.affected_files.len()),
            refs: p.reference_count.min(i32::MAX as u64) as i32,
            refs_lower_bound: p.reference_lower_bound,
            nodes_after: int(p.after.cyclic_nodes),
            edges_after: int(p.after.cyclic_edges),
            largest_after: int(p.after.largest_scc),
            arg: arg.join("\n").into(),
        }
    });
    CyclePlans {
        plans: ModelRc::new(VecModel::from(plans.collect::<Vec<_>>())),
        nodes_before: int(r.before.cyclic_nodes),
        edges_before: int(r.before.cyclic_edges),
        largest_before: int(r.before.largest_scc),
        complete: r.search.complete,
        examined: int(r.search.examined),
        lower_bound: r.ranking_lower_bound,
        error: SharedString::new(),
    }
}

fn single(g: &GraphState, action: &str, arg: &str) -> Option<SimOp> {
    let name = |n: usize| g.tree.nodes[n].name.clone();
    let selected = name(g.selected?);
    let other = || {
        let row = arg.parse::<usize>().ok()?;
        Some(name(g.view.rows.get(row)?.node))
    };
    match action {
        "cut-out" => Some(SimOp::CutEdge {
            from: selected,
            to: other()?,
        }),
        "cut-in" => Some(SimOp::CutEdge {
            from: other()?,
            to: selected,
        }),
        "move" if !arg.is_empty() => Some(SimOp::MoveNode {
            node: selected,
            new_parent: arg.to_string(),
        }),
        _ => None,
    }
}

/// Do `action` with `arg` (simulation.slint) on the active document's
/// simulation; `proposed` is the Graph's [`proposal`] for it. `true` when
/// the confirmed changes changed and the tree must be built again.
pub fn act(ui: &AppWindow, action: &str, arg: &str, proposed: Vec<SimOp>) -> bool {
    let Some(analysis) = with_store(|s| s.active()?.analysis.clone()) else {
        return false;
    };
    if action == "export" {
        export(ui);
        return false;
    }
    let full = &analysis.run.graph;
    if action.starts_with("scenario") {
        let Some(root) = with_store(|s| s.active().map(|d| d.root.clone())) else {
            return false;
        };
        return scenario(action, arg, &root, full);
    }
    let changed = documents::with_active_state_or(Simulation::default, |s| match action {
        "toggle" => {
            s.on = !s.on;
            s.pending.clear();
            false
        }
        "cut-out" | "cut-in" | "move" => {
            s.pending = proposed;
            false
        }
        // The planner's "Preview" turns proposing on, as the web viewer's.
        "plan" => {
            s.on = true;
            s.pending = proposed;
            false
        }
        "confirm" => s.confirm(full),
        "cancel" => {
            s.pending.clear();
            false
        }
        "undo" => s.ops.pop().is_some(),
        "clear" => !std::mem::take(&mut s.ops).is_empty(),
        _ => false,
    });
    changed == Some(true)
}

/// A Scenarios panel action; `true` when a scenario was loaded.
fn scenario(action: &str, arg: &str, root: &Path, full: &Graph) -> bool {
    let current =
        documents::with_active_state_or(Simulation::default, |s| s.ops.clone()).unwrap_or_default();
    // Outside the panel's state: the dialog runs its own event loop.
    let import = (action == "scenario-import")
        .then(|| {
            rfd::FileDialog::new()
                .add_filter("JSON", &["json"])
                .set_directory(root)
                .pick_file()
        })
        .flatten();
    let index = arg.parse::<usize>().ok();
    let loaded = documents::with_active_state_or(Scenarios::default, |p| {
        match action {
            "scenarios" => {
                p.open = !p.open;
                if p.open {
                    p.refresh(root, full);
                }
            }
            "scenario-save" => p.save(root, arg, full, &current),
            "scenario-load" => {
                if let Some(path) = index.and_then(|i| p.saved.get(i)).map(|s| s.path.clone()) {
                    p.load(&path, full, current.len());
                }
            }
            "scenario-import" => {
                if let Some(path) = &import {
                    p.load(path, full, current.len());
                }
            }
            "scenario-delete" => {
                if let Some(i) = index {
                    p.delete(i);
                }
            }
            "scenario-confirm" => return p.confirm(root, full),
            "scenario-cancel" => p.ask = None,
            _ => {}
        }
        None
    })
    .flatten();
    let Some(ops) = loaded else {
        return false;
    };
    documents::with_active_state_or(Simulation::default, |s| {
        s.ops = ops;
        s.on = true;
        s.pending.clear();
    });
    true
}

/// Save the ops where the user chooses; a failure shows as the error.
fn export(ui: &AppWindow) {
    let Some(json) = documents::with_active_state(|s: &mut Simulation| s.export()) else {
        return;
    };
    let mut dialog = rfd::FileDialog::new().set_file_name("archguard-simops.json");
    if let Some(dir) = with_store(|s| s.active().map(|d| d.root.clone())) {
        dialog = dialog.set_directory(dir);
    }
    let Some(path) = dialog.save_file() else {
        return;
    };
    if let Err(e) = std::fs::write(&path, json) {
        documents::with_active_state(|s: &mut Simulation| {
            s.error = Some(format!("{}: {e}", path.display()));
        });
        panes_ui::sync(ui);
    }
}

/// Modules `node` can move under: not itself, not inside it, not its
/// current parent; by name.
fn targets(tree: &Tree, node: usize) -> Vec<String> {
    let inside = |mut n: usize| loop {
        if n == node {
            return true;
        }
        match tree.nodes[n].parent {
            Some(p) => n = p,
            None => return false,
        }
    };
    let parent = tree.nodes[node].parent;
    let mut out: Vec<String> = (0..tree.nodes.len())
        .filter(|&n| !tree.nodes[n].file && Some(n) != parent && !inside(n))
        .map(|n| tree.nodes[n].name.clone())
        .collect();
    out.sort();
    out
}

fn op_row(op: &SimOp) -> SimOpRow {
    let (kind, a, b) = match op {
        SimOp::CutEdge { from, to } => (0, from, to),
        SimOp::MoveNode { node, new_parent } => (1, node, new_parent),
        SimOp::Merge { survivor, absorbed } => (2, survivor, absorbed),
    };
    SimOpRow {
        kind,
        a: a.into(),
        b: b.into(),
    }
}

/// The active document's simulation as Graph `g` shows it.
pub fn data(g: &GraphState, locale: Locale, old: &SimData) -> SimData {
    documents::with_active_state(|s: &mut Simulation| {
        let targets = match (s.on, g.selected) {
            (true, Some(n)) => targets(&g.tree, n),
            _ => Vec::new(),
        };
        let d = s.delta.as_ref();
        SimData {
            on: s.on,
            ops: sync_rows(&old.ops, s.ops.iter().map(op_row)),
            pending: sync_rows(&old.pending, s.pending.iter().map(op_row)),
            error: s.error.clone().unwrap_or_default().into(),
            has_delta: d.is_some(),
            cycles_before: d.map_or(0, |d| d.scc_count_before as i32),
            cycles_after: d.map_or(0, |d| d.scc_count_after as i32),
            acd_before: d
                .map_or(String::new(), |d| locale.dec(d.acd_before, 2))
                .into(),
            acd_after: d
                .map_or(String::new(), |d| locale.dec(d.acd_after, 2))
                .into(),
            targets: sync_rows(&old.targets, targets.into_iter().map(SharedString::from)),
            ..scenario_data(old)
        }
    })
    .unwrap_or_default()
}

/// The Scenarios panel's part of [`SimData`].
fn scenario_data(old: &SimData) -> SimData {
    documents::with_active_state(|p: &mut Scenarios| {
        let rows = p.saved.iter().map(|s| ScenarioRow {
            name: s.name.as_str().into(),
            ops: s.ops as i32,
            stale: s.stale,
        });
        let (ask_kind, ask_name, ask_count, ask_current, ask_stale) = match &p.ask {
            None => (0, String::new(), 0, 0, false),
            Some(Ask::Load {
                scenario,
                current,
                stale,
            }) => (
                1,
                scenario.name.clone(),
                scenario.ops.len(),
                *current,
                *stale,
            ),
            Some(Ask::Delete(s)) => (2, s.name.clone(), s.ops, 0, false),
            Some(Ask::Replace(s)) => (3, s.name.clone(), s.ops.len(), 0, false),
        };
        let (status_kind, status_text) = match &p.status {
            None => (0, ""),
            Some(Status::Saved(t)) => (1, t.as_str()),
            Some(Status::Loaded(t)) => (2, t.as_str()),
            Some(Status::Deleted(t)) => (3, t.as_str()),
            Some(Status::Failed(t)) => (4, t.as_str()),
        };
        SimData {
            scenarios_open: p.open,
            scenarios: sync_rows(&old.scenarios, rows),
            ask_kind,
            ask_name: ask_name.into(),
            ask_count: ask_count as i32,
            ask_current: ask_current as i32,
            ask_stale,
            status_kind,
            status_text: status_text.into(),
            ..Default::default()
        }
    })
    .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::simulation::tests::{cut, graph};

    /// Cuts name the selection and the neighbour in the arrow's direction;
    /// a move names the selection and the chosen module.
    #[test]
    fn proposals_come_from_the_selection() {
        let mut g = GraphState::new(Tree::from_graph(&graph()));
        assert_eq!(proposal(&g, "cut-out", "1"), [], "nothing selected");
        assert_eq!(
            proposal(&g, "plan", "core\napp"),
            [cut("core", "app")],
            "the planner needs no selection"
        );
        assert_eq!(
            proposal(&g, "plan", "core\napp\napp\ncore"),
            [cut("core", "app"), cut("app", "core")],
            "a repair plan proposes all its cuts"
        );
        g.select_row(Some(0));
        let names: Vec<&str> = g
            .view
            .rows
            .iter()
            .map(|r| g.tree.nodes[r.node].name.as_str())
            .collect();
        assert_eq!(names, ["app", "core"]);
        assert_eq!(proposal(&g, "cut-out", "1"), [cut("app", "core")]);
        assert_eq!(proposal(&g, "cut-in", "1"), [cut("core", "app")]);
        assert_eq!(proposal(&g, "cut-in", "9"), [], "no such row");
        assert_eq!(
            proposal(&g, "move", "core"),
            [SimOp::MoveNode {
                node: "app".into(),
                new_parent: "core".into()
            }]
        );
        assert_eq!(proposal(&g, "move", ""), []);
    }

    /// Each plan's "Preview" argument is what [`proposal`] reads back.
    #[test]
    fn cycle_plans_preview_their_cuts() {
        use slint::Model;
        let g = GraphState::new(Tree::from_graph(&graph()));
        let p = cycle_plans(&graph(), &["core".into(), "app".into()]);
        assert!(p.complete && p.error.is_empty());
        assert_eq!((p.nodes_before, p.edges_before), (2, 2));
        assert_eq!(p.plans.row_count(), 2);
        let first = p.plans.row_data(0).expect("a plan");
        assert!(first.complete);
        assert_eq!((first.files, first.refs, first.nodes_after), (1, 1, 0));
        assert_eq!(first.cuts.row_count(), 1);
        assert_eq!(proposal(&g, "plan", &first.arg), [cut("app", "core")]);
        let unknown = cycle_plans(&graph(), &["nowhere".into()]);
        assert!(unknown.error.contains("nowhere"));
    }

    #[test]
    fn targets_exclude_the_node_its_subtree_and_its_parent() {
        let t = Tree::from_graph(&graph());
        let at = |name: &str| t.nodes.iter().position(|n| n.name == name).expect("node");
        assert_eq!(targets(&t, at("app/a.rs")), ["core"], "not its parent app");
        assert_eq!(targets(&t, at("app")), ["core"], "not itself");
    }
}
