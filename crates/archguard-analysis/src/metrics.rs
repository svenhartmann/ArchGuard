//! Martin metrics for every node of the supplied view (including modules).
//! Callers aggregate a hierarchical graph before evaluating view metrics.
//! Duplicate edges count once; unknown endpoints are ignored.

use std::collections::{BTreeMap, BTreeSet};

use archguard_core::{
    Finding, FindingKind, Graph, ModuleMetrics, NodeId, Severity, Status, Summary,
};

use crate::scc::Scc;

/// Martin metrics for each visible node, including aggregated modules.
pub fn compute_metrics(graph: &Graph, sccs: &[Scc]) -> BTreeMap<String, ModuleMetrics> {
    let in_cycle: BTreeSet<NodeId> = sccs
        .iter()
        .flat_map(|scc| scc.node_ids.iter().copied())
        .collect();

    // (from, to) pairs already counted, so duplicates fold into one.
    let mut counted: BTreeSet<(u32, u32)> = BTreeSet::new();
    let mut fan_in: BTreeMap<NodeId, u32> = BTreeMap::new();
    let mut fan_out: BTreeMap<NodeId, u32> = BTreeMap::new();
    for edge in &graph.edges {
        if !is_node(graph, edge.from) || !is_node(graph, edge.to) {
            continue;
        }
        if !counted.insert((edge.from.0, edge.to.0)) {
            continue;
        }
        *fan_in.entry(edge.to).or_default() += 1;
        *fan_out.entry(edge.from).or_default() += 1;
    }

    graph
        .nodes
        .iter()
        .map(|node| {
            let fan_in = fan_in.get(&node.id).copied().unwrap_or(0);
            let fan_out = fan_out.get(&node.id).copied().unwrap_or(0);
            let instability = instability(fan_in, fan_out);
            (
                node.name.clone(),
                ModuleMetrics {
                    fan_in,
                    fan_out,
                    instability,
                    in_cycle: in_cycle.contains(&node.id),
                },
            )
        })
        .collect()
}

/// The run-wide [`Summary`] (Kap. 5): node and edge counts straight
/// from `graph`, SCC counts from `sccs` (`largest_scc` is `0` for an
/// acyclic graph), `layer_violations` counting the [`FindingKind::LayerViolation`]
/// findings, and `status` [`Status::Fail`] as soon as any finding has
/// [`Severity::Error`], [`Status::Pass`] otherwise. The Lakos triple
/// `ccd`/`acd`/`nccd` (Kap. 7) comes from [`crate::lakos::lakos_metrics`].
pub fn summarize(graph: &Graph, sccs: &[Scc], findings: &[Finding]) -> Summary {
    let (ccd, acd, nccd) = crate::lakos::lakos_metrics(graph);
    Summary {
        nodes: graph.nodes.len() as u32,
        edges: graph.edges.len() as u32,
        sccs: sccs.len() as u32,
        largest_scc: sccs
            .iter()
            .map(|scc| scc.node_ids.len() as u32)
            .max()
            .unwrap_or(0),
        layer_violations: findings
            .iter()
            .filter(|finding| finding.kind == FindingKind::LayerViolation)
            .count() as u32,
        status: if findings
            .iter()
            .any(|finding| finding.severity == Severity::Error)
        {
            Status::Fail
        } else {
            Status::Pass
        },
        ccd,
        acd,
        nccd,
    }
}

/// `fan_out / (fan_in + fan_out)`, `0.0` when both are `0`.
fn instability(fan_in: u32, fan_out: u32) -> f32 {
    let total = fan_in + fan_out;
    if total == 0 {
        0.0
    } else {
        fan_out as f32 / total as f32
    }
}

/// Whether `id` names a node in the supplied view.
fn is_node(graph: &Graph, id: NodeId) -> bool {
    graph.nodes.iter().any(|node| node.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scc::find_sccs;
    use archguard_core::{Edge, Node, NodeKind};

    /// Hand-assembled graph of `File`-kind nodes: `nodes` gives names in
    /// *insertion* order (id = position), `edges` references nodes by
    /// name (same helper style as `scc.rs` and `cycles.rs`).
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

    /// A finding with only the fields `summarize` looks at being set.
    fn finding(kind: FindingKind, severity: Severity) -> Finding {
        Finding {
            message: None,
            evidence: Vec::new(),
            kind,
            id: "f-1".to_string(),
            severity,
            nodes: Vec::new(),
            edge_count: 0,
            cycles_sampled: 0,
            sampling: None,
            suggested_cuts: Vec::new(),
        }
    }

    #[test]
    fn empty_graph_yields_no_metrics_and_zeroed_summary() {
        let g = graph(&[], &[]);
        assert!(compute_metrics(&g, &[]).is_empty());

        let summary = summarize(&g, &[], &[]);
        assert_eq!(summary.nodes, 0);
        assert_eq!(summary.edges, 0);
        assert_eq!(summary.sccs, 0);
        assert_eq!(summary.largest_scc, 0);
        assert_eq!(summary.layer_violations, 0);
        assert_eq!(summary.status, Status::Pass);
    }

    #[test]
    fn fan_in_and_fan_out_count_distinct_edges() {
        // Diamond-shaped DAG a → b, a → c, b → c; a second a → b edge is
        // a hand-assembled duplicate that must not be counted twice.
        let mut g = graph(&["a", "b", "c"], &[("a", "b"), ("b", "c"), ("a", "c")]);
        g.edges.push(Edge {
            from: NodeId(0),
            to: NodeId(1),
            refs: Vec::new(),
            count: None,
        });
        let metrics = compute_metrics(&g, &[]);

        let keys: Vec<&str> = metrics.keys().map(String::as_str).collect();
        assert_eq!(keys, ["a", "b", "c"]); // BTreeMap → name order
        assert_eq!(
            metrics["a"],
            ModuleMetrics {
                fan_in: 0,
                fan_out: 2,
                instability: 1.0,
                in_cycle: false,
            }
        );
        assert_eq!(
            metrics["b"],
            ModuleMetrics {
                fan_in: 1,
                fan_out: 1,
                instability: 0.5,
                in_cycle: false,
            }
        );
        assert_eq!(
            metrics["c"],
            ModuleMetrics {
                fan_in: 2,
                fan_out: 0,
                instability: 0.0,
                in_cycle: false,
            }
        );
    }

    #[test]
    fn isolated_nodes_and_visible_modules_have_metrics() {
        // `module` is the only non-File node: no entry, and its incoming
        // edge (a hand-assembled violation of the file-only edge rule)
        // must not influence any metric.
        let mut g = graph(&["a", "lonely"], &[("a", "a")]);
        g.nodes.push(Node {
            id: NodeId(2),
            name: "module".to_string(),
            kind: NodeKind::Module,
            parent: None,
            lang: None,
        });
        g.edges.push(Edge {
            from: NodeId(0),
            to: NodeId(2),
            refs: Vec::new(),
            count: None,
        });
        let metrics = compute_metrics(&g, &[]);

        assert_eq!(metrics.len(), 3);
        assert_eq!(metrics["module"].fan_in, 1);
        assert_eq!(
            metrics["lonely"],
            ModuleMetrics {
                fan_in: 0,
                fan_out: 0,
                instability: 0.0,
                in_cycle: false,
            }
        );
        // The self-loop a → a counts one in *and* one out.
        assert_eq!(
            metrics["a"],
            ModuleMetrics {
                fan_in: 1,
                fan_out: 2,
                instability: 2.0 / 3.0,
                in_cycle: false,
            }
        );
    }

    #[test]
    fn scc_members_are_marked_in_cycle() {
        // Two-cycle a ↔ b plus the outside node c and its edge b → c.
        let g = graph(&["a", "b", "c"], &[("a", "b"), ("b", "a"), ("b", "c")]);
        let sccs = find_sccs(&g);
        assert_eq!(sccs.len(), 1);
        let metrics = compute_metrics(&g, &sccs);

        assert!(metrics["a"].in_cycle);
        assert!(metrics["b"].in_cycle);
        assert!(!metrics["c"].in_cycle);
        // c is only a sink; its edge does not make b cyclic-external.
        assert_eq!(metrics["c"].fan_in, 1);
        assert_eq!(metrics["c"].fan_out, 0);
        assert_eq!(metrics["c"].instability, 0.0);
    }

    #[test]
    fn instability_uses_exact_f32_ratios() {
        // a → b, a → c, a → d: pure source, then pure sinks.
        let g = graph(&["a", "b", "c", "d"], &[("a", "b"), ("a", "c"), ("a", "d")]);
        let metrics = compute_metrics(&g, &[]);
        assert_eq!(metrics["a"].instability, 1.0);
        assert_eq!(metrics["b"].instability, 0.0);

        // a has fan_in 2 (b → a, c → a) and fan_out 1 (a → b): 1/3 is
        // not exactly representable — compare the identical division,
        // not a decimal literal.
        let g = graph(&["a", "b", "c"], &[("b", "a"), ("c", "a"), ("a", "b")]);
        let metrics = compute_metrics(&g, &[]);
        assert_eq!(metrics["a"].instability, 1.0f32 / 3.0f32);
    }

    #[test]
    fn edges_with_unresolvable_endpoints_are_skipped() {
        let mut g = graph(&["a", "b"], &[("a", "b")]);
        g.edges.push(Edge {
            from: NodeId(99),
            to: NodeId(0),
            refs: Vec::new(),
            count: None,
        });
        g.edges.push(Edge {
            from: NodeId(0),
            to: NodeId(99),
            refs: Vec::new(),
            count: None,
        });
        let metrics = compute_metrics(&g, &[]);

        assert_eq!(metrics["a"].fan_out, 1);
        assert_eq!(metrics["a"].fan_in, 0);
        assert_eq!(metrics["b"].fan_in, 1);
        assert_eq!(metrics["b"].fan_out, 0);
    }

    #[test]
    fn summarize_counts_nodes_edges_sccs_and_largest() {
        // Two SCCs: {a, b} (size 2) and {c, d, e} (size 3), plus isolated f.
        let g = graph(
            &["a", "b", "c", "d", "e", "f"],
            &[
                ("a", "b"),
                ("b", "a"),
                ("c", "d"),
                ("d", "e"),
                ("e", "c"),
                ("b", "c"),
            ],
        );
        let sccs = find_sccs(&g);
        assert_eq!(sccs.len(), 2);

        let summary = summarize(&g, &sccs, &[]);
        assert_eq!(summary.nodes, 6);
        assert_eq!(summary.edges, 6);
        assert_eq!(summary.sccs, 2);
        assert_eq!(summary.largest_scc, 3);
        assert_eq!(summary.layer_violations, 0);
        assert_eq!(summary.status, Status::Pass);
    }

    #[test]
    fn summarize_status_fails_only_on_error_findings() {
        let g = graph(&["a"], &[]);
        let sccs: Vec<Scc> = Vec::new();

        let summary = summarize(
            &g,
            &sccs,
            &[
                finding(FindingKind::Cycle, Severity::Warning),
                finding(FindingKind::MetricThreshold, Severity::Info),
            ],
        );
        assert_eq!(summary.status, Status::Pass);

        let summary = summarize(&g, &sccs, &[finding(FindingKind::Cycle, Severity::Error)]);
        assert_eq!(summary.status, Status::Fail);
    }

    #[test]
    fn summarize_counts_layer_violations_only() {
        let g = graph(&["a"], &[]);
        let summary = summarize(
            &g,
            &[],
            &[
                finding(FindingKind::LayerViolation, Severity::Error),
                finding(FindingKind::LayerViolation, Severity::Info),
                finding(FindingKind::Cycle, Severity::Error),
                finding(FindingKind::MetricThreshold, Severity::Warning),
            ],
        );
        assert_eq!(summary.layer_violations, 2);
        assert_eq!(summary.status, Status::Fail);
    }
}
