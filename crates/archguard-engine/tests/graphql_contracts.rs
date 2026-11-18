use archguard_core::ServiceDependencyKind;
use archguard_engine::{analyze_services, read_api_contracts, save_graphql_baseline};
use serde_json::Value;
use std::{fs, path::Path};

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/graphql-services")
}
fn copy(from: &Path, to: &Path) {
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            fs::create_dir_all(&target).unwrap();
            copy(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}
fn id(root: &Path, name: &str) -> String {
    analyze_services(root)
        .unwrap()
        .services
        .into_iter()
        .find(|s| s.name == name)
        .unwrap()
        .id
}
fn graphql(root: &Path) -> Value {
    let topology = analyze_services(root).unwrap();
    serde_json::to_value(read_api_contracts(root, &topology)).unwrap()["graphql"].clone()
}

#[test]
fn schemas_belong_to_their_service_and_operations_link_to_the_one_matching_provider() {
    let root = fixture();
    let (users, products, web, admin) = (
        id(&root, "users"),
        id(&root, "products"),
        id(&root, "web"),
        id(&root, "admin"),
    );
    let report = graphql(&root);
    assert_eq!(report["status"], "ready", "{report:#}");
    let schemas = report["schemas"].as_array().unwrap();
    let schema = |p: &str| schemas.iter().find(|s| s["provider"] == p).unwrap();
    assert_eq!(
        schema(&users)["operations"],
        serde_json::json!([
            "Query.node",
            "Query.user",
            "Query.users",
            "Mutation.createUser"
        ])
    );
    assert_eq!(schema(&products)["files"][0], "products/src/schema.ts");
    assert!(
        schema(&users)["directives"]
            .as_array()
            .unwrap()
            .contains(&"User @key(fields: \"id\")".into())
    );
    assert_eq!(
        schema(&users)["types"]["Mutation"]["fields"]["createUser"]["arguments"]["email"]["type"],
        "String"
    );

    let links: Vec<(String, String, String, String)> = report["links"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| {
            (
                l["consumer"].as_str().unwrap().into(),
                l["provider"].as_str().unwrap().into(),
                l["operation"].as_str().unwrap().into(),
                l["evidence"].as_str().unwrap().into(),
            )
        })
        .collect();
    for expected in [
        (&web, &users, "query GetUser", "operation_document"),
        (&web, &products, "query GetProduct", "operation_document"),
        (&admin, &users, "mutation CreateUser", "operation_document"),
        (&web, &users, "", "configured_endpoint"),
    ] {
        assert!(
            links.contains(&(
                expected.0.clone(),
                expected.1.clone(),
                expected.2.into(),
                expected.3.into()
            )),
            "{expected:?} missing in {links:#?}"
        );
    }
    assert_eq!(links.len(), 4, "{links:#?}");
    let get_user = &report["links"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["operation"] == "query GetUser")
        .unwrap();
    assert_eq!(get_user["file"], "web/src/api.ts");
    assert_eq!(get_user["line"], 8);
    assert_eq!(get_user["root_fields"], serde_json::json!(["Query.user"]));

    let unresolved = report["unresolved"].as_array().unwrap();
    let reason = |op: &str| {
        unresolved.iter().find(|u| u["operation"] == op).unwrap()["reason"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert!(reason("query Node").contains("match 2 schemas"));
    assert!(reason("query Orders").contains("No GraphQL schema"));
    assert_eq!(report["comparison_status"], "no_baseline");
}

#[test]
fn linked_operations_are_service_connection_evidence() {
    let root = fixture();
    let topology = analyze_services(&root).unwrap();
    let (users, web) = (id(&root, "users"), id(&root, "web"));
    let edge = topology
        .dependencies
        .iter()
        .find(|d| d.from == web && d.to == users && d.kind == ServiceDependencyKind::HttpCall)
        .expect("GraphQL operation becomes an HTTP call edge");
    assert!(
        edge.evidence
            .iter()
            .any(|e| e.pointer == "graphql:query GetUser" && e.line == Some(8))
    );
    let products = id(&root, "products");
    assert!(
        !topology
            .dependencies
            .iter()
            .any(|d| d.to == products
                && d.evidence.iter().any(|e| e.pointer == "graphql:query Node")),
        "ambiguous operations add no edge"
    );
}

#[test]
fn removed_fields_and_stricter_arguments_are_breaking_against_the_saved_baseline() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    copy(&fixture(), root);
    let topology = analyze_services(root).unwrap();
    assert_eq!(save_graphql_baseline(root, &topology).unwrap(), 2);
    let report = graphql(root);
    assert_eq!(report["comparison_status"], "compared");
    assert_eq!(report["changes"], serde_json::json!([]));

    let path = root.join("users/schema.graphql");
    let schema = fs::read_to_string(&path).unwrap();
    fs::write(
        &path,
        schema
            .replace(
                "createUser(name: String!, email: String)",
                "createUser(name: String!, email: String!)",
            )
            .replace("  email: String\n}", "}")
            .replace("interface Node {\n  id: ID!\n}\n", ""),
    )
    .unwrap();
    let report = graphql(root);
    let changes = report["changes"].as_array().unwrap();
    let find = |kind: &str, at: &str| {
        changes
            .iter()
            .find(|c| c["kind"] == kind && c["operation"] == at)
            .unwrap_or_else(|| panic!("{kind} {at} missing in {changes:#?}"))
    };
    let stricter = find(
        "argument_nullability_changed",
        "Mutation.createUser(email:)",
    );
    assert_eq!(stricter["severity"], "breaking");
    assert_eq!(
        stricter["consumers"],
        serde_json::json!([id(root, "admin")])
    );
    assert_eq!(find("field_removed", "User.email")["severity"], "breaking");
    assert_eq!(find("type_removed", "Node")["severity"], "breaking");
}
