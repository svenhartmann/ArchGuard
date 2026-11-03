//! Tangledness quote and hub concentration (R07).
//!
//! * Tangledness: share of nodes inside a cyclic SCC (> 1 member).
//! * Hub-Gini: Gini coefficient of the fan-in distribution plus the edge
//!   share of the five biggest hubs.
//!
//! Edge weights are taken from [`Edge::weight`]: where the graph carries
//! the total reference count the measure is exact, otherwise the sample
//! length is a lower bound — reported as-is, never inflated.

use archguard_core::{Graph, HubGini, NodeId, Tangledness};

use crate::scc::find_sccs;

/// Number of hubs whose incoming edge share is reported in `top_share`.
const TOP_HUBS: usize = 5;

/// Share of nodes that live inside a cyclic SCC.
pub fn tangledness(graph: &Graph) -> Tangledness {
    let tangled_nodes: usize = find_sccs(graph).iter().map(|scc| scc.node_ids.len()).sum();
    let total = graph.nodes.len();
    Tangledness {
        tangled_nodes,
        total,
        quote: if total == 0 {
            0.0
        } else {
            tangled_nodes as f64 / total as f64
        },
    }
}

/// Fan-in inequality (Gini) and the share of all incoming edge weight
/// ending in the five biggest hubs.
pub fn hub_gini(graph: &Graph) -> HubGini {
    let fan_in = incoming_weight(graph);
    let values: Vec<f64> = graph
        .nodes
        .iter()
        .map(|node| fan_in.get(&node.id).copied().unwrap_or(0.0))
        .collect();
    let total: f64 = values.iter().sum();
    let gini = if values.is_empty() || total == 0.0 {
        0.0
    } else {
        // Gini over the sorted values: Σ (2i − n − 1) · v_i / (n · Σv),
        // i = 1..=n, v ascending. Equal distribution → 0.
        let mut sorted = values.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = sorted.len();
        sorted
            .iter()
            .enumerate()
            .map(|(index, value)| (2.0 * (index + 1) as f64 - n as f64 - 1.0) * value)
            .sum::<f64>()
            / (n as f64 * total)
    };
    // Top hubs: highest incoming weight first, name as deterministic tie
    // breaker.
    let mut ranked: Vec<(f64, &str)> = graph
        .nodes
        .iter()
        .map(|node| {
            (
                fan_in.get(&node.id).copied().unwrap_or(0.0),
                node.name.as_str(),
            )
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.cmp(b.1))
    });
    let top_share = if total == 0.0 {
        0.0
    } else {
        ranked
            .iter()
            .take(TOP_HUBS)
            .map(|(weight, _)| weight)
            .sum::<f64>()
            / total
    };
    HubGini { gini, top_share }
}

/// Incoming edge weight per node; edges to unknown ids are skipped.
///
/// An edge always represents at least one dependency reference, so the
/// lower-bound [`Edge::weight`] of hand-assembled edges (no refs, no
/// count) is floored at 1 instead of silently counting zero.
fn incoming_weight(graph: &Graph) -> std::collections::BTreeMap<NodeId, f64> {
    let mut fan_in: std::collections::BTreeMap<NodeId, f64> = std::collections::BTreeMap::new();
    for edge in &graph.edges {
        if graph.node_name(edge.to).is_some() {
            *fan_in.entry(edge.to).or_insert(0.0) += edge.weight().max(1) as f64;
        }
    }
    fan_in
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_helpers::graph;

    #[test]
    fn tangledness_is_zero_for_a_dag_and_positive_for_cycles() {
        let dag = graph(&["a", "b", "c", "d"], &[("a", "b"), ("b", "c"), ("a", "c")]);
        let t = tangledness(&dag);
        assert_eq!((t.tangled_nodes, t.total, t.quote), (0, 4, 0.0));

        // a ↔ b cycle, c/d outside.
        let cyclic = graph(&["a", "b", "c", "d"], &[("a", "b"), ("b", "a")]);
        let t = tangledness(&cyclic);
        assert_eq!((t.tangled_nodes, t.total), (2, 4));
        assert!((t.quote - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn gini_is_zero_for_equal_fan_in() {
        // Every node receives exactly one edge: a→b, b→c, c→a over 3
        // nodes plus a disconnected self-balanced pair would break
        // equality, so keep the ring alone.
        let ring = graph(&["a", "b", "c"], &[("a", "b"), ("b", "c"), ("c", "a")]);
        let g = hub_gini(&ring);
        assert!(g.gini.abs() < 1e-9, "gini {}", g.gini);
        assert!((g.top_share - 1.0).abs() < 1e-9); // 3 hubs ≤ top-5
    }

    #[test]
    fn gini_is_high_and_top_share_captures_the_single_hub() {
        // Twelve sources → one hub, nothing else incoming.
        let mut nodes: Vec<String> = (0..12).map(|i| format!("s{i}")).collect();
        nodes.push("hub".to_string());
        let names: Vec<&str> = nodes.iter().map(String::as_str).collect();
        let edges: Vec<(&str, &str)> = names
            .iter()
            .take(12)
            .map(|source| (*source, "hub"))
            .collect();
        let g = graph(&names, &edges);
        let h = hub_gini(&g);
        // Single hub over n nodes: Gini = (n−1)/n.
        assert!((h.gini - 12.0 / 13.0).abs() < 1e-9, "gini {}", h.gini);
        assert!((h.top_share - 1.0).abs() < 1e-9);
    }

    #[test]
    fn empty_graph_is_neutral() {
        let g = graph(&[], &[]);
        let h = hub_gini(&g);
        assert_eq!((h.gini, h.top_share), (0.0, 0.0));
        let t = tangledness(&g);
        assert_eq!((t.tangled_nodes, t.total, t.quote), (0, 0, 0.0));
    }

    #[test]
    fn signature_is_stable() {
        let f: fn(&Graph) -> Tangledness = tangledness;
        let g: fn(&Graph) -> HubGini = hub_gini;
        let _ = (f, g);
    }
}
