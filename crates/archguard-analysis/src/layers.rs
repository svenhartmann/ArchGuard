//! Layer-rule checking (T15): every edge must respect the declared or
//! derived layering of the graph (ARCHITECTURE.md Kap. 11).
//!
//! Each node is assigned to the **first** [`LayerConfig`] in slice order
//! whose `contains` globs match the node's *name* — names are normalized
//! module paths (Kap. 5.1), matched with `globset` (Kap. 9 table: the
//! Layer-Check row). A node no layer claims stays *unconstrained*: edges
//! touching it can never violate a layer rule.
//!
//! An edge is checked only when *both* endpoints have a layer. It violates
//! the rules when the source layer differs from the target layer and the
//! target layer's name is not in the source layer's `may_depend_on`
//! (direct edges only, no transitivity — Kap. 9). All violating edges
//! between the same *(source layer, target layer)* pair aggregate into a
//! **single** [`Finding`]:
//!
//! * `kind` [`FindingKind::LayerViolation`], `edge_count` `1` (one
//!   aggregated finding, not a sum), no `cycles_sampled`, no suggestions.
//! * `severity` follows the *origin of the violated (source) layer*
//!   (Kap. 11 Promotion): [`LayerOrigin::Declared`] → [`Severity::Error`]
//!   (a CI gate), [`LayerOrigin::Derived`] → [`Severity::Info`] (a hint
//!   that never fails `check`).
//! * `nodes` interleaves the involved module **names** as flat
//!   `src, tgt, src, tgt, …` pairs — up to 10 distinct pairs (20 names)
//!   per finding, the lexicographically smallest ones.
//! * `id` is `layer-<n>` (1-based, the `scc-<n>` style of `scc.rs`),
//!   numbered in output order.
//!
//! Determinism (Kap. 12): pairs are deduplicated and sorted in a
//! `BTreeSet`, groups live in a `BTreeMap` keyed by *(source layer
//! index, target layer index)*, so repeated calls return the same vector
//! in the same order and the `layer-<n>` ids are stable.

use std::collections::{BTreeMap, BTreeSet};

use archguard_core::{
    ArchguardError, Finding, FindingKind, Graph, LayerConfig, LayerOrigin, NodeId, Severity,
};
use globset::{Glob, GlobSet, GlobSetBuilder};
use sha2::{Digest, Sha256};

/// Maximum number of node pairs one aggregated finding lists in `nodes`.
const MAX_PAIRS_PER_FINDING: usize = 10;

/// Check `graph`'s edges against `layers` (slice order = assignment
/// priority), returning one aggregated [`Finding`] per violating layer
/// pair. See the module docs for the assignment rule, the severity
/// mapping (Kap. 11) and the determinism guarantee. An empty `layers`
/// slice leaves every node unconstrained and yields no findings.
pub fn check_layers(graph: &Graph, layers: &[LayerConfig]) -> Result<Vec<Finding>, ArchguardError> {
    Ok(collect_violations(graph, layers)?
        .into_iter()
        .enumerate()
        .map(|(index, ((_, _), violation))| Finding {
            message: None,
            evidence: Vec::new(),
            kind: FindingKind::LayerViolation,
            id: format!("layer-{}", index + 1),
            severity: violation.severity,
            nodes: flatten_pairs(&violation.pairs),
            edge_count: 1,
            cycles_sampled: 0,
            sampling: None,
            suggested_cuts: Vec::new(),
        })
        .collect())
}

/// The untruncated directed-pair form used by revision comparison. Display
/// samples cannot determine whether a new violation was introduced.
pub fn check_layer_pairs(
    graph: &Graph,
    layers: &[LayerConfig],
) -> Result<Vec<Finding>, ArchguardError> {
    let mut evidence = BTreeMap::new();
    for edge in &graph.edges {
        if let (Some(from), Some(to)) = (graph.node_name(edge.from), graph.node_name(edge.to)) {
            evidence
                .entry((from.to_string(), to.to_string()))
                .or_insert_with(Vec::new)
                .extend(edge.refs.iter().cloned());
        }
    }
    let mut findings = vec![];
    for ((from_layer, to_layer), violation) in collect_violations(graph, layers)? {
        for (from, to) in violation.pairs {
            let mut refs = evidence
                .remove(&(from.clone(), to.clone()))
                .unwrap_or_default();
            refs.sort_by(|left, right| {
                (&left.file, left.line, &left.raw).cmp(&(&right.file, right.line, &right.raw))
            });
            refs.dedup();
            refs.truncate(10);
            let mut digest = Sha256::new();
            for name in [&from, &to] {
                digest.update((name.len() as u64).to_be_bytes());
                digest.update(name.as_bytes());
            }
            findings.push(Finding {
                kind: FindingKind::LayerViolation,
                id: format!("layer-pair:{:x}", digest.finalize()),
                severity: violation.severity,
                message: Some(format!(
                    "Layer {:?} cannot depend on {:?}: {from} → {to}.",
                    layers[from_layer].name, layers[to_layer].name
                )),
                evidence: refs,
                nodes: vec![from, to],
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

fn collect_violations(
    graph: &Graph,
    layers: &[LayerConfig],
) -> Result<BTreeMap<(usize, usize), Violation>, ArchguardError> {
    let sets = compile_layers(layers)?;
    let assignment = assign_layers(graph, &sets);

    // (source layer, target layer) -> sorted distinct node-name pairs.
    // BTreeMap over layer *indices* fixes both grouping and output order.
    let mut violations: BTreeMap<(usize, usize), Violation> = BTreeMap::new();
    for edge in &graph.edges {
        // Unknown endpoint ids cannot be assigned and are skipped.
        let (Some(src), Some(tgt)) = (
            layer_of(&assignment, edge.from),
            layer_of(&assignment, edge.to),
        ) else {
            continue;
        };
        if src == tgt || may_depend_on(&layers[src], &layers[tgt].name) {
            continue;
        }
        let (Some(src_name), Some(tgt_name)) =
            (graph.node_name(edge.from), graph.node_name(edge.to))
        else {
            continue;
        };
        violations
            .entry((src, tgt))
            .or_insert_with(|| Violation {
                pairs: Default::default(),
                severity: severity_of(&layers[src]),
            })
            .pairs
            .insert((src_name.to_string(), tgt_name.to_string()));
    }

    Ok(violations)
}

/// One aggregated violation group: the distinct node-name pairs and the
/// severity inherited from the violated (source) layer.
struct Violation {
    pairs: BTreeSet<(String, String)>,
    severity: Severity,
}

/// Reject malformed rules even when called directly without the engine.
pub(crate) fn compile_layers(layers: &[LayerConfig]) -> Result<Vec<GlobSet>, ArchguardError> {
    layers
        .iter()
        .map(|layer| {
            let mut builder = GlobSetBuilder::new();
            for pattern in &layer.contains {
                let glob = Glob::new(pattern).map_err(|err| {
                    ArchguardError::Config(format!("layers[{}].contains: {err}", layer.name))
                })?;
                builder.add(glob);
            }
            builder.build().map_err(|err| {
                ArchguardError::Config(format!("layers[{}].contains: {err}", layer.name))
            })
        })
        .collect()
}

/// Map every node id to its layer index: the first layer (slice order)
/// whose glob set matches the node name. Nodes no layer matches are
/// absent — unconstrained.
pub(crate) fn assign_layers(graph: &Graph, sets: &[GlobSet]) -> BTreeMap<NodeId, usize> {
    graph
        .nodes
        .iter()
        .filter_map(|node| {
            sets.iter()
                .position(|set| set.is_match(&node.name))
                .map(|layer| (node.id, layer))
        })
        .collect()
}

/// Layer of `id`, `None` when the node is unconstrained or unknown.
fn layer_of(assignment: &BTreeMap<NodeId, usize>, id: NodeId) -> Option<usize> {
    assignment.get(&id).copied()
}

/// Whether the source layer allows depending on the target layer.
fn may_depend_on(source: &LayerConfig, target_name: &str) -> bool {
    source.may_depend_on.iter().any(|name| name == target_name)
}

/// Kap. 11 Promotion: declared layers are `Error` gates, derived ones
/// `Info` hints.
fn severity_of(layer: &LayerConfig) -> Severity {
    match layer.origin {
        LayerOrigin::Declared => Severity::Error,
        LayerOrigin::Derived => Severity::Info,
    }
}

/// The first `MAX_PAIRS_PER_FINDING` pairs of the sorted set, flattened
/// into `nodes` as `src, tgt, src, tgt, …` (Kap. 5: names, never ids).
fn flatten_pairs(pairs: &BTreeSet<(String, String)>) -> Vec<String> {
    pairs
        .iter()
        .take(MAX_PAIRS_PER_FINDING)
        .flat_map(|(from, to)| [from.clone(), to.clone()])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{Edge, Node, NodeId, NodeKind};

    /// Hand-assembled graph of `File`-kind nodes: `nodes` gives names in
    /// *insertion* order (id = position), `edges` references nodes by
    /// name (same helper style as `scc.rs`, `cycles.rs`, `metrics.rs`).
    fn graph(nodes: &[&str], edges: &[(&str, &str)]) -> Graph {
        let id_of = |name: &str| -> NodeId {
            NodeId(
                nodes
                    .iter()
                    .position(|n| *n == name)
                    .expect("test: edge endpoint must be a listed node") as u32,
            )
        };
        Graph {
            review_kpis: None,
            semantic: None,
            schema_version: 1,
            root: "/abs".to_string(),
            nodes: nodes
                .iter()
                .enumerate()
                .map(|(i, n)| Node {
                    id: NodeId(i as u32),
                    name: (*n).to_string(),
                    kind: NodeKind::File,
                    parent: None,
                    lang: None,
                })
                .collect(),
            edges: edges
                .iter()
                .map(|(from, to)| Edge {
                    from: id_of(from),
                    to: id_of(to),
                    refs: Vec::new(),
                    count: None,
                })
                .collect(),
        }
    }

    /// A layer with the given name, globs, allowed targets and origin.
    fn layer(
        name: &str,
        contains: &[&str],
        may_depend_on: &[&str],
        origin: LayerOrigin,
    ) -> LayerConfig {
        LayerConfig {
            name: name.to_string(),
            contains: contains.iter().map(|g| (*g).to_string()).collect(),
            may_depend_on: may_depend_on.iter().map(|g| (*g).to_string()).collect(),
            origin,
        }
    }

    /// App → domain → core stack, all declared; `may_depend_on` allows
    /// exactly the downward steps.
    fn declared_stack() -> Vec<LayerConfig> {
        vec![
            layer("app", &["app/**"], &["domain"], LayerOrigin::Declared),
            layer("domain", &["domain/**"], &["core"], LayerOrigin::Declared),
            layer("core", &["core/**"], &[], LayerOrigin::Declared),
        ]
    }

    /// The first finding's `nodes`, which must be the only one.
    fn sole_finding(findings: &[Finding]) -> &Finding {
        assert_eq!(findings.len(), 1, "expected exactly one finding");
        &findings[0]
    }

    #[test]
    fn allowed_edges_yield_no_findings() {
        // app → domain → core, each step in the source layer's
        // may_depend_on; same-layer and self edges are never violations.
        let g = graph(
            &["app/a.ts", "domain/b.ts", "core/c.ts", "domain/d.ts"],
            &[
                ("app/a.ts", "domain/b.ts"),
                ("domain/b.ts", "core/c.ts"),
                ("domain/b.ts", "domain/d.ts"),
            ],
        );
        assert!(
            check_layers(&g, &declared_stack())
                .expect("valid rules")
                .is_empty()
        );
        // No layers at all: everything is unconstrained.
        assert!(check_layers(&g, &[]).expect("valid rules").is_empty());
    }

    #[test]
    fn forbidden_edge_on_declared_layer_is_error() {
        // core → app skips uphill; core is declared, so Error.
        let g = graph(&["app/a.ts", "core/c.ts"], &[("core/c.ts", "app/a.ts")]);
        let findings = check_layers(&g, &declared_stack()).expect("valid rules");
        let finding = sole_finding(&findings);

        assert_eq!(finding.kind, FindingKind::LayerViolation);
        assert_eq!(finding.id, "layer-1");
        assert_eq!(finding.severity, Severity::Error);
        assert_eq!(finding.nodes, ["core/c.ts", "app/a.ts"]);
        assert_eq!(finding.edge_count, 1);
        assert_eq!(finding.cycles_sampled, 0);
        assert!(finding.suggested_cuts.is_empty());
    }

    #[test]
    fn forbidden_edge_on_derived_layer_is_info() {
        // Same violation, but the violated (source) layer is derived:
        // Kap. 11 Promotion keeps it an Info hint, never a gate.
        let mut layers = declared_stack();
        layers[2].origin = LayerOrigin::Derived;
        let g = graph(&["app/a.ts", "core/c.ts"], &[("core/c.ts", "app/a.ts")]);
        let findings = check_layers(&g, &layers).expect("valid rules");
        let finding = sole_finding(&findings);

        assert_eq!(finding.severity, Severity::Info);
    }

    #[test]
    fn unassigned_nodes_are_unconstrained() {
        // `vendor/v.ts` matches no layer: both edge directions through it
        // stay allowed, while the real violation is still reported.
        let g = graph(
            &["app/a.ts", "core/c.ts", "vendor/v.ts"],
            &[
                ("vendor/v.ts", "app/a.ts"),
                ("core/c.ts", "vendor/v.ts"),
                ("core/c.ts", "app/a.ts"),
            ],
        );
        let findings = check_layers(&g, &declared_stack()).expect("valid rules");

        assert_eq!(findings.len(), 1, "only core → app may be reported");
        assert_eq!(findings[0].nodes, ["core/c.ts", "app/a.ts"]);
    }

    #[test]
    fn many_edges_in_one_layer_pair_aggregate_into_one_finding() {
        // Twelve distinct core → app pairs, one of them duplicated:
        // one finding listing the 10 lexicographically smallest pairs.
        let mut names: Vec<String> = (0..12).map(|i| format!("core/c{i:02}.ts")).collect();
        let mut edges: Vec<(&str, &str)> = names.iter().map(|c| (c.as_str(), "app/a.ts")).collect();
        edges.push(("core/c00.ts", "app/a.ts")); // duplicate
        let mut node_names: Vec<&str> = names.iter().map(String::as_str).collect();
        node_names.push("app/a.ts");
        let g = graph(&node_names, &edges);

        let findings = check_layers(&g, &declared_stack()).expect("valid rules");
        let finding = sole_finding(&findings);
        assert_eq!(finding.severity, Severity::Error);
        assert_eq!(finding.edge_count, 1); // aggregated, not summed
        let pairs: Vec<&str> = finding.nodes.iter().map(String::as_str).collect();
        assert_eq!(pairs.len(), 20); // 10 pairs cap
        names.sort();
        let expected: Vec<&str> = names[..10]
            .iter()
            .flat_map(|c| [c.as_str(), "app/a.ts"])
            .collect();
        assert_eq!(pairs, expected);

        // Deterministic (Kap. 12): same input, same output.
        assert_eq!(
            check_layers(&g, &declared_stack()).expect("valid rules"),
            vec![finding.clone()]
        );
    }

    #[test]
    fn first_matching_layer_wins_and_findings_are_ordered_by_layer_pair() {
        // "lib/shared.ts" is matched by both `lib/**` (first) and
        // `shared/**` (later) — first layer wins, so its may_depend_on
        // applies: lib → app is allowed, lib → core is not.
        let layers = vec![
            layer(
                "lib",
                &["lib/**", "shared/**"],
                &["app"],
                LayerOrigin::Declared,
            ),
            layer("app", &["app/**"], &[], LayerOrigin::Declared),
            layer("core", &["core/**"], &[], LayerOrigin::Derived),
        ];
        let g = graph(
            &["lib/shared.ts", "app/a.ts", "core/c.ts"],
            &[
                ("lib/shared.ts", "app/a.ts"),
                ("lib/shared.ts", "core/c.ts"),
                ("app/a.ts", "core/c.ts"),
            ],
        );
        let findings = check_layers(&g, &layers).expect("valid rules");

        // Two layer pairs violate: (lib, core) and (app, core); BTreeMap
        // order over layer indices numbers the ids, both source layers
        // decide severity individually.
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].id, "layer-1");
        assert_eq!(findings[0].nodes, ["lib/shared.ts", "core/c.ts"]);
        assert_eq!(findings[0].severity, Severity::Error); // lib is declared
        assert_eq!(findings[1].id, "layer-2");
        assert_eq!(findings[1].nodes, ["app/a.ts", "core/c.ts"]);
        assert_eq!(findings[1].severity, Severity::Error); // app is declared
    }

    #[test]
    fn severity_follows_the_violated_source_layer() {
        // One derived source layer (domain) and one declared one (core)
        // violating in parallel: Error and Info side by side.
        let mut layers = declared_stack();
        layers[1].origin = LayerOrigin::Derived; // domain derived
        let g = graph(
            &["app/a.ts", "domain/b.ts", "core/c.ts"],
            &[("domain/b.ts", "app/a.ts"), ("core/c.ts", "app/a.ts")],
        );
        let findings = check_layers(&g, &layers).expect("valid rules");

        assert_eq!(findings.len(), 2);
        // (domain→app) has the lower source index (1) than (core→app) (2).
        assert_eq!(findings[0].nodes, ["domain/b.ts", "app/a.ts"]);
        assert_eq!(findings[0].severity, Severity::Info); // domain derived
        assert_eq!(findings[1].nodes, ["core/c.ts", "app/a.ts"]);
        assert_eq!(findings[1].severity, Severity::Error); // core declared
    }

    #[test]
    fn invalid_glob_cannot_hide_a_forbidden_edge() {
        let mut layers = declared_stack();
        let g = graph(&["app/a.ts", "core/c.ts"], &[("core/c.ts", "app/a.ts")]);
        assert_eq!(check_layers(&g, &layers).unwrap().len(), 1);
        layers[2].contains = vec!["[".into()];
        let err = check_layers(&g, &layers).unwrap_err();
        assert!(err.to_string().contains("contains"));
    }
}
