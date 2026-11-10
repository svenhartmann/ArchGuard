//! Manual simulation in the Graph view: proposals from a Graph's
//! selection, the actions of simulation.slint and what a Graph shows of the
//! document's [`Simulation`] (`crate::tree::simulation`).

use archguard_engine::analysis::SimOp;
use slint::SharedString;

use crate::documents::{self, with_store};
use crate::format::Locale;
use crate::model_sync::sync_rows;
use crate::panes_ui;
use crate::state::GraphState;
use crate::tree::Tree;
pub use crate::tree::simulation::Simulation;
use crate::ui::{AppWindow, SimData, SimOpRow};

/// What Graph `g` proposes for `action` (`cut-out`/`cut-in` with the
/// neighbour's row, `move` with the target module), from its selection.
pub fn proposal(g: &GraphState, action: &str, arg: &str) -> Option<SimOp> {
    let name = |n: usize| g.tree.nodes[n].name.clone();
    // A cut from the cycle planner: `from` and `to` by name, no selection.
    if action == "plan" {
        let (from, to) = arg.split_once('\n')?;
        return Some(SimOp::CutEdge {
            from: from.into(),
            to: to.into(),
        });
    }
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

/// Do `action` (simulation.slint) on the active document's simulation;
/// `proposed` is the Graph's [`proposal`] for it. `true` when the
/// confirmed changes changed and the tree must be built again.
pub fn act(ui: &AppWindow, action: &str, proposed: Option<SimOp>) -> bool {
    let Some(analysis) = with_store(|s| s.active()?.analysis.clone()) else {
        return false;
    };
    if action == "export" {
        export(ui);
        return false;
    }
    let full = &analysis.run.graph;
    let changed = documents::with_active_state_or(Simulation::default, |s| match action {
        "toggle" => {
            s.on = !s.on;
            s.pending = None;
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
            s.pending = None;
            false
        }
        "undo" => s.ops.pop().is_some(),
        "clear" => !std::mem::take(&mut s.ops).is_empty(),
        _ => false,
    });
    changed == Some(true)
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
            has_pending: s.pending.is_some(),
            pending: s.pending.as_ref().map(op_row).unwrap_or_default(),
            error: s.error.clone().unwrap_or_default().into(),
            has_delta: d.is_some(),
            cycles_before: d.map_or(0, |d| d.scc_count_before as i32),
            cycles_after: d.map_or(0, |d| d.scc_count_after as i32),
            acd_before: d.map_or(String::new(), |d| locale.dec(d.acd_before, 2)).into(),
            acd_after: d.map_or(String::new(), |d| locale.dec(d.acd_after, 2)).into(),
            targets: sync_rows(&old.targets, targets.into_iter().map(SharedString::from)),
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
        assert_eq!(proposal(&g, "cut-out", "1"), None, "nothing selected");
        assert_eq!(
            proposal(&g, "plan", "core\napp"),
            Some(cut("core", "app")),
            "the planner needs no selection"
        );
        g.select_row(Some(0));
        let names: Vec<&str> = g.view.rows.iter().map(|r| g.tree.nodes[r.node].name.as_str()).collect();
        assert_eq!(names, ["app", "core"]);
        assert_eq!(proposal(&g, "cut-out", "1"), Some(cut("app", "core")));
        assert_eq!(proposal(&g, "cut-in", "1"), Some(cut("core", "app")));
        assert_eq!(proposal(&g, "cut-in", "9"), None, "no such row");
        assert_eq!(
            proposal(&g, "move", "core"),
            Some(SimOp::MoveNode {
                node: "app".into(),
                new_parent: "core".into()
            })
        );
        assert_eq!(proposal(&g, "move", ""), None);
        assert_eq!(proposal(&g, "plan", "core\napp"), Some(cut("core", "app")));
    }

    #[test]
    fn targets_exclude_the_node_its_subtree_and_its_parent() {
        let t = Tree::from_graph(&graph());
        let at = |name: &str| t.nodes.iter().position(|n| n.name == name).expect("node");
        assert_eq!(targets(&t, at("app/a.rs")), ["core"], "not its parent app");
        assert_eq!(targets(&t, at("app")), ["core"], "not itself");
    }

}
