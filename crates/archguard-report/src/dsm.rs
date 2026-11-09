//! Dependency-Structure-Matrix renderings (Kap. 9, `archguard dsm`;
//! Kap. 10.2, MCP `dsm`).
//!
//! The matrix has one row and one column per entry of `order` — the
//! caller (the engine) precomputes the reverse topological order of the
//! condensation with cycle groups adjacent, and the set of cyclic
//! edges, because `archguard-report` must not depend on `archguard-analysis`.
//! Given that order, the cyclic edge of every cycle group appears in a
//! cell *above* the diagonal; those cells are marked with `*`.
//!
//! A cell's weight is the number of concrete import references
//! ([`archguard_core::Edge::weight`], the F12 total rather than the
//! capped evidence sample) of all edges from the row node to the column
//! node (summed when several `Edge`s connect the same pair). Zero-weight
//! cells are rendered as
//! `.` in text and omitted entirely from the JSON. The diagonal is
//! always `-` (the core graph never contains self-edges).
//!
//! Rows and columns follow `order` strictly: ids not present in the
//! graph are still rendered (their name falls back to `?`), edges that
//! touch an id outside `order` are ignored, and duplicate ids in
//! `order` are deduplicated keeping the first occurrence.

use std::collections::{BTreeMap, HashMap, HashSet};

use archguard_core::{Graph, NodeId};

/// Trailing legend line of the text rendering.
const LEGEND: &str = "* = Zyklen-Kante (über der Diagonalen)";

/// Aggregated cell weights, keyed by `(row, col)` order indices and
/// iterated in that order so the output is byte-stable.
type Cells = BTreeMap<(usize, usize), u64>;

/// `order` with duplicates removed, keeping the first occurrence.
fn dedupe(order: &[NodeId]) -> Vec<NodeId> {
    let mut seen: HashSet<NodeId> = HashSet::new();
    order
        .iter()
        .copied()
        .filter(|id| seen.insert(*id))
        .collect()
}

/// Order index of every id; first occurrence wins (input is deduped).
fn index(order: &[NodeId]) -> HashMap<NodeId, usize> {
    order
        .iter()
        .copied()
        .enumerate()
        .map(|(i, id)| (id, i))
        .collect()
}

/// Sum of import-site counts per `(row, col)` order-index pair.
/// Self-edges (never produced by the core graph) and edges touching
/// any id outside `order` are skipped.
fn weights(g_view: &Graph, idx: &HashMap<NodeId, usize>) -> Cells {
    let mut cells: Cells = BTreeMap::new();
    for edge in &g_view.edges {
        if edge.from == edge.to {
            continue;
        }
        let (Some(&row), Some(&col)) = (idx.get(&edge.from), idx.get(&edge.to)) else {
            continue;
        };
        *cells.entry((row, col)).or_default() += edge.weight();
    }
    cells
}

/// Display name of a node: `?` for an id the graph does not contain.
fn name(g_view: &Graph, id: NodeId) -> &str {
    g_view.node_name(id).unwrap_or("?")
}

/// Whether the cell at order indices `(row, col)` is a cycle cell.
fn is_cyclic(
    cyclic_edges: &HashSet<(NodeId, NodeId)>,
    order: &[NodeId],
    row: usize,
    col: usize,
) -> bool {
    cyclic_edges.contains(&(order[row], order[col]))
}

/// Render the DSM as aligned plain text: a header line of column
/// indices, one line per row (`index  label  cells…`), then the
/// [`LEGEND`]. Every line ends with `\n`; an empty `order` yields just
/// the legend (there are no columns to enumerate). See the module docs
/// for the format.
#[tracing::instrument(skip_all)]
pub fn to_dsm_text(
    g_view: &Graph,
    order: &[NodeId],
    cyclic_edges: &HashSet<(NodeId, NodeId)>,
) -> String {
    let order = dedupe(order);
    let mut out = String::new();
    if order.is_empty() {
        out.push_str(LEGEND);
        out.push('\n');
        return out;
    }
    let cells = weights(g_view, &index(&order));
    let labels: Vec<&str> = order.iter().map(|&id| name(g_view, id)).collect();
    let rendered: Vec<Vec<String>> = (0..order.len())
        .map(|row| {
            (0..order.len())
                .map(|col| match cells.get(&(row, col)) {
                    Some(&weight) if row != col => {
                        let mark = if is_cyclic(cyclic_edges, &order, row, col) {
                            "*"
                        } else {
                            ""
                        };
                        format!("{weight}{mark}")
                    }
                    _ if row == col => "-".to_string(),
                    _ => ".".to_string(),
                })
                .collect()
        })
        .collect();

    let idx_w = (order.len() - 1).to_string().len();
    let label_w = labels.iter().map(|l| l.len()).max().unwrap_or(0);
    let cell_w = rendered
        .iter()
        .flatten()
        .map(String::len)
        .max()
        .unwrap_or(1);

    out.push_str(&" ".repeat(idx_w + 2 + label_w + 2));
    let header: Vec<String> = (0..order.len())
        .map(|col| format!("{col:>cell_w$}"))
        .collect();
    out.push_str(&header.join("  "));
    out.push('\n');
    for (row, cells) in rendered.iter().enumerate() {
        let padded: Vec<String> = cells.iter().map(|c| format!("{c:>cell_w$}")).collect();
        out.push_str(&format!(
            "{row:>idx_w$}  {:<label_w$}  {}\n",
            labels[row],
            padded.join("  ")
        ));
    }
    out.push_str(LEGEND);
    out.push('\n');
    out
}

/// Render the DSM as JSON: `{"order": [names], "cells": [[row, col,
/// weight, cyclic]]}` — `order` the node names in matrix order, `cells`
/// only the non-zero cells sorted by row then column. Key order is
/// fixed (built literally, not through a map). `pretty` selects the
/// indented form, mirroring [`crate::to_json`].
#[tracing::instrument(skip_all)]
pub fn to_dsm_json(
    g_view: &Graph,
    order: &[NodeId],
    cyclic_edges: &HashSet<(NodeId, NodeId)>,
    pretty: bool,
) -> String {
    let order = dedupe(order);
    let cells = weights(g_view, &index(&order));
    let names: Vec<&str> = order.iter().map(|&id| name(g_view, id)).collect();
    let entries: Vec<(usize, usize, u64, bool)> = cells
        .iter()
        .map(|(&(row, col), &weight)| (row, col, weight, is_cyclic(cyclic_edges, &order, row, col)))
        .collect();
    if pretty {
        pretty_json(&names, &entries)
    } else {
        compact_json(&names, &entries)
    }
}

/// JSON string literal for a name (escaping via serde_json).
fn json_str(s: &str) -> String {
    serde_json::to_string(s).expect("a &str is always serializable")
}

fn bool_str(b: bool) -> &'static str {
    if b { "true" } else { "false" }
}

fn compact_json(names: &[&str], cells: &[(usize, usize, u64, bool)]) -> String {
    let order = names
        .iter()
        .map(|n| json_str(n))
        .collect::<Vec<_>>()
        .join(",");
    let cells = cells
        .iter()
        .map(|(row, col, weight, cyclic)| format!("[{row},{col},{weight},{}]", bool_str(*cyclic)))
        .collect::<Vec<_>>()
        .join(",");
    format!("{{\"order\":[{order}],\"cells\":[{cells}]}}")
}

/// The same document in the `serde_json::to_string_pretty` style
/// (two-space indent, no trailing newline).
fn pretty_json(names: &[&str], cells: &[(usize, usize, u64, bool)]) -> String {
    let mut out = String::from("{\n  \"order\": [");
    for (i, n) in names.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!("\n    {}", json_str(n)));
    }
    out.push_str(if names.is_empty() {
        "],\n  \"cells\": ["
    } else {
        "\n  ],\n  \"cells\": ["
    });
    for (i, (row, col, weight, cyclic)) in cells.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "\n    [\n      {row},\n      {col},\n      {weight},\n      {}\n    ]",
            bool_str(*cyclic)
        ));
    }
    out.push_str(if cells.is_empty() { "]" } else { "\n  ]" });
    out.push_str("\n}");
    out
}

#[cfg(test)]
mod tests {
    use super::{to_dsm_json, to_dsm_text};
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

    /// Edge with `n` distinct import sites — its weight contribution.
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

    /// Five file nodes with one 2-cycle `a.ts ↔ b.ts` plus one-way
    /// tails `a → c`, `c → d`, `b → e`, `d → e`. Ids: a=0 … e=4.
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

    /// Hand-computed reverse topological order of the condensation
    /// ({a,b}, {c}, {d}, {e}): sinks first, cycle group adjacent.
    /// Indices: e=0, d=1, c=2, b=3, a=4 — the `b → a` cell (3, 4) is
    /// the cycle edge above the diagonal.
    fn order() -> Vec<NodeId> {
        [4, 3, 2, 1, 0].map(NodeId).to_vec()
    }

    #[test]
    fn golden_text() {
        let text = to_dsm_text(&sample_graph(), &order(), &cyclic_edges());
        assert_eq!(
            text,
            "       0   1   2   3   4\n\
             0  e   -   .   .   .   .\n\
             1  d   1   -   .   .   .\n\
             2  c   .   3   -   .   .\n\
             3  b   1   .   .   -  1*\n\
             4  a   .   .   1  2*   -\n\
             * = Zyklen-Kante (über der Diagonalen)\n"
        );
    }

    #[test]
    fn golden_json() {
        let json = to_dsm_json(&sample_graph(), &order(), &cyclic_edges(), false);
        assert_eq!(
            json,
            "{\"order\":[\"e\",\"d\",\"c\",\"b\",\"a\"],\"cells\":\
             [[1,0,1,false],[2,1,3,false],[3,0,1,false],[3,4,1,true],\
             [4,2,1,false],[4,3,2,true]]}"
        );
    }

    #[test]
    fn json_pretty_mirrors_to_json_style() {
        let graph = Graph {
            semantic: None,
            review_kpis: None,
            schema_version: 1,
            root: "/abs/projekt".to_string(),
            nodes: vec![file(0, "a"), file(1, "b")],
            edges: vec![edge_with(0, 1, 2)],
        };
        let order = [NodeId(0), NodeId(1)];
        let json = to_dsm_json(&graph, &order, &HashSet::new(), true);
        assert_eq!(
            json,
            r#"{
  "order": [
    "a",
    "b"
  ],
  "cells": [
    [
      0,
      1,
      2,
      false
    ]
  ]
}"#
        );
    }

    #[test]
    fn empty_order_yields_legend_only_and_empty_json() {
        assert_eq!(
            to_dsm_text(&sample_graph(), &[], &cyclic_edges()),
            "* = Zyklen-Kante (über der Diagonalen)\n"
        );
        assert_eq!(
            to_dsm_json(&sample_graph(), &[], &cyclic_edges(), false),
            "{\"order\":[],\"cells\":[]}"
        );
    }

    #[test]
    fn weights_of_several_edges_between_a_pair_are_summed() {
        let graph = Graph {
            semantic: None,
            review_kpis: None,
            schema_version: 1,
            root: "/abs/projekt".to_string(),
            nodes: vec![file(0, "a"), file(1, "b")],
            edges: vec![edge_with(0, 1, 1), edge_with(0, 1, 2)],
        };
        let order = [NodeId(0), NodeId(1)];
        assert_eq!(
            to_dsm_json(&graph, &order, &HashSet::new(), false),
            "{\"order\":[\"a\",\"b\"],\"cells\":[[0,1,3,false]]}"
        );
    }

    #[test]
    fn the_f12_total_shows_even_with_a_capped_sample() {
        // A 100-import edge whose evidence sample holds 10 refs renders
        // as 100, not 10; a legacy edge without a total falls back to
        // its sample length — a lower bound, never an invented number.
        let sample = |from: u32| -> Vec<Ref> {
            (0..10)
                .map(|k| Ref {
                    file: format!("{from}.ts"),
                    line: k + 1,
                    raw: "import …".to_string(),
                })
                .collect()
        };
        let graph = Graph {
            semantic: None,
            review_kpis: None,
            schema_version: 1,
            root: "/abs/projekt".to_string(),
            nodes: vec![file(0, "a"), file(1, "b"), file(2, "c")],
            edges: vec![
                Edge {
                    from: NodeId(0),
                    to: NodeId(1),
                    refs: sample(0),
                    count: Some(100),
                },
                Edge {
                    from: NodeId(1),
                    to: NodeId(2),
                    refs: sample(1),
                    count: None,
                },
            ],
        };
        let order = [NodeId(0), NodeId(1), NodeId(2)];
        assert_eq!(
            to_dsm_json(&graph, &order, &HashSet::new(), false),
            "{\"order\":[\"a\",\"b\",\"c\"],\"cells\":[[0,1,100,false],[1,2,10,false]]}"
        );
    }

    #[test]
    fn duplicate_order_entries_keep_the_first_occurrence() {
        let graph = Graph {
            semantic: None,
            review_kpis: None,
            schema_version: 1,
            root: "/abs/projekt".to_string(),
            nodes: vec![file(0, "a"), file(1, "b")],
            edges: vec![edge(0, 1)],
        };
        let order = [NodeId(0), NodeId(1), NodeId(0)];
        let cyclic: HashSet<(NodeId, NodeId)> = [(NodeId(0), NodeId(1))].into_iter().collect();
        assert_eq!(
            to_dsm_json(&graph, &order, &cyclic, false),
            "{\"order\":[\"a\",\"b\"],\"cells\":[[0,1,1,true]]}"
        );
    }

    #[test]
    fn ids_outside_the_graph_and_edges_outside_the_order_are_ignored() {
        // `c` exists but stays out of the order; NodeId(9) never existed.
        let order = [NodeId(9), NodeId(0), NodeId(1)];
        let text = to_dsm_text(&sample_graph(), &order, &cyclic_edges());
        let json = to_dsm_json(&sample_graph(), &order, &cyclic_edges(), false);
        // Row/col 0 is the unknown node: no edges touch it.
        assert!(json.starts_with("{\"order\":[\"?\",\"a\",\"b\"],\"cells\":["));
        // b → a now sits at (2, 1), below the diagonal; a → b at (1, 2).
        assert_eq!(
            json,
            "{\"order\":[\"?\",\"a\",\"b\"],\"cells\":[[1,2,2,true],[2,1,1,true]]}"
        );
        assert!(text.contains("?"));
        assert!(!text.contains(" c "));
    }
}
