use archguard_engine::{
    TestCoverageReport, architecture_world,
    core::{CallAnalysis, GraphBuilder},
};
use serde_json::{Value, json};

#[test]
fn world_uses_measured_classes_preserves_unknowns_and_invalidates_revision() {
    let mut builder = GraphBuilder::new();
    builder.ensure_path("p/A.java", None);
    builder.ensure_path("p/B.java", None);
    let graph = builder.build("/project".into());
    let calls: CallAnalysis = serde_json::from_value(json!({"status":"partial","files":1,"classes":[{"id":"p.A","name":"A","package":"p","graph_node":"p/A.java","file":"src/p/A.java","line":1,"kind":"class"}],"methods":[{"id":"p.A#m()","class_id":"p.A","name":"m","signature":"m()","file":"src/p/A.java","line":2,"is_static":false,"complexity":{"cyclomatic":7,"max_nesting":2,"definition":"ast-decisions-v1"}}],"calls":[],"diagnostics":[]})).unwrap();
    let mut coverage: TestCoverageReport = serde_json::from_value(json!({"schema_version":1,"status":"imported","files":[{"node":"p/A.java","path":"src/p/A.java","lines":{"covered":3,"total":10},"source_sha256":"abc","status":"unchanged_since_import"}],"unmatched_files":0,"diagnostics":[]})).unwrap();
    let world = architecture_world(&graph, &calls, &coverage);
    let value = serde_json::to_value(&world).unwrap();
    let entities = value["entities"].as_array().unwrap();
    assert_eq!(entities.len(), 2);
    let class = entities.iter().find(|r| r["class_id"] == "p.A").unwrap();
    assert_eq!(class["max_complexity"], 7);
    assert_eq!(class["measured_methods"], 1);
    assert_eq!(class["coverage"]["lines"]["total"], 10);
    assert_eq!(class["coverage"]["scope"], "file");
    let missing = entities.iter().find(|r| r["node"] == "p/B.java").unwrap();
    assert_eq!(missing["max_complexity"], Value::Null);
    assert_eq!(missing["coverage"], Value::Null);
    assert_eq!(
        world.revision,
        architecture_world(&graph, &calls, &coverage).revision
    );
    coverage.files[0].status = "changed_since_import".into();
    let stale = architecture_world(&graph, &calls, &coverage);
    assert_ne!(world.revision, stale.revision);
    let stale = serde_json::to_value(stale).unwrap();
    assert!(
        stale["entities"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["coverage"]["lines"].is_null())
    );
    coverage.files[0].status = "unchanged_since_import".into();
    coverage.files.push(coverage.files[0].clone());
    let ambiguous = serde_json::to_value(architecture_world(&graph, &calls, &coverage)).unwrap();
    assert!(
        ambiguous["entities"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["coverage"].is_null())
    );
}
