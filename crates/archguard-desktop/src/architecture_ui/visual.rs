//! The derived chapter visual (`renderArc42PackageMap`): scope, selection,
//! building-block levels and the relationship list, computed for the window.

use archguard_engine::core::Graph;

use std::collections::HashMap;

use super::projection::{self, Group, Labels, Levels, Projection, SourceModel};
use super::{diagram, names};

/// Relationships listed per page, as the web (30).
pub const RELATIONS_PAGE: usize = 30;

/// Which package map a chapter shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Chapter 5, with building-block levels.
    Dependencies,
    /// Chapter 6: static calls between packages.
    Calls,
    /// Chapter 11: cyclic package relationships.
    Cycles,
}

/// Per-kind state that survives chapter changes, as `diagramStates`.
#[derive(Debug, Clone, Default)]
pub struct MapState {
    pub scope: Option<usize>,
    pub selected: Option<Group>,
    pub relations: usize,
}

/// What a selected card offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    /// Open the package as the next building-block level.
    OpenLevel(usize),
    /// Reveal the package in the Graph tab.
    OpenInGraph(usize),
    /// Re-scope the map to the package.
    OpenPackage(usize),
}

/// One computed map, ready for the window.
pub struct Map {
    /// The resolved scope (a level entry for the dependency map).
    pub scope: Option<usize>,
    pub levels: Option<Levels>,
    pub projection: Projection,
    /// The shown graph: the projection, or its cycles only.
    pub shown: Projection,
    pub layout: diagram::Layout,
    pub selected: Option<usize>,
    pub action: Action,
    /// Scope choices: `None` (overview) first, then packages by name.
    pub scopes: Vec<Option<usize>>,
}

pub fn build(
    kind: Kind,
    state: &MapState,
    model: &SourceModel,
    graph: &Graph,
    edges: &[(usize, usize)],
    labels: &Labels,
) -> Map {
    let levels =
        (kind == Kind::Dependencies).then(|| projection::levels(model, graph, state.scope));
    let scope = levels.as_ref().map_or(state.scope, |l| l.scope);
    let projection = projection::project(model, graph, scope, edges, labels);
    let shown = if kind == Kind::Cycles {
        projection.cycles_only()
    } else {
        projection.clone()
    };
    let pairs: Vec<(usize, usize)> = shown.edges.iter().map(|e| (e.from, e.to)).collect();
    let layout = diagram::layout(shown.nodes.len(), &pairs);
    let selected = state
        .selected
        .and_then(|g| shown.nodes.iter().position(|n| n.group == g));
    let action = selected.map_or(Action::None, |s| action(&shown, s, levels.as_ref()));
    let scopes = scopes(graph, levels.as_ref());
    Map {
        scope,
        levels,
        projection,
        shown,
        layout,
        selected,
        action,
        scopes,
    }
}

fn action(shown: &Projection, selected: usize, levels: Option<&Levels>) -> Action {
    let Group::Package(drill) = shown.nodes[selected].group else {
        return Action::None;
    };
    match levels {
        None => Action::OpenPackage(drill),
        Some(l) if l.level < 3 && l.entry(drill).is_some() => Action::OpenLevel(drill),
        Some(_) => Action::OpenInGraph(drill),
    }
}

/// The scope list: level entries with levels, else every package.
fn scopes(graph: &Graph, levels: Option<&Levels>) -> Vec<Option<usize>> {
    let mut nodes: Vec<usize> = match levels {
        Some(l) => l.entries.iter().map(|e| e.node).collect(),
        None => projection::packages(graph),
    };
    nodes.sort_by(|&a, &b| graph.nodes[a].name.cmp(&graph.nodes[b].name));
    std::iter::once(None)
        .chain(nodes.into_iter().map(Some))
        .collect()
}

/// Relationships touching the selection (all without one).
pub fn relations(map: &Map) -> Vec<&projection::PEdge> {
    map.shown
        .edges
        .iter()
        .filter(|e| map.selected.is_none_or(|s| e.from == s || e.to == s))
        .collect()
}

/// Labels of the scope choices after the overview, named among each other.
pub fn scope_labels(map: &Map, graph: &Graph) -> Vec<String> {
    let paths: Vec<&str> = map
        .scopes
        .iter()
        .flatten()
        .map(|&n| graph.nodes[n].name.as_str())
        .collect();
    names::short_names(&paths)
}

/// Level-entry labels: level 2 among each other, level 3 among its
/// siblings, as the list indents it below its parent.
pub fn level_labels(levels: &Levels, graph: &Graph) -> HashMap<usize, String> {
    let mut groups: HashMap<Option<usize>, Vec<usize>> = HashMap::new();
    for e in &levels.entries {
        groups.entry(e.parent).or_default().push(e.node);
    }
    let mut out = HashMap::new();
    for nodes in groups.values() {
        let paths: Vec<&str> = nodes
            .iter()
            .map(|&n| graph.nodes[n].name.as_str())
            .collect();
        out.extend(nodes.iter().copied().zip(names::short_names(&paths)));
    }
    out
}

/// Level list: the overview, level-2 entries, and the children of the
/// opened level-2 entry right after it. (scope, level, label, current)
pub fn level_items(levels: &Levels, graph: &Graph) -> Vec<(Option<usize>, u8, String)> {
    let labels = level_labels(levels, graph);
    let short = |n: usize| labels[&n].clone();
    let mut out = vec![(None, 1, String::new())];
    let expanded = levels.path.first().map(|e| e.node);
    for e in levels.entries.iter().filter(|e| e.level == 2) {
        out.push((Some(e.node), 2, short(e.node)));
        if expanded == Some(e.node) {
            for c in levels.entries.iter().filter(|c| c.parent == Some(e.node)) {
                out.push((Some(c.node), 3, short(c.node)));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::{Edge, Node, NodeId, NodeKind};

    /// p/{a/{x.rs, deep/{d.rs}}, b/{y.rs}}; x↔y, d→x.
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
                n(0, "p", Module, None),
                n(1, "p/a", Module, Some(0)),
                n(2, "p/a/x.rs", File, Some(1)),
                n(3, "p/b", Module, Some(0)),
                n(4, "p/b/y.rs", File, Some(3)),
                n(5, "p/a/deep", Module, Some(1)),
                n(6, "p/a/deep/d.rs", File, Some(5)),
            ],
            edges: vec![e(2, 4), e(4, 2), e(6, 2)],
            semantic: None,
            review_kpis: None,
        }
    }

    fn labels() -> Labels {
        Labels {
            direct_root: "root files".into(),
            direct: "direct".into(),
            outside: "outside".into(),
        }
    }

    #[test]
    fn selecting_a_level_two_package_offers_the_next_level() {
        let g = graph();
        let m = SourceModel::new(&g);
        let edges = projection::dependency_edges(&m, &g);
        let mut state = MapState {
            selected: Some(Group::Package(1)),
            ..Default::default()
        };
        let map = build(Kind::Dependencies, &state, &m, &g, &edges, &labels());
        assert_eq!(map.action, Action::OpenLevel(1));
        assert_eq!(map.scopes, vec![None, Some(1), Some(5), Some(3)]);
        assert_eq!(relations(&map).len(), 2);
        // Inside level 3 a package opens in the graph.
        state.scope = Some(1);
        state.selected = Some(Group::Package(5));
        let map = build(Kind::Dependencies, &state, &m, &g, &edges, &labels());
        assert_eq!(map.levels.as_ref().map(|l| l.level), Some(2));
        assert_eq!(map.action, Action::OpenLevel(5));
        state.scope = Some(5);
        let map = build(Kind::Dependencies, &state, &m, &g, &edges, &labels());
        assert_eq!(map.levels.as_ref().map(|l| l.level), Some(3));
        assert_eq!(map.selected, None);
    }

    #[test]
    fn cycle_maps_open_packages_and_list_every_package_as_scope() {
        let g = graph();
        let m = SourceModel::new(&g);
        let edges = projection::dependency_edges(&m, &g);
        let state = MapState {
            selected: Some(Group::Package(3)),
            ..Default::default()
        };
        let map = build(Kind::Cycles, &state, &m, &g, &edges, &labels());
        assert_eq!(map.shown.nodes.len(), 2);
        assert_eq!(map.action, Action::OpenPackage(3));
        assert_eq!(map.scopes, vec![None, Some(0), Some(1), Some(5), Some(3)]);
    }

    /// This repository's shape: crates/{x/src/{x.rs}, y/src/{y.rs,
    /// sub/{s.rs, deep/{d.rs}}}}, every crate a root; s→y, x→y.
    fn crates() -> Graph {
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
                n(0, "crates/x", Module, None),
                n(1, "crates/x/src", Module, Some(0)),
                n(2, "crates/x/src/x.rs", File, Some(1)),
                n(3, "crates/y", Module, None),
                n(4, "crates/y/src", Module, Some(3)),
                n(5, "crates/y/src/y.rs", File, Some(4)),
                n(6, "crates/y/src/sub", Module, Some(4)),
                n(7, "crates/y/src/sub/s.rs", File, Some(6)),
                n(8, "crates/y/src/sub/deep", Module, Some(6)),
                n(9, "crates/y/src/sub/deep/d.rs", File, Some(8)),
            ],
            edges: vec![e(7, 5), e(2, 5)],
            semantic: None,
            review_kpis: None,
        }
    }

    #[test]
    fn crate_source_roots_are_named_after_their_crate() {
        let g = crates();
        let m = SourceModel::new(&g);
        let edges = projection::dependency_edges(&m, &g);
        let mut state = MapState::default();
        let map = build(Kind::Dependencies, &state, &m, &g, &edges, &labels());
        let cards: Vec<&str> = map.shown.nodes.iter().map(|n| n.label.as_str()).collect();
        assert_eq!(cards, vec!["y/sub", "x", "y"]);
        assert_eq!(map.shown.nodes[1].name, "crates/x/src", "the path stays");
        assert_eq!(
            scope_labels(&map, &g),
            vec!["x", "y", "y/sub", "y/sub/deep"]
        );
        state.scope = Some(8);
        let map = build(Kind::Dependencies, &state, &m, &g, &edges, &labels());
        let items: Vec<String> = level_items(map.levels.as_ref().expect("levels"), &g)
            .into_iter()
            .map(|(_, _, l)| l)
            .collect();
        assert_eq!(items, vec!["", "x", "y/sub", "deep", "y"]);
        state.scope = Some(4);
        let map = build(Kind::Dependencies, &state, &m, &g, &edges, &labels());
        let cards: Vec<&str> = map.shown.nodes.iter().map(|n| n.label.as_str()).collect();
        assert_eq!(cards, vec!["sub", "y / direct", "outside"]);
    }

    #[test]
    fn single_segment_packages_keep_their_names() {
        // examples/service-shop: one directory per service.
        let g = graph();
        let m = SourceModel::new(&g);
        let edges = projection::dependency_edges(&m, &g);
        let map = build(
            Kind::Dependencies,
            &MapState::default(),
            &m,
            &g,
            &edges,
            &labels(),
        );
        let cards: Vec<&str> = map.shown.nodes.iter().map(|n| n.label.as_str()).collect();
        assert_eq!(cards, vec!["a", "b"]);
    }

    #[test]
    fn level_items_expand_the_opened_entry() {
        let g = graph();
        let m = SourceModel::new(&g);
        let l = projection::levels(&m, &g, Some(5));
        let items: Vec<(Option<usize>, u8)> = level_items(&l, &g)
            .into_iter()
            .map(|(s, lv, _)| (s, lv))
            .collect();
        assert_eq!(
            items,
            vec![(None, 1), (Some(1), 2), (Some(5), 3), (Some(3), 2)]
        );
    }
}
