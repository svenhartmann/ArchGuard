//! Explainable review priorities derived from the current import graph.

use std::collections::{BTreeMap, BTreeSet};

use archguard_core::{
    ArchguardError, GitStatus, Graph, ModuleReview, NodeId, NodeKind, OrphanCandidate,
    ReviewPriority, ReviewReport, ReviewScore, ReviewSources, ReviewSummary, SourceEvidence,
};
use globset::{Glob, GlobSetBuilder};

/// Direct entry-point matches, addressed by stable full-graph IDs. Consumers
/// can preserve these source-backed identities through virtual moves.
pub fn match_review_entry_points(
    full: &Graph,
    sources: &ReviewSources,
    patterns: &[String],
) -> Result<Vec<NodeId>, ArchguardError> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(Glob::new(pattern).map_err(|error| {
            ArchguardError::Config(format!(
                "invalid review entry-point glob {pattern:?}: {error}"
            ))
        })?);
    }
    let matcher = builder.build().map_err(|error| {
        ArchguardError::Config(format!("invalid review entry-point globs: {error}"))
    })?;
    let paths: BTreeMap<_, _> = sources
        .files
        .iter()
        .map(|file| (file.node_id, file.path.as_deref()))
        .collect();
    Ok(full
        .nodes
        .iter()
        .filter(|node| {
            matcher.is_match(&node.name)
                || paths
                    .get(&node.id)
                    .and_then(|path| *path)
                    .is_some_and(|path| matcher.is_match(path))
        })
        .map(|node| node.id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect())
}

/// Review all nodes of a view. Its names must refer to `full`; source IDs may
/// differ because aggregation assigns dense view IDs. Scores are an explicit
/// bounded heuristic, not a probability of defects or a claim of dead code.
pub fn analyze_review(
    full: &Graph,
    view: &Graph,
    sources: &ReviewSources,
    entry_points: &[String],
) -> Result<ReviewReport, ArchguardError> {
    let entry_point_ids = match_review_entry_points(full, sources, entry_points)?;
    let full_by_name: BTreeMap<_, _> = full
        .nodes
        .iter()
        .map(|node| (node.name.as_str(), node))
        .collect();
    let parents: BTreeMap<_, _> = full
        .nodes
        .iter()
        .map(|node| (node.id, node.parent))
        .collect();
    let evidence: BTreeMap<_, _> = sources
        .files
        .iter()
        .map(|file| (file.node_id, file))
        .collect();
    let mut members: BTreeMap<NodeId, BTreeSet<NodeId>> = BTreeMap::new();
    for file in full
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::File && !node.name.ends_with("/*"))
    {
        for id in parent_chain(file.id, &parents) {
            members.entry(id).or_default().insert(file.id);
        }
    }
    let entry_ancestors: BTreeSet<_> = entry_point_ids
        .iter()
        .flat_map(|id| parent_chain(*id, &parents))
        .collect();
    let indices: BTreeMap<_, _> = view
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id, index))
        .collect();
    let mut outgoing = vec![BTreeSet::new(); view.nodes.len()];
    let mut incoming = outgoing.clone();
    for edge in &view.edges {
        if let (Some(&from), Some(&to)) = (indices.get(&edge.from), indices.get(&edge.to)) {
            outgoing[from].insert(to);
            incoming[to].insert(from);
        }
    }
    let sccs = crate::scc::find_sccs(view);
    let scc_sizes: BTreeMap<_, _> = sccs
        .iter()
        .flat_map(|scc| {
            scc.node_ids
                .iter()
                .map(|id| (*id, scc.node_ids.len() as u32))
        })
        .collect();
    let mut shown_files = BTreeSet::new();
    let mut nodes = Vec::with_capacity(view.nodes.len());
    for (index, node) in view.nodes.iter().enumerate() {
        let source_id = full_by_name.get(node.name.as_str()).map(|source| source.id);
        let files = source_id
            .and_then(|id| members.get(&id))
            .cloned()
            .unwrap_or_default();
        shown_files.extend(files.iter().copied());
        let (mapped_files, lines, changes) = source_totals(&files, &evidence, sources.git.status);
        let fan_in = incoming[index].len() as u32;
        let fan_out = outgoing[index].len() as u32;
        let degree = fan_in.saturating_add(fan_out);
        let scc_size = scc_sizes.get(&node.id).copied().unwrap_or(0);
        let dependencies = reachable(index, &outgoing);
        let dependents = reachable(index, &incoming);
        let components = ReviewScore {
            cycle: if scc_size > 0 {
                30u32
                    .saturating_add(2u32.saturating_mul(scc_size - 1))
                    .min(40)
            } else {
                0
            },
            coupling: degree.saturating_mul(2).min(20),
            impact: dependents.min(20),
            churn: changes.map_or(0, |count| {
                count.saturating_mul(u64::from(degree.min(5))).min(20) as u32
            }),
        };
        let score = components.cycle + components.coupling + components.impact + components.churn;
        let priority = if score >= 40 {
            ReviewPriority::High
        } else if score >= 20 {
            ReviewPriority::Medium
        } else {
            ReviewPriority::Low
        };
        let entry_point = source_id.is_some_and(|id| {
            entry_ancestors.contains(&id)
                || parent_chain(id, &parents)
                    .iter()
                    .any(|ancestor| entry_point_ids.contains(ancestor))
        });
        let orphan = if fan_in == 0 && !entry_point && !files.is_empty() {
            Some(if fan_out == 0 {
                OrphanCandidate::Isolated
            } else {
                OrphanCandidate::Unreferenced
            })
        } else {
            None
        };
        let mut reasons = Vec::new();
        if components.cycle > 0 {
            reasons.push(format!(
                "Cyclic group: {scc_size} nodes (+{})",
                components.cycle
            ));
        }
        if components.coupling > 0 {
            reasons.push(format!(
                "Direct coupling: {fan_in} incoming + {fan_out} outgoing (+{})",
                components.coupling
            ));
        }
        if components.impact > 0 {
            reasons.push(format!(
                "Transitive dependents: {dependents} (+{})",
                components.impact
            ));
        }
        if components.churn > 0 {
            reasons.push(format!(
                "Git change touches × coupling: {} × {} (+{})",
                changes.unwrap_or(0),
                degree.min(5),
                components.churn
            ));
        }
        if orphan.is_some() {
            reasons.push("No incoming imports in this scope; check entry points and dynamic consumers before removing code.".into());
        }
        nodes.push(ModuleReview {
            node_id: node.id,
            source_id,
            name: node.name.clone(),
            kind: node.kind,
            rank: 0,
            score,
            priority,
            fan_in,
            fan_out,
            instability: if degree == 0 {
                0.0
            } else {
                f64::from(fan_out) / f64::from(degree)
            },
            scc_size,
            dependencies,
            dependents,
            file_count: files.len() as u32,
            mapped_files,
            lines,
            changes,
            entry_point,
            orphan,
            components,
            reasons,
        });
    }
    nodes.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.node_id.cmp(&right.node_id))
    });
    for (index, node) in nodes.iter_mut().enumerate() {
        node.rank = index as u32 + 1;
    }
    let (ccd, acd, nccd) = crate::lakos::lakos_metrics(view);
    let (mapped_files, lines, changes) = source_totals(&shown_files, &evidence, sources.git.status);
    let summary = ReviewSummary {
        scope_nodes: view.nodes.len() as u32,
        scope_edges: outgoing.iter().map(BTreeSet::len).sum::<usize>() as u32,
        ccd,
        acd,
        nccd,
        cyclic_nodes: scc_sizes.len() as u32,
        largest_scc: sccs
            .iter()
            .map(|scc| scc.node_ids.len() as u32)
            .max()
            .unwrap_or(0),
        high_priority: nodes
            .iter()
            .filter(|node| node.priority == ReviewPriority::High)
            .count() as u32,
        orphan_candidates: nodes.iter().filter(|node| node.orphan.is_some()).count() as u32,
        source_files: shown_files.len() as u32,
        mapped_files,
        lines,
        changes,
    };
    Ok(ReviewReport {
        schema_version: 1,
        score_version: "review-priority/v1".into(),
        summary,
        nodes,
        git: sources.git.clone(),
        entry_points: entry_points.to_vec(),
        entry_point_ids,
        limitations: vec![
            "Priority is a heuristic review order, not defect probability or measured runtime/build performance.".into(),
            "Orphan candidates have no incoming imports in this scope; public APIs, entry points and dynamic consumers may still use them.".into(),
            "Git changes count file-commit touches, not changed lines or distinct package commits; shallow/truncated history supplies lower bounds.".into(),
            "LOC and Git totals are unknown when any represented source file lacks evidence; no abstractness or Main Sequence Distance is inferred.".into(),
        ],
    })
}

fn parent_chain(id: NodeId, parents: &BTreeMap<NodeId, Option<NodeId>>) -> BTreeSet<NodeId> {
    let mut ids = BTreeSet::new();
    let mut current = Some(id);
    while let Some(id) = current {
        if !ids.insert(id) {
            break;
        }
        current = parents.get(&id).copied().flatten();
    }
    ids
}

fn source_totals(
    files: &BTreeSet<NodeId>,
    evidence: &BTreeMap<NodeId, &SourceEvidence>,
    git: GitStatus,
) -> (u32, Option<u64>, Option<u64>) {
    let mapped = files
        .iter()
        .filter(|id| evidence.get(id).is_some_and(|file| file.path.is_some()))
        .count() as u32;
    let lines = files
        .iter()
        .try_fold(0u64, |sum, id| sum.checked_add(evidence.get(id)?.lines?));
    let changes = if git == GitStatus::Available {
        files.iter().try_fold(0u64, |sum, id| {
            sum.checked_add(u64::from(evidence.get(id)?.changes?))
        })
    } else {
        None
    };
    (mapped, lines, changes)
}

fn reachable(start: usize, adjacency: &[BTreeSet<usize>]) -> u32 {
    let mut reached = BTreeSet::from([start]);
    let mut stack = vec![start];
    while let Some(node) = stack.pop() {
        for next in &adjacency[node] {
            if reached.insert(*next) {
                stack.push(*next);
            }
        }
    }
    (reached.len() - 1) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{Edge, GitStatus, Node, NodeId, NodeKind, SourceEvidence};

    fn graph() -> Graph {
        Graph {
            review_kpis: None,
            semantic: None,
            schema_version: 1,
            root: "/project".into(),
            nodes: [(3, "a.ts"), (10, "b.ts"), (20, "c.ts"), (42, "isolated.ts")]
                .into_iter()
                .map(|(id, name)| Node {
                    id: NodeId(id),
                    name: name.into(),
                    kind: NodeKind::File,
                    parent: None,
                    lang: None,
                })
                .collect(),
            edges: [(3, 10), (10, 3), (20, 3)]
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
    fn score_explains_cycles_coupling_impact_and_git_touches() {
        let full = graph();
        let mut sources = ReviewSources::default();
        sources.git.status = GitStatus::Available;
        sources.files = full
            .nodes
            .iter()
            .map(|node| SourceEvidence {
                node_id: node.id,
                path: Some(node.name.clone()),
                lines: Some(10),
                changes: Some(if node.id == NodeId(3) { 4 } else { 0 }),
            })
            .collect();
        let report = analyze_review(&full, &full, &sources, &[]).unwrap();
        assert_eq!(
            report.nodes.len(),
            4,
            "every view node receives an explainable priority"
        );
        let a = &report.nodes[0];
        assert_eq!(a.name, "a.ts");
        assert_eq!(
            (
                a.fan_in,
                a.fan_out,
                a.dependencies,
                a.dependents,
                a.scc_size
            ),
            (2, 1, 1, 2, 2)
        );
        assert_eq!(
            (
                a.components.cycle,
                a.components.coupling,
                a.components.impact,
                a.components.churn
            ),
            (32, 6, 2, 12)
        );
        assert_eq!(a.score, 52);
        assert_eq!(report.summary.ccd, 8);
        assert_eq!(report.summary.acd, 2.0);
    }

    #[test]
    fn unknown_history_is_not_measured_zero_and_ranking_is_not_a_high_label() {
        let full = graph();
        let report = analyze_review(&full, &full, &ReviewSources::default(), &[]).unwrap();
        let a = &report.nodes[0];
        assert_eq!((a.score, a.priority), (40, ReviewPriority::High));
        assert_eq!(a.changes, None);
        assert_eq!(a.lines, None);
        assert_eq!(a.mapped_files, 0);
        let lone = Graph {
            semantic: None,
            review_kpis: None,
            schema_version: 1,
            root: "/project".into(),
            nodes: vec![full.nodes[3].clone()],
            edges: vec![],
        };
        let only = &analyze_review(&lone, &lone, &ReviewSources::default(), &[])
            .unwrap()
            .nodes[0];
        assert_eq!(
            (only.rank, only.score, only.priority),
            (1, 0, ReviewPriority::Low)
        );
        assert_eq!(only.orphan, Some(OrphanCandidate::Isolated));
        assert!(only.reasons[0].contains("before removing code"));
    }

    #[test]
    fn source_aggregation_uses_parent_ids_and_unknown_files_make_totals_unknown() {
        let mut full = graph();
        full.nodes.push(Node {
            id: NodeId(100),
            name: "package".into(),
            kind: NodeKind::Module,
            parent: None,
            lang: None,
        });
        full.nodes[0].parent = Some(NodeId(100));
        full.nodes[1].parent = Some(NodeId(100));
        // Neither file name has a package prefix: containment is authoritative.
        let view = archguard_core::aggregate(&full, &archguard_core::Cut::at_depth(&full, 0));
        let mut sources = ReviewSources::default();
        sources.git.status = GitStatus::Available;
        sources.files = vec![
            SourceEvidence {
                node_id: NodeId(3),
                path: Some("source/a.ts".into()),
                lines: Some(10),
                changes: Some(4),
            },
            SourceEvidence {
                node_id: NodeId(10),
                path: Some("elsewhere/b.ts".into()),
                lines: Some(20),
                changes: Some(7),
            },
        ];
        let report = analyze_review(&full, &view, &sources, &[]).unwrap();
        let package = report
            .nodes
            .iter()
            .find(|node| node.name == "package")
            .unwrap();
        assert_eq!(package.source_id, Some(NodeId(100)));
        assert_ne!(package.node_id, NodeId(100));
        assert_eq!(
            (
                package.file_count,
                package.mapped_files,
                package.lines,
                package.changes
            ),
            (2, 2, Some(30), Some(11))
        );
        assert_eq!(report.summary.source_files, 4);
        assert_eq!(
            (
                report.summary.mapped_files,
                report.summary.lines,
                report.summary.changes
            ),
            (2, None, None)
        );
        sources.files.pop();
        let report = analyze_review(&full, &view, &sources, &[]).unwrap();
        let package = report
            .nodes
            .iter()
            .find(|node| node.name == "package")
            .unwrap();
        assert_eq!(
            (
                package.file_count,
                package.mapped_files,
                package.lines,
                package.changes
            ),
            (2, 1, None, None)
        );
        assert_eq!(package.components.churn, 0);
    }

    #[test]
    fn entry_points_match_physical_paths_and_propagate_to_containing_package() {
        let mut full = graph();
        full.nodes.push(Node {
            id: NodeId(100),
            name: "package".into(),
            kind: NodeKind::Module,
            parent: None,
            lang: None,
        });
        full.nodes[3].parent = Some(NodeId(100));
        let sources = ReviewSources {
            files: vec![SourceEvidence {
                node_id: NodeId(42),
                path: Some("src/cli/entry.ts".into()),
                lines: None,
                changes: None,
            }],
            ..Default::default()
        };
        let patterns = vec!["src/cli/**".into()];
        assert_eq!(
            match_review_entry_points(&full, &sources, &patterns).unwrap(),
            vec![NodeId(42)]
        );
        let view = archguard_core::aggregate(&full, &archguard_core::Cut::at_depth(&full, 0));
        let report = analyze_review(&full, &view, &sources, &patterns).unwrap();
        let package = report
            .nodes
            .iter()
            .find(|node| node.name == "package")
            .unwrap();
        assert!(package.entry_point);
        assert_eq!(package.orphan, None);
        assert_eq!(report.entry_point_ids, vec![NodeId(42)]);
        assert!(matches!(
            analyze_review(&full, &view, &sources, &["[".into()]),
            Err(ArchguardError::Config(_))
        ));
    }

    #[test]
    fn declared_module_entry_point_also_exempts_its_file_children() {
        let mut full = graph();
        full.nodes.push(Node {
            id: NodeId(100),
            name: "cli".into(),
            kind: NodeKind::Module,
            parent: None,
            lang: None,
        });
        full.nodes[3].parent = Some(NodeId(100));
        let view =
            archguard_core::aggregate(&full, &archguard_core::Cut::at_depth(&full, usize::MAX));
        let report =
            analyze_review(&full, &view, &ReviewSources::default(), &["cli".into()]).unwrap();
        let child = report
            .nodes
            .iter()
            .find(|node| node.name == "isolated.ts")
            .unwrap();
        assert!(
            child.entry_point,
            "a file inside a declared entry-point module is not an orphan"
        );
        assert_eq!(child.orphan, None);
    }

    #[test]
    fn default_entrypoints_exclude_roots_tests_and_cli_without_excluding_normal_files() {
        let names = [
            "main.rs",
            "src/index.ts",
            "tests/spec.ts",
            "lib.rs",
            "src/cli/run.ts",
            "src/test_value.py",
            "src/value.test.ts",
            "src/domain/value.ts",
        ];
        let full = Graph {
            semantic: None,
            review_kpis: None,
            schema_version: 1,
            root: "/project".into(),
            nodes: names
                .iter()
                .enumerate()
                .map(|(id, name)| Node {
                    id: NodeId(id as u32),
                    name: (*name).into(),
                    kind: NodeKind::File,
                    parent: None,
                    lang: None,
                })
                .collect(),
            edges: vec![],
        };
        let report = analyze_review(
            &full,
            &full,
            &ReviewSources::default(),
            &archguard_core::ReviewConfig::default().entry_points,
        )
        .unwrap();
        assert_eq!(
            report.entry_point_ids,
            (0..7).map(NodeId).collect::<Vec<_>>()
        );
        assert_eq!(report.summary.orphan_candidates, 1);
        assert_eq!(
            report
                .nodes
                .iter()
                .find(|node| node.orphan.is_some())
                .unwrap()
                .name,
            "src/domain/value.ts"
        );
    }

    #[test]
    fn duplicate_and_unknown_edges_do_not_inflate_metrics_or_acyclic_rows() {
        let mut full = graph();
        full.edges.push(full.edges[0].clone());
        full.edges.push(Edge {
            from: NodeId(999),
            to: NodeId(3),
            refs: vec![],
            count: Some(1),
        });
        let report = analyze_review(&full, &full, &ReviewSources::default(), &[]).unwrap();
        assert_eq!(report.summary.scope_edges, 3);
        assert_eq!(
            (report.summary.cyclic_nodes, report.summary.largest_scc),
            (2, 2)
        );
        assert_eq!((report.nodes[0].fan_in, report.nodes[0].fan_out), (2, 1));
        let c = report
            .nodes
            .iter()
            .find(|node| node.name == "c.ts")
            .unwrap();
        assert_eq!(
            (c.scc_size, c.dependencies, c.dependents, c.orphan),
            (0, 2, 0, Some(OrphanCandidate::Unreferenced))
        );
    }

    #[test]
    fn self_cycles_and_empty_graph_have_finite_honest_metrics() {
        let mut full = graph();
        full.nodes.truncate(1);
        full.edges = vec![Edge {
            from: NodeId(3),
            to: NodeId(3),
            refs: vec![],
            count: None,
        }];
        let report = analyze_review(&full, &full, &ReviewSources::default(), &[]).unwrap();
        assert_eq!(
            (
                report.nodes[0].scc_size,
                report.nodes[0].dependencies,
                report.nodes[0].dependents
            ),
            (1, 0, 0)
        );
        assert_eq!(report.nodes[0].components.cycle, 30);
        assert_eq!(report.nodes[0].orphan, None);
        full.nodes.clear();
        full.edges.clear();
        let report = analyze_review(&full, &full, &ReviewSources::default(), &[]).unwrap();
        assert!(report.nodes.is_empty());
        assert_eq!(report.summary.acd, 0.0);
        assert_eq!(report.summary.nccd, 0.0);
        assert_eq!(
            (report.summary.lines, report.summary.changes),
            (Some(0), None)
        );
        assert!(serde_json::to_string(&report).is_ok());
    }

    #[test]
    fn pseudo_files_are_not_orphan_or_size_evidence_and_ties_are_deterministic() {
        let mut full = graph();
        full.nodes.push(Node {
            id: NodeId(99),
            name: "external/*".into(),
            kind: NodeKind::File,
            parent: None,
            lang: None,
        });
        let report = analyze_review(&full, &full, &ReviewSources::default(), &[]).unwrap();
        let pseudo = report
            .nodes
            .iter()
            .find(|node| node.name == "external/*")
            .unwrap();
        assert_eq!((pseudo.file_count, pseudo.orphan), (0, None));
        full.nodes.reverse();
        full.edges.reverse();
        assert_eq!(
            analyze_review(&full, &full, &ReviewSources::default(), &[]).unwrap(),
            report
        );
    }

    #[test]
    fn missing_git_counts_differ_from_measured_zero_and_partial_history_is_labelled() {
        let full = graph();
        let mut sources = ReviewSources::default();
        sources.git.status = GitStatus::Available;
        sources.files = full
            .nodes
            .iter()
            .map(|node| SourceEvidence {
                node_id: node.id,
                path: Some(node.name.clone()),
                lines: Some(10),
                changes: Some(0),
            })
            .collect();
        sources.git.shallow = true;
        sources.git.truncated = true;
        let report = analyze_review(&full, &full, &sources, &[]).unwrap();
        assert_eq!(report.summary.changes, Some(0));
        assert!(report.git.shallow && report.git.truncated);
        assert!(
            report
                .limitations
                .iter()
                .any(|message| message.contains("lower bounds"))
        );
        sources.git.status = GitStatus::Unavailable;
        let report = analyze_review(&full, &full, &sources, &[]).unwrap();
        assert_eq!(report.summary.changes, None);
        assert!(
            report
                .nodes
                .iter()
                .all(|node| node.changes.is_none() && node.components.churn == 0)
        );
    }
}
