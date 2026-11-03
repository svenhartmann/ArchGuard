//! Lakos coupling metrics (T36, Kap. 7): CCD, ACD and NCCD.
//!
//! For every node the size of its *Depends-Upon* set is the number of
//! nodes reachable from it over the dependency edges **including the
//! node itself**; the CCD is the sum of those sizes over all nodes, the
//! ACD the mean `ccd / n` and the NCCD the CCD normalized by the CCD a
//! balanced tree of the same size would have —
//! `ccd_balanced(n) = (n + 1) * log2(n + 1) - n` (Lakos' approximation).
//! Values near `1.0` mean tree-like coupling, well
//! above `1.0` means tangled coupling.
//!
//! Every node of `graph.nodes` counts, `Module`-kind nodes included
//! (they carry no edges and contribute exactly themselves).
//!
//! # Algorithm
//!
//! Naive per-node DFS would cost O(n · (n + m)). Instead the graph is
//! condensed first: nodes in the same strongly connected component all
//! reach exactly the same set, so [`crate::scc::find_sccs`] components become
//! single condensation nodes and every remaining node is a singleton
//! component. Reachable-component sets are then computed **memoized** on
//! the condensation DAG — one component's set is the union of its
//! successors' sets plus the successors themselves — visiting each
//! component once in reverse topological order (Kahn). Each member node
//! of a component gets that component's reachable node count.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use archguard_core::Graph;

use crate::scc::find_sccs;

/// The Lakos triple `(ccd, acd, nccd)` of `graph` (Kap. 7).
///
/// * `ccd`: sum of *Depends-Upon* set sizes over all nodes, each node
///   counting itself; nodes of one SCC share one set.
/// * `acd`: `ccd / n` — `0.0` for the empty graph (no division by zero).
/// * `nccd`: `ccd / ccd_balanced(n)` with
///   `ccd_balanced(n) = (n + 1) * log2(n + 1) - n`. The normalizer is
///   positive for every `n >= 1`; it is `0` exactly at `n = 0`, where the
///   whole triple is defined to be `(0, 0.0, 0.0)`. Should it ever not be
///   positive, `nccd` is `0.0` as well (no infinity in a report).
///
/// Edges whose endpoints are absent from `graph.nodes` cannot be
/// resolved and are skipped, as elsewhere in the analysis crate.
pub fn lakos_metrics(graph: &Graph) -> (u64, f64, f64) {
    let n = graph.nodes.len();
    if n == 0 {
        return (0, 0.0, 0.0);
    }

    // Component id per node: cyclic SCCs from Tarjan first (in
    // `find_sccs` order), then one singleton per remaining node.
    let mut comp_of: BTreeMap<u32, usize> = BTreeMap::new();
    let mut comp_size: Vec<u32> = Vec::new();
    for scc in find_sccs(graph) {
        let id = comp_size.len();
        comp_size.push(scc.node_ids.len() as u32);
        for node_id in scc.node_ids {
            comp_of.insert(node_id.0, id);
        }
    }
    for node in &graph.nodes {
        comp_of.entry(node.id.0).or_insert_with(|| {
            let id = comp_size.len();
            comp_size.push(1);
            id
        });
    }

    // Condensation DAG: deduplicated inter-component edges (self-loops
    // and edges inside one component drop out, duplicates merge).
    let mut successors: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); comp_size.len()];
    for edge in &graph.edges {
        let (Some(&from), Some(&to)) = (comp_of.get(&edge.from.0), comp_of.get(&edge.to.0)) else {
            continue;
        };
        if from != to {
            successors[from].insert(to);
        }
    }

    // Kahn topological order over the components.
    let mut in_degree = vec![0u32; comp_size.len()];
    for succs in &successors {
        for &s in succs {
            in_degree[s] += 1;
        }
    }
    let mut queue: VecDeque<usize> = (0..comp_size.len())
        .filter(|&c| in_degree[c] == 0)
        .collect();
    let mut topo: Vec<usize> = Vec::with_capacity(comp_size.len());
    while let Some(c) = queue.pop_front() {
        topo.push(c);
        for &s in &successors[c] {
            in_degree[s] -= 1;
            if in_degree[s] == 0 {
                queue.push_back(s);
            }
        }
    }
    debug_assert_eq!(topo.len(), comp_size.len(), "condensation is a DAG");

    // Memoized reachable sets, reverse topological order: every
    // successor of `c` is already done when `c` is processed.
    let mut reachable: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); comp_size.len()];
    for &c in topo.iter().rev() {
        let mut set = BTreeSet::new();
        for &s in &successors[c] {
            set.insert(s);
            set.extend(reachable[s].iter().copied());
        }
        reachable[c] = set;
    }

    // Each member node of `c` reaches `c` itself plus every node of the
    // components in `reachable[c]`.
    let mut ccd: u64 = 0;
    for c in 0..comp_size.len() {
        let reached: u64 = comp_size[c] as u64
            + reachable[c]
                .iter()
                .map(|&s| comp_size[s] as u64)
                .sum::<u64>();
        ccd += reached * comp_size[c] as u64;
    }

    let acd = ccd as f64 / n as f64;
    let nf = n as f64;
    let balanced = (nf + 1.0) * (nf + 1.0).log2() - nf;
    let nccd = if balanced > 0.0 {
        ccd as f64 / balanced
    } else {
        0.0
    };
    (ccd, acd, nccd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{Edge, Node, NodeId, NodeKind};

    /// Hand-assembled graph of `File`-kind nodes: `nodes` gives names in
    /// *insertion* order (id = position), `edges` references nodes by
    /// name (same helper style as `metrics.rs`).
    fn graph(nodes: &[&str], edges: &[(&str, &str)]) -> Graph {
        let id_of = |name: &str| -> NodeId {
            NodeId(
                nodes
                    .iter()
                    .position(|n| *n == name)
                    .expect("test: edge endpoint must be a listed node") as u32,
            )
        };
        Graph {
            review_kpis: None,
            semantic: None,
            schema_version: 1,
            root: "/abs".to_string(),
            nodes: nodes
                .iter()
                .enumerate()
                .map(|(i, n)| Node {
                    id: NodeId(i as u32),
                    name: (*n).to_string(),
                    kind: NodeKind::File,
                    parent: None,
                    lang: None,
                })
                .collect(),
            edges: edges
                .iter()
                .map(|(from, to)| Edge {
                    from: id_of(from),
                    to: id_of(to),
                    refs: Vec::new(),
                    count: None,
                })
                .collect(),
        }
    }

    /// `ccd_balanced(n)` as defined by Lakos.
    fn balanced(n: usize) -> f64 {
        let nf = n as f64;
        (nf + 1.0) * (nf + 1.0).log2() - nf
    }

    #[test]
    fn empty_graph_is_all_zero() {
        let g = graph(&[], &[]);
        assert_eq!(lakos_metrics(&g), (0, 0.0, 0.0));
    }

    #[test]
    fn chain_of_three_has_ccd_six() {
        // a → b → c: a reaches {a,b,c} (3), b reaches {b,c} (2),
        // c reaches {c} (1) → CCD = 3 + 2 + 1 = 6.
        let g = graph(&["a", "b", "c"], &[("a", "b"), ("b", "c")]);
        let (ccd, acd, nccd) = lakos_metrics(&g);
        assert_eq!(ccd, 6);
        assert_eq!(acd, 6.0 / 3.0);
        // balanced(3) = 4 * log2(4) - 3 = 5 exactly → nccd = 6/5.
        assert_eq!(balanced(3), 5.0);
        assert!((nccd - 6.0 / 5.0).abs() < 1e-12);
    }

    #[test]
    fn star_has_ccd_seven() {
        // center → {l1, l2, l3}: the center reaches all four nodes (4),
        // each leaf reaches only itself (1) → CCD = 4 + 1 + 1 + 1 = 7.
        let g = graph(
            &["center", "l1", "l2", "l3"],
            &[("center", "l1"), ("center", "l2"), ("center", "l3")],
        );
        let (ccd, acd, nccd) = lakos_metrics(&g);
        assert_eq!(ccd, 7);
        assert_eq!(acd, 7.0 / 4.0);
        assert!((nccd - 7.0 / balanced(4)).abs() < 1e-12);
    }

    #[test]
    fn three_cycle_every_node_reaches_all() {
        // a → b → c → a is one SCC: every node's Depends-Upon set is the
        // whole component → CCD = 3 * 3 = 9.
        let g = graph(&["a", "b", "c"], &[("a", "b"), ("b", "c"), ("c", "a")]);
        let (ccd, acd, nccd) = lakos_metrics(&g);
        assert_eq!(ccd, 9);
        assert_eq!(acd, 3.0);
        assert!((nccd - 9.0 / balanced(3)).abs() < 1e-12);
    }

    #[test]
    fn two_cycles_share_one_reachable_set_per_member() {
        // {a, b} is a cycle, c points into it (c → a). The two members
        // share one Depends-Upon set {a, b} (2 each), c reaches
        // {c, a, b} (3) → CCD = 2 + 2 + 3 = 7. A duplicated and a
        // dangling edge must not change the result.
        let mut g = graph(&["a", "b", "c"], &[("a", "b"), ("b", "a"), ("c", "a")]);
        g.edges.push(Edge {
            from: NodeId(0),
            to: NodeId(1),
            refs: Vec::new(),
            count: None,
        }); // duplicate a → b
        g.edges.push(Edge {
            from: NodeId(2),
            to: NodeId(99),
            refs: Vec::new(),
            count: None,
        }); // dangling
        let (ccd, acd, _nccd) = lakos_metrics(&g);
        assert_eq!(ccd, 7);
        assert_eq!(acd, 7.0 / 3.0);
    }

    #[test]
    fn summarize_carries_the_lakos_triple() {
        use crate::metrics::summarize;
        use crate::scc::find_sccs;
        // Chain of 3 again: the summary must carry the same numbers.
        let g = graph(&["a", "b", "c"], &[("a", "b"), ("b", "c")]);
        let summary = summarize(&g, &find_sccs(&g), &[]);
        assert_eq!(summary.ccd, 6);
        assert_eq!(summary.acd, 2.0);
        assert!((summary.nccd - 6.0 / 5.0).abs() < 1e-12);
    }

    #[test]
    fn isolated_singletons_reach_only_themselves() {
        let g = graph(&["x", "y"], &[]);
        let (ccd, acd, _nccd) = lakos_metrics(&g);
        assert_eq!(ccd, 2);
        assert_eq!(acd, 1.0);
    }
}
