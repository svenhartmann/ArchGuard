//! GraphML rendering of [`Graph`] (T34).
//!
//! The GraphML 1.0 document declares four keys — `name`, `lang` and
//! `in_cycle` for nodes, `ref_count` for edges — and carries them as
//! `<data>` values. `focus`/`depth` select the subgraph exactly like
//! [`crate::to_mermaid`] (see [`crate::render::focus_subset`]); `in_cycle`
//! marks every node that is an endpoint of one of the `cyclic_edges`
//! pairs (the nodes inside an SCC). A node without a `lang` simply omits
//! the `lang` data element (GraphML treats a missing value as the key's
//! default).
//!
//! Node ids are `n<internal NodeId>` (unique by construction); edge ids
//! are `e<counter>` numbered in edge emission order, so both are
//! deterministic across runs. All interpolated values are XML-escaped.

use std::collections::HashSet;

use archguard_core::{Graph, NodeId};

use crate::render::{escape_xml, focus_subset, nodes_in_cycles};

/// Render `g` as a GraphML 1.0 document.
///
/// `focus`/`depth` select the subgraph as described in the module docs
/// (`focus` matches a node by *name*); `cyclic_edges` holds the
/// `(from, to)` [`NodeId`] pairs of the edges inside SCCs, which mark the
/// `in_cycle` node attribute. The output ends with a trailing newline and
/// is deterministic for equal inputs.
#[tracing::instrument(skip_all)]
pub fn to_graphml(
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
    let in_cycle = nodes_in_cycles(cyclic_edges);

    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <graphml xmlns=\"http://graphml.graphdrawing.org/xmlns\">\n",
    );
    for (key, for_whom, name, ty) in [
        ("d_name", "node", "name", "string"),
        ("d_lang", "node", "lang", "string"),
        ("d_in_cycle", "node", "in_cycle", "boolean"),
        ("d_ref_count", "edge", "ref_count", "int"),
    ] {
        out.push_str(&format!(
            "  <key id=\"{key}\" for=\"{for_whom}\" attr.name=\"{name}\" attr.type=\"{ty}\"/>\n"
        ));
    }
    out.push_str("  <graph edgedefault=\"directed\">\n");
    for node in &visible {
        out.push_str(&format!("    <node id=\"n{}\">\n", node.id.0));
        out.push_str(&format!(
            "      <data key=\"d_name\">{}</data>\n",
            escape_xml(&node.name)
        ));
        if let Some(lang) = &node.lang {
            out.push_str(&format!(
                "      <data key=\"d_lang\">{}</data>\n",
                escape_xml(lang)
            ));
        }
        out.push_str(&format!(
            "      <data key=\"d_in_cycle\">{}</data>\n    </node>\n",
            in_cycle.contains(&node.id)
        ));
    }
    // Edge ids count only *rendered* edges (both endpoints visible), in
    // graph edge order — the deterministic counterpart of mermaid's
    // `linkStyle` index.
    let mut edge_id = 0usize;
    for edge in &g.edges {
        if !visible.iter().any(|n| n.id == edge.from) || !visible.iter().any(|n| n.id == edge.to) {
            continue;
        }
        out.push_str(&format!(
            "    <edge id=\"e{edge_id}\" source=\"n{}\" target=\"n{}\">\n",
            edge.from.0, edge.to.0
        ));
        out.push_str(&format!(
            "      <data key=\"d_ref_count\">{}</data>\n    </edge>\n",
            edge.weight()
        ));
        edge_id += 1;
    }
    out.push_str("  </graph>\n</graphml>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::to_graphml;
    use archguard_core::{Edge, Graph, Node, NodeId, NodeKind, Ref};
    use std::collections::HashSet;

    fn file(id: u32, name: &str) -> Node {
        Node {
            id: NodeId(id),
            name: name.to_string(),
            kind: NodeKind::File,
            parent: None,
            lang: Some("ts".to_string()),
        }
    }

    /// Edge with `n` distinct import sites — its `ref_count`.
    fn edge_with(from: u32, to: u32, n: usize) -> Edge {
        Edge {
            from: NodeId(from),
            to: NodeId(to),
            refs: (0..n)
                .map(|k| Ref {
                    file: format!("{from}.ts"),
                    line: k as u32 + 1,
                    raw: "import …".to_string(),
                })
                .collect(),
            // Builder-shaped: the total equals the (complete) sample.
            count: Some(n as u64),
        }
    }

    fn edge(from: u32, to: u32) -> Edge {
        edge_with(from, to, 1)
    }

    /// The shared 5-node fixture (same shape as the dsm tests): one
    /// 2-cycle `a ↔ b` plus one-way tails `a → c`, `c → d`, `b → e`,
    /// `d → e`; `a → b` carries 2 refs, `c → d` 3. Ids: a=0 … e=4.
    fn sample_graph() -> Graph {
        Graph {
            review_kpis: None,
            semantic: None,
            schema_version: 1,
            root: "/abs/projekt".to_string(),
            nodes: vec![
                file(0, "a"),
                file(1, "b"),
                file(2, "c"),
                file(3, "d"),
                file(4, "e"),
            ],
            edges: vec![
                edge_with(0, 1, 2),
                edge(1, 0),
                edge(0, 2),
                edge_with(2, 3, 3),
                edge(1, 4),
                edge(3, 4),
            ],
        }
    }

    fn cyclic_edges() -> HashSet<(NodeId, NodeId)> {
        [(NodeId(0), NodeId(1)), (NodeId(1), NodeId(0))]
            .into_iter()
            .collect()
    }

    #[test]
    fn to_graphml_full_graph_matches_golden() {
        assert_eq!(
            to_graphml(&sample_graph(), None, 0, &cyclic_edges()),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <graphml xmlns=\"http://graphml.graphdrawing.org/xmlns\">\n\
             \x20\x20<key id=\"d_name\" for=\"node\" attr.name=\"name\" attr.type=\"string\"/>\n\
             \x20\x20<key id=\"d_lang\" for=\"node\" attr.name=\"lang\" attr.type=\"string\"/>\n\
             \x20\x20<key id=\"d_in_cycle\" for=\"node\" attr.name=\"in_cycle\" attr.type=\"boolean\"/>\n\
             \x20\x20<key id=\"d_ref_count\" for=\"edge\" attr.name=\"ref_count\" attr.type=\"int\"/>\n\
             \x20\x20<graph edgedefault=\"directed\">\n\
             \x20\x20\x20\x20<node id=\"n0\">\n\
             \x20\x20\x20\x20\x20\x20<data key=\"d_name\">a</data>\n\
             \x20\x20\x20\x20\x20\x20<data key=\"d_lang\">ts</data>\n\
             \x20\x20\x20\x20\x20\x20<data key=\"d_in_cycle\">true</data>\n\
             \x20\x20\x20\x20</node>\n\
             \x20\x20\x20\x20<node id=\"n1\">\n\
             \x20\x20\x20\x20\x20\x20<data key=\"d_name\">b</data>\n\
             \x20\x20\x20\x20\x20\x20<data key=\"d_lang\">ts</data>\n\
             \x20\x20\x20\x20\x20\x20<data key=\"d_in_cycle\">true</data>\n\
             \x20\x20\x20\x20</node>\n\
             \x20\x20\x20\x20<node id=\"n2\">\n\
             \x20\x20\x20\x20\x20\x20<data key=\"d_name\">c</data>\n\
             \x20\x20\x20\x20\x20\x20<data key=\"d_lang\">ts</data>\n\
             \x20\x20\x20\x20\x20\x20<data key=\"d_in_cycle\">false</data>\n\
             \x20\x20\x20\x20</node>\n\
             \x20\x20\x20\x20<node id=\"n3\">\n\
             \x20\x20\x20\x20\x20\x20<data key=\"d_name\">d</data>\n\
             \x20\x20\x20\x20\x20\x20<data key=\"d_lang\">ts</data>\n\
             \x20\x20\x20\x20\x20\x20<data key=\"d_in_cycle\">false</data>\n\
             \x20\x20\x20\x20</node>\n\
             \x20\x20\x20\x20<node id=\"n4\">\n\
             \x20\x20\x20\x20\x20\x20<data key=\"d_name\">e</data>\n\
             \x20\x20\x20\x20\x20\x20<data key=\"d_lang\">ts</data>\n\
             \x20\x20\x20\x20\x20\x20<data key=\"d_in_cycle\">false</data>\n\
             \x20\x20\x20\x20</node>\n\
             \x20\x20\x20\x20<edge id=\"e0\" source=\"n0\" target=\"n1\">\n\
             \x20\x20\x20\x20\x20\x20<data key=\"d_ref_count\">2</data>\n\
             \x20\x20\x20\x20</edge>\n\
             \x20\x20\x20\x20<edge id=\"e1\" source=\"n1\" target=\"n0\">\n\
             \x20\x20\x20\x20\x20\x20<data key=\"d_ref_count\">1</data>\n\
             \x20\x20\x20\x20</edge>\n\
             \x20\x20\x20\x20<edge id=\"e2\" source=\"n0\" target=\"n2\">\n\
             \x20\x20\x20\x20\x20\x20<data key=\"d_ref_count\">1</data>\n\
             \x20\x20\x20\x20</edge>\n\
             \x20\x20\x20\x20<edge id=\"e3\" source=\"n2\" target=\"n3\">\n\
             \x20\x20\x20\x20\x20\x20<data key=\"d_ref_count\">3</data>\n\
             \x20\x20\x20\x20</edge>\n\
             \x20\x20\x20\x20<edge id=\"e4\" source=\"n1\" target=\"n4\">\n\
             \x20\x20\x20\x20\x20\x20<data key=\"d_ref_count\">1</data>\n\
             \x20\x20\x20\x20</edge>\n\
             \x20\x20\x20\x20<edge id=\"e5\" source=\"n3\" target=\"n4\">\n\
             \x20\x20\x20\x20\x20\x20<data key=\"d_ref_count\">1</data>\n\
             \x20\x20\x20\x20</edge>\n\
             \x20\x20</graph>\n\
             </graphml>\n"
        );
    }

    #[test]
    fn to_graphml_focus_selects_bfs_subgraph() {
        let output = to_graphml(&sample_graph(), Some("a"), 1, &cyclic_edges());
        assert!(output.contains("<node id=\"n0\">"));
        assert!(output.contains("<node id=\"n2\">")); // a → c, one hop
        assert!(!output.contains("<node id=\"n3\">")); // d is two hops away
        // Only edges between kept nodes survive, renumbered from 0.
        assert!(output.contains("id=\"e0\" source=\"n0\" target=\"n1\""));
        assert!(output.contains("id=\"e1\" source=\"n1\" target=\"n0\""));
        assert!(output.contains("id=\"e2\" source=\"n0\" target=\"n2\""));
        assert!(!output.contains("e3"));
    }

    #[test]
    fn to_graphml_escapes_values_and_omits_missing_lang() {
        let mut graph = sample_graph();
        graph.nodes[0].name = "a&b<c>d\"e'f".to_string();
        graph.nodes[1].lang = None;
        let output = to_graphml(&graph, None, 0, &cyclic_edges());
        assert!(output.contains("a&amp;b&lt;c&gt;d&quot;e&apos;f"));
        assert!(
            !output.contains("<data key=\"d_name\">b</data>\n      <data key=\"d_lang\">"),
            "missing lang must be omitted: {output}"
        );
        // The other nodes still carry their lang.
        assert!(
            output.contains("<data key=\"d_name\">c</data>\n      <data key=\"d_lang\">ts</data>")
        );
    }
}
