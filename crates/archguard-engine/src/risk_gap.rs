//! Risk-weighted coverage gap (R21).
//!
//! Ranks source files by `hotspot score × (1 − line coverage)`: risk
//! that is both dangerous and untested. The hotspot score mirrors the
//! analysis crate's review components ([`archguard_core::ReviewScore`]
//! minus the git churn term, which needs git evidence this function
//! deliberately does not shell out for):
//!
//! - cycle: `min(30 + 2·(scc_size − 1), 40)` for members of a cycle,
//! - coupling: `min(degree · 2, 20)` over direct neighbours,
//! - impact: `min(transitive dependents, 20)`.
//!
//! Coverage comes from the imported `.archguard/coverage.json` via
//! [`crate::test_coverage::read_test_coverage`]. Files without a coverage entry keep
//! `coverage: None` and `gap: None` — a gap is never computed against
//! an invented zero coverage; absence means "not measured", not "clean".

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use archguard_analysis::find_sccs;
use archguard_core::{ArchguardError, Config, Graph, NodeKind, ReviewScore, RiskCoverageGap};

use crate::test_coverage::read_test_coverage;

/// Hotspot components without churn: churn needs git evidence, this
/// ranking stays purely static and says so in the docs instead of
/// pretending a zero.
fn hotspot_score(scc_size: u32, degree: u32, dependents: usize) -> f64 {
    let score = ReviewScore {
        cycle: if scc_size > 0 {
            30u32
                .saturating_add(2u32.saturating_mul(scc_size - 1))
                .min(40)
        } else {
            0
        },
        coupling: degree.saturating_mul(2).min(20),
        impact: dependents.min(20) as u32,
        churn: 0,
    };
    f64::from(score.cycle + score.coupling + score.impact)
}

/// `hotspot × (1 − coverage)`; `None` when coverage is unknown — never
/// computed against an invented zero.
fn risk_gap(hotspot: f64, coverage: Option<f64>) -> Option<f64> {
    coverage.map(|covered| hotspot * (1.0 - covered))
}

/// Transitive dependents of `index` over the incoming adjacency,
/// excluding the node itself.
fn transitive_dependents(index: usize, incoming: &[BTreeSet<usize>]) -> usize {
    let mut seen = BTreeSet::new();
    let mut queue: Vec<usize> = incoming[index].iter().copied().collect();
    while let Some(node) = queue.pop() {
        if seen.insert(node) {
            queue.extend(incoming[node].iter().copied());
        }
    }
    seen.len()
}

/// Line coverage ratio of a file, `None` when the report carries no
/// usable line counter.
fn line_coverage(covered: u64, total: u64) -> Option<f64> {
    (total > 0).then(|| covered as f64 / total as f64)
}

/// Top-`top` risk-weighted coverage gaps, deterministic order: known
/// `gap` descending, then hotspot descending, then path; files without
/// coverage data are ranked last (unknown risk, never assumed clean).
pub fn risk_coverage_gaps(
    root: &Path,
    config: &Config,
    graph: &Graph,
    top: usize,
) -> Result<Vec<RiskCoverageGap>, ArchguardError> {
    let report = read_test_coverage(root, config, graph);
    let mut coverage_by_node: BTreeMap<&str, Option<f64>> = BTreeMap::new();
    for file in &report.files {
        let ratio = file
            .lines
            .as_ref()
            .and_then(|counter| line_coverage(counter.covered, counter.total));
        // First entry wins: the importer already maps one report file per
        // graph node; a duplicate must not silently double-count.
        coverage_by_node.entry(file.node.as_str()).or_insert(ratio);
    }

    let indices: BTreeMap<_, usize> = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id, index))
        .collect();
    let mut outgoing = vec![BTreeSet::new(); graph.nodes.len()];
    let mut incoming = outgoing.clone();
    for edge in &graph.edges {
        if let (Some(&from), Some(&to)) = (indices.get(&edge.from), indices.get(&edge.to))
            && from != to
        {
            outgoing[from].insert(to);
            incoming[to].insert(from);
        }
    }
    let scc_sizes: BTreeMap<_, u32> = find_sccs(graph)
        .iter()
        .flat_map(|scc| {
            scc.node_ids
                .iter()
                .map(|id| (*id, scc.node_ids.len() as u32))
        })
        .collect();

    let mut gaps: Vec<RiskCoverageGap> = graph
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.kind == NodeKind::File)
        .map(|(index, node)| {
            let degree = (incoming[index].len() + outgoing[index].len()) as u32;
            let hotspot = hotspot_score(
                scc_sizes.get(&node.id).copied().unwrap_or(0),
                degree,
                transitive_dependents(index, &incoming),
            );
            let coverage = coverage_by_node.get(node.name.as_str()).copied().flatten();
            RiskCoverageGap {
                gap: risk_gap(hotspot, coverage),
                path: node.name.clone(),
                hotspot,
                coverage,
            }
        })
        .collect();
    gaps.sort_by(|a, b| {
        match (a.gap, b.gap) {
            (Some(x), Some(y)) => y.partial_cmp(&x).unwrap_or(std::cmp::Ordering::Equal),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
        .then_with(|| b.hotspot.total_cmp(&a.hotspot))
        .then_with(|| a.path.cmp(&b.path))
    });
    gaps.truncate(top);
    Ok(gaps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{Edge, Node, NodeId};

    #[test]
    fn hotspots_of_80_with_25_percent_coverage_give_gap_60() {
        assert_eq!(risk_gap(80.0, Some(0.25)), Some(60.0));
        assert_eq!(risk_gap(80.0, Some(1.0)), Some(0.0));
    }

    #[test]
    fn missing_coverage_never_invents_a_gap() {
        assert_eq!(risk_gap(80.0, None), None, "not measured is not clean");
    }

    #[test]
    fn hotspot_matches_the_analysis_review_components() {
        // Cycle of 3: min(30+2·2,40)=34; degree 4 → 8; dependents 5.
        assert_eq!(hotspot_score(3, 4, 5), 34.0 + 8.0 + 5.0);
        // Acyclic hub: coupling and impact only, both capped.
        assert_eq!(hotspot_score(0, 50, 99), 20.0 + 20.0);
    }

    #[test]
    fn zero_total_lines_counts_as_unknown_not_zero_coverage() {
        assert_eq!(line_coverage(0, 0), None);
        assert_eq!(line_coverage(3, 4), Some(0.75));
    }

    fn file_node(id: u32, name: &str) -> Node {
        Node {
            id: NodeId(id),
            name: name.to_string(),
            kind: NodeKind::File,
            parent: None,
            lang: None,
        }
    }

    #[test]
    fn risky_untested_file_outranks_fully_covered_ones() {
        // hot.ts sits in a 2-cycle (hotspot ≥ 30), cold.ts is acyclic
        // with no dependents.
        let graph = Graph {
            semantic: None,
            review_kpis: None,
            schema_version: 1,
            root: "/p".to_string(),
            nodes: vec![
                file_node(0, "hot.ts"),
                file_node(1, "hot_peer.ts"),
                file_node(2, "cold.ts"),
            ],
            edges: vec![
                Edge {
                    from: NodeId(0),
                    to: NodeId(1),
                    refs: vec![],
                    count: None,
                },
                Edge {
                    from: NodeId(1),
                    to: NodeId(0),
                    refs: vec![],
                    count: None,
                },
            ],
        };
        // No imported coverage at all: everything stays unknown.
        let dir = tempfile::tempdir().expect("tempdir");
        let config = Config::default();
        let gaps = risk_coverage_gaps(dir.path(), &config, &graph, 10).expect("gaps");
        assert_eq!(gaps.len(), 3, "every file node is reported");
        assert!(
            gaps.iter().all(|g| g.gap.is_none() && g.coverage.is_none()),
            "missing coverage.json keeps gaps unknown: {gaps:?}"
        );
        // Unknown gaps sort last by hotspot: the cyclic file leads.
        assert_eq!(gaps[0].path, "hot.ts");
        assert!(gaps[0].hotspot >= 30.0);
        assert_eq!(gaps[2].path, "cold.ts");
        assert_eq!(gaps[2].hotspot, 0.0);
    }

    #[test]
    fn signature_is_stable() {
        type RiskGapsFn =
            fn(&Path, &Config, &Graph, usize) -> Result<Vec<RiskCoverageGap>, ArchguardError>;
        let _: RiskGapsFn = risk_coverage_gaps;
    }
}
