//! Evidence projection shared by the landscape viewer and MCP. No quality scores.
use crate::test_coverage::{CoverageCounter, TestCoverageReport};
use archguard_core::{CallAnalysis, Graph, NodeKind};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldCoverage {
    pub scope: String,
    pub status: String,
    pub lines: Option<CoverageCounter>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldEntity {
    pub id: String,
    pub node: String,
    pub class_id: Option<String>,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub max_complexity: Option<u32>,
    pub measured_methods: usize,
    pub declared_methods: usize,
    pub coverage: Option<WorldCoverage>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchitectureWorld {
    pub definition: String,
    pub revision: String,
    pub status: String,
    pub entities: Vec<WorldEntity>,
}

/// Class tiles use the observed maximum source-method cyclomatic complexity.
/// Files without declarations remain present with unknown metrics. Coverage is
/// file-scoped, never class coverage; stale, ambiguous and empty totals stay unknown.
pub fn architecture_world(
    graph: &Graph,
    calls: &CallAnalysis,
    coverage: &TestCoverageReport,
) -> ArchitectureWorld {
    let nodes: BTreeSet<_> = graph
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::File && !n.name.ends_with("/*"))
        .map(|n| n.name.as_str())
        .collect();
    let mut methods = BTreeMap::<_, Vec<_>>::new();
    for method in &calls.methods {
        methods
            .entry(method.class_id.as_str())
            .or_default()
            .push(method);
    }
    let mut by_node = BTreeMap::<_, Vec<_>>::new();
    for row in &coverage.files {
        by_node.entry(row.node.as_str()).or_default().push(row);
    }
    let covered = |node: &str, file: Option<&str>| {
        let candidates: Vec<_> = by_node
            .get(node)?
            .iter()
            .filter(|r| file.is_none_or(|f| f == r.path))
            .collect();
        if candidates.len() != 1 {
            return None;
        }
        let row = candidates[0];
        Some(WorldCoverage {
            scope: "file".into(),
            status: row.status.clone(),
            lines: row
                .lines
                .as_ref()
                .filter(|c| {
                    row.status == "unchanged_since_import" && c.total > 0 && c.covered <= c.total
                })
                .cloned(),
        })
    };
    let mut entities = Vec::new();
    let mut represented = BTreeSet::new();
    for class in &calls.classes {
        if !nodes.contains(class.graph_node.as_str()) {
            continue;
        }
        represented.insert(class.graph_node.as_str());
        let methods = methods
            .get(class.id.as_str())
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let measurements: Vec<_> = methods
            .iter()
            .filter_map(|m| m.complexity.as_ref().map(|v| v.cyclomatic))
            .collect();
        // Partial declarations can span files. A single declaration's coverage
        // must not be attributed to the entire type.
        let single_file = class
            .declaration_files
            .as_ref()
            .is_none_or(|files| files.len() <= 1);
        entities.push(WorldEntity {
            id: format!("class:{}", class.id),
            node: class.graph_node.clone(),
            class_id: Some(class.id.clone()),
            file: Some(class.file.clone()),
            line: Some(class.line),
            max_complexity: measurements.iter().copied().max(),
            measured_methods: measurements.len(),
            declared_methods: methods.len(),
            coverage: single_file
                .then(|| covered(&class.graph_node, Some(&class.file)))
                .flatten(),
        });
    }
    for node in nodes.difference(&represented) {
        entities.push(WorldEntity {
            id: format!("file:{node}"),
            node: (*node).into(),
            class_id: None,
            file: None,
            line: None,
            max_complexity: None,
            measured_methods: 0,
            declared_methods: 0,
            coverage: covered(node, None),
        });
    }
    entities.sort_by(|a, b| a.id.cmp(&b.id));
    let revision = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&(graph, &entities, &calls.status, &coverage.status))
                .expect("serializable world evidence")
        )
    );
    ArchitectureWorld {
        definition: "architecture-world/v1".into(),
        revision,
        status: calls.status.clone(),
        entities,
    }
}
