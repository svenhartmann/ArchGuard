//! Structural package cohesion from observed file dependencies.
use archguard_core::{GitStatus, Graph, NodeId, NodeKind, Ref};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CohesionReport {
    pub schema_version: u32,
    pub status: String,
    pub definition: String,
    pub packages: Vec<CohesionPackage>,
    pub diagnostics: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CohesionPackage {
    pub node: String,
    pub files: usize,
    pub internal_dependencies: usize,
    pub incoming_dependencies: usize,
    pub outgoing_dependencies: usize,
    pub internal_share: Option<f64>,
    pub groups: Vec<CohesionGroup>,
    pub split_candidate: bool,
    pub co_change_status: String,
    pub cross_group_co_changes: usize,
    pub reason: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CohesionGroup {
    pub files: Vec<String>,
    pub internal_dependencies: usize,
    pub incoming_dependencies: usize,
    pub outgoing_dependencies: usize,
    pub boundaries: Vec<CohesionBoundary>,
    pub boundaries_truncated: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CohesionBoundary {
    pub from: String,
    pub to: String,
    pub evidence: Vec<Ref>,
}

/// Packages contain their immediate real source-file nodes. Nested packages are
/// separate scopes. No inferred semantic connection or automatic move is emitted.
#[tracing::instrument(skip_all)]
pub fn analyze_cohesion(
    graph: &Graph,
    co_change: Option<&crate::co_change::CoChangeReport>,
) -> CohesionReport {
    let nodes: BTreeMap<_, _> = graph.nodes.iter().map(|n| (n.id, n)).collect();
    let files: BTreeMap<_, _> = nodes
        .iter()
        .filter(|(_, n)| n.kind == NodeKind::File && !n.name.ends_with("/*"))
        .map(|(id, n)| (*id, *n))
        .collect();
    let mut members = BTreeMap::<String, Vec<NodeId>>::new();
    let mut package_of = BTreeMap::new();
    for (&id, file) in &files {
        let name = file
            .parent
            .and_then(|p| nodes.get(&p))
            .filter(|n| n.kind == NodeKind::Module)
            .map(|n| n.name.as_str())
            .unwrap_or(".");
        members.entry(name.into()).or_default().push(id);
        package_of.insert(id, name.to_string());
    }
    let mut pairs = BTreeMap::<(NodeId, NodeId), Vec<Ref>>::new();
    let mut adjacent = BTreeMap::<NodeId, BTreeSet<NodeId>>::new();
    for edge in &graph.edges {
        if edge.from == edge.to || !files.contains_key(&edge.from) || !files.contains_key(&edge.to)
        {
            continue;
        }
        let refs = pairs.entry((edge.from, edge.to)).or_default();
        for evidence in &edge.refs {
            if refs.len() < 4 && !refs.contains(evidence) {
                refs.push(evidence.clone());
            }
        }
        if package_of[&edge.from] == package_of[&edge.to] {
            adjacent.entry(edge.from).or_default().insert(edge.to);
            adjacent.entry(edge.to).or_default().insert(edge.from);
        }
    }
    let mut packages = Vec::new();
    let mut locations = BTreeMap::<NodeId, (usize, usize)>::new();
    for (name, mut ids) in members {
        ids.sort_by_key(|id| &files[id].name);
        let mut seen = BTreeSet::new();
        let mut groups = Vec::new();
        for id in &ids {
            if !seen.insert(*id) {
                continue;
            }
            let mut stack = vec![*id];
            let mut group = Vec::new();
            while let Some(current) = stack.pop() {
                group.push(files[&current].name.clone());
                locations.insert(current, (packages.len(), groups.len()));
                for next in adjacent.get(&current).into_iter().flatten() {
                    if seen.insert(*next) {
                        stack.push(*next);
                    }
                }
            }
            group.sort();
            groups.push(CohesionGroup {
                files: group,
                internal_dependencies: 0,
                incoming_dependencies: 0,
                outgoing_dependencies: 0,
                boundaries: vec![],
                boundaries_truncated: false,
            });
        }
        packages.push(CohesionPackage {
            node: name,
            files: ids.len(),
            internal_dependencies: 0,
            incoming_dependencies: 0,
            outgoing_dependencies: 0,
            internal_share: None,
            groups,
            split_candidate: false,
            co_change_status: match co_change {
                None => "not_checked",
                Some(c)
                    if c.git.status == GitStatus::Available
                        && !c.git.shallow
                        && !c.git.truncated
                        && !c.results_truncated
                        && !c.pair_search_truncated =>
                {
                    "available"
                }
                Some(_) => "partial",
            }
            .into(),
            cross_group_co_changes: 0,
            reason: String::new(),
        });
    }
    for ((from, to), evidence) in pairs {
        let (fp, fg) = locations[&from];
        let (tp, tg) = locations[&to];
        if fp == tp {
            packages[fp].internal_dependencies += 1;
            packages[fp].groups[fg].internal_dependencies += 1;
        } else {
            packages[fp].outgoing_dependencies += 1;
            packages[tp].incoming_dependencies += 1;
            packages[fp].groups[fg].outgoing_dependencies += 1;
            packages[tp].groups[tg].incoming_dependencies += 1;
            let boundary = CohesionBoundary {
                from: files[&from].name.clone(),
                to: files[&to].name.clone(),
                evidence,
            };
            for (p, g) in [(fp, fg), (tp, tg)] {
                let group = &mut packages[p].groups[g];
                if group.boundaries.len() < 12 {
                    group.boundaries.push(boundary.clone());
                } else {
                    group.boundaries_truncated = true;
                }
            }
        }
    }
    if let Some(changes) = co_change {
        let by_name: BTreeMap<_, _> = files.iter().map(|(id, n)| (n.name.as_str(), id)).collect();
        let mut seen = BTreeSet::new();
        for pair in &changes.pairs {
            if let (Some(left), Some(right)) = (
                by_name.get(pair.left.name.as_str()),
                by_name.get(pair.right.name.as_str()),
            ) {
                let (lp, lg) = locations[left];
                let (rp, rg) = locations[right];
                let ordered = if left < right {
                    (*left, *right)
                } else {
                    (*right, *left)
                };
                if lp == rp && lg != rg && seen.insert(ordered) {
                    packages[lp].cross_group_co_changes += 1;
                }
            }
        }
    }
    for package in &mut packages {
        let denominator = package.internal_dependencies + package.outgoing_dependencies;
        package.internal_share =
            (denominator > 0).then(|| package.internal_dependencies as f64 / denominator as f64);
        package.split_candidate = package
            .groups
            .iter()
            .filter(|g| g.internal_dependencies > 0)
            .count()
            >= 2;
        package.reason=if package.split_candidate {
            if package.cross_group_co_changes>0 {"Multiple internally connected groups have no observed internal dependency between them, but cross-group co-change evidence argues for reviewing shared responsibilities before splitting."}
            else {"Multiple internally connected groups have no observed internal dependency between them. Review their boundary evidence and domain responsibilities before splitting; this does not prove independent behavior."}
        } else if package.internal_dependencies==0 {"Observed internal connections are insufficient for a split recommendation; independent entry points, data declarations, or missing analysis may explain isolated files."}
        else {"No split candidate with two independently connected internal groups was found in this source graph."}.into();
    }
    CohesionReport{schema_version:1,status:if graph.semantic.as_ref().is_some_and(|s|s.status=="partial" || s.status=="unavailable") {"partial"} else {"ready"}.into(),definition:"direct-package-file-dependencies-v1".into(),packages,diagnostics:vec!["Immediate real file members only; nested packages and namespace placeholders are excluded. Internal share = internal distinct directed file pairs / (internal + outgoing distinct pairs); incoming pairs are reported separately. A zero denominator has no percentage.".into(),"Groups are weakly connected components of observed internal file dependencies. These are structural review candidates, not class-level LCOM, domain boundaries, or proof that a move preserves behavior. Co-change results are thresholded observations; absent pairs do not prove independent changes. Boundary samples retain at most 12 pairs and 4 references per pair.".into()]}
}
