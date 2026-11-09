//! Virtual refactoring simulation (T37, ARCHITECTURE.md Kap. 7
//! "Simulation" row): apply [`SimOp`]s to a *copy* of the graph, rerun the
//! analyses and diff findings and metrics — the effect is checked before
//! any code is touched.
//!
//! # The SimOp JSON contract (frozen)
//!
//! The ops are the universal format consumed by the CLI
//! `archguard simulate --ops` (T40), produced by the HTML viewer's
//! drag & drop export (T39) and exchanged with agents over MCP `simulate`.
//! The representation is serde's default *externally tagged* enum with
//! `#[serde(rename_all = "snake_case")]`; a `Vec<SimOp>` therefore round
//! trips as a JSON array of single-key objects:
//!
//! ```json
//! [
//!   {"cut_edge": {"from": "app/billing", "to": "domain/money"}},
//!   {"move_node": {"node": "domain/x", "new_parent": "app"}},
//!   {"merge": {"survivor": "app/a", "absorbed": "app/b"}}
//! ]
//! ```
//!
//! This exact shape is pinned by a test; changing it is a breaking change
//! to all three producers/consumers.
//!
//! # Op semantics
//!
//! All ops address nodes by their *name* (the normalized module path,
//! Kap. 5 — never internal ids) and are applied in the given order to a
//! clone of the graph; node ids survive every op, so edges follow renamed
//! nodes for free. An op whose name is unknown *at the time it is applied*
//! (including names destroyed by an earlier op, e.g. the absorbed node of
//! a [`SimOp::Merge`]) fails the whole call with
//! [`ArchguardError::Analysis`] — the partially modified clone is discarded,
//! so `apply_ops` is all-or-nothing.
//!
//! * [`SimOp::CutEdge`]: removes every edge whose from-endpoint is the
//!   `from` node or one of its containment-tree descendants and whose
//!   to-endpoint is the `to` node or one of its descendants. That is
//!   exactly the set of (file-level) edges the aggregated view shows as
//!   one `from → to` edge at any zoom level — at the canonical
//!   module zoom the endpoints simply are the files owned by the two
//!   modules. Removing zero edges is a no-op, not an error (the names
//!   exist; only the dependency does not).
//! * [`SimOp::MoveNode`]: re-parents the node under the existing module
//!   node `new_parent` (the parent must exist, be a `Module`, and be
//!   neither the node itself nor one of its descendants) and rewrites the
//!   names of the whole subtree: `node` takes `new_parent` as its prefix
//!   and keeps its base name (`domain/x` → `app` becomes `app/x`), every
//!   descendant keeps its path below the new name. A rename that would
//!   collide with an existing node outside the moved subtree fails —
//!   node names stay unique.
//! * [`SimOp::Merge`]: re-parents all children of `absorbed` under
//!   `survivor`, renames the whole subtree of `absorbed` accordingly,
//!   redirects every edge touching `absorbed` to `survivor` (edges
//!   redirected onto themselves are dropped, parallel edges merged with
//!   their refs concatenated, sorted by `(file, line, raw)` and
//!   deduplicated — the `aggregate` convention), and removes the
//!   `absorbed` node. `survivor` must not sit inside the subtree of
//!   `absorbed`; the merge must not create duplicate names.
//!
//! # [`simulate`]
//!
//! `simulate` applies the ops to a clone, aggregates both the untouched
//! and the changed graph onto the *same* view (the [`Cut`], with its node
//! names translated through the ops so the after-graph keeps the before
//! zoom level), and runs SCC detection, the layer check and the Lakos
//! metrics on both views. Findings are compared by source-node membership
//! for cycles and by the violated source/target layer pair for layer rules.
//! Display ids (`scc-N`, `layer-N`) are positional and may be renumbered:
//! resolved findings return their before ids, introduced findings their
//! after ids. A reused display id can therefore appear in both lists when
//! its underlying finding changed. Renaming alone preserves cycle identity.
//! Everything is deterministic (Kap. 12): `BTreeSet`/`BTreeMap`
//! throughout, sorted results.

use std::collections::{BTreeMap, BTreeSet};

use archguard_core::{ArchguardError, Cut, Graph, LayerConfig, NodeId, NodeKind, Ref, aggregate};
use serde::{Deserialize, Serialize};

use crate::lakos::lakos_metrics;
use crate::layers::check_layers;
use crate::scc::find_sccs;

/// One virtual refactoring step — the frozen JSON contract of Kap. 7
/// (see the module docs for the exact serialized shape and the op
/// semantics). Nodes are addressed by name, never by internal ids.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SimOp {
    /// Remove the dependency `from → to` (all file-level edges whose
    /// endpoints aggregate to that module edge; see the module docs).
    CutEdge { from: String, to: String },
    /// Re-parent `node` (with its whole subtree) under `new_parent`.
    MoveNode { node: String, new_parent: String },
    /// Re-parent all children of `absorbed` under `survivor` and drop
    /// the `absorbed` node, redirecting its edges.
    Merge { survivor: String, absorbed: String },
}

/// What a simulation changed on the analysed view (Kap. 7: "Findings/
/// Metriken diffen"). Finding lists carry the respective before/after
/// detector ids (`scc-N`, `layer-N`) and are sorted; metrics come from the same
/// detectors as in a normal run ([`crate::lakos::lakos_metrics`],
/// [`crate::scc::find_sccs`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimDelta {
    /// Ids of findings present before the ops and gone afterwards.
    pub resolved_findings: Vec<String>,
    /// Ids of findings that appeared only after the ops.
    pub introduced_findings: Vec<String>,
    /// Lakos ACD of the aggregated view before the ops.
    pub acd_before: f64,
    /// Lakos ACD of the aggregated view after the ops.
    pub acd_after: f64,
    /// Number of cyclic SCCs on the view before the ops.
    pub scc_count_before: u32,
    /// Number of cyclic SCCs on the view after the ops.
    pub scc_count_after: u32,
}

/// Apply `ops` to a clone of `g`, in the given order, and return the
/// changed graph. See the module docs for the exact semantics of each op
/// and for the all-or-nothing error behaviour: an op naming an unknown
/// node — including one destroyed by an earlier op — fails the whole
/// call with [`ArchguardError::Analysis`].
pub fn apply_ops(g: &Graph, ops: &[SimOp]) -> Result<Graph, ArchguardError> {
    let mut work = g.clone();
    for op in ops {
        apply_op(&mut work, op)?;
    }
    Ok(work)
}

/// Simulate `ops` on a clone of `g` and diff the analyses of the
/// aggregated views before and after. The after-view uses the *same*
/// zoom level: the visible names of `cut` are translated through the ops
/// (a moved module keeps its place in the view under its new name, a
/// merged one appears as its survivor), so the two views are comparable.
/// The ops are validated against the full graph *before* aggregation;
/// invalid ops fail with [`ArchguardError::Analysis`] exactly as in
/// [`apply_ops`].
pub fn simulate(
    g: &Graph,
    cut: &Cut,
    ops: &[SimOp],
    layers: &[LayerConfig],
) -> Result<SimDelta, ArchguardError> {
    let after = apply_ops(g, ops)?;
    let before_view = aggregate(g, cut);
    let after_view = aggregate(&after, &translate_cut(g, &after, cut, ops));

    let before = snapshot(&before_view, g, layers)?;
    let after = snapshot(&after_view, &after, layers)?;
    let mut resolved_findings: Vec<_> = before
        .findings
        .iter()
        .filter(|(identity, _)| !after.findings.contains_key(*identity))
        .map(|(_, id)| id.clone())
        .collect();
    let mut introduced_findings: Vec<_> = after
        .findings
        .iter()
        .filter(|(identity, _)| !before.findings.contains_key(*identity))
        .map(|(_, id)| id.clone())
        .collect();
    resolved_findings.sort();
    introduced_findings.sort();
    Ok(SimDelta {
        resolved_findings,
        introduced_findings,
        acd_before: before.acd,
        acd_after: after.acd,
        scc_count_before: before.scc_count,
        scc_count_after: after.scc_count,
    })
}

// ------------------------------------------------------------------ ops

/// Apply a single op in place.
fn apply_op(work: &mut Graph, op: &SimOp) -> Result<(), ArchguardError> {
    match op {
        SimOp::CutEdge { from, to } => cut_edge(work, from, to),
        SimOp::MoveNode { node, new_parent } => move_node(work, node, new_parent),
        SimOp::Merge { survivor, absorbed } => merge(work, survivor, absorbed),
    }
}

/// Remove every edge with from-endpoint in `subtree(from)` and
/// to-endpoint in `subtree(to)` (module names address the subtree they
/// own; see the module docs).
fn cut_edge(work: &mut Graph, from: &str, to: &str) -> Result<(), ArchguardError> {
    let from_id = node_id(work, from)?;
    let to_id = node_id(work, to)?;
    let from_sub = subtree(work, from_id);
    let to_sub = subtree(work, to_id);
    work.edges
        .retain(|e| !(from_sub.contains(&e.from) && to_sub.contains(&e.to)));
    Ok(())
}

/// Re-parent `node` under `new_parent` and rewrite the subtree names.
fn move_node(work: &mut Graph, node: &str, new_parent: &str) -> Result<(), ArchguardError> {
    let moved_id = node_id(work, node)?;
    let parent_id = node_id(work, new_parent)?;
    if lookup(work, parent_id).kind != NodeKind::Module {
        return Err(ArchguardError::Analysis(format!(
            "simulate: move target \"{new_parent}\" is not a module"
        )));
    }
    let sub = subtree(work, moved_id);
    if sub.contains(&parent_id) {
        return Err(ArchguardError::Analysis(format!(
            "simulate: cannot move \"{node}\" into itself or its own subtree"
        )));
    }
    let parent_name = work
        .node_name(parent_id)
        .expect("moved_id() just resolved it")
        .to_string();
    // The moved node keeps its base name below the new parent:
    // `domain/x` → `app` becomes `app/x`, subtree included.
    let base = node.rsplit('/').next().unwrap_or(node).to_string();
    let new_base = format!("{parent_name}/{base}");
    rename_subtree(work, moved_id, &new_base, true, &sub)?;
    if let Some(moved) = work.nodes.iter_mut().find(|n| n.id == moved_id) {
        moved.parent = Some(parent_id);
    }
    Ok(())
}

/// Re-parent the children of `absorbed` under `survivor`, redirect its
/// edges and remove it. See the module docs.
fn merge(work: &mut Graph, survivor: &str, absorbed: &str) -> Result<(), ArchguardError> {
    let survivor_id = node_id(work, survivor)?;
    let absorbed_id = node_id(work, absorbed)?;
    if survivor_id == absorbed_id {
        return Err(ArchguardError::Analysis(
            "simulate: merge needs two different nodes".to_string(),
        ));
    }
    let sub = subtree(work, absorbed_id);
    if sub.contains(&survivor_id) {
        return Err(ArchguardError::Analysis(format!(
            "simulate: merge target \"{survivor}\" lies inside the subtree of \"{absorbed}\""
        )));
    }
    let survivor_name = work
        .node_name(survivor_id)
        .expect("node_id() just resolved it")
        .to_string();

    // Rename the surviving members of the absorbed subtree under the
    // survivor's name (the absorbed node itself disappears).
    rename_subtree(work, absorbed_id, &survivor_name, false, &sub)?;
    // The direct children move from the absorbed node to the survivor;
    // deeper descendants keep their parent and only follow the rename.
    for node in &mut work.nodes {
        if node.parent == Some(absorbed_id) {
            node.parent = Some(survivor_id);
        }
    }

    // Redirect edges touching the absorbed node onto the survivor:
    // self-edges are dropped, parallel edges merged with refs sorted by
    // (file, line, raw), deduplicated (the `aggregate` convention), and
    // the merged block re-emitted in (from, to) order after the
    // untouched edges — deterministic either way.
    let mut kept = Vec::new();
    let mut redirected: BTreeMap<(u32, u32), (Option<u64>, Vec<Ref>)> = BTreeMap::new();
    for edge in std::mem::take(&mut work.edges) {
        let from = if edge.from == absorbed_id {
            survivor_id
        } else {
            edge.from
        };
        let to = if edge.to == absorbed_id {
            survivor_id
        } else {
            edge.to
        };
        if from == to {
            continue;
        }
        if edge.from == absorbed_id || edge.to == absorbed_id {
            let slot = redirected
                .entry((from.0, to.0))
                .or_insert_with(|| (Some(0u64), Vec::new()));
            // F12: the total is the sum of the contributors' counts —
            // never recounted from the (possibly capped) sample refs —
            // and `None` if any contributor lacks an exact total.
            slot.0 = match (slot.0, edge.count) {
                (Some(sum), Some(count)) => Some(sum + count),
                _ => None,
            };
            slot.1.extend(edge.refs);
        } else {
            kept.push(edge);
        }
    }
    kept.extend(
        redirected
            .into_iter()
            .map(|((from, to), (count, mut refs))| {
                refs.sort_by(|a, b| (&a.file, a.line, &a.raw).cmp(&(&b.file, b.line, &b.raw)));
                refs.dedup();
                archguard_core::Edge {
                    from: NodeId(from),
                    to: NodeId(to),
                    refs,
                    count,
                }
            }),
    );
    work.edges = kept;
    work.nodes.retain(|n| n.id != absorbed_id);
    Ok(())
}

// ---------------------------------------------------------------- helpers

/// Id of the node named `name`; `Err` if no node carries that name.
fn node_id(g: &Graph, name: &str) -> Result<NodeId, ArchguardError> {
    g.nodes
        .iter()
        .find(|n| n.name == name)
        .map(|n| n.id)
        .ok_or_else(|| ArchguardError::Analysis(format!("simulate: no node named \"{name}\"")))
}

/// The node with `id`; panics only for ids that were resolved from `g`
/// itself (via [`node_id`] or a tree walk over `g`).
fn lookup(g: &Graph, id: NodeId) -> &archguard_core::Node {
    g.nodes
        .iter()
        .find(|n| n.id == id)
        .expect("id was resolved from this graph's nodes")
}

/// `root` plus all its containment-tree descendants, ids in a
/// `BTreeSet` for deterministic iteration. The children are indexed in
/// one pass; asking the graph per visited node would scan it every time.
fn subtree(g: &Graph, root: NodeId) -> BTreeSet<NodeId> {
    let mut children = BTreeMap::<NodeId, Vec<NodeId>>::new();
    for node in &g.nodes {
        if let Some(parent) = node.parent {
            children.entry(parent).or_default().push(node.id);
        }
    }
    let mut set = BTreeSet::from([root]);
    let mut frontier = vec![root];
    while let Some(id) = frontier.pop() {
        for &child in children.get(&id).into_iter().flatten() {
            if set.insert(child) {
                frontier.push(child);
            }
        }
    }
    set
}

/// Rewrite the names of `sub` (a [`subtree`] of `root`) so that the
/// subtree lives under `new_base` instead of under the old name of
/// `root`: descendants keep their path below `new_base`, and `root`
/// itself becomes `new_base` unless `keep_root` is false (a merge drops
/// the root node right after renaming its members). Fails with a
/// collision error if a new name already belongs to a node outside
/// `sub`. Node ids (and hence edges) are untouched.
fn rename_subtree(
    work: &mut Graph,
    root: NodeId,
    new_base: &str,
    keep_root: bool,
    sub: &BTreeSet<NodeId>,
) -> Result<(), ArchguardError> {
    let old_base = lookup(work, root).name.clone();
    let old_prefix = format!("{old_base}/");
    let retained: BTreeSet<&str> = work
        .nodes
        .iter()
        .filter(|n| !sub.contains(&n.id))
        .map(|n| n.name.as_str())
        .collect();

    let new_names: BTreeMap<NodeId, String> = work
        .nodes
        .iter()
        .filter(|n| sub.contains(&n.id) && (keep_root || n.id != root))
        .map(|n| {
            let new_name = if n.id == root {
                new_base.to_string()
            } else if let Some(rest) = n.name.strip_prefix(&old_prefix) {
                format!("{new_base}/{rest}")
            } else {
                // Malformed containment data: the name does not follow
                // the tree — keep at least the base name.
                let base = n.name.rsplit('/').next().unwrap_or(&n.name);
                format!("{new_base}/{base}")
            };
            (n.id, new_name)
        })
        .collect();

    if let Some((_, clash)) = new_names
        .iter()
        .find(|(_, new_name)| retained.contains(new_name.as_str()))
    {
        return Err(ArchguardError::Analysis(format!(
            "simulate: rename to \"{clash}\" would collide with an existing node"
        )));
    }
    for node in &mut work.nodes {
        if let Some(new_name) = new_names.get(&node.id) {
            node.name = new_name.clone();
        }
    }
    Ok(())
}

/// Project the names visible in `cut` (read from `before`, the graph
/// `cut` addresses) through `ops` onto `after`: a renamed name follows
/// its node, the name of a merged node continues as the survivor's
/// name. The result is the [`Cut`] for the after-graph that keeps the
/// before zoom level.
fn translate_cut(before: &Graph, after: &Graph, cut: &Cut, ops: &[SimOp]) -> Cut {
    let mut names: BTreeSet<String> = cut
        .nodes()
        .iter()
        .filter_map(|id| before.node_name(*id).map(str::to_string))
        .collect();
    for op in ops {
        // Mirror the exact rename of `apply_ops`: a moved node lands
        // under its new parent keeping its base name, a merged node
        // continues as the survivor.
        let (old, new) = match op {
            SimOp::CutEdge { .. } => continue,
            SimOp::MoveNode { node, new_parent } => {
                let base = node.rsplit('/').next().unwrap_or(node);
                (node.clone(), format!("{new_parent}/{base}"))
            }
            SimOp::Merge { survivor, absorbed } => (absorbed.clone(), survivor.clone()),
        };
        let old_prefix = format!("{old}/");
        names = names
            .iter()
            .map(|name| {
                if name.as_str() == old {
                    new.to_string()
                } else if let Some(rest) = name.strip_prefix(&old_prefix) {
                    format!("{new}/{rest}")
                } else {
                    name.clone()
                }
            })
            .collect();
    }
    Cut::new(
        after
            .nodes
            .iter()
            // `names` are the *final* names; `after` carries them.
            .filter(|n| names.contains(&n.name))
            .map(|n| n.id),
    )
}

/// Source ids survive moves; the aggregated view's dense ids do not.
/// Layer findings represent a rule pair, regardless of sampled evidence.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum FindingIdentity {
    Cycle(Vec<NodeId>),
    Layer(usize, usize),
}

/// Comparable identities paired with the view's original display ids.
struct Snapshot {
    findings: BTreeMap<FindingIdentity, String>,
    scc_count: u32,
    acd: f64,
}

/// Run the view through the existing detectors (no new id schemes).
fn snapshot(
    view: &Graph,
    source: &Graph,
    layers: &[LayerConfig],
) -> Result<Snapshot, ArchguardError> {
    let sccs = find_sccs(view);
    let source_ids: BTreeMap<_, _> = source
        .nodes
        .iter()
        .map(|n| (n.name.as_str(), n.id))
        .collect();
    let mut findings = BTreeMap::new();
    for scc in &sccs {
        let mut members: Vec<_> = scc
            .node_ids
            .iter()
            .map(|id| {
                let name = view.node_name(*id).expect("SCC member belongs to view");
                source_ids[name]
            })
            .collect();
        members.sort();
        findings.insert(FindingIdentity::Cycle(members), scc.id.clone());
    }
    let assignment = crate::layers::assign_layers(view, &crate::layers::compile_layers(layers)?);
    let view_ids: BTreeMap<_, _> = view.nodes.iter().map(|n| (n.name.as_str(), n.id)).collect();
    for finding in check_layers(view, layers)? {
        // Every layer finding contains at least one real violating pair.
        // All its evidence pairs share the same source/target layer indices.
        let from = assignment[&view_ids[finding.nodes[0].as_str()]];
        let to = assignment[&view_ids[finding.nodes[1].as_str()]];
        findings.insert(FindingIdentity::Layer(from, to), finding.id);
    }
    let (_ccd, acd, _nccd) = lakos_metrics(view);
    Ok(Snapshot {
        findings,
        scc_count: sccs.len() as u32,
        acd,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{Edge, LayerOrigin, Node, NodeKind, Ref, SuggestedCut};

    use crate::cuts::suggest_cuts;
    use crate::cycles::sample_cycles;

    /// Hand-assembled graph: `nodes` are `(name, kind, parent-name)` in
    /// insertion order (id = position), `edges` reference nodes by name
    /// and carry one deterministic ref each.
    fn build(nodes: &[(&str, NodeKind, Option<&str>)], edges: &[(&str, &str)]) -> Graph {
        let id_of = |name: &str| -> NodeId {
            NodeId(
                nodes
                    .iter()
                    .position(|(n, _, _)| *n == name)
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
                .map(|(index, (name, kind, parent))| Node {
                    id: NodeId(index as u32),
                    name: (*name).to_string(),
                    kind: *kind,
                    parent: parent.map(id_of),
                    lang: None,
                })
                .collect(),
            edges: edges
                .iter()
                .map(|(from, to)| Edge {
                    from: id_of(from),
                    to: id_of(to),
                    refs: vec![Ref {
                        file: (*from).to_string(),
                        line: 1,
                        raw: format!("import {to}"),
                    }],
                    // Builder-shaped: one ref, one total.
                    count: Some(1),
                })
                .collect(),
        }
    }

    /// The canonical module-level cycle: two modules owning one file
    /// each, imported by each other.
    fn cycle_graph() -> Graph {
        build(
            &[
                ("app", NodeKind::Module, None),
                ("app/billing", NodeKind::Module, Some("app")),
                (
                    "app/billing/invoice.ts",
                    NodeKind::File,
                    Some("app/billing"),
                ),
                ("domain", NodeKind::Module, None),
                ("domain/money", NodeKind::Module, Some("domain")),
                (
                    "domain/money/money.ts",
                    NodeKind::File,
                    Some("domain/money"),
                ),
            ],
            &[
                ("app/billing/invoice.ts", "domain/money/money.ts"),
                ("domain/money/money.ts", "app/billing/invoice.ts"),
            ],
        )
    }

    /// The two-layer DAG: `app/ui` on top, `domain/x` and `domain/y`
    /// below, with `domain/y → domain/x` (same layer, allowed).
    fn layered_graph() -> Graph {
        build(
            &[
                ("app", NodeKind::Module, None),
                ("app/ui", NodeKind::Module, Some("app")),
                ("app/ui/main.ts", NodeKind::File, Some("app/ui")),
                ("domain", NodeKind::Module, None),
                ("domain/x", NodeKind::Module, Some("domain")),
                ("domain/x/x.ts", NodeKind::File, Some("domain/x")),
                ("domain/y", NodeKind::Module, Some("domain")),
                ("domain/y/y.ts", NodeKind::File, Some("domain/y")),
            ],
            &[("domain/y/y.ts", "domain/x/x.ts")],
        )
    }

    fn layers() -> Vec<LayerConfig> {
        vec![
            LayerConfig {
                name: "app".to_string(),
                contains: vec!["app/**".to_string()],
                may_depend_on: vec!["domain".to_string()],
                origin: LayerOrigin::Declared,
            },
            LayerConfig {
                name: "domain".to_string(),
                contains: vec!["domain/**".to_string()],
                may_depend_on: Vec::new(),
                origin: LayerOrigin::Declared,
            },
        ]
    }

    fn module_names(g: &Graph) -> Vec<String> {
        let mut names: Vec<String> = g
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Module)
            .map(|n| n.name.clone())
            .collect();
        names.sort();
        names
    }

    // ------------------------------------------------------------ contract

    #[test]
    fn sim_op_json_contract_is_pinned() {
        // The frozen wire format (Kap. 7): externally tagged, snake_case.
        // T39 (HTML viewer) and T40 (CLI --ops) produce/consume exactly
        // this; any change here breaks all three consumers.
        let ops = vec![
            SimOp::CutEdge {
                from: "app/billing".to_string(),
                to: "domain/money".to_string(),
            },
            SimOp::MoveNode {
                node: "domain/x".to_string(),
                new_parent: "app".to_string(),
            },
            SimOp::Merge {
                survivor: "app/a".to_string(),
                absorbed: "app/b".to_string(),
            },
        ];
        let json = serde_json::to_string(&ops).expect("serialization");
        assert_eq!(
            json,
            r#"[{"cut_edge":{"from":"app/billing","to":"domain/money"}},{"move_node":{"node":"domain/x","new_parent":"app"}},{"merge":{"survivor":"app/a","absorbed":"app/b"}}]"#
        );
        let back: Vec<SimOp> = serde_json::from_str(&json).expect("deserialization");
        assert_eq!(back, ops);
    }

    #[test]
    fn sim_delta_serializes_with_documented_fields() {
        let delta = SimDelta {
            resolved_findings: vec!["scc-1".to_string()],
            introduced_findings: Vec::new(),
            acd_before: 2.0,
            acd_after: 1.5,
            scc_count_before: 1,
            scc_count_after: 0,
        };
        let value = serde_json::to_value(&delta).expect("serialization");
        assert_eq!(value["resolved_findings"][0], "scc-1");
        assert_eq!(value["acd_before"], 2.0);
        assert_eq!(value["scc_count_after"], 0);
        let back: SimDelta = serde_json::from_value(value).expect("deserialization");
        assert_eq!(back, delta);
    }

    // -------------------------------------------------------------- cycles

    #[test]
    fn removing_first_cycle_does_not_resolve_the_renumbered_survivor() {
        let g = build(
            &[
                ("a", NodeKind::File, None),
                ("b", NodeKind::File, None),
                ("c", NodeKind::File, None),
                ("d", NodeKind::File, None),
            ],
            &[("a", "b"), ("b", "a"), ("c", "d"), ("d", "c")],
        );
        let delta = simulate(
            &g,
            &Cut::at_depth(&g, 0),
            &[SimOp::CutEdge {
                from: "a".into(),
                to: "b".into(),
            }],
            &[],
        )
        .unwrap();
        assert_eq!(delta.resolved_findings, ["scc-1"]);
        assert!(delta.introduced_findings.is_empty());
        assert_eq!(delta.scc_count_after, 1);
    }

    #[test]
    fn changed_cycle_members_are_reported_even_when_the_display_id_is_reused() {
        let g = build(
            &[
                ("a", NodeKind::File, None),
                ("b", NodeKind::File, None),
                ("c", NodeKind::File, None),
            ],
            &[("a", "b"), ("b", "a"), ("b", "c"), ("c", "a")],
        );
        let delta = simulate(
            &g,
            &Cut::at_depth(&g, 0),
            &[SimOp::CutEdge {
                from: "c".into(),
                to: "a".into(),
            }],
            &[],
        )
        .unwrap();
        assert_eq!(delta.resolved_findings, ["scc-1"]);
        assert_eq!(delta.introduced_findings, ["scc-1"]);
        assert_eq!(delta.scc_count_before, delta.scc_count_after);
    }

    #[test]
    fn renaming_cycle_members_preserves_source_identity_despite_view_reordering() {
        let g = build(
            &[
                ("a", NodeKind::File, None),
                ("b", NodeKind::File, None),
                ("z", NodeKind::Module, None),
            ],
            &[("a", "b"), ("b", "a")],
        );
        let delta = simulate(
            &g,
            &Cut::new([NodeId(0), NodeId(1)]),
            &[SimOp::MoveNode {
                node: "a".into(),
                new_parent: "z".into(),
            }],
            &[],
        )
        .unwrap();
        assert!(delta.resolved_findings.is_empty());
        assert!(delta.introduced_findings.is_empty());
        assert_eq!(delta.scc_count_after, 1);
    }

    #[test]
    fn layer_group_identity_survives_renumbering_and_changing_evidence() {
        let g = build(
            &[
                ("a/x", NodeKind::File, None),
                ("b/y", NodeKind::File, None),
                ("b/z", NodeKind::File, None),
                ("c/w", NodeKind::File, None),
            ],
            &[("a/x", "b/y"), ("b/y", "c/w"), ("b/z", "c/w")],
        );
        let layers: Vec<_> = ["a", "b", "c"]
            .into_iter()
            .map(|name| LayerConfig {
                name: name.into(),
                contains: vec![format!("{name}/**")],
                may_depend_on: vec![],
                origin: LayerOrigin::Declared,
            })
            .collect();
        let delta = simulate(
            &g,
            &Cut::at_depth(&g, 0),
            &[
                SimOp::CutEdge {
                    from: "a/x".into(),
                    to: "b/y".into(),
                },
                SimOp::CutEdge {
                    from: "b/y".into(),
                    to: "c/w".into(),
                },
            ],
            &layers,
        )
        .unwrap();
        assert_eq!(delta.resolved_findings, ["layer-1"]);
        assert!(delta.introduced_findings.is_empty());
    }

    #[test]
    fn cutting_the_top_suggested_edge_resolves_the_cycle() {
        let g = cycle_graph();
        let cut = Cut::new([NodeId(1), NodeId(4)]);

        // The top suggested cut of the module-view cycle, as a consumer
        // (CLI/MCP/viewer) would read it from the cycle finding.
        let view = aggregate(&g, &cut);
        let sccs = find_sccs(&view);
        assert_eq!(sccs.len(), 1);
        let cycles = sample_cycles(&view, &sccs[0], 100, 20);
        let SuggestedCut { from, to, .. } = suggest_cuts(&view, &sccs[0], &cycles, 1)
            .into_iter()
            .next()
            .expect("one suggestion");

        let ops = vec![SimOp::CutEdge { from, to }];
        let delta = simulate(&g, &cut, &ops, &[]).expect("simulation");
        assert_eq!(delta.resolved_findings, vec!["scc-1".to_string()]);
        assert!(delta.introduced_findings.is_empty());
        assert_eq!(delta.scc_count_before, 1);
        assert_eq!(delta.scc_count_after, 0);
        assert!(delta.acd_after < delta.acd_before);
    }

    #[test]
    fn cut_edge_removes_the_file_edges_under_the_named_modules() {
        let g = cycle_graph();
        let before = g.edges.len();
        // Cutting one direction of the cycle.
        let after = apply_ops(
            &g,
            &[SimOp::CutEdge {
                from: "app/billing".to_string(),
                to: "domain/money".to_string(),
            }],
        )
        .expect("cut");
        assert_eq!(after.edges.len(), before - 1);
        // The surviving edge is the one running domain → app.
        let surviving = &after.edges[0];
        assert_eq!(
            (g.node_name(surviving.from), g.node_name(surviving.to)),
            (
                Some("domain/money/money.ts"),
                Some("app/billing/invoice.ts")
            )
        );
    }

    #[test]
    fn cut_edge_at_a_coarse_name_covers_nested_subtrees() {
        // The aggregated view shows one `app → domain` edge for all file
        // edges between the subtrees; cutting the coarse name removes
        // exactly those (and only those).
        let g = cycle_graph();
        let after = apply_ops(
            &g,
            &[SimOp::CutEdge {
                from: "app".to_string(),
                to: "domain".to_string(),
            }],
        )
        .expect("cut");
        // Only the app → domain direction is cut; the reverse edge
        // (money.ts → invoice.ts) has its from-side outside `app`.
        assert_eq!(after.edges.len(), 1);
        assert_eq!(
            (
                g.node_name(after.edges[0].from),
                g.node_name(after.edges[0].to)
            ),
            (
                Some("domain/money/money.ts"),
                Some("app/billing/invoice.ts")
            )
        );
    }

    #[test]
    fn cut_edge_with_no_matching_edges_is_a_no_op() {
        let g = layered_graph();
        let after = apply_ops(
            &g,
            &[SimOp::CutEdge {
                from: "app/ui".to_string(),
                to: "domain/x".to_string(),
            }],
        )
        .expect("no such edge is fine");
        assert_eq!(after, g);
    }

    // --------------------------------------------------------------- moves

    #[test]
    fn moving_a_module_across_layers_introduces_a_violation() {
        let g = layered_graph();
        let cut = Cut::new([NodeId(1), NodeId(4), NodeId(6)]);
        let ops = vec![SimOp::MoveNode {
            node: "domain/x".to_string(),
            new_parent: "app".to_string(),
        }];
        let delta = simulate(&g, &cut, &ops, &layers()).expect("simulation");
        assert!(delta.resolved_findings.is_empty());
        assert_eq!(delta.introduced_findings, vec!["layer-1".to_string()]);
        assert_eq!(delta.scc_count_before, 0);
        assert_eq!(delta.scc_count_after, 0);

        // The after-view really contains the moved module under its new
        // name: the translated cut keeps the zoom level comparable.
        let after = apply_ops(&g, &ops).expect("move");
        let after_view = aggregate(&after, &translate_cut(&g, &after, &cut, &ops));
        let mut view_names: Vec<&str> = after_view.nodes.iter().map(|n| n.name.as_str()).collect();
        view_names.sort();
        assert_eq!(view_names, ["app/ui", "app/x", "domain/y"]);
    }

    #[test]
    fn move_renames_the_whole_subtree_and_keeps_ids() {
        let g = layered_graph();
        let after = apply_ops(
            &g,
            &[SimOp::MoveNode {
                node: "domain/x".to_string(),
                new_parent: "app".to_string(),
            }],
        )
        .expect("move");
        let moved = after
            .nodes
            .iter()
            .find(|n| n.name == "app/x")
            .expect("moved module");
        assert_eq!(moved.id, NodeId(4)); // id survives the rename
        assert_eq!(moved.parent, Some(NodeId(0))); // "app"
        let file = after
            .nodes
            .iter()
            .find(|n| n.name == "app/x/x.ts")
            .expect("renamed file");
        assert_eq!(file.id, NodeId(5));
        assert_eq!(file.parent, Some(NodeId(4))); // still below the module
        // The edge follows automatically (ids unchanged).
        assert_eq!(after.edges[0].to, NodeId(5));
    }

    // -------------------------------------------------------------- merges

    #[test]
    fn merge_reduces_node_count_and_reparents_children() {
        let g = build(
            &[
                ("app", NodeKind::Module, None),
                ("app/a", NodeKind::Module, Some("app")),
                ("app/a/a.ts", NodeKind::File, Some("app/a")),
                ("app/b", NodeKind::Module, Some("app")),
                ("app/b/b.ts", NodeKind::File, Some("app/b")),
            ],
            &[("app/a/a.ts", "app/b/b.ts"), ("app/b", "app/a/a.ts")],
        );
        let ops = vec![SimOp::Merge {
            survivor: "app/a".to_string(),
            absorbed: "app/b".to_string(),
        }];
        let after = apply_ops(&g, &ops).expect("merge");
        assert!(after.nodes.len() < g.nodes.len());
        assert_eq!(module_names(&after), ["app", "app/a"]);

        // Children of the absorbed node moved under the survivor.
        let child = after
            .nodes
            .iter()
            .find(|n| n.name == "app/a/b.ts")
            .expect("reparented child");
        assert_eq!(child.parent, Some(NodeId(1))); // "app/a"
        // The edge that touched the absorbed module now touches the
        // survivor; the untouched file edge stayed in place.
        let redirected = after
            .edges
            .iter()
            .find(|e| e.from == NodeId(1) && e.to == NodeId(2))
            .expect("redirected edge");
        assert_eq!(redirected.refs.len(), 1);
        assert!(
            after
                .edges
                .iter()
                .all(|e| e.from != NodeId(3) && e.to != NodeId(3))
        );
    }

    #[test]
    fn merge_merges_parallel_edges_and_deduplicates_refs() {
        // a.ts → b.ts and b-module → a.ts; after merging b into a, the
        // redirected edge coincides with … let's also touch a-module so
        // two edges collapse onto one (from, to) pair.
        let mut g = build(
            &[
                ("app", NodeKind::Module, None),
                ("app/a", NodeKind::Module, Some("app")),
                ("app/a/a.ts", NodeKind::File, Some("app/a")),
                ("app/b", NodeKind::Module, Some("app")),
            ],
            &[("app/b", "app/a/a.ts"), ("app/b", "app/a/a.ts")],
        );
        g.edges[1].refs.push(Ref {
            file: "extra".to_string(),
            line: 2,
            raw: "second".to_string(),
        });
        // F12: this contributor actually saw 50 import references — its
        // sample here holds only two of them. The merge must carry the
        // totals (1 + 50), never recount the deduplicated sample.
        g.edges[1].count = Some(50);
        let after = apply_ops(
            &g,
            &[SimOp::Merge {
                survivor: "app/a".to_string(),
                absorbed: "app/b".to_string(),
            }],
        )
        .expect("merge");
        assert_eq!(after.edges.len(), 1);
        assert_eq!(after.edges[0].from, NodeId(1));
        assert_eq!(after.edges[0].to, NodeId(2));
        // Refs concatenated, sorted by (file, line, raw), deduplicated:
        // both edges carry the same builder ref for app/b → app/a/a.ts,
        // so the duplicates collapse and only the extra ref survives
        // alongside one copy of it.
        let files: Vec<&str> = after.edges[0]
            .refs
            .iter()
            .map(|r| r.file.as_str())
            .collect();
        assert_eq!(files, ["app/b", "extra"]);
        assert_eq!(after.edges[0].count, Some(51));
        assert_eq!(after.edges[0].weight(), 51);
    }

    #[test]
    fn merge_keeps_the_after_view_at_the_same_zoom_level() {
        // The caller's cut still references the merged-away module id;
        // the translated cut must map it to the survivor instead of
        // silently dropping the subtree from the view.
        let g = build(
            &[
                ("app", NodeKind::Module, None),
                ("app/a", NodeKind::Module, Some("app")),
                ("app/a/a.ts", NodeKind::File, Some("app/a")),
                ("app/b", NodeKind::Module, Some("app")),
                ("app/b/b.ts", NodeKind::File, Some("app/b")),
            ],
            &[("app/b/b.ts", "app/a/a.ts")],
        );
        let cut = Cut::new([NodeId(1), NodeId(3)]);
        let delta = simulate(
            &g,
            &cut,
            &[SimOp::Merge {
                survivor: "app/a".to_string(),
                absorbed: "app/b".to_string(),
            }],
            &[],
        )
        .expect("simulation with stale cut id");
        // The single edge survives the merge (b's file now belongs to a)
        // and becomes a self-edge of `app/a` — dropped by aggregation,
        // so no cycle appears and nothing is introduced. The ACD drops
        // from 1.5 (two view nodes, one edge) to 1.0 (a lone node).
        assert_eq!(delta.scc_count_before, 0);
        assert_eq!(delta.scc_count_after, 0);
        assert!(delta.resolved_findings.is_empty());
        assert!(delta.introduced_findings.is_empty());
        assert_eq!(delta.acd_before, 1.5);
        assert_eq!(delta.acd_after, 1.0);
    }

    // -------------------------------------------------------------- errors

    #[test]
    fn unknown_names_in_ops_fail_with_analysis_errors() {
        let g = cycle_graph();
        let cases = [
            SimOp::CutEdge {
                from: "nope".to_string(),
                to: "app/billing".to_string(),
            },
            SimOp::CutEdge {
                from: "app/billing".to_string(),
                to: "nope".to_string(),
            },
            SimOp::MoveNode {
                node: "nope".to_string(),
                new_parent: "app".to_string(),
            },
            SimOp::MoveNode {
                node: "app/billing".to_string(),
                new_parent: "nope".to_string(),
            },
            SimOp::Merge {
                survivor: "nope".to_string(),
                absorbed: "app/billing".to_string(),
            },
            SimOp::Merge {
                survivor: "app/billing".to_string(),
                absorbed: "nope".to_string(),
            },
        ];
        for op in cases {
            let err = apply_ops(&g, &[op]).expect_err("unknown name must fail");
            assert!(matches!(err, ArchguardError::Analysis(_)), "{err}");
            assert!(err.to_string().contains("no node named"), "{err}");
        }
        // An op invalid at its turn fails the whole (all-or-nothing) call.
        let ops = vec![
            SimOp::CutEdge {
                from: "app/billing".to_string(),
                to: "domain/money".to_string(),
            },
            SimOp::CutEdge {
                from: "ghost".to_string(),
                to: "domain/money".to_string(),
            },
        ];
        assert!(apply_ops(&g, &ops).is_err());
    }

    #[test]
    fn op_referencing_a_name_destroyed_by_an_earlier_op_fails() {
        let g = layered_graph();
        // After the move there is no `domain/x` anymore; the cut that
        // still names it must Err (documented, deterministic semantics).
        let ops = vec![
            SimOp::MoveNode {
                node: "domain/x".to_string(),
                new_parent: "app".to_string(),
            },
            SimOp::CutEdge {
                from: "domain/x".to_string(),
                to: "app/ui".to_string(),
            },
        ];
        let err = apply_ops(&g, &ops).expect_err("renamed away");
        assert!(err.to_string().contains("domain/x"), "{err}");

        // Same for the absorbed node of a merge.
        let ops = vec![
            SimOp::Merge {
                survivor: "domain/x".to_string(),
                absorbed: "domain/y".to_string(),
            },
            SimOp::MoveNode {
                node: "domain/y".to_string(),
                new_parent: "app".to_string(),
            },
        ];
        assert!(apply_ops(&g, &ops).is_err());
    }

    #[test]
    fn move_into_self_or_own_subtree_is_rejected() {
        let g = layered_graph();
        let err = apply_ops(
            &g,
            &[SimOp::MoveNode {
                node: "domain/x".to_string(),
                new_parent: "domain/x".to_string(),
            }],
        )
        .expect_err("self-parent");
        assert!(err.to_string().contains("itself"), "{err}");
        let err = apply_ops(
            &g,
            &[SimOp::MoveNode {
                node: "domain".to_string(),
                new_parent: "domain/x".to_string(),
            }],
        )
        .expect_err("cycle in the tree");
        assert!(err.to_string().contains("subtree"), "{err}");
    }

    #[test]
    fn move_needs_an_existing_module_as_parent() {
        let g = layered_graph();
        // A file is not a valid parent.
        let err = apply_ops(
            &g,
            &[SimOp::MoveNode {
                node: "domain/x".to_string(),
                new_parent: "app/ui/main.ts".to_string(),
            }],
        )
        .expect_err("file parent");
        assert!(err.to_string().contains("not a module"), "{err}");
    }

    #[test]
    fn rename_collisions_are_rejected() {
        // Moving app/ui under domain would create a second `domain/ui`
        // next to the existing one.
        let g = build(
            &[
                ("app", NodeKind::Module, None),
                ("app/ui", NodeKind::Module, Some("app")),
                ("domain", NodeKind::Module, None),
                ("domain/ui", NodeKind::Module, Some("domain")),
            ],
            &[],
        );
        let err = apply_ops(
            &g,
            &[SimOp::MoveNode {
                node: "app/ui".to_string(),
                new_parent: "domain".to_string(),
            }],
        )
        .expect_err("collision");
        assert!(err.to_string().contains("collide"), "{err}");
    }

    #[test]
    fn merge_guards_survivor_identity_and_subtree() {
        let g = layered_graph();
        let err = apply_ops(
            &g,
            &[SimOp::Merge {
                survivor: "domain/x".to_string(),
                absorbed: "domain/x".to_string(),
            }],
        )
        .expect_err("self-merge");
        assert!(err.to_string().contains("different"), "{err}");
        // Survivor inside the absorbed subtree would tie the tree into a
        // knot.
        let err = apply_ops(
            &g,
            &[SimOp::Merge {
                survivor: "domain/x/x.ts".to_string(),
                absorbed: "domain/x".to_string(),
            }],
        )
        .expect_err("survivor below absorbed");
        assert!(err.to_string().contains("subtree"), "{err}");
    }

    // -------------------------------------------------------- determinism

    #[test]
    fn simulate_is_deterministic_and_neutral_without_ops() {
        let g = cycle_graph();
        let cut = Cut::new([NodeId(1), NodeId(4)]);
        let ops = [SimOp::CutEdge {
            from: "app/billing".to_string(),
            to: "domain/money".to_string(),
        }];
        let first = simulate(&g, &cut, &ops, &[]).expect("first run");
        let second = simulate(&g, &cut, &ops, &[]).expect("second run");
        assert_eq!(first, second);

        let neutral = simulate(&g, &cut, &[], &[]).expect("no ops");
        assert!(neutral.resolved_findings.is_empty());
        assert!(neutral.introduced_findings.is_empty());
        assert_eq!(neutral.scc_count_before, neutral.scc_count_after);
        assert_eq!(neutral.acd_before, neutral.acd_after);
    }
}
