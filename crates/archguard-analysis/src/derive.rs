//! Layer & depth derivation (T45): the zero-config start hypothesis of
//! Kap. 7 ("Layer-Ableitung") and of the Kap. 11 rows `[modules].depth`
//! and `[[layers]]`.
//!
//! [`suggest_depth`] chooses the `[modules].depth` zoom whose
//! [`Cut::at_depth`] cut shows a target number of module nodes, and
//! [`infer_layers`] condenses the cyclic components reported by
//! [`crate::scc::find_sccs`], ranks the condensed DAG by longest dependency
//! path and turns every rank into one [`LayerConfig`] named by the
//! affinity vocabularies of Kap. 7. Modules inside a cycle cannot be
//! ranked — they land in [`DerivedLayers::unranked`] and get no layer
//! (Kap. 11: "Zyklus verhindert Schichtung").
//!
//! Both functions are pure and deterministic (Kap. 12): no IO, every
//! intermediate collection is a `BTreeMap`/`BTreeSet` or gets sorted
//! before use, so two runs over the same input return equal results.

use std::collections::{BTreeMap, BTreeSet};

use archguard_core::{Cut, Graph, LayerConfig, LayerOrigin, NodeId, NodeKind};

use crate::scc::Scc;

/// Affinity vocabulary of the bottom layer (Kap. 7).
const BOTTOM_WORDS: &[&str] = &["core", "domain", "model", "entity", "shared"];
/// Affinity vocabulary of the middle layers (Kap. 7).
const MIDDLE_WORDS: &[&str] = &["app", "service", "usecase", "logic", "feature"];
/// Affinity vocabulary of the top layer (Kap. 7).
const TOP_WORDS: &[&str] = &["api", "ui", "cli", "web", "controller", "http"];

/// Output of [`infer_layers`]: the derived `[[layers]]` block plus the
/// modules a cycle kept unrankable (Kap. 11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedLayers {
    /// One layer per rank, sorted bottom-up (rank 0 first), every entry
    /// carrying [`LayerOrigin::Derived`].
    pub layers: Vec<LayerConfig>,
    /// Names of the modules inside cyclic components — sorted, no layer.
    pub unranked: Vec<String>,
}

/// Suggest the `[modules].depth` zoom level for `g` (Kap. 11: depth with
/// a target of ~10–100 visible module nodes).
///
/// For every candidate depth `1..=max` the number of nodes
/// [`Cut::at_depth`] would show is counted, and the smallest depth whose
/// count lies in `min..=max` is returned. When no candidate qualifies,
/// the depth whose count sits closest to the range wins, ties broken
/// towards the smaller depth. A graph without module levels has nothing
/// to zoom into and returns `1`; an empty candidate range (`max == 0`)
/// likewise returns `1`, keeping the result a valid depth either way.
#[tracing::instrument(skip_all)]
pub fn suggest_depth(g: &Graph, min: usize, max: usize) -> u32 {
    // Without a single module node every file already sits at the root
    // and every cut is the full file set — depth 1 is the only answer.
    if !g.nodes.iter().any(|node| node.kind == NodeKind::Module) {
        return 1;
    }
    let mut best = 1u32;
    let mut best_distance = usize::MAX;
    for depth in 1..=max {
        let count = Cut::at_depth(g, depth).len();
        let distance = distance_to_range(count, min, max);
        if distance == 0 {
            // Candidates are tried ascending, so this is the smallest
            // qualifying depth.
            return depth as u32;
        }
        if distance < best_distance {
            best_distance = distance;
            best = depth as u32;
        }
    }
    best
}

/// How far `count` sits from the inclusive range `min..=max` (`0` inside).
fn distance_to_range(count: usize, min: usize, max: usize) -> usize {
    if count < min {
        min - count
    } else {
        // `count >= min`: inside the range this saturates to `0`.
        count.saturating_sub(max)
    }
}

/// Derive the `[[layers]]` block of the effective config from `view`
/// (Kap. 7 "Layer-Ableitung").
///
/// The `sccs` — the [`crate::scc::find_sccs`] output for the very graph that
/// is passed as `view` — are condensed: all members of one component
/// share a node in the condensation and can therefore never distort each
/// other's rank. On the condensation the longest-path rank
/// `rank = 1 + max(rank of the dependency targets)` (sinks `0`) is
/// computed by monotone relaxation to a fixpoint; the iteration is
/// bounded by the component count, so even a caller that withheld an
/// SCC terminates. Every member of a component is then unrankable: it
/// goes to [`DerivedLayers::unranked`] (sorted names) and gets no layer,
/// while every remaining node forms a layer together with all nodes of
/// the same rank — exactly one layer per rank, rank 0 at the bottom.
///
/// A layer is named after the affinity vocabulary word that matches the
/// last path segment of a *strict majority* of its members (bottom
/// `core|domain|model|entity|shared`, middle `app|service|usecase|logic|feature`,
/// top `api|ui|cli|web|controller|http`), else `layer-<rank>`. `contains`
/// is `<member>/**` for every member (sorted), `may_depend_on` holds the
/// names of all lower-rank layers (sorted), and every layer carries
/// [`LayerOrigin::Derived`]. Node ids unknown to `view` (only reachable
/// for malformed input) are skipped, mirroring [`crate::scc::find_sccs`].
#[tracing::instrument(skip_all)]
pub fn infer_layers(view: &Graph, sccs: &[Scc]) -> DerivedLayers {
    let (components, in_cycle) = condense(view, sccs);

    let mut unranked: Vec<String> = view
        .nodes
        .iter()
        .filter(|node| in_cycle.contains(&node.id))
        .map(|node| node.name.clone())
        .collect();
    unranked.sort();

    let deps = condensed_deps(view, &components);
    let ranks = longest_path_ranks(&deps);

    // Rankable nodes grouped by rank; member names are unique, so the
    // group is sorted once and used as the `contains` order.
    let mut by_rank: BTreeMap<u32, Vec<String>> = BTreeMap::new();
    for node in &view.nodes {
        if !in_cycle.contains(&node.id) {
            let rank = ranks.get(&node.id).copied().unwrap_or(0);
            by_rank.entry(rank).or_default().push(node.name.clone());
        }
    }

    let mut layers: Vec<LayerConfig> = Vec::new();
    let mut lower_names: Vec<String> = Vec::new();
    for (rank, mut members) in by_rank {
        members.sort();
        let name = majority_word(&members)
            .map(str::to_string)
            .unwrap_or_else(|| format!("layer-{rank}"));
        layers.push(LayerConfig {
            name: name.clone(),
            contains: members
                .iter()
                .map(|member| format!("{member}/**"))
                .collect(),
            may_depend_on: lower_names.clone(),
            origin: LayerOrigin::Derived,
        });
        lower_names.push(name);
        lower_names.sort();
    }

    DerivedLayers { layers, unranked }
}

/// Condense `view` over `sccs`: the representative map (SCC members to
/// the representative of their component — all members share one — every
/// other node to itself) plus the set of known SCC members, which are
/// the nodes that get no rank at all. Members of unknown ids and edges
/// towards unknown ids are skipped, as in [`crate::scc::find_sccs`].
fn condense(view: &Graph, sccs: &[Scc]) -> (BTreeMap<NodeId, NodeId>, BTreeSet<NodeId>) {
    let known: BTreeSet<NodeId> = view.nodes.iter().map(|node| node.id).collect();
    let mut components: BTreeMap<NodeId, NodeId> = known.iter().map(|&id| (id, id)).collect();
    let mut in_cycle: BTreeSet<NodeId> = BTreeSet::new();
    for scc in sccs {
        let members: Vec<NodeId> = scc
            .node_ids
            .iter()
            .copied()
            .filter(|id| known.contains(id))
            .collect();
        let Some(&representative) = members.iter().min_by_key(|&&id| (name(view, id), id)) else {
            continue;
        };
        for member in members {
            components.insert(member, representative);
            in_cycle.insert(member);
        }
    }
    (components, in_cycle)
}

/// The condensation's dependency sets: component → components it
/// directly depends on, self-dependencies dropped (they cannot exist
/// between true SCCs, but malformed input is guarded the same way).
fn condensed_deps(
    view: &Graph,
    components: &BTreeMap<NodeId, NodeId>,
) -> BTreeMap<NodeId, BTreeSet<NodeId>> {
    let mut deps: BTreeMap<NodeId, BTreeSet<NodeId>> = components
        .values()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|representative| (representative, BTreeSet::new()))
        .collect();
    for edge in &view.edges {
        let (Some(&from), Some(&to)) = (components.get(&edge.from), components.get(&edge.to))
        else {
            continue;
        };
        if from != to {
            deps.entry(from).or_default().insert(to);
        }
    }
    deps
}

/// Longest-path ranks of the condensation: sinks `0`, every other
/// component `1 + max(rank of its dependency targets)`.
///
/// Monotone relaxation over the fixed `BTreeMap` order until a pass
/// changes nothing; in the DAG every pass raises at least one rank, so
/// `len` passes always suffice, and the cap also lets a residual cycle
/// (an incomplete `sccs` argument) terminate deterministically instead
/// of looping forever.
fn longest_path_ranks(deps: &BTreeMap<NodeId, BTreeSet<NodeId>>) -> BTreeMap<NodeId, u32> {
    let mut ranks: BTreeMap<NodeId, u32> = deps.keys().map(|&id| (id, 0)).collect();
    for _ in 0..=deps.len() {
        let mut changed = false;
        for (&component, targets) in deps {
            let candidate = targets
                .iter()
                .filter_map(|target| ranks.get(target))
                .map(|&rank| rank + 1)
                .max()
                .unwrap_or(0);
            if candidate > ranks[&component] {
                ranks.insert(component, candidate);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    ranks
}

/// The affinity vocabulary word matching the last path segment of a
/// strict majority of `members`, if any (Kap. 7). A strict majority is
/// unique across all three vocabularies, and the fixed vocabulary and
/// word order make the choice deterministic.
fn majority_word(members: &[String]) -> Option<&'static str> {
    let total = members.len();
    if total == 0 {
        return None;
    }
    [BOTTOM_WORDS, MIDDLE_WORDS, TOP_WORDS]
        .into_iter()
        .flat_map(|vocabulary| vocabulary.iter().copied())
        .find(|word| {
            members
                .iter()
                .filter(|member| last_segment(member) == *word)
                .count()
                * 2
                > total
        })
}

/// Last `/`-separated segment of a node name (the whole name without
/// any `/`).
fn last_segment(node_name: &str) -> &str {
    node_name.rsplit('/').next().unwrap_or(node_name)
}

/// Name of `id`, empty for an unknown id (only reachable for malformed
/// input; ids fed in here come from `view.nodes`).
fn name(view: &Graph, id: NodeId) -> &str {
    view.node_name(id).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scc::find_sccs;
    use archguard_core::{Edge, GraphBuilder, Node, NodeKind};

    /// Hand-assembled graph: `nodes` gives names in *insertion* order
    /// (id = position), `edges` references nodes by name. All nodes are
    /// parentless files — `infer_layers` only ever looks at names.
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

    /// Layer names bottom-up.
    fn layer_names(result: &DerivedLayers) -> Vec<&str> {
        result.layers.iter().map(|l| l.name.as_str()).collect()
    }

    /// Containment tree: `a/b/f1.ts … a/x/f3.ts` — two depth-0 modules,
    /// two depth-1 modules, six depth-2 files.
    fn tree() -> Graph {
        let mut builder = GraphBuilder::new();
        for dir in ["b", "x"] {
            for file in ["f1", "f2", "f3"] {
                builder.ensure_path(&format!("a/{dir}/{file}.ts"), Some("ts"));
            }
        }
        builder.build("/abs".to_string())
    }

    // ------------------------------------------------------ suggest_depth

    #[test]
    fn suggest_depth_picks_the_smallest_qualifying_depth() {
        let g = tree();
        // Cut sizes: depth 1 → 2 module nodes, depth 2+ → the 6 files.
        assert_eq!(suggest_depth(&g, 1, 2), 1);
        assert_eq!(suggest_depth(&g, 3, 10), 2);
        // A range both depths qualify: the smaller depth wins.
        assert_eq!(suggest_depth(&g, 1, 10), 1);
    }

    #[test]
    fn suggest_depth_falls_back_to_the_closest_count() {
        let g = tree();
        // No depth reaches 100: depth 2..=max all show 6 (distance 94),
        // depth 1 shows 2 (distance 98) → smallest depth among the best.
        assert_eq!(suggest_depth(&g, 100, 200), 2);
        // Everything is too big: depth 1's count of 2 is closest to 0..=1.
        assert_eq!(suggest_depth(&g, 0, 1), 1);
    }

    #[test]
    fn suggest_depth_returns_one_without_module_levels() {
        // Only parentless file nodes — nothing to zoom into.
        let g = graph(&["a.ts", "b.ts"], &[("a.ts", "b.ts")]);
        assert_eq!(suggest_depth(&g, 1, 10), 1);
        assert_eq!(suggest_depth(&g, 100, 200), 1);
        // Degenerate candidate range and empty graph also answer 1.
        assert_eq!(suggest_depth(&g, 1, 0), 1);
        assert_eq!(suggest_depth(&graph(&[], &[]), 1, 5), 1);
    }

    #[test]
    fn suggest_depth_is_deterministic() {
        let g = tree();
        assert_eq!(
            suggest_depth(&g, 3, 10),
            suggest_depth(&g, 3, 10),
            "run twice → equal"
        );
    }

    // ------------------------------------------------------- infer_layers

    #[test]
    fn three_tier_dag_named_domain_app_api_comes_out_ranked_and_named() {
        let g = graph(
            &["domain", "app", "api"],
            &[("app", "domain"), ("api", "app"), ("api", "domain")],
        );
        let result = infer_layers(&g, &[]);

        assert!(result.unranked.is_empty());
        assert_eq!(layer_names(&result), ["domain", "app", "api"]);
        assert!(
            result
                .layers
                .iter()
                .all(|l| l.origin == LayerOrigin::Derived)
        );

        assert_eq!(result.layers[0].contains, ["domain/**"]);
        assert!(result.layers[0].may_depend_on.is_empty());
        assert_eq!(result.layers[1].contains, ["app/**"]);
        assert_eq!(result.layers[1].may_depend_on, ["domain"]);
        assert_eq!(result.layers[2].contains, ["api/**"]);
        assert_eq!(result.layers[2].may_depend_on, ["app", "domain"]);
    }

    #[test]
    fn chain_without_vocabulary_is_named_by_rank() {
        // one → two → three: three is the sink (rank 0).
        let g = graph(
            &["one", "two", "three"],
            &[("one", "two"), ("two", "three")],
        );
        let result = infer_layers(&g, &[]);

        assert_eq!(layer_names(&result), ["layer-0", "layer-1", "layer-2"]);
        let contains: Vec<&[String]> = result
            .layers
            .iter()
            .map(|l| l.contains.as_slice())
            .collect();
        assert_eq!(
            contains,
            vec![
                ["three/**".to_string()],
                ["two/**".to_string()],
                ["one/**".to_string()]
            ]
        );
        // All lower-rank layer names, sorted.
        assert_eq!(result.layers[2].may_depend_on, ["layer-0", "layer-1"]);
    }

    #[test]
    fn scc_members_appear_in_unranked_and_in_no_layer() {
        // a ↔ b cycle, c depends on the cycle.
        let g = graph(&["a", "b", "c"], &[("a", "b"), ("b", "a"), ("c", "a")]);
        let sccs = find_sccs(&g);
        assert_eq!(sccs.len(), 1);

        let result = infer_layers(&g, &sccs);

        assert_eq!(result.unranked, ["a", "b"]);
        // The condensation makes {a,b} the sink; c ranks above it, but
        // the rank-0 layer itself is empty (its only members are the
        // cycle) → c's layer carries no lower names.
        assert_eq!(layer_names(&result), ["layer-1"]);
        assert_eq!(result.layers[0].contains, ["c/**"]);
        assert!(result.layers[0].may_depend_on.is_empty());
    }

    #[test]
    fn all_members_cycle_leaves_no_layers() {
        let g = graph(&["x", "y"], &[("x", "y"), ("y", "x")]);
        let result = infer_layers(&g, &find_sccs(&g));
        assert!(result.layers.is_empty());
        assert_eq!(result.unranked, ["x", "y"]);
    }

    #[test]
    fn layer_takes_the_word_matching_a_strict_majority() {
        // rank 0: sales/core + stock/core beat legacy/domain;
        // rank 1: the single api member.
        let g = graph(
            &["shop/api", "sales/core", "stock/core", "legacy/domain"],
            &[
                ("shop/api", "sales/core"),
                ("shop/api", "stock/core"),
                ("shop/api", "legacy/domain"),
            ],
        );
        let result = infer_layers(&g, &[]);

        assert_eq!(layer_names(&result), ["core", "api"]);
        assert_eq!(
            result.layers[0].contains,
            ["legacy/domain/**", "sales/core/**", "stock/core/**"]
        );
        assert_eq!(result.layers[1].may_depend_on, ["core"]);
    }

    #[test]
    fn no_strict_majority_falls_back_to_the_rank_name() {
        // One `core`, one `domain` — each matches half, not more.
        let g = graph(&["a/core", "b/domain"], &[]);
        let result = infer_layers(&g, &[]);
        assert_eq!(layer_names(&result), ["layer-0"]);
    }

    #[test]
    fn empty_view_yields_no_layers_and_no_unranked() {
        let result = infer_layers(&graph(&[], &[]), &[]);
        assert!(result.layers.is_empty());
        assert!(result.unranked.is_empty());
    }

    #[test]
    fn edges_to_unknown_node_ids_are_skipped() {
        let mut g = graph(&["core", "app"], &[("app", "core")]);
        g.edges.push(Edge {
            from: NodeId(99),
            to: NodeId(0),
            refs: Vec::new(),
            count: None,
        });
        let result = infer_layers(&g, &[]);
        assert_eq!(layer_names(&result), ["core", "app"]);
    }

    #[test]
    fn infer_layers_is_deterministic_across_runs() {
        // Two members share the `app` word; the loop keeps two names out
        // of the layering; `shared/lib` ranks beside the cycle component
        // but is rankable itself.
        let g = graph(
            &[
                "edge/api",
                "green/app",
                "blue/app",
                "shared/lib",
                "loop-1",
                "loop-2",
            ],
            &[
                ("edge/api", "green/app"),
                ("edge/api", "blue/app"),
                ("green/app", "shared/lib"),
                ("blue/app", "loop-1"),
                ("loop-1", "loop-2"),
                ("loop-2", "loop-1"),
            ],
        );
        let sccs = find_sccs(&g);

        let first = infer_layers(&g, &sccs);
        let second = infer_layers(&g, &sccs);
        assert_eq!(first, second, "run twice → equal");

        // Shape sanity: bottom-up, cycle members out, names by majority.
        assert_eq!(first.unranked, ["loop-1", "loop-2"]);
        assert_eq!(layer_names(&first), ["layer-0", "app", "api"]);
        assert_eq!(first.layers[2].may_depend_on, ["app", "layer-0"]);
    }

    #[test]
    fn residual_cycles_terminate_even_without_scc_input() {
        // Caller withholds the SCCs: the relaxation cap must converge
        // (the cycle then ranks arbitrarily high but deterministically)
        // instead of looping forever.
        let g = graph(&["a", "b", "c"], &[("a", "b"), ("b", "a"), ("c", "a")]);
        let result = infer_layers(&g, &[]);
        assert!(result.unranked.is_empty());
        // Every node ended up in exactly one layer…
        let ranked: Vec<&str> = result
            .layers
            .iter()
            .flat_map(|layer| {
                layer
                    .contains
                    .iter()
                    .map(|glob| glob.trim_end_matches("/**"))
            })
            .collect();
        assert_eq!(ranked, ["a", "b", "c"]);
        // …and the run is deterministic despite the uncondensed cycle.
        assert_eq!(result, infer_layers(&g, &[]), "run twice → equal");
    }
}
