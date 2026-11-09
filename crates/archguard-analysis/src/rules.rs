//! Declared file-edge prohibitions and opt-in orphan candidates.

use archguard_core::{
    ArchguardError, DependencyRule, Finding, FindingKind, Graph, NodeKind, OrphanPolicy,
    ReviewConfig, ReviewSources, Severity,
};
use globset::{Glob, GlobSet, GlobSetBuilder};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

struct CompiledRule<'a> {
    rule: &'a DependencyRule,
    from: GlobSet,
    to: GlobSet,
}

fn compile_rules(rules: &[DependencyRule]) -> Result<Vec<CompiledRule<'_>>, ArchguardError> {
    let mut names = BTreeSet::new();
    let mut compiled = vec![];
    for (index, rule) in rules.iter().enumerate() {
        if rule.name.trim().is_empty() || !names.insert(&rule.name) {
            return Err(ArchguardError::Config(format!(
                "dependency_rules[{index}].name must be nonempty and unique"
            )));
        }
        let make_set = |patterns: &[String], field: &str| -> Result<GlobSet, ArchguardError> {
            if patterns.is_empty() {
                return Err(ArchguardError::Config(format!(
                    "dependency_rules[{index}].{field} must contain at least one pattern"
                )));
            }
            let mut set = GlobSetBuilder::new();
            for pattern in patterns {
                if pattern.trim().is_empty() {
                    return Err(ArchguardError::Config(format!(
                        "dependency_rules[{index}].{field} contains an empty pattern"
                    )));
                }
                set.add(Glob::new(pattern).map_err(|error| {
                    ArchguardError::Config(format!("dependency_rules[{index}].{field}: {error}"))
                })?);
            }
            set.build()
                .map_err(|error| ArchguardError::Config(error.to_string()))
        };
        compiled.push(CompiledRule {
            rule,
            from: make_set(&rule.from, "from")?,
            to: make_set(&rule.to, "to")?,
        });
    }
    Ok(compiled)
}

pub fn validate_dependency_rules(rules: &[DependencyRule]) -> Result<(), ArchguardError> {
    compile_rules(rules).map(|_| ())
}

/// Evaluate prohibitions against original file imports, never a collapsed view.
/// Rule/pair identities are independent of input ordering and unrelated findings.
#[tracing::instrument(skip_all)]
pub fn check_dependency_rules(
    full: &Graph,
    rules: &[DependencyRule],
) -> Result<Vec<Finding>, ArchguardError> {
    let compiled = compile_rules(rules)?;
    let nodes: BTreeMap<_, _> = full.nodes.iter().map(|node| (node.id, node)).collect();
    let mut pairs = BTreeMap::new();
    for edge in &full.edges {
        let (Some(from), Some(to)) = (nodes.get(&edge.from), nodes.get(&edge.to)) else {
            continue;
        };
        if from.kind != NodeKind::File || to.kind != NodeKind::File {
            continue;
        }
        pairs
            .entry((from.name.as_str(), to.name.as_str()))
            .or_insert_with(Vec::new)
            .extend(edge.refs.iter().cloned());
    }
    let mut findings = vec![];
    for ((from, to), mut evidence) in pairs {
        evidence.sort_by(|left, right| {
            (&left.file, left.line, &left.raw).cmp(&(&right.file, right.line, &right.raw))
        });
        evidence.dedup();
        evidence.truncate(10);
        for compiled in &compiled {
            if !compiled.from.is_match(from) || !compiled.to.is_match(to) {
                continue;
            }
            let rule = compiled.rule;
            let message = if rule.message.trim().is_empty() {
                format!("Rule {:?} forbids {from} → {to}.", rule.name)
            } else {
                format!("{} ({from} → {to})", rule.message)
            };
            let mut digest = Sha256::new();
            for text in [&rule.name, from, to] {
                digest.update((text.len() as u64).to_be_bytes());
                digest.update(text.as_bytes());
            }
            let mut members = vec![from.to_string(), to.to_string()];
            members.sort();
            members.dedup();
            findings.push(Finding {
                kind: FindingKind::DependencyRule,
                id: format!("dependency-rule:{}:{:x}", rule.name, digest.finalize()),
                severity: rule.severity,
                message: Some(message),
                evidence: evidence.clone(),
                nodes: members,
                edge_count: 1,
                cycles_sampled: 0,
                sampling: None,
                suggested_cuts: vec![],
            });
        }
    }
    findings.sort_by(|left, right| {
        left.severity
            .cmp(&right.severity)
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(findings)
}

/// Files without incoming static imports are review candidates. This opt-in
/// policy does not prove that dynamic consumers or public API callers are absent.
#[tracing::instrument(skip_all)]
pub fn check_orphans(full: &Graph, config: &ReviewConfig) -> Result<Vec<Finding>, ArchguardError> {
    let severity = match config.orphans {
        OrphanPolicy::Off => return Ok(vec![]),
        OrphanPolicy::Info => Severity::Info,
        OrphanPolicy::Warning => Severity::Warning,
        OrphanPolicy::Error => Severity::Error,
    };
    let entry_points: BTreeSet<_> = crate::review::match_review_entry_points(
        full,
        &ReviewSources::default(),
        &config.entry_points,
    )?
    .into_iter()
    .collect();
    let known: BTreeSet<_> = full.nodes.iter().map(|node| node.id).collect();
    let mut incoming = BTreeSet::new();
    let mut outgoing = BTreeSet::new();
    for edge in &full.edges {
        if known.contains(&edge.from) && known.contains(&edge.to) {
            incoming.insert(edge.to);
            outgoing.insert(edge.from);
        }
    }
    let mut findings = vec![];
    for node in &full.nodes {
        if node.kind != NodeKind::File
            || node.name.ends_with("/*")
            || incoming.contains(&node.id)
            || entry_points.contains(&node.id)
            || full
                .ancestors_of(node.id)
                .iter()
                .any(|id| entry_points.contains(id))
        {
            continue;
        }
        let kind = if outgoing.contains(&node.id) {
            "Unreferenced source file"
        } else {
            "Isolated source file"
        };
        findings.push(Finding {kind:FindingKind::Orphan,id:format!("orphan:{}",node.name),severity,message:Some(format!("{kind}: no incoming static imports were found. Check entry points, public APIs and dynamic consumers before removing code.")),evidence:vec![],nodes:vec![node.name.clone()],edge_count:0,cycles_sampled:0,sampling:None,suggested_cuts:vec![]});
    }
    findings.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(findings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{Edge, Node, NodeId, Ref};
    fn fixture() -> Graph {
        Graph {
            review_kpis: None,
            semantic: None,
            schema_version: 1,
            root: "/repo".into(),
            nodes: [(5, "ui/View.ts"), (10, "storage/Db.ts")]
                .into_iter()
                .map(|(id, name)| Node {
                    id: NodeId(id),
                    name: name.into(),
                    kind: NodeKind::File,
                    parent: None,
                    lang: None,
                })
                .collect(),
            edges: vec![Edge {
                from: NodeId(5),
                to: NodeId(10),
                count: Some(1),
                refs: vec![Ref {
                    file: "ui/View.ts".into(),
                    line: 3,
                    raw: "import '../storage/Db';".into(),
                }],
            }],
        }
    }
    fn rule() -> DependencyRule {
        DependencyRule {
            name: "ui-no-storage".into(),
            from: vec!["ui/**".into()],
            to: vec!["storage/**".into()],
            message: "Use the application service.".into(),
            ..Default::default()
        }
    }
    #[test]
    fn duplicate_edges_merge_evidence_and_identity_survives_unrelated_changes() {
        let mut graph = fixture();
        let rules = vec![rule()];
        let original = check_dependency_rules(&graph, &rules).unwrap();
        assert_eq!(original.len(), 1);
        graph.edges.push(graph.edges[0].clone());
        graph.nodes.reverse();
        graph.nodes.push(Node {
            id: NodeId(2),
            name: "a.ts".into(),
            kind: NodeKind::File,
            parent: None,
            lang: None,
        });
        graph.edges.push(Edge {
            from: NodeId(999),
            to: NodeId(5),
            count: Some(1),
            refs: vec![],
        });
        assert_eq!(check_dependency_rules(&graph, &rules).unwrap(), original);
        assert_eq!(original[0].evidence.len(), 1);
        assert_eq!(original[0].edge_count, 1);
        assert_eq!(original[0].severity, Severity::Error);
        assert!(
            original[0]
                .message
                .as_ref()
                .unwrap()
                .contains("ui/View.ts → storage/Db.ts")
        );
    }
    #[test]
    fn symbolic_pseudo_nodes_are_not_orphan_candidates_and_module_entries_cover_children() {
        let mut graph = fixture();
        graph.edges.clear();
        graph.nodes.push(Node {
            id: NodeId(1),
            name: "ui".into(),
            kind: NodeKind::Module,
            parent: None,
            lang: None,
        });
        graph.nodes[0].parent = Some(NodeId(1));
        graph.nodes.push(Node {
            id: NodeId(40),
            name: "external/*".into(),
            kind: NodeKind::File,
            parent: None,
            lang: None,
        });
        let config = ReviewConfig {
            orphans: OrphanPolicy::Warning,
            entry_points: vec!["ui".into()],
            ..Default::default()
        };
        let findings = check_orphans(&graph, &config).unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].nodes, vec!["storage/Db.ts"]);
    }
}
