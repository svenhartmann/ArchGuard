//! A scoped, deterministic package map; cycles describe this aggregation only.
use archguard_analysis::find_sccs;
use archguard_core::{ArchguardError, Cut, Graph, NodeId, NodeKind, aggregate};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtlasArea {
    pub name: String,
    pub kind: NodeKind,
    pub source_files: usize,
    pub symbolic_files: usize,
    pub incoming: usize,
    pub outgoing: usize,
    pub external_incoming: usize,
    pub external_outgoing: usize,
    pub cyclic: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtlasConnection {
    pub from: String,
    pub to: String,
    pub evidence_count: Option<u64>,
    pub cyclic: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchitectureAtlas {
    pub definition: String,
    pub revision: String,
    pub scope: Option<String>,
    pub status: String,
    pub cycle_groups: usize,
    pub external_connections: usize,
    pub areas: Vec<AtlasArea>,
    pub connections: Vec<AtlasConnection>,
}
/// Skip only single-module containment chains. Explicit scopes never get skipped.
/// No inferred domain names, source changes or implicit runtime claims.
pub fn architecture_atlas(
    graph: &Graph,
    within: Option<&str>,
) -> Result<ArchitectureAtlas, ArchguardError> {
    let nodes: BTreeMap<_, _> = graph.nodes.iter().map(|n| (n.id, n)).collect();
    let mut children: BTreeMap<Option<NodeId>, Vec<NodeId>> = BTreeMap::new();
    for n in &graph.nodes {
        children.entry(n.parent).or_default().push(n.id);
    }
    let mut scope = within
        .map(|name| {
            graph
                .nodes
                .iter()
                .find(|n| n.name == name)
                .map(|n| n.id)
                .ok_or_else(|| ArchguardError::Analysis(format!("Unknown atlas scope: {name}")))
        })
        .transpose()?;
    let mut ids = if scope.is_some_and(|id| nodes[&id].kind == NodeKind::File) {
        vec![scope.unwrap()]
    } else {
        children.get(&scope).cloned().unwrap_or_default()
    };
    if within.is_none() {
        while ids.len() == 1 && nodes[&ids[0]].kind == NodeKind::Module {
            scope = Some(ids[0]);
            ids = children.get(&scope).cloned().unwrap_or_default();
        }
    }
    let cut = Cut::new(ids.iter().copied());
    let view = aggregate(graph, &cut);
    let groups = find_sccs(&view);
    let source_by_name: BTreeMap<_, _> = graph
        .nodes
        .iter()
        .map(|n| (n.name.as_str(), n.id))
        .collect();
    let original_ids: BTreeMap<_, _> = view
        .nodes
        .iter()
        .map(|n| (n.id, source_by_name[n.name.as_str()]))
        .collect();
    let cyclic_nodes: BTreeSet<_> = groups
        .iter()
        .flat_map(|s| s.node_ids.iter().map(|id| original_ids[id]))
        .collect();
    let cyclic_edges: BTreeSet<_> = groups
        .iter()
        .flat_map(|s| s.edges.iter().copied())
        .collect();
    let id_set: BTreeSet<_> = ids.iter().copied().collect();
    let mut owners = BTreeMap::new();
    for node in &graph.nodes {
        let mut current = Some(node.id);
        while let Some(id) = current {
            if id_set.contains(&id) {
                owners.insert(node.id, id);
                break;
            }
            current = nodes.get(&id).and_then(|n| n.parent);
        }
    }
    let mut areas: BTreeMap<_, _> = ids
        .into_iter()
        .map(|id| {
            (
                id,
                AtlasArea {
                    name: nodes[&id].name.clone(),
                    kind: nodes[&id].kind,
                    source_files: 0,
                    symbolic_files: 0,
                    incoming: 0,
                    outgoing: 0,
                    external_incoming: 0,
                    external_outgoing: 0,
                    cyclic: cyclic_nodes.contains(&id),
                },
            )
        })
        .collect();
    for node in &graph.nodes {
        if node.kind == NodeKind::File
            && let Some(owner) = owners.get(&node.id)
        {
            let row = areas.get_mut(owner).unwrap();
            if node.name.ends_with("/*") {
                row.symbolic_files += 1;
            } else {
                row.source_files += 1;
            }
        }
    }
    let mut external_connections = 0;
    for edge in &graph.edges {
        match (owners.get(&edge.from), owners.get(&edge.to)) {
            (Some(id), None) => {
                areas.get_mut(id).unwrap().external_outgoing += 1;
                external_connections += 1;
            }
            (None, Some(id)) => {
                areas.get_mut(id).unwrap().external_incoming += 1;
                external_connections += 1;
            }
            _ => {}
        }
    }
    let connections = view
        .edges
        .iter()
        .map(|edge| {
            let from = original_ids[&edge.from];
            let to = original_ids[&edge.to];
            areas.get_mut(&from).unwrap().outgoing += 1;
            areas.get_mut(&to).unwrap().incoming += 1;
            AtlasConnection {
                from: nodes[&from].name.clone(),
                to: nodes[&to].name.clone(),
                evidence_count: edge.count,
                cyclic: cyclic_edges.contains(&(edge.from, edge.to)),
            }
        })
        .collect();
    let mut areas: Vec<_> = areas.into_values().collect();
    areas.sort_by(|a, b| a.name.cmp(&b.name));
    let scope = scope.map(|id| nodes[&id].name.clone());
    let revision = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(graph, &scope)).expect("graph serializes"))
    );
    Ok(ArchitectureAtlas {
        definition: "package-atlas/v1".into(),
        revision,
        scope,
        status: graph
            .semantic
            .as_ref()
            .map(|s| s.status.clone())
            .unwrap_or_else(|| "imports_only".into()),
        cycle_groups: groups.len(),
        external_connections,
        areas,
        connections,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{GraphBuilder, Ref};
    fn fixture() -> Graph {
        let mut b = GraphBuilder::new();
        let a = b.ensure_path("org/app/search/A.ts", Some("ts"));
        let c = b.ensure_path("org/app/client/C.ts", Some("ts"));
        let t = b.ensure_path("org/app/search/Types.ts", Some("ts"));
        b.edge(
            a,
            c,
            Ref {
                file: "A.ts".into(),
                line: 1,
                raw: "import C".into(),
            },
        )
        .unwrap();
        b.edge(
            c,
            t,
            Ref {
                file: "C.ts".into(),
                line: 2,
                raw: "import Types".into(),
            },
        )
        .unwrap();
        b.build("/fixture".into())
    }
    #[test]
    fn package_cycle_is_not_a_file_cycle_and_prefix_is_skipped() {
        let graph = fixture();
        let atlas = architecture_atlas(&graph, None).unwrap();
        assert_eq!(atlas.scope.as_deref(), Some("org/app"));
        assert_eq!(atlas.cycle_groups, 1);
        assert!(find_sccs(&graph).is_empty());
        assert!(atlas.areas.iter().all(|r| r.cyclic));
        assert_eq!(atlas.areas.iter().map(|r| r.source_files).sum::<usize>(), 3);
        assert_eq!(atlas.connections.len(), 2);
        assert!(
            atlas
                .connections
                .iter()
                .all(|r| r.evidence_count == Some(1))
        );
        assert_eq!(atlas.status, "imports_only");
    }
    #[test]
    fn focused_scope_preserves_external_connections_and_revision() {
        let graph = fixture();
        let base = architecture_atlas(&graph, None).unwrap();
        let focused = architecture_atlas(&graph, Some("org/app/search")).unwrap();
        assert_eq!(focused.external_connections, 2);
        assert_eq!(focused.cycle_groups, 0);
        assert_eq!(focused.areas.len(), 2);
        assert_ne!(base.revision, focused.revision);
        let file = architecture_atlas(&graph, Some("org/app/search/A.ts")).unwrap();
        assert_eq!(file.areas.len(), 1);
        assert_eq!(file.external_connections, 1);
        assert_eq!(file.areas[0].source_files, 1);
        assert!(architecture_atlas(&graph, Some("search")).is_err());
        assert_eq!(
            architecture_atlas(&graph, None).unwrap().revision,
            base.revision
        );
        let mut changed = graph;
        changed.edges.clear();
        assert_ne!(
            architecture_atlas(&changed, None).unwrap().revision,
            base.revision
        );
    }
    #[test]
    fn symbolic_files_and_unknown_totals_are_not_invented_measurements() {
        let mut b = GraphBuilder::new();
        let a = b.ensure_path("a/Real.ts", Some("ts"));
        let p = b.ensure_pseudo("b");
        b.edge(
            a,
            p,
            Ref {
                file: "a/Real.ts".into(),
                line: 1,
                raw: "import unknown".into(),
            },
        )
        .unwrap();
        let mut graph = b.build("/fixture".into());
        graph.edges[0].count = None;
        let atlas = architecture_atlas(&graph, None).unwrap();
        assert_eq!(atlas.areas.iter().map(|r| r.source_files).sum::<usize>(), 1);
        assert_eq!(
            atlas.areas.iter().map(|r| r.symbolic_files).sum::<usize>(),
            1
        );
        assert_eq!(atlas.connections[0].evidence_count, None);
    }
}
