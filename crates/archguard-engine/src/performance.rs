//! Source-backed structural cost indicators; never runtime measurements.
use archguard_core::{
    AbstractionMetric, ArchguardError, CallAnalysis, Config, ContractCandidate, CyclicBuildGroup,
    EntryClosure, Graph, NodeId, NodeKind, PerformanceReport, Ref, ReviewSources,
    SourceCostFinding, ZoneStats,
};
use archguard_extract::{Lang, TargetSpec, extractor_for, strip_root};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::Path;

#[tracing::instrument(skip_all)]
#[allow(clippy::too_many_arguments)]
pub fn analyze_performance(
    root: &Path,
    config: &Config,
    full: &Graph,
    view: &Graph,
    sources: &ReviewSources,
    calls: &CallAnalysis,
    entry_point_ids: &[NodeId],
) -> Result<PerformanceReport, ArchguardError> {
    let mut report = PerformanceReport {
        schema_version: 1,
        ..PerformanceReport::default()
    };
    let entry_ids: BTreeSet<_> = entry_point_ids.iter().copied().collect();
    let mut by_name = BTreeMap::new();
    for node in &full.nodes {
        by_name.insert(node.name.as_str(), node);
    }
    for file in crate::pipeline::walk_files(root, config)? {
        if !matches!(file.lang, Lang::Py | Lang::Ts | Lang::Js) {
            continue;
        }
        let Some(extractor) = extractor_for(file.lang) else {
            continue;
        };
        let content = match fs::read_to_string(&file.path) {
            Ok(content) => content,
            Err(error) => {
                report
                    .diagnostics
                    .push(format!("{}: {error}", file.rel_path));
                continue;
            }
        };
        let stripped = strip_root(&file.rel_path, &config.project.source_roots);
        let extraction = extractor.extract(&stripped, &content);
        let name = crate::pipeline::file_node_name(&stripped, &extraction.declarations);
        let Some(node) = by_name.get(name.as_str()) else {
            continue;
        };
        let excluded = entry_ids.contains(&node.id)
            || full
                .ancestors_of(node.id)
                .iter()
                .any(|ancestor| entry_ids.contains(ancestor));
        let raw_lines: Vec<_> = content.lines().collect();
        let code_lines = if file.lang == Lang::Py {
            raw_lines.iter().map(|line| line.to_string()).collect()
        } else {
            javascript_code_lines(&content)
        };
        let substantive = code_lines
            .iter()
            .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
            .count() as u32;
        let mut reexports = BTreeMap::new();
        let mut heavy = BTreeSet::new();
        for import in extraction.imports {
            let Some(code) = code_lines.get(import.line.saturating_sub(1) as usize) else {
                continue;
            };
            let trimmed = code.trim();
            let raw = raw_lines
                .get(import.line.saturating_sub(1) as usize)
                .copied()
                .unwrap_or("");
            let eager = match file.lang {
                Lang::Py => {
                    !raw.starts_with(char::is_whitespace)
                        && (trimmed.starts_with("import ") || trimmed.starts_with("from "))
                }
                _ => {
                    trimmed.starts_with("import ") && !type_only(trimmed, "import")
                        || trimmed.starts_with("export ")
                            && !type_only(trimmed, "export")
                            && trimmed.contains(" from ")
                }
            };
            if matches!(file.lang, Lang::Ts | Lang::Js)
                && trimmed.starts_with("export ")
                && !type_only(trimmed, "export")
                && trimmed.contains(" from ")
            {
                reexports
                    .entry(import.line)
                    .or_insert_with(|| raw.to_string());
            }
            if excluded || !eager {
                continue;
            }
            let spec = match import.target {
                TargetSpec::Path(spec) => spec,
                TargetSpec::Symbol { path, stem } => {
                    stem.map_or(path.clone(), |stem| format!("{path}/{stem}"))
                }
                // A name used in code is no import statement and loads nothing.
                TargetSpec::Qualified(_) => continue,
            };
            for prefix in &config.performance.heavy_imports {
                let normalized = if file.lang == Lang::Py {
                    prefix.replace('.', "/")
                } else {
                    prefix.clone()
                };
                if (spec == normalized || spec.starts_with(&(normalized.clone() + "/")))
                    && heavy.insert((import.line, prefix.clone()))
                {
                    report.source_findings.push(SourceCostFinding {kind:"heavy_import".into(),node_id:node.id,name:node.name.clone(),file:file.rel_path.clone(),line:import.line,raw:raw.to_string(),library:Some(prefix.clone()),reexports:0,code_lines:0,message:"Eager import of a configured cost-review dependency outside an entry point. Measure import cost before choosing lazy loading.".into()});
                }
            }
        }
        if reexports.len() >= config.performance.min_barrel_exports as usize
            && substantive > 0
            && reexports.len() as f64 / f64::from(substantive)
                >= config.performance.min_barrel_ratio
        {
            let (&line, raw) = reexports.first_key_value().expect("nonempty re-exports");
            report.source_findings.push(SourceCostFinding {kind:"barrel".into(),node_id:node.id,name:node.name.clone(),file:file.rel_path.clone(),line,raw:raw.clone(),library:None,reexports:reexports.len() as u32,code_lines:substantive,message:"Re-export-dominated module. Review transitive coupling and entry imports; tree shaking can avoid loading all exports.".into()});
        }
    }
    report.source_findings.sort_by(|a, b| {
        (&a.file, a.line, &a.kind, &a.library).cmp(&(&b.file, b.line, &b.kind, &b.library))
    });
    let members = source_members(full);
    let line_evidence: BTreeMap<_, _> = sources
        .files
        .iter()
        .map(|source| (source.node_id, source.lines))
        .collect();
    let full_sccs = archguard_analysis::find_sccs(full);
    let mut component = BTreeMap::new();
    let mut cyclic = BTreeSet::new();
    let mut components = 0;
    for scc in &full_sccs {
        for id in &scc.node_ids {
            component.insert(*id, components);
        }
        cyclic.insert(components);
        components += 1;
    }
    for node in &full.nodes {
        if let std::collections::btree_map::Entry::Vacant(entry) = component.entry(node.id) {
            entry.insert(components);
            components += 1;
        }
    }
    let mut dag = vec![BTreeSet::new(); components];
    let mut adj: BTreeMap<NodeId, BTreeSet<NodeId>> = full
        .nodes
        .iter()
        .map(|node| (node.id, BTreeSet::new()))
        .collect();
    for edge in &full.edges {
        if let (Some(&from), Some(&to)) = (component.get(&edge.from), component.get(&edge.to)) {
            adj.entry(edge.from).or_default().insert(edge.to);
            if from != to {
                dag[from].insert(to);
            }
        }
    }
    let longest = longest_dag_paths(&dag);
    for id in entry_ids {
        let Some(node) = full.nodes.iter().find(|node| node.id == id) else {
            continue;
        };
        let seeds = members.get(&id).cloned().unwrap_or_default();
        let mut reached = seeds.clone();
        let mut pending: Vec<_> = seeds.iter().copied().collect();
        while let Some(id) = pending.pop() {
            for next in adj.get(&id).into_iter().flatten() {
                if reached.insert(*next) {
                    pending.push(*next);
                }
            }
        }
        let files: BTreeSet<_> = reached
            .iter()
            .filter(|id| {
                members
                    .get(id)
                    .is_some_and(|children| children.len() == 1 && children.contains(id))
            })
            .copied()
            .collect();
        let reached_components: BTreeSet<_> = reached
            .iter()
            .filter_map(|id| component.get(id))
            .copied()
            .collect();
        report.entries.push(EntryClosure {
            node_id: id,
            name: node.name.clone(),
            reachable_files: files.len() as u32,
            lines: sum_lines(&files, &line_evidence),
            longest_component_path: seeds
                .iter()
                .filter_map(|id| component.get(id))
                .map(|id| longest[*id])
                .max()
                .unwrap_or(0),
            cyclic_components: reached_components.intersection(&cyclic).count() as u32,
        });
    }
    report.entries.sort_by(|a, b| {
        b.reachable_files
            .cmp(&a.reachable_files)
            .then_with(|| a.name.cmp(&b.name))
    });
    let sccs = archguard_analysis::find_sccs(view);
    for scc in &sccs {
        let nodes: Vec<_> = scc
            .node_ids
            .iter()
            .filter_map(|id| view.nodes.iter().find(|node| node.id == *id))
            .collect();
        let files: BTreeSet<_> = nodes
            .iter()
            .filter_map(|node| by_name.get(node.name.as_str()))
            .flat_map(|node| members.get(&node.id).into_iter().flatten().copied())
            .collect();
        report.cyclic_groups.push(CyclicBuildGroup {
            nodes: nodes.iter().map(|node| node.name.clone()).collect(),
            source_files: files.len() as u32,
            lines: sum_lines(&files, &line_evidence),
        });
    }
    report.cyclic_groups.sort_by(|a, b| {
        b.nodes
            .len()
            .cmp(&a.nodes.len())
            .then_with(|| a.nodes.cmp(&b.nodes))
    });
    for node in &view.nodes {
        report.abstractions.push(abstraction_metric(
            full,
            view,
            calls,
            &members,
            "node",
            node.name.clone(),
            &BTreeSet::from([node.id]),
        ));
    }
    for scc in &sccs {
        report.abstractions.push(abstraction_metric(
            full,
            view,
            calls,
            &members,
            "scc",
            scc.id.clone(),
            &scc.node_ids.iter().copied().collect(),
        ));
    }
    report
        .abstractions
        .sort_by(|a, b| (&a.scope, &a.name).cmp(&(&b.scope, &b.name)));
    report.contract_candidates = contract_candidates(calls);
    report.contract_candidates_truncated = report.contract_candidates.len() > 100;
    report.contract_candidates.truncate(100);
    report.limitations=vec![
        "Import closure, SCC size and source lines describe structural exposure, not measured startup, bundle size, compilation time or guaranteed build parallelism.".into(),
        "Heavy-import and barrel hints use line-scanner evidence; multiline syntax and computed imports may be missed. Dynamic imports and type-only statements are excluded; bundler tree shaking may avoid loading re-exports.".into(),
        "Entry closures count project source files only; unresolved or external imports and runtime-loaded code are outside this graph. Longest paths count SCC condensation components, not individual files through a cycle.".into(),
        "Main Sequence Distance is abs(A + I - 1), with A from compiler-declared abstract Java/C# types and I from current-scope import boundary coupling. Partial semantic coverage for a scope's languages yields no A or D; it is not a DIP/LSP proof.".into(),
        "Contract candidates identify compiler-declared cross-package calls to concrete types. Interfaces require behavioral and API review; substitution correctness cannot be established from this report.".into(),
    ];
    Ok(report)
}

fn source_members(full: &Graph) -> BTreeMap<NodeId, BTreeSet<NodeId>> {
    let mut result: BTreeMap<NodeId, BTreeSet<NodeId>> = BTreeMap::new();
    for file in full
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::File && !node.name.ends_with("/*"))
    {
        result.entry(file.id).or_default().insert(file.id);
        for parent in full.ancestors_of(file.id) {
            result.entry(parent).or_default().insert(file.id);
        }
    }
    result
}
fn sum_lines(files: &BTreeSet<NodeId>, evidence: &BTreeMap<NodeId, Option<u64>>) -> Option<u64> {
    files.iter().try_fold(0u64, |sum, id| {
        sum.checked_add(evidence.get(id).copied().flatten()?)
    })
}
fn longest_dag_paths(dag: &[BTreeSet<usize>]) -> Vec<u32> {
    let mut indegree = vec![0; dag.len()];
    for next in dag {
        for id in next {
            indegree[*id] += 1;
        }
    }
    let mut queue: VecDeque<_> = indegree
        .iter()
        .enumerate()
        .filter_map(|(id, count)| (*count == 0).then_some(id))
        .collect();
    let mut order = Vec::new();
    while let Some(id) = queue.pop_front() {
        order.push(id);
        for next in &dag[id] {
            indegree[*next] -= 1;
            if indegree[*next] == 0 {
                queue.push_back(*next);
            }
        }
    }
    let mut lengths = vec![1; dag.len()];
    for id in order.into_iter().rev() {
        lengths[id] = 1 + dag[id].iter().map(|next| lengths[*next]).max().unwrap_or(0);
    }
    lengths
}
fn type_only(code: &str, keyword: &str) -> bool {
    let body = code.strip_prefix(keyword).unwrap_or("").trim_start();
    if body.starts_with("type ") {
        return true;
    }
    if let Some(body) = body
        .strip_prefix('{')
        .and_then(|body| body.split('}').next())
    {
        let fields: Vec<_> = body
            .split(',')
            .map(str::trim)
            .filter(|field| !field.is_empty())
            .collect();
        return !fields.is_empty() && fields.iter().all(|field| field.starts_with("type "));
    }
    false
}
// Keep structural tokens but blank comments/string/template bodies. This
// prevents a line beginning with import inside a multiline template from
// being mistaken for an eager declaration by the existing linescanner.
fn javascript_code_lines(content: &str) -> Vec<String> {
    let mut block = false;
    let mut quote = None;
    let mut escaped = false;
    content
        .lines()
        .map(|line| {
            let chars: Vec<_> = line.chars().collect();
            let mut code = String::new();
            let mut i = 0;
            while i < chars.len() {
                let c = chars[i];
                let next = chars.get(i + 1).copied();
                if block {
                    if c == '*' && next == Some('/') {
                        block = false;
                        i += 1;
                    }
                    code.push(' ');
                } else if let Some(end) = quote {
                    if escaped {
                        escaped = false;
                    } else if c == '\\' {
                        escaped = true;
                    } else if c == end {
                        quote = None;
                    }
                    code.push(' ');
                } else if c == '/' && next == Some('/') {
                    break;
                } else if c == '/' && next == Some('*') {
                    block = true;
                    i += 1;
                    code.push(' ');
                } else if matches!(c, '\'' | '"' | '`') {
                    quote = Some(c);
                    code.push(' ');
                } else {
                    code.push(c);
                }
                i += 1;
            }
            if quote != Some('`') && !escaped {
                quote = None;
            }
            escaped = false;
            code
        })
        .collect()
}
/// The language of a file node whose abstractness a provider can supply.
fn abstractness_language(node: &archguard_core::Node) -> Option<&'static str> {
    match node.lang.as_deref() {
        Some("java") => Some("java"),
        Some("cs" | "csharp") => Some("csharp"),
        Some("rs") => Some("rust"),
        _ => None,
    }
}
/// Whether the providers enumerated every source file of each scope language.
fn provider_coverage(
    full: &Graph,
    calls: &CallAnalysis,
    scope_languages: &BTreeSet<Option<&str>>,
) -> bool {
    let source_files = |language: &str| {
        full.nodes
            .iter()
            .filter(|node| {
                node.kind == NodeKind::File
                    && !node.name.ends_with("/*")
                    && abstractness_language(node) == Some(language)
            })
            .count()
    };
    if calls.analyzers.is_empty() {
        // Old reports only described Java. Their global status/file count is
        // still required; absent metadata cannot silently opt C# into coverage.
        return calls.status == "ready"
            && *scope_languages == BTreeSet::from([Some("java")])
            && source_files("java") == calls.files;
    }
    scope_languages.iter().all(|needed| {
        let Some(needed) = needed else {
            return false;
        };
        let providers: Vec<_> = calls
            .analyzers
            .iter()
            .filter(|analyzer| analyzer.language == *needed)
            .collect();
        // Rust declarations come from syntax; its provider stays `partial`
        // for calls, and files whose types syntax cannot enumerate carry an
        // `unknown` class without abstractness instead.
        let enumerated =
            |status: &str| status == "ready" || (*needed == "rust" && status == "partial");
        !providers.is_empty()
            && providers
                .iter()
                .all(|provider| enumerated(&provider.status))
            && providers
                .iter()
                .try_fold(0usize, |sum, provider| sum.checked_add(provider.files))
                == Some(source_files(needed))
    })
}
fn abstraction_metric(
    full: &Graph,
    view: &Graph,
    calls: &CallAnalysis,
    members: &BTreeMap<NodeId, BTreeSet<NodeId>>,
    scope: &str,
    name: String,
    ids: &BTreeSet<NodeId>,
) -> AbstractionMetric {
    let names: Vec<_> = view
        .nodes
        .iter()
        .filter(|node| ids.contains(&node.id))
        .map(|node| node.name.clone())
        .collect();
    let files: BTreeSet<_> = full
        .nodes
        .iter()
        .filter(|node| names.contains(&node.name))
        .flat_map(|node| members.get(&node.id).into_iter().flatten().copied())
        .collect();
    let file_names: BTreeSet<_> = full
        .nodes
        .iter()
        .filter(|node| files.contains(&node.id))
        .map(|node| node.name.as_str())
        .collect();
    // Function groups and Rust impl blocks are no type declarations.
    let types: Vec<_> = calls
        .classes
        .iter()
        .filter(|class| {
            file_names.contains(class.graph_node.as_str())
                && !matches!(class.kind.as_str(), "module" | "impl")
        })
        .collect();
    let incoming: BTreeSet<_> = view
        .edges
        .iter()
        .filter(|edge| {
            !ids.contains(&edge.from)
                && ids.contains(&edge.to)
                && view.nodes.iter().any(|node| node.id == edge.from)
        })
        .map(|edge| edge.from)
        .collect();
    let outgoing: BTreeSet<_> = view
        .edges
        .iter()
        .filter(|edge| {
            ids.contains(&edge.from)
                && !ids.contains(&edge.to)
                && view.nodes.iter().any(|node| node.id == edge.to)
        })
        .map(|edge| edge.to)
        .collect();
    let instability = if incoming.len() + outgoing.len() == 0 {
        0.0
    } else {
        outgoing.len() as f64 / (incoming.len() + outgoing.len()) as f64
    };
    let scope_languages: BTreeSet<_> = full
        .nodes
        .iter()
        .filter(|node| files.contains(&node.id))
        .map(abstractness_language)
        .collect();
    let complete = provider_coverage(full, calls, &scope_languages);
    let reason = if files.is_empty() || types.is_empty() {
        Some("No compiler-backed types in this scope.".into())
    } else if scope_languages.contains(&None) {
        Some("This scope includes source languages without supported abstractness analysis.".into())
    } else if !complete {
        Some("Complete compiler coverage is unavailable for this scope's source languages.".into())
    } else if types.iter().any(|class| class.is_abstract.is_none()) {
        Some("The call report has no complete abstractness metadata.".into())
    } else {
        None
    };
    let abstract_types = types
        .iter()
        .filter(|class| class.is_abstract == Some(true))
        .count() as u32;
    let abstractness = reason
        .is_none()
        .then(|| f64::from(abstract_types) / types.len() as f64);
    AbstractionMetric {
        scope: scope.into(),
        name,
        nodes: names,
        types: types.len() as u32,
        abstract_types,
        fan_in: incoming.len() as u32,
        fan_out: outgoing.len() as u32,
        instability,
        abstractness,
        distance: abstractness.map(|a| (a + instability - 1.0).abs()),
        coverage: if reason.is_none() {
            "complete"
        } else {
            "unavailable"
        }
        .into(),
        reason,
    }
}
/// The review chain behind the structural performance report: review
/// sources, configured entry points, call analysis (runs compiler helpers
/// such as javac, hence expensive) and [`analyze_performance`].
pub fn review_performance(
    root: &Path,
    config: &Config,
    full: &Graph,
    view: &Graph,
) -> Result<PerformanceReport, ArchguardError> {
    let sources = crate::review_sources::collect_review_sources(
        root,
        config,
        full,
        &config.review.git_since,
    )?;
    let ids =
        archguard_analysis::match_review_entry_points(full, &sources, &config.review.entry_points)?;
    let calls = crate::semantic_calls::analyze_calls(root, config);
    analyze_performance(root, config, full, view, &sources, &calls, &ids)
}
/// Opt-in zone measurement for consumers whose pipeline KPI block leaves
/// `zones` unmeasured: [`review_performance`] then [`review_zones`].
pub fn measure_review_zones(
    root: &Path,
    config: &Config,
    full: &Graph,
    view: &Graph,
) -> Result<Option<ZoneStats>, ArchguardError> {
    Ok(review_zones(
        view,
        &review_performance(root, config, full, view)?,
    ))
}
/// Martin A/I zone counts for the review KPIs from the node-scope
/// abstraction metrics of `report`. SCC metrics are skipped because their
/// members already count as nodes. `None` while no boundary is measurable.
pub fn review_zones(view: &Graph, report: &PerformanceReport) -> Option<ZoneStats> {
    let nodes: Vec<_> = report
        .abstractions
        .iter()
        .filter(|metric| metric.scope == "node")
        .cloned()
        .collect();
    nodes
        .iter()
        .any(|metric| metric.coverage == "complete")
        .then(|| archguard_analysis::zone_stats(view, &nodes))
}
fn contract_candidates(calls: &CallAnalysis) -> Vec<ContractCandidate> {
    let classes: BTreeMap<_, _> = calls
        .classes
        .iter()
        .map(|class| (class.id.as_str(), class))
        .collect();
    let mut grouped: BTreeMap<(String, String), ContractCandidate> = BTreeMap::new();
    for call in &calls.calls {
        if call.resolution != "project" {
            continue;
        }
        let (Some(from), Some(to)) = (
            classes.get(call.from_class.as_str()),
            call.to_class.as_deref().and_then(|id| classes.get(id)),
        ) else {
            continue;
        };
        if from.package == to.package || to.is_abstract != Some(false) {
            continue;
        }
        let mut inherited = BTreeSet::new();
        let mut pending = to.supertypes.clone();
        let mut interfaces = BTreeSet::new();
        while let Some(id) = pending.pop() {
            if !inherited.insert(id.clone()) {
                continue;
            }
            if let Some(parent) = classes.get(id.as_str()) {
                if parent.kind == "interface" {
                    interfaces.insert(id);
                }
                pending.extend(parent.supertypes.iter().cloned());
            }
        }
        let interfaces = interfaces.into_iter().collect();
        let row=grouped.entry((from.id.clone(),to.id.clone())).or_insert_with(||ContractCandidate {from_class:from.id.clone(),to_class:to.id.clone(),from_node:from.graph_node.clone(),to_node:to.graph_node.clone(),interfaces,call_sites:0,evidence:vec![],message:"Cross-package call to a concrete type. Review whether the caller can depend on a narrow existing interface or an adapter; method compatibility and behavior still need verification.".into()});
        if matches!(call.dispatch.as_str(), "constructor" | "static") {
            row.message="Cross-package construction or static call to a concrete type. Review an adapter or factory boundary; an interface alone cannot replace construction/static dispatch.".into();
        }
        row.call_sites += 1;
        if row.evidence.len() < 10 {
            row.evidence.push(Ref {
                file: call.file.clone(),
                line: call.line,
                raw: call.code.clone(),
            });
        }
    }
    let mut result: Vec<_> = grouped.into_values().collect();
    result.sort_by(|a, b| {
        b.call_sites
            .cmp(&a.call_sites)
            .then_with(|| (&a.from_class, &a.to_class).cmp(&(&b.from_class, &b.to_class)))
    });
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{Edge, Node, NodeKind, SourceEvidence};
    use std::fs;
    fn graph() -> Graph {
        Graph {
            review_kpis: None,
            semantic: None,
            schema_version: 1,
            root: "/fixture".into(),
            nodes: [(3, "main.py"), (10, "worker.py"), (20, "model.py")]
                .into_iter()
                .map(|(id, name)| Node {
                    id: NodeId(id),
                    name: name.into(),
                    kind: NodeKind::File,
                    parent: None,
                    lang: Some("py".into()),
                })
                .collect(),
            edges: [(3, 10), (10, 20), (20, 10)]
                .into_iter()
                .map(|(from, to)| Edge {
                    from: NodeId(from),
                    to: NodeId(to),
                    refs: vec![],
                    count: Some(1),
                })
                .collect(),
        }
    }
    #[test]
    fn heavy_imports_are_evidenced_and_entry_points_exempted() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("main.py"), "import numpy\n").unwrap();
        fs::write(root.path().join("worker.py"),"import pandas\n\ndef local():\n    import numpy\n\ntext = '''\nimport tensorflow\n'''\n# import scipy\n").unwrap();
        fs::write(root.path().join("model.py"), "").unwrap();
        let full = graph();
        let report = analyze_performance(
            root.path(),
            &Config::default(),
            &full,
            &full,
            &ReviewSources::default(),
            &CallAnalysis::default(),
            &[NodeId(3)],
        )
        .unwrap();
        assert_eq!(
            report.source_findings.len(),
            1,
            "only the real eager import outside the configured entry point is flagged"
        );
        assert_eq!(report.source_findings[0].library.as_deref(), Some("pandas"));
        assert_eq!(report.source_findings[0].line, 1);
        assert_eq!(report.source_findings[0].raw, "import pandas");
    }
    #[test]
    fn entry_closure_condenses_cycles_and_counts_files_once() {
        let root = tempfile::tempdir().unwrap();
        let full = graph();
        let sources = ReviewSources {
            files: full
                .nodes
                .iter()
                .map(|node| SourceEvidence {
                    node_id: node.id,
                    path: Some(node.name.clone()),
                    lines: Some(10),
                    changes: None,
                })
                .collect(),
            ..ReviewSources::default()
        };
        let report = analyze_performance(
            root.path(),
            &Config::default(),
            &full,
            &full,
            &sources,
            &CallAnalysis::default(),
            &[NodeId(3)],
        )
        .unwrap();
        assert_eq!(
            report.entries.len(),
            1,
            "configured entry points receive a transitive load-closure report"
        );
        assert_eq!(report.entries[0].reachable_files, 3);
        assert_eq!(report.entries[0].lines, Some(30));
        assert_eq!(report.entries[0].longest_component_path, 2);
        assert_eq!(report.entries[0].cyclic_components, 1);
        assert_eq!(report.cyclic_groups[0].nodes, ["model.py", "worker.py"]);
    }
    #[test]
    fn javascript_dynamic_and_type_imports_are_not_eager_cost_findings() {
        let root = tempfile::tempdir().unwrap();
        fs::write(
            root.path().join("index.ts"),
            "// comment\nexport {a} from './a';\nexport * from './b';\n",
        )
        .unwrap();
        fs::write(root.path().join("load.ts"),"import type {T} from '@tensorflow/tfjs';\nconst lazy = () => import('@tensorflow/tfjs');\n// import fake from '@tensorflow/tfjs';\nconst text = `\nimport fake from '@tensorflow/tfjs';\n`;\nimport heavy from '@tensorflow/tfjs';\n").unwrap();
        let mut builder = archguard_core::GraphBuilder::new();
        builder.ensure_path("index.ts", Some("ts"));
        builder.ensure_path("load.ts", Some("ts"));
        let full = builder.build("/fixture".into());
        let mut config = Config::default();
        config.performance.min_barrel_exports = 2;
        let report = analyze_performance(
            root.path(),
            &config,
            &full,
            &full,
            &ReviewSources::default(),
            &CallAnalysis::default(),
            &[],
        )
        .unwrap();
        assert_eq!(report.source_findings.len(), 2);
        let barrel = report
            .source_findings
            .iter()
            .find(|finding| finding.kind == "barrel")
            .unwrap();
        assert_eq!((barrel.reexports, barrel.code_lines), (2, 2));
        let heavy = report
            .source_findings
            .iter()
            .find(|finding| finding.kind == "heavy_import")
            .unwrap();
        assert_eq!(heavy.line, 7);
    }
    #[test]
    fn abstraction_coverage_is_per_language_and_does_not_hide_partial_providers() {
        use archguard_core::{CallAnalyzer, CallClass, Cut, aggregate};
        let root = tempfile::tempdir().unwrap();
        let mut builder = archguard_core::GraphBuilder::new();
        for (file, language) in [
            ("java/Port.java", "java"),
            ("java/Impl.java", "java"),
            ("dotnet/Port.cs", "cs"),
            ("dotnet/Impl.cs", "cs"),
            ("python/worker.py", "py"),
        ] {
            builder.ensure_path(file, Some(language));
        }
        let full = builder.build("/fixture".into());
        let view = aggregate(&full, &Cut::at_depth(&full, 0));
        let mut calls = CallAnalysis {
            status: "partial".into(),
            files: 5,
            classes: [
                ("java/Port.java", "java", true),
                ("java/Impl.java", "java", false),
                ("dotnet/Port.cs", "csharp", true),
                ("dotnet/Impl.cs", "csharp", false),
            ]
            .into_iter()
            .map(|(file, language, is_abstract)| CallClass {
                language: Some(language.into()),
                id: file.into(),
                name: file.into(),
                package: file.split('/').next().unwrap().into(),
                file: file.into(),
                graph_node: file.into(),
                line: 1,
                kind: if is_abstract { "interface" } else { "class" }.into(),
                is_abstract: Some(is_abstract),
                visibility: Some("public".into()),
                supertypes: vec![],
                supertype_sites: None,
                declaration_files: None,
            })
            .collect(),
            analyzers: [
                ("java", "ready", 2),
                ("csharp", "ready", 2),
                ("python", "unavailable", 1),
            ]
            .into_iter()
            .map(|(language, status, files)| CallAnalyzer {
                language: language.into(),
                backend: "fixture".into(),
                status: status.into(),
                files,
                version: None,
                project: None,
                diagnostics: vec![],
            })
            .collect(),
            ..CallAnalysis::default()
        };
        let analyze = |calls: &CallAnalysis| {
            analyze_performance(
                root.path(),
                &Config::default(),
                &full,
                &view,
                &ReviewSources::default(),
                calls,
                &[],
            )
            .unwrap()
        };
        let metric = |report: &PerformanceReport, name: &str| {
            report
                .abstractions
                .iter()
                .find(|row| row.name == name)
                .unwrap()
                .clone()
        };
        let report = analyze(&calls);
        assert_eq!(
            metric(&report, "java").abstractness,
            Some(0.5),
            "an unsupported Python provider cannot invalidate complete Java evidence"
        );
        assert_eq!(
            metric(&report, "dotnet").abstractness,
            Some(0.5),
            "C# interface and class metadata participates in abstraction metrics"
        );
        assert_eq!(metric(&report, "dotnet").distance, Some(0.5));
        assert!(metric(&report, "python").abstractness.is_none());
        calls.analyzers[1].status = "partial".into();
        let report = analyze(&calls);
        assert_eq!(metric(&report, "java").abstractness, Some(0.5));
        assert!(
            metric(&report, "dotnet").abstractness.is_none(),
            "partial C# semantics must never imply complete abstractness"
        );
        assert_eq!(metric(&report, "dotnet").coverage, "unavailable");
        calls.analyzers[1].status = "ready".into();
        calls.analyzers[1].files = 1;
        assert!(
            metric(&analyze(&calls), "dotnet").abstractness.is_none(),
            "ready status alone cannot cover an omitted source file"
        );
        calls.analyzers[1].files = 2;
        calls.classes[2].is_abstract = None;
        assert!(
            metric(&analyze(&calls), "dotnet").abstractness.is_none(),
            "missing type metadata remains unknown"
        );
        calls.classes[2].is_abstract = Some(true);
        calls
            .analyzers
            .retain(|analyzer| analyzer.language != "csharp");
        assert!(
            metric(&analyze(&calls), "dotnet").abstractness.is_none(),
            "classes without a matching provider do not prove coverage"
        );
    }
    #[test]
    fn main_sequence_distance_uses_abstract_types_and_complete_java_coverage() {
        use archguard_core::{CallClass, Cut, aggregate};
        let root = tempfile::tempdir().unwrap();
        let mut builder = archguard_core::GraphBuilder::new();
        let a = builder.ensure_path("a/Port.java", Some("java"));
        let b = builder.ensure_path("b/Adapter.java", Some("java"));
        builder
            .edge(
                b,
                a,
                archguard_core::Ref {
                    file: "b/Adapter.java".into(),
                    line: 1,
                    raw: "import a.Port;".into(),
                },
            )
            .unwrap();
        let full = builder.build("/fixture".into());
        let view = aggregate(&full, &Cut::at_depth(&full, 0));
        let class = |id: &str, package: &str, graph_node: &str, is_abstract| CallClass {
            language: None,
            id: id.into(),
            name: id.into(),
            package: package.into(),
            file: graph_node.into(),
            graph_node: graph_node.into(),
            line: 1,
            kind: if is_abstract { "interface" } else { "class" }.into(),
            is_abstract: Some(is_abstract),
            visibility: Some("public".into()),
            supertypes: vec![],
            supertype_sites: None,
            declaration_files: None,
        };
        let mut calls = CallAnalysis {
            status: "ready".into(),
            files: 2,
            classes: vec![
                class("a.Port", "a", "a/Port.java", true),
                class("b.Adapter", "b", "b/Adapter.java", false),
            ],
            ..CallAnalysis::default()
        };
        let report = analyze_performance(
            root.path(),
            &Config::default(),
            &full,
            &view,
            &ReviewSources::default(),
            &calls,
            &[],
        )
        .unwrap();
        let a = report
            .abstractions
            .iter()
            .find(|metric| metric.name == "a")
            .expect("per-package abstraction metric");
        let b = report
            .abstractions
            .iter()
            .find(|metric| metric.name == "b")
            .unwrap();
        assert_eq!(
            (a.abstractness, a.instability, a.distance),
            (Some(1.0), 0.0, Some(0.0))
        );
        assert_eq!(
            (b.abstractness, b.instability, b.distance),
            (Some(0.0), 1.0, Some(0.0))
        );
        assert_eq!(
            review_zones(&view, &report),
            Some(archguard_core::ZoneStats {
                main_sequence: 2,
                ..Default::default()
            })
        );
        calls.status = "partial".into();
        let partial = analyze_performance(
            root.path(),
            &Config::default(),
            &full,
            &view,
            &ReviewSources::default(),
            &calls,
            &[],
        )
        .unwrap();
        assert!(
            partial
                .abstractions
                .iter()
                .all(|metric| metric.abstractness.is_none()
                    && metric.distance.is_none()
                    && metric.coverage == "unavailable")
        );
        assert_eq!(
            review_zones(&view, &partial),
            None,
            "no measurable boundary is not measured, never all-unmeasured zones"
        );
    }
    #[test]
    fn review_zones_count_nodes_once_and_skip_scc_metrics() {
        let metric =
            |scope: &str, name: &str, nodes: &[&str], a: Option<f64>, i: f64| AbstractionMetric {
                scope: scope.into(),
                name: name.into(),
                nodes: nodes.iter().map(|node| node.to_string()).collect(),
                types: 1,
                abstract_types: 0,
                fan_in: 0,
                fan_out: 0,
                instability: i,
                abstractness: a,
                distance: None,
                coverage: if a.is_some() {
                    "complete"
                } else {
                    "unavailable"
                }
                .into(),
                reason: None,
            };
        let view = graph();
        let names: Vec<_> = view.nodes.iter().map(|node| node.name.as_str()).collect();
        let mut abstractions = vec![
            metric("node", names[0], &[names[0]], Some(0.0), 0.0),
            metric("scc", "scc-1", &names, Some(0.0), 0.0),
        ];
        for name in &names[1..] {
            abstractions.push(metric("node", name, &[name], None, 0.0));
        }
        let report = PerformanceReport {
            abstractions,
            ..PerformanceReport::default()
        };
        let zones = review_zones(&view, &report).expect("one measured boundary");
        assert_eq!(zones.zone_of_pain, 1, "the SCC metric must not count again");
        assert_eq!(zones.unmeasured, names.len() - 1);
    }
    #[test]
    fn scc_abstractness_counts_types_instead_of_averaging_package_ratios() {
        use archguard_core::CallClass;
        let root = tempfile::tempdir().unwrap();
        let mut full = graph();
        for node in &mut full.nodes {
            node.lang = Some("java".into());
        }
        full.edges = vec![
            Edge {
                from: NodeId(3),
                to: NodeId(10),
                refs: vec![],
                count: Some(1),
            },
            Edge {
                from: NodeId(10),
                to: NodeId(3),
                refs: vec![],
                count: Some(1),
            },
            Edge {
                from: NodeId(10),
                to: NodeId(20),
                refs: vec![],
                count: Some(1),
            },
        ];
        let classes = [
            ("A", "main.py", true),
            ("Nested", "main.py", false),
            ("B", "worker.py", false),
            ("C", "model.py", false),
        ]
        .into_iter()
        .map(|(name, file, is_abstract)| CallClass {
            language: None,
            id: name.into(),
            name: name.into(),
            package: name.into(),
            file: file.into(),
            graph_node: file.into(),
            line: 1,
            kind: if is_abstract { "interface" } else { "class" }.into(),
            is_abstract: Some(is_abstract),
            visibility: Some("public".into()),
            supertypes: vec![],
            supertype_sites: None,
            declaration_files: None,
        })
        .collect();
        let calls = CallAnalysis {
            status: "ready".into(),
            files: 3,
            classes,
            ..CallAnalysis::default()
        };
        let report = analyze_performance(
            root.path(),
            &Config::default(),
            &full,
            &full,
            &ReviewSources::default(),
            &calls,
            &[],
        )
        .unwrap();
        let component = report
            .abstractions
            .iter()
            .find(|row| row.scope == "scc")
            .unwrap();
        assert_eq!((component.types, component.abstract_types), (3, 1));
        assert_eq!(component.abstractness, Some(1.0 / 3.0));
        assert_eq!(
            (component.fan_in, component.fan_out, component.instability),
            (0, 1, 1.0)
        );
        assert!((component.distance.unwrap() - 1.0 / 3.0).abs() < 1e-12);
    }
    #[test]
    fn concrete_call_candidates_find_inherited_contracts_and_keep_evidence() {
        use archguard_core::{CallClass, MethodCall};
        let class =
            |name: &str, package: &str, kind: &str, is_abstract, parents: Vec<&str>| CallClass {
                language: None,
                id: name.into(),
                name: name.into(),
                package: package.into(),
                file: format!("{name}.java"),
                graph_node: format!("{name}.java"),
                line: 1,
                kind: kind.into(),
                is_abstract: Some(is_abstract),
                visibility: Some("public".into()),
                supertypes: parents.into_iter().map(String::from).collect(),
                supertype_sites: None,
                declaration_files: None,
            };
        let calls = CallAnalysis {
            status: "ready".into(),
            files: 4,
            classes: vec![
                class("Port", "api", "interface", true, vec![]),
                class("Base", "core", "class", true, vec!["Port"]),
                class("Impl", "impl", "class", false, vec!["Base"]),
                class("Client", "client", "class", false, vec![]),
            ],
            calls: vec![MethodCall {
                from_class: "Client".into(),
                from_method: None,
                to_class: Some("Impl".into()),
                to_method: Some("Impl#run()".into()),
                target_label: "Impl#run()".into(),
                dispatch: "instance".into(),
                resolution: "project".into(),
                file: "Client.java".into(),
                line: 7,
                column: 3,
                code: "service.run()".into(),
            }],
            ..CallAnalysis::default()
        };
        let candidates = contract_candidates(&calls);
        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].interfaces,
            ["Port"],
            "a concrete target can inherit an interface through an abstract base"
        );
        assert_eq!(candidates[0].evidence[0].line, 7);
        assert_eq!(candidates[0].evidence[0].raw, "service.run()");
        let mut construction = calls.clone();
        construction.calls[0].dispatch = "constructor".into();
        assert!(
            contract_candidates(&construction)[0]
                .message
                .contains("factory"),
            "constructor dependencies need a construction boundary, not just interface substitution"
        );
    }
}
