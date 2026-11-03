//! Stable Dependencies Principle violations (R05).
//!
//! A module edge `A → B` violates the SDP when `B` is *less* stable than
//! `A` (`I(B) > I(A)`): a stable component depending on an unstable one.
//! Instability comes from [`compute_metrics`] over the same view, so the
//! rule is measured where it is judged. Edges whose endpoint instability
//! cannot be computed (endpoint missing from the metrics) are skipped —
//! never guessed.

use std::collections::BTreeMap;

use archguard_core::{Graph, SdpViolation};

use crate::metrics::compute_metrics;
use crate::scc::find_sccs;

/// SDP violations on the edges of `graph` (a module-level view), each
/// carrying its `file:line` import evidence.
///
/// Deterministic: results are sorted by `(from, to)` and evidence keeps
/// graph order, deduplicated.
pub fn sdp_violations(graph: &Graph) -> Vec<SdpViolation> {
    let sccs = find_sccs(graph);
    let metrics = compute_metrics(graph, &sccs);
    let mut violations: BTreeMap<(String, String), SdpViolation> = BTreeMap::new();
    for edge in &graph.edges {
        let (Some(from), Some(to)) = (graph.node_name(edge.from), graph.node_name(edge.to)) else {
            continue;
        };
        if from == to {
            continue;
        }
        let (Some(i_from), Some(i_to)) = (
            metrics.get(from).map(|m| f64::from(m.instability)),
            metrics.get(to).map(|m| f64::from(m.instability)),
        ) else {
            // Unknown instability: skipped with the metrics lookup as the
            // reason — an invented value would fabricate a violation.
            continue;
        };
        if i_to <= i_from {
            continue;
        }
        let entry = violations
            .entry((from.to_string(), to.to_string()))
            .or_insert_with(|| SdpViolation {
                from: from.to_string(),
                to: to.to_string(),
                i_from: Some(i_from),
                i_to: Some(i_to),
                evidence: Vec::new(),
            });
        for reference in &edge.refs {
            let site = format!("{}:{}", reference.file, reference.line);
            if !entry.evidence.contains(&site) {
                entry.evidence.push(site);
            }
        }
    }
    violations.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_helpers::graph;

    #[test]
    fn equal_instability_is_never_a_violation() {
        // The three-node ring has I = 0.5 everywhere; guards against
        // `>=` instead of `>` as the violation condition.
        let g = graph(
            &["core", "util", "app"],
            &[("util", "core"), ("core", "app"), ("app", "util")],
        );
        assert!(sdp_violations(&g).is_empty());
    }

    #[test]
    fn i_from_02_to_i_to_08_is_reported_with_evidence() {
        // core: fan_in 4, fan_out 1 → I = 1/5 = 0.2 (stable-ish)
        // util: fan_in 1, fan_out 4 → I = 4/5 = 0.8 (unstable)
        // Edges: four files depend on core; core depends on util;
        // util depends on four files (its fan_out).
        let mut g = graph(
            &["core", "util", "f1", "f2", "f3", "f4"],
            &[
                ("f1", "core"),
                ("f2", "core"),
                ("f3", "core"),
                ("f4", "core"),
                ("core", "util"),
                ("util", "f1"),
                ("util", "f2"),
                ("util", "f3"),
                ("util", "f4"),
            ],
        );
        // The violating edge carries its import site.
        let core_id = archguard_core::NodeId(0);
        let util_id = archguard_core::NodeId(1);
        for edge in &mut g.edges {
            if edge.from == core_id && edge.to == util_id {
                edge.refs.push(archguard_core::Ref {
                    file: "src/core.rs".to_string(),
                    line: 7,
                    raw: "use util::Thing;".to_string(),
                });
            }
        }
        let sccs = find_sccs(&g);
        let m = compute_metrics(&g, &sccs);
        assert!((f64::from(m["core"].instability) - 0.2).abs() < 1e-6);
        assert!((f64::from(m["util"].instability) - 0.8).abs() < 1e-6);

        let violations = sdp_violations(&g);
        // Only core → util points from more stable to less stable.
        let pairs: Vec<(&str, &str)> = violations
            .iter()
            .map(|v| (v.from.as_str(), v.to.as_str()))
            .collect();
        assert_eq!(pairs, [("core", "util")]);
        // Instability is computed in f32 and widened: compare with a
        // tolerance instead of pinning the exact f32 bits.
        assert!((violations[0].i_from.unwrap() - 0.2).abs() < 1e-6);
        assert!((violations[0].i_to.unwrap() - 0.8).abs() < 1e-6);
        assert_eq!(violations[0].evidence, ["src/core.rs:7"]);
    }

    #[test]
    fn i_not_greater_is_never_a_violation() {
        // Equal instabilities (mirror image) must not report anything.
        let g = graph(&["a", "b", "c", "d"], &[("a", "b"), ("c", "d")]);
        assert!(sdp_violations(&g).is_empty());
    }

    #[test]
    fn unknown_instability_is_skipped_not_invented() {
        // Edge endpoint missing from the node list cannot be measured.
        let mut g = graph(&["a", "b"], &[("a", "b")]);
        g.edges.push(archguard_core::Edge {
            from: archguard_core::NodeId(42),
            to: archguard_core::NodeId(1),
            refs: Vec::new(),
            count: None,
        });
        // Only the measurable edge is considered; no panic, no guess.
        assert!(sdp_violations(&g).is_empty());
    }

    #[test]
    fn signature_is_stable() {
        let f: fn(&Graph) -> Vec<SdpViolation> = sdp_violations;
        let _ = f;
    }
}
