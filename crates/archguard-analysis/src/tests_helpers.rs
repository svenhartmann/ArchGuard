//! Shared hand-assembled [`Graph`] construction for unit tests.

use archguard_core::{Edge, Graph, Node, NodeId, NodeKind};

/// Hand-assembled graph: `nodes` gives names in insertion order
/// (id = position), `edges` references nodes by name. Mirrors the
/// fixture style of the `scc` tests.
pub fn graph(nodes: &[&str], edges: &[(&str, &str)]) -> Graph {
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
