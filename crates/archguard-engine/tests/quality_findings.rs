use archguard_engine::core::{Config, Graph, GraphBuilder};
use archguard_engine::{
    import_quality_findings, quality_findings_fingerprint, read_quality_findings,
};
use serde_json::{Value, json};
use std::{fs, path::Path};

fn project() -> (tempfile::TempDir, Config, Graph) {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("src/pkg")).unwrap();
    fs::write(
        root.path().join("src/pkg/A.java"),
        "package pkg;\nclass A {\n int value = 1;\n}\n",
    )
    .unwrap();
    let mut graph = GraphBuilder::new();
    graph.ensure_path("pkg/A.java", Some("java"));
    (root, Config::default(), graph.build("fixture".into()))
}
fn result(uri: &str, message: &str) -> Value {
    json!({"ruleId":"R1","message":{"text":message},"locations":[{"physicalLocation":{"artifactLocation":{"uri":uri},"region":{"startLine":3,"startColumn":2}}}]})
}
fn sarif(results: Vec<Value>) -> Value {
    json!({"version":"2.1.0","runs":[{"tool":{"driver":{"name":"Example analyzer","rules":[{"id":"R1","defaultConfiguration":{"level":"error"},"helpUri":"https://example.test/rules/R1"}]}},"invocations":[{"executionSuccessful":true}],"results":results}]})
}
fn save(root: &Path, value: &Value) {
    fs::write(
        root.join("report.sarif"),
        serde_json::to_vec(value).unwrap(),
    )
    .unwrap();
}

#[test]
fn imports_source_evidence_and_reads_offline_with_freshness() {
    let (root, config, graph) = project();
    assert_eq!(
        read_quality_findings(root.path(), &config, &graph).status,
        "not_imported"
    );
    let before = quality_findings_fingerprint(root.path());
    save(
        root.path(),
        &sarif(vec![result(
            "src/pkg/A.java",
            "<img src=x> keep source text",
        )]),
    );
    let imported = import_quality_findings(
        root.path(),
        &config,
        &graph,
        Path::new("report.sarif"),
        None,
    )
    .unwrap();
    assert_eq!(imported.status, "ready");
    assert_eq!(imported.tools, ["Example analyzer"]);
    assert_eq!(imported.findings.len(), 1);
    let finding = &imported.findings[0];
    assert_eq!(finding.path.as_deref(), Some("src/pkg/A.java"));
    assert_eq!(finding.node.as_deref(), Some("pkg/A.java"));
    assert_eq!(finding.line, Some(3));
    assert_eq!(finding.column, Some(2));
    assert_eq!(finding.rule_id, "R1");
    assert_eq!(finding.level, "error");
    assert_eq!(finding.status, "unchanged_since_import");
    assert_eq!(finding.message, "<img src=x> keep source text");
    assert_eq!(finding.source_sha256.as_ref().unwrap().len(), 64);
    assert_eq!(
        finding.help_uri.as_deref(),
        Some("https://example.test/rules/R1")
    );
    assert!(
        imported
            .diagnostics
            .iter()
            .any(|d| d.contains("revision") && d.contains("unverified"))
    );
    assert_ne!(quality_findings_fingerprint(root.path()), before);
    fs::remove_file(root.path().join("report.sarif")).unwrap();
    assert_eq!(
        read_quality_findings(root.path(), &config, &graph),
        imported
    );
    fs::write(
        root.path().join("src/pkg/A.java"),
        "package pkg;\nclass A {}\n",
    )
    .unwrap();
    let stale = read_quality_findings(root.path(), &config, &graph);
    assert_eq!(stale.status, "stale");
    assert_eq!(stale.findings[0].status, "changed_since_import");
}

#[test]
fn artifact_indexes_uri_bases_and_percent_encoded_paths_are_mapped_exactly() {
    let (root, config, graph) = project();
    let mut report = sarif(vec![result("ignored", "indexed")]);
    report["runs"][0]["originalUriBaseIds"] =
        json!({"ROOT":{"uri":"file:///foreign/build/"},"SRC":{"uri":"src/","uriBaseId":"ROOT"}});
    report["runs"][0]["artifacts"] = json!([{"location":{"uri":"pkg/%41.java","uriBaseId":"SRC"}}]);
    report["runs"][0]["results"][0]["locations"][0]["physicalLocation"]["artifactLocation"] =
        json!({"index":0});
    save(root.path(), &report);
    let imported = import_quality_findings(
        root.path(),
        &config,
        &graph,
        Path::new("report.sarif"),
        None,
    )
    .unwrap();
    assert_eq!(imported.status, "ready");
    assert_eq!(imported.findings[0].path.as_deref(), Some("src/pkg/A.java"));
}

#[test]
fn source_root_is_explicit_and_absolute_local_files_are_supported() {
    let (root, config, graph) = project();
    save(root.path(), &sarif(vec![result("pkg/A.java", "relative")]));
    let imported = import_quality_findings(
        root.path(),
        &config,
        &graph,
        Path::new("report.sarif"),
        Some(Path::new("src")),
    )
    .unwrap();
    assert_eq!(imported.findings[0].node.as_deref(), Some("pkg/A.java"));
    let absolute = format!("file://{}", root.path().join("src/pkg/A.java").display());
    save(root.path(), &sarif(vec![result(&absolute, "absolute")]));
    let imported = import_quality_findings(
        root.path(),
        &config,
        &graph,
        Path::new("report.sarif"),
        None,
    )
    .unwrap();
    assert_eq!(imported.findings[0].path.as_deref(), Some("src/pkg/A.java"));
    let outside = tempfile::tempdir().unwrap();
    assert!(
        import_quality_findings(
            root.path(),
            &config,
            &graph,
            Path::new("report.sarif"),
            Some(outside.path())
        )
        .is_err()
    );
}

#[test]
fn preserves_duplicate_records_multiple_tools_suppressions_and_unmapped_locations() {
    let (root, config, graph) = project();
    let mut suppressed = result("src/pkg/A.java", "suppressed");
    suppressed["suppressions"] = json!([{"kind":"inSource","status":"accepted"}]);
    let duplicate = result("src/pkg/A.java", "duplicate");
    let mut report = sarif(vec![
        duplicate.clone(),
        duplicate,
        suppressed,
        result("missing/Foo.java", "outside scope"),
    ]);
    report["runs"].as_array_mut().unwrap().push(json!({"tool":{"driver":{"name":"Second"}},"results":[{"message":{"text":"global finding"},"level":"note"}]}));
    save(root.path(), &report);
    let imported = import_quality_findings(
        root.path(),
        &config,
        &graph,
        Path::new("report.sarif"),
        None,
    )
    .unwrap();
    assert_eq!(imported.status, "partial");
    assert_eq!(imported.tools, ["Example analyzer", "Second"]);
    assert_eq!(imported.findings.len(), 5);
    assert_ne!(imported.findings[0].id, imported.findings[1].id);
    assert!(imported.findings[2].suppressed);
    assert_eq!(imported.findings[3].status, "unmapped");
    assert_eq!(
        imported.findings[3].path.as_deref(),
        Some("missing/Foo.java"),
        "unmapped safe relative source identifiers remain available as evidence"
    );
    assert!(imported.findings[3].node.is_none());
    assert_eq!(imported.findings[4].level, "note");
    assert!(imported.findings[4].line.is_none());
}

#[test]
fn zero_results_preserve_failed_or_missing_analysis_status() {
    let (root, config, graph) = project();
    for results in [Value::Null, json!([])] {
        let mut report = sarif(vec![]);
        report["runs"][0]["results"] = results;
        report["runs"][0]["invocations"][0]["executionSuccessful"] = json!(false);
        save(root.path(), &report);
        let imported = import_quality_findings(
            root.path(),
            &config,
            &graph,
            Path::new("report.sarif"),
            None,
        )
        .unwrap();
        assert_eq!(imported.status, "partial");
        assert!(imported.findings.is_empty());
        assert!(
            imported
                .diagnostics
                .iter()
                .any(|d| d.contains("failed") || d.contains("unsuccessful"))
        );
    }
    let mut report = sarif(vec![]);
    report["runs"][0]["invocations"][0]["toolConfigurationNotifications"] =
        json!([{"level":"error","message":{"text":"Dependency missing"}}]);
    save(root.path(), &report);
    assert_eq!(
        import_quality_findings(
            root.path(),
            &config,
            &graph,
            Path::new("report.sarif"),
            None
        )
        .unwrap()
        .status,
        "partial"
    );
    save(root.path(), &sarif(vec![]));
    assert_eq!(
        import_quality_findings(
            root.path(),
            &config,
            &graph,
            Path::new("report.sarif"),
            None
        )
        .unwrap()
        .status,
        "ready"
    );
}

#[test]
fn malformed_reports_do_not_replace_previous_snapshot() {
    let (root, config, graph) = project();
    save(root.path(), &sarif(vec![result("src/pkg/A.java", "valid")]));
    import_quality_findings(
        root.path(),
        &config,
        &graph,
        Path::new("report.sarif"),
        None,
    )
    .unwrap();
    let before = quality_findings_fingerprint(root.path());
    for invalid in [
        json!({}),
        json!({"version":"2.2","runs":[]}),
        json!({"version":"2.1.0","runs":[{}]}),
        json!({"version":"2.1.0","runs":"bad"}),
    ] {
        save(root.path(), &invalid);
        assert!(
            import_quality_findings(
                root.path(),
                &config,
                &graph,
                Path::new("report.sarif"),
                None
            )
            .is_err()
        );
        assert_eq!(quality_findings_fingerprint(root.path()), before);
    }
}

#[test]
fn invalid_and_outside_uris_never_map_to_local_source() {
    let (root, config, graph) = project();
    let uris = [
        "../src/pkg/A.java",
        "%2e%2e/src/pkg/A.java",
        "src/%00pkg/A.java",
        "src/%GG/A.java",
        "https://example.test/src/pkg/A.java",
        "file:///outside/src/pkg/A.java",
        "file://remote/src/pkg/A.java",
    ];
    save(
        root.path(),
        &sarif(uris.iter().map(|uri| result(uri, uri)).collect()),
    );
    let imported = import_quality_findings(
        root.path(),
        &config,
        &graph,
        Path::new("report.sarif"),
        None,
    )
    .unwrap();
    assert_eq!(imported.status, "partial");
    assert_eq!(imported.findings.len(), uris.len());
    assert!(
        imported
            .findings
            .iter()
            .all(|f| f.node.is_none() && f.source_sha256.is_none() && f.status == "unmapped")
    );
}

#[test]
fn missing_region_and_rule_defaults_are_not_invented() {
    let (root, config, graph) = project();
    let mut a = result("src/pkg/A.java", "no region");
    a["locations"][0]["physicalLocation"]
        .as_object_mut()
        .unwrap()
        .remove("region");
    a.as_object_mut().unwrap().remove("ruleId");
    let mut b = result("src/pkg/A.java", "pass");
    b["kind"] = json!("pass");
    let mut c = result("src/pkg/A.java", "rejected suppression");
    c["suppressions"] = json!([{"kind":"external","status":"rejected"}]);
    save(root.path(), &sarif(vec![a, b, c]));
    let imported = import_quality_findings(
        root.path(),
        &config,
        &graph,
        Path::new("report.sarif"),
        None,
    )
    .unwrap();
    assert_eq!(imported.findings[0].level, "warning");
    assert!(imported.findings[0].line.is_none());
    assert!(imported.findings[0].column.is_none());
    assert_eq!(imported.findings[1].level, "none");
    assert!(!imported.findings[2].suppressed);
}

#[test]
fn cyclic_uri_base_invalid_snapshot_and_non_http_help_are_honest() {
    let (root, config, graph) = project();
    let mut report = sarif(vec![result("src/pkg/A.java", "loop")]);
    report["runs"][0]["originalUriBaseIds"] =
        json!({"A":{"uri":"","uriBaseId":"B"},"B":{"uri":"","uriBaseId":"A"}});
    report["runs"][0]["results"][0]["locations"][0]["physicalLocation"]["artifactLocation"]["uriBaseId"] =
        json!("A");
    report["runs"][0]["tool"]["driver"]["rules"][0]["helpUri"] = json!("javascript:alert(1)");
    save(root.path(), &report);
    let imported = import_quality_findings(
        root.path(),
        &config,
        &graph,
        Path::new("report.sarif"),
        None,
    )
    .unwrap();
    assert_eq!(imported.status, "partial");
    assert!(imported.findings[0].node.is_none());
    assert!(imported.findings[0].help_uri.is_none());
    fs::write(root.path().join(".archguard/quality.json"), "{}").unwrap();
    assert_eq!(
        read_quality_findings(root.path(), &config, &graph).status,
        "error"
    );
}

#[test]
fn message_arguments_are_not_reexpanded_and_overrides_use_rule_indexes() {
    let (root, config, graph) = project();
    let mut entry = result("src/pkg/A.java", "ignored");
    entry["ruleIndex"] = json!(0);
    entry["message"] = json!({"id":"template","arguments":["{1}","right"]});
    entry["provenance"] = json!({"invocationIndex":0});
    let mut report = sarif(vec![entry]);
    report["runs"][0]["tool"]["driver"]["rules"][0]["messageStrings"] =
        json!({"template":{"text":"{0} {1} {{0}}"}});
    report["runs"][0]["invocations"][0]["ruleConfigurationOverrides"] =
        json!([{"descriptor":{"index":0},"configuration":{"level":"note"}}]);
    save(root.path(), &report);
    let imported = import_quality_findings(
        root.path(),
        &config,
        &graph,
        Path::new("report.sarif"),
        None,
    )
    .unwrap();
    assert_eq!(imported.findings[0].message, "{1} right {0}");
    assert_eq!(imported.findings[0].level, "note");
}

#[test]
fn pending_suppressions_are_not_hidden_and_absent_results_remain_historical() {
    let (root, config, graph) = project();
    let mut pending = result("src/pkg/A.java", "pending");
    pending["suppressions"] = json!([{"kind":"inSource"}]);
    let mut review = result("src/pkg/A.java", "review");
    review["suppressions"] = json!([{"kind":"external","status":"underReview"}]);
    let mut absent = result("src/pkg/A.java", "historical");
    absent["baselineState"] = json!("absent");
    save(root.path(), &sarif(vec![pending, review, absent]));
    let imported = import_quality_findings(
        root.path(),
        &config,
        &graph,
        Path::new("report.sarif"),
        None,
    )
    .unwrap();
    assert_eq!(imported.status, "partial");
    assert!(!imported.findings[0].suppressed);
    assert!(!imported.findings[1].suppressed);
    assert_eq!(
        serde_json::to_value(&imported.findings[2]).unwrap()["baseline_state"],
        "absent"
    );
    assert!(
        imported
            .diagnostics
            .iter()
            .any(|d| d.contains("suppression") && d.contains("not accepted"))
    );
}

#[test]
fn contradictory_artifact_indexes_do_not_claim_a_source_location() {
    let (root, config, graph) = project();
    let mut report = sarif(vec![result("src/pkg/A.java", "conflict")]);
    report["runs"][0]["artifacts"] = json!([{"location":{"uri":"other/Foo.java"}}]);
    report["runs"][0]["results"][0]["locations"][0]["physicalLocation"]["artifactLocation"]["index"] =
        json!(0);
    save(root.path(), &report);
    let imported = import_quality_findings(
        root.path(),
        &config,
        &graph,
        Path::new("report.sarif"),
        None,
    )
    .unwrap();
    assert_eq!(imported.status, "partial");
    assert_eq!(imported.findings[0].status, "unmapped");
}

#[test]
fn invalid_snapshot_fields_and_oversized_reports_are_rejected() {
    let (root, config, graph) = project();
    save(
        root.path(),
        &sarif(vec![result("src/pkg/A.java", "original")]),
    );
    let imported = import_quality_findings(
        root.path(),
        &config,
        &graph,
        Path::new("report.sarif"),
        None,
    )
    .unwrap();
    let before = quality_findings_fingerprint(root.path());
    let file = fs::File::create(root.path().join("huge.sarif")).unwrap();
    file.set_len(32 * 1024 * 1024 + 1).unwrap();
    assert!(
        import_quality_findings(root.path(), &config, &graph, Path::new("huge.sarif"), None)
            .is_err()
    );
    assert_eq!(quality_findings_fingerprint(root.path()), before);
    for (field, value) in [
        ("path", json!("../outside.java")),
        ("source_sha256", json!("bad")),
        ("level", json!("critical")),
        ("line", json!(0)),
        ("help_uri", json!("javascript:alert(1)")),
        ("baseline_state", json!("invented")),
    ] {
        let mut invalid = serde_json::to_value(&imported).unwrap();
        invalid["findings"][0][field] = value;
        fs::write(
            root.path().join(".archguard/quality.json"),
            serde_json::to_vec(&invalid).unwrap(),
        )
        .unwrap();
        assert_eq!(
            read_quality_findings(root.path(), &config, &graph).status,
            "error",
            "{field}"
        );
    }
}

#[test]
fn ambiguous_source_roots_require_an_explicit_choice() {
    let (root, mut config, _) = project();
    fs::create_dir_all(root.path().join("lib/pkg")).unwrap();
    fs::write(
        root.path().join("lib/pkg/A.java"),
        "package other;\nclass A {}\n",
    )
    .unwrap();
    config.project.source_roots = vec!["src".into(), "lib".into()];
    let mut graph = GraphBuilder::new();
    graph.ensure_path("pkg/A.java", Some("java"));
    graph.ensure_path("other/A.java", Some("java"));
    let graph = graph.build("fixture".into());
    save(root.path(), &sarif(vec![result("pkg/A.java", "ambiguous")]));
    let ambiguous = import_quality_findings(
        root.path(),
        &config,
        &graph,
        Path::new("report.sarif"),
        None,
    )
    .unwrap();
    assert_eq!(ambiguous.status, "partial");
    assert!(ambiguous.findings[0].node.is_none());
    let explicit = import_quality_findings(
        root.path(),
        &config,
        &graph,
        Path::new("report.sarif"),
        Some(Path::new("src")),
    )
    .unwrap();
    assert_eq!(explicit.findings[0].path.as_deref(), Some("src/pkg/A.java"));
}

#[test]
fn malformed_result_metadata_is_rejected_instead_of_assigning_defaults() {
    let (root, config, graph) = project();
    save(root.path(), &sarif(vec![result("src/pkg/A.java", "valid")]));
    import_quality_findings(
        root.path(),
        &config,
        &graph,
        Path::new("report.sarif"),
        None,
    )
    .unwrap();
    let before = quality_findings_fingerprint(root.path());
    for (pointer, value) in [
        ("/runs/0/results/0/level", json!(3)),
        ("/runs/0/results/0/kind", json!("invented")),
        ("/runs/0/invocations/0/executionSuccessful", json!("false")),
        (
            "/runs/0/tool/driver/rules/0/defaultConfiguration/level",
            json!("critical"),
        ),
    ] {
        let mut invalid = sarif(vec![result("src/pkg/A.java", "invalid")]);
        if pointer.ends_with("/kind") {
            invalid["runs"][0]["results"][0]["kind"] = value;
        } else if pointer.ends_with("/level") && pointer.contains("results") {
            invalid["runs"][0]["results"][0]["level"] = value;
        } else {
            *invalid.pointer_mut(pointer).unwrap() = value;
        }
        save(root.path(), &invalid);
        assert!(
            import_quality_findings(
                root.path(),
                &config,
                &graph,
                Path::new("report.sarif"),
                None
            )
            .is_err(),
            "{pointer}"
        );
        assert_eq!(quality_findings_fingerprint(root.path()), before);
    }
}

#[cfg(unix)]
#[test]
fn source_and_snapshot_symlinks_are_not_followed() {
    let (root, config, graph) = project();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("A.java"), "package pkg; class A {}").unwrap();
    fs::remove_file(root.path().join("src/pkg/A.java")).unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("A.java"),
        root.path().join("src/pkg/A.java"),
    )
    .unwrap();
    save(
        root.path(),
        &sarif(vec![result("src/pkg/A.java", "outside source")]),
    );
    let imported = import_quality_findings(
        root.path(),
        &config,
        &graph,
        Path::new("report.sarif"),
        None,
    )
    .unwrap();
    assert_eq!(imported.status, "partial");
    assert!(imported.findings[0].node.is_none());
    fs::remove_file(root.path().join(".archguard/quality.json")).unwrap();
    fs::write(outside.path().join("snapshot"), "sentinel").unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("snapshot"),
        root.path().join(".archguard/quality.json"),
    )
    .unwrap();
    assert_eq!(
        read_quality_findings(root.path(), &config, &graph).status,
        "error"
    );
    assert!(
        import_quality_findings(
            root.path(),
            &config,
            &graph,
            Path::new("report.sarif"),
            None
        )
        .is_err()
    );
    assert_eq!(
        fs::read_to_string(outside.path().join("snapshot")).unwrap(),
        "sentinel"
    );
}
