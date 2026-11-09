//! Independently modeled logical boundaries over the original source graph.
use archguard_core::{
    AmbiguousArchitectureNode, ArchguardError, ArchitectureDependency, ArchitecturePrivateAccess,
    Config, Finding, FindingKind, Graph, MappedArchitectureComponent, NodeKind, Ref,
    TargetArchitectureConfig, TargetArchitectureReport,
};
use globset::{Glob, GlobSet, GlobSetBuilder};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

struct ComponentPatterns {
    include: GlobSet,
    public: Option<GlobSet>,
}
fn compile_patterns(patterns: &[String]) -> Result<GlobSet, ArchguardError> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        if pattern.trim().is_empty() || pattern.contains('\0') {
            return Err(ArchguardError::Config(
                "architecture.model: patterns must not be empty or contain NUL".into(),
            ));
        }
        builder.add(
            Glob::new(pattern)
                .map_err(|e| ArchguardError::Config(format!("architecture.model: {e}")))?,
        );
    }
    builder
        .build()
        .map_err(|e| ArchguardError::Config(format!("architecture.model: {e}")))
}
fn compile(model: &TargetArchitectureConfig) -> Result<Vec<ComponentPatterns>, ArchguardError> {
    let invalid =
        |message: String| ArchguardError::Config(format!("architecture.model: {message}"));
    if model.name.trim().is_empty() || model.name.contains('\0') {
        return Err(invalid("name must be nonempty and contain no NUL".into()));
    }
    if model.components.is_empty() {
        return Err(invalid("components must not be empty".into()));
    }
    let mut names = BTreeSet::new();
    let mut patterns = vec![];
    for component in &model.components {
        if component.name.trim().is_empty()
            || component.name.contains('\0')
            || !names.insert(component.name.as_str())
        {
            return Err(invalid(
                "component names must be nonempty, unique and contain no NUL".into(),
            ));
        }
        if component.include.is_empty() {
            return Err(invalid(format!(
                "component {:?} needs include patterns",
                component.name
            )));
        }
        patterns.push(ComponentPatterns {
            include: compile_patterns(&component.include)?,
            public: component
                .public
                .as_deref()
                .map(compile_patterns)
                .transpose()?,
        });
    }
    let mut directions = BTreeSet::new();
    for direction in &model.allowed_dependencies {
        if !names.contains(direction.from.as_str()) || !names.contains(direction.to.as_str()) {
            return Err(invalid(format!(
                "unknown component in {} → {}",
                direction.from, direction.to
            )));
        }
        if direction.from == direction.to {
            return Err(invalid(
                "internal component dependencies are already allowed; self-directions are invalid"
                    .into(),
            ));
        }
        if !directions.insert((&direction.from, &direction.to)) {
            return Err(invalid(
                "allowed dependency directions must be unique".into(),
            ));
        }
    }
    Ok(patterns)
}
pub(crate) fn validate(config: &Config) -> Result<(), ArchguardError> {
    if let Some(model) = &config.architecture.model {
        compile(model)?;
    }
    Ok(())
}
fn identity(parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for value in parts {
        hash.update((value.len() as u64).to_be_bytes());
        hash.update(value.as_bytes());
    }
    format!("target:{:x}", hash.finalize())
}
fn finding(
    model: &TargetArchitectureConfig,
    kind: &str,
    keys: &[&str],
    mut nodes: Vec<String>,
    message: String,
    edge_count: u32,
    evidence: Vec<Ref>,
) -> Finding {
    nodes.sort();
    nodes.dedup();
    let mut id = vec![model.name.as_str(), kind];
    id.extend_from_slice(keys);
    Finding {
        kind: FindingKind::TargetArchitecture,
        id: identity(&id),
        severity: model.severity,
        message: Some(message),
        evidence,
        nodes,
        edge_count,
        cycles_sampled: 0,
        sampling: None,
        suggested_cuts: vec![],
    }
}

pub fn analyze_target_architecture(
    full: &Graph,
    config: &Config,
) -> Result<Option<TargetArchitectureReport>, ArchguardError> {
    evaluate(full, config).map(|result| result.map(|(report, _)| report))
}
#[tracing::instrument(skip_all)]
pub(crate) fn check_target_architecture(
    full: &Graph,
    config: &Config,
) -> Result<Vec<Finding>, ArchguardError> {
    Ok(evaluate(full, config)?
        .map(|(_, findings)| findings)
        .unwrap_or_default())
}
fn evaluate(
    full: &Graph,
    config: &Config,
) -> Result<Option<(TargetArchitectureReport, Vec<Finding>)>, ArchguardError> {
    let Some(model) = &config.architecture.model else {
        return Ok(None);
    };
    let patterns = compile(model)?;
    let mut components: Vec<_> = model
        .components
        .iter()
        .map(|c| MappedArchitectureComponent {
            name: c.name.clone(),
            include: c.include.clone(),
            node_count: 0,
            nodes: vec![],
            public: c.public.clone(),
            public_nodes: vec![],
            incoming_components: vec![],
            outgoing_components: vec![],
            max_incoming_components: c.max_incoming_components,
            max_outgoing_components: c.max_outgoing_components,
        })
        .collect();
    let mut assigned = BTreeMap::new();
    let mut unmapped = vec![];
    let mut ambiguous = vec![];
    let mut findings = vec![];
    let mut source_nodes = 0;
    let nodes: BTreeMap<_, _> = full.nodes.iter().map(|n| (n.id, n)).collect();
    for node in &full.nodes {
        if node.kind != NodeKind::File || node.name.ends_with("/*") {
            continue;
        }
        source_nodes += 1;
        let candidates: Vec<_> = patterns
            .iter()
            .enumerate()
            .filter_map(|(i, patterns)| patterns.include.is_match(&node.name).then_some(i))
            .collect();
        match candidates.as_slice() {
            [index] => {
                assigned.insert(node.id, *index);
                components[*index].nodes.push(node.name.clone());
                if patterns[*index]
                    .public
                    .as_ref()
                    .is_none_or(|p| p.is_match(&node.name))
                {
                    components[*index].public_nodes.push(node.name.clone());
                }
            }
            [] => {
                unmapped.push(node.name.clone());
                if model.require_assignment {
                    findings.push(finding(
                        model,
                        "unmapped",
                        &[&node.name],
                        vec![node.name.clone()],
                        format!(
                            "Target architecture {:?}: {} is not assigned to any component.",
                            model.name, node.name
                        ),
                        0,
                        vec![],
                    ));
                }
            }
            _ => {
                let mut names: Vec<_> = candidates
                    .iter()
                    .map(|index| components[*index].name.clone())
                    .collect();
                names.sort();
                findings.push(finding(model,"ambiguous",&[&node.name],vec![node.name.clone()],format!("Target architecture {:?}: {} matches multiple components ({}); no assignment was guessed.",model.name,node.name,names.join(", ")),0,vec![]));
                ambiguous.push(AmbiguousArchitectureNode {
                    node: node.name.clone(),
                    components: names,
                });
            }
        }
    }
    let allowed: BTreeSet<_> = model
        .allowed_dependencies
        .iter()
        .map(|d| (d.from.as_str(), d.to.as_str()))
        .collect();
    // Store the contributing source names alongside each logical edge for gate evidence.
    let mut pairs: BTreeMap<_, (ArchitectureDependency, BTreeSet<String>)> = BTreeMap::new();
    let mut evaluated_edges = 0;
    let mut unclassified_edges = 0;
    for edge in &full.edges {
        let (Some(&from), Some(&to)) = (assigned.get(&edge.from), assigned.get(&edge.to)) else {
            unclassified_edges += 1;
            continue;
        };
        evaluated_edges += 1;
        if from == to {
            continue;
        }
        let from_name = &components[from].name;
        let to_name = &components[to].name;
        let (dependency, members) = pairs
            .entry((from_name.clone(), to_name.clone()))
            .or_insert_with(|| {
                (
                    ArchitectureDependency {
                        from: from_name.clone(),
                        to: to_name.clone(),
                        allowed: allowed.contains(&(from_name.as_str(), to_name.as_str())),
                        source_edges: 0,
                        import_references: 0,
                        count_exact: true,
                        evidence_total: 0,
                        evidence: vec![],
                        private_accesses: vec![],
                    },
                    BTreeSet::new(),
                )
            });
        dependency.source_edges += 1;
        dependency.import_references = dependency.import_references.saturating_add(edge.weight());
        dependency.count_exact &= edge.count.is_some();
        dependency.evidence.extend(edge.refs.iter().cloned());
        members.insert(nodes[&edge.from].name.clone());
        members.insert(nodes[&edge.to].name.clone());
        if patterns[to]
            .public
            .as_ref()
            .is_some_and(|p| !p.is_match(&nodes[&edge.to].name))
        {
            let from_node = &nodes[&edge.from].name;
            let to_node = &nodes[&edge.to].name;
            if let Some(access) = dependency
                .private_accesses
                .iter_mut()
                .find(|a| a.from == *from_node && a.to == *to_node)
            {
                access.evidence.extend(edge.refs.iter().cloned());
            } else {
                dependency.private_accesses.push(ArchitecturePrivateAccess {
                    from: from_node.clone(),
                    to: to_node.clone(),
                    evidence: edge.refs.clone(),
                });
            }
        }
    }
    let mut dependencies = vec![];
    for (_, (mut dependency, members)) in pairs {
        dependency
            .evidence
            .sort_by(|a, b| (&a.file, a.line, &a.raw).cmp(&(&b.file, b.line, &b.raw)));
        dependency.evidence.dedup();
        dependency.evidence_total = dependency.evidence.len();
        dependency
            .private_accesses
            .sort_by(|a, b| (&a.from, &a.to).cmp(&(&b.from, &b.to)));
        for access in &mut dependency.private_accesses {
            access
                .evidence
                .sort_by(|a, b| (&a.file, a.line, &a.raw).cmp(&(&b.file, b.line, &b.raw)));
            access.evidence.dedup();
            findings.push(finding(model,"private_access",&[&dependency.from,&dependency.to,&access.from,&access.to],vec![access.from.clone(),access.to.clone()],format!("Target architecture {:?}: {} accesses private source {} in component {:?}; enter through its declared public interface.",model.name,access.from,access.to,dependency.to),1,access.evidence.iter().take(10).cloned().collect()));
        }
        if !dependency.allowed {
            findings.push(finding(
                model,
                "dependency",
                &[&dependency.from, &dependency.to],
                members.into_iter().collect(),
                format!(
                    "Target architecture {:?} does not allow {} → {} ({} source dependencies).",
                    model.name, dependency.from, dependency.to, dependency.source_edges
                ),
                u32::try_from(dependency.source_edges).unwrap_or(u32::MAX),
                dependency.evidence.iter().take(10).cloned().collect(),
            ));
        }
        dependencies.push(dependency);
    }
    let mut incoming = BTreeMap::<&str, Vec<&ArchitectureDependency>>::new();
    let mut outgoing = BTreeMap::<&str, Vec<&ArchitectureDependency>>::new();
    for dependency in &dependencies {
        incoming.entry(&dependency.to).or_default().push(dependency);
        outgoing
            .entry(&dependency.from)
            .or_default()
            .push(dependency);
    }
    let mut coupling_violations = 0;
    for component in &mut components {
        component.nodes.sort();
        component.public_nodes.sort();
        component.node_count = component.nodes.len();
        let incoming = incoming
            .get(component.name.as_str())
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let outgoing = outgoing
            .get(component.name.as_str())
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        component.incoming_components = incoming.iter().map(|d| d.from.clone()).collect();
        component.outgoing_components = outgoing.iter().map(|d| d.to.clone()).collect();
        for (direction, limit, neighbors, relevant) in [
            (
                "incoming",
                component.max_incoming_components,
                &component.incoming_components,
                incoming,
            ),
            (
                "outgoing",
                component.max_outgoing_components,
                &component.outgoing_components,
                outgoing,
            ),
        ] {
            let Some(limit) = limit.filter(|limit| neighbors.len() as u64 > u64::from(*limit))
            else {
                continue;
            };
            coupling_violations += 1;
            let mut evidence: Vec<_> = relevant
                .iter()
                .flat_map(|d| d.evidence.iter().cloned())
                .collect();
            evidence.sort_by(|a, b| (&a.file, a.line, &a.raw).cmp(&(&b.file, b.line, &b.raw)));
            evidence.dedup();
            evidence.truncate(10);
            findings.push(finding(model,"coupling_budget",&[&component.name,direction],component.nodes.clone(),format!("Target architecture {:?}: component {:?} has {} observed {direction} component dependencies, exceeding limit {limit} ({}).",model.name,component.name,neighbors.len(),neighbors.join(", ")),u32::try_from(relevant.iter().map(|d|d.source_edges).sum::<usize>()).unwrap_or(u32::MAX),evidence));
        }
    }
    components.sort_by(|a, b| a.name.cmp(&b.name));
    unmapped.sort();
    ambiguous.sort_by(|a, b| a.node.cmp(&b.node));
    let empty_components: Vec<_> = components
        .iter()
        .filter(|c| c.node_count == 0)
        .map(|c| c.name.clone())
        .collect();
    let mut allowed_dependencies = model.allowed_dependencies.clone();
    allowed_dependencies.sort();
    let status = if unmapped.is_empty()
        && ambiguous.is_empty()
        && empty_components.is_empty()
        && unclassified_edges == 0
    {
        "complete"
    } else {
        "partial"
    };
    Ok(Some((TargetArchitectureReport{
        schema_version:1,name:model.name.clone(),status:status.into(),severity:model.severity,require_assignment:model.require_assignment,
        source_nodes,mapped_nodes:assigned.len(),unmapped_nodes:unmapped.len(),ambiguous_nodes:ambiguous.len(),total_edges:full.edges.len(),evaluated_edges,unclassified_edges,
        violating_dependencies:dependencies.iter().filter(|d|!d.allowed).count(),violating_interfaces:dependencies.iter().map(|d|d.private_accesses.len()).sum(),coupling_violations,components,allowed_dependencies,dependencies,unmapped,ambiguous,empty_components,
        limitations:vec!["Coverage describes the configured source graph, not excluded files, unresolved imports or runtime calls.".into(),"Internal component dependencies are allowed. Cross-component directions must be explicitly declared.".into(),"Public interface patterns select assigned source files, not public symbols. Omitted public permits entry to any assigned file; public=[] closes external entry. Internal access remains allowed.".into(),"Coupling limits count distinct observed neighboring components, not imports or references. Partial extraction or mapping makes these counts lower bounds; a count within its budget does not establish complete conformance.".into(),"Namespace placeholders are not source files and remain unclassified; ambiguous mappings never create inferred component dependencies.".into(),"Logical dependencies retain all references stored in the input graph. Original extraction samples can be smaller than import_references.".into()],
    },findings)))
}
