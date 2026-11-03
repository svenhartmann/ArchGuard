use archguard_engine::{code_metrics, core::CallAnalysis};
use serde_json::{Value, json};

fn fixture() -> CallAnalysis {
    let classes: Vec<_> = ["A", "B", "C"].iter().map(|id| json!({"id":id,"name":id,"language":"java","package":"p","file":format!("{id}.java"),"graph_node":format!("p/{id}.java"),"line":1,"kind":"class"})).collect();
    serde_json::from_value(json!({"status":"ready","files":3,"classes":classes,"methods":[
        {"id":"A#one()","class_id":"A","name":"one","signature":"one()","file":"A.java","line":2,"is_static":false,"complexity":{"cyclomatic":2,"max_nesting":1,"definition":"ast-decisions-v1"}},
        {"id":"A#two()","class_id":"A","name":"two","signature":"two()","file":"A.java","line":3,"is_static":true,"complexity":{"cyclomatic":1,"max_nesting":0,"definition":"ast-decisions-v1"}}
    ],"calls":[
        {"from_class":"A","from_method":"A#one()","to_class":"B","to_method":"B#run()","target_label":"B#run()","dispatch":"instance","resolution":"project","file":"A.java","line":2,"column":1,"code":"b.run()"},
        {"from_class":"A","from_method":"A#two()","to_class":"B","to_method":"B#run()","target_label":"B#run()","dispatch":"instance","resolution":"project","file":"A.java","line":3,"column":1,"code":"b.run()"}
    ],"diagnostics":[],"details":{"enumeration_complete":true,"definition":"source-types-state/v1","class_states":[{"class_id":"A","status":"ready","fields":[],"accesses":[]}],"type_uses":[
        {"from_class":"A","from_method":null,"to_class":"B","target_label":"B","role":"field","resolution":"project","file":"A.java","line":1,"code":"B"},
        {"from_class":"A","from_method":null,"to_class":"C","target_label":"C","role":"return","resolution":"project","file":"A.java","line":1,"code":"C"}
    ],"inheritance":[{"class_id":"A","base_class":"B","depth":2,"status":"ready"},{"class_id":"B","base_class":"C","depth":1,"status":"ready"},{"class_id":"C","base_class":null,"depth":0,"status":"ready"}],
    "method_metrics":[{"method_id":"A#one()","definition":"source-maintainability/v1","cognitive":3,"distinct_operators":2,"distinct_operands":2,"operators":3,"operands":5,"source_lines":2}]}})).unwrap()
}

#[test]
fn class_metrics_deduplicate_couplings_and_response_targets_and_keep_evidence() {
    let value = serde_json::to_value(code_metrics(&fixture())).unwrap();
    let a = &value["classes"][0];
    assert_eq!(a["cbo"]["value"], 2);
    assert_eq!(a["cbo"]["evidence"], json!(["B", "C"]));
    assert_eq!(a["rfc"]["value"], 3);
    assert_eq!(a["dit"]["value"], 2);
    assert_eq!(value["classes"][1]["noc"]["value"], 1);
    assert_eq!(a["wmc"]["value"], 3);
    let method = &value["methods"][0];
    assert_eq!(
        method["node"], "p/A.java",
        "metric locations retain the logical graph identity"
    );
    assert_eq!(method["cognitive"], 3);
    assert_eq!(method["halstead"]["volume"], 16.0);
    assert_eq!(method["halstead"]["difficulty"], 2.5);
    assert_eq!(method["halstead"]["effort"], 40.0);
    let expected = (171.0 - 5.2 * 16f64.ln() - 0.23 * 2.0 - 16.2 * 2f64.ln()) * 100.0 / 171.0;
    assert!((method["maintainability_index"].as_f64().unwrap() - expected).abs() < 1e-9);
}

#[test]
fn incomplete_semantics_and_legacy_reports_never_manufacture_zero_metrics() {
    let mut calls = fixture();
    calls.details.as_mut().unwrap().class_states[0].status = "partial".into();
    calls.status = "partial".into();
    calls.calls[0].resolution = "unresolved".into();
    let value = serde_json::to_value(code_metrics(&calls)).unwrap();
    assert_eq!(value["classes"][0]["cbo"]["value"], Value::Null);
    assert_eq!(value["classes"][0]["rfc"]["value"], Value::Null);
    assert_eq!(value["classes"][0]["cbo"]["observed"], 2);
    assert_eq!(value["classes"][1]["noc"]["value"], Value::Null);
    calls.details = None;
    let value = serde_json::to_value(code_metrics(&calls)).unwrap();
    assert_eq!(value["classes"][0]["dit"]["value"], Value::Null);
    assert!(
        value["methods"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m["cognitive"].is_null())
    );
}
#[test]
fn capped_call_evidence_does_not_claim_complete_coupling_or_response_sets() {
    let mut value = serde_json::to_value(fixture()).unwrap();
    value["details"]["enumeration_complete"] = json!(false);
    let calls: CallAnalysis = serde_json::from_value(value).unwrap();
    let metrics = code_metrics(&calls);
    assert!(
        metrics.classes[0].cbo.value.is_none(),
        "an omitted call may introduce another coupled class"
    );
    assert!(
        metrics.classes[0].rfc.value.is_none(),
        "an omitted call may introduce another response target"
    );
    assert!(
        archguard_engine::class_cohesion(&calls)[0].lcom4.is_none(),
        "an omitted internal call may join cohesion groups"
    );
}
