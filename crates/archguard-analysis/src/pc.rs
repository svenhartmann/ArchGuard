//! Propagation Cost (R04): how far a cyclic SCC propagates change.
//!
//! Per SCC of the condensation DAG: the SCC members plus every node
//! reachable from them, divided by the total node count. Reachability is
//! computed over the original edges — a BFS from all SCC members at once
//! yields exactly the SCC plus its transitive successors, because nodes
//! inside the SCC reach each other by definition.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use archguard_core::{Graph, NodeId, PropagationCost};

use crate::scc::find_sccs;

/// Propagation cost per cyclic SCC of `graph` (empty when acyclic).
///
/// Deterministic: entries follow the stable `scc-N` order of
/// [`find_sccs`]. Edges to unknown node ids are skipped, mirroring the
/// SCC detection.
pub fn propagation_cost(graph: &Graph) -> Vec<PropagationCost> {
    let sccs = find_sccs(graph);
    if sccs.is_empty() {
        return Vec::new();
    }
    let mut successors: BTreeMap<u32, Vec<NodeId>> = BTreeMap::new();
    for edge in &graph.edges {
        if graph.node_name(edge.from).is_some() && graph.node_name(edge.to).is_some() {
            successors.entry(edge.from.0).or_default().push(edge.to);
        }
    }
    let total = graph.nodes.len();
    sccs.into_iter()
        .map(|scc| {
            let mut affected: BTreeSet<NodeId> = scc.node_ids.iter().copied().collect();
            let mut queue: VecDeque<NodeId> = affected.iter().copied().collect();
            while let Some(node) = queue.pop_front() {
                if let Some(outs) = successors.get(&node.0) {
                    for &next in outs {
                        if affected.insert(next) {
                            queue.push_back(next);
                        }
                    }
                }
            }
            PropagationCost {
                scc_id: scc.id,
                size: scc.node_ids.len(),
                affected: affected.len(),
                total,
                pc: if total == 0 {
                    0.0
                } else {
                    affected.len() as f64 / total as f64
                },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_helpers::graph;

    #[test]
    fn three_node_scc_in_ten_node_dag_propagates_over_reachable() {
        // a ↔ b ↔ c form the SCC; c → d → e and a → f pull in five
        // more; g/h/i/j stay disconnected.
        let g = graph(
            &["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"],
            &[
                ("a", "b"),
                ("b", "c"),
                ("c", "a"),
                ("c", "d"),
                ("d", "e"),
                ("a", "f"),
                ("g", "h"),
            ],
        );
        let costs = propagation_cost(&g);
        assert_eq!(costs.len(), 1);
        let pc = &costs[0];
        assert_eq!(pc.scc_id, "scc-1");
        assert_eq!(pc.size, 3);
        // SCC (a,b,c) + d,e,f = 6 of 10 nodes.
        assert_eq!(pc.affected, 6);
        assert_eq!(pc.total, 10);
        assert!((pc.pc - 0.6).abs() < f64::EPSILON, "pc {}", pc.pc);
    }

    #[test]
    fn empty_graph_yields_no_costs() {
        assert!(propagation_cost(&graph(&[], &[])).is_empty());
    }

    #[test]
    fn acyclic_graph_yields_no_costs() {
        let g = graph(&["a", "b", "c"], &[("a", "b"), ("b", "c")]);
        assert!(propagation_cost(&g).is_empty());
    }

    #[test]
    fn two_sccs_each_measure_their_own_reach() {
        // SCC1 = {a,b} → x; SCC2 = {c,d} → nothing further.
        let g = graph(
            &["a", "b", "c", "d", "x"],
            &[("a", "b"), ("b", "a"), ("b", "x"), ("c", "d"), ("d", "c")],
        );
        let costs = propagation_cost(&g);
        assert_eq!(costs.len(), 2);
        // scc-1 is the {a,b} component (smallest member "a").
        assert_eq!(costs[0].scc_id, "scc-1");
        assert_eq!((costs[0].affected, costs[0].total), (3, 5));
        assert_eq!(costs[1].scc_id, "scc-2");
        assert_eq!((costs[1].affected, costs[1].total), (2, 5));
    }

    #[test]
    fn signature_is_stable() {
        let f: fn(&Graph) -> Vec<PropagationCost> = propagation_cost;
        let _ = f;
    }
}
