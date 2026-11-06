//! Graph and Matrix snapshots: the last branch of `exportSnapshot()`
//! (`kind` "imports" or "dependencies").

use archguard_engine::core::{Ref, SemanticDependencyCoverage};
use serde_json::{Value, json};

use super::snapshot::{Snapshot, add_filters, base, extend};

/// A node as shown: the tree row's full name and kind.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphNode {
    pub name: String,
    /// "file" or "module".
    pub kind: String,
}

/// A directed connection between two shown nodes (by name).
#[derive(Debug, Clone, PartialEq)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    pub count: u64,
    /// False when `count` is a lower bound (legacy graph without counts).
    pub count_exact: bool,
    /// The edge lies in a cycle of the current scope.
    pub cyclic: bool,
    /// The evidence sample.
    pub evidence: Vec<Ref>,
}

/// Display settings of the Graph tab, under the web's names and values.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GraphFilters {
    /// `ui.diagram`: "overview" for the package tree, "network".
    pub diagram: String,
    /// Name of the selected node, if any.
    pub reference: Option<String>,
    /// `ui.edgeMode`: "all", "incoming", "outgoing", "both", "cycles".
    pub direction: String,
    pub show_files: bool,
    pub cycles_only: bool,
    pub search: String,
}

/// Everything the Graph or Matrix export needs, as plain data.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GraphInput {
    pub project: String,
    /// Matrix tab: complete scope, no display filters.
    pub matrix: bool,
    /// Compiler coverage of a semantic dependency graph, if any.
    pub semantic: Option<SemanticDependencyCoverage>,
    pub filters: GraphFilters,
    pub scope_nodes: usize,
    pub scope_dependencies: usize,
    pub cyclic_groups: usize,
    pub acd: f64,
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

const NOTES: [&str; 2] = [
    "ACD and cyclic groups describe the full analysis scope, before display filters. ACD is Average Component Dependency: average transitive dependency count, including each starting node itself. It does not measure runtime performance.",
    "Reference counts are exact when count_exact is true; otherwise they are lower bounds. Evidence may be a sample. Coarser package-cycle context does not imply a cycle at the current detail level.",
];

/// The snapshot of the Graph tab (or the Matrix tab with `matrix`).
pub fn graph_snapshot(input: &GraphInput) -> Snapshot {
    let semantic = input.semantic.is_some();
    let mut map = base(&input.project, if input.matrix { "dsm" } else { "graph" });
    let coverage = if input.matrix {
        "Complete analysis scope across all matrix tiles."
    } else {
        "Shown graph nodes and directed connections after file, direction and cycle filters."
    };
    extend(
        &mut map,
        json!({"kind": if semantic { "dependencies" } else { "imports" }, "coverage": coverage}),
    );
    if let Some(s) = &input.semantic {
        extend(
            &mut map,
            json!({"semantic": s, "diagnostics": s.diagnostics}),
        );
    }
    add_filters(&mut map, filters(input));
    extend(
        &mut map,
        json!({
            "metrics": {
                "scope_nodes": input.scope_nodes,
                "scope_dependencies": input.scope_dependencies,
                "shown_nodes": input.nodes.len(),
                "shown_dependencies": input.edges.len(),
                "cyclic_groups": input.cyclic_groups,
                "acd": input.acd,
            },
            "nodes": input.nodes.iter().map(|n| json!({"id": n.name, "name": n.name, "kind": n.kind})).collect::<Vec<_>>(),
            "edges": input.edges.iter().map(|e| edge(e, semantic)).collect::<Vec<_>>(),
            "simulation_ops": [],
            "count_unit": if semantic { "dependency evidence records" } else { "import references" },
            "notes": notes(input),
        }),
    );
    Snapshot(Value::Object(map))
}

fn filters(input: &GraphInput) -> Value {
    let f = &input.filters;
    if input.matrix {
        return json!({"diagram": "matrix", "reference": f.reference});
    }
    json!({
        "diagram": f.diagram,
        "reference": f.reference,
        "direction": f.direction,
        "show_files": f.show_files,
        "cycles_only": f.cycles_only,
        "search": f.search,
        "search_behavior": "Search dims graph nodes; it does not remove connections.",
    })
}

fn edge(e: &GraphEdge, semantic: bool) -> Value {
    json!({
        "from": e.from,
        "to": e.to,
        "kind": if semantic { "dependency" } else { "import" },
        "count": e.count,
        "count_exact": e.count_exact,
        "cyclic_current_scope": e.cyclic,
        "evidence": e.evidence,
    })
}

fn notes(input: &GraphInput) -> Vec<String> {
    let mut notes: Vec<String> = NOTES.iter().map(|n| n.to_string()).collect();
    if let Some(s) = &input.semantic {
        notes.push(format!(
            "Dependency graph: imports + resolved calls + supertypes + available declaration types. Compiler coverage: {} (original source snapshot). Missing references, unsupported syntax and runtime wiring remain unknown.",
            s.status
        ));
    }
    notes
}

#[cfg(test)]
pub(super) fn sample() -> GraphInput {
    let node = |name: &str| GraphNode {
        name: name.into(),
        kind: "module".into(),
    };
    let edge = |from: &str, to: &str, count| GraphEdge {
        from: from.into(),
        to: to.into(),
        count,
        count_exact: true,
        cyclic: true,
        evidence: vec![Ref {
            file: format!("{from}/index.ts"),
            line: 1,
            raw: format!("import '../{to}'"),
        }],
    };
    GraphInput {
        project: "ts-cyclic".into(),
        filters: GraphFilters {
            diagram: "overview".into(),
            direction: "all".into(),
            ..Default::default()
        },
        scope_nodes: 2,
        scope_dependencies: 2,
        cyclic_groups: 1,
        acd: 2.0,
        nodes: vec![node("src/a"), node("src/b")],
        edges: vec![edge("src/a", "src/b", 3), edge("src/b", "src/a", 1)],
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_json_has_the_web_shape() {
        let snapshot = graph_snapshot(&sample());
        let v: Value = serde_json::from_str(&snapshot.json()).expect("valid JSON");
        assert_eq!(v["schema_version"], 1);
        assert_eq!(v["kind"], "imports");
        assert_eq!(v["filters"]["workspace"], "graph");
        assert_eq!(v["filters"]["direction"], "all");
        assert_eq!(v["metrics"]["shown_nodes"], 2);
        assert_eq!(v["metrics"]["shown_dependencies"], 2);
        assert_eq!(v["metrics"]["cyclic_groups"], 1);
        assert_eq!(v["edges"][0]["count"], 3);
        assert_eq!(v["edges"][0]["evidence"][0]["file"], "src/a/index.ts");
        assert_eq!(v["count_unit"], "import references");
        assert!(
            v["coverage"]
                .as_str()
                .unwrap()
                .starts_with("Shown graph nodes")
        );
        assert_eq!(v["notes"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn matrix_exports_the_complete_scope_without_display_filters() {
        let input = GraphInput {
            matrix: true,
            ..sample()
        };
        let v = graph_snapshot(&input).0;
        assert_eq!(v["filters"]["workspace"], "dsm");
        assert_eq!(v["filters"]["diagram"], "matrix");
        assert!(v["filters"].get("direction").is_none());
        assert_eq!(
            v["coverage"],
            "Complete analysis scope across all matrix tiles."
        );
    }

    #[test]
    fn graph_markdown_has_a_mermaid_block_with_counts() {
        let md = graph_snapshot(&sample()).markdown();
        assert!(md.starts_with("# Architecture: ts-cyclic\n"), "{md}");
        assert!(
            md.contains(
                "```mermaid\nflowchart LR\n  n0[\"src/a\"]\n  n1[\"src/b\"]\n  n0 -->|\"3\"| n1\n"
            ),
            "{md}"
        );
        assert!(md.contains("| Shown dependencies | 2 |"), "{md}");
        assert!(md.contains("| ACD in analysis scope | 2.00 |"), "{md}");
        assert!(md.contains("| direction | all |"), "{md}");
        assert!(
            md.contains("| src/a | src/b | import | 3 | yes | no |"),
            "{md}"
        );
        assert!(md.contains("## Source evidence"), "{md}");
    }
}
