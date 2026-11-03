//! Evidence-backed selection of a small change-oriented graph neighborhood.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use archguard_core::{
    ArchguardError, CallAnalysis, CallClass, CallMethod, Config, Edge, Graph, Node, NodeId,
    NodeKind,
};
use globset::Glob;
use serde_json::{Value, json};

pub(crate) struct ContextSelection {
    pub items: Vec<Value>,
    pub matched_files: Vec<String>,
}

pub(crate) fn select_context(
    graph: &Graph,
    calls: &CallAnalysis,
    config: &Config,
    source_nodes: &BTreeMap<String, Vec<NodeId>>,
    targets: &[String],
    symbols: &[String],
    depth: usize,
) -> Result<ContextSelection, ArchguardError> {
    if depth > 3 {
        return Err(ArchguardError::Analysis(
            "change context depth must be between 0 and 3".into(),
        ));
    }
    let index = SourceIndex::new(graph, config, source_nodes)?;
    let mut seeds = BTreeSet::new();
    for target in targets {
        seeds.insert(index.target(target)?);
    }
    let mut selected_symbols = BTreeMap::new();
    for symbol in symbols {
        let selected = select_symbol(calls, symbol)?;
        let primary = match selected {
            SelectedSymbol::Class(class) => {
                let primary = index.reference_target(&class.file)?;
                if index.nodes[&primary].name != class.graph_node {
                    return Err(ArchguardError::Analysis(format!(
                        "symbol {symbol:?} does not match its graph source"
                    )));
                }
                let declarations = class
                    .declaration_files
                    .as_deref()
                    .unwrap_or(std::slice::from_ref(&class.file));
                let mut files = vec![];
                for declaration in declarations {
                    let node = index.reference_target(declaration)?;
                    seeds.insert(node);
                    files.push(index.files[&node]);
                }
                files.sort();
                files.dedup();
                selected_symbols.insert(class.id.clone(), json!({"kind":"symbol","symbol_kind":"class","id":class.id,"name":class.name,"file":index.files[&primary],"reference_file":class.file,"line":class.line,"declaration_files":files}));
                primary
            }
            SelectedSymbol::Method(method, class) => {
                let owner = index.reference_target(&class.file)?;
                if index.nodes[&owner].name != class.graph_node {
                    return Err(ArchguardError::Analysis(format!(
                        "symbol {symbol:?} does not match its graph source"
                    )));
                }
                let primary = index.reference_target(&method.file)?;
                selected_symbols.insert(method.id.clone(),json!({"kind":"symbol","symbol_kind":"method","id":method.id,"class_id":class.id,"name":format!("{}.{}",class.name,method.name),"signature":method.signature,"file":index.files[&primary],"reference_file":method.file,"line":method.line}));
                primary
            }
        };
        seeds.insert(primary);
    }
    if seeds.is_empty() {
        return Ok(ContextSelection {
            items: vec![],
            matched_files: vec![],
        });
    }
    let mut ordered_seeds: Vec<_> = seeds.iter().copied().collect();
    ordered_seeds.sort_by_key(|id| index.nodes[id].name.as_str());
    let mut items: Vec<Value> = ordered_seeds.iter().map(|id|json!({"kind":"seed","node":index.nodes[id].name,"file":index.files[id],"depth":0,"path":[]})).collect();
    items.extend(selected_symbols.values().cloned());
    items.extend(call_evidence(calls, &selected_symbols, &index));
    let incoming = neighborhood(graph, &index, &ordered_seeds, depth, true);
    let outgoing = neighborhood(graph, &index, &ordered_seeds, depth, false);
    let mut neighbors = vec![];
    for (kind, paths) in [("dependent", incoming), ("dependency", outgoing)] {
        for (id, path) in paths {
            if seeds.contains(&id) {
                continue;
            }
            let witness: Vec<_> = path.iter().map(|edge| index.edge(edge)).collect();
            neighbors.push((path.len(),if kind=="dependent" {0} else {1},index.nodes[&id].name.clone(),json!({"kind":kind,"node":index.nodes[&id].name,"file":index.files[&id],"depth":path.len(),"path":witness})));
        }
    }
    neighbors.sort_by(|a, b| (&a.0, &a.1, &a.2).cmp(&(&b.0, &b.1, &b.2)));
    // Alternate the alphabetically ordered directions at each distance so a
    // bounded first page can include both callers and dependencies.
    let mut positions = BTreeMap::<_, usize>::new();
    let mut neighbors: Vec<_> = neighbors
        .into_iter()
        .map(|(depth, direction, _, item)| {
            let position = positions.entry((depth, direction)).or_default();
            let row = (depth, *position, direction, item);
            *position += 1;
            row
        })
        .collect();
    neighbors.sort_by_key(|row| (row.0, row.1, row.2));
    items.extend(
        neighbors
            .iter()
            .filter(|row| row.0 == 1)
            .map(|row| row.3.clone()),
    );
    for cycle in archguard_analysis::find_sccs(graph)
        .into_iter()
        .filter(|cycle| cycle.node_ids.iter().any(|id| seeds.contains(id)))
    {
        let members: Vec<_> = cycle
            .node_ids
            .iter()
            .map(|id| json!({"node":index.nodes[id].name,"file":index.files.get(id)}))
            .collect();
        let edges: Vec<_> = cycle
            .edges
            .iter()
            .take(3)
            .filter_map(|(from, to)| graph.edges.iter().find(|e| e.from == *from && e.to == *to))
            .map(|edge| index.edge(edge))
            .collect();
        items.push(json!({"kind":"cycle","level":"file","id":cycle.id,"members":members,"edges_total":cycle.edges.len(),"edge_sample":edges}));
    }
    let names: Vec<_> = ordered_seeds
        .iter()
        .map(|id| index.nodes[id].name.as_str())
        .collect();
    let layer_names: Vec<_> = ordered_seeds
        .iter()
        .map(|id| {
            let mut lineage = vec![*id];
            let mut current = *id;
            while let Some(parent) = index.nodes.get(&current).and_then(|node| node.parent) {
                if lineage.contains(&parent) || !index.nodes.contains_key(&parent) {
                    break;
                }
                lineage.push(parent);
                current = parent;
            }
            lineage.reverse();
            let visible = lineage.get(config.modules.depth as usize).unwrap_or(id);
            index.nodes[visible].name.as_str()
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    items.extend(policies(config, &names, &layer_names)?);
    items.push(json!({"kind":"coverage","impact_scope":"conservative_file_dependencies","namespace_nodes":graph.nodes.iter().filter(|n|n.kind==NodeKind::File && n.name.ends_with("/*")).count(),"unmapped_file_nodes":graph.nodes.iter().filter(|n|n.kind==NodeKind::File && !n.name.ends_with("/*") && !index.files.contains_key(&n.id)).count(),"note":"Namespace-only and ambiguous file nodes are excluded from concrete dependency paths; this is static evidence, not proof of runtime reachability."}));
    items.extend(
        neighbors
            .into_iter()
            .filter(|row| row.0 > 1)
            .map(|row| row.3),
    );
    let matched_files = ordered_seeds
        .iter()
        .map(|id| index.files[id].to_string())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    Ok(ContextSelection {
        items,
        matched_files,
    })
}

struct SourceIndex<'a> {
    nodes: BTreeMap<NodeId, &'a Node>,
    names: BTreeMap<&'a str, NodeId>,
    files: BTreeMap<NodeId, &'a str>,
    references: BTreeMap<String, Option<&'a str>>,
    source_nodes: &'a BTreeMap<String, Vec<NodeId>>,
}

impl<'a> SourceIndex<'a> {
    fn new(
        graph: &'a Graph,
        config: &Config,
        source_nodes: &'a BTreeMap<String, Vec<NodeId>>,
    ) -> Result<Self, ArchguardError> {
        let mut nodes = BTreeMap::new();
        let mut names = BTreeMap::new();
        for node in &graph.nodes {
            if nodes.insert(node.id, node).is_some()
                || names.insert(node.name.as_str(), node.id).is_some()
            {
                return Err(ArchguardError::Analysis(
                    "duplicate graph node identity in change context".into(),
                ));
            }
        }
        let mut candidates = BTreeMap::<NodeId, BTreeSet<&str>>::new();
        let mut references = BTreeMap::new();
        for (file, ids) in source_nodes {
            let reference = archguard_extract::strip_root(file, &config.project.source_roots);
            references
                .entry(reference)
                .and_modify(|entry| *entry = None)
                .or_insert(Some(file.as_str()));
            // Even an ambiguous source contributes to reverse ambiguity; it
            // must never allow another physical file to claim this node.
            for id in ids {
                candidates.entry(*id).or_default().insert(file);
            }
        }
        let files = candidates
            .into_iter()
            .filter_map(|(id, paths)| {
                let node = nodes.get(&id)?;
                if node.kind != NodeKind::File || node.name.ends_with("/*") || paths.len() != 1 {
                    return None;
                }
                let file = *paths.first()?;
                (source_nodes[file].as_slice() == [id]).then_some((id, file))
            })
            .collect();
        Ok(Self {
            nodes,
            names,
            files,
            references,
            source_nodes,
        })
    }

    fn target(&self, target: &str) -> Result<NodeId, ArchguardError> {
        let id = match self.source_nodes.get(target) {
            Some(ids) if ids.len() == 1 => Some(ids[0]),
            Some(_) => None,
            None => self.names.get(target).copied(),
        };
        id.filter(|id| self.files.contains_key(id)).ok_or_else(|| {
            ArchguardError::Analysis(format!(
                "unknown, non-file or ambiguous change target {target:?}"
            ))
        })
    }

    fn reference_target(&self, reference: &str) -> Result<NodeId, ArchguardError> {
        self.references
            .get(reference)
            .copied()
            .flatten()
            .ok_or_else(|| {
                ArchguardError::Analysis(format!(
                    "unknown or ambiguous symbol source {reference:?}"
                ))
            })
            .and_then(|file| self.target(file))
    }

    fn edge(&self, edge: &Edge) -> Value {
        let mut refs: Vec<_> = edge.refs.iter().collect();
        refs.sort_by(|a, b| (&a.file, a.line, &a.raw).cmp(&(&b.file, b.line, &b.raw)));
        refs.dedup();
        let evidence:Vec<_>=refs.iter().take(3).map(|r|json!({"file":self.references.get(&r.file).copied().flatten(),"reference_file":r.file,"line":r.line,"raw":r.raw})).collect();
        json!({"from":self.nodes[&edge.from].name,"to":self.nodes[&edge.to].name,"evidence":evidence,"evidence_total":edge.weight(),"evidence_count_exact":edge.count.is_some()})
    }
}

enum SelectedSymbol<'a> {
    Class(&'a CallClass),
    Method(&'a CallMethod, &'a CallClass),
}

fn select_symbol<'a>(
    calls: &'a CallAnalysis,
    query: &str,
) -> Result<SelectedSymbol<'a>, ArchguardError> {
    let classes: Vec<_> = calls
        .classes
        .iter()
        .filter(|class| class.id == query)
        .collect();
    let methods: Vec<_> = calls
        .methods
        .iter()
        .filter(|method| method.id == query)
        .collect();
    let (classes, methods) = if classes.is_empty() && methods.is_empty() {
        (
            calls
                .classes
                .iter()
                .filter(|class| class.name == query)
                .collect::<Vec<_>>(),
            calls
                .methods
                .iter()
                .filter(|method| {
                    calls.classes.iter().any(|class| {
                        class.id == method.class_id
                            && format!("{}.{}", class.name, method.name) == query
                    })
                })
                .collect::<Vec<_>>(),
        )
    } else {
        (classes, methods)
    };
    if classes.len() + methods.len() != 1 {
        return Err(ArchguardError::Analysis(format!(
            "unknown or ambiguous symbol {query:?}; use its unique compiler ID (including overload signature)"
        )));
    }
    if let Some(class) = classes.first() {
        if calls
            .classes
            .iter()
            .filter(|candidate| candidate.id == class.id)
            .count()
            != 1
        {
            return Err(ArchguardError::Analysis(format!(
                "ambiguous compiler class identity {:?}",
                class.id
            )));
        }
        return Ok(SelectedSymbol::Class(class));
    }
    let method = methods[0];
    let owners: Vec<_> = calls
        .classes
        .iter()
        .filter(|class| class.id == method.class_id)
        .collect();
    if owners.len() != 1
        || calls
            .methods
            .iter()
            .filter(|candidate| candidate.id == method.id)
            .count()
            != 1
    {
        return Err(ArchguardError::Analysis(format!(
            "ambiguous compiler method identity {:?}",
            method.id
        )));
    }
    Ok(SelectedSymbol::Method(method, owners[0]))
}

fn neighborhood<'a>(
    graph: &'a Graph,
    index: &SourceIndex<'_>,
    seeds: &[NodeId],
    depth: usize,
    incoming: bool,
) -> BTreeMap<NodeId, Vec<&'a Edge>> {
    let mut adjacency = BTreeMap::<NodeId, Vec<&Edge>>::new();
    for edge in &graph.edges {
        if !index.files.contains_key(&edge.from) || !index.files.contains_key(&edge.to) {
            continue;
        }
        adjacency
            .entry(if incoming { edge.to } else { edge.from })
            .or_default()
            .push(edge);
    }
    for edges in adjacency.values_mut() {
        edges.sort_by_key(|edge| {
            index.nodes[&if incoming { edge.from } else { edge.to }]
                .name
                .as_str()
        });
    }
    let mut paths: BTreeMap<NodeId, Vec<&Edge>> = seeds.iter().map(|id| (*id, vec![])).collect();
    let mut queue: VecDeque<_> = seeds.iter().copied().collect();
    while let Some(current) = queue.pop_front() {
        if paths[&current].len() >= depth {
            continue;
        }
        for edge in adjacency.get(&current).into_iter().flatten() {
            let next = if incoming { edge.from } else { edge.to };
            if paths.contains_key(&next) {
                continue;
            }
            let mut path = paths[&current].clone();
            if incoming {
                path.insert(0, *edge);
            } else {
                path.push(*edge);
            }
            paths.insert(next, path);
            queue.push_back(next);
        }
    }
    paths
}

fn call_evidence(
    calls: &CallAnalysis,
    selected: &BTreeMap<String, Value>,
    index: &SourceIndex<'_>,
) -> Vec<Value> {
    let classes: BTreeMap<_, _> = calls
        .classes
        .iter()
        .fold(BTreeMap::new(), |mut map, class| {
            map.entry(class.id.as_str())
                .and_modify(|entry| *entry = None)
                .or_insert(Some(class));
            map
        });
    let methods: BTreeMap<_, _> = calls
        .methods
        .iter()
        .fold(BTreeMap::new(), |mut map, method| {
            map.entry(method.id.as_str())
                .and_modify(|entry| *entry = None)
                .or_insert(Some(method));
            map
        });
    let selected_class = |id: &str| {
        selected
            .get(id)
            .is_some_and(|v| v["symbol_kind"] == "class")
    };
    let selected_method = |id: Option<&str>| {
        id.is_some_and(|id| {
            selected
                .get(id)
                .is_some_and(|v| v["symbol_kind"] == "method")
        })
    };
    let mut evidence = vec![];
    for call in &calls.calls {
        if call.resolution != "project" {
            continue;
        }
        let incoming = call.to_class.as_deref().is_some_and(selected_class)
            || selected_method(call.to_method.as_deref());
        let outgoing =
            selected_class(&call.from_class) || selected_method(call.from_method.as_deref());
        if !incoming && !outgoing {
            continue;
        }
        let from = classes.get(call.from_class.as_str()).copied().flatten();
        let to = call
            .to_class
            .as_deref()
            .and_then(|id| classes.get(id))
            .copied()
            .flatten();
        if from.is_none()
            || to.is_none()
            || [
                (call.from_method.as_deref(), Some(call.from_class.as_str())),
                (call.to_method.as_deref(), call.to_class.as_deref()),
            ]
            .into_iter()
            .any(|(id, owner)| {
                id.is_some_and(|id| {
                    methods
                        .get(id)
                        .copied()
                        .flatten()
                        .is_none_or(|method| Some(method.class_id.as_str()) != owner)
                })
            })
        {
            continue;
        }
        let Ok(source) = index.reference_target(&call.file) else {
            continue;
        };
        evidence.push(json!({"kind":"call_evidence","direction":if incoming && outgoing {"internal"} else if incoming {"incoming"} else {"outgoing"},"from_class":call.from_class,"from_method":call.from_method,"to_class":call.to_class,"to_method":call.to_method,"resolution":call.resolution,"dispatch":call.dispatch,"file":index.files[&source],"reference_file":call.file,"line":call.line,"column":call.column,"code":call.code}));
    }
    evidence.sort_by_key(Value::to_string);
    evidence.dedup();
    evidence
}

fn policies(
    config: &Config,
    names: &[&str],
    layer_names: &[&str],
) -> Result<Vec<Value>, ArchguardError> {
    let matches = |patterns: &[String], candidates: &[&str]| -> Result<bool, ArchguardError> {
        let mut matched = false;
        for pattern in patterns {
            let glob = Glob::new(pattern)
                .map_err(|error| ArchguardError::Config(error.to_string()))?
                .compile_matcher();
            matched |= candidates.iter().any(|name| glob.is_match(name));
        }
        Ok(matched)
    };
    let mut items = vec![];
    for rule in &config.dependency_rules {
        if matches(&rule.from, names)? || matches(&rule.to, names)? {
            items.push(json!({"kind":"policy","policy_type":"dependency_rule","value":rule}));
        }
    }
    let mut remaining: BTreeSet<_> = layer_names.iter().copied().collect();
    for layer in &config.layers {
        let mut applies_to = vec![];
        for name in &remaining {
            if matches(&layer.contains, &[name])? {
                applies_to.push(*name);
            }
        }
        for name in &applies_to {
            remaining.remove(name);
        }
        if !applies_to.is_empty() {
            items.push(
                json!({"kind":"policy","policy_type":"layer","scope":"configured_module_view","value":layer,"origin":layer.origin,"applies_to":applies_to}),
            );
        }
    }
    if let Some(model) = &config.architecture.model {
        let mut assignments: BTreeMap<_, Vec<&str>> =
            names.iter().map(|name| (*name, vec![])).collect();
        for component in &model.components {
            for pattern in &component.include {
                let glob = Glob::new(pattern)
                    .map_err(|error| ArchguardError::Config(error.to_string()))?
                    .compile_matcher();
                for (name, assigned) in &mut assignments {
                    if glob.is_match(name) && !assigned.contains(&component.name.as_str()) {
                        assigned.push(&component.name);
                    }
                }
            }
            if matches(&component.include, names)? {
                let directions: Vec<_> = model
                    .allowed_dependencies
                    .iter()
                    .filter(|d| d.from == component.name || d.to == component.name)
                    .collect();
                items.push(json!({"kind":"policy","policy_type":"architecture_component","model":model.name,"value":component,"allowed_dependencies":directions}));
            }
        }
        let unassigned: Vec<_> = assignments
            .iter()
            .filter(|(_, components)| components.is_empty())
            .map(|(name, _)| *name)
            .collect();
        let ambiguous: Vec<_> = assignments
            .iter()
            .filter(|(_, components)| components.len() > 1)
            .map(|(name, components)| json!({"node":name,"components":components}))
            .collect();
        items.push(json!({"kind":"policy","policy_type":"architecture_model","name":model.name,"severity":model.severity,"require_assignment":model.require_assignment,"unassigned_seeds":unassigned,"ambiguous_seeds":ambiguous,"cross_component_policy":"deny_unlisted_directions"}));
    }
    items.push(json!({"kind":"policy","policy_type":"quality_gates","scope":"configured_analysis_views","value":config.rules}));
    for goal in &config.architecture.goals {
        items.push(json!({"kind":"policy","policy_type":"architecture_goal","scope":"project","value":goal}));
    }
    for constraint in &config.architecture.constraints {
        items.push(json!({"kind":"policy","policy_type":"architecture_constraint","scope":"project","value":constraint}));
    }
    for decision in &config.architecture.decisions {
        items.push(json!({"kind":"policy","policy_type":"architecture_decision","scope":"project","value":decision}));
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{CallClass, CallMethod, DependencyRule, GraphBuilder, Ref};
    use serde_json::json;

    fn fixture(edges: &[(&str, &str)]) -> (Graph, BTreeMap<String, Vec<NodeId>>) {
        let mut builder = GraphBuilder::new();
        for (from, to) in edges {
            let a = builder.ensure_path(from, Some("ts"));
            let b = builder.ensure_path(to, Some("ts"));
            builder
                .edge(
                    a,
                    b,
                    Ref {
                        file: from.to_string(),
                        line: 7,
                        raw: format!("import '{to}'"),
                    },
                )
                .unwrap();
        }
        let graph = builder.build("/project".into());
        let sources = graph
            .nodes
            .iter()
            .filter(|n| n.kind == archguard_core::NodeKind::File && !n.name.ends_with("/*"))
            .map(|n| (format!("src/{}", n.name), vec![n.id]))
            .collect();
        (graph, sources)
    }

    fn run(
        graph: &Graph,
        sources: &BTreeMap<String, Vec<NodeId>>,
        depth: usize,
    ) -> ContextSelection {
        select_context(
            graph,
            &CallAnalysis::default(),
            &Config::default(),
            sources,
            &["src/core.ts".into()],
            &[],
            depth,
        )
        .unwrap()
    }

    fn item<'a>(selection: &'a ContextSelection, kind: &str, node: &str) -> &'a Value {
        selection
            .items
            .iter()
            .find(|v| v["kind"] == kind && v["node"] == node)
            .unwrap_or_else(|| panic!("missing {kind} {node}: {:?}", selection.items))
    }

    #[test]
    fn incoming_and_outgoing_paths_never_mix_directions() {
        let (g, s) = fixture(&[
            ("entry.ts", "core.ts"),
            ("top.ts", "entry.ts"),
            ("core.ts", "leaf.ts"),
            ("other.ts", "leaf.ts"),
            ("entry.ts", "unrelated.ts"),
        ]);
        let selection = run(&g, &s, 2);
        assert_eq!(selection.matched_files, vec!["src/core.ts"]);
        assert_eq!(item(&selection, "seed", "core.ts")["file"], "src/core.ts");
        let top = item(&selection, "dependent", "top.ts");
        assert_eq!(top["depth"], 2);
        assert_eq!(top["path"][0]["from"], "top.ts");
        assert_eq!(top["path"][0]["to"], "entry.ts");
        assert_eq!(top["path"][1]["to"], "core.ts");
        assert_eq!(top["path"][0]["evidence"][0]["file"], "src/top.ts");
        assert_eq!(top["path"][0]["evidence"][0]["line"], 7);
        assert_eq!(item(&selection, "dependency", "leaf.ts")["depth"], 1);
        assert!(
            !selection
                .items
                .iter()
                .any(|v| v["node"] == "other.ts" || v["node"] == "unrelated.ts")
        );
        let shallow = run(&g, &s, 1);
        assert!(!shallow.items.iter().any(|v| v["node"] == "top.ts"));
    }

    #[test]
    fn neighbors_alternate_directions_at_each_depth_before_spending_the_packet_budget() {
        let (mut graph, sources) = fixture(&[
            ("z_in.ts", "core.ts"),
            ("a_in.ts", "core.ts"),
            ("m_in.ts", "core.ts"),
            ("core.ts", "b_out.ts"),
            ("core.ts", "y_out.ts"),
            ("a_deep.ts", "a_in.ts"),
            ("z_deep.ts", "z_in.ts"),
            ("b_out.ts", "deep_a.ts"),
            ("y_out.ts", "deep_z.ts"),
        ]);
        let selected = run(&graph, &sources, 2);
        let neighbors: Vec<_> = selected
            .items
            .iter()
            .filter(|row| row["kind"] == "dependent" || row["kind"] == "dependency")
            .map(|row| {
                (
                    row["depth"].as_u64().unwrap(),
                    row["kind"].as_str().unwrap(),
                    row["node"].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            neighbors,
            vec![
                (1, "dependent", "a_in.ts"),
                (1, "dependency", "b_out.ts"),
                (1, "dependent", "m_in.ts"),
                (1, "dependency", "y_out.ts"),
                (1, "dependent", "z_in.ts"),
                (2, "dependent", "a_deep.ts"),
                (2, "dependency", "deep_a.ts"),
                (2, "dependent", "z_deep.ts"),
                (2, "dependency", "deep_z.ts"),
            ]
        );
        let policy = selected
            .items
            .iter()
            .position(|row| row["kind"] == "policy")
            .unwrap();
        let transitive = selected
            .items
            .iter()
            .position(|row| row["depth"] == 2)
            .unwrap();
        assert!(policy < transitive);
        graph.nodes.reverse();
        graph.edges.reverse();
        assert_eq!(selected.items, run(&graph, &sources, 2).items);
    }

    #[test]
    fn deterministic_shortest_witness_and_sampled_evidence() {
        let (mut g, s) = fixture(&[
            ("core.ts", "z.ts"),
            ("core.ts", "a.ts"),
            ("z.ts", "end.ts"),
            ("a.ts", "end.ts"),
        ]);
        let edge = g
            .edges
            .iter_mut()
            .find(|e| s["src/core.ts"].contains(&e.from) && s["src/a.ts"].contains(&e.to))
            .unwrap();
        for line in [20, 19, 18, 17] {
            edge.refs.push(Ref {
                file: "core.ts".into(),
                line,
                raw: "extra".into(),
            });
        }
        edge.count = Some(5);
        let first = run(&g, &s, 2);
        assert_eq!(
            item(&first, "dependency", "end.ts")["path"][0]["to"],
            "a.ts"
        );
        let witness = &item(&first, "dependency", "a.ts")["path"][0];
        assert_eq!(witness["evidence"].as_array().unwrap().len(), 3);
        assert_eq!(witness["evidence_total"], 5);
        g.nodes.reverse();
        g.edges.reverse();
        for edge in &mut g.edges {
            edge.refs.reverse();
        }
        assert_eq!(first.items, run(&g, &s, 2).items);
    }

    #[test]
    fn unknown_ambiguous_and_namespace_targets_are_rejected() {
        let (g, mut s) = fixture(&[("core.ts", "ns/*")]);
        for target in ["missing.ts", "ns/*"] {
            assert!(
                select_context(
                    &g,
                    &CallAnalysis::default(),
                    &Config::default(),
                    &s,
                    &[target.into()],
                    &[],
                    1
                )
                .is_err()
            );
        }
        let valid = run(&g, &s, 1);
        assert!(
            !valid
                .items
                .iter()
                .any(|v| v["kind"] == "dependency" && v["node"] == "ns/*")
        );
        assert!(
            valid
                .items
                .iter()
                .any(|v| v["kind"] == "coverage" && v["namespace_nodes"] == 1)
        );
        s.insert("other/core.ts".into(), s["src/core.ts"].clone());
        assert!(
            select_context(
                &g,
                &CallAnalysis::default(),
                &Config::default(),
                &s,
                &["src/core.ts".into()],
                &[],
                1
            )
            .is_err()
        );
    }

    #[test]
    fn malformed_duplicate_node_ids_are_not_silently_reused() {
        let (mut g, s) = fixture(&[("core.ts", "leaf.ts")]);
        g.nodes.push(g.nodes[0].clone());
        assert!(
            select_context(
                &g,
                &CallAnalysis::default(),
                &Config::default(),
                &s,
                &["src/core.ts".into()],
                &[],
                1
            )
            .is_err()
        );
    }

    #[test]
    fn cycles_and_relevant_policy_precede_distant_files() {
        let (g, s) = fixture(&[
            ("core.ts", "peer.ts"),
            ("peer.ts", "core.ts"),
            ("far.ts", "peer.ts"),
        ]);
        let config = Config {
            dependency_rules: vec![
                DependencyRule {
                    name: "core-boundary".into(),
                    from: vec!["core.ts".into()],
                    to: vec!["private/**".into()],
                    ..Default::default()
                },
                DependencyRule {
                    name: "unrelated".into(),
                    from: vec!["ui/**".into()],
                    to: vec!["db/**".into()],
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let selection = select_context(
            &g,
            &CallAnalysis::default(),
            &config,
            &s,
            &["core.ts".into()],
            &[],
            2,
        )
        .unwrap();
        let cycle = selection
            .items
            .iter()
            .position(|v| v["kind"] == "cycle")
            .unwrap();
        assert_eq!(
            selection.items[cycle]["level"], "file",
            "file SCC IDs must not be confused with module-view SCC IDs"
        );
        let policy = selection
            .items
            .iter()
            .position(|v| v["value"]["name"] == "core-boundary")
            .unwrap();
        let far = selection
            .items
            .iter()
            .position(|v| v["node"] == "far.ts")
            .unwrap();
        assert!(cycle < far && policy < far);
        assert!(
            !selection
                .items
                .iter()
                .any(|v| v["value"]["name"] == "unrelated")
        );
    }

    fn symbol_fixture() -> (Graph, BTreeMap<String, Vec<NodeId>>, CallAnalysis) {
        let (g, s) = fixture(&[("core.ts", "part.ts")]);
        let class: CallClass=serde_json::from_value(json!({"id":"project:Core","name":"demo.Core","package":"demo","file":"core.ts","graph_node":"core.ts","line":2,"kind":"class","declaration_files":["core.ts","part.ts"]})).unwrap();
        let method = CallMethod {
            id: "project:Core#run()".into(),
            class_id: class.id.clone(),
            name: "run".into(),
            signature: "run()".into(),
            file: "part.ts".into(),
            line: 15,
            end_line: None,
            is_static: false,
            complexity: None,
        };
        (
            g,
            s,
            CallAnalysis {
                classes: vec![class],
                methods: vec![method],
                ..Default::default()
            },
        )
    }

    #[test]
    fn qualified_method_selection_uses_actual_partial_declaration() {
        let (g, s, calls) = symbol_fixture();
        let selected = select_context(
            &g,
            &calls,
            &Config::default(),
            &s,
            &[],
            &["demo.Core.run".into()],
            1,
        )
        .unwrap();
        assert_eq!(selected.matched_files, vec!["src/part.ts"]);
        let symbol = selected
            .items
            .iter()
            .find(|v| v["kind"] == "symbol")
            .unwrap();
        assert_eq!(symbol["id"], "project:Core#run()");
        assert_eq!(symbol["file"], "src/part.ts");
        assert_eq!(symbol["line"], 15);
    }

    #[test]
    fn overloaded_alias_and_duplicate_compiler_ids_are_rejected() {
        let (g, s, mut calls) = symbol_fixture();
        let mut overload = calls.methods[0].clone();
        overload.id = "project:Core#run(int)".into();
        overload.signature = "run(int)".into();
        calls.methods.push(overload);
        assert!(
            select_context(
                &g,
                &calls,
                &Config::default(),
                &s,
                &[],
                &["demo.Core.run".into()],
                1
            )
            .is_err()
        );
        assert!(
            select_context(
                &g,
                &calls,
                &Config::default(),
                &s,
                &[],
                &["project:Core#run()".into()],
                1
            )
            .is_ok()
        );
        calls.methods.push(calls.methods[0].clone());
        assert!(
            select_context(
                &g,
                &calls,
                &Config::default(),
                &s,
                &[],
                &["project:Core#run()".into()],
                1
            )
            .is_err()
        );
    }

    #[test]
    fn evidence_mapping_never_guesses_ambiguous_reference_path() {
        let (g, mut s) = fixture(&[("core.ts", "leaf.ts")]);
        s.insert("other/core.ts".into(), vec![]);
        let mut config = Config::default();
        config.project.source_roots.push("other".into());
        let selected = select_context(
            &g,
            &CallAnalysis::default(),
            &config,
            &s,
            &["src/core.ts".into()],
            &[],
            1,
        )
        .unwrap();
        let evidence = &item(&selected, "dependency", "leaf.ts")["path"][0]["evidence"][0];
        assert!(evidence["file"].is_null());
        assert_eq!(evidence["reference_file"], "core.ts");
    }

    #[test]
    fn zero_depth_retains_selected_context_without_graph_neighbors() {
        let (g, s) = fixture(&[("core.ts", "leaf.ts")]);
        let selected = run(&g, &s, 0);
        assert_eq!(selected.matched_files, vec!["src/core.ts"]);
        assert!(
            selected
                .items
                .iter()
                .all(|v| v["kind"] != "dependency" && v["kind"] != "dependent")
        );
        assert!(
            select_context(
                &g,
                &CallAnalysis::default(),
                &Config::default(),
                &s,
                &["core.ts".into()],
                &[],
                4
            )
            .is_err()
        );
    }

    #[test]
    fn call_evidence_keeps_resolved_directions_and_rejects_wrong_method_owner() {
        let (g, s, mut calls) = symbol_fixture();
        let mut helper = calls.methods[0].clone();
        helper.id = "project:Core#helper()".into();
        helper.name = "helper".into();
        helper.file = "core.ts".into();
        calls.methods.push(helper.clone());
        let incoming:archguard_core::MethodCall=serde_json::from_value(json!({"from_class":"project:Core","from_method":helper.id,"to_class":"project:Core","to_method":"project:Core#run()","target_label":"run","dispatch":"instance","resolution":"project","file":"core.ts","line":10,"column":4,"code":"part.run()"})).unwrap();
        let mut outgoing = incoming.clone();
        outgoing.from_method = Some("project:Core#run()".into());
        outgoing.to_method = Some(helper.id.clone());
        outgoing.file = "part.ts".into();
        outgoing.line = 16;
        outgoing.code = "helper()".into();
        let mut external = incoming.clone();
        external.resolution = "external".into();
        external.line = 20;
        let mut wrong_owner = incoming.clone();
        wrong_owner.line = 30;
        let mut foreign = helper;
        foreign.id = "project:Other#foreign()".into();
        foreign.class_id = "project:Other".into();
        wrong_owner.from_method = Some(foreign.id.clone());
        calls.methods.push(foreign);
        calls.calls = vec![wrong_owner, external, outgoing, incoming];
        let selected = select_context(
            &g,
            &calls,
            &Config::default(),
            &s,
            &[],
            &["demo.Core.run".into()],
            1,
        )
        .unwrap();
        let evidence: Vec<_> = selected
            .items
            .iter()
            .filter(|v| v["kind"] == "call_evidence")
            .collect();
        assert_eq!(evidence.len(), 2);
        assert!(evidence.iter().any(|v| v["direction"] == "incoming"
            && v["file"] == "src/core.ts"
            && v["line"] == 10));
        assert!(evidence.iter().any(|v| v["direction"] == "outgoing"
            && v["file"] == "src/part.ts"
            && v["line"] == 16));
    }

    #[test]
    fn target_architecture_policy_covers_unassigned_seed() {
        let (g, s) = fixture(&[("core.ts", "leaf.ts")]);
        let config:Config=serde_json::from_value(json!({"architecture":{"model":{"name":"Boundaries","require_assignment":true,"components":[{"name":"Web","include":["ui/**"]}],"allowed_dependencies":[]}}})).unwrap();
        let selected = select_context(
            &g,
            &CallAnalysis::default(),
            &config,
            &s,
            &["core.ts".into()],
            &[],
            1,
        )
        .unwrap();
        let model = selected
            .items
            .iter()
            .find(|v| v["policy_type"] == "architecture_model")
            .unwrap();
        assert_eq!(model["require_assignment"], true);
        assert_eq!(model["unassigned_seeds"], json!(["core.ts"]));
        assert!(
            !selected
                .items
                .iter()
                .any(|v| v["policy_type"] == "architecture_component")
        );
    }

    #[test]
    fn layer_context_uses_configured_module_view_and_first_matching_layer() {
        let (g, s) = fixture(&[("app/core.ts", "domain/value.ts")]);
        let config:Config=serde_json::from_value(json!({"modules":{"depth":0},"layers":[{"name":"Application","contains":["app"],"may_depend_on":["Domain"]},{"name":"Shadowed","contains":["app"]},{"name":"Domain","contains":["domain"]}]})).unwrap();
        let selected = select_context(
            &g,
            &CallAnalysis::default(),
            &config,
            &s,
            &["src/app/core.ts".into()],
            &[],
            1,
        )
        .unwrap();
        let layers: Vec<_> = selected
            .items
            .iter()
            .filter(|v| v["policy_type"] == "layer")
            .collect();
        assert_eq!(layers.len(), 1);
        assert_eq!(layers[0]["value"]["name"], "Application");
        assert_eq!(layers[0]["applies_to"], json!(["app"]));
    }
}
