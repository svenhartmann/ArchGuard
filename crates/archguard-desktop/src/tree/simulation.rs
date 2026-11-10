//! Manual simulation, as the web viewer's "Refactor → Manual simulation"
//! and its ops strip: confirmed [`SimOp`]s change only the package tree
//! that the document's Graphs and Matrices show. Measurements, findings
//! and source files stay those of the analysis.
//!
//! One per document and not derived from the analysis: the ops address
//! nodes by name, so a newer analysis gets them applied again. Ops that no
//! longer apply leave the original graph shown and say why.

use archguard_engine::analysis::{SimDelta, SimOp, apply_ops, find_sccs, simulate};
use archguard_engine::core::{Config, Cut, Graph, NodeId, aggregate};

#[derive(Default)]
pub struct Simulation {
    /// Proposing is on (web: the "Manual simulation" toggle). Turning it
    /// off keeps the ops.
    pub on: bool,
    pub ops: Vec<SimOp>,
    /// Proposed, waiting for confirmation.
    pub pending: Option<SimOp>,
    /// Why the ops do not apply to the current analysis.
    pub error: Option<String>,
    /// Before/after of the confirmed ops, from the last [`Self::apply`].
    pub delta: Option<SimDelta>,
    /// Cycle groups of the simulated graph by member name, from the last
    /// [`Self::apply`]; `None` without ops.
    pub cycles: Option<Vec<Vec<String>>>,
}

impl Simulation {
    /// The graph the views show: `full` with the ops applied. Also
    /// refreshes [`Self::error`], [`Self::delta`] and [`Self::cycles`].
    /// Ops that fail leave `full` shown.
    pub fn apply(&mut self, full: &Graph, config: &Config) -> Option<Graph> {
        self.error = None;
        self.delta = None;
        self.cycles = None;
        if self.ops.is_empty() {
            return None;
        }
        let after = match apply_ops(full, &self.ops) {
            Ok(g) => g,
            Err(e) => {
                self.error = Some(e.to_string());
                return None;
            }
        };
        // shortcut: compares at `modules.depth` also for `--level file`
        // runs; take the run's level once AnalysisRun records it.
        let depth = config.modules.depth as usize;
        let cut = Cut::at_depth(full, depth);
        self.delta = simulate(full, &cut, &self.ops, &config.layers).ok();
        self.cycles = Some(cycle_groups(&after, depth));
        Some(after)
    }

    /// Confirm the pending op. `false` when there was none or it does not
    /// apply to `full` after the confirmed ones (it is then dropped and
    /// [`Self::error`] says why).
    pub fn confirm(&mut self, full: &Graph) -> bool {
        let Some(op) = self.pending.take() else {
            return false;
        };
        self.ops.push(op);
        if let Err(e) = apply_ops(full, &self.ops) {
            self.ops.pop();
            self.error = Some(e.to_string());
            return false;
        }
        true
    }

    /// The ops as the frozen SimOp JSON (`archguard simulate --ops`, the
    /// web viewer's `archguard-simops.json`).
    pub fn export(&self) -> String {
        serde_json::to_string_pretty(&self.ops).expect("SimOps serialize")
    }
}

/// Cyclic groups of `g` aggregated to `depth`, members by name.
fn cycle_groups(g: &Graph, depth: usize) -> Vec<Vec<String>> {
    let view = aggregate(g, &Cut::at_depth(g, depth));
    let name = |id: &NodeId| view.nodes.iter().find(|n| n.id == *id).map(|n| n.name.clone());
    find_sccs(&view)
        .iter()
        .map(|s| s.node_ids.iter().filter_map(name).collect())
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use archguard_engine::core::{Edge, Node, NodeKind, Ref};

    /// app/a.rs ↔ core/b.rs: one cycle between `app` and `core`.
    pub(crate) fn graph() -> Graph {
        let node = |id: u32, name: &str, kind, parent: Option<u32>| Node {
            id: NodeId(id),
            name: name.into(),
            kind,
            parent: parent.map(NodeId),
            lang: None,
        };
        let edge = |from: u32, to: u32| Edge {
            from: NodeId(from),
            to: NodeId(to),
            refs: vec![Ref {
                file: "f".into(),
                line: 1,
                raw: "use".into(),
            }],
            count: Some(1),
        };
        Graph {
            schema_version: 1,
            root: ".".into(),
            nodes: vec![
                node(0, "app", NodeKind::Module, None),
                node(1, "app/a.rs", NodeKind::File, Some(0)),
                node(2, "core", NodeKind::Module, None),
                node(3, "core/b.rs", NodeKind::File, Some(2)),
            ],
            edges: vec![edge(1, 3), edge(3, 1)],
            semantic: None,
            review_kpis: None,
        }
    }

    pub(crate) fn cut(from: &str, to: &str) -> SimOp {
        SimOp::CutEdge {
            from: from.into(),
            to: to.into(),
        }
    }

    #[test]
    fn without_ops_the_analysis_is_shown() {
        let mut s = Simulation::default();
        assert!(s.apply(&graph(), &Config::default()).is_none());
        assert!(s.delta.is_none() && s.cycles.is_none() && s.error.is_none());
    }

    #[test]
    fn a_confirmed_cut_resolves_the_cycle() {
        let (g, mut s) = (graph(), Simulation::default());
        s.pending = Some(cut("core", "app"));
        assert!(s.confirm(&g));
        assert!(s.pending.is_none());
        let after = s.apply(&g, &Config::default()).expect("simulated");
        assert_eq!(after.edges.len(), 1);
        let d = s.delta.as_ref().expect("delta");
        assert_eq!((d.scc_count_before, d.scc_count_after), (1, 0));
        assert_eq!(s.cycles.as_deref(), Some(&[][..]));
    }

    #[test]
    fn an_op_that_does_not_apply_is_dropped_with_the_reason() {
        let (g, mut s) = (graph(), Simulation::default());
        s.pending = Some(cut("core", "nowhere"));
        assert!(!s.confirm(&g));
        assert!(s.ops.is_empty());
        assert!(s.error.as_deref().is_some_and(|e| e.contains("nowhere")));
    }

    /// Ops kept from an older analysis that no longer apply: the original
    /// graph stays shown, the ops are kept, the error says why.
    #[test]
    fn stale_ops_leave_the_original_graph() {
        let mut s = Simulation {
            ops: vec![cut("gone", "app")],
            ..Default::default()
        };
        assert!(s.apply(&graph(), &Config::default()).is_none());
        assert_eq!(s.ops.len(), 1);
        assert!(s.error.is_some());
    }

    /// The export is the frozen SimOp JSON the CLI and web viewer read.
    #[test]
    fn export_is_the_simop_contract() {
        let s = Simulation {
            ops: vec![cut("core", "app")],
            ..Default::default()
        };
        let back: Vec<SimOp> = serde_json::from_str(&s.export()).expect("json");
        assert_eq!(back, s.ops);
        assert!(s.export().contains("\"cut_edge\""));
    }
}
