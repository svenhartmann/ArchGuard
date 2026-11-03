use archguard_engine::{class_cohesion, core::CallAnalysis};
use serde_json::json;
fn fixture() -> CallAnalysis {
    serde_json::from_value(json!({"status":"ready","files":1,"diagnostics":[],"classes":[{"id":"C","name":"C","package":"p","file":"C.java","graph_node":"p/C.java","line":1,"kind":"class"}],
    "methods":[
        {"id":"one","class_id":"C","name":"one","signature":"one()","file":"C.java","line":2,"is_static":false,"complexity":{"cyclomatic":2,"max_nesting":1,"definition":"ast-decisions-v1"}},
        {"id":"two","class_id":"C","name":"two","signature":"two()","file":"C.java","line":3,"is_static":false,"complexity":{"cyclomatic":1,"max_nesting":0,"definition":"ast-decisions-v1"}},
        {"id":"three","class_id":"C","name":"three","signature":"three()","file":"C.java","line":4,"is_static":false,"complexity":{"cyclomatic":1,"max_nesting":0,"definition":"ast-decisions-v1"}},
        {"id":"ctor","class_id":"C","name":"<init>","signature":"C()","file":"C.java","line":5,"is_static":false,"complexity":{"cyclomatic":1,"max_nesting":0,"definition":"ast-decisions-v1"}},
        {"id":"static","class_id":"C","name":"utility","signature":"utility()","file":"C.java","line":6,"is_static":true,"complexity":{"cyclomatic":1,"max_nesting":0,"definition":"ast-decisions-v1"}}
    ],"calls":[],"details":{"definition":"source-types-state/v1","type_uses":[],"class_states":[{"class_id":"C","status":"ready","fields":[
        {"id":"left","name":"left","is_static":false,"file":"C.java","line":1},{"id":"right","name":"right","is_static":false,"file":"C.java","line":1},{"id":"shared","name":"shared","is_static":true,"file":"C.java","line":1}],"accesses":[
        {"method":"one","field":"left","file":"C.java","line":2},{"method":"two","field":"right","file":"C.java","line":3},{"method":"three","field":"left","file":"C.java","line":4},
        {"method":"ctor","field":"left","file":"C.java","line":5},{"method":"ctor","field":"right","file":"C.java","line":5},{"method":"one","field":"shared","file":"C.java","line":2},{"method":"two","field":"shared","file":"C.java","line":3}]}]}})).unwrap()
}
#[test]
fn lcom4_uses_shared_instance_state_and_internal_calls_without_constructor_bridges() {
    let mut calls = fixture();
    let rows = class_cohesion(&calls);
    assert_eq!(rows[0].lcom4, Some(2));
    assert_eq!(rows[0].methods, 3);
    assert_eq!(rows[0].fields, 2);
    assert_eq!(rows[0].groups, vec![vec!["one", "three"], vec!["two"]]);
    assert_eq!(rows[0].cyclomatic_sum, Some(4));
    calls.calls.push(serde_json::from_value(json!({"from_class":"C","from_method":"one","to_class":"C","to_method":"two","target_label":"two()","dispatch":"instance","resolution":"project","file":"C.java","line":2,"column":1,"code":"two()"})).unwrap());
    assert_eq!(class_cohesion(&calls)[0].lcom4, Some(1));
    calls.calls[0].resolution = "unresolved".into();
    assert_eq!(
        class_cohesion(&calls)[0].lcom4,
        None,
        "unresolved calls cannot establish a complete LCOM4 measurement"
    );
}
#[test]
fn absent_and_partial_state_evidence_never_becomes_zero_cohesion() {
    let mut calls = fixture();
    calls.details.as_mut().unwrap().class_states[0].status = "partial".into();
    assert_eq!(class_cohesion(&calls)[0].lcom4, None);
    calls.details = None;
    assert_eq!(class_cohesion(&calls)[0].status, "unavailable");
    assert_eq!(class_cohesion(&calls)[0].lcom4, None);
}
