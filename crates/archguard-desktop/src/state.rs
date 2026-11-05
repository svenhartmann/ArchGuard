//! Graph tab state, independent of Slint: expansion, selection, edge
//! mode, search and the lists derived from them.

use std::collections::BTreeSet;

use archguard_engine::core::{Finding, FindingKind};

use archguard_engine::core::{ReviewPriority, ReviewSources};

use crate::tree::{EdgeMode, Lane, Tree, TreeView};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub node: usize,
    pub label: String,
    pub scope: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CycleEntry {
    pub first_node: usize,
    pub label: String,
    pub size: usize,
}

/// A neighbouring row of the selection with the reference count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Neighbor {
    pub row: usize,
    pub weight: u64,
}

pub struct GraphState {
    pub tree: Tree,
    pub expanded: BTreeSet<usize>,
    pub view: TreeView,
    /// Selected tree node (not row: rows move when branches open).
    pub selected: Option<usize>,
    pub mode: EdgeMode,
    /// Review priority per visible row; `None` for low priority.
    pub priorities: Vec<Option<ReviewPriority>>,
    review_sources: ReviewSources,
    entry_points: Vec<String>,
}

impl GraphState {
    #[cfg(test)]
    pub fn new(tree: Tree) -> Self {
        Self::with_review(tree, ReviewSources::default(), Vec::new())
    }

    pub fn with_review(
        tree: Tree,
        review_sources: ReviewSources,
        entry_points: Vec<String>,
    ) -> Self {
        let mut s = GraphState {
            view: tree.view(&BTreeSet::new()),
            tree,
            expanded: BTreeSet::new(),
            selected: None,
            mode: EdgeMode::All,
            priorities: Vec::new(),
            review_sources,
            entry_points,
        };
        s.priorities = s.review();
        s
    }

    /// Review priorities of the visible rows, computed on the current view
    /// as the web viewer does (`analyze_review`, mirrored in
    /// `viewer_review.js`). Low priorities and errors show no badge.
    fn review(&self) -> Vec<Option<ReviewPriority>> {
        let view = self.view.as_graph(&self.tree);
        let Ok(report) = archguard_engine::analysis::analyze_review(
            &self.tree.full,
            &view,
            &self.review_sources,
            &self.entry_points,
        ) else {
            return vec![None; self.view.rows.len()];
        };
        let mut out = vec![None; self.view.rows.len()];
        for m in report.nodes {
            if let Some(slot) = out.get_mut(m.node_id.0 as usize)
                && m.priority != ReviewPriority::Low
            {
                *slot = Some(m.priority);
            }
        }
        out
    }

    fn refresh(&mut self) {
        self.view = self.tree.view(&self.expanded);
        self.priorities = self.review();
        if let Some(node) = self.selected
            && self.selected_row_of(node).is_none()
        {
            self.selected = None;
        }
    }

    fn selected_row_of(&self, node: usize) -> Option<usize> {
        self.view.rows.iter().position(|r| r.node == node)
    }

    pub fn selected_row(&self) -> Option<usize> {
        self.selected.and_then(|n| self.selected_row_of(n))
    }

    pub fn select_row(&mut self, row: Option<usize>) {
        self.selected = row.and_then(|r| self.view.rows.get(r)).map(|r| r.node);
        // A new selection shows both directions, as in the web viewer.
        self.mode = match (self.selected, self.mode) {
            (Some(_), EdgeMode::All) => EdgeMode::Both,
            (None, EdgeMode::Cycles) => EdgeMode::Cycles,
            (None, _) => EdgeMode::All,
            (Some(_), mode) => mode,
        };
    }

    pub fn toggle_row(&mut self, row: usize) {
        let Some(r) = self.view.rows.get(row) else {
            return;
        };
        if !r.expandable {
            return;
        }
        if !self.expanded.remove(&r.node) {
            self.expanded.insert(r.node);
        }
        self.refresh();
    }

    pub fn expand_all(&mut self) {
        self.expanded = (0..self.tree.nodes.len())
            .filter(|&n| !self.tree.nodes[n].children.is_empty())
            .collect();
        self.refresh();
    }

    pub fn collapse_all(&mut self) {
        self.expanded.clear();
        self.refresh();
    }

    /// Open every ancestor of `node` and select it.
    pub fn reveal(&mut self, node: usize) {
        let mut p = self.tree.nodes.get(node).and_then(|n| n.parent);
        while let Some(a) = p {
            self.expanded.insert(a);
            p = self.tree.nodes[a].parent;
        }
        self.refresh();
        let row = self.selected_row_of(node);
        self.select_row(row);
    }

    /// The selection's ancestors and itself, outermost first.
    pub fn crumbs(&self) -> Vec<usize> {
        let mut path = Vec::new();
        let mut cur = self.selected;
        while let Some(n) = cur {
            path.push(n);
            cur = self.tree.nodes[n].parent;
        }
        path.reverse();
        path
    }

    /// Nodes whose name contains `query`, ignoring case; at most `limit`.
    pub fn search(&self, query: &str, limit: usize) -> Vec<Hit> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        self.tree
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.name.to_lowercase().contains(&q))
            .take(limit)
            .map(|(i, n)| Hit {
                node: i,
                label: n.label.clone(),
                scope: n
                    .parent
                    .map(|p| self.tree.nodes[p].name.clone())
                    .unwrap_or_default(),
            })
            .collect()
    }

    /// `(incoming, outgoing)` neighbours of the selection, heaviest first.
    pub fn neighbors(&self) -> (Vec<Neighbor>, Vec<Neighbor>) {
        let Some(s) = self.selected_row() else {
            return (Vec::new(), Vec::new());
        };
        let (mut inc, mut out) = (Vec::new(), Vec::new());
        for (i, lane) in self.view.shown(Some(s), EdgeMode::Both) {
            let l = &self.view.links[i];
            match lane {
                Lane::Incoming => inc.push(Neighbor {
                    row: l.from,
                    weight: l.weight,
                }),
                Lane::Outgoing => out.push(Neighbor {
                    row: l.to,
                    weight: l.weight,
                }),
            }
        }
        let by_weight =
            |a: &Neighbor, b: &Neighbor| b.weight.cmp(&a.weight).then(a.row.cmp(&b.row));
        inc.sort_by(by_weight);
        out.sort_by(by_weight);
        (inc, out)
    }

    /// `(incoming, outgoing, all)` link counts for the filter buttons.
    pub fn counts(&self) -> (usize, usize, usize) {
        let all = self.view.links.len();
        let Some(s) = self.selected_row() else {
            return (0, 0, all);
        };
        let inc = self.view.shown(Some(s), EdgeMode::Incoming).len();
        let out = self.view.shown(Some(s), EdgeMode::Outgoing).len();
        (inc, out, all)
    }

    /// Cycle findings of the report, mapped onto tree nodes by name.
    pub fn cycles(&self, findings: &[Finding]) -> Vec<CycleEntry> {
        findings
            .iter()
            .filter(|f| f.kind == FindingKind::Cycle)
            .filter_map(|f| {
                let first = f.nodes.first()?;
                let node = self.tree.nodes.iter().position(|n| &n.name == first)?;
                let shown: Vec<&str> = f
                    .nodes
                    .iter()
                    .take(3)
                    .map(|n| n.rsplit('/').next().unwrap_or(n))
                    .collect();
                let more = if f.nodes.len() > 3 { " …" } else { "" };
                Some(CycleEntry {
                    first_node: node,
                    label: format!("{}{more}", shown.join(" ↔ ")),
                    size: f.nodes.len(),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::{Edge, Graph, Node, NodeId, NodeKind, Ref, Severity};

    fn tree() -> Tree {
        let node = |id: u32, name: &str, kind, parent: Option<u32>| Node {
            id: NodeId(id),
            name: name.into(),
            kind,
            parent: parent.map(NodeId),
            lang: None,
        };
        let edge = |from: u32, to: u32, count: u64| Edge {
            from: NodeId(from),
            to: NodeId(to),
            refs: vec![Ref {
                file: "f".into(),
                line: 1,
                raw: "use".into(),
            }],
            count: Some(count),
        };
        Tree::from_graph(&Graph {
            schema_version: 1,
            root: ".".into(),
            nodes: vec![
                node(0, "app", NodeKind::Module, None),
                node(1, "app/ui", NodeKind::Module, Some(0)),
                node(2, "app/ui/view.rs", NodeKind::File, Some(1)),
                node(3, "core", NodeKind::Module, None),
                node(4, "core/lib.rs", NodeKind::File, Some(3)),
            ],
            edges: vec![edge(2, 4, 5), edge(4, 2, 1)],
            semantic: None,
            review_kpis: None,
        })
    }

    #[test]
    fn review_priorities_follow_the_visible_rows() {
        // app ↔ core: cyclic group of 2 (+32), 2 direct links (+4),
        // 1 transitive dependent (+1) = 37 → medium (P2) for both.
        let s = GraphState::new(tree());
        assert_eq!(
            s.priorities,
            [Some(ReviewPriority::Medium), Some(ReviewPriority::Medium)]
        );
        let mut s = s;
        s.expand_all();
        assert_eq!(s.priorities.len(), s.view.rows.len());
    }

    #[test]
    fn reveal_opens_ancestors_and_selects() {
        let mut s = GraphState::new(tree());
        s.reveal(2);
        assert!(s.expanded.contains(&0) && s.expanded.contains(&1));
        assert_eq!(s.selected, Some(2));
        assert_eq!(s.view.rows[s.selected_row().unwrap()].node, 2);
        assert_eq!(s.crumbs(), [0, 1, 2]);
        assert_eq!(s.mode, EdgeMode::Both);
    }

    #[test]
    fn collapsing_hides_and_drops_a_selection_inside() {
        let mut s = GraphState::new(tree());
        s.reveal(2);
        s.collapse_all();
        assert_eq!(s.view.rows.len(), 2);
        assert_eq!(s.selected, None);
    }

    #[test]
    fn search_finds_by_path_and_names_the_parent() {
        let s = GraphState::new(tree());
        let hits = s.search("VIEW", 10);
        assert_eq!(
            hits,
            [Hit {
                node: 2,
                label: "view.rs".into(),
                scope: "app/ui".into()
            }]
        );
        assert!(s.search("  ", 10).is_empty());
    }

    #[test]
    fn neighbors_and_counts_follow_the_selection() {
        let mut s = GraphState::new(tree());
        assert_eq!(s.counts(), (0, 0, 2));
        s.select_row(Some(0));
        let (inc, out) = s.neighbors();
        assert_eq!(out, [Neighbor { row: 1, weight: 5 }]);
        assert_eq!(inc, [Neighbor { row: 1, weight: 1 }]);
        assert_eq!(s.counts(), (1, 1, 2));
    }

    #[test]
    fn cycle_findings_map_onto_tree_nodes() {
        let s = GraphState::new(tree());
        let finding = Finding {
            kind: FindingKind::Cycle,
            id: "scc-1".into(),
            severity: Severity::Error,
            message: None,
            evidence: Vec::new(),
            nodes: vec!["app".into(), "core".into()],
            edge_count: 2,
            cycles_sampled: 1,
            sampling: None,
            suggested_cuts: Vec::new(),
        };
        let c = s.cycles(&[finding]);
        assert_eq!(c.len(), 1);
        assert_eq!((c[0].first_node, c[0].size), (0, 2));
        assert!(c[0].label.contains("app") && c[0].label.contains("core"));
    }
}
