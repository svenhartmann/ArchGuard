//! `service_topology` with the optional `paths` project set (ticket 29, ADR 0024).
use crate::server::ArchguardTools;
use crate::tools::MAX_RESPONSE_BYTES;
use rmcp::handler::server::wrapper::Parameters;
use serde_json::{Value, json};
use std::path::Path;

fn set_tools() -> ArchguardTools {
    ArchguardTools::new(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/es-set"))
}

async fn topology(tools: &ArchguardTools, args: Value) -> Result<Value, rmcp::ErrorData> {
    let params = serde_json::from_value(args).unwrap();
    let text = tools.service_topology(Parameters(params)).await?;
    Ok(serde_json::from_str(&text).unwrap())
}

fn findings(value: &Value, kind: &str) -> usize {
    value["elasticsearch"]["findings"]
        .as_array()
        .map_or(0, |all| all.iter().filter(|f| f["kind"] == kind).count())
}

#[tokio::test]
async fn without_paths_the_output_has_no_set_keys() {
    let tools = set_tools();
    let plain = topology(&tools, json!({"path": "adapter"})).await.unwrap();
    assert!(plain["elasticsearch"].get("projects").is_none(), "{plain}");
    assert!(plain.to_string().find("origins").is_none());
    assert_eq!(findings(&plain, "undeclared_index"), 1);
}

#[tokio::test]
async fn paths_merge_infra_and_adapter() {
    let tools = set_tools();
    let plain = topology(&tools, json!({"path": "adapter"})).await.unwrap();
    let set = topology(&tools, json!({"paths": ["infra", "adapter"]}))
        .await
        .unwrap();
    assert_eq!(findings(&set, "undeclared_index"), 0, "{set}");
    assert_eq!(
        set["elasticsearch"]["projects"],
        json!(["infra", "adapter"])
    );
    assert!(
        set["elasticsearch"]["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["origins"] == json!(["infra"]))
    );
    let keys =
        |v: &Value| -> Vec<String> { v["totals"].as_object().unwrap().keys().cloned().collect() };
    assert_eq!(keys(&set), keys(&plain));
}

#[tokio::test]
async fn paths_outside_the_server_root_are_rejected_without_a_read() {
    let tools = set_tools();
    let err = topology(&tools, json!({"paths": ["infra", "../../.."]}))
        .await
        .unwrap_err();
    assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert!(err.message.contains("outside"), "{}", err.message);
}

#[tokio::test]
async fn path_and_paths_together_and_wrong_counts_are_invalid() {
    let tools = set_tools();
    let many: Vec<String> = (0..17).map(|_| "infra".into()).collect();
    for args in [
        json!({"path": "infra", "paths": ["infra", "adapter"]}),
        json!({"paths": ["infra"]}),
        json!({"paths": many}),
    ] {
        let err = topology(&tools, args.clone()).await.unwrap_err();
        assert_eq!(err.code, rmcp::model::ErrorCode::INVALID_PARAMS, "{args}");
    }
}

#[test]
fn the_schema_lists_paths_with_its_description() {
    let tools = ArchguardTools::full_router().list_all();
    let tool = tools.iter().find(|t| t.name == "service_topology").unwrap();
    let schema = Value::Object((*tool.input_schema).clone());
    let description = schema["properties"]["paths"]["description"]
        .as_str()
        .unwrap();
    assert!(description.contains("origins"), "{description}");
}

#[tokio::test]
async fn a_large_set_reply_stays_within_the_budget() {
    let dir = tempfile::tempdir().unwrap();
    for project in ["a", "b"] {
        let mut tf = String::new();
        for i in 0..400 {
            tf += &format!(
                "resource \"elasticstack_elasticsearch_index\" \"i{i}\" {{\n  name = \"{project}-index-{i}\"\n}}\n"
            );
        }
        std::fs::create_dir_all(dir.path().join(project)).unwrap();
        std::fs::write(dir.path().join(project).join("main.tf"), tf).unwrap();
    }
    let tools = ArchguardTools::new(dir.path().into());
    let params = serde_json::from_value(json!({"paths": ["a", "b"]})).unwrap();
    let text = tools.service_topology(Parameters(params)).await.unwrap();
    let value: Value = serde_json::from_str(&text).unwrap();
    assert!(text.len() <= MAX_RESPONSE_BYTES, "{}", text.len());
    assert_eq!(value["truncated"], true);
    assert_eq!(value["totals"]["elasticsearch_artifacts"], 800);
}
