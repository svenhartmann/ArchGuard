//! Mermaid `flowchart` rendering of [`Graph`] (T20).
//!
//! Without a focus the diagram shows every node and every dependency
//! edge of the [`Graph`] (the containment tree is not a dependency and is
//! not drawn). With a focus, only the nodes an undirected BFS from the
//! focus node reaches within `depth` hops are shown, together with the
//! edges between them — see [`crate::render::focus_subset`] for the exact
//! selection, including the empty-diagram behaviour for an unknown focus
//! name.
//!
//! Node names are sanitized into valid mermaid ids (`a/b/c.ts` → `a_b_c_ts`,
//! collisions disambiguated with `_2`, …) while the real name stays the
//! quoted label. Edges listed in `cyclic_edges` — the pairs inside any
//! SCC, keyed by internal [`NodeId`]s — are styled red and bold via
//! `linkStyle`.

use std::collections::HashSet;

use archguard_core::{Graph, NodeId};

use crate::render::{focus_subset, unique_ids};

/// Red used for edges inside an SCC.
const CYCLE_COLOR: &str = "#cc0000";
/// Stroke width of a cycle edge, versus the mermaid default of 1–2px.
const CYCLE_STROKE_WIDTH: &str = "2px";

/// Render `g` as a mermaid `flowchart TD` document.
///
/// `focus`/`depth` select the subgraph as described in the module docs
/// (`focus` matches a node by *name*); `cyclic_edges` holds the
/// `(from, to)` [`NodeId`] pairs of the edges inside SCCs and styles them
/// red and bold. The output ends with a trailing newline and is
/// deterministic for equal inputs.
#[tracing::instrument(skip_all)]
pub fn to_mermaid(
    g: &Graph,
    focus: Option<&str>,
    depth: u32,
    cyclic_edges: &HashSet<(NodeId, NodeId)>,
) -> String {
    let keep = focus_subset(g, focus, depth);
    let visible: Vec<&_> = g
        .nodes
        .iter()
        .filter(|n| keep.as_ref().is_none_or(|set| set.contains(&n.id)))
        .collect();
    let ids = unique_ids(visible.iter().copied());

    let mut out = String::from("flowchart TD\n");
    for node in &visible {
        out.push_str(&format!(
            "  {}[\"{}\"]\n",
            ids[&node.id],
            escape_label(&node.name)
        ));
    }
    // `linkStyle` indexes links in emission order, so the index must be
    // counted over the rendered edges, not over `g.edges`. The directives
    // are collected and emitted after all links, the customary placement.
    let mut cycle_links: Vec<usize> = Vec::new();
    let mut link_index = 0usize;
    for edge in &g.edges {
        if !visible.iter().any(|n| n.id == edge.from) || !visible.iter().any(|n| n.id == edge.to) {
            continue;
        }
        out.push_str(&format!("  {} --> {}\n", ids[&edge.from], ids[&edge.to]));
        if cyclic_edges.contains(&(edge.from, edge.to)) {
            cycle_links.push(link_index);
        }
        link_index += 1;
    }
    for index in cycle_links {
        out.push_str(&format!(
            "  linkStyle {index} stroke:{CYCLE_COLOR},stroke-width:{CYCLE_STROKE_WIDTH};\n"
        ));
    }
    out
}

/// Escape a node name for the quoted mermaid label: quotes become the
/// HTML entity, which mermaid renders inside `"…"`.
fn escape_label(name: &str) -> String {
    name.replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::{CYCLE_COLOR, CYCLE_STROKE_WIDTH, to_mermaid};
    use archguard_core::{Edge, Graph, Node, NodeId, NodeKind, Ref};

    fn file(id: u32, name: &str) -> Node {
        Node {
            id: NodeId(id),
            name: name.to_string(),
            kind: NodeKind::File,
            parent: None,
            lang: Some("ts".to_string()),
        }
    }

    fn edge(from: u32, to: u32) -> Edge {
        Edge {
            from: NodeId(from),
            to: NodeId(to),
            refs: vec![Ref {
                file: format!("{from}.ts"),
                line: 1,
                raw: "import …".to_string(),
            }],
            // Builder-shaped: the total equals the (complete) sample.
            count: Some(1),
        }
    }

    /// The golden shape of T20: five file nodes with exactly one cycle
    /// `a.ts → b.ts → c.ts → a.ts` plus the tail `c.ts → d.ts → e.ts`.
    /// Edges are in canonical `(from, to)` order.
    fn sample_graph() -> Graph {
        Graph {
            review_kpis: None,
            semantic: None,
            schema_version: 1,
            root: "/abs/projekt".to_string(),
            nodes: vec![
                file(0, "a.ts"),
                file(1, "b.ts"),
                file(2, "c.ts"),
                file(3, "d.ts"),
                file(4, "e.ts"),
            ],
            edges: vec![edge(0, 1), edge(1, 2), edge(2, 0), edge(2, 3), edge(3, 4)],
        }
    }

    fn cyclic_edges() -> std::collections::HashSet<(NodeId, NodeId)> {
        [
            (NodeId(0), NodeId(1)),
            (NodeId(1), NodeId(2)),
            (NodeId(2), NodeId(0)),
        ]
        .into_iter()
        .collect()
    }

    #[test]
    fn to_mermaid_full_graph_matches_golden() {
        assert_eq!(
            to_mermaid(&sample_graph(), None, 0, &cyclic_edges()),
            "flowchart TD\n  \
             a_ts[\"a.ts\"]\n  \
             b_ts[\"b.ts\"]\n  \
             c_ts[\"c.ts\"]\n  \
             d_ts[\"d.ts\"]\n  \
             e_ts[\"e.ts\"]\n  \
             a_ts --> b_ts\n  \
             b_ts --> c_ts\n  \
             c_ts --> a_ts\n  \
             c_ts --> d_ts\n  \
             d_ts --> e_ts\n  \
             linkStyle 0 stroke:#cc0000,stroke-width:2px;\n  \
             linkStyle 1 stroke:#cc0000,stroke-width:2px;\n  \
             linkStyle 2 stroke:#cc0000,stroke-width:2px;\n"
        );
    }

    #[test]
    fn to_mermaid_marks_only_the_scc_edges_red_and_bold() {
        let output = to_mermaid(&sample_graph(), None, 0, &cyclic_edges());
        // Three cycle links, two plain links.
        assert_eq!(output.matches("linkStyle").count(), 3);
        assert!(output.contains(&format!(
            "stroke:{CYCLE_COLOR},stroke-width:{CYCLE_STROKE_WIDTH};"
        )));
        // Only links 0–2 (the SCC) are styled; the two tail links are not.
        assert!(output.contains("linkStyle 2 "));
        assert!(!output.contains("linkStyle 3"));
        assert!(!output.contains("linkStyle 4"));
        // An empty SCC set styles nothing.
        assert!(!to_mermaid(&sample_graph(), None, 0, &Default::default()).contains("linkStyle"));
    }

    #[test]
    fn to_mermaid_focus_renders_bfs_subgraph_in_both_directions() {
        // From b.ts, one hop reaches a.ts and c.ts along either direction
        // — including the a→b and c→a edges that "point at" the focus.
        assert_eq!(
            to_mermaid(&sample_graph(), Some("b.ts"), 1, &cyclic_edges()),
            "flowchart TD\n  \
             a_ts[\"a.ts\"]\n  \
             b_ts[\"b.ts\"]\n  \
             c_ts[\"c.ts\"]\n  \
             a_ts --> b_ts\n  \
             b_ts --> c_ts\n  \
             c_ts --> a_ts\n  \
             linkStyle 0 stroke:#cc0000,stroke-width:2px;\n  \
             linkStyle 1 stroke:#cc0000,stroke-width:2px;\n  \
             linkStyle 2 stroke:#cc0000,stroke-width:2px;\n"
        );
    }

    #[test]
    fn to_mermaid_focus_depth_limits_hops_and_zero_yields_the_focus_alone() {
        let depth2 = to_mermaid(&sample_graph(), Some("a.ts"), 2, &cyclic_edges());
        assert!(depth2.contains("d_ts[\"d.ts\"]"));
        assert!(!depth2.contains("e_ts"));
        assert_eq!(
            to_mermaid(&sample_graph(), Some("a.ts"), 0, &cyclic_edges()),
            "flowchart TD\n  a_ts[\"a.ts\"]\n"
        );
    }

    #[test]
    fn to_mermaid_unknown_focus_renders_empty_diagram() {
        assert_eq!(
            to_mermaid(&sample_graph(), Some("nope.ts"), 3, &cyclic_edges()),
            "flowchart TD\n"
        );
    }

    #[test]
    fn to_mermaid_sanitizes_names_into_unique_ids_and_keeps_labels() {
        let graph = Graph {
            semantic: None,
            review_kpis: None,
            schema_version: 1,
            root: "/abs".to_string(),
            nodes: vec![
                file(0, "a/b"),
                file(1, "a_b"),
                file(2, "1.ts"),
                file(3, "we\"ird.ts"),
            ],
            edges: Vec::new(),
        };
        assert_eq!(
            to_mermaid(&graph, None, 0, &Default::default()),
            "flowchart TD\n  \
             a_b[\"a/b\"]\n  \
             a_b_2[\"a_b\"]\n  \
             n1_ts[\"1.ts\"]\n  \
             we_ird_ts[\"we&quot;ird.ts\"]\n"
        );
    }

    #[test]
    fn to_mermaid_is_deterministic_and_newline_terminated() {
        let a = to_mermaid(&sample_graph(), None, 0, &cyclic_edges());
        assert_eq!(a, to_mermaid(&sample_graph(), None, 0, &cyclic_edges()));
        assert!(a.ends_with('\n'));
    }
}
