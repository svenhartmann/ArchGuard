//! Review ordering from imported line coverage and observed file dependents.
use crate::test_coverage::TestCoverageReport;
use archguard_core::{Graph, NodeId, NodeKind};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoverageReviewFile {
    pub node: String,
    pub path: String,
    pub dependents: Option<u32>,
    pub score: Option<f64>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoverageReview {
    pub definition: String,
    pub semantic_status: Option<String>,
    pub files: Vec<CoverageReviewFile>,
}

pub fn test_coverage_review(graph: &Graph, coverage: &TestCoverageReport) -> CoverageReview {
    let mut report = CoverageReview {
        definition: "uncovered-line-fraction-times-impact-v1".into(),
        semantic_status: graph.semantic.as_ref().map(|s| s.status.clone()),
        files: Vec::new(),
    };
    if coverage.files.is_empty() {
        return report;
    }
    let ids: BTreeMap<_, _> = graph
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::File && !n.name.ends_with("/*"))
        .map(|n| (n.name.as_str(), n.id))
        .collect();
    let known: BTreeSet<_> = ids.values().copied().collect();
    let mut incoming = BTreeMap::<NodeId, BTreeSet<NodeId>>::new();
    for edge in &graph.edges {
        if known.contains(&edge.from) && known.contains(&edge.to) {
            incoming.entry(edge.to).or_default().insert(edge.from);
        }
    }
    for file in &coverage.files {
        let dependents = ids.get(file.node.as_str()).map(|start| {
            let mut seen = BTreeSet::from([*start]);
            let mut stack = vec![*start];
            while let Some(id) = stack.pop() {
                for next in incoming.get(&id).into_iter().flatten() {
                    if seen.insert(*next) {
                        stack.push(*next);
                    }
                }
            }
            (seen.len() - 1) as u32
        });
        let score = dependents.and_then(|impact| {
            file.lines
                .as_ref()
                .filter(|lines| {
                    matches!(coverage.status.as_str(), "ready" | "partial" | "stale")
                        && file.status == "unchanged_since_import"
                        && lines.total > 0
                        && lines.covered <= lines.total
                })
                .map(|lines| {
                    (1.0 - lines.covered as f64 / lines.total as f64) * (1.0 + f64::from(impact))
                })
        });
        report.files.push(CoverageReviewFile {
            node: file.node.clone(),
            path: file.path.clone(),
            dependents,
            score,
        });
    }
    report.files.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.node.cmp(&b.node))
            .then_with(|| a.path.cmp(&b.path))
    });
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{GraphBuilder, Ref};
    use serde_json::json;

    #[test]
    fn impact_counts_distinct_transitive_files_without_self_or_namespace_placeholders() {
        let mut builder = GraphBuilder::new();
        let a = builder.ensure_path("lib/a.ts", None);
        let b = builder.ensure_path("lib/b.ts", None);
        let c = builder.ensure_path("lib/c.ts", None);
        let unknown = builder.ensure_path("other/*", None);
        for (from, to) in [(a, a), (a, b), (b, a), (c, b), (c, a), (unknown, a)] {
            builder
                .edge(
                    from,
                    to,
                    Ref {
                        file: "source.ts".into(),
                        line: 1,
                        raw: "import".into(),
                    },
                )
                .unwrap();
        }
        let graph = builder.build("/project".into());
        let mut coverage:TestCoverageReport=serde_json::from_value(json!({
            "schema_version":1,"status":"ready","format":"cobertura","report_path":"coverage.xml",
            "imported_at_unix":1,"report_modified_unix":1,"unmatched_files":0,"diagnostics":[],
            "files":[{"node":"lib/a.ts","path":"lib/a.ts","status":"unchanged_since_import","lines":{"covered":1,"total":4},"branches":null,"source_sha256":"x"},
                     {"node":"lib/b.ts","path":"lib/b.ts","status":"unchanged_since_import","lines":{"covered":4,"total":4},"branches":null,"source_sha256":"x"},
                     {"node":"gone.ts","path":"gone.ts","status":"changed_since_import","lines":{"covered":0,"total":1},"branches":null,"source_sha256":"x"}]
        })).unwrap();
        let ranked = test_coverage_review(&graph, &coverage);
        assert_eq!(ranked.files[0].node, "lib/a.ts");
        assert_eq!(ranked.files[0].dependents, Some(2));
        assert_eq!(ranked.files[0].score, Some(2.25));
        assert_eq!(ranked.files[1].score, Some(0.0));
        assert_eq!(ranked.files[2].dependents, None);
        assert_eq!(ranked.files[2].score, None);
        coverage.files[0].status = "changed_since_import".into();
        assert!(
            test_coverage_review(&graph, &coverage)
                .files
                .iter()
                .find(|r| r.node == "lib/a.ts")
                .unwrap()
                .score
                .is_none()
        );
        coverage.status = "error".into();
        assert!(
            test_coverage_review(&graph, &coverage)
                .files
                .iter()
                .all(|r| r.score.is_none()),
            "freshness verification failure must not retain numeric review scores"
        );
        coverage.status = "ready".into();
        coverage.files[0].status = "unchanged_since_import".into();
        coverage.files[0].lines = None;
        assert!(
            test_coverage_review(&graph, &coverage)
                .files
                .iter()
                .find(|r| r.node == "lib/a.ts")
                .unwrap()
                .score
                .is_none()
        );
    }
}
