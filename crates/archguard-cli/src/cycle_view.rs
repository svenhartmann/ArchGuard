//! Cycle findings of a report and the graph edges inside them, shared by views.
use std::collections::{HashMap, HashSet};

use archguard_engine::core::{Finding, FindingKind, Graph, NodeId, Report};

/// The cycle findings of a report, in the report's stable order.
pub(crate) fn cycle_findings(report: &Report) -> impl Iterator<Item = &Finding> {
    report
        .findings
        .iter()
        .filter(|finding| finding.kind == FindingKind::Cycle)
}

/// The `(from, to)` [`NodeId`] pairs of every edge whose endpoints are
/// both members of one of the shown cycle findings — the set
/// [`to_mermaid`] styles red.
pub(crate) fn cyclic_edges(graph: &Graph, shown: &[&Finding]) -> HashSet<(NodeId, NodeId)> {
    let name_of: HashMap<NodeId, &str> = graph
        .nodes
        .iter()
        .map(|node| (node.id, node.name.as_str()))
        .collect();
    let mut edges = HashSet::new();
    for finding in shown {
        let members: HashSet<&str> = finding.nodes.iter().map(String::as_str).collect();
        for edge in &graph.edges {
            let inside = |id: NodeId| name_of.get(&id).is_some_and(|name| members.contains(name));
            if inside(edge.from) && inside(edge.to) {
                edges.insert((edge.from, edge.to));
            }
        }
    }
    edges
}
