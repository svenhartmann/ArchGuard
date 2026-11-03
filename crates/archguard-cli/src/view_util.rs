//! View operations over the engine-provided effective configuration.

use std::collections::hash_map::Entry;
use std::collections::{BTreeSet, HashMap, HashSet};

use archguard_engine::LoadedConfig;
use archguard_engine::analysis::find_sccs;
use archguard_engine::core::{Cut, Graph, LayerConfig, NodeId};

/// Consumers receive the effective configuration from `AnalysisRun`.
pub fn effective_depth(_graph: &Graph, loaded: &LoadedConfig) -> u32 {
    loaded.config.modules.depth
}

/// Preserve the engine's layer origins and merge, also after drill-down.
pub fn effective_layers(_view: &Graph, loaded: &LoadedConfig) -> Vec<LayerConfig> {
    loaded.config.layers.clone()
}

/// Look a node up by its exact name (view node names are unique).
pub fn find_node(graph: &Graph, name: &str) -> Option<NodeId> {
    graph.nodes.iter().find(|n| n.name == name).map(|n| n.id)
}

/// The module cut: [`Cut::at_depth`] at [`effective_depth`].
pub fn module_cut(graph: &Graph, loaded: &LoadedConfig) -> Cut {
    Cut::at_depth(graph, effective_depth(graph, loaded) as usize)
}

/// Zoom `base` in by every named module in order; `Err(name)` for the
/// first unknown name (the callers turn that into a usage error).
/// Expanding an invisible or childless node is a no-op of [`Cut::expand`]
/// itself and stays one here.
pub fn expand_cut(graph: &Graph, base: &Cut, names: &[String]) -> Result<Cut, String> {
    let mut cut = Cut::new(base.nodes().iter().copied());
    for name in names {
        let id = find_node(graph, name).ok_or_else(|| name.clone())?;
        cut = cut.expand(graph, id);
    }
    Ok(cut)
}

/// The cut showing exactly the direct children of `parent` (an invisible
/// parent yields the empty cut — [`aggregate`] then renders an empty
/// view, which is the honest reading of "nothing of it is visible").
pub fn within_cut(graph: &Graph, parent: NodeId) -> Cut {
    Cut::new(graph.children_of(parent))
}

/// Precomputed arguments of the T38 DSM renderers: the reverse
/// topological order of the condensation (sinks first, cycle-group
/// members adjacent) and the set of edges inside cyclic components.
///
/// Groups are the cyclic SCCs plus one singleton per remaining node.
/// Kahn runs on the condensation's OUT-degree, so groups nothing
/// depends on (sinks) come first; ties break on the smallest member
/// name, then on group index, keeping the order deterministic. The
/// condensation is a DAG, so the loop always empties; a defensive tail
/// keeps stray ids from vanishing.
pub fn dsm_layout(view: &Graph) -> (Vec<NodeId>, HashSet<(NodeId, NodeId)>) {
    let sccs = find_sccs(view);
    let mut groups: Vec<Vec<NodeId>> = Vec::new();
    let mut group_of: HashMap<NodeId, usize> = HashMap::new();
    for scc in &sccs {
        let gid = groups.len();
        for &id in &scc.node_ids {
            group_of.insert(id, gid);
        }
        groups.push(scc.node_ids.clone());
    }
    for node in &view.nodes {
        if let Entry::Vacant(entry) = group_of.entry(node.id) {
            let gid = groups.len();
            entry.insert(gid);
            groups.push(vec![node.id]);
        }
    }

    // Condensation: group → groups it depends on.
    let mut depends: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); groups.len()];
    for edge in &view.edges {
        if let (Some(&a), Some(&b)) = (group_of.get(&edge.from), group_of.get(&edge.to))
            && a != b
        {
            depends[a].insert(b);
        }
    }
    let mut pending: Vec<usize> = depends.iter().map(|set| set.len()).collect();

    let sort_key = |gid: usize| -> (String, usize) {
        let smallest = groups[gid]
            .iter()
            .filter_map(|id| view.node_name(*id))
            .min()
            .unwrap_or("?")
            .to_string();
        (smallest, gid)
    };

    let mut emitted = vec![false; groups.len()];
    let mut order: Vec<NodeId> = Vec::new();
    loop {
        let next = (0..groups.len())
            .filter(|&g| !emitted[g] && pending[g] == 0)
            .map(|g| (sort_key(g), g))
            .min()
            .map(|(_, g)| g);
        let Some(gid) = next else { break };
        emitted[gid] = true;
        order.extend(groups[gid].iter().copied());
        for p in 0..groups.len() {
            if !emitted[p] && depends[p].contains(&gid) {
                pending[p] -= 1;
            }
        }
    }
    // Defensive tail (unreachable for a real DAG condensation).
    for gid in 0..groups.len() {
        if !emitted[gid] {
            order.extend(groups[gid].iter().copied());
        }
    }

    let cyclic: HashSet<(NodeId, NodeId)> = sccs.iter().flat_map(|s| s.edges.clone()).collect();
    (order, cyclic)
}

#[cfg(test)]
mod tests {
    use super::{dsm_layout, effective_depth, expand_cut, find_node, within_cut};
    use archguard_engine::LoadedConfig;
    use archguard_engine::core::{
        Config, ConfigSource, Cut, Edge, Graph, LayerConfig, Node, NodeId, NodeKind, Ref,
    };
    use std::collections::BTreeMap;

    fn module(id: u32, name: &str, parent: Option<u32>) -> Node {
        Node {
            id: NodeId(id),
            name: name.to_string(),
            kind: NodeKind::Module,
            parent: parent.map(NodeId),
            lang: None,
        }
    }

    fn file(id: u32, name: &str, parent: u32) -> Node {
        Node {
            id: NodeId(id),
            name: name.to_string(),
            kind: NodeKind::File,
            parent: Some(NodeId(parent)),
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

    /// `app` (0) holds `a` (2) and `b` (3); `domain` (1) holds `c` (4).
    /// Cycle `a ↔ b` inside `app`, plus `a → c`.
    fn graph() -> Graph {
        Graph {
            review_kpis: None,
            semantic: None,
            schema_version: 1,
            root: "/abs/projekt".to_string(),
            nodes: vec![
                module(0, "app", None),
                module(1, "domain", None),
                file(2, "app/a", 0),
                file(3, "app/b", 0),
                file(4, "domain/c", 1),
            ],
            edges: vec![edge(2, 3), edge(3, 2), edge(2, 4)],
        }
    }

    fn loaded(declared_modules: bool) -> LoadedConfig {
        LoadedConfig {
            config: Config::default(),
            sources: if declared_modules {
                BTreeMap::from([("modules".to_string(), ConfigSource::Declared)])
            } else {
                BTreeMap::new()
            },
        }
    }

    #[test]
    fn effective_depth_preserves_engine_value() {
        let g = graph();
        let mut l = loaded(true);
        l.config.modules.depth = 7;
        assert_eq!(effective_depth(&g, &l), 7, "declared depth wins");
        // Not declared: every node in this tiny graph is already a
        // leaf-or-depth-1 candidate, so the suggestion is depth 1.
        assert_eq!(
            effective_depth(&g, &loaded(false)),
            loaded(false).config.modules.depth
        );
    }

    #[test]
    fn find_node_matches_exact_names_only() {
        let g = graph();
        assert_eq!(find_node(&g, "app"), Some(NodeId(0)));
        assert_eq!(find_node(&g, "app/a"), Some(NodeId(2)));
        assert_eq!(find_node(&g, "nope"), None);
    }

    #[test]
    fn expand_cut_reveals_children_and_reports_unknown_names() {
        let g = graph();
        // Roots are depth 0, so the depth-0 cut shows app and domain;
        // expanding app replaces it with its files and leaves domain.
        let base = Cut::at_depth(&g, 0);
        let expanded = expand_cut(&g, &base, &["app".to_string()]).expect("app exists");
        assert!(expanded.contains(NodeId(2)) && expanded.contains(NodeId(3)));
        assert!(!expanded.contains(NodeId(0)), "the parent leaves the cut");
        assert!(expanded.contains(NodeId(1)), "untouched nodes stay");
        let err = expand_cut(&g, &base, &["ghost".to_string()]).unwrap_err();
        assert_eq!(err, "ghost");
    }

    #[test]
    fn within_cut_shows_direct_children_only() {
        let g = graph();
        let cut = within_cut(&g, NodeId(0));
        assert_eq!(
            cut.nodes().iter().copied().collect::<Vec<_>>(),
            vec![NodeId(2), NodeId(3)]
        );
    }

    #[test]
    fn dsm_layout_puts_sinks_first_and_keeps_the_cycle_group_adjacent() {
        // Aggregated view of `graph()`: app (cycle a↔b) depends on domain.
        let view = Graph {
            semantic: None,
            review_kpis: None,
            schema_version: 1,
            root: "/abs/projekt".to_string(),
            nodes: vec![module(0, "app", None), module(1, "domain", None)],
            edges: vec![edge(0, 1)],
        };
        let (order, cyclic) = dsm_layout(&view);
        assert_eq!(order, vec![NodeId(1), NodeId(0)], "domain (sink) first");
        assert!(cyclic.is_empty(), "the view itself has no cycle");

        // A real cycle in the view keeps its members adjacent.
        let view = Graph {
            semantic: None,
            review_kpis: None,
            schema_version: 1,
            root: "/abs/projekt".to_string(),
            nodes: vec![file(0, "a", 0), file(1, "b", 0), file(2, "sink", 0)],
            edges: vec![edge(0, 1), edge(1, 0), edge(0, 2)],
        };
        let (order, cyclic) = dsm_layout(&view);
        assert_eq!(order, vec![NodeId(2), NodeId(0), NodeId(1)]);
        assert!(cyclic.contains(&(NodeId(0), NodeId(1))));
        assert!(cyclic.contains(&(NodeId(1), NodeId(0))));
    }

    #[test]
    fn effective_layers_preserves_engine_merge() {
        let view = Graph {
            semantic: None,
            review_kpis: None,
            schema_version: 1,
            root: "/abs/projekt".to_string(),
            nodes: vec![module(0, "app", None), module(1, "domain", None)],
            edges: vec![edge(0, 1)],
        };
        let mut l = loaded(false);
        l.config.layers = vec![LayerConfig {
            name: "l0".to_string(),
            contains: vec!["domain/*".to_string()],
            may_depend_on: vec![],
            origin: Default::default(),
        }];
        let merged = super::effective_layers(&view, &l);
        assert_eq!(merged[0].name, "l0");
        assert_eq!(
            merged[0].origin,
            archguard_engine::core::LayerOrigin::Declared
        );
        assert_eq!(merged, l.config.layers);
        // derive_layers = false keeps only the declared entry.
        l.config.derive_layers = false;
        assert_eq!(super::effective_layers(&view, &l).len(), 1);
    }
}
