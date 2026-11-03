//! Explainable architecture hypotheses over original observations, never policy.
use archguard_core::{
    CallAnalysis, Config, Graph, NodeKind, ServiceDependencyKind, ServiceTopology,
};
use model::{DETAIL_LIMIT, add, assessment, evidence};
pub use model::{PatternAssessment, PatternEvidence};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

#[path = "pattern_contracts.rs"]
mod contracts;
#[path = "pattern_model.rs"]
mod model;
#[path = "pattern_structure.rs"]
mod structure;
pub use contracts::pattern_inputs_fingerprint;
const PATTERN_DEFINITION: &str = "architecture-patterns/v3";
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchitecturePatternReport {
    pub schema_version: u32,
    pub definition: String,
    pub status: String,
    pub coverage: Value,
    pub assessments: Vec<PatternAssessment>,
    pub assessments_total: usize,
    pub diagnostics: Vec<String>,
    pub limitations: Vec<String>,
}
fn finish(p: &mut PatternAssessment) {
    p.nodes.sort();
    p.nodes.dedup();
    p.nodes_total = p.nodes.len();
    p.nodes.truncate(DETAIL_LIMIT);
    p.services.sort();
    p.services.dedup();
    p.services_total = p.services.len();
    p.services.truncate(DETAIL_LIMIT);
}
// Directory/package tokens are explicitly heuristic roles, never semantic proof.
fn role(name: &str) -> Option<u8> {
    name.rsplit_once('/')
        .map(|(p, _)| p)
        .unwrap_or("")
        .split(['/', '.'])
        .rev()
        .find_map(|part| match part.to_ascii_lowercase().as_str() {
            "web" | "ui" | "api" | "presentation" | "controller" | "controllers" => Some(0),
            "application" | "app" | "service" | "services" | "usecases" | "use_cases" => Some(1),
            "domain" | "core" => Some(2),
            "infrastructure" | "infra" | "persistence" | "storage" | "data" | "repository"
            | "repositories" | "adapter" | "adapters" => Some(3),
            _ => None,
        })
}
fn core(name: &str) -> bool {
    matches!(role(name), Some(1 | 2))
}

/// Reuses caller-supplied compiler and deployment observations. The graph must
/// be the original file graph, not a display cut or a simulated graph.
pub fn analyze_architecture_patterns(
    root: &Path,
    config: &Config,
    graph: &Graph,
    calls: &CallAnalysis,
    services: &ServiceTopology,
) -> ArchitecturePatternReport {
    let files: BTreeMap<_, _> = graph
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::File && !n.name.ends_with("/*"))
        .map(|n| (n.id, n.name.as_str()))
        .collect();
    let names: BTreeSet<_> = files.values().copied().collect();
    let edges: Vec<_> = graph
        .edges
        .iter()
        .filter(|e| files.contains_key(&e.from) && files.contains_key(&e.to))
        .collect();
    let source_graph = Graph {
        review_kpis: None,
        schema_version: 1,
        root: graph.root.clone(),
        nodes: graph
            .nodes
            .iter()
            .filter(|n| files.contains_key(&n.id))
            .cloned()
            .map(|mut n| {
                n.parent = None;
                n
            })
            .collect(),
        edges: edges.iter().map(|e| (*e).clone()).collect(),
        semantic: graph.semantic.clone(),
    };
    let cycles = archguard_analysis::find_sccs(&source_graph);
    let mut annotated_roles = BTreeMap::<&str, BTreeSet<u8>>::new();
    let mut annotation_evidence = vec![];
    if let Some(details) = &calls.details {
        for used in details.type_uses.iter().filter(|u| {
            u.role == "class_annotation" && matches!(u.resolution.as_str(), "project" | "external")
        }) {
            let observed = match used.target_label.as_str() {
                "org.springframework.web.bind.annotation.RestController"
                | "org.springframework.stereotype.Controller"
                | "Microsoft.AspNetCore.Mvc.ApiControllerAttribute" => Some(0),
                "org.springframework.stereotype.Service" => Some(1),
                "org.springframework.stereotype.Repository" => Some(3),
                _ => None,
            };
            let matching: Vec<_> = calls
                .classes
                .iter()
                .filter(|c| c.id == used.from_class && names.contains(c.graph_node.as_str()))
                .collect();
            if let (Some(role), [class]) = (observed, matching.as_slice()) {
                annotated_roles
                    .entry(class.graph_node.as_str())
                    .or_default()
                    .insert(role);
                annotation_evidence.push((class.graph_node.as_str(), used));
            }
        }
    }
    let observed_role = |name: &str| match annotated_roles.get(name) {
        Some(roles) if roles.len() == 1 => roles.iter().next().copied(),
        Some(_) => None,
        None => role(name),
    };
    let mut layered = assessment(
        "layered",
        ".",
        "dependency_structure",
        &[
            "Directory role names are heuristics; responsibilities and architectural intent need human review.",
            "Unresolved and external imports are not measured by this file graph; apparent absence of dependencies is not proof.",
        ],
    );
    for (node, used) in annotation_evidence {
        if annotated_roles
            .get(node)
            .is_some_and(|roles| roles.len() == 1)
        {
            add(
                &mut layered,
                evidence(
                    "framework_role",
                    Some(node),
                    None,
                    Some(&used.file),
                    Some(used.line),
                    &format!("Resolved framework declaration: {}", used.target_label),
                ),
                false,
            );
        }
    }
    let mut forward = 0;
    let mut backward = 0;
    let mut roles = BTreeSet::new();
    let mut support: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut incoming = BTreeMap::new();
    let mut outgoing = BTreeMap::new();
    let mut active = BTreeSet::new();
    for e in &edges {
        let (from, to) = (files[&e.from], files[&e.to]);
        *incoming.entry(to).or_insert(0usize) += 1;
        *outgoing.entry(from).or_insert(0usize) += 1;
        active.extend([from, to]);
        if let (Some(a), Some(b)) = (observed_role(from), observed_role(to)) {
            if a <= b {
                support.entry(from).or_default().push(to);
                support.entry(to).or_default().push(from);
            }
            if a == b {
                continue;
            }
            roles.extend([a, b]);
            layered.nodes.extend([from.into(), to.into()]);
            if a < b {
                forward += 1;
            } else {
                backward += 1;
            }
            if e.refs.is_empty() {
                add(
                    &mut layered,
                    evidence(
                        "dependency",
                        Some(from),
                        Some(to),
                        None,
                        None,
                        "Observed dependency; no stored source reference.",
                    ),
                    a > b,
                );
            }
            for r in &e.refs {
                add(
                    &mut layered,
                    evidence(
                        "dependency",
                        Some(from),
                        Some(to),
                        Some(&r.file),
                        Some(r.line),
                        &r.raw,
                    ),
                    a > b,
                );
            }
        }
    }
    let mut visited = BTreeSet::new();
    let mut connected_levels = 0;
    for start in support.keys() {
        let mut pending = vec![*start];
        let mut levels = BTreeSet::new();
        while let Some(node) = pending.pop() {
            if visited.insert(node) {
                levels.extend(observed_role(node));
                pending.extend(support[node].iter().copied());
            }
        }
        connected_levels = connected_levels.max(levels.len());
    }
    let role_nodes: BTreeSet<_> = layered.nodes.iter().map(String::as_str).collect();
    let role_cycles: Vec<_> = cycles
        .iter()
        .filter(|cycle| {
            cycle
                .node_ids
                .iter()
                .any(|id| role_nodes.contains(files[id]))
        })
        .collect();
    layered.metrics = json!({"forward_dependencies":forward,"backward_dependencies":backward,"role_levels":roles.len(),"connected_role_levels":connected_levels,"cyclic_source_groups":role_cycles.len(),"candidate_rule":"at least three role levels connected by forward or same-role dependencies"});
    if connected_levels >= 3 && forward >= 2 {
        layered.status = if backward > 0 || !role_cycles.is_empty() {
            "mixed"
        } else {
            "candidate"
        }
        .into();
    }
    for cycle in &role_cycles {
        add(
            &mut layered,
            evidence(
                "cycle",
                None,
                None,
                None,
                None,
                &format!(
                    "Observed cyclic group: {}",
                    cycle
                        .node_ids
                        .iter()
                        .map(|id| files[id])
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            ),
            true,
        );
    }
    let mut pipeline = assessment(
        "pipeline",
        ".",
        "dependency_structure",
        &[
            "A static dependency chain does not establish data flow, runtime order or a pipes-and-filters implementation.",
        ],
    );
    let chain = active.len() >= 3
        && edges.len() + 1 == active.len()
        && cycles.is_empty()
        && incoming.values().all(|n| *n <= 1)
        && outgoing.values().all(|n| *n <= 1);
    pipeline.status = if chain {
        "candidate"
    } else if active.len() >= 3 {
        "not_observed"
    } else {
        "insufficient_evidence"
    }
    .into();
    pipeline.metrics = json!({"active_files":active.len(),"dependencies":edges.len(),"isolated_files":files.len()-active.len(),"cyclic_groups":cycles.len(),"candidate_rule":"one acyclic chain of at least three active source files, with in/out degree at most one"});
    if chain {
        for e in &edges {
            let (from, to) = (files[&e.from], files[&e.to]);
            pipeline.nodes.extend([from.into(), to.into()]);
            let r = e.refs.first();
            add(
                &mut pipeline,
                evidence(
                    "dependency",
                    Some(from),
                    Some(to),
                    r.map(|r| r.file.as_str()),
                    r.map(|r| r.line),
                    "Observed chain link; dependency direction is not execution order.",
                ),
                false,
            );
        }
    }

    let mut modular = assessment(
        "modular_monolith",
        ".",
        "dependency_structure",
        &[
            "Packages are structural groups, not verified business modules.",
            "A common deployment unit and independent module responsibilities are not established by package cohesion.",
        ],
    );
    let cohesion = crate::cohesion::analyze_cohesion(graph, None);
    let internal: usize = cohesion
        .packages
        .iter()
        .map(|p| p.internal_dependencies)
        .sum();
    let external: usize = cohesion
        .packages
        .iter()
        .map(|p| p.outgoing_dependencies)
        .sum();
    let connected: Vec<_> = cohesion
        .packages
        .iter()
        .filter(|p| p.files >= 2 && p.groups.len() == 1 && p.internal_dependencies > 0)
        .collect();
    let cross_share =
        (internal + external > 0).then(|| external as f64 / (internal + external) as f64);
    let deployments: BTreeSet<_> = services
        .services
        .iter()
        .filter_map(|s| s.project.as_ref().or(s.build_context.as_ref()))
        .collect();
    modular.metrics = json!({"connected_packages":connected.len(),"internal_dependencies":internal,"cross_package_dependencies":external,"cross_package_share":cross_share,"candidate_rule":"at least two connected packages of at least two files; cross-package share <= 0.25"});
    if connected.len() >= 2 && cross_share.is_some_and(|s| s <= 0.25) {
        modular.status = if deployments.len() > 1 {
            "mixed"
        } else {
            "candidate"
        }
        .into();
    }
    for p in connected {
        modular
            .nodes
            .extend(p.groups.iter().flat_map(|g| g.files.clone()));
        add(
            &mut modular,
            evidence(
                "package",
                Some(&p.node),
                None,
                None,
                None,
                &format!(
                    "{} files, {} internal file dependencies, one connected group",
                    p.files, p.internal_dependencies
                ),
            ),
            false,
        );
    }
    if deployments.len() > 1 {
        add(
            &mut modular,
            evidence(
                "deployment",
                None,
                None,
                None,
                None,
                "Multiple configured build/project units; the selected repository is not proven to be one deployable monolith.",
            ),
            true,
        );
    }
    let mut assessments = vec![layered, pipeline, modular];
    structure::assess(&source_graph, &mut assessments);
    ports(
        calls,
        &names,
        &edges
            .iter()
            .map(|e| (files[&e.from], files[&e.to]))
            .collect::<Vec<_>>(),
        &mut assessments,
    );
    assessments.push(service_pattern(services));
    let (event, contract_status, diagnostics) = contracts::assess(root, config);
    assessments.push(event);
    let references =
        crate::reference_paths::source_reference_paths(root, config).unwrap_or_default();
    for p in &mut assessments {
        for item in p.evidence.iter_mut().chain(&mut p.counter_evidence) {
            if matches!(
                item.kind.as_str(),
                "dependency"
                    | "dependency_level"
                    | "port"
                    | "implementation"
                    | "port_call"
                    | "port_type"
                    | "core_to_adapter_call"
            ) && let Some(logical) = item.file.take()
            {
                item.file = references.get(&logical).cloned();
                if item.file.is_none() {
                    item.message = format!(
                        "{} [unmapped logical source: {}:{}]",
                        item.message,
                        logical,
                        item.line.unwrap_or(0)
                    );
                    item.line = None;
                }
            }
        }
        finish(p);
    }
    assessments.sort_by(|a, b| (&a.pattern, &a.scope).cmp(&(&b.pattern, &b.scope)));
    ArchitecturePatternReport {schema_version:1,definition:PATTERN_DEFINITION.into(),status:"assessed".into(),coverage:json!({"source_files":files.len(),"source_dependencies":edges.len(),"excluded_symbolic_or_unmapped_edges":graph.edges.len()-edges.len(),"import_resolution":"not_measured","semantic_status":calls.status,"semantic_analyzers":calls.analyzers,"service_status":if services.complete{"configured_scope_read"}else{"partial"},"asyncapi_status":contract_status}),assessments_total:assessments.len(),assessments,diagnostics,limitations:vec![
        "These are reproducible hypotheses, not proof of architecture intent, quality or conformance. Several patterns can coexist; no percentage probability is inferred.".into(),
        "Original file dependencies, compiler relationships, deployment configuration and messaging contracts remain distinct evidence types. Display depth and graph simulation do not change this assessment.".into(),
        "At most 256 nodes, services and evidence records per assessment are retained; exact totals expose omissions. Missing evidence is not evidence of absence.".into(),
        "No new policy, gate, build, network request or runtime trace is generated.".into(),
    ]}
}

fn ports(
    calls: &CallAnalysis,
    names: &BTreeSet<&str>,
    edges: &[(&str, &str)],
    assessments: &mut Vec<PatternAssessment>,
) {
    let mut indexed = BTreeMap::<&str, Vec<_>>::new();
    for c in &calls.classes {
        indexed.entry(c.id.as_str()).or_default().push(c);
    }
    let unique: BTreeMap<_, _> = indexed
        .iter()
        .filter_map(|(id, cs)| {
            (cs.len() == 1 && names.contains(cs[0].graph_node.as_str())).then_some((*id, cs[0]))
        })
        .collect();
    let mut implementations = BTreeMap::<&str, Vec<_>>::new();
    for c in unique.values() {
        for parent in &c.supertypes {
            implementations.entry(parent).or_default().push(*c);
        }
    }
    struct Use {
        from_class: String,
        file: String,
        line: u32,
        code: String,
        kind: &'static str,
    }
    let mut consumers = BTreeMap::<&str, Vec<Use>>::new();
    for c in &calls.calls {
        if c.resolution == "project"
            && let Some(target) = &c.to_class
        {
            consumers.entry(target).or_default().push(Use {
                from_class: c.from_class.clone(),
                file: c.file.clone(),
                line: c.line,
                code: c.code.clone(),
                kind: "port_call",
            });
        }
    }
    if let Some(details) = &calls.details {
        for used in &details.type_uses {
            if used.resolution == "project"
                && matches!(used.role.as_str(), "field" | "parameter" | "return")
                && let Some(target) = &used.to_class
            {
                consumers.entry(target).or_default().push(Use {
                    from_class: used.from_class.clone(),
                    file: used.file.clone(),
                    line: used.line,
                    code: format!("{} type: {}", used.role, used.code),
                    kind: "port_type",
                });
            }
        }
    }
    let mut count = 0;
    for port in unique
        .values()
        .filter(|c| c.kind.eq_ignore_ascii_case("interface"))
    {
        let mut outer: Vec<_> = implementations
            .get(port.id.as_str())
            .into_iter()
            .flatten()
            .filter(|c| {
                !c.kind.eq_ignore_ascii_case("interface")
                    && c.is_abstract != Some(true)
                    && c.graph_node.rsplit_once('/').map(|(p, _)| p)
                        != port.graph_node.rsplit_once('/').map(|(p, _)| p)
            })
            .collect();
        let mut uses: Vec<_> = consumers
            .get(port.id.as_str())
            .into_iter()
            .flatten()
            .filter(|c| {
                unique.get(c.from_class.as_str()).is_some_and(|from| {
                    from.id != port.id
                        && !outer
                            .iter()
                            .any(|adapter| adapter.graph_node == from.graph_node)
                })
            })
            .collect();
        if outer.is_empty() || uses.is_empty() {
            continue;
        }
        let typed = core(&port.graph_node)
            && outer
                .iter()
                .any(|c| matches!(role(&c.graph_node), Some(0 | 3)))
            && uses
                .iter()
                .any(|c| core(&unique[c.from_class.as_str()].graph_node));
        if typed {
            outer.retain(|c| matches!(role(&c.graph_node), Some(0 | 3)));
            uses.retain(|c| core(&unique[c.from_class.as_str()].graph_node));
            count += 1;
        }
        let mut p = assessment(
            if typed {
                "ports_and_adapters"
            } else {
                "dependency_inversion"
            },
            &port.id,
            "compiler_relationships",
            &[
                "An interface caller and an implementation in another package establish a structural indirection, not architectural intent or a complete ports-and-adapters architecture.",
                "Compiler calls, declaration types and inheritance do not establish dynamic dispatch or runtime wiring. Hexagonal, clean and onion styles cannot be distinguished from this evidence alone.",
            ],
        );
        if typed {
            p.missing_evidence.push("Core and outer roles use directory/package vocabulary; domain meaning remains unverified.".into());
        }
        p.status = "candidate".into();
        p.nodes.push(port.graph_node.clone());
        add(
            &mut p,
            evidence(
                "port",
                Some(&port.graph_node),
                None,
                Some(&port.file),
                Some(port.line),
                &format!("Compiler-declared interface {}", port.id),
            ),
            false,
        );
        let outer_names: BTreeSet<_> = outer.iter().map(|c| c.graph_node.as_str()).collect();
        for adapter in &outer {
            p.nodes.push(adapter.graph_node.clone());
            let site = adapter
                .supertype_sites
                .as_ref()
                .and_then(|s| s.iter().find(|s| s.target == port.id));
            add(
                &mut p,
                evidence(
                    "implementation",
                    Some(&adapter.graph_node),
                    Some(&port.graph_node),
                    Some(site.map(|s| s.file.as_str()).unwrap_or(&adapter.file)),
                    Some(site.map(|s| s.line).unwrap_or(adapter.line)),
                    &format!("{} declares implementation of {}", adapter.id, port.id),
                ),
                false,
            );
        }
        let consumer_names: BTreeSet<_> = uses
            .iter()
            .map(|c| unique[c.from_class.as_str()].graph_node.as_str())
            .collect();
        let consumers_total = consumer_names.len();
        for call in uses {
            let caller = unique[call.from_class.as_str()];
            p.nodes.push(caller.graph_node.clone());
            add(
                &mut p,
                evidence(
                    call.kind,
                    Some(&caller.graph_node),
                    Some(&port.graph_node),
                    Some(&call.file),
                    Some(call.line),
                    &call.code,
                ),
                false,
            );
        }
        for (from, to) in edges {
            if (if typed {
                core(from)
            } else {
                consumer_names.contains(from)
            }) && outer_names.contains(to)
            {
                add(
                    &mut p,
                    evidence(
                        "core_to_adapter",
                        Some(from),
                        Some(to),
                        None,
                        None,
                        "An abstraction consumer also depends directly on its concrete implementation.",
                    ),
                    true,
                );
            }
        }
        for call in &calls.calls {
            if call.resolution == "project"
                && let (Some(from), Some(to)) = (
                    unique.get(call.from_class.as_str()),
                    call.to_class.as_deref().and_then(|t| unique.get(t)),
                )
                && (if typed {
                    core(&from.graph_node)
                } else {
                    consumer_names.contains(from.graph_node.as_str())
                })
                && outer_names.contains(to.graph_node.as_str())
            {
                add(
                    &mut p,
                    evidence(
                        "core_to_adapter_call",
                        Some(&from.graph_node),
                        Some(&to.graph_node),
                        Some(&call.file),
                        Some(call.line),
                        &call.code,
                    ),
                    true,
                );
            }
        }
        if p.counter_evidence_total > 0 {
            p.status = "mixed".into();
        }
        if calls.status != "ready" {
            p.missing_evidence.push(format!(
                "Compiler analysis is {}; missing providers or references limit this candidate.",
                calls.status
            ));
        }
        p.metrics = json!({"implementations":outer.len(),"consumers":consumers_total,"compiler_status":calls.status,"role_names_required":typed});
        assessments.push(p);
    }
    if count == 0 {
        assessments.push(assessment("ports_and_adapters",".","compiler_relationships",&["Need a uniquely mapped compiler-declared core interface, an outer implementation and a core caller of that interface. Names or interface counts alone do not establish ports and adapters."]));
    }
}
fn service_pattern(topology: &ServiceTopology) -> PatternAssessment {
    let mut p = assessment(
        "microservices",
        ".",
        "deployment_configuration",
        &[
            "Independent deployment, business boundaries, data ownership and runtime communication are unverified. Configured HTTP endpoints alone do not establish microservices.",
        ],
    );
    let by_id: BTreeMap<_, _> = topology
        .services
        .iter()
        .map(|s| (s.id.as_str(), s))
        .collect();
    let mut units = BTreeSet::new();
    let mut links = 0;
    let mut cross_unit_links = 0;
    for d in &topology.dependencies {
        if d.kind != ServiceDependencyKind::ConfiguredHttp {
            continue;
        }
        let (Some(a), Some(b)) = (by_id.get(d.from.as_str()), by_id.get(d.to.as_str())) else {
            continue;
        };
        p.services.extend([a.id.clone(), b.id.clone()]);
        links += 1;
        if let (Some(from), Some(to)) = (
            a.project.as_ref().or(a.build_context.as_ref()),
            b.project.as_ref().or(b.build_context.as_ref()),
        ) && from != to
        {
            cross_unit_links += 1;
        }
        for s in [a, b] {
            if let Some(unit) = s.project.as_ref().or(s.build_context.as_ref()) {
                units.insert(unit.clone());
            }
        }
        for r in &d.evidence {
            let mut item = evidence(
                "configured_http",
                Some(&d.from),
                Some(&d.to),
                Some(&r.file),
                r.line,
                &r.description,
            );
            item.pointer = Some(r.pointer.clone());
            add(&mut p, item, false);
        }
    }
    let cycles: Vec<_> = topology
        .cycles
        .iter()
        .filter(|c| c.kind == ServiceDependencyKind::ConfiguredHttp)
        .collect();
    p.metrics = json!({"configured_http_dependencies":links,"cross_unit_http_dependencies":cross_unit_links,"distinct_build_or_project_units":units.len(),"http_cycles":cycles.len(),"candidate_rule":"configured HTTP between at least two distinct build/project units"});
    if cross_unit_links > 0 {
        p.status = if cycles.is_empty() {
            "candidate"
        } else {
            "mixed"
        }
        .into();
    }
    for c in cycles {
        add(
            &mut p,
            evidence(
                "configured_http_cycle",
                None,
                None,
                None,
                None,
                &format!(
                    "Configured cyclic service group: {}. Review coupling; a cycle alone does not disprove microservices.",
                    c.nodes.join(", ")
                ),
            ),
            true,
        );
    }
    if !topology.complete {
        p.missing_evidence.push(
            "Service configuration was only partially read; inspect topology diagnostics.".into(),
        );
    }
    p
}

pub fn architecture_patterns(
    root: &Path,
    config: &Config,
    graph: &Graph,
) -> Result<ArchitecturePatternReport, archguard_core::ArchguardError> {
    let calls = crate::semantic_calls::analyze_calls(root, config);
    let services = crate::services::analyze_services(root)?;
    Ok(analyze_architecture_patterns(
        root, config, graph, &calls, &services,
    ))
}
