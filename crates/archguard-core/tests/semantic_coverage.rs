use archguard_core::{Cut, Graph, GraphBuilder, Report, aggregate};
use serde_json::json;

#[test]
fn semantic_dependency_coverage_survives_wire_format_and_package_projection() {
    let mut builder = GraphBuilder::new();
    builder.ensure_path("example/A.cs", Some("csharp"));
    let mut value = serde_json::to_value(builder.build("example".into())).unwrap();
    let coverage = json!({
        "status":"partial", "analyzers":[{
            "language":"csharp", "backend":"roslyn", "status":"partial",
            "files":1, "version":"8.0", "project":"app.csproj",
            "diagnostics":["Missing reference"]
        }],
        "projected_calls":2, "projected_supertypes":1,
        "projected_type_uses":0, "skipped_type_uses":0,
        "skipped_project_calls":1,
        "diagnostics":["Calls and inheritance only; type-only dependencies are not included."]
    });
    value["semantic"] = coverage.clone();
    let graph: Graph = serde_json::from_value(value).unwrap();
    assert_eq!(serde_json::to_value(&graph).unwrap()["semantic"], coverage);
    let view = aggregate(&graph, &Cut::at_depth(&graph, 0));
    assert_eq!(serde_json::to_value(view).unwrap()["semantic"], coverage);
}

#[test]
fn legacy_graph_keeps_absent_coverage_distinct_from_complete_coverage() {
    let value = json!({"schema_version":1,"root":"example","nodes":[],"edges":[]});
    let graph: Graph = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(graph).unwrap(), value);
}

#[test]
fn report_roundtrip_retains_coverage_and_accepts_legacy_reports() {
    let legacy = json!({
        "schema_version":1,
        "summary":{"nodes":0,"edges":0,"sccs":0,"largest_scc":0,
            "layer_violations":0,"status":"pass","ccd":0,"acd":0.0,"nccd":0.0},
        "config":{},"findings":[],"metrics":{}
    });
    let report: Report = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(serde_json::to_value(report).unwrap(), legacy);
    let mut current = legacy;
    current["semantic"] = json!({"status":"unavailable", "analyzers":[],
        "projected_calls":0,"projected_supertypes":0,
        "projected_type_uses":0,"skipped_type_uses":0,"skipped_project_calls":0,
        "diagnostics":["Compiler is unavailable"]});
    let report: Report = serde_json::from_value(current.clone()).unwrap();
    assert_eq!(serde_json::to_value(report).unwrap(), current);
}
