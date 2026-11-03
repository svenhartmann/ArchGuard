//! Shared helpers for the graph formats ([`crate::to_mermaid`],
//! [`crate::to_dot`], [`crate::to_graphml`], [`crate::to_gexf`],
//! [`crate::to_html`]).
//!
//! Both formats render the same two ingredients and differ only in syntax:
//! the set of nodes kept by a focus selection ([`focus_subset`]) and a
//! stable, collision-free id per node ([`unique_ids`]).

use std::collections::{HashMap, HashSet};

use archguard_core::{Graph, Node, NodeId};

/// Nodes the renderers must draw for a `focus`/`depth` selection.
///
/// `None` means "render everything". `Some(set)` means "render exactly the
/// nodes in `set`": an undirected BFS starting at the node whose `name`
/// equals `focus`, expanding along both incoming and outgoing [`Edge`]s
/// for at most `depth` hops (the focus node itself is always included, so
/// `depth == 0` yields just it). A focus that matches no node yields the
/// empty set — the renderers then draw an empty diagram.
///
/// `focus` matches the node *name* (the source-root-relative module path
/// used everywhere outside the core internals), not the internal
/// [`NodeId`]; if several nodes shared a name, the first in graph order
/// wins.
pub(crate) fn focus_subset(g: &Graph, focus: Option<&str>, depth: u32) -> Option<HashSet<NodeId>> {
    let name = focus?;
    let mut keep = HashSet::new();
    let Some(&start) = g.nodes.iter().find(|n| n.name == name).map(|n| &n.id) else {
        return Some(keep); // unknown focus: nothing to render
    };
    keep.insert(start);

    // Undirected adjacency built from the dependency edges only; the
    // containment tree (`Node::parent`) is not a dependency and stays out.
    let mut adjacency: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for edge in &g.edges {
        adjacency.entry(edge.from).or_default().push(edge.to);
        adjacency.entry(edge.to).or_default().push(edge.from);
    }

    let mut frontier = vec![start];
    for _ in 0..depth {
        let mut next: Vec<NodeId> = Vec::new();
        for &current in &frontier {
            for &neighbor in adjacency.get(&current).into_iter().flatten() {
                if keep.insert(neighbor) {
                    next.push(neighbor);
                }
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    Some(keep)
}

/// A unique, valid mermaid/DOT id per node: [`sanitize_id`] on the node
/// name, disambiguated with `_2`, `_3`, … on collisions (e.g. `a/b` and
/// `a_b` both reduce to `a_b`). Ids are assigned in graph node order, so
/// the mapping is deterministic across runs.
pub(crate) fn unique_ids<'a>(nodes: impl Iterator<Item = &'a Node>) -> HashMap<NodeId, String> {
    let mut used: HashSet<String> = HashSet::new();
    let mut ids: HashMap<NodeId, String> = HashMap::new();
    for node in nodes {
        let base = sanitize_id(&node.name);
        let mut candidate = base.clone();
        let mut suffix = 2;
        while !used.insert(candidate.clone()) {
            candidate = format!("{base}_{suffix}");
            suffix += 1;
        }
        ids.insert(node.id, candidate);
    }
    ids
}

/// The nodes that appear as an endpoint of any `(from, to)` pair in
/// `cyclic_edges` — the nodes inside some SCC. Used by the GraphML, GEXF
/// and HTML renderers for their `in_cycle` node attribute.
pub(crate) fn nodes_in_cycles(cyclic_edges: &HashSet<(NodeId, NodeId)>) -> HashSet<NodeId> {
    cyclic_edges
        .iter()
        .flat_map(|(from, to)| [*from, *to])
        .collect()
}

/// Escape the five XML predefined entities (`&`, `<`, `>`, `"`, `'`) in
/// an interpolated value, so it is safe both in a quoted attribute and as
/// element text content. Used by the GraphML and GEXF renderers.
pub(crate) fn escape_xml(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Map every non-ASCII-alphanumeric character of `name` to `_` so the
/// result is a safe identifier in both mermaid and DOT. A result that is
/// empty or starts with a digit gets an `n` prefix (DOT would otherwise
/// read it as a number). The real name is never lost: the renderers carry
/// it as the node label.
fn sanitize_id(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if out.is_empty() || out.as_bytes()[0].is_ascii_digit() {
        out.insert(0, 'n');
    }
    out
}
