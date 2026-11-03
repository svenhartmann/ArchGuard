//! Elementary-cycle sampling (T13): a deterministic handful of the
//! cycles inside one [`Scc`], found by depth-first search.
//!
//! [`sample_cycles`] starts at the lexicographically smallest member of
//! the component (by node name, ties broken by id — the same order
//! `scc.rs` sorts members by) and walks SCC-internal edges only, so the
//! search can neither leave the component nor touch the rest of the
//! graph. Every walk that closes back onto the start node is an
//! *elementary* cycle: no node repeats except the implicit return to
//! the start (a returned vector lists each node once; the closing edge
//! back to the first node is implied).
//!
//! Each cycle is canonicalized by rotating it so its smallest node comes
//! first, and canonical forms already collected are deduplicated; the
//! output order is the DFS discovery order over name-sorted neighbour
//! lists, so repeated calls on the same input return the same vector —
//! everything is deterministic (Kap. 12). A separate examined-edge budget
//! bounds work even when no short cycle can be found:
//! at most `max_cycles` cycles are collected and no cycle is longer than
//! `max_len` nodes.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use archguard_core::{CycleSampling, Graph, NodeId};

use crate::scc::Scc;

/// Sample up to `max_cycles` elementary cycles of at most `max_len`
/// nodes from `scc`, using the edges of `graph` whose *both* endpoints
/// are members of `scc` (edges with unknown endpoints cannot be members
/// and are skipped).
///
/// The search is a DFS from the lexicographically smallest member, with
/// every neighbour list explored in name order (ties broken by id). A
/// cycle is returned as the list of its nodes in traversal order, the
/// closing edge back to the first node implied; `max_cycles == 0`,
/// `max_len == 0` or an empty component yield no cycles. See the module
/// docs for the canonicalization and the determinism guarantee.
pub fn sample_cycles(
    graph: &Graph,
    scc: &Scc,
    max_cycles: usize,
    max_len: usize,
) -> Vec<Vec<NodeId>> {
    sample_cycles_bounded(graph, scc, max_cycles, max_len, DEFAULT_EDGE_BUDGET).cycles
}

pub const DEFAULT_EDGE_BUDGET: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CycleSample {
    pub cycles: Vec<Vec<NodeId>>,
    pub sampling: CycleSampling,
}

/// Synchronous search bounded by examined edges, independently of the
/// number of successful cycles. No worker/thread cancellation is required
/// inside this bounded phase; callers must still bound extraction/SCC work.
pub fn sample_cycles_bounded(
    graph: &Graph,
    scc: &Scc,
    max_cycles: usize,
    max_len: usize,
    edge_budget: usize,
) -> CycleSample {
    let mut sampling = CycleSampling {
        searched_from: None,
        explored_edges: 0,
        edge_budget,
        budget_exhausted: false,
        length_limited: max_len == 0,
        cycle_limit_reached: max_cycles == 0,
        root_search_complete: false,
        complete: false,
    };
    if max_cycles == 0 || max_len == 0 {
        return CycleSample {
            cycles: Vec::new(),
            sampling,
        };
    }
    // DFS root: the lexicographically smallest member, by name, ties by id.
    let Some(&start) = scc.node_ids.iter().min_by_key(|&&id| (name(graph, id), id)) else {
        return CycleSample {
            cycles: Vec::new(),
            sampling,
        };
    };
    sampling.searched_from = Some(name(graph, start).to_string());
    let members: BTreeSet<NodeId> = scc.node_ids.iter().copied().collect();

    // SCC-internal adjacency; neighbours explored in name order (ties by
    // id), duplicate edges folded — hand-assembled graphs may repeat one.
    let mut adjacency: BTreeMap<NodeId, Vec<NodeId>> = BTreeMap::new();
    for edge in &graph.edges {
        if members.contains(&edge.from) && members.contains(&edge.to) {
            adjacency.entry(edge.from).or_default().push(edge.to);
        }
    }
    for neighbours in adjacency.values_mut() {
        neighbours.sort_unstable_by_key(|&id| (name(graph, id), id));
        neighbours.dedup();
    }

    let mut sampler = Sampler {
        graph,
        adjacency,
        start,
        max_cycles,
        // Keep recursion bounded even for programmatic usize::MAX callers.
        max_len: max_len.min(256),
        sampling,
        path: vec![start],
        on_path: [start].into_iter().collect(),
        seen: HashSet::new(),
        cycles: Vec::new(),
    };
    let completed = sampler.extend();
    sampler.sampling.root_search_complete = completed && !sampler.sampling.length_limited;
    CycleSample {
        cycles: sampler.cycles,
        sampling: sampler.sampling,
    }
}

/// DFS state of [`sample_cycles`], carried through the recursion.
struct Sampler<'a> {
    sampling: CycleSampling,
    graph: &'a Graph,
    adjacency: BTreeMap<NodeId, Vec<NodeId>>,
    start: NodeId,
    max_cycles: usize,
    max_len: usize,
    /// Current walk, always beginning with `start`; its length never
    /// exceeds `max_len`, so every closed walk is at most that long.
    path: Vec<NodeId>,
    /// Membership mirror of `path` for O(1) elementary-cycle checks.
    on_path: HashSet<NodeId>,
    /// Canonical forms already collected, for dedup.
    seen: HashSet<Vec<NodeId>>,
    /// Collected cycles, in discovery order.
    cycles: Vec<Vec<NodeId>>,
}

impl Sampler<'_> {
    /// Extend `self.path` from its last node. Returns `false` as soon as
    /// the cycle cap is reached and the whole search must stop.
    fn extend(&mut self) -> bool {
        let current = *self.path.last().expect("path always holds the start node");
        // Copy so the loop does not borrow `self` while recursing.
        let neighbours = self.adjacency.get(&current).cloned().unwrap_or_default();
        for &next in &neighbours {
            if self.sampling.explored_edges >= self.sampling.edge_budget {
                self.sampling.budget_exhausted = true;
                return false;
            }
            self.sampling.explored_edges += 1;
            if next == self.start {
                // Closing the walk onto the start is the cycle; its
                // length is `path.len()`, already within `max_len`.
                self.record();
                if self.cycles.len() >= self.max_cycles {
                    self.sampling.cycle_limit_reached = true;
                    return false;
                }
            } else if !self.on_path.contains(&next) && self.path.len() < self.max_len {
                self.path.push(next);
                self.on_path.insert(next);
                let keep_going = self.extend();
                self.on_path.remove(&next);
                self.path.pop();
                if !keep_going {
                    return false;
                }
            } else if !self.on_path.contains(&next) {
                self.sampling.length_limited = true;
            }
        }
        true
    }

    /// Record the current `path` as a cycle in canonical form, unless
    /// that form was collected before.
    fn record(&mut self) {
        let cycle = canonicalize(self.graph, &self.path);
        if self.seen.insert(cycle.clone()) {
            self.cycles.push(cycle);
        }
    }
}

/// Rotate `cycle` so its lexicographically smallest node (by name, ties
/// broken by id) comes first.
fn canonicalize(graph: &Graph, cycle: &[NodeId]) -> Vec<NodeId> {
    let pivot = cycle
        .iter()
        .enumerate()
        .min_by_key(|&(_, &id)| (name(graph, id), id))
        .map(|(index, _)| index)
        .unwrap_or(0);
    cycle[pivot..]
        .iter()
        .chain(cycle[..pivot].iter())
        .copied()
        .collect()
}

/// Name of `id`, empty for an unknown id (only reachable for malformed
/// input; ids fed in here come from `graph.nodes`).
fn name(graph: &Graph, id: NodeId) -> &str {
    graph.node_name(id).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scc::find_sccs;
    use archguard_core::{Edge, Node, NodeKind};

    /// Hand-assembled graph: `nodes` gives names in *insertion* order
    /// (id = position), `edges` references nodes by name.
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

    /// The single SCC of `g`, failing the test if there is not exactly one.
    fn sole_scc(g: &Graph) -> Scc {
        let mut sccs = find_sccs(g);
        assert_eq!(sccs.len(), 1, "expected exactly one SCC");
        sccs.remove(0)
    }

    /// Complete digraph on `n` nodes named `"a"`, `"b"`, … (id = position);
    /// every ordered pair of distinct nodes is an edge.
    fn complete_digraph(n: usize) -> Graph {
        let names: Vec<String> = (0..n)
            .map(|i| char::from(b'a' + i as u8).to_string())
            .collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut edges = Vec::new();
        for &from in &refs {
            for &to in &refs {
                if from != to {
                    edges.push((from, to));
                }
            }
        }
        graph(&refs, &edges)
    }

    #[test]
    fn exponential_dead_end_search_exhausts_the_edge_budget_deterministically() {
        let mut nodes = vec!["a-start".to_string()];
        for layer in 0..20 {
            for item in 0..3 {
                nodes.push(format!("layer-{layer:02}-{item}"));
            }
        }
        let mut edges = Vec::new();
        for item in 0..3 {
            edges.push((nodes[0].as_str(), nodes[1 + item].as_str()));
        }
        for layer in 0..19 {
            for from in 0..3 {
                for to in 0..3 {
                    edges.push((
                        nodes[1 + layer * 3 + from].as_str(),
                        nodes[1 + (layer + 1) * 3 + to].as_str(),
                    ));
                }
            }
        }
        for item in 0..3 {
            edges.push((nodes[58 + item].as_str(), nodes[0].as_str()));
        }
        let g = graph(
            &nodes.iter().map(String::as_str).collect::<Vec<_>>(),
            &edges,
        );
        assert_eq!((g.nodes.len(), g.edges.len()), (61, 177));
        let scc = sole_scc(&g);
        let first = sample_cycles_bounded(&g, &scc, 100, 20, 1234);
        assert_eq!(first, sample_cycles_bounded(&g, &scc, 100, 20, 1234));
        assert!(first.cycles.is_empty());
        assert_eq!(first.sampling.explored_edges, 1234);
        assert!(first.sampling.budget_exhausted && first.sampling.length_limited);
        assert!(!first.sampling.complete && !first.sampling.root_search_complete);
        assert_eq!(
            find_sccs(&g).len(),
            1,
            "empty sample does not erase the SCC"
        );
    }

    #[test]
    fn triangle_yields_exactly_one_cycle() {
        // Insertion order differs from name order: ids are c=0, a=1, b=2,
        // so the DFS start and the canonical rotation must key on names.
        let g = graph(&["c", "a", "b"], &[("a", "b"), ("b", "c"), ("c", "a")]);
        let scc = sole_scc(&g);

        let cycles = sample_cycles(&g, &scc, 10, usize::MAX);
        assert_eq!(cycles, vec![vec![NodeId(1), NodeId(2), NodeId(0)]]);
    }

    #[test]
    fn k4_respects_max_cycles() {
        let g = complete_digraph(4);
        let scc = sole_scc(&g);

        let cycles = sample_cycles(&g, &scc, 3, usize::MAX);
        assert_eq!(cycles.len(), 3);
        // Sorted-neighbour DFS discovery order: [a,b], [a,b,c], [a,b,c,d].
        assert_eq!(
            cycles,
            vec![
                vec![NodeId(0), NodeId(1)],
                vec![NodeId(0), NodeId(1), NodeId(2)],
                vec![NodeId(0), NodeId(1), NodeId(2), NodeId(3)],
            ]
        );
        // Deterministic: the same input yields the same output.
        assert_eq!(cycles, sample_cycles(&g, &scc, 3, usize::MAX));
    }

    #[test]
    fn k4_without_caps_yields_all_fifteen_cycles() {
        // Through the start node a: 3 two-cycles, 6 oriented triangles
        // and 6 oriented four-cycles.
        let g = complete_digraph(4);
        let scc = sole_scc(&g);

        let cycles = sample_cycles(&g, &scc, usize::MAX, usize::MAX);
        assert_eq!(cycles.len(), 15);
        // Every cycle starts at the smallest node a and repeats no node.
        for cycle in &cycles {
            assert_eq!(cycle[0], NodeId(0));
            let unique: HashSet<NodeId> = cycle.iter().copied().collect();
            assert_eq!(unique.len(), cycle.len());
        }
    }

    #[test]
    fn max_len_is_honored() {
        // SCC: two-cycle a↔b and three-cycle a→c→d→a.
        let g = graph(
            &["a", "b", "c", "d"],
            &[("a", "b"), ("b", "a"), ("a", "c"), ("c", "d"), ("d", "a")],
        );
        let scc = sole_scc(&g);

        let short = sample_cycles(&g, &scc, 10, 2);
        assert_eq!(short, vec![vec![NodeId(0), NodeId(1)]]);

        let all = sample_cycles(&g, &scc, 10, 3);
        assert_eq!(
            all,
            vec![
                vec![NodeId(0), NodeId(1)],
                vec![NodeId(0), NodeId(2), NodeId(3)],
            ]
        );
    }

    #[test]
    fn opposite_orientations_are_distinct_cycles() {
        // a→b→c→a and a→c→b→a coexist; rotation never merges them.
        let g = graph(
            &["a", "b", "c"],
            &[
                ("a", "b"),
                ("b", "c"),
                ("c", "a"),
                ("a", "c"),
                ("c", "b"),
                ("b", "a"),
            ],
        );
        let scc = sole_scc(&g);

        let cycles = sample_cycles(&g, &scc, 10, usize::MAX);
        assert_eq!(
            cycles,
            vec![
                vec![NodeId(0), NodeId(1)],
                vec![NodeId(0), NodeId(1), NodeId(2)],
                vec![NodeId(0), NodeId(2)],
                vec![NodeId(0), NodeId(2), NodeId(1)],
            ]
        );
    }

    #[test]
    fn self_loop_is_a_one_node_cycle() {
        // Hand-assembled: the canonical builder drops self-loops, but
        // the analysis must still treat one as a cycle of length 1.
        let g = graph(&["a"], &[("a", "a")]);
        let scc = sole_scc(&g);

        assert_eq!(
            sample_cycles(&g, &scc, 10, usize::MAX),
            vec![vec![NodeId(0)]]
        );
        assert!(sample_cycles(&g, &scc, 10, 0).is_empty());
    }

    #[test]
    fn edges_outside_the_scc_are_ignored() {
        // Two disjoint cycles plus an a→c bridge; sampling scc-1 must
        // only ever see a and b, even though the graph contains c, d and
        // the bridge edge.
        let g = graph(
            &["a", "b", "c", "d"],
            &[("a", "b"), ("b", "a"), ("a", "c"), ("c", "d"), ("d", "c")],
        );
        let sccs = find_sccs(&g);
        assert_eq!(sccs.len(), 2);

        let cycles = sample_cycles(&g, &sccs[0], 10, usize::MAX);
        assert_eq!(cycles, vec![vec![NodeId(0), NodeId(1)]]);
    }

    #[test]
    fn zero_caps_yield_no_cycles() {
        let g = complete_digraph(3);
        let scc = sole_scc(&g);

        assert!(sample_cycles(&g, &scc, 0, usize::MAX).is_empty());
        assert!(sample_cycles(&g, &scc, 10, 0).is_empty());
    }
}
