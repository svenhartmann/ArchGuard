//! Cut suggestions (T16): the ranked candidates that break the sampled
//! cycles of one [`Scc`].
//!
//! [`suggest_cuts`] scores every edge of the component that is *internal*
//! — both endpoints are members of the SCC — by how many of the sampled
//! cycles it breaks and how expensive it is to cut. An edge that appears
//! in many cycles breaks a lot; an edge backed by many concrete import
//! references (its `ref_count`, the summed [`archguard_core::Edge::weight`]
//! — the F12 total, not the capped sample) is hard to
//! untangle, so the raw break ratio is damped by `1.0 / (1.0 + ln(ref_count))`.
//! The highest-scoring edges are the suggestions a cut should start with.
//!
//! Everything is deterministic (Kap. 12): candidates are taken from the
//! already name-sorted [`Scc::edges`], the sort compares scores with
//! [`f32::total_cmp`] (no NaN can occur here) and falls back to the
//! `(from, to)` endpoint names, so repeated calls return the same vector.

use std::collections::BTreeSet;

use archguard_core::{Graph, NodeId, SuggestedCut};

use crate::scc::Scc;

/// The remediation hint every suggested [`SuggestedCut`] carries in its
/// `reason` (Kap. 5): the recommended way to break a cyclic dependency.
const DEPENDENCY_INVERSION_HINT: &str =
    "consider dependency inversion: define an interface on the depended-upon side";

/// Suggest up to `max` cuts for `scc`, ranked from best to worst.
///
/// For every internal edge of the component (both endpoints members of
/// `scc`) the candidate counts the `cycles` — the [`crate::cycles::sample_cycles`]
/// output, each cycle a node list whose closing edge back to the first
/// node is implied — that contain it (`broken`), and takes the edge's
/// `ref_count` (the number of concrete import sites, summed over all
/// [`Graph`] edges with exactly this `(from, to)`) as its cost:
/// `score = (broken / cycles.len()) * 1.0 / (1.0 + ln(ref_count))`.
/// Candidates are sorted by score descending, ties broken by the
/// `(from, to)` node names, and truncated to `max`.
///
/// `cycles` empty means no bounded witness was found (the SCC is still cyclic): the result is
/// empty (this also guards the division by `cycles.len()`), and `max ==
/// 0` yields no suggestions either. Names of unknown node ids render as
/// the empty string, mirroring [`crate::scc::find_sccs`]'s handling.
pub fn suggest_cuts(
    graph: &Graph,
    scc: &Scc,
    cycles: &[Vec<NodeId>],
    max: usize,
) -> Vec<SuggestedCut> {
    // Without a sampled cycle there is nothing to break, and the score
    // formula would divide by zero.
    if cycles.is_empty() {
        return Vec::new();
    }
    let members: BTreeSet<NodeId> = scc.node_ids.iter().copied().collect();
    let total = cycles.len();

    let mut cuts: Vec<SuggestedCut> = scc
        .edges
        .iter()
        .filter(|(from, to)| members.contains(from) && members.contains(to))
        .map(|&(from, to)| {
            let broken = cycles
                .iter()
                .filter(|cycle| contains_edge(cycle, from, to))
                .count();
            let ref_count = ref_count(graph, from, to);
            let score = (broken as f32 / total as f32) * (1.0 / (1.0 + (ref_count as f32).ln()));
            SuggestedCut {
                from: name(graph, from).to_string(),
                to: name(graph, to).to_string(),
                score,
                reason: format!(
                    "ref_count: {ref_count}; cutting this dependency breaks \
                     {broken} of {total} sampled cycles; {DEPENDENCY_INVERSION_HINT}"
                ),
            }
        })
        .collect();

    cuts.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then_with(|| a.from.cmp(&b.from))
            .then_with(|| a.to.cmp(&b.to))
    });
    cuts.truncate(max);
    cuts
}

/// Does the elementary cycle `cycle` contain the edge `from → to`? The
/// closing edge back to the first node is implied, as in
/// [`crate::cycles::sample_cycles`]'s output; an empty cycle contains nothing.
fn contains_edge(cycle: &[NodeId], from: NodeId, to: NodeId) -> bool {
    cycle
        .iter()
        .enumerate()
        .any(|(index, node)| *node == from && cycle[(index + 1) % cycle.len()] == to)
}

/// Number of concrete import references proving `from → to`, summed
/// over all graph edges with exactly this endpoint pair (the canonical
/// builder keeps one such edge; hand-assembled graphs may repeat it and
/// each copy contributes its [`Edge::weight`], the F12 total rather
/// than the capped sample). `0` for an edge absent from the graph.
fn ref_count(graph: &Graph, from: NodeId, to: NodeId) -> u64 {
    graph
        .edges
        .iter()
        .filter(|edge| edge.from == from && edge.to == to)
        .map(|edge| edge.weight())
        .sum()
}

/// Name of `id`, empty for an unknown id (only reachable for malformed
/// input; ids fed in here come from `graph.nodes`).
fn name(graph: &Graph, id: NodeId) -> &str {
    graph.node_name(id).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cycles::sample_cycles;
    use crate::scc::find_sccs;
    use archguard_core::{Edge, Node, NodeKind, Ref};

    /// Hand-assembled graph: `nodes` gives names in *insertion* order
    /// (id = position), `edges` references nodes by name and carries the
    /// given number of distinct import-site refs.
    fn graph(nodes: &[&str], edges: &[(&str, &str, usize)]) -> Graph {
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
                .map(|(from, to, refs)| Edge {
                    from: id_of(from),
                    to: id_of(to),
                    refs: (0..*refs)
                        .map(|line| Ref {
                            file: (*from).to_string(),
                            line: line as u32 + 1,
                            raw: String::new(),
                        })
                        .collect(),
                    // Builder-shaped: the total equals the sample.
                    count: Some(*refs as u64),
                })
                .collect(),
        }
    }

    /// The single SCC of `g`, failing the test if there is not exactly one.
    fn sole_scc(g: &Graph) -> Scc {
        let mut sccs = find_sccs(g);
        assert_eq!(sccs.len(), 1, "expected exactly one SCC");
        sccs.remove(0)
    }

    /// The shared diamond: `a → b` sits in both cycles (`a→b→c→a` and
    /// `a→b→d→a`), every other internal edge in exactly one. Ref counts
    /// default to one, `refs` overrides single edges by `(from, to)`.
    fn diamond(refs: &[(&str, &str, usize)]) -> Graph {
        let mut edges = vec![
            ("a", "b", 1),
            ("b", "c", 1),
            ("c", "a", 3),
            ("b", "d", 1),
            ("d", "a", 1),
        ];
        for edge in refs {
            let slot = edges
                .iter_mut()
                .find(|(from, to, _)| (*from, *to) == (edge.0, edge.1))
                .expect("test: override must name an existing edge");
            slot.2 = edge.2;
        }
        graph(&["a", "b", "c", "d"], &edges)
    }

    /// Endpoint names of `cuts` as `(from, to)`.
    fn pairs(cuts: &[SuggestedCut]) -> Vec<(&str, &str)> {
        cuts.iter()
            .map(|c| (c.from.as_str(), c.to.as_str()))
            .collect()
    }

    #[test]
    fn dominant_edge_ranked_first_with_exact_scores() {
        let g = diamond(&[]);
        let scc = sole_scc(&g);
        let cycles = sample_cycles(&g, &scc, 10, usize::MAX);
        assert_eq!(cycles.len(), 2);

        let cuts = suggest_cuts(&g, &scc, &cycles, usize::MAX);

        // All five internal edges are candidates; a→b breaks both cycles.
        assert_eq!(
            pairs(&cuts),
            [("a", "b"), ("b", "c"), ("b", "d"), ("d", "a"), ("c", "a")]
        );
        // broken 2/2, ref_count 1 → 1.0; broken 1/2, ref_count 1 → 0.5.
        assert_eq!(cuts[0].score, 1.0);
        assert_eq!(cuts[1].score, 0.5);
        assert_eq!(cuts[2].score, 0.5);
        assert_eq!(cuts[3].score, 0.5);
        // c→a has 3 refs: 0.5 / (1 + ln 3).
        assert!((cuts[4].score - 0.5 / (1.0 + 3.0f32.ln())).abs() < 1e-7);
    }

    #[test]
    fn high_ref_count_demotes_the_dominant_edge() {
        // Same graph, but a→b is proven by 50 import sites: its raw
        // 2/2 ratio is damped below the other edges' 1/2.
        let g = diamond(&[("a", "b", 50)]);
        let scc = sole_scc(&g);
        let cycles = sample_cycles(&g, &scc, 10, usize::MAX);

        let cuts = suggest_cuts(&g, &scc, &cycles, usize::MAX);
        assert_eq!(
            pairs(&cuts),
            [("b", "c"), ("b", "d"), ("d", "a"), ("c", "a"), ("a", "b")]
        );
        assert!((cuts[0].score - 0.5).abs() < 1e-7);
        assert!((cuts[4].score - 1.0 / (1.0 + 50.0f32.ln())).abs() < 1e-7);
    }

    #[test]
    fn a_capped_sample_with_exact_total_scores_by_the_total() {
        // F12: a → b was proven by 100 import references but the
        // (capped) evidence sample keeps only 10. The damping and the
        // reason must use the total 100, not the sample length.
        let mut g = diamond(&[]);
        let ab = g
            .edges
            .iter_mut()
            .find(|e| e.from == NodeId(0) && e.to == NodeId(1))
            .expect("test: a → b exists");
        ab.refs = (0..10)
            .map(|line| Ref {
                file: "a".to_string(),
                line: line + 1,
                raw: String::new(),
            })
            .collect();
        ab.count = Some(100);
        let scc = sole_scc(&g);
        let cycles = sample_cycles(&g, &scc, 10, usize::MAX);

        let cuts = suggest_cuts(&g, &scc, &cycles, usize::MAX);
        assert_eq!(pairs(&cuts)[4], ("a", "b"));
        assert!(cuts[4].reason.starts_with("ref_count: 100;"));
    }

    #[test]
    fn external_edges_are_excluded() {
        // d → e leaves the component; e must never show up as endpoint.
        let g = graph(
            &["a", "b", "c", "d", "e"],
            &[
                ("a", "b", 1),
                ("b", "c", 1),
                ("c", "a", 1),
                ("b", "d", 1),
                ("d", "a", 1),
                ("d", "e", 4),
            ],
        );
        let scc = sole_scc(&g);
        let cycles = sample_cycles(&g, &scc, 10, usize::MAX);

        let cuts = suggest_cuts(&g, &scc, &cycles, usize::MAX);
        assert_eq!(cuts.len(), 5);
        assert!(cuts.iter().all(|c| c.from != "e" && c.to != "e"));
    }

    #[test]
    fn inconsistent_scc_edges_leaking_outside_are_filtered() {
        // "Internal" is defined by membership, not by trust in `scc.edges`.
        let g = graph(
            &["a", "b", "c"],
            &[("a", "b", 1), ("b", "a", 1), ("a", "c", 1)],
        );
        let mut scc = sole_scc(&g); // {a, b}
        let c = NodeId(2);
        scc.edges.push((NodeId(0), c)); // hand-assembled leak to a non-member
        let cycles = vec![vec![NodeId(0), NodeId(1)]];

        let cuts = suggest_cuts(&g, &scc, &cycles, usize::MAX);
        assert_eq!(pairs(&cuts), [("a", "b"), ("b", "a")]);
    }

    #[test]
    fn max_truncates_and_zero_yields_nothing() {
        let g = diamond(&[]);
        let scc = sole_scc(&g);
        let cycles = sample_cycles(&g, &scc, 10, usize::MAX);

        let cuts = suggest_cuts(&g, &scc, &cycles, 2);
        assert_eq!(pairs(&cuts), [("a", "b"), ("b", "c")]);
        assert!(suggest_cuts(&g, &scc, &cycles, 0).is_empty());
    }

    #[test]
    fn empty_cycles_yield_no_suggestions_without_dividing_by_zero() {
        let g = diamond(&[]);
        let scc = sole_scc(&g);
        assert!(suggest_cuts(&g, &scc, &[], usize::MAX).is_empty());
    }

    #[test]
    fn reason_names_ref_count_broken_and_total_cycles_and_hint() {
        let g = diamond(&[]);
        let scc = sole_scc(&g);
        let cycles = sample_cycles(&g, &scc, 10, usize::MAX);

        let cuts = suggest_cuts(&g, &scc, &cycles, 1);
        assert_eq!(
            cuts[0].reason,
            "ref_count: 1; cutting this dependency breaks 2 of 2 sampled \
             cycles; consider dependency inversion: define an interface on \
             the depended-upon side"
        );
    }

    #[test]
    fn equal_scores_tie_break_by_endpoint_names() {
        // One cycle, both edges break it, both have one ref → same score;
        // insertion order must not leak into the result.
        let g = graph(&["b", "a"], &[("b", "a", 1), ("a", "b", 1)]);
        let scc = sole_scc(&g);
        let cycles = sample_cycles(&g, &scc, 10, usize::MAX);
        assert_eq!(cycles.len(), 1);

        let cuts = suggest_cuts(&g, &scc, &cycles, usize::MAX);
        assert_eq!(pairs(&cuts), [("a", "b"), ("b", "a")]);
        assert_eq!(cuts[0].score, 1.0);
        assert_eq!(cuts[1].score, 1.0);
        // Deterministic: repeated calls return the same ranking.
        assert_eq!(cuts, suggest_cuts(&g, &scc, &cycles, usize::MAX));
    }

    #[test]
    fn self_loop_cycle_suggests_its_own_edge() {
        let g = graph(&["a"], &[("a", "a", 2)]);
        let scc = sole_scc(&g);
        let cycles = sample_cycles(&g, &scc, 10, usize::MAX);
        assert_eq!(cycles, vec![vec![NodeId(0)]]);

        let cuts = suggest_cuts(&g, &scc, &cycles, usize::MAX);
        assert_eq!(pairs(&cuts), [("a", "a")]);
        assert!((cuts[0].score - 1.0 / (1.0 + 2.0f32.ln())).abs() < 1e-7);
    }

    #[test]
    fn duplicated_graph_edges_merge_their_refs_into_one_candidate() {
        let mut g = graph(&["a", "b"], &[("a", "b", 2), ("b", "a", 1)]);
        // Hand-assembled duplicate of a → b: its refs add to the count.
        g.edges.push(Edge {
            from: NodeId(0),
            to: NodeId(1),
            refs: (0..3)
                .map(|line| Ref {
                    file: "a".to_string(),
                    line: line + 1,
                    raw: String::new(),
                })
                .collect(),
            count: Some(3),
        });
        let scc = sole_scc(&g);
        let cycles = sample_cycles(&g, &scc, 10, usize::MAX);
        assert_eq!(cycles.len(), 1);

        let cuts = suggest_cuts(&g, &scc, &cycles, usize::MAX);
        // One candidate per internal edge; a→b now carries 2+3=5 refs and
        // loses to b→a's single ref despite breaking the same cycle.
        assert_eq!(pairs(&cuts), [("b", "a"), ("a", "b")]);
        assert_eq!(cuts[0].score, 1.0);
        assert!((cuts[1].score - 1.0 / (1.0 + 5.0f32.ln())).abs() < 1e-7);
        assert!(cuts[1].reason.starts_with("ref_count: 5;"));
    }
}
