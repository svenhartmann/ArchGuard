//! Layout arguments of the T38 DSM renderers, shared by every caller.
//!
//! It lives in the analysis crate, next to [`crate::scc`], because the
//! order is an SCC condensation and the report crate must not know about
//! SCCs. CLI and MCP server both reach it through
//! `archguard_engine::analysis`, the same path they already use for
//! [`find_sccs`].

use std::collections::hash_map::Entry;
use std::collections::{BTreeSet, HashMap, HashSet};

use archguard_core::{Graph, NodeId};

use crate::scc::{Scc, find_sccs};

/// Precomputed arguments of the T38 DSM renderers: the reverse
/// topological order of the condensation (sinks first, cycle-group
/// members adjacent) and the set of edges inside cyclic components.
///
/// Groups are the cyclic SCCs plus one singleton per remaining node.
/// Kahn runs on the condensation's OUT-degree, so groups nothing
/// depends on (sinks) come first; ties break on the smallest member
/// name, then on group index, keeping the order deterministic. The
/// condensation is a DAG, so the loop always empties; a defensive tail
/// keeps stray ids from vanishing.
pub fn dsm_layout(view: &Graph) -> (Vec<NodeId>, HashSet<(NodeId, NodeId)>) {
    // Kosaraju runs once: the groups and the cyclic edges share the result.
    let sccs = find_sccs(view);
    let (groups, group_of) = condensation_groups(view, &sccs);

    // Condensation: group → groups it depends on.
    let mut depends: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); groups.len()];
    for edge in &view.edges {
        if let (Some(&a), Some(&b)) = (group_of.get(&edge.from), group_of.get(&edge.to))
            && a != b
        {
            depends[a].insert(b);
        }
    }
    let mut pending: Vec<usize> = depends.iter().map(|set| set.len()).collect();

    let sort_key = |gid: usize| -> (String, usize) {
        let smallest = groups[gid]
            .iter()
            .filter_map(|id| view.node_name(*id))
            .min()
            .unwrap_or("?")
            .to_string();
        (smallest, gid)
    };

    let mut emitted = vec![false; groups.len()];
    let mut order: Vec<NodeId> = Vec::new();
    loop {
        let next = (0..groups.len())
            .filter(|&g| !emitted[g] && pending[g] == 0)
            .map(|g| (sort_key(g), g))
            .min()
            .map(|(_, g)| g);
        let Some(gid) = next else { break };
        emitted[gid] = true;
        order.extend(groups[gid].iter().copied());
        for p in 0..groups.len() {
            if !emitted[p] && depends[p].contains(&gid) {
                pending[p] -= 1;
            }
        }
    }
    // Defensive tail (unreachable for a real DAG condensation).
    for gid in 0..groups.len() {
        if !emitted[gid] {
            order.extend(groups[gid].iter().copied());
        }
    }

    let cyclic: HashSet<(NodeId, NodeId)> = sccs.iter().flat_map(|s| s.edges.clone()).collect();
    (order, cyclic)
}

/// The groups the order is built from — the cyclic SCCs first, then one
/// singleton per remaining node — and the group index of every node.
fn condensation_groups(view: &Graph, sccs: &[Scc]) -> (Vec<Vec<NodeId>>, HashMap<NodeId, usize>) {
    let mut groups: Vec<Vec<NodeId>> = Vec::new();
    let mut group_of: HashMap<NodeId, usize> = HashMap::new();
    for scc in sccs {
        let gid = groups.len();
        for &id in &scc.node_ids {
            group_of.insert(id, gid);
        }
        groups.push(scc.node_ids.clone());
    }
    for node in &view.nodes {
        if let Entry::Vacant(entry) = group_of.entry(node.id) {
            let gid = groups.len();
            entry.insert(gid);
            groups.push(vec![node.id]);
        }
    }
    (groups, group_of)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_helpers::graph;

    #[test]
    fn sinks_come_first_and_a_cycle_free_view_reports_no_cyclic_edge() {
        let (order, cyclic) = dsm_layout(&graph(&["app", "domain"], &[("app", "domain")]));
        assert_eq!(order, vec![NodeId(1), NodeId(0)], "domain (sink) first");
        assert!(cyclic.is_empty(), "the view itself has no cycle");
    }

    #[test]
    fn a_cycle_keeps_its_members_adjacent_and_reports_its_inner_edges() {
        let view = graph(
            &["a", "b", "sink"],
            &[("a", "b"), ("b", "a"), ("a", "sink")],
        );
        let (order, cyclic) = dsm_layout(&view);
        assert_eq!(order, vec![NodeId(2), NodeId(0), NodeId(1)]);
        assert!(cyclic.contains(&(NodeId(0), NodeId(1))));
        assert!(cyclic.contains(&(NodeId(1), NodeId(0))));
    }

    #[test]
    fn ties_break_on_the_smallest_member_name_so_the_order_is_deterministic() {
        // Two independent sinks: the name decides, not the node id.
        let view = graph(&["zeta", "alpha"], &[]);
        assert_eq!(dsm_layout(&view).0, vec![NodeId(1), NodeId(0)]);
    }
}
