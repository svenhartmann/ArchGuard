//! The package tree of the Graph tab, as in the web viewer's "Tree"
//! display: expandable containment rows, dependencies aggregated onto
//! the visible rows, incoming links left and outgoing links right of the
//! selection, cycles marked.

use std::collections::{BTreeMap, BTreeSet};

use archguard_engine::core::{Graph, NodeKind, Ref};

/// Evidence sample per link, as `core::view::aggregate` caps it.
const MAX_REFS: usize = 10;

/// One node of the containment tree, addressed by its index in
/// `Graph::nodes`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeNode {
    pub name: String,
    /// Last path segment.
    pub label: String,
    pub file: bool,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Tree {
    pub nodes: Vec<TreeNode>,
    pub roots: Vec<usize>,
    /// File-level dependencies `(from, to, weight, edge index in full)`.
    edges: Vec<(usize, usize, u64, usize)>,
    /// The analysed graph the tree was built from.
    pub full: Graph,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub node: usize,
    /// Nodes this row stands for, outermost first; more than one when
    /// open containers with a single open child are compressed.
    pub chain: Vec<usize>,
    pub depth: usize,
    pub expandable: bool,
    pub expanded: bool,
}

/// A dependency between two visible rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub from: usize,
    pub to: usize,
    pub weight: u64,
    pub cyclic: bool,
    /// Deduplicated source references, sorted, at most [`MAX_REFS`].
    pub refs: Vec<Ref>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TreeView {
    pub rows: Vec<Row>,
    pub links: Vec<Link>,
    /// Distinct incoming / outgoing links per row.
    pub incoming: Vec<u32>,
    pub outgoing: Vec<u32>,
    pub in_cycle: Vec<bool>,
    /// Cycle groups of the scope rows, the base [`TreeView::acd`] uses
    /// too (expanded containers carry no links).
    pub cycle_groups: usize,
}

/// Sibling order of the tree, as the web viewer's "Order" option.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    Alphabetical,
    /// Reorder sibling subtrees to reduce crossings (the web default).
    Connections,
}

/// Which links of a selection are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeMode {
    Incoming,
    Outgoing,
    Both,
    All,
    Cycles,
}

/// Where a shown link is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    /// Left of the tree: dependencies on the selection.
    Incoming,
    /// Right of the tree: dependencies of the selection, or any link
    /// without a selection.
    Outgoing,
    /// Right of the tree, neutral: links inside the selection ("Both")
    /// or unrelated to it ("All").
    Other,
}

impl Tree {
    pub fn from_graph(g: &Graph) -> Self {
        let index: BTreeMap<_, usize> =
            g.nodes.iter().enumerate().map(|(i, n)| (n.id, i)).collect();
        let mut nodes: Vec<TreeNode> = g
            .nodes
            .iter()
            .map(|n| TreeNode {
                name: n.name.clone(),
                label: n
                    .name
                    .trim_end_matches('/')
                    .rsplit('/')
                    .next()
                    .unwrap_or(&n.name)
                    .to_string(),
                file: n.kind == NodeKind::File,
                parent: n.parent.and_then(|p| index.get(&p).copied()),
                children: Vec::new(),
            })
            .collect();
        let mut roots = Vec::new();
        for i in 0..nodes.len() {
            match nodes[i].parent {
                Some(p) => nodes[p].children.push(i),
                None => roots.push(i),
            }
        }
        // Packages before files, then by name — as the web tree lists them.
        let order = |nodes: &[TreeNode], list: &mut Vec<usize>| {
            list.sort_by(|&a, &b| {
                (nodes[a].file, &nodes[a].name).cmp(&(nodes[b].file, &nodes[b].name))
            })
        };
        order(&nodes, &mut roots);
        for i in 0..nodes.len() {
            let mut c = std::mem::take(&mut nodes[i].children);
            order(&nodes, &mut c);
            nodes[i].children = c;
        }
        let edges = g
            .edges
            .iter()
            .enumerate()
            .filter_map(|(i, e)| Some((*index.get(&e.from)?, *index.get(&e.to)?, e.weight(), i)))
            .collect();
        Tree {
            nodes,
            roots,
            edges,
            full: g.clone(),
        }
    }

    /// File-level edges `(from, to, edge index in full)` as node indices.
    pub fn edges(&self) -> impl Iterator<Item = (usize, usize, usize)> + '_ {
        self.edges.iter().map(|&(from, to, _, e)| (from, to, e))
    }

    /// `(files below, direct child packages)` of `node`, as the web
    /// viewer's "N files · M package" on network cards.
    pub fn counts(&self, node: usize) -> (usize, usize) {
        let packages = self.nodes[node]
            .children
            .iter()
            .filter(|&&c| !self.nodes[c].file)
            .count();
        let (mut files, mut pending) = (0, self.nodes[node].children.clone());
        while let Some(n) = pending.pop() {
            if self.nodes[n].file {
                files += 1;
            }
            pending.extend(self.nodes[n].children.iter().copied());
        }
        (files, packages)
    }

    /// [`Tree::view`] with the sibling order `sort`.
    pub fn view_sorted(&self, expanded: &BTreeSet<usize>, sort: Sort) -> TreeView {
        let mut v = self.view(expanded);
        if sort == Sort::Connections {
            let order = self.connection_order(&v);
            v.reorder(&order);
        }
        v
    }

    /// Old row indices in their connection order.
    fn connection_order(&self, v: &TreeView) -> Vec<usize> {
        let row_of: BTreeMap<usize, usize> = v
            .rows
            .iter()
            .enumerate()
            .map(|(i, r)| (r.node, i))
            .collect();
        let mut children = crate::tree_order::Children::new();
        for r in &v.rows {
            let parent = self.nodes[r.node].parent.filter(|p| row_of.contains_key(p));
            children.entry(parent).or_default().push(r.node);
        }
        // Rows are alphabetical (packages first), the web's initial order.
        let rank: BTreeMap<usize, usize> = row_of.clone();
        // The web keys links by view ids (scope nodes ranked by name);
        // that key order decides the sample on scopes above 256 links.
        let mut scope: Vec<usize> = v.scope_rows();
        scope.sort_by(|&a, &b| {
            self.nodes[v.rows[a].node]
                .name
                .cmp(&self.nodes[v.rows[b].node].name)
        });
        let mut view_id = vec![usize::MAX; v.rows.len()];
        for (id, &r) in scope.iter().enumerate() {
            view_id[r] = id;
        }
        let node_of_id: Vec<usize> = scope.iter().map(|&r| v.rows[r].node).collect();
        let keyed: Vec<(usize, usize)> = v
            .links
            .iter()
            .map(|l| (view_id[l.from], view_id[l.to]))
            .collect();
        let deps: Vec<(usize, usize)> = web_link_keys(&keyed)
            .into_iter()
            .map(|(a, b)| (node_of_id[a], node_of_id[b]))
            .collect();
        let parent: BTreeMap<usize, usize> = v
            .rows
            .iter()
            .filter_map(|r| {
                let p = self.nodes[r.node].parent?;
                row_of.contains_key(&p).then_some((r.node, p))
            })
            .collect();
        crate::tree_order::order(&mut children, &deps, &rank, &parent);
        crate::tree_order::preorder(&children)
            .into_iter()
            .map(|n| row_of[&n])
            .collect()
    }

    pub fn view(&self, expanded: &BTreeSet<usize>) -> TreeView {
        let rows = self.rows(expanded);
        let mut row_of = vec![usize::MAX; self.nodes.len()];
        for (r, row) in rows.iter().enumerate() {
            row_of[row.node] = r;
        }
        // Every node is represented by its nearest visible ancestor-or-self.
        let rep: Vec<usize> = (0..self.nodes.len())
            .map(|n| self.representative(n, &row_of))
            .collect();
        let mut merged: BTreeMap<(usize, usize), (u64, Vec<Ref>)> = BTreeMap::new();
        for &(from, to, w, edge) in &self.edges {
            let (a, b) = (rep[from], rep[to]);
            if a != b && a != usize::MAX && b != usize::MAX {
                let entry = merged.entry((a, b)).or_default();
                entry.0 += w;
                entry.1.extend(self.full.edges[edge].refs.iter().cloned());
            }
        }
        let component = components(rows.len(), merged.keys().copied());
        let mut size = BTreeMap::<usize, usize>::new();
        for &c in &component {
            *size.entry(c).or_default() += 1;
        }
        let in_cycle: Vec<bool> = component.iter().map(|c| size[c] > 1).collect();
        let cycle_groups = size.values().filter(|&&s| s > 1).count();
        let links: Vec<Link> = merged
            .into_iter()
            .map(|((from, to), (weight, mut refs))| {
                refs.sort_by(|a, b| (&a.file, a.line, &a.raw).cmp(&(&b.file, b.line, &b.raw)));
                refs.dedup();
                refs.truncate(MAX_REFS);
                Link {
                    from,
                    to,
                    weight,
                    cyclic: component[from] == component[to],
                    refs,
                }
            })
            .collect();
        let (mut incoming, mut outgoing) = (vec![0; rows.len()], vec![0; rows.len()]);
        for l in &links {
            outgoing[l.from] += 1;
            incoming[l.to] += 1;
        }
        TreeView {
            rows,
            links,
            incoming,
            outgoing,
            in_cycle,
            cycle_groups,
        }
    }

    /// Visible rows in preorder: roots, and the children of every
    /// expanded row.
    fn rows(&self, expanded: &BTreeSet<usize>) -> Vec<Row> {
        let open = |n: usize| !self.nodes[n].children.is_empty() && expanded.contains(&n);
        let mut rows = Vec::new();
        let mut pending: Vec<(usize, usize)> = self.roots.iter().rev().map(|&r| (r, 0)).collect();
        while let Some((n, depth)) = pending.pop() {
            // Compress open containers whose only child is open too, as the
            // web viewer's containmentLayout does with context chains.
            let mut chain = vec![n];
            while let [only] = self.nodes[*chain.last().unwrap()].children[..] {
                if !open(*chain.last().unwrap()) || !open(only) {
                    break;
                }
                chain.push(only);
            }
            let last = *chain.last().unwrap();
            rows.push(Row {
                node: last,
                chain,
                depth,
                expandable: !self.nodes[last].children.is_empty(),
                expanded: open(last),
            });
            if open(last) {
                pending.extend(
                    self.nodes[last]
                        .children
                        .iter()
                        .rev()
                        .map(|&c| (c, depth + 1)),
                );
            }
        }
        rows
    }

    fn representative(&self, mut n: usize, row_of: &[usize]) -> usize {
        loop {
            if row_of[n] != usize::MAX {
                return row_of[n];
            }
            match self.nodes[n].parent {
                Some(p) => n = p,
                None => return usize::MAX,
            }
        }
    }
}

/// Strongly connected component per row.
fn components(n: usize, links: impl Iterator<Item = (usize, usize)>) -> Vec<usize> {
    let mut component = vec![0; n];
    for (c, members) in crate::tree_order::scc(n, links).into_iter().enumerate() {
        for v in members {
            component[v] = c;
        }
    }
    component
}

impl TreeView {
    /// Lakos ACD of the scope rows and their links, as the web viewer
    /// shows it for the current scope.
    pub fn acd(&self, tree: &Tree) -> f64 {
        archguard_engine::analysis::lakos_metrics(&self.as_graph(tree)).1
    }

    /// The scope rows as a flat view graph: a node's id is its row index,
    /// its name that of its tree node, edges are the links (which only
    /// connect scope rows).
    pub fn as_graph(&self, tree: &Tree) -> Graph {
        use archguard_engine::core::{Edge, Node, NodeId};
        let id = |i: usize| NodeId(i as u32);
        Graph {
            schema_version: 1,
            root: tree.full.root.clone(),
            nodes: self
                .scope_rows()
                .into_iter()
                .map(|i| Node {
                    id: id(i),
                    name: tree.nodes[self.rows[i].node].name.clone(),
                    kind: NodeKind::Module,
                    parent: None,
                    lang: None,
                })
                .collect(),
            edges: self
                .links
                .iter()
                .map(|l| Edge {
                    from: id(l.from),
                    to: id(l.to),
                    refs: Vec::new(),
                    count: Some(l.weight),
                })
                .collect(),
            semantic: None,
            review_kpis: None,
        }
    }

    /// Rows of the analysis scope: all but expanded containers.
    pub fn scope_rows(&self) -> Vec<usize> {
        (0..self.rows.len())
            .filter(|&r| !self.rows[r].expanded)
            .collect()
    }

    /// Rows covered by selecting `row`: the row and its visible
    /// descendants.
    pub fn selection(&self, row: usize) -> BTreeSet<usize> {
        let depth = self.rows[row].depth;
        std::iter::once(row)
            .chain((row + 1..self.rows.len()).take_while(|&r| self.rows[r].depth > depth))
            .collect()
    }

    /// Links to draw for `selected` under `mode`, with their lane.
    /// Without a selection every link (or every cyclic link) is drawn on
    /// the right.
    pub fn shown(&self, selected: Option<usize>, mode: EdgeMode) -> Vec<(usize, Lane)> {
        let Some(s) = selected else {
            return (0..self.links.len())
                .filter(|&i| mode != EdgeMode::Cycles || self.links[i].cyclic)
                .map(|i| (i, Lane::Outgoing))
                .collect();
        };
        let set = self.selection(s);
        let mut shown = Vec::new();
        for (i, l) in self.links.iter().enumerate() {
            let (out, inc) = (set.contains(&l.from), set.contains(&l.to));
            let lane = match (out, inc) {
                (true, false) => Lane::Outgoing,
                (false, true) => Lane::Incoming,
                // Inside the selection: the web shows these with "Both".
                (true, true) => {
                    if matches!(mode, EdgeMode::Both | EdgeMode::All) {
                        shown.push((i, Lane::Other));
                    }
                    continue;
                }
                (false, false) => {
                    if mode == EdgeMode::All {
                        shown.push((i, Lane::Other));
                    }
                    continue;
                }
            };
            let keep = match mode {
                EdgeMode::Incoming => lane == Lane::Incoming,
                EdgeMode::Outgoing => lane == Lane::Outgoing,
                EdgeMode::Both | EdgeMode::All => true,
                EdgeMode::Cycles => l.cyclic,
            };
            if keep {
                shown.push((i, lane));
            }
        }
        shown
    }
}

// Geometry shared with ui/tree_view.slint (TreeGeo).
pub const ROW_H: f32 = 30.0;
pub const LANE_W: f32 = 280.0;
pub const TOP: f32 = 34.0;

/// Colour class of a drawn link, as `TreeLink.kind` in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkKind {
    Neutral = 0,
    Incoming = 1,
    Outgoing = 2,
    Cyclic = 3,
}

/// One link as drawn: path, colour class, endpoint rows (for hover) and
/// whether it lies in the right lane.
#[derive(Debug, Clone, PartialEq)]
pub struct Drawn {
    pub path: String,
    pub kind: LinkKind,
    pub from: usize,
    pub to: usize,
    pub right: bool,
}

pub fn row_center(row: usize) -> f32 {
    TOP + row as f32 * ROW_H + (ROW_H - 4.0) / 2.0
}

impl TreeView {
    /// Put the rows into `order` (old row indices) and renumber links.
    fn reorder(&mut self, order: &[usize]) {
        let mut new_of = vec![0; order.len()];
        for (new, &old) in order.iter().enumerate() {
            new_of[old] = new;
        }
        let pick = |v: &[u32]| order.iter().map(|&o| v[o]).collect::<Vec<_>>();
        self.rows = order.iter().map(|&o| self.rows[o].clone()).collect();
        self.incoming = pick(&self.incoming);
        self.outgoing = pick(&self.outgoing);
        self.in_cycle = order.iter().map(|&o| self.in_cycle[o]).collect();
        for l in &mut self.links {
            l.from = new_of[l.from];
            l.to = new_of[l.to];
        }
        self.links.sort_by_key(|l| (l.from, l.to));
    }

    /// Colour class of shown link `i`: cycles red without a selection or
    /// in "cycles only", otherwise the direction colour of its lane.
    pub fn link_kind(
        &self,
        i: usize,
        lane: Lane,
        selected: Option<usize>,
        mode: EdgeMode,
    ) -> LinkKind {
        match (selected, lane) {
            _ if self.links[i].cyclic && (selected.is_none() || mode == EdgeMode::Cycles) => {
                LinkKind::Cyclic
            }
            (None, _) | (Some(_), Lane::Other) => LinkKind::Neutral,
            (Some(_), Lane::Incoming) => LinkKind::Incoming,
            (Some(_), Lane::Outgoing) => LinkKind::Outgoing,
        }
    }

    /// Paths for every link shown under `selected` and `mode`.
    pub fn draw(&self, selected: Option<usize>, mode: EdgeMode) -> Vec<Drawn> {
        self.shown(selected, mode)
            .into_iter()
            .map(|(i, lane)| {
                let l = &self.links[i];
                // Left lane in absolute coordinates; the right lane relative
                // to its own left edge (the tree's right edge).
                let (x, right) = match lane {
                    Lane::Incoming => (LANE_W - 22.0, false),
                    Lane::Outgoing | Lane::Other => (22.0, true),
                };
                Drawn {
                    path: arc(x, row_center(l.from), row_center(l.to), lane, LANE_W - 30.0),
                    kind: self.link_kind(i, lane, selected, mode),
                    from: l.from,
                    to: l.to,
                    right,
                }
            })
            .collect()
    }
}

/// Arc between two row centres in a lane: it leaves and enters at
/// `x` and bulges outwards (left for the incoming lane) the further the
/// rows are apart. Ends with an arrow head at the target.
pub fn arc(x: f32, from_y: f32, to_y: f32, lane: Lane, lane_width: f32) -> String {
    let side = match lane {
        Lane::Outgoing | Lane::Other => 1.0,
        Lane::Incoming => -1.0,
    };
    let reach = (14.0 + (to_y - from_y).abs() * 0.3).min(lane_width - 10.0);
    let bx = x + side * reach;
    let wing = x + side * 7.0;
    format!(
        "M {x:.1} {from_y:.1} C {bx:.1} {from_y:.1} {bx:.1} {to_y:.1} {x:.1} {to_y:.1} \
         M {wing:.1} {:.1} L {x:.1} {to_y:.1} L {wing:.1} {:.1}",
        to_y - 4.5,
        to_y + 4.5
    )
}

/// Sort link endpoints (view ids) as the web viewer sorts its link keys
/// `"<from> <to>"`: as strings, which decides the sample on large scopes.
pub fn web_link_keys(links: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let mut keyed: Vec<(String, (usize, usize))> = links
        .iter()
        .map(|&(a, b)| (format!("{a} {b}"), (a, b)))
        .collect();
    keyed.sort();
    keyed.into_iter().map(|(_, l)| l).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::{Edge, Node, NodeId, Ref};

    /// a/{x.rs, y.rs}, b/{z.rs}, c/{w.rs}; x→z (2), y→z (1), z→w, w→z.
    fn graph() -> Graph {
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
        Graph {
            schema_version: 1,
            root: ".".into(),
            nodes: vec![
                node(0, "a", NodeKind::Module, None),
                node(1, "a/x.rs", NodeKind::File, Some(0)),
                node(2, "a/y.rs", NodeKind::File, Some(0)),
                node(3, "b", NodeKind::Module, None),
                node(4, "b/z.rs", NodeKind::File, Some(3)),
                node(5, "c", NodeKind::Module, None),
                node(6, "c/w.rs", NodeKind::File, Some(5)),
            ],
            edges: vec![edge(1, 4, 2), edge(2, 4, 1), edge(4, 6, 1), edge(6, 4, 1)],
            semantic: None,
            review_kpis: None,
        }
    }

    fn names(t: &Tree, v: &TreeView) -> Vec<String> {
        v.rows
            .iter()
            .map(|r| t.nodes[r.node].name.clone())
            .collect()
    }

    /// The nine crates of this repository with their 13 root-level
    /// dependencies, as the web viewer showed them.
    fn crates() -> Graph {
        let names = [
            "archguard-analysis",
            "archguard-cli",
            "archguard-core",
            "archguard-desktop",
            "archguard-engine",
            "archguard-extract",
            "archguard-mcp",
            "archguard-report",
            "archguard-store",
        ];
        let id = |n: &str| names.iter().position(|x| x.ends_with(n)).unwrap() as u32;
        let deps = [
            ("analysis", "core"),
            ("cli", "engine"),
            ("cli", "mcp"),
            ("desktop", "engine"),
            ("engine", "analysis"),
            ("engine", "core"),
            ("engine", "extract"),
            ("engine", "report"),
            ("engine", "store"),
            ("extract", "core"),
            ("mcp", "engine"),
            ("report", "core"),
            ("store", "core"),
        ];
        Graph {
            schema_version: 1,
            root: ".".into(),
            nodes: names
                .iter()
                .enumerate()
                .map(|(i, n)| Node {
                    id: NodeId(i as u32),
                    name: n.to_string(),
                    kind: NodeKind::Module,
                    parent: None,
                    lang: None,
                })
                .collect(),
            edges: deps
                .iter()
                .map(|(a, b)| Edge {
                    from: NodeId(id(a)),
                    to: NodeId(id(b)),
                    refs: Vec::new(),
                    count: Some(1),
                })
                .collect(),
            semantic: None,
            review_kpis: None,
        }
    }

    #[test]
    fn connection_order_matches_the_web_viewer() {
        let t = Tree::from_graph(&crates());
        let v = t.view_sorted(&BTreeSet::new(), Sort::Connections);
        let labels: Vec<&str> = v
            .rows
            .iter()
            .map(|r| t.nodes[r.node].name.trim_start_matches("archguard-"))
            .collect();
        assert_eq!(
            labels,
            [
                "analysis", "cli", "mcp", "desktop", "engine", "extract", "report", "core", "store"
            ]
        );
        // Links follow their rows.
        let mcp_to_engine = v.links.iter().any(|l| l.from == 2 && l.to == 4);
        assert!(mcp_to_engine);
    }

    #[test]
    fn sampling_follows_the_web_link_keys() {
        // View ids are name ranks; keys sort as strings ("10 2" < "2 3").
        let keys = web_link_keys(&[(2, 3), (10, 2), (1, 0)]);
        assert_eq!(keys, [(1, 0), (10, 2), (2, 3)]);
    }

    #[test]
    fn alphabetical_order_is_still_available() {
        let t = Tree::from_graph(&crates());
        let v = t.view_sorted(&BTreeSet::new(), Sort::Alphabetical);
        assert_eq!(t.nodes[v.rows[2].node].name, "archguard-core");
    }

    #[test]
    fn collapsed_tree_shows_roots_and_aggregates_links() {
        let t = Tree::from_graph(&graph());
        let v = t.view(&BTreeSet::new());
        assert_eq!(names(&t, &v), ["a", "b", "c"]);
        assert!(v.rows.iter().all(|r| r.expandable && !r.expanded));
        let a_to_b = v.links.iter().find(|l| l.from == 0 && l.to == 1).unwrap();
        assert_eq!(a_to_b.weight, 3, "x→z and y→z merge");
        assert!(!a_to_b.cyclic);
        assert_eq!(v.outgoing, [1, 1, 1]);
        assert_eq!(v.incoming, [0, 2, 1]);
    }

    #[test]
    fn expanding_shows_children_below_their_parent() {
        let t = Tree::from_graph(&graph());
        let v = t.view(&BTreeSet::from([0]));
        assert_eq!(names(&t, &v), ["a", "a/x.rs", "a/y.rs", "b", "c"]);
        assert_eq!(v.rows[1].depth, 1);
        assert!(v.rows[0].expanded && !v.rows[1].expandable);
        let into_b: Vec<(usize, u64)> = v
            .links
            .iter()
            .filter(|l| l.to == 3)
            .map(|l| (l.from, l.weight))
            .collect();
        // x→z and y→z now start at their own rows; c→b stays.
        assert_eq!(into_b, [(1, 2), (2, 1), (4, 1)]);
        assert_eq!(v.incoming[0], 0, "an expanded row has no links of its own");
    }

    #[test]
    fn cycles_mark_links_rows_and_groups() {
        let t = Tree::from_graph(&graph());
        let v = t.view(&BTreeSet::new());
        let cyclic: Vec<(usize, usize)> = v
            .links
            .iter()
            .filter(|l| l.cyclic)
            .map(|l| (l.from, l.to))
            .collect();
        assert_eq!(cyclic, [(1, 2), (2, 1)]);
        assert_eq!(v.in_cycle, [false, true, true]);
        assert_eq!(v.cycle_groups, 1);
    }

    #[test]
    fn selecting_an_expanded_row_covers_its_children() {
        let t = Tree::from_graph(&graph());
        let v = t.view(&BTreeSet::from([0]));
        assert_eq!(v.selection(0), BTreeSet::from([0, 1, 2]));
        let shown = v.shown(Some(0), EdgeMode::Both);
        assert_eq!(shown.len(), 2);
        assert!(shown.iter().all(|&(_, lane)| lane == Lane::Outgoing));
    }

    #[test]
    fn edge_modes_split_incoming_and_outgoing() {
        let t = Tree::from_graph(&graph());
        let v = t.view(&BTreeSet::new());
        let lanes = |mode| -> Vec<(usize, usize, Lane)> {
            v.shown(Some(1), mode)
                .into_iter()
                .map(|(i, lane)| (v.links[i].from, v.links[i].to, lane))
                .collect()
        };
        assert_eq!(
            lanes(EdgeMode::Both),
            [
                (0, 1, Lane::Incoming),
                (1, 2, Lane::Outgoing),
                (2, 1, Lane::Incoming)
            ]
        );
        assert_eq!(lanes(EdgeMode::Incoming).len(), 2);
        assert_eq!(lanes(EdgeMode::Outgoing).len(), 1);
        assert_eq!(v.shown(None, EdgeMode::All).len(), v.links.len());
        assert!(
            v.shown(None, EdgeMode::Cycles)
                .iter()
                .all(|&(i, _)| v.links[i].cyclic)
        );
    }

    #[test]
    fn counts_descendant_files_and_direct_packages() {
        let t = Tree::from_graph(&graph());
        assert_eq!(t.counts(0), (2, 0), "a holds x.rs and y.rs");
        assert_eq!(t.counts(1), (0, 0), "a file has no children");
    }

    #[test]
    fn links_carry_merged_source_evidence() {
        let mut g = graph();
        g.edges[0].refs = vec![
            Ref {
                file: "a/x.rs".into(),
                line: 9,
                raw: "use b".into(),
            },
            Ref {
                file: "a/x.rs".into(),
                line: 2,
                raw: "use b".into(),
            },
        ];
        g.edges[1].refs = vec![Ref {
            file: "a/x.rs".into(),
            line: 2,
            raw: "use b".into(),
        }];
        let t = Tree::from_graph(&g);
        let v = t.view(&BTreeSet::new());
        let a_to_b = v.links.iter().find(|l| l.from == 0 && l.to == 1).unwrap();
        let lines: Vec<u32> = a_to_b.refs.iter().map(|r| r.line).collect();
        assert_eq!(lines, [2, 9], "deduplicated and sorted");
        assert_eq!(a_to_b.weight, 3, "the count stays the sum of the edges");
    }

    #[test]
    fn scope_excludes_expanded_containers() {
        // Expanded `a` carries no links; the web counts only x, y, b, c.
        let t = Tree::from_graph(&graph());
        let v = t.view(&BTreeSet::from([0]));
        assert_eq!(v.rows.len(), 5);
        assert_eq!(v.scope_rows(), [1, 2, 3, 4]);
        // x→b, y→b, b↔c over 4 nodes: reach 3 + 3 + 2 + 2 = 10.
        assert!((v.acd(&t) - 10.0 / 4.0).abs() < 1e-9, "{}", v.acd(&t));
        // Cycle groups share ACD's base: the scope rows. The expanded
        // container has no links, so it never joins a group.
        let scope = v.as_graph(&t);
        let groups = crate::tree_order::scc(
            v.rows.len(),
            scope
                .edges
                .iter()
                .map(|e| (e.from.0 as usize, e.to.0 as usize)),
        );
        assert_eq!(
            v.cycle_groups,
            groups.iter().filter(|g| g.len() > 1).count()
        );
        assert_eq!(v.cycle_groups, 1, "b↔c");
    }

    #[test]
    fn both_also_shows_links_inside_the_selection() {
        let mut g = graph();
        g.edges.push(archguard_engine::core::Edge {
            from: NodeId(1),
            to: NodeId(2),
            refs: Vec::new(),
            count: Some(1),
        });
        let t = Tree::from_graph(&g);
        let v = t.view(&BTreeSet::from([0]));
        // Selecting `a` covers x and y; x→y is internal.
        let both = v.shown(Some(0), EdgeMode::Both);
        assert_eq!(both.len(), 3, "x→b, y→b and the internal x→y");
        let internal = both.iter().find(|&&(i, _)| v.links[i].to == 2).unwrap();
        assert_eq!(
            v.link_kind(internal.0, internal.1, Some(0), EdgeMode::Both),
            LinkKind::Neutral
        );
        assert_eq!(
            v.shown(Some(0), EdgeMode::Outgoing).len(),
            2,
            "internal links only in Both"
        );
    }

    #[test]
    fn acd_is_measured_on_the_visible_rows() {
        // a→b, b↔c: reach(a) = 3, reach(b) = reach(c) = 2 → 7 / 3.
        let t = Tree::from_graph(&graph());
        let v = t.view(&BTreeSet::new());
        assert!((v.acd(&t) - 7.0 / 3.0).abs() < 1e-9, "{}", v.acd(&t));
    }

    #[test]
    fn drawn_links_use_their_lane_and_colour() {
        let t = Tree::from_graph(&graph());
        let v = t.view(&BTreeSet::new());
        let none = v.draw(None, EdgeMode::All);
        assert_eq!(none.len(), 3);
        assert_eq!(none[0].kind, LinkKind::Neutral);
        assert_eq!(none[1].kind, LinkKind::Cyclic);
        // The right lane is drawn relative to its own origin, so the tree
        // between the lanes can take any width.
        assert!(
            none.iter()
                .all(|d| d.right && d.path.starts_with("M 22.0 "))
        );
        assert_eq!((none[0].from, none[0].to), (0, 1), "endpoints for hover");
        let sel = v.draw(Some(1), EdgeMode::Both);
        let kinds: Vec<LinkKind> = sel.iter().map(|d| d.kind).collect();
        assert_eq!(
            kinds,
            [LinkKind::Incoming, LinkKind::Outgoing, LinkKind::Incoming]
        );
        assert!(!sel[0].right);
        assert!(sel[0].path.starts_with(&format!("M {:.1} ", LANE_W - 22.0)));
    }

    #[test]
    fn arcs_bulge_outwards_and_grow_with_distance() {
        let right = arc(100.0, 10.0, 90.0, Lane::Outgoing, 200.0);
        assert!(right.starts_with("M 100.0 10.0 C "));
        let far = arc(100.0, 10.0, 400.0, Lane::Outgoing, 200.0);
        let bulge = |p: &str| -> f32 { p.split(' ').nth(4).unwrap().parse().unwrap() };
        assert!(bulge(&right) > 100.0 && bulge(&far) > bulge(&right));
        let left = arc(50.0, 10.0, 90.0, Lane::Incoming, 200.0);
        assert!(bulge(&left) < 50.0);
    }
}
