//! A tree view as the export sees it: the shown rows and links of a Graph
//! or Matrix instance as [`GraphInput`], and the Graph's tree display as
//! a [`Diagram`] for the SVG (mapping in `export.rs` and `export/svg.rs`).

use crate::export::{
    Diagram, DiagramBox, DiagramEdge, EdgeKind, GraphEdge, GraphFilters, GraphInput, GraphNode,
};
use crate::tree::{EdgeMode, LANE_W, LinkKind, ROW_H, TOP, Tree, TreeView};

/// The tree's width between the lanes in the SVG, fixed (the tree in
/// `ui/tree_view.slint` at a 1120 px pane), so the file does not depend on
/// the pane width.
const TREE_W: f32 = 560.0;
/// Indent per depth level (`18px` in `ui/tree_view.slint`).
const INDENT: f32 = 18.0;

/// The web's `ui.edgeMode` value of `mode`.
pub fn direction(mode: EdgeMode) -> &'static str {
    match mode {
        EdgeMode::Incoming => "incoming",
        EdgeMode::Outgoing => "outgoing",
        EdgeMode::Both => "both",
        EdgeMode::All => "all",
        EdgeMode::Cycles => "cycles",
    }
}

/// The web's `ui.diagram` value of a Graph display (0 tree, 1 network,
/// 2 dependencies).
pub fn diagram(display: i32) -> &'static str {
    match display {
        1 => "network",
        2 => "dependencies",
        _ => "overview",
    }
}

/// The Graph's display settings under the web's names.
pub fn graph_filters(
    display: i32,
    mode: EdgeMode,
    reference: Option<String>,
    search: &str,
) -> GraphFilters {
    GraphFilters {
        diagram: diagram(display).into(),
        reference,
        direction: direction(mode).into(),
        show_files: true,
        cycles_only: mode == EdgeMode::Cycles,
        search: search.trim().to_string(),
    }
}

/// The rows `keep` accepts as nodes and the links `shown` (indices into
/// `view.links`) between them as edges. The scope counts are those of
/// the KPI row within `keep`; cycle groups and ACD those of the view.
pub fn tree_input(
    tree: &Tree,
    view: &TreeView,
    shown: &[usize],
    keep: impl Fn(usize) -> bool,
) -> GraphInput {
    let exact = tree.full.edges.iter().all(|e| e.count.is_some());
    let name = |row: usize| tree.nodes[view.rows[row].node].name.clone();
    let nodes = (0..view.rows.len())
        .filter(|&r| keep(r))
        .map(|r| GraphNode {
            name: name(r),
            kind: if tree.nodes[view.rows[r].node].file {
                "file"
            } else {
                "module"
            }
            .into(),
        })
        .collect();
    let edges = shown
        .iter()
        .map(|&i| &view.links[i])
        .filter(|l| keep(l.from) && keep(l.to))
        .map(|l| GraphEdge {
            from: name(l.from),
            to: name(l.to),
            count: l.weight,
            count_exact: exact,
            cyclic: l.cyclic,
            evidence: l.refs.clone(),
        })
        .collect();
    GraphInput {
        scope_nodes: view.scope_rows().into_iter().filter(|&r| keep(r)).count(),
        scope_dependencies: view
            .links
            .iter()
            .filter(|l| keep(l.from) && keep(l.to))
            .count(),
        cyclic_groups: view.cycle_groups,
        acd: view.acd(tree),
        nodes,
        edges,
        ..GraphInput::default()
    }
}

fn edge_kind(kind: LinkKind) -> EdgeKind {
    match kind {
        LinkKind::Neutral => EdgeKind::Neutral,
        LinkKind::Incoming => EdgeKind::Incoming,
        LinkKind::Outgoing => EdgeKind::Outgoing,
        LinkKind::Cyclic => EdgeKind::Cyclic,
    }
}

/// The Graph's tree display: one box per row, one edge per drawn link,
/// at the geometry of `ui/tree_view.slint`.
pub fn tree_diagram(
    tree: &Tree,
    view: &TreeView,
    selected: Option<usize>,
    mode: EdgeMode,
) -> Diagram {
    let boxes = view
        .rows
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let indent = r.depth as f32 * INDENT;
            let label: Vec<&str> = r
                .chain
                .iter()
                .map(|&c| tree.nodes[c].label.as_str())
                .collect();
            DiagramBox {
                x: LANE_W + indent,
                y: TOP + i as f32 * ROW_H,
                w: TREE_W - indent,
                h: ROW_H - 4.0,
                label: label.join("/"),
                selected: selected == Some(i),
                cyclic: view.in_cycle[i],
            }
        })
        .collect();
    let edges = view
        .draw(selected, mode)
        .into_iter()
        .map(|d| DiagramEdge {
            path: d.path,
            kind: edge_kind(d.kind),
            dx: if d.right { LANE_W + TREE_W } else { 0.0 },
            dy: 0.0,
            width: 0.0,
            arrow: false,
        })
        .collect();
    Diagram {
        width: 2.0 * LANE_W + TREE_W,
        height: TOP + view.rows.len() as f32 * ROW_H + 20.0,
        boxes,
        edges,
        ..Diagram::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::GraphState;

    /// The Graph's shown rows and links become nodes and edges; a
    /// selection's direction filter leaves only its links.
    #[test]
    fn a_graph_exports_its_shown_rows_and_links() {
        let mut g = GraphState::new(crate::matrix::tests::tree());
        g.expand_all();
        let shown: Vec<usize> = g.view.shown(None, g.mode).iter().map(|s| s.0).collect();
        let input = tree_input(&g.tree, &g.view, &shown, |_| true);
        assert_eq!(input.nodes.len(), g.view.rows.len());
        assert!(
            input
                .nodes
                .iter()
                .any(|n| n.name == "lib/src/a.rs" && n.kind == "file")
        );
        assert!(
            input
                .nodes
                .iter()
                .any(|n| n.name == "lib/src" && n.kind == "module"),
            "{:?}",
            input.nodes
        );
        let edge = |from: &str, to: &str| {
            input
                .edges
                .iter()
                .find(|e| e.from == from && e.to == to)
                .cloned()
        };
        let a_b = edge("lib/src/a.rs", "lib/src/b.rs").expect("a → b");
        assert_eq!((a_b.count, a_b.cyclic, a_b.count_exact), (2, true, true));
        assert_eq!(
            edge("app/ui/v.rs", "lib/src/a.rs").map(|e| e.count),
            Some(4)
        );
        assert_eq!(input.scope_dependencies, g.view.links.len());
        assert_eq!(input.cyclic_groups, 1);

        let v = g
            .view
            .rows
            .iter()
            .position(|r| g.tree.nodes[r.node].name == "app/ui/v.rs");
        g.select_row(v);
        g.mode = EdgeMode::Outgoing;
        let shown: Vec<usize> = g
            .view
            .shown(g.selected_row(), g.mode)
            .iter()
            .map(|s| s.0)
            .collect();
        let only = tree_input(&g.tree, &g.view, &shown, |_| true);
        let pairs: Vec<_> = only
            .edges
            .iter()
            .map(|e| (e.from.as_str(), e.to.as_str()))
            .collect();
        assert_eq!(pairs, [("app/ui/v.rs", "lib/src/a.rs")]);
        let f = graph_filters(0, g.mode, Some("app/ui/v.rs".into()), " ui ");
        assert_eq!(
            (f.diagram.as_str(), f.direction.as_str(), f.search.as_str()),
            ("overview", "outgoing", "ui")
        );
    }

    /// The tree display: a box per row at the view's row geometry, right
    /// lane paths shifted past the tree.
    #[test]
    fn the_tree_diagram_follows_the_row_geometry() {
        let mut g = GraphState::new(crate::matrix::tests::tree());
        g.expand_all();
        let d = tree_diagram(&g.tree, &g.view, None, g.mode);
        assert_eq!(d.boxes.len(), g.view.rows.len());
        assert_eq!(d.boxes[1].y, TOP + ROW_H);
        assert_eq!(d.boxes[0].x, LANE_W);
        assert!(d.boxes.iter().any(|b| b.cyclic));
        assert_eq!(d.edges.len(), g.view.links.len());
        assert!(d.edges.iter().all(|e| e.dx == LANE_W + TREE_W));
        assert_eq!(d.width, 2.0 * LANE_W + TREE_W);
    }
}
