//! Zone of Pain / Zone of Uselessness statistics (R06).
//!
//! Classifies module boundaries by their Martin Main Sequence Distance
//! `D' = |A + I − 1|`: boundaries far from the main sequence with
//! `A + I < 1` form the zone of pain (concrete and stable), far ones with
//! `A + I > 1` the zone of uselessness (abstract and unstable).
//! Boundaries without complete compiler evidence stay `unmeasured`.

use std::collections::BTreeSet;

use archguard_core::{AbstractionMetric, Graph, ZoneStats};

/// A boundary with `D'` beyond this is considered far from the main
/// sequence (the classic ±1/3 corridor).
const MAIN_SEQUENCE_MAX_DISTANCE: f64 = 1.0 / 3.0;

/// Zone distribution over the abstraction metrics of `graph`.
///
/// Each metric counts as exactly one boundary. Nodes of `graph` that no
/// metric covers count as `unmeasured` — missing evidence is reported,
/// never guessed into a zone. The `distance` field of the metric is not
/// trusted; `D'` is recomputed from `A` and `I`.
pub fn zone_stats(graph: &Graph, abstractions: &[AbstractionMetric]) -> ZoneStats {
    let mut stats = ZoneStats::default();
    let mut covered: BTreeSet<String> = BTreeSet::new();
    for metric in abstractions {
        if metric.scope == "node" {
            covered.insert(metric.name.clone());
        }
        covered.extend(metric.nodes.iter().cloned());
        let measurable = metric.coverage == "complete";
        match (measurable, metric.abstractness) {
            (true, Some(abstractness)) => {
                let sum = abstractness + metric.instability;
                let distance = (sum - 1.0).abs();
                if distance <= MAIN_SEQUENCE_MAX_DISTANCE {
                    stats.main_sequence += 1;
                } else if sum < 1.0 {
                    stats.zone_of_pain += 1;
                } else {
                    stats.zone_of_uselessness += 1;
                }
            }
            _ => stats.unmeasured += 1,
        }
    }
    stats.unmeasured += graph
        .nodes
        .iter()
        .filter(|node| !covered.contains(&node.name))
        .count();
    stats
}

/// Test helper: a complete metric with explicit A/I values.
#[cfg(test)]
fn metric(name: &str, abstractness: f64, instability: f64) -> AbstractionMetric {
    AbstractionMetric {
        scope: "node".to_string(),
        name: name.to_string(),
        nodes: vec![name.to_string()],
        types: 1,
        abstract_types: if abstractness > 0.5 { 1 } else { 0 },
        fan_in: 0,
        fan_out: 0,
        instability,
        abstractness: Some(abstractness),
        distance: None,
        coverage: "complete".to_string(),
        reason: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_helpers::graph;

    #[test]
    fn synthetic_a_i_pairs_land_in_the_right_zones() {
        let names: Vec<String> = ["pain", "useless", "main", "edge"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let g = graph(&names.iter().map(String::as_str).collect::<Vec<_>>(), &[]);
        let abstractions = vec![
            // Far from the sequence towards A+I < 1 → zone of pain.
            metric("pain", 0.1, 0.1),
            // Far towards A+I > 1 → zone of uselessness.
            metric("useless", 0.9, 0.9),
            // On the sequence → main.
            metric("main", 0.5, 0.5),
            // D' = 0.5 > 1/3 but exactly A+I = 1 → uselessness branch
            // only via the `else`; with sum == 1 the corridor applies.
            metric("edge", 1.0, 0.0),
        ];
        let stats = zone_stats(&g, &abstractions);
        assert_eq!(stats.zone_of_pain, 1);
        assert_eq!(stats.zone_of_uselessness, 1);
        assert_eq!(stats.main_sequence, 2);
        assert_eq!(stats.unmeasured, 0);
    }

    #[test]
    fn incomplete_coverage_counts_as_unmeasured_never_guessed() {
        let mut incomplete = metric("hidden", 0.5, 0.5);
        incomplete.coverage = "unavailable".to_string();
        incomplete.abstractness = None;
        let g = graph(&["measured", "hidden", "free"], &[]);
        let stats = zone_stats(&g, &[metric("measured", 0.9, 0.9), incomplete]);
        assert_eq!(stats.zone_of_uselessness, 1);
        // The incomplete metric and the uncovered node `free` are both
        // visibly unmeasured.
        assert_eq!(stats.unmeasured, 2);
    }

    #[test]
    fn scc_scope_metrics_cover_their_members() {
        let mut scc_metric = metric("scc-1", 0.1, 0.1);
        scc_metric.scope = "scc".to_string();
        scc_metric.name = "scc-1".to_string();
        scc_metric.nodes = vec!["a".to_string(), "b".to_string()];
        let g = graph(&["a", "b"], &[("a", "b"), ("b", "a")]);
        let stats = zone_stats(&g, &[scc_metric]);
        assert_eq!(stats.zone_of_pain, 1);
        // Members are covered by the SCC metric, no double counting.
        assert_eq!(stats.unmeasured, 0);
    }

    #[test]
    fn signature_is_stable() {
        let f: fn(&Graph, &[AbstractionMetric]) -> ZoneStats = zone_stats;
        let _ = f;
    }
}
