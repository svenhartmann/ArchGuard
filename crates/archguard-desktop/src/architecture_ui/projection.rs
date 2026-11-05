//! Package projections of the original source graph for the arc42 chapters;
//! a port of `arc42SourceModel`, `arc42PackageProjection`, `arc42CallEdges`
//! and `arc42BuildingLevels` (viewer_architecture.js).

use std::collections::{HashMap, HashSet};

use archguard_engine::core::{CallAnalysis, Graph, NodeId, NodeKind};
use petgraph::algo::tarjan_scc;
use petgraph::graph::DiGraph;

/// Containment indexes over `graph.nodes` (positions, not ids).
pub struct SourceModel {
    index: HashMap<NodeId, usize>,
    /// Children per parent position (`None`: roots), in graph order.
    children: HashMap<Option<usize>, Vec<usize>>,
    /// The node itself, then its ancestors upwards.
    ancestors: Vec<Vec<usize>>,
    /// Level-2 building blocks of the project overview.
    pub overview: Vec<usize>,
}

impl SourceModel {
    pub fn new(graph: &Graph) -> Self {
        let index: HashMap<NodeId, usize> = graph
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id, i))
            .collect();
        let parent = |i: usize| graph.nodes[i].parent.and_then(|p| index.get(&p).copied());
        let mut children: HashMap<Option<usize>, Vec<usize>> = HashMap::new();
        for i in 0..graph.nodes.len() {
            children.entry(parent(i)).or_default().push(i);
        }
        let ancestors = (0..graph.nodes.len())
            .map(|i| {
                let mut chain = vec![i];
                let mut current = parent(i);
                while let Some(p) = current.filter(|p| !chain.contains(p)) {
                    chain.push(p);
                    current = parent(p);
                }
                chain
            })
            .collect();
        let mut model = SourceModel {
            index,
            children,
            ancestors,
            overview: Vec::new(),
        };
        model.overview = model.overview_blocks(graph);
        model
    }

    fn kids(&self, parent: Option<usize>) -> &[usize] {
        self.children.get(&parent).map_or(&[], Vec::as_slice)
    }

    fn modules(&self, graph: &Graph, parent: Option<usize>) -> Vec<usize> {
        self.kids(parent)
            .iter()
            .copied()
            .filter(|&k| graph.nodes[k].kind == NodeKind::Module)
            .collect()
    }

    /// Root packages with single-package wrappers skipped; a root with
    /// packages and direct files is listed after its packages.
    fn overview_blocks(&self, graph: &Graph) -> Vec<usize> {
        let mut out = Vec::new();
        for &first in self.kids(None) {
            if graph.nodes[first].kind != NodeKind::Module {
                continue;
            }
            let mut root = first;
            while let [only] = self.kids(Some(root)) {
                if graph.nodes[*only].kind != NodeKind::Module {
                    break;
                }
                root = *only;
            }
            let packages = self.modules(graph, Some(root));
            let files = self
                .kids(Some(root))
                .iter()
                .any(|&k| graph.nodes[k].kind == NodeKind::File);
            if packages.is_empty() {
                out.push(root);
            } else {
                out.extend(&packages);
                if files {
                    out.push(root);
                }
            }
        }
        out
    }

    pub fn position(&self, id: NodeId) -> Option<usize> {
        self.index.get(&id).copied()
    }
}

/// What a diagram node stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    Package(usize),
    /// The direct files of the scope (or files without a package).
    Direct,
    /// Project nodes outside the scope.
    Outside,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PNode {
    pub group: Group,
    pub name: String,
    pub files: usize,
    pub cycle: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PEdge {
    /// Positions in [`Projection::nodes`].
    pub from: usize,
    pub to: usize,
    pub count: usize,
    pub cycle: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Projection {
    pub nodes: Vec<PNode>,
    pub edges: Vec<PEdge>,
    /// Source connections inside one shown group.
    pub internal: usize,
    /// Source connections with at least one end in the scope.
    pub total: usize,
}

impl Projection {
    /// Only the cyclic relationships and the nodes they touch.
    pub fn cycles_only(&self) -> Projection {
        let mut keep = vec![false; self.nodes.len()];
        for e in self.edges.iter().filter(|e| e.cycle) {
            keep[e.from] = true;
            keep[e.to] = true;
        }
        let mut slot = vec![usize::MAX; self.nodes.len()];
        let nodes = (0..self.nodes.len())
            .filter(|&i| keep[i])
            .enumerate()
            .map(|(k, i)| {
                slot[i] = k;
                self.nodes[i].clone()
            })
            .collect();
        let edges = self
            .edges
            .iter()
            .filter(|e| e.cycle)
            .map(|e| PEdge {
                from: slot[e.from],
                to: slot[e.to],
                ..e.clone()
            })
            .collect();
        Projection {
            nodes,
            edges,
            ..*self
        }
    }
}

/// Every package (module node) position, in graph order.
pub fn packages(graph: &Graph) -> Vec<usize> {
    (0..graph.nodes.len())
        .filter(|&i| graph.nodes[i].kind == NodeKind::Module)
        .collect()
}

/// Resolved project calls between classes as (from, to) node positions.
pub fn call_edges(graph: &Graph, calls: &CallAnalysis) -> Vec<(usize, usize)> {
    let by_name: HashMap<&str, usize> = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.name.as_str(), i))
        .collect();
    let classes: HashMap<&str, Option<usize>> = calls
        .classes
        .iter()
        .map(|c| (c.id.as_str(), by_name.get(c.graph_node.as_str()).copied()))
        .collect();
    let class = |id: &str| classes.get(id).copied().flatten();
    calls
        .calls
        .iter()
        .filter(|c| c.resolution == "project")
        .filter_map(|c| Some((class(&c.from_class)?, class(c.to_class.as_deref()?)?)))
        .collect()
}

/// Source dependencies as (from, to) node positions.
pub fn dependency_edges(model: &SourceModel, graph: &Graph) -> Vec<(usize, usize)> {
    graph
        .edges
        .iter()
        .filter_map(|e| Some((model.position(e.from)?, model.position(e.to)?)))
        .collect()
}

/// Translated names of the synthetic groups (from the window).
pub struct Labels {
    /// "Files without a package" (project overview).
    pub direct_root: String,
    /// "direct files" (after the scope name).
    pub direct: String,
    /// "Other project packages".
    pub outside: String,
}

/// Packages of `scope` (or the overview) with bundled relationships.
pub fn project(
    model: &SourceModel,
    graph: &Graph,
    scope: Option<usize>,
    edges: &[(usize, usize)],
    labels: &Labels,
) -> Projection {
    let candidates = match scope {
        None => model.overview.clone(),
        Some(s) => model.modules(graph, Some(s)),
    };
    let mut nodes: Vec<PNode> = candidates
        .iter()
        .map(|&c| PNode {
            group: Group::Package(c),
            name: graph.nodes[c].name.clone(),
            files: 0,
            cycle: false,
        })
        .collect();
    let has_direct = model
        .kids(scope)
        .iter()
        .any(|&k| graph.nodes[k].kind == NodeKind::File);
    if has_direct {
        nodes.push(PNode {
            group: Group::Direct,
            name: match scope {
                None => labels.direct_root.clone(),
                Some(s) => format!("{} / {}", graph.nodes[s].name, labels.direct),
            },
            files: 0,
            cycle: false,
        });
    }
    let owner = owners(model, graph, scope, &candidates, has_direct);
    for (i, n) in graph.nodes.iter().enumerate() {
        if let (Some(g), NodeKind::File) = (owner[i], n.kind)
            && let Some(node) = nodes.iter_mut().find(|x| x.group == g)
        {
            node.files += 1;
        }
    }
    nodes.sort_by(|a, b| b.files.cmp(&a.files).then_with(|| a.name.cmp(&b.name)));
    bundle(nodes, &owner, edges, &labels.outside)
}

/// The shown group of every node in the scope; `None` outside it.
fn owners(
    model: &SourceModel,
    graph: &Graph,
    scope: Option<usize>,
    candidates: &[usize],
    has_direct: bool,
) -> Vec<Option<Group>> {
    let ids: HashSet<usize> = candidates.iter().copied().collect();
    (0..graph.nodes.len())
        .map(|i| {
            let chain = &model.ancestors[i];
            if scope.is_some_and(|s| !chain.contains(&s)) {
                return None;
            }
            match chain.iter().find(|p| ids.contains(p)) {
                Some(&p) => Some(Group::Package(p)),
                None => has_direct.then_some(Group::Direct),
            }
        })
        .collect()
}

/// Bundle node-level edges between groups and mark package cycles.
fn bundle(
    mut nodes: Vec<PNode>,
    owner: &[Option<Group>],
    edges: &[(usize, usize)],
    outside_label: &str,
) -> Projection {
    let mut outside = HashSet::new();
    let (mut internal, mut total) = (0, 0);
    let mut bundled: Vec<(Group, Group, usize)> = Vec::new();
    let mut at: HashMap<(Group, Group), usize> = HashMap::new();
    for &(from, to) in edges {
        let (a, b) = (owner[from], owner[to]);
        if a.is_none() && b.is_none() {
            continue;
        }
        let a = a.unwrap_or_else(|| {
            outside.insert(from);
            Group::Outside
        });
        let b = b.unwrap_or_else(|| {
            outside.insert(to);
            Group::Outside
        });
        total += 1;
        if a == b {
            internal += 1;
            continue;
        }
        let k = *at.entry((a, b)).or_insert_with(|| {
            bundled.push((a, b, 0));
            bundled.len() - 1
        });
        bundled[k].2 += 1;
    }
    if !outside.is_empty() {
        nodes.push(PNode {
            group: Group::Outside,
            name: outside_label.to_string(),
            files: outside.len(),
            cycle: false,
        });
    }
    let slot = |g: Group| {
        nodes
            .iter()
            .position(|n| n.group == g)
            .expect("group shown")
    };
    let mut edges: Vec<PEdge> = bundled
        .iter()
        .map(|&(a, b, count)| PEdge {
            from: slot(a),
            to: slot(b),
            count,
            cycle: false,
        })
        .collect();
    mark_cycles(&mut nodes, &mut edges);
    Projection {
        nodes,
        edges,
        internal,
        total,
    }
}

fn mark_cycles(nodes: &mut [PNode], edges: &mut [PEdge]) {
    let mut g = DiGraph::<(), ()>::new();
    let ix: Vec<_> = nodes.iter().map(|_| g.add_node(())).collect();
    for e in edges.iter() {
        g.add_edge(ix[e.from], ix[e.to], ());
    }
    let mut comp = vec![usize::MAX; nodes.len()];
    for (c, scc) in tarjan_scc(&g).into_iter().enumerate() {
        if scc.len() > 1 {
            for n in scc {
                comp[n.index()] = c;
                nodes[n.index()].cycle = true;
            }
        }
    }
    for e in edges.iter_mut() {
        e.cycle = comp[e.from] != usize::MAX && comp[e.from] == comp[e.to];
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Entry {
    pub node: usize,
    pub level: u8,
    pub parent: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Levels {
    /// Level 2 and 3 entries in navigation order.
    pub entries: Vec<Entry>,
    /// The resolved scope (an entry node or `None`).
    pub scope: Option<usize>,
    pub level: u8,
    /// Entries from level 2 down to the scope.
    pub path: Vec<Entry>,
}

impl Levels {
    pub fn entry(&self, node: usize) -> Option<&Entry> {
        self.entries.iter().find(|e| e.node == node)
    }
}

/// Building-block levels relative to the project overview.
pub fn levels(model: &SourceModel, graph: &Graph, scope: Option<usize>) -> Levels {
    let mut entries: Vec<Entry> = model
        .overview
        .iter()
        .map(|&node| Entry {
            node,
            level: 2,
            parent: None,
        })
        .collect();
    for &parent in &model.overview {
        for child in model.modules(graph, Some(parent)) {
            if !entries.iter().any(|e| e.node == child) {
                entries.push(Entry {
                    node: child,
                    level: 3,
                    parent: Some(parent),
                });
            }
        }
    }
    let known = |n: &usize| entries.iter().any(|e| e.node == *n);
    let scope = scope.and_then(|s| model.ancestors[s].iter().copied().find(known));
    let entry = scope.and_then(|s| entries.iter().find(|e| e.node == s).copied());
    let mut path = Vec::new();
    if let Some(e) = entry {
        if let Some(p) = e.parent.and_then(|p| entries.iter().find(|x| x.node == p)) {
            path.push(*p);
        }
        path.push(e);
    }
    Levels {
        level: entry.map_or(1, |e| e.level),
        entries,
        scope,
        path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::{CallClass, Edge, MethodCall, Node};

    /// crates/{a/src/{x.rs, y.rs, sub/{z.rs}}, a/build.rs, b/src/{p.rs}},
    /// top.rs; x→p, p→x, z→y, y→z, x→y, top→x.
    fn graph() -> Graph {
        use NodeKind::{File, Module};
        let n = |id: u32, name: &str, kind, parent: Option<u32>| Node {
            id: NodeId(id),
            name: name.into(),
            kind,
            parent: parent.map(NodeId),
            lang: None,
        };
        let e = |from: u32, to: u32| Edge {
            from: NodeId(from),
            to: NodeId(to),
            refs: Vec::new(),
            count: Some(1),
        };
        Graph {
            schema_version: 1,
            root: ".".into(),
            nodes: vec![
                n(0, "crates", Module, None),
                n(1, "crates/a", Module, Some(0)),
                n(2, "crates/a/src", Module, Some(1)),
                n(3, "crates/a/src/x.rs", File, Some(2)),
                n(4, "crates/a/src/y.rs", File, Some(2)),
                n(5, "crates/a/src/sub", Module, Some(2)),
                n(6, "crates/a/src/sub/z.rs", File, Some(5)),
                n(7, "crates/a/build.rs", File, Some(1)),
                n(8, "crates/b", Module, Some(0)),
                n(9, "crates/b/src", Module, Some(8)),
                n(10, "crates/b/src/p.rs", File, Some(9)),
                n(11, "top.rs", File, None),
            ],
            edges: vec![e(3, 10), e(10, 3), e(6, 4), e(4, 6), e(3, 4), e(11, 3)],
            semantic: None,
            review_kpis: None,
        }
    }

    fn labels() -> Labels {
        Labels {
            direct_root: "Files without a package".into(),
            direct: "direct files".into(),
            outside: "Other project packages".into(),
        }
    }

    fn names(p: &Projection) -> Vec<(String, usize, bool)> {
        p.nodes
            .iter()
            .map(|n| (n.name.clone(), n.files, n.cycle))
            .collect()
    }

    #[test]
    fn overview_skips_single_package_wrappers() {
        let g = graph();
        let m = SourceModel::new(&g);
        // crates has two packages (a, b): no wrapper chain; crates has no
        // direct files.
        assert_eq!(m.overview, vec![1, 8]);
    }

    #[test]
    fn overview_projection_bundles_and_finds_package_cycles() {
        let g = graph();
        let m = SourceModel::new(&g);
        let edges = dependency_edges(&m, &g);
        let p = project(&m, &g, None, &edges, &labels());
        assert_eq!(
            names(&p),
            vec![
                ("crates/a".to_string(), 4, true),
                ("Files without a package".to_string(), 1, false),
                ("crates/b".to_string(), 1, true),
            ]
        );
        // x→p, p→x between packages; z↔y, x→y inside a; top→x from direct.
        assert_eq!(p.total, 6);
        assert_eq!(p.internal, 3);
        let e: Vec<(usize, usize, usize, bool)> = p
            .edges
            .iter()
            .map(|e| (e.from, e.to, e.count, e.cycle))
            .collect();
        assert_eq!(e, vec![(0, 2, 1, true), (2, 0, 1, true), (1, 0, 1, false)]);
    }

    #[test]
    fn scoped_projection_has_direct_files_and_outside() {
        let g = graph();
        let m = SourceModel::new(&g);
        let edges = dependency_edges(&m, &g);
        // Scope crates/a/src: package sub, direct files x, y; outside: p, top.
        let p = project(&m, &g, Some(2), &edges, &labels());
        assert_eq!(
            names(&p),
            vec![
                ("crates/a/src / direct files".to_string(), 2, true),
                ("crates/a/src/sub".to_string(), 1, true),
                ("Other project packages".to_string(), 2, true),
            ]
        );
        assert_eq!(p.total, 6);
        assert_eq!(p.internal, 1);
        let cyc = p.cycles_only();
        assert_eq!(cyc.nodes.len(), 3);
        assert!(cyc.edges.iter().all(|e| e.cycle));
        assert_eq!(cyc.edges.len(), 4);
    }

    #[test]
    fn cycles_only_drops_acyclic_nodes() {
        let g = graph();
        let m = SourceModel::new(&g);
        let edges = dependency_edges(&m, &g);
        let p = project(&m, &g, None, &edges, &labels()).cycles_only();
        assert_eq!(
            p.nodes.iter().map(|n| n.name.as_str()).collect::<Vec<_>>(),
            vec!["crates/a", "crates/b"]
        );
        assert_eq!(
            p.edges.iter().map(|e| (e.from, e.to)).collect::<Vec<_>>(),
            vec![(0, 1), (1, 0)]
        );
    }

    #[test]
    fn building_levels_resolve_nested_scopes() {
        let g = graph();
        let m = SourceModel::new(&g);
        let l = levels(&m, &g, None);
        assert_eq!(l.level, 1);
        assert_eq!(
            l.entries
                .iter()
                .map(|e| (e.node, e.level))
                .collect::<Vec<_>>(),
            vec![(1, 2), (8, 2), (2, 3), (9, 3)]
        );
        let l = levels(&m, &g, Some(2));
        assert_eq!((l.scope, l.level), (Some(2), 3));
        assert_eq!(
            l.path.iter().map(|e| e.node).collect::<Vec<_>>(),
            vec![1, 2]
        );
        // A deeper package resolves to its nearest entry.
        let l = levels(&m, &g, Some(5));
        assert_eq!(l.scope, Some(2));
    }

    #[test]
    fn call_edges_map_resolved_project_calls_to_nodes() {
        let g = graph();
        let class = |id: &str, node: &str| CallClass {
            language: None,
            id: id.into(),
            name: id.into(),
            package: String::new(),
            file: node.into(),
            graph_node: node.into(),
            line: 1,
            kind: "class".into(),
            is_abstract: None,
            visibility: None,
            supertypes: Vec::new(),
            supertype_sites: None,
            declaration_files: None,
        };
        let call = |from: &str, to: Option<&str>, resolution: &str| MethodCall {
            from_class: from.into(),
            from_method: None,
            to_class: to.map(Into::into),
            to_method: None,
            target_label: String::new(),
            dispatch: "static".into(),
            resolution: resolution.into(),
            file: String::new(),
            line: 1,
            column: 1,
            code: String::new(),
        };
        let calls = CallAnalysis {
            classes: vec![
                class("X", "crates/a/src/x.rs"),
                class("P", "crates/b/src/p.rs"),
            ],
            calls: vec![
                call("X", Some("P"), "project"),
                call("X", Some("P"), "external"),
                call("X", None, "project"),
                call("P", Some("X"), "project"),
            ],
            ..Default::default()
        };
        assert_eq!(call_edges(&g, &calls), vec![(3, 10), (10, 3)]);
    }
}
