use archguard_engine::{analyze_services, import_api_contract, read_api_contracts};
use serde_json::{Value, json};
use std::{fs, path::Path};

fn root() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("compose.yaml"), "services:\n  orders:\n    image: orders\n  shop:\n    image: shop\n    environment:\n      ORDERS_URL: http://orders:8080\n").unwrap();
    root
}
fn service(root: &Path, name: &str) -> String {
    analyze_services(root)
        .unwrap()
        .services
        .iter()
        .find(|s| s.name == name)
        .unwrap()
        .id
        .clone()
}
fn doc() -> Value {
    json!({"openapi":"3.1.0","info":{"title":"Orders","version":"1"},"paths":{
        "/orders":{"get":{"operationId":"list","responses":{"200":{"description":"OK","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Order"}}}}}}},
        "/orders/{id}":{"post":{"responses":{"204":{"description":"OK"}}}}
    },"components":{"schemas":{"Order":{"type":"object","properties":{"id":{"type":"integer"}}}}}})
}
#[test]
fn contract_changes_keep_exact_operations_and_declared_consumers() {
    let dir = root();
    let root = dir.path();
    let topology = analyze_services(root).unwrap();
    let provider = service(root, "orders");
    let consumer = service(root, "shop");
    fs::write(root.join("api.json"), doc().to_string()).unwrap();
    fs::write(
        root.join("usage.json"),
        json!({"schema_version":1,"usages":[{"consumer":consumer,"operation":"GET /orders"}]})
            .to_string(),
    )
    .unwrap();
    let first = import_api_contract(
        root,
        &topology,
        Path::new("api.json"),
        &provider,
        Some(Path::new("usage.json")),
    )
    .unwrap();
    let first = serde_json::to_value(first).unwrap();
    assert_eq!(
        first["contracts"][0]["operations"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(first["contracts"][0]["comparison_status"], "no_baseline");
    assert_eq!(
        first["contracts"][0]["operations"][0]["response_references"],
        json!(["#/components/schemas/Order"])
    );
    let mut next = doc();
    next["paths"]["/orders/{id}"]["post"]["parameters"] =
        json!([{"name":"token","in":"query","required":true,"schema":{"type":"string"}}]);
    next["components"]["schemas"]["Order"]["properties"]["id"]["type"] = json!("string");
    fs::write(root.join("api.json"), next.to_string()).unwrap();
    let second = serde_json::to_value(
        import_api_contract(root, &topology, Path::new("api.json"), &provider, None).unwrap(),
    )
    .unwrap();
    let changes = second["contracts"][0]["changes"].as_array().unwrap();
    let response = changes
        .iter()
        .find(|v| v["kind"] == "response_schema_changed")
        .unwrap();
    assert_eq!(response["operation"], "GET /orders");
    assert_eq!(response["consumers"], json!([consumer]));
    assert!(
        changes
            .iter()
            .any(|v| v["kind"] == "required_parameter_added"
                && v["operation"] == "POST /orders/{id}")
    );
    next["paths"].as_object_mut().unwrap().remove("/orders");
    fs::write(root.join("api.json"), next.to_string()).unwrap();
    let third = serde_json::to_value(
        import_api_contract(root, &topology, Path::new("api.json"), &provider, None).unwrap(),
    )
    .unwrap();
    assert!(
        third["contracts"][0]["changes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["kind"] == "operation_removed" && v["consumers"] == json!([consumer]))
    );
}
#[test]
fn unresolved_refs_and_unmapped_consumers_never_look_complete() {
    let dir = root();
    let root = dir.path();
    let topology = analyze_services(root).unwrap();
    let mut spec = doc();
    spec["paths"]["/orders"]["get"]["responses"]["200"]["content"]["application/json"]["schema"]
        ["$ref"] = json!("https://example.invalid/schema.json");
    fs::write(root.join("api.json"), spec.to_string()).unwrap();
    let report = serde_json::to_value(
        import_api_contract(
            root,
            &topology,
            Path::new("api.json"),
            "missing-service",
            None,
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(report["status"], "partial");
    assert_eq!(report["contracts"][0]["mapped_service"], false);
    assert!(
        report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .any(|d| d.as_str().unwrap().contains("reference"))
    );
    assert!(
        report["contracts"][0]["usages"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn invalid_import_preserves_snapshot_and_file_changes_mark_stale() {
    let dir = root();
    let root = dir.path();
    let topology = analyze_services(root).unwrap();
    let provider = service(root, "orders");
    fs::write(root.join("api.json"), doc().to_string()).unwrap();
    import_api_contract(root, &topology, Path::new("api.json"), &provider, None).unwrap();
    let old = fs::read(root.join(".archguard/contracts.json")).unwrap();
    fs::write(root.join("bad.json"), r#"{"openapi":"2.0"}"#).unwrap();
    assert!(import_api_contract(root, &topology, Path::new("bad.json"), &provider, None).is_err());
    assert_eq!(
        fs::read(root.join(".archguard/contracts.json")).unwrap(),
        old
    );
    fs::write(root.join("api.json"), "changed").unwrap();
    let stale = serde_json::to_value(read_api_contracts(root, &topology)).unwrap();
    assert_eq!(stale["status"], "stale");
    assert_eq!(
        stale["contracts"][0]["source_status"],
        "changed_since_import"
    );
}
#[test]
fn yaml_local_refs_work_without_fetching_or_following_source_symlinks() {
    let dir = root();
    let root = dir.path();
    let topology = analyze_services(root).unwrap();
    let provider = service(root, "orders");
    fs::write(
        root.join("api.yaml"),
        serde_yaml_ng::to_string(&doc()).unwrap(),
    )
    .unwrap();
    let report = serde_json::to_value(
        import_api_contract(root, &topology, Path::new("api.yaml"), &provider, None).unwrap(),
    )
    .unwrap();
    assert_eq!(report["status"], "ready");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("api.yaml", root.join("link.yaml")).unwrap();
        assert!(
            import_api_contract(root, &topology, Path::new("link.yaml"), &provider, None).is_err()
        );
    }
}

#[test]
fn schema_properties_named_like_annotations_are_not_discarded() {
    let dir = root();
    let root = dir.path();
    let topology = analyze_services(root).unwrap();
    let provider = service(root, "orders");
    let mut spec = doc();
    spec["components"]["schemas"]["Order"]["properties"]["description"] = json!({"type":"string"});
    fs::write(root.join("api.json"), spec.to_string()).unwrap();
    import_api_contract(root, &topology, Path::new("api.json"), &provider, None).unwrap();
    spec["components"]["schemas"]["Order"]["properties"]["description"]["type"] = json!("integer");
    fs::write(root.join("api.json"), spec.to_string()).unwrap();
    let next = serde_json::to_value(
        import_api_contract(root, &topology, Path::new("api.json"), &provider, None).unwrap(),
    )
    .unwrap();
    assert!(
        next["contracts"][0]["changes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["kind"] == "response_schema_changed")
    );
}

#[test]
fn explicit_usage_can_retain_consumers_of_a_removed_operation() {
    let dir = root();
    let root = dir.path();
    let topology = analyze_services(root).unwrap();
    let provider = service(root, "orders");
    let consumer = service(root, "shop");
    let mut spec = doc();
    fs::write(root.join("api.json"), spec.to_string()).unwrap();
    fs::write(
        root.join("usage.json"),
        json!({"schema_version":1,"usages":[{"consumer":consumer,"operation":"GET /orders"}]})
            .to_string(),
    )
    .unwrap();
    import_api_contract(
        root,
        &topology,
        Path::new("api.json"),
        &provider,
        Some(Path::new("usage.json")),
    )
    .unwrap();
    spec["paths"].as_object_mut().unwrap().remove("/orders");
    fs::write(root.join("api.json"), spec.to_string()).unwrap();
    let next = serde_json::to_value(
        import_api_contract(
            root,
            &topology,
            Path::new("api.json"),
            &provider,
            Some(Path::new("usage.json")),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(
        next["contracts"][0]["changes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["kind"] == "operation_removed" && c["consumers"] == json!([consumer]))
    );
}

#[test]
fn malformed_path_items_and_duplicate_saved_identities_are_rejected() {
    let dir = root();
    let root = dir.path();
    let topology = analyze_services(root).unwrap();
    let provider = service(root, "orders");
    let mut spec = doc();
    spec["paths"]["/orders"] = json!(42);
    fs::write(root.join("api.json"), spec.to_string()).unwrap();
    assert!(import_api_contract(root, &topology, Path::new("api.json"), &provider, None).is_err());
    fs::write(root.join("api.json"), doc().to_string()).unwrap();
    import_api_contract(root, &topology, Path::new("api.json"), &provider, None).unwrap();
    let path = root.join(".archguard/contracts.json");
    let mut saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let duplicate = saved["contracts"][0].clone();
    saved["contracts"].as_array_mut().unwrap().push(duplicate);
    fs::write(&path, saved.to_string()).unwrap();
    assert_eq!(read_api_contracts(root, &topology).status, "error");
}

#[test]
fn openapi_32_query_operations_participate_in_contract_comparison() {
    let dir = root();
    let root = dir.path();
    let topology = analyze_services(root).unwrap();
    let provider = service(root, "orders");
    let mut spec = doc();
    spec["openapi"] = json!("3.2.1");
    spec["paths"]["/search"] =
        json!({"query":{"operationId":"searchOrders","responses":{"200":{"description":"OK"}}}});
    fs::write(root.join("api.json"), spec.to_string()).unwrap();
    let first =
        import_api_contract(root, &topology, Path::new("api.json"), &provider, None).unwrap();
    assert_eq!(first.status, "ready");
    let query = first.contracts[0]
        .operations
        .iter()
        .find(|op| op.id == "QUERY /search")
        .expect("QUERY is a standard OpenAPI 3.2 operation");
    assert_eq!(query.pointer, "/paths/~1search/query");
    assert_eq!(query.operation_id.as_deref(), Some("searchOrders"));
    spec["paths"]["/search"]["query"]["parameters"] =
        json!([{"name":"term","in":"query","required":true,"schema":{"type":"string"}}]);
    fs::write(root.join("api.json"), spec.to_string()).unwrap();
    let next =
        import_api_contract(root, &topology, Path::new("api.json"), &provider, None).unwrap();
    assert!(
        next.contracts[0]
            .changes
            .iter()
            .any(|c| c.operation == "QUERY /search" && c.kind == "required_parameter_added")
    );
}

#[test]
fn unsupported_additional_operations_are_explicitly_partial() {
    let dir = root();
    let root = dir.path();
    let topology = analyze_services(root).unwrap();
    let provider = service(root, "orders");
    let mut spec = doc();
    spec["openapi"] = json!("3.2.1");
    spec["paths"]["/orders"]["additionalOperations"] =
        json!({"COPY":{"responses":{"200":{"description":"OK"}}}});
    fs::write(root.join("api.json"), spec.to_string()).unwrap();
    let first =
        import_api_contract(root, &topology, Path::new("api.json"), &provider, None).unwrap();
    assert_eq!(first.status, "partial");
    assert!(
        first
            .diagnostics
            .iter()
            .any(|d| d.contains("additionalOperations") && d.contains("/orders"))
    );
    assert!(
        first.contracts[0]
            .operations
            .iter()
            .any(|op| op.id == "GET /orders")
    );
    let next =
        import_api_contract(root, &topology, Path::new("api.json"), &provider, None).unwrap();
    assert_eq!(next.contracts[0].comparison_status, "partial");
}

#[test]
fn reference_sibling_fields_cannot_silently_disappear() {
    let dir = root();
    let root = dir.path();
    let topology = analyze_services(root).unwrap();
    let provider = service(root, "orders");
    let mut spec = doc();
    spec["components"]["pathItems"] = json!({"Orders":spec["paths"]["/orders"].clone()});
    spec["paths"]["/orders"] = json!({"$ref":"#/components/pathItems/Orders","put":{"responses":{"204":{"description":"OK"}}}});
    fs::write(root.join("api.json"), spec.to_string()).unwrap();
    let report =
        import_api_contract(root, &topology, Path::new("api.json"), &provider, None).unwrap();
    assert_eq!(report.status, "partial");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.contains("sibling") && d.contains("#/components/pathItems/Orders"))
    );
    // Parameter references take the same resolution path; required overrides must not look complete.
    spec = doc();
    spec["components"]["parameters"] =
        json!({"Token":{"name":"token","in":"query","required":false,"schema":{"type":"string"}}});
    spec["paths"]["/orders"]["get"]["parameters"] =
        json!([{"$ref":"#/components/parameters/Token","required":true}]);
    fs::write(root.join("api.json"), spec.to_string()).unwrap();
    let report =
        import_api_contract(root, &topology, Path::new("api.json"), &provider, None).unwrap();
    assert_eq!(report.status, "partial");
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.contains("sibling") && d.contains("#/components/parameters/Token"))
    );
}

#[test]
fn invalid_path_item_fields_never_replace_a_valid_snapshot() {
    let dir = root();
    let root = dir.path();
    let topology = analyze_services(root).unwrap();
    let provider = service(root, "orders");
    fs::write(root.join("api.json"), doc().to_string()).unwrap();
    import_api_contract(root, &topology, Path::new("api.json"), &provider, None).unwrap();
    let snapshot = fs::read(root.join(".archguard/contracts.json")).unwrap();
    let operation = json!({"responses":{"200":{"description":"OK"}}});
    for item in [
        json!(null),
        json!([]),
        json!("invalid"),
        json!({"$ref":42}),
        json!({"GET":operation}),
        json!({"get":[]}),
        json!({"get":null}),
        json!({"query":operation}),
        json!({"parameters":42}),
        json!({"servers":false}),
        json!({"description":{}}),
    ] {
        let mut spec = doc();
        spec["paths"]["/orders"] = item.clone();
        fs::write(root.join("api.json"), spec.to_string()).unwrap();
        assert!(
            import_api_contract(root, &topology, Path::new("api.json"), &provider, None).is_err(),
            "invalid path item accepted: {item}"
        );
        assert_eq!(
            fs::read(root.join(".archguard/contracts.json")).unwrap(),
            snapshot
        );
    }
}

#[cfg(unix)]
#[test]
fn absolute_contract_and_usage_paths_accept_an_outer_root_alias() {
    let dir = root();
    let canonical = fs::canonicalize(dir.path()).unwrap();
    let topology = analyze_services(&canonical).unwrap();
    let provider = service(&canonical, "orders");
    let consumer = service(&canonical, "shop");
    fs::create_dir_all(canonical.join(".archguard/reports")).unwrap();
    fs::write(
        canonical.join(".archguard/reports/api.json"),
        doc().to_string(),
    )
    .unwrap();
    fs::write(
        canonical.join(".archguard/reports/usage.json"),
        json!({"schema_version":1,"usages":[{"consumer":consumer,"operation":"GET /orders"}]})
            .to_string(),
    )
    .unwrap();
    let aliases = tempfile::tempdir().unwrap();
    let alias = aliases.path().join("project-alias");
    std::os::unix::fs::symlink(&canonical, &alias).unwrap();
    let report = import_api_contract(
        &canonical,
        &topology,
        &alias.join(".archguard/reports/api.json"),
        &provider,
        Some(&alias.join(".archguard/reports/usage.json")),
    )
    .expect("an absolute input through an alias of the chosen root is inside that root");
    assert_eq!(report.status, "ready");
    assert_eq!(
        report.contracts[0].source_path,
        ".archguard/reports/api.json"
    );
    assert_eq!(
        report.contracts[0].usage_path.as_deref(),
        Some(".archguard/reports/usage.json")
    );
    assert_eq!(report.contracts[0].usages[0].consumer, consumer);
    assert_eq!(read_api_contracts(&canonical, &topology).status, "ready");
}

#[cfg(unix)]
#[test]
fn absolute_root_aliases_do_not_allow_inner_symlinks_or_outside_inputs() {
    use std::os::unix::fs::symlink;
    let dir = root();
    let canonical = fs::canonicalize(dir.path()).unwrap();
    let topology = analyze_services(&canonical).unwrap();
    let provider = service(&canonical, "orders");
    fs::write(canonical.join("api.json"), doc().to_string()).unwrap();
    fs::write(
        canonical.join("usage.json"),
        json!({"schema_version":1,"usages":[]}).to_string(),
    )
    .unwrap();
    import_api_contract(
        &canonical,
        &topology,
        Path::new("api.json"),
        &provider,
        None,
    )
    .unwrap();
    let snapshot = fs::read(canonical.join(".archguard/contracts.json")).unwrap();
    let aliases = tempfile::tempdir().unwrap();
    let alias = aliases.path().join("project-alias");
    symlink(&canonical, &alias).unwrap();
    symlink(&canonical, canonical.join("inner-root-alias")).unwrap();
    symlink("api.json", canonical.join("api-link.json")).unwrap();
    symlink("usage.json", canonical.join("usage-link.json")).unwrap();
    fs::write(aliases.path().join("outside-api.json"), doc().to_string()).unwrap();
    fs::write(
        aliases.path().join("outside-usage.json"),
        json!({"schema_version":1,"usages":[]}).to_string(),
    )
    .unwrap();
    symlink(
        aliases.path().join("outside-api.json"),
        canonical.join("outside-link.json"),
    )
    .unwrap();
    for input in [
        canonical.join("inner-root-alias/api.json"),
        alias.join("inner-root-alias/api.json"),
        canonical.join("api-link.json"),
        alias.join("api-link.json"),
        alias.join("outside-link.json"),
        aliases.path().join("outside-api.json"),
    ] {
        assert!(
            import_api_contract(&canonical, &topology, &input, &provider, None).is_err(),
            "unsafe input accepted: {}",
            input.display()
        );
        assert_eq!(
            fs::read(canonical.join(".archguard/contracts.json")).unwrap(),
            snapshot
        );
    }
    for input in [
        canonical.join("usage-link.json"),
        alias.join("usage-link.json"),
        alias.join("inner-root-alias/usage.json"),
        aliases.path().join("outside-usage.json"),
    ] {
        assert!(
            import_api_contract(
                &canonical,
                &topology,
                &canonical.join("api.json"),
                &provider,
                Some(&input)
            )
            .is_err(),
            "unsafe usage accepted: {}",
            input.display()
        );
        assert_eq!(
            fs::read(canonical.join(".archguard/contracts.json")).unwrap(),
            snapshot
        );
    }
}
