//! GEXF 1.3 rendering of [`Graph`] (T34).
//!
//! The document declares three node attributes (`name`, `lang`,
//! `in_cycle`) and one edge attribute (`ref_count`) in the `<attributes>`
//! sections and carries them as `<attvalue>` entries. `focus`/`depth`
//! select the subgraph exactly like [`crate::to_mermaid`] (see
//! [`crate::render::focus_subset`]); `in_cycle` marks every node that is
//! an endpoint of one of the `cyclic_edges` pairs (the nodes inside an
//! SCC). A node without a `lang` omits the `lang` attvalue.
//!
//! Node ids are `n<internal NodeId>` (unique by construction); GEXF
//! requires edge ids, so they are `e<counter>` numbered in edge
//! *emission* order (rendered edges only, graph edge order) — a plain
//! deterministic counter, no random ids. All interpolated values are
//! XML-escaped.

use std::collections::HashSet;

use archguard_core::{Graph, NodeId};

use crate::render::{escape_xml, focus_subset, nodes_in_cycles};

/// Render `g` as a GEXF 1.3 document.
///
/// `focus`/`depth` select the subgraph as described in the module docs
/// (`focus` matches a node by *name*); `cyclic_edges` holds the
/// `(from, to)` [`NodeId`] pairs of the edges inside SCCs, which mark the
/// `in_cycle` node attribute. The output ends with a trailing newline and
/// is deterministic for equal inputs.
#[tracing::instrument(skip_all)]
pub fn to_gexf(
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
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<gexf \
         xmlns=\"http://gexf.net/1.3\" version=\"1.3\">\n  <graph \
         mode=\"static\" defaultedgetype=\"directed\">\n",
    );
    out.push_str("    <attributes class=\"node\">\n");
    out.push_str("      <attribute id=\"name\" title=\"name\" type=\"string\"/>\n");
    out.push_str("      <attribute id=\"lang\" title=\"lang\" type=\"string\"/>\n");
    out.push_str("      <attribute id=\"in_cycle\" title=\"in_cycle\" type=\"boolean\"/>\n");
    out.push_str("    </attributes>\n");
    out.push_str("    <attributes class=\"edge\">\n");
    out.push_str("      <attribute id=\"ref_count\" title=\"ref_count\" type=\"integer\"/>\n");
    out.push_str("    </attributes>\n");
    out.push_str("    <nodes>\n");
    for node in &visible {
        out.push_str(&format!(
            "      <node id=\"n{}\" label=\"{}\">\n",
            node.id.0,
            escape_xml(&node.name)
        ));
        out.push_str("        <attvalues>\n");
        out.push_str(&format!(
            "          <attvalue for=\"name\" value=\"{}\"/>\n",
            escape_xml(&node.name)
        ));
        if let Some(lang) = &node.lang {
            out.push_str(&format!(
                "          <attvalue for=\"lang\" value=\"{}\"/>\n",
                escape_xml(lang)
            ));
        }
        out.push_str(&format!(
            "          <attvalue for=\"in_cycle\" value=\"{}\"/>\n",
            in_cycle.contains(&node.id)
        ));
        out.push_str("        </attvalues>\n      </node>\n");
    }
    out.push_str("    </nodes>\n    <edges>\n");
    // Edge ids are a deterministic counter over *rendered* edges (both
    // endpoints visible), in graph edge order — GEXF ids must be unique
    // and stable, so no random generation.
    let mut edge_id = 0usize;
    for edge in &g.edges {
        if !visible.iter().any(|n| n.id == edge.from) || !visible.iter().any(|n| n.id == edge.to) {
            continue;
        }
        out.push_str(&format!(
            "      <edge id=\"e{edge_id}\" source=\"n{}\" target=\"n{}\">\n",
            edge.from.0, edge.to.0
        ));
        out.push_str("        <attvalues>\n");
        out.push_str(&format!(
            "          <attvalue for=\"ref_count\" value=\"{}\"/>\n",
            edge.weight()
        ));
        out.push_str("        </attvalues>\n      </edge>\n");
        edge_id += 1;
    }
    out.push_str("    </edges>\n  </graph>\n</gexf>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::to_gexf;
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
    fn to_gexf_full_graph_matches_golden() {
        assert_eq!(
            to_gexf(&sample_graph(), None, 0, &cyclic_edges()),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <gexf xmlns=\"http://gexf.net/1.3\" version=\"1.3\">\n\
             \x20\x20<graph mode=\"static\" defaultedgetype=\"directed\">\n\
             \x20\x20\x20\x20<attributes class=\"node\">\n\
             \x20\x20\x20\x20\x20\x20<attribute id=\"name\" title=\"name\" type=\"string\"/>\n\
             \x20\x20\x20\x20\x20\x20<attribute id=\"lang\" title=\"lang\" type=\"string\"/>\n\
             \x20\x20\x20\x20\x20\x20<attribute id=\"in_cycle\" title=\"in_cycle\" type=\"boolean\"/>\n\
             \x20\x20\x20\x20</attributes>\n\
             \x20\x20\x20\x20<attributes class=\"edge\">\n\
             \x20\x20\x20\x20\x20\x20<attribute id=\"ref_count\" title=\"ref_count\" type=\"integer\"/>\n\
             \x20\x20\x20\x20</attributes>\n\
             \x20\x20\x20\x20<nodes>\n\
             \x20\x20\x20\x20\x20\x20<node id=\"n0\" label=\"a\">\n\
             \x20\x20\x20\x20\x20\x20\x20\x20<attvalues>\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20<attvalue for=\"name\" value=\"a\"/>\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20<attvalue for=\"lang\" value=\"ts\"/>\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20<attvalue for=\"in_cycle\" value=\"true\"/>\n\
             \x20\x20\x20\x20\x20\x20\x20\x20</attvalues>\n\
             \x20\x20\x20\x20\x20\x20</node>\n\
             \x20\x20\x20\x20\x20\x20<node id=\"n1\" label=\"b\">\n\
             \x20\x20\x20\x20\x20\x20\x20\x20<attvalues>\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20<attvalue for=\"name\" value=\"b\"/>\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20<attvalue for=\"lang\" value=\"ts\"/>\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20<attvalue for=\"in_cycle\" value=\"true\"/>\n\
             \x20\x20\x20\x20\x20\x20\x20\x20</attvalues>\n\
             \x20\x20\x20\x20\x20\x20</node>\n\
             \x20\x20\x20\x20\x20\x20<node id=\"n2\" label=\"c\">\n\
             \x20\x20\x20\x20\x20\x20\x20\x20<attvalues>\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20<attvalue for=\"name\" value=\"c\"/>\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20<attvalue for=\"lang\" value=\"ts\"/>\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20<attvalue for=\"in_cycle\" value=\"false\"/>\n\
             \x20\x20\x20\x20\x20\x20\x20\x20</attvalues>\n\
             \x20\x20\x20\x20\x20\x20</node>\n\
             \x20\x20\x20\x20\x20\x20<node id=\"n3\" label=\"d\">\n\
             \x20\x20\x20\x20\x20\x20\x20\x20<attvalues>\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20<attvalue for=\"name\" value=\"d\"/>\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20<attvalue for=\"lang\" value=\"ts\"/>\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20<attvalue for=\"in_cycle\" value=\"false\"/>\n\
             \x20\x20\x20\x20\x20\x20\x20\x20</attvalues>\n\
             \x20\x20\x20\x20\x20\x20</node>\n\
             \x20\x20\x20\x20\x20\x20<node id=\"n4\" label=\"e\">\n\
             \x20\x20\x20\x20\x20\x20\x20\x20<attvalues>\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20<attvalue for=\"name\" value=\"e\"/>\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20<attvalue for=\"lang\" value=\"ts\"/>\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20<attvalue for=\"in_cycle\" value=\"false\"/>\n\
             \x20\x20\x20\x20\x20\x20\x20\x20</attvalues>\n\
             \x20\x20\x20\x20\x20\x20</node>\n\
             \x20\x20\x20\x20</nodes>\n\
             \x20\x20\x20\x20<edges>\n\
             \x20\x20\x20\x20\x20\x20<edge id=\"e0\" source=\"n0\" target=\"n1\">\n\
             \x20\x20\x20\x20\x20\x20\x20\x20<attvalues>\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20<attvalue for=\"ref_count\" value=\"2\"/>\n\
             \x20\x20\x20\x20\x20\x20\x20\x20</attvalues>\n\
             \x20\x20\x20\x20\x20\x20</edge>\n\
             \x20\x20\x20\x20\x20\x20<edge id=\"e1\" source=\"n1\" target=\"n0\">\n\
             \x20\x20\x20\x20\x20\x20\x20\x20<attvalues>\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20<attvalue for=\"ref_count\" value=\"1\"/>\n\
             \x20\x20\x20\x20\x20\x20\x20\x20</attvalues>\n\
             \x20\x20\x20\x20\x20\x20</edge>\n\
             \x20\x20\x20\x20\x20\x20<edge id=\"e2\" source=\"n0\" target=\"n2\">\n\
             \x20\x20\x20\x20\x20\x20\x20\x20<attvalues>\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20<attvalue for=\"ref_count\" value=\"1\"/>\n\
             \x20\x20\x20\x20\x20\x20\x20\x20</attvalues>\n\
             \x20\x20\x20\x20\x20\x20</edge>\n\
             \x20\x20\x20\x20\x20\x20<edge id=\"e3\" source=\"n2\" target=\"n3\">\n\
             \x20\x20\x20\x20\x20\x20\x20\x20<attvalues>\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20<attvalue for=\"ref_count\" value=\"3\"/>\n\
             \x20\x20\x20\x20\x20\x20\x20\x20</attvalues>\n\
             \x20\x20\x20\x20\x20\x20</edge>\n\
             \x20\x20\x20\x20\x20\x20<edge id=\"e4\" source=\"n1\" target=\"n4\">\n\
             \x20\x20\x20\x20\x20\x20\x20\x20<attvalues>\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20<attvalue for=\"ref_count\" value=\"1\"/>\n\
             \x20\x20\x20\x20\x20\x20\x20\x20</attvalues>\n\
             \x20\x20\x20\x20\x20\x20</edge>\n\
             \x20\x20\x20\x20\x20\x20<edge id=\"e5\" source=\"n3\" target=\"n4\">\n\
             \x20\x20\x20\x20\x20\x20\x20\x20<attvalues>\n\
             \x20\x20\x20\x20\x20\x20\x20\x20\x20\x20<attvalue for=\"ref_count\" value=\"1\"/>\n\
             \x20\x20\x20\x20\x20\x20\x20\x20</attvalues>\n\
             \x20\x20\x20\x20\x20\x20</edge>\n\
             \x20\x20\x20\x20</edges>\n\
             \x20\x20</graph>\n\
             </gexf>\n"
        );
    }

    #[test]
    fn to_gexf_focus_selects_bfs_subgraph() {
        let output = to_gexf(&sample_graph(), Some("a"), 1, &cyclic_edges());
        assert!(output.contains("<node id=\"n0\" label=\"a\">"));
        assert!(output.contains("<node id=\"n2\" label=\"c\">"));
        assert!(!output.contains("<node id=\"n3\""));
        // Only edges between kept nodes survive, renumbered from 0.
        assert!(output.contains("id=\"e0\" source=\"n0\" target=\"n1\""));
        assert!(output.contains("id=\"e2\" source=\"n0\" target=\"n2\""));
        assert!(!output.contains("e3"));
    }

    #[test]
    fn to_gexf_escapes_values_and_omits_missing_lang() {
        let mut graph = sample_graph();
        graph.nodes[0].name = "a&b<c>d\"e'f".to_string();
        graph.nodes[1].lang = None;
        let output = to_gexf(&graph, None, 0, &cyclic_edges());
        assert!(output.contains("label=\"a&amp;b&lt;c&gt;d&quot;e&apos;f\""));
        assert!(
            output.contains("<attvalue for=\"name\" value=\"a&amp;b&lt;c&gt;d&quot;e&apos;f\"/>")
        );
        assert!(
            !output.contains("label=\"b\">\n        <attvalues>\n          <attvalue for=\"lang\""),
            "missing lang must be omitted: {output}"
        );
        // The other nodes still carry their lang.
        assert!(output.contains(
            "<attvalue for=\"name\" value=\"c\"/>\n          <attvalue for=\"lang\" value=\"ts\"/>"
        ));
    }
}
