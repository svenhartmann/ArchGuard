//! Connected graph regions, independent of names and of the display cut.
use super::model::{PatternAssessment, add, assessment, evidence};
use archguard_core::{Edge, Graph, NodeId};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn assess(graph: &Graph, assessments: &mut Vec<PatternAssessment>) {
    let nodes: BTreeMap<_, _> = graph.nodes.iter().map(|n| (n.id, n)).collect();
    let mut neighbors = BTreeMap::<NodeId, BTreeSet<NodeId>>::new();
    for edge in &graph.edges {
        neighbors.entry(edge.from).or_default().insert(edge.to);
        neighbors.entry(edge.to).or_default().insert(edge.from);
    }
    let mut owner = BTreeMap::new();
    let mut components = Vec::new();
    for start in neighbors.keys() {
        if owner.contains_key(start) {
            continue;
        }
        let mut members = BTreeSet::new();
        let mut pending = vec![*start];
        while let Some(id) = pending.pop() {
            if members.insert(id) {
                owner.insert(id, components.len());
                pending.extend(neighbors[&id].iter().filter(|id| !members.contains(id)));
            }
        }
        components.push(members);
    }
    let mut links = vec![Vec::new(); components.len()];
    for edge in &graph.edges {
        links[owner[&edge.from]].push(edge);
    }
    let global_chain = assessments
        .iter()
        .any(|p| p.pattern == "pipeline" && p.status == "candidate");
    let mut level_candidates = 0;
    for (members, edges) in components.iter().zip(&links) {
        if members.len() < 3 {
            continue;
        }
        let local = Graph {
            schema_version: graph.schema_version,
            root: graph.root.clone(),
            nodes: members.iter().map(|id| nodes[id].clone()).collect(),
            edges: edges.iter().map(|e| (*e).clone()).collect(),
            semantic: graph.semantic.clone(),
            review_kpis: None,
        };
        let cycles = archguard_analysis::find_sccs(&local);
        let mut group: BTreeMap<_, _> = members.iter().map(|id| (*id, *id)).collect();
        for cycle in &cycles {
            let representative = *cycle.node_ids.iter().min().unwrap();
            for id in &cycle.node_ids {
                group.insert(*id, representative);
            }
        }
        let mut dependencies: BTreeMap<_, BTreeSet<NodeId>> =
            group.values().map(|id| (*id, BTreeSet::new())).collect();
        let mut dependents = dependencies.clone();
        let mut boundary = BTreeMap::<(NodeId, NodeId), &Edge>::new();
        let mut incoming = BTreeMap::<NodeId, usize>::new();
        let mut outgoing = BTreeMap::<NodeId, usize>::new();
        for edge in edges {
            *incoming.entry(edge.to).or_default() += 1;
            *outgoing.entry(edge.from).or_default() += 1;
            let (a, b) = (group[&edge.from], group[&edge.to]);
            if a != b {
                dependencies.get_mut(&a).unwrap().insert(b);
                dependents.get_mut(&b).unwrap().insert(a);
                boundary.entry((a, b)).or_insert(edge);
            }
        }
        let mut remaining: BTreeMap<_, _> = dependencies
            .iter()
            .map(|(id, targets)| (*id, targets.len()))
            .collect();
        let mut rank: BTreeMap<_, usize> = dependencies.keys().map(|id| (*id, 0)).collect();
        let mut ready: BTreeSet<_> = remaining
            .iter()
            .filter_map(|(id, count)| (*count == 0).then_some(*id))
            .collect();
        while let Some(id) = ready.pop_first() {
            for parent in &dependents[&id] {
                rank.insert(*parent, rank[parent].max(rank[&id] + 1));
                let count = remaining.get_mut(parent).unwrap();
                *count -= 1;
                if *count == 0 {
                    ready.insert(*parent);
                }
            }
        }
        let levels = rank.values().max().copied().unwrap_or(0) + 1;
        let scope = if members.len() == neighbors.len() {
            "."
        } else {
            members
                .iter()
                .map(|id| nodes[id].name.as_str())
                .min()
                .unwrap()
        };
        // Follow a longest condensed path. Each recorded link is an original
        // edge; crossing an SCC does not invent an internal file-level path.
        let mut current = *rank
            .iter()
            .filter(|(_, r)| **r + 1 == levels)
            .map(|(id, _)| id)
            .min()
            .unwrap();
        let mut witness = Vec::new();
        while let Some(next) = dependencies[&current]
            .iter()
            .filter(|id| rank[id] + 1 == rank[&current])
            .min()
        {
            witness.push(boundary[&(current, *next)]);
            current = *next;
        }
        if levels >= 3 {
            level_candidates += 1;
            let mut p = assessment(
                "dependency_layers",
                scope,
                "dependency_structure",
                &[
                    "Dependency levels describe the recorded graph, not architectural intent or verified layer responsibilities.",
                    "Cycles are condensed only for level calculation; there is no internal ordering of files inside a cycle. Missing dependencies can change these levels.",
                ],
            );
            p.status = if cycles.is_empty() {
                "candidate"
            } else {
                "mixed"
            }
            .into();
            p.nodes = members.iter().map(|id| nodes[id].name.clone()).collect();
            p.metrics = json!({"source_files":members.len(),"scope_source_files":graph.nodes.len(),"source_dependencies":edges.len(),"dependency_levels":levels,"cyclic_groups":cycles.len(),"cyclic_files":cycles.iter().map(|s|s.node_ids.len()).sum::<usize>(),"witness_connected":witness.windows(2).all(|pair|pair[0].to==pair[1].from),"candidate_rule":"at least three dependency levels within one connected region, after cycle condensation"});
            for edge in &witness {
                add_link(&mut p, edge, "dependency_level", &nodes);
            }
            for cycle in &cycles {
                add(
                    &mut p,
                    evidence(
                        "cycle",
                        None,
                        None,
                        None,
                        None,
                        &format!(
                            "Cycle within this region: {}",
                            cycle
                                .node_ids
                                .iter()
                                .map(|id| nodes[id].name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    ),
                    true,
                );
            }
            assessments.push(p);
        }
        let chain = edges.len() + 1 == members.len()
            && cycles.is_empty()
            && incoming.values().all(|n| *n <= 1)
            && outgoing.values().all(|n| *n <= 1);
        if chain && !global_chain {
            let mut p = assessment(
                "pipeline",
                scope,
                "dependency_structure",
                &[
                    "This chain describes one connected source region, not the whole project. Static dependency order does not prove runtime flow or pipes-and-filters.",
                ],
            );
            p.status = "candidate".into();
            p.nodes = members.iter().map(|id| nodes[id].name.clone()).collect();
            p.metrics = json!({"active_files":members.len(),"scope_source_files":graph.nodes.len(),"dependencies":edges.len(),"cyclic_groups":0,"candidate_rule":"one connected acyclic region with at least three files and in/out degree at most one"});
            for edge in &witness {
                add_link(&mut p, edge, "dependency", &nodes);
            }
            assessments.push(p);
        }
    }
    if level_candidates == 0 {
        assessments.push(assessment("dependency_layers", ".", "dependency_structure", &[
            "No connected region has at least three dependency levels after cycle condensation; names alone are not structural evidence.",
        ]));
    }
}

fn add_link(
    p: &mut PatternAssessment,
    edge: &Edge,
    kind: &str,
    nodes: &BTreeMap<NodeId, &archguard_core::Node>,
) {
    let from = nodes[&edge.from];
    let to = nodes[&edge.to];
    let reference = edge.refs.first();
    add(
        p,
        evidence(
            kind,
            Some(&from.name),
            Some(&to.name),
            reference.map(|r| r.file.as_str()),
            reference.map(|r| r.line),
            reference
                .map(|r| r.raw.as_str())
                .unwrap_or("Recorded dependency; source location unavailable."),
        ),
        false,
    );
}
