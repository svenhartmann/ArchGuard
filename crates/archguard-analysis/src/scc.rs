//! SCC detection (T12): the cyclic strongly connected components of a
//! canonical [`Graph`], found with Tarjan's algorithm over a
//! [`petgraph`] digraph.
//!
//! An [`Scc`] is reported only when it actually contains a cycle: more
//! than one node, or a single node with a self-loop. The canonical
//! builder drops self-loops (see `archguard_core`'s T04 builder), so in
//! practice every reported component has more than one member; the
//! self-loop branch keeps the analysis correct for hand-assembled
//! graphs.
//!
//! Everything is deterministic (Kap. 12): members are sorted by node
//! name, internal edges by their endpoint names, and the components
//! themselves by their smallest member name — which also fixes the
//! `scc-N` ids (1-based, matching the `scc-1`/`scc-2` ids of Kap. 9).

use std::collections::{BTreeMap, BTreeSet};

use archguard_core::{Graph, NodeId};
use petgraph::algo::tarjan_scc;
use petgraph::prelude::{DiGraph, NodeIndex};

/// A strongly connected component of the dependency graph that contains
/// a cycle — the unit the cycle findings of Kap. 5 are built from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scc {
    /// Stable id `scc-N`; `N` reflects the order by smallest member
    /// name, starting at 1 (Kap. 9: `scc-1`, `scc-2`, …).
    pub id: String,
    /// Members, sorted by node name (ties broken by id).
    pub node_ids: Vec<NodeId>,
    /// Dependency edges with *both* endpoints inside the component
    /// (external edges are excluded), sorted by endpoint names,
    /// deduplicated.
    pub edges: Vec<(NodeId, NodeId)>,
}

/// Find every cyclic strongly connected component of `graph`.
///
/// The graph is mirrored into a `petgraph` digraph and run through
/// [`tarjan_scc`]; only components with more than one node or a
/// self-loop survive. Edges pointing at node ids absent from
/// `graph.nodes` cannot be resolved and are skipped. The result is
/// fully deterministic: see the module docs.
pub fn find_sccs(graph: &Graph) -> Vec<Scc> {
    let mut pet = DiGraph::<NodeId, ()>::new();
    let mut index_of: BTreeMap<u32, NodeIndex> = BTreeMap::new();
    for node in &graph.nodes {
        index_of.insert(node.id.0, pet.add_node(node.id));
    }
    for edge in &graph.edges {
        if let (Some(&from), Some(&to)) = (index_of.get(&edge.from.0), index_of.get(&edge.to.0)) {
            pet.add_edge(from, to, ());
        }
    }

    // (smallest member name, members sorted by name) — the pair the
    // final ordering and id assignment keys on.
    let mut components: Vec<(String, Vec<NodeId>)> = Vec::new();
    for component in tarjan_scc(&pet) {
        let cyclic = component.len() > 1 || component.iter().any(|&i| pet.contains_edge(i, i));
        if !cyclic {
            continue;
        }
        let mut members: Vec<NodeId> = component.iter().map(|&i| pet[i]).collect();
        members.sort_by_key(|&id| (name(graph, id), id));
        let smallest = members
            .first()
            .map(|id| name(graph, *id).to_string())
            .unwrap_or_default();
        components.push((smallest, members));
    }

    components.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    components
        .into_iter()
        .enumerate()
        .map(|(index, (_smallest, node_ids))| Scc {
            id: format!("scc-{}", index + 1),
            edges: internal_edges(graph, &node_ids),
            node_ids,
        })
        .collect()
}

/// Edges of `graph` whose endpoints are all members of `node_ids`,
/// sorted by endpoint names (ties broken by id) and deduplicated.
fn internal_edges(graph: &Graph, node_ids: &[NodeId]) -> Vec<(NodeId, NodeId)> {
    let members: BTreeSet<NodeId> = node_ids.iter().copied().collect();
    let mut edges: Vec<(NodeId, NodeId)> = graph
        .edges
        .iter()
        .filter(|e| members.contains(&e.from) && members.contains(&e.to))
        .map(|e| (e.from, e.to))
        .collect();
    edges.sort_by(|a, b| {
        (name(graph, a.0), name(graph, a.1), *a).cmp(&(name(graph, b.0), name(graph, b.1), *b))
    });
    edges.dedup();
    edges
}

/// Name of `id`, empty for an unknown id (only reachable for
/// malformed input; ids fed in here come from `graph.nodes`).
fn name(graph: &Graph, id: NodeId) -> &str {
    graph.node_name(id).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[test]
    fn empty_graph_has_no_sccs() {
        assert!(find_sccs(&graph(&[], &[])).is_empty());
    }

    #[test]
    fn dag_has_no_sccs() {
        let g = graph(
            &["a", "b", "c", "d"],
            &[("a", "b"), ("b", "c"), ("a", "c"), ("c", "d")],
        );
        assert!(find_sccs(&g).is_empty());
    }

    #[test]
    fn finds_one_three_cycle_sorted_by_name() {
        // Insertion order deliberately differs from name order:
        // ids are c=0, a=1, b=2 — sorting must happen by name.
        let g = graph(&["c", "a", "b"], &[("a", "b"), ("b", "c"), ("c", "a")]);
        let sccs = find_sccs(&g);

        assert_eq!(sccs.len(), 1);
        assert_eq!(sccs[0].id, "scc-1");
        // Members a(1), b(2), c(0) in name order.
        assert_eq!(sccs[0].node_ids, vec![NodeId(1), NodeId(2), NodeId(0)]);
        // Internal edges sorted by endpoint names: (a,b) < (b,c) < (c,a).
        assert_eq!(
            sccs[0].edges,
            vec![
                (NodeId(1), NodeId(2)),
                (NodeId(2), NodeId(0)),
                (NodeId(0), NodeId(1))
            ]
        );
    }

    #[test]
    fn two_disjoint_cycles_sorted_by_smallest_member_name() {
        // Insertion order is fully reversed against name order, and an
        // external edge b → d must not appear in either component.
        let g = graph(
            &["e", "d", "c", "b", "a"],
            &[
                ("a", "b"),
                ("b", "a"),
                ("c", "d"),
                ("d", "e"),
                ("e", "c"),
                ("b", "d"),
            ],
        );
        let sccs = find_sccs(&g);

        assert_eq!(sccs.len(), 2);
        // Insertion ids: e=0, d=1, c=2, b=3, a=4.
        assert_eq!(sccs[0].id, "scc-1"); // smallest member "a"
        assert_eq!(sccs[0].node_ids, vec![NodeId(4), NodeId(3)]);
        assert_eq!(
            sccs[0].edges,
            vec![(NodeId(4), NodeId(3)), (NodeId(3), NodeId(4))]
        );
        assert_eq!(sccs[1].id, "scc-2"); // smallest member "c"
        assert_eq!(sccs[1].node_ids, vec![NodeId(2), NodeId(1), NodeId(0)]);
        // Sorted by names: (c,d) < (d,e) < (e,c).
        assert_eq!(
            sccs[1].edges,
            vec![
                (NodeId(2), NodeId(1)),
                (NodeId(1), NodeId(0)),
                (NodeId(0), NodeId(2))
            ]
        );
    }

    #[test]
    fn single_node_self_loop_is_an_scc() {
        let g = graph(&["a"], &[("a", "a")]);
        let sccs = find_sccs(&g);

        assert_eq!(sccs.len(), 1);
        assert_eq!(sccs[0].id, "scc-1");
        assert_eq!(sccs[0].node_ids, vec![NodeId(0)]);
        assert_eq!(sccs[0].edges, vec![(NodeId(0), NodeId(0))]);
    }

    #[test]
    fn edges_to_unknown_node_ids_are_skipped() {
        let mut g = graph(&["a", "b"], &[("a", "b")]);
        g.edges.push(Edge {
            from: NodeId(99),
            to: NodeId(0),
            refs: Vec::new(),
            count: None,
        });
        assert!(find_sccs(&g).is_empty());
    }

    #[test]
    fn duplicate_internal_edges_are_deduplicated() {
        let mut g = graph(&["a", "b"], &[("a", "b"), ("b", "a")]);
        // Hand-assembled graphs may repeat an edge; the builder would not.
        g.edges.push(Edge {
            from: NodeId(0),
            to: NodeId(1),
            refs: Vec::new(),
            count: None,
        });
        let sccs = find_sccs(&g);

        assert_eq!(sccs.len(), 1);
        assert_eq!(
            sccs[0].edges,
            vec![(NodeId(0), NodeId(1)), (NodeId(1), NodeId(0))]
        );
    }
}
