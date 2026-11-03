//! Views & zoom (T35, Kap. 5.2): the [`Cut`] through the containment tree
//! and [`aggregate`], which projects a [`Graph`] onto a cut and returns a
//! new [`Graph`] — which is why every analysis runs unchanged at every
//! zoom level.
//!
//! A cut is a set of visible nodes such that every file node has exactly
//! one visible ancestor-or-self (the default is the depth from
//! `modules.depth`). Zooming in [`Cut::expand`]s a node into its children,
//! zooming out [`Cut::collapse`]s a subtree back onto its root. The
//! operations are stateless: clients resend the cut, the server keeps no
//! session.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::model::{ArchguardError, Edge, Graph, Node, NodeId, NodeKind, Ref};

/// Upper bound of [`Ref`]s kept as evidence sample on one aggregated edge.
const MAX_SAMPLE_REFS: usize = 10;

/// A view onto the containment tree: the set of visible node ids (Kap. 5.2).
///
/// A [`Cut`] is valid for a [`Graph`] when every file node of that graph
/// has exactly one visible ancestor-or-self ([`Cut::validate`]). All
/// constructors and combinators preserve validity for valid inputs:
/// expand/collapse of a node outside the cut (or without children) are
/// documented no-ops rather than silent corruptions.
///
/// `BTreeSet` keeps iteration and serialization deterministic (Kap. 12);
/// in JSON the cut is transparently an array of node ids.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Cut {
    nodes: BTreeSet<NodeId>,
}

impl Cut {
    /// A cut from any iterator of visible node ids.
    pub fn new(nodes: impl IntoIterator<Item = NodeId>) -> Self {
        Self {
            nodes: nodes.into_iter().collect(),
        }
    }

    /// The visible node ids, in deterministic (id) order.
    pub fn nodes(&self) -> &BTreeSet<NodeId> {
        &self.nodes
    }

    /// Whether `id` is visible in this cut.
    pub fn contains(&self, id: NodeId) -> bool {
        self.nodes.contains(&id)
    }

    /// Number of visible nodes.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether no node is visible.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The default cut at tree depth `d` (roots are depth 0): every node
    /// at depth `d` plus every shallower childless node.
    ///
    /// Every file node then has exactly one visible ancestor-or-self:
    /// itself when it sits at or above `d` (files are always leaves), or
    /// its unique ancestor at depth `d` when it sits below `d`. Tree
    /// modules above `d` that still have children stay hidden, matching
    /// the `modules.depth` default (Kap. 5.2).
    pub fn at_depth(g: &Graph, d: usize) -> Self {
        let mut nodes = BTreeSet::new();
        for node in &g.nodes {
            let depth = g.ancestors_of(node.id).len();
            if depth == d || (depth < d && g.children_of(node.id).is_empty()) {
                nodes.insert(node.id);
            }
        }
        Self { nodes }
    }

    /// Zoom in: replace the visible `node` by its children.
    ///
    /// No-op (clone of `self`) when `node` is not visible in this cut or
    /// has no children — expanding a file node, for instance, would
    /// otherwise silently drop its only visible node.
    pub fn expand(&self, g: &Graph, node: NodeId) -> Cut {
        if !self.contains(node) {
            return self.clone();
        }
        let children = g.children_of(node);
        if children.is_empty() {
            return self.clone();
        }
        let mut nodes = self.nodes.clone();
        nodes.remove(&node);
        nodes.extend(children);
        Cut { nodes }
    }

    /// Zoom out: replace every visible node inside the subtree of `node`
    /// (including `node` itself) by `node` — the exact inverse of
    /// [`Self::expand`] on a visible node with children.
    ///
    /// No-op when the subtree of `node` holds no visible node at all:
    /// `node` is already fully covered by one of its own visible
    /// ancestors and adding it would give those files two visible nodes.
    pub fn collapse(&self, g: &Graph, node: NodeId) -> Cut {
        let hidden: Vec<NodeId> = self
            .nodes
            .iter()
            .filter(|visible| in_subtree_of(g, **visible, node))
            .copied()
            .collect();
        if hidden.is_empty() {
            return self.clone();
        }
        let mut nodes = self.nodes.clone();
        for visible in hidden {
            nodes.remove(&visible);
        }
        nodes.insert(node);
        Cut { nodes }
    }

    /// Check the cut invariant against `g` (Kap. 5.2): every id must name
    /// an existing node, and every file node must have exactly one
    /// visible ancestor-or-self.
    pub fn validate(&self, g: &Graph) -> Result<(), ArchguardError> {
        for id in &self.nodes {
            if g.node_name(*id).is_none() {
                return Err(ArchguardError::Analysis(format!(
                    "unknown node id {} in cut",
                    id.0
                )));
            }
        }
        for node in &g.nodes {
            if node.kind != NodeKind::File {
                continue;
            }
            let visible = visible_chain(g, self, node.id).len();
            match visible {
                1 => {}
                0 => {
                    return Err(ArchguardError::Analysis(format!(
                        "file node {} has no visible node in cut",
                        node.name
                    )));
                }
                count => {
                    return Err(ArchguardError::Analysis(format!(
                        "file node {} has {} visible nodes in cut",
                        node.name, count
                    )));
                }
            }
        }
        Ok(())
    }
}

/// Project `g` onto `cut` and return the view as a new [`Graph`]
/// (Kap. 5.2).
///
/// Every edge endpoint maps to its nearest visible ancestor-or-self;
/// edges whose endpoints collapse onto one node are dropped, parallel
/// edges are merged by summing their refs into one evidence sample,
/// deduplicated and sorted by `(file, line)` (ties broken by `raw`) and
/// capped at 10 [`Ref`]s. The result is flat (all `parent`s `None`), its
/// nodes are sorted by name and renumbered `0..n`, and its edges are
/// sorted by `(from, to)` — so the same input always yields a
/// byte-identical graph (Kap. 12).
///
/// Edge endpoints that have no visible ancestor-or-self (a broken cut)
/// simply produce no edge; use [`Cut::validate`] to catch that case.
///
/// F12: the merged [`Edge::count`] is the sum of the contributors'
/// counts (the architecture's "Ref-Counts summiert") and stays exact
/// across repeated aggregations, even though the evidence sample in
/// `refs` is deduplicated and capped at 10; a legacy input edge
/// without an exact count makes the merged count `None` instead of
/// inventing a total from the truncated samples.
pub fn aggregate(g: &Graph, cut: &Cut) -> Graph {
    // Visible nodes sorted by name, renumbered densely (canonical order,
    // same convention as GraphBuilder::build).
    let mut visible: Vec<&Node> = g
        .nodes
        .iter()
        .filter(|node| cut.contains(node.id))
        .collect();
    visible.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));

    let mut id_map: BTreeMap<NodeId, NodeId> = BTreeMap::new();
    let nodes: Vec<Node> = visible
        .iter()
        .enumerate()
        .map(|(index, node)| {
            let new_id = NodeId(index as u32);
            id_map.insert(node.id, new_id);
            Node {
                id: new_id,
                name: node.name.clone(),
                kind: node.kind,
                parent: None,
                lang: node.lang.clone(),
            }
        })
        .collect();

    // Project every edge onto its visible ancestors; `BTreeMap` keyed by
    // (from, to) merges parallel edges in deterministic order. The
    // total-count slot sums contributor counts (F12) and turns `None`
    // as soon as one contributor lacks an exact total; only the sample
    // refs are deduplicated and capped below.
    let mut merged: BTreeMap<(u32, u32), (Option<u64>, Vec<Ref>)> = BTreeMap::new();
    for edge in &g.edges {
        let (Some(from), Some(to)) = (
            nearest_visible(g, cut, edge.from).and_then(|v| id_map.get(&v).copied()),
            nearest_visible(g, cut, edge.to).and_then(|v| id_map.get(&v).copied()),
        ) else {
            continue;
        };
        if from == to {
            continue;
        }
        let slot = merged
            .entry((from.0, to.0))
            .or_insert_with(|| (Some(0u64), Vec::new()));
        slot.0 = match (slot.0, edge.count) {
            (Some(sum), Some(count)) => Some(sum + count),
            _ => None,
        };
        slot.1.extend(edge.refs.iter().cloned());
    }

    let edges = merged
        .into_iter()
        .map(|((from, to), (count, mut refs))| {
            refs.sort_by(|a, b| (&a.file, a.line, &a.raw).cmp(&(&b.file, b.line, &b.raw)));
            refs.dedup();
            refs.truncate(MAX_SAMPLE_REFS);
            Edge {
                from: NodeId(from),
                to: NodeId(to),
                refs,
                count,
            }
        })
        .collect();

    Graph {
        semantic: g.semantic.clone(),
        // A projection carries no KPIs: they describe the full graph and
        // are (re)attached by the pipeline, never by aggregation.
        review_kpis: None,
        schema_version: g.schema_version,
        root: g.root.clone(),
        nodes,
        edges,
    }
}

/// `id` itself plus all its ancestors, nearest first.
fn visible_chain(g: &Graph, cut: &Cut, id: NodeId) -> Vec<NodeId> {
    let mut chain = vec![id];
    chain.extend(g.ancestors_of(id));
    chain.retain(|candidate| cut.contains(*candidate));
    chain
}

/// Nearest visible ancestor-or-self of `id`, `None` if the cut covers
/// neither `id` nor any of its ancestors.
fn nearest_visible(g: &Graph, cut: &Cut, id: NodeId) -> Option<NodeId> {
    let mut chain = vec![id];
    chain.extend(g.ancestors_of(id));
    chain.into_iter().find(|candidate| cut.contains(*candidate))
}

/// Whether `id` is `root` itself or a descendant of `root`.
fn in_subtree_of(g: &Graph, id: NodeId, root: NodeId) -> bool {
    id == root || g.ancestors_of(id).contains(&root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::GraphBuilder;

    /// Name-sorted tree: `app/billing/invoice.ts` and `app/tax/tax.ts`
    /// import `domain/money.ts`, invoice imports tax, and the root file
    /// `c.ts` imports invoice. Covers roots, deep modules and a root file.
    fn fixture() -> Graph {
        let mut builder = GraphBuilder::new();
        let invoice = builder.ensure_path("app/billing/invoice.ts", Some("ts"));
        let tax = builder.ensure_path("app/tax/tax.ts", Some("ts"));
        let money = builder.ensure_path("domain/money.ts", Some("ts"));
        let root_file = builder.ensure_path("c.ts", Some("ts"));
        let mut edge = |from: NodeId, to: NodeId, file: &str, line: u32| {
            builder
                .edge(
                    from,
                    to,
                    Ref {
                        file: file.to_string(),
                        line,
                        raw: format!("import x from \"y{line}\";"),
                    },
                )
                .expect("test: valid edge");
        };
        edge(invoice, money, "app/billing/invoice.ts", 4);
        edge(invoice, money, "app/billing/invoice.ts", 9); // merges into same edge
        edge(tax, money, "app/tax/tax.ts", 2);
        edge(invoice, tax, "app/billing/invoice.ts", 1);
        edge(root_file, invoice, "c.ts", 1);
        builder.build("/abs/projekt".to_string())
    }

    fn id(g: &Graph, name: &str) -> NodeId {
        g.nodes
            .iter()
            .find(|n| n.name == name)
            .unwrap_or_else(|| panic!("test: no node {name}"))
            .id
    }

    /// Visible ids of a cut as sorted names.
    fn visible_names(g: &Graph, cut: &Cut) -> Vec<String> {
        let mut names: Vec<String> = cut
            .nodes()
            .iter()
            .filter_map(|i| g.node_name(*i))
            .map(str::to_string)
            .collect();
        names.sort();
        names
    }

    fn names(values: &[&str]) -> Vec<String> {
        let mut v: Vec<String> = values.iter().map(|s| s.to_string()).collect();
        v.sort();
        v
    }

    /// Aggregated edges as `(from name, to name, ref lines)` in graph order.
    fn edge_lines(view: &Graph) -> Vec<(String, String, Vec<u32>)> {
        view.edges
            .iter()
            .map(|e| {
                (
                    view.node_name(e.from).expect("test: known").to_string(),
                    view.node_name(e.to).expect("test: known").to_string(),
                    e.refs.iter().map(|r| r.line).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn at_depth_covers_every_expected_zoom_level() {
        let g = fixture();
        // (depth, expected visible names) — table-driven.
        // Depths in the fixture: app/c.ts/domain = 0, app/billing, app/tax and
        // domain/money.ts = 1, invoice.ts and tax.ts = 2.
        let cases: Vec<(usize, Vec<String>)> = vec![
            (0, names(&["app", "c.ts", "domain"])),
            (
                1,
                names(&["app/billing", "app/tax", "c.ts", "domain/money.ts"]),
            ),
            (
                2,
                names(&[
                    "app/billing/invoice.ts",
                    "app/tax/tax.ts",
                    "c.ts",
                    "domain/money.ts",
                ]),
            ),
            // Deeper than the tree: every file is a childless shallow node.
            (
                5,
                names(&[
                    "app/billing/invoice.ts",
                    "app/tax/tax.ts",
                    "c.ts",
                    "domain/money.ts",
                ]),
            ),
        ];
        for (depth, expected) in cases {
            let cut = Cut::at_depth(&g, depth);
            assert_eq!(visible_names(&g, &cut), expected, "depth {depth}");
            assert!(cut.validate(&g).is_ok(), "depth {depth} must be valid");
        }
    }

    #[test]
    fn expand_replaces_visible_node_by_children_and_collapse_inverts_it() {
        let g = fixture();
        // (zoom level, node to expand, expected visible names afterwards)
        let cases: Vec<(usize, &str, Vec<String>)> = vec![
            (
                1,
                "app/billing",
                names(&[
                    "app/billing/invoice.ts",
                    "app/tax",
                    "c.ts",
                    "domain/money.ts",
                ]),
            ),
            (
                0,
                "app",
                names(&["app/billing", "app/tax", "c.ts", "domain"]),
            ),
            (0, "domain", names(&["app", "c.ts", "domain/money.ts"])),
            (
                1,
                "c.ts", // file node without children: no-op
                names(&["app/billing", "app/tax", "c.ts", "domain/money.ts"]),
            ),
            (
                1,
                "domain/money.ts", // visible file without children: no-op
                names(&["app/billing", "app/tax", "c.ts", "domain/money.ts"]),
            ),
            (
                1,
                "app/billing/invoice.ts", // not visible at depth 1: no-op
                names(&["app/billing", "app/tax", "c.ts", "domain/money.ts"]),
            ),
        ];
        for (depth, node, expected) in cases {
            let base = Cut::at_depth(&g, depth);
            let zoomed = base.expand(&g, id(&g, node));
            assert_eq!(visible_names(&g, &zoomed), expected, "expand {node}");
            assert!(zoomed.validate(&g).is_ok(), "expand {node} stays valid");
            // Round trip: expanding and collapsing a visible node is the identity.
            let back = zoomed.collapse(&g, id(&g, node));
            assert_eq!(back, base, "collapse after expand of {node}");
        }
    }

    #[test]
    fn collapse_gathers_a_whole_subtree_onto_its_root() {
        let g = fixture();
        // Depth 1 shows `app`'s two children (and `domain/money.ts`, which
        // sits at depth 1); collapsing `app` gathers both back onto `app`.
        let base = Cut::at_depth(&g, 1);
        let collapsed = base.collapse(&g, id(&g, "app"));
        assert_eq!(
            visible_names(&g, &collapsed),
            names(&["app", "c.ts", "domain/money.ts"])
        );
        assert!(collapsed.validate(&g).is_ok());

        // Collapsing a node whose subtree holds no visible node is a no-op:
        // `c.ts` is covered by itself, so collapsing it changes nothing.
        assert_eq!(base.collapse(&g, id(&g, "c.ts")), base);
    }

    #[test]
    fn expand_order_is_commutative() {
        let g = fixture();
        let base = Cut::at_depth(&g, 0);
        let a = id(&g, "app");
        let b = id(&g, "domain");
        assert_eq!(
            base.expand(&g, a).expand(&g, b),
            base.expand(&g, b).expand(&g, a)
        );
    }

    /// (cut depth, expected node names, expected `(from, to, ref lines)`)
    type AggCase = (
        usize,
        Vec<String>,
        Vec<(&'static str, &'static str, Vec<u32>)>,
    );

    #[test]
    fn aggregate_projects_edges_and_keeps_the_view_flat() {
        let g = fixture();
        let cases: Vec<AggCase> = vec![
            (
                1,
                names(&["app/billing", "app/tax", "c.ts", "domain/money.ts"]),
                vec![
                    ("app/billing", "app/tax", vec![1]),
                    ("app/billing", "domain/money.ts", vec![4, 9]),
                    ("app/tax", "domain/money.ts", vec![2]),
                    ("c.ts", "app/billing", vec![1]),
                ],
            ),
            (
                // `app` swallows both app files: the invoice→tax edge becomes a
                // self-edge (dropped), and the two edges into `domain` merge
                // with summed refs, sorted by (file, line): invoice.ts lines
                // 4+9 come before tax.ts line 2.
                0,
                names(&["app", "c.ts", "domain"]),
                vec![("app", "domain", vec![4, 9, 2]), ("c.ts", "app", vec![1])],
            ),
            (
                // File level: the cut is all files, so aggregation is identity
                // up to renumbering; parallel input edges were already merged
                // by the builder.
                5,
                names(&[
                    "app/billing/invoice.ts",
                    "app/tax/tax.ts",
                    "c.ts",
                    "domain/money.ts",
                ]),
                vec![
                    ("app/billing/invoice.ts", "app/tax/tax.ts", vec![1]),
                    ("app/billing/invoice.ts", "domain/money.ts", vec![4, 9]),
                    ("app/tax/tax.ts", "domain/money.ts", vec![2]),
                    ("c.ts", "app/billing/invoice.ts", vec![1]),
                ],
            ),
        ];
        for (depth, expected_nodes, expected_edges) in cases {
            let cut = Cut::at_depth(&g, depth);
            let view = aggregate(&g, &cut);

            // Flat view: dense ids `0..n` in name order, no parents.
            let node_names: Vec<&str> = view.nodes.iter().map(|n| n.name.as_str()).collect();
            let mut sorted = node_names.clone();
            sorted.sort_unstable();
            assert_eq!(node_names, sorted, "depth {depth}: nodes sorted by name");
            assert!(
                view.nodes
                    .iter()
                    .enumerate()
                    .all(|(i, n)| n.id == NodeId(i as u32)),
                "depth {depth}: dense ids"
            );
            assert!(view.nodes.iter().all(|n| n.parent.is_none()));
            assert_eq!(view.root, g.root);
            assert_eq!(view.schema_version, g.schema_version);
            let mut expected_nodes_sorted = expected_nodes.clone();
            expected_nodes_sorted.sort();
            assert_eq!(
                node_names,
                expected_nodes_sorted
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
            );

            // Edges sorted by (from, to) with merged, sorted sample refs.
            let position = |name: &str| {
                view.nodes
                    .iter()
                    .position(|n| n.name == name)
                    .expect("test: visible node exists")
            };
            let mut expected: Vec<(String, String, Vec<u32>)> = expected_edges
                .iter()
                .map(|(f, t, lines)| (f.to_string(), t.to_string(), lines.clone()))
                .collect();
            expected.sort_by_key(|(f, t, _)| (position(f), position(t)));
            assert_eq!(edge_lines(&view), expected, "depth {depth}");
        }
    }

    #[test]
    fn aggregate_caps_the_sample_at_ten_refs() {
        let mut builder = GraphBuilder::new();
        let x = builder.ensure_path("a/x.ts", Some("ts"));
        let y = builder.ensure_path("b/y.ts", Some("ts"));
        for line in 1..=12 {
            builder
                .edge(
                    x,
                    y,
                    Ref {
                        file: "a/x.ts".to_string(),
                        line,
                        raw: format!("import y{line};"),
                    },
                )
                .expect("test: valid edge");
        }
        let g = builder.build("/abs".to_string());
        let view = aggregate(&g, &Cut::at_depth(&g, 0));
        assert_eq!(view.edges.len(), 1);
        assert_eq!(view.edges[0].refs.len(), 10);
        let lines: Vec<u32> = view.edges[0].refs.iter().map(|r| r.line).collect();
        assert_eq!(lines, (1..=10).collect::<Vec<u32>>());
        // F12: the sample is capped, the total is not.
        assert_eq!(view.edges[0].count, Some(12));
        assert_eq!(view.edges[0].weight(), 12);
    }

    #[test]
    fn a_hundred_import_edge_keeps_its_total_at_every_view() {
        // F12 acceptance: count stays 100 with ≤10 samples at file
        // level (identity aggregation truncates the sample) and after
        // the module-level aggregation.
        let mut builder = GraphBuilder::new();
        let x = builder.ensure_path("a/x.ts", Some("ts"));
        let y = builder.ensure_path("b/y.ts", Some("ts"));
        for line in 1..=100 {
            builder
                .edge(
                    x,
                    y,
                    Ref {
                        file: "a/x.ts".to_string(),
                        line,
                        raw: format!("import y{line};"),
                    },
                )
                .expect("test: valid edge");
        }
        let g = builder.build("/abs".to_string());
        assert_eq!(g.edges[0].count, Some(100));

        // Deepest cut: only the files are visible (identity level).
        let file_level = Cut::at_depth(&g, 1);
        assert!(file_level.validate(&g).is_ok());
        let view = aggregate(&g, &file_level);
        assert_eq!(view.edges[0].refs.len(), 10, "sample capped");
        assert_eq!(view.edges[0].weight(), 100, "total exact");

        let module_level = aggregate(&g, &Cut::at_depth(&g, 0));
        assert_eq!(module_level.edges[0].weight(), 100);
        assert_eq!(module_level.edges[0].refs.len(), 10);
    }

    #[test]
    fn counts_are_summed_and_stay_exact_across_repeated_aggregation() {
        // Two files under a/ import two files under b/ with 12 and 5
        // refs: the module-level edge must carry the sum 17 even though
        // the merged sample is deduplicated, and re-aggregating the
        // view must not lose or double anything.
        let mut builder = GraphBuilder::new();
        let x1 = builder.ensure_path("a/x1.ts", Some("ts"));
        let x2 = builder.ensure_path("a/x2.ts", Some("ts"));
        let y = builder.ensure_path("b/y.ts", Some("ts"));
        let mut refs = |from: NodeId, to: NodeId, n: u32| {
            for line in 1..=n {
                builder
                    .edge(
                        from,
                        to,
                        Ref {
                            file: "shared.ts".to_string(),
                            line,
                            raw: "import y;".to_string(),
                        },
                    )
                    .expect("test: valid edge");
            }
        };
        refs(x1, y, 12);
        refs(x2, y, 5);
        let g = builder.build("/abs".to_string());

        let view = aggregate(&g, &Cut::at_depth(&g, 0));
        assert_eq!(view.edges.len(), 1);
        assert_eq!(view.edges[0].count, Some(17));
        // The two contributors share file+line space, so the deduped
        // sample is smaller than the sum of the totals.
        assert!(view.edges[0].refs.len() <= 12);

        // Aggregate the aggregated view: identity cut re-aggregates
        // every node onto itself — the total must survive unchanged.
        let again = aggregate(&view, &Cut::new(view.nodes.iter().map(|n| n.id)));
        assert_eq!(again.edges[0].count, Some(17));
        assert_eq!(again.edges[0].weight(), 17);
    }

    #[test]
    fn legacy_edges_without_total_stay_without_invented_count() {
        // Hand-built (legacy-shaped) edges carry no `count`; merging
        // them must yield `None`, never a total assembled from
        // truncated samples.
        let node = |id: u32, name: &str, kind: NodeKind| Node {
            id: NodeId(id),
            name: name.to_string(),
            kind,
            parent: None,
            lang: None,
        };
        let legacy_edge = |from: u32, to: u32, lines: std::ops::Range<u32>| Edge {
            from: NodeId(from),
            to: NodeId(to),
            refs: lines
                .map(|line| Ref {
                    file: "a.ts".to_string(),
                    line,
                    raw: "import".to_string(),
                })
                .collect(),
            count: None,
        };
        let g = Graph {
            semantic: None,
            review_kpis: None,
            schema_version: 1,
            root: "/abs".to_string(),
            nodes: vec![
                node(0, "a", NodeKind::Module),
                node(1, "a/x.ts", NodeKind::File),
                node(2, "a/y.ts", NodeKind::File),
                node(3, "b", NodeKind::Module),
                node(4, "b/z.ts", NodeKind::File),
            ],
            edges: vec![
                legacy_edge(1, 4, 1..21), // 20 refs, total unknown
                legacy_edge(2, 4, 1..6),  // 5 refs, overlapping lines
            ],
        };
        // Parent the files to their modules (Node has no builder here).
        let mut g = g;
        g.nodes[1].parent = Some(NodeId(0));
        g.nodes[2].parent = Some(NodeId(0));
        g.nodes[4].parent = Some(NodeId(3));

        let view = aggregate(&g, &Cut::at_depth(&g, 0));
        assert_eq!(view.edges.len(), 1);
        assert_eq!(view.edges[0].count, None, "no total is invented");
        assert_eq!(view.edges[0].weight(), 10, "lower bound = capped sample");
    }

    #[test]
    fn aggregate_is_deterministic_across_runs_and_input_order() {
        let g = fixture();
        let cut = Cut::at_depth(&g, 1);
        assert_eq!(aggregate(&g, &cut), aggregate(&g, &cut));

        // Same facts, different builder insertion order → same view.
        let mut other = GraphBuilder::new();
        let root_file = other.ensure_path("c.ts", Some("ts"));
        let money = other.ensure_path("domain/money.ts", Some("ts"));
        let tax = other.ensure_path("app/tax/tax.ts", Some("ts"));
        let invoice = other.ensure_path("app/billing/invoice.ts", Some("ts"));
        let mut edge = |from: NodeId, to: NodeId, file: &str, line: u32| {
            other
                .edge(
                    from,
                    to,
                    Ref {
                        file: file.to_string(),
                        line,
                        raw: format!("import x from \"y{line}\";"),
                    },
                )
                .expect("test: valid edge");
        };
        edge(root_file, invoice, "c.ts", 1);
        edge(invoice, tax, "app/billing/invoice.ts", 1);
        edge(tax, money, "app/tax/tax.ts", 2);
        edge(invoice, money, "app/billing/invoice.ts", 9);
        edge(invoice, money, "app/billing/invoice.ts", 4);
        let other_g = other.build("/abs/projekt".to_string());
        let other_cut = Cut::at_depth(&other_g, 1);
        assert_eq!(aggregate(&g, &cut), aggregate(&other_g, &other_cut));
    }

    #[test]
    fn validate_reports_broken_cuts() {
        let g = fixture();
        // Missing coverage: `domain/money.ts` has no visible node.
        let err = Cut::new([id(&g, "app"), id(&g, "c.ts")]).validate(&g);
        assert!(matches!(err, Err(ArchguardError::Analysis(_))));
        assert!(
            err.unwrap_err()
                .to_string()
                .contains("file node domain/money.ts has no visible node in cut")
        );

        // Double coverage: invoice sees both itself and `app`.
        let err = Cut::new([id(&g, "app"), id(&g, "app/billing/invoice.ts")]).validate(&g);
        assert!(matches!(err, Err(ArchguardError::Analysis(_))));
        assert!(
            err.unwrap_err()
                .to_string()
                .contains("file node app/billing/invoice.ts has 2 visible nodes in cut")
        );

        // Unknown node id.
        let err = Cut::new([NodeId(99)]).validate(&g);
        assert!(matches!(err, Err(ArchguardError::Analysis(_))));
        assert!(err.unwrap_err().to_string().contains("unknown node id 99"));

        // An empty cut covers nothing.
        assert!(matches!(
            Cut::default().validate(&g),
            Err(ArchguardError::Analysis(_))
        ));

        // The default cut is valid, and so are all expand/collapse results above.
        assert!(Cut::at_depth(&g, 1).validate(&g).is_ok());
    }

    #[test]
    fn aggregate_skips_edges_without_visible_ancestors_instead_of_panicking() {
        let g = fixture();
        // Broken cut (fails validation): only `domain/money.ts` is visible, so
        // no edge has two visible endpoints.
        let cut = Cut::new([id(&g, "domain/money.ts")]);
        assert!(cut.validate(&g).is_err());
        let view = aggregate(&g, &cut);
        assert_eq!(view.nodes.len(), 1);
        assert_eq!(view.nodes[0].name, "domain/money.ts");
        assert!(view.edges.is_empty());
    }

    #[test]
    fn cut_ser_is_transparent_and_deterministic() {
        let g = fixture();
        let cut = Cut::at_depth(&g, 0);
        let json = serde_json::to_string(&cut).expect("serialization");
        let ids: Vec<u32> = serde_json::from_str(&json).expect("transparent array");
        let back: Cut = serde_json::from_str(&json).expect("deserialization");
        assert_eq!(back, cut);
        assert_eq!(ids.len(), cut.len());
        assert!(!ids.is_empty() && ids.windows(2).all(|w| w[0] < w[1]));
    }
}
