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

    // Component id per node: cyclic SCCs first (in
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
    // Cyclic components hold ids `0..cyclic`; every later one is a
    // singleton.
    let cyclic = comp_size.len();
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

    let topo = topo_order(&successors);
    let ccd = ccd_over(&successors, &topo, &comp_size, cyclic);

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

/// Number of predecessors of every component in the condensation DAG.
fn in_degrees(successors: &[BTreeSet<usize>]) -> Vec<u32> {
    let mut in_degree = vec![0u32; successors.len()];
    for succs in successors {
        for &s in succs {
            in_degree[s] += 1;
        }
    }
    in_degree
}

/// Kahn topological order over the components.
fn topo_order(successors: &[BTreeSet<usize>]) -> Vec<usize> {
    let mut in_degree = in_degrees(successors);
    let mut queue: VecDeque<usize> = (0..successors.len())
        .filter(|&c| in_degree[c] == 0)
        .collect();
    let mut topo: Vec<usize> = Vec::with_capacity(successors.len());
    while let Some(c) = queue.pop_front() {
        topo.push(c);
        for &s in &successors[c] {
            in_degree[s] -= 1;
            if in_degree[s] == 0 {
                queue.push_back(s);
            }
        }
    }
    debug_assert_eq!(topo.len(), successors.len(), "condensation is a DAG");
    topo
}

/// CCD from memoized reachable sets, kept as bitsets (one bit per
/// component) and built in reverse topological order: every successor
/// of `c` is already done when `c` is processed. A union is a run of
/// word ORs instead of a tree insert per member, which kept long chains
/// quadratic in allocations (20 s for a 20k chain). A set is dropped
/// once its last predecessor has merged it, so a chain holds only a few.
fn ccd_over(
    successors: &[BTreeSet<usize>],
    topo: &[usize],
    comp_size: &[u32],
    cyclic: usize,
) -> u64 {
    let words = successors.len().div_ceil(64);
    let mut pending = in_degrees(successors);
    let mut reachable: Vec<Vec<u64>> = vec![Vec::new(); successors.len()];
    let mut ccd: u64 = 0;
    for &c in topo.iter().rev() {
        let mut set = vec![0u64; words];
        for &s in &successors[c] {
            set[s / 64] |= 1 << (s % 64);
            for (word, &other) in set.iter_mut().zip(&reachable[s]) {
                *word |= other;
            }
            pending[s] -= 1;
            if pending[s] == 0 {
                reachable[s] = Vec::new();
            }
        }
        let reached = comp_size[c] as u64 + reached_nodes(&set, comp_size, cyclic);
        ccd += reached * comp_size[c] as u64;
        if pending[c] > 0 {
            reachable[c] = set;
        }
    }
    ccd
}

/// Number of nodes in the components of `set`. Singletons weigh one
/// node, so a popcount covers them; only the cyclic SCCs (ids
/// `0..cyclic`) add their extra members.
fn reached_nodes(set: &[u64], comp_size: &[u32], cyclic: usize) -> u64 {
    let mut reached: u64 = set.iter().map(|w| u64::from(w.count_ones())).sum();
    for (index, &word) in set.iter().enumerate().take(cyclic.div_ceil(64)) {
        let mut bits = word;
        while bits != 0 {
            let s = index * 64 + bits.trailing_zeros() as usize;
            if s < cyclic {
                reached += comp_size[s] as u64 - 1;
            }
            bits &= bits - 1;
        }
    }
    reached
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
    fn isolated_singletons_reach_only_themselves() {
        let g = graph(&["x", "y"], &[]);
        let (ccd, acd, _nccd) = lakos_metrics(&g);
        assert_eq!(ccd, 2);
        assert_eq!(acd, 1.0);
    }

    /// Scale (audit): a 20 000-node chain whose last two nodes form a
    /// cycle. One `BTreeSet` of reachable components per component took
    /// 20 s in release; bitsets finish well inside a second in debug.
    #[test]
    fn long_chain_scales() {
        let n: u32 = 20_000;
        let edge = |from: u32, to: u32| Edge {
            from: NodeId(from),
            to: NodeId(to),
            refs: Vec::new(),
            count: None,
        };
        let mut edges: Vec<Edge> = (1..n).map(|i| edge(i - 1, i)).collect();
        edges.push(edge(n - 1, n - 2));
        let g = Graph {
            review_kpis: None,
            semantic: None,
            schema_version: 1,
            root: "/abs".to_string(),
            nodes: (0..n)
                .map(|i| Node {
                    id: NodeId(i),
                    name: format!("n{i:05}"),
                    kind: NodeKind::File,
                    parent: None,
                    lang: None,
                })
                .collect(),
            edges,
        };

        let started = std::time::Instant::now();
        let (ccd, _acd, _nccd) = lakos_metrics(&g);
        let elapsed = started.elapsed();

        // Node i < n-2 reaches n - i nodes; both cycle members reach 2.
        let n = u64::from(n);
        assert_eq!(ccd, n * (n + 1) / 2 + 1);
        assert!(
            elapsed < std::time::Duration::from_secs(2),
            "lakos on a 20k chain took {elapsed:?}"
        );
    }
}
