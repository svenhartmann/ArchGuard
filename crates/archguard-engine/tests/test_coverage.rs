use archguard_engine::core::{Config, Graph, GraphBuilder};
use archguard_engine::{import_test_coverage, read_test_coverage, test_coverage_fingerprint};
use std::{fs, path::Path};

fn project() -> (tempfile::TempDir, Config, Graph) {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("src/pkg")).unwrap();
    fs::write(
        root.path().join("src/pkg/A.java"),
        "package pkg;\npublic class A { int f() { return 1; } }\n",
    )
    .unwrap();
    let mut builder = GraphBuilder::new();
    builder.ensure_path("pkg/A.java", Some("java"));
    let graph = builder.build(root.path().to_string_lossy().into_owned());
    (root, Config::default(), graph)
}

const JACOCO: &str = r#"<?xml version="1.0"?><!DOCTYPE report PUBLIC "-//JACOCO//DTD Report 1.1//EN" "report.dtd"><report name="unit"><package name="pkg"><class name="pkg/A" sourcefilename="A.java"><counter type="LINE" missed="99" covered="99"/></class><sourcefile name="A.java"><line nr="1" mi="3" ci="1" mb="1" cb="1"/><line nr="2" mi="4" ci="0" mb="0" cb="0"/><counter type="LINE" missed="1" covered="1"/><counter type="BRANCH" missed="1" covered="1"/></sourcefile></package></report>"#;

#[test]
fn jacoco_source_counters_are_not_class_sums_and_snapshot_is_offline() {
    let (root, config, graph) = project();
    fs::write(root.path().join("jacoco.xml"), JACOCO).unwrap();
    let absent = read_test_coverage(root.path(), &config, &graph);
    assert_eq!(absent.status, "not_imported");
    let before = test_coverage_fingerprint(root.path());
    let report =
        import_test_coverage(root.path(), &config, &graph, Path::new("jacoco.xml"), None).unwrap();
    assert_eq!(report.status, "ready");
    assert_eq!(report.format.as_deref(), Some("jacoco"));
    assert_eq!(report.files[0].node, "pkg/A.java");
    assert_eq!(report.files[0].path, "src/pkg/A.java");
    let lines = report.files[0].lines.as_ref().unwrap();
    assert_eq!((lines.covered, lines.total), (1, 2));
    assert_eq!(
        serde_json::to_value(&report.files[0]).unwrap()["line_hits"],
        serde_json::json!([{"line":1,"covered":true},{"line":2,"covered":false}])
    );
    let branches = report.files[0].branches.as_ref().unwrap();
    assert_eq!((branches.covered, branches.total), (1, 2));
    assert!(
        report
            .diagnostics
            .iter()
            .any(|d| d.contains("revision") && d.contains("unverified"))
    );
    assert_ne!(test_coverage_fingerprint(root.path()), before);
    fs::remove_file(root.path().join("jacoco.xml")).unwrap();
    assert_eq!(read_test_coverage(root.path(), &config, &graph), report);
    fs::write(
        root.path().join("src/pkg/A.java"),
        "package pkg;\npublic class A { int f() { return 2; } }\n",
    )
    .unwrap();
    let stale = read_test_coverage(root.path(), &config, &graph);
    assert_eq!(stale.status, "stale");
    assert_eq!(stale.files[0].status, "changed_since_import");
}

#[test]
fn cobertura_unions_file_lines_and_reports_missing_branch_information() {
    let (root, config, graph) = project();
    let xml = r#"<coverage><sources><source>/foreign/ci/root</source></sources><packages><package><classes><class filename="src/pkg/A.java"><methods><method><lines><line number="99" hits="8"/></lines></method></methods><lines><line number="1" hits="0"/><line number="2" hits="1" branch="true" condition-coverage="50% (1/2)"/></lines></class><class filename="src/pkg/A.java"><lines><line number="1" hits="2"/><line number="3" hits="0" branch="true"/></lines></class></classes></package></packages></coverage>"#;
    fs::write(root.path().join("coverage.xml"), xml).unwrap();
    let report = import_test_coverage(
        root.path(),
        &config,
        &graph,
        Path::new("coverage.xml"),
        None,
    )
    .unwrap();
    assert_eq!(report.status, "partial");
    assert_eq!(report.files.len(), 1);
    let lines = report.files[0].lines.as_ref().unwrap();
    assert_eq!((lines.covered, lines.total), (2, 3));
    assert_eq!(
        serde_json::to_value(&report.files[0]).unwrap()["line_hits"],
        serde_json::json!([{"line":1,"covered":true},{"line":2,"covered":true},{"line":3,"covered":false}])
    );
    assert!(
        report.files[0].branches.is_none(),
        "unknown branch counts are not zero"
    );
    assert!(report.diagnostics.iter().any(|d| d.contains("branch")));
}

#[test]
fn mismatched_line_records_keep_aggregate_but_cannot_supply_changed_line_gate() {
    let (root, config, graph) = project();
    fs::write(root.path().join("report.xml"),"<report><package name='pkg'><sourcefile name='A.java'><line nr='2' mi='0' ci='1' mb='0' cb='0'/><counter type='LINE' covered='2' missed='0'/></sourcefile></package></report>").unwrap();
    let report =
        import_test_coverage(root.path(), &config, &graph, Path::new("report.xml"), None).unwrap();
    assert_eq!(report.status, "partial");
    assert_eq!(report.files[0].lines.as_ref().unwrap().covered, 2);
    assert!(report.files[0].line_hits.is_none());
    assert!(
        report
            .diagnostics
            .iter()
            .any(|message| message.contains("differ"))
    );
}

#[test]
fn corrupt_line_identity_or_totals_are_rejected_when_reading_snapshot() {
    let (root, config, graph) = project();
    fs::write(root.path().join("report.xml"), JACOCO).unwrap();
    let report =
        import_test_coverage(root.path(), &config, &graph, Path::new("report.xml"), None).unwrap();
    for hits in [
        serde_json::json!([{"line":0,"covered":true},{"line":2,"covered":false}]),
        serde_json::json!([{"line":1,"covered":true},{"line":1,"covered":false}]),
        serde_json::json!([{"line":1,"covered":true}]),
    ] {
        let mut value = serde_json::to_value(&report).unwrap();
        value["files"][0]["line_hits"] = hits;
        fs::write(
            root.path().join(".archguard/coverage.json"),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
        assert_eq!(
            read_test_coverage(root.path(), &config, &graph).status,
            "error"
        );
    }
}

#[test]
fn explicit_source_root_maps_foreign_reports_and_missing_files_stay_unknown() {
    let (root, config, graph) = project();
    let xml = r#"<coverage><sources><source>/agent/project</source></sources><packages><package><classes><class filename="/agent/project/pkg/A.java"><lines><line number="1" hits="3" branch="true" condition-coverage="100% (2/2)"/></lines></class><class filename="pkg/Absent.java"><lines><line number="1" hits="0"/></lines></class></classes></package></packages></coverage>"#;
    fs::write(root.path().join("coverage.xml"), xml).unwrap();
    let report = import_test_coverage(
        root.path(),
        &config,
        &graph,
        Path::new("coverage.xml"),
        Some(Path::new("src")),
    )
    .unwrap();
    assert_eq!(report.status, "partial");
    assert_eq!(report.unmatched_files, 1);
    assert_eq!(report.files.len(), 1);
    let branches = report.files[0].branches.as_ref().unwrap();
    assert_eq!((branches.covered, branches.total), (2, 2));
}

#[test]
fn malformed_xml_counters_entities_and_traversal_do_not_replace_snapshot() {
    let (root, config, graph) = project();
    fs::write(root.path().join("report.xml"), JACOCO).unwrap();
    import_test_coverage(root.path(), &config, &graph, Path::new("report.xml"), None).unwrap();
    let before = test_coverage_fingerprint(root.path());
    for xml in [
        "<report>",
        "<!DOCTYPE report [<!ENTITY x SYSTEM 'file:///etc/passwd'>]><report name='&x;'/>",
        "<report><package name='pkg'><sourcefile name='../A.java'><counter type='LINE' missed='0' covered='1'/></sourcefile></package></report>",
        "<report><package name='pkg'><sourcefile name='A.java'><counter type='LINE' missed='-1' covered='1'/></sourcefile></package></report>",
        "<coverage><packages><package><classes><class filename='src/pkg/A.java'><lines><line number='1' hits='1' branch='true' condition-coverage='150% (3/2)'/></lines></class></classes></package></packages></coverage>",
    ] {
        fs::write(root.path().join("report.xml"), xml).unwrap();
        assert!(
            import_test_coverage(root.path(), &config, &graph, Path::new("report.xml"), None)
                .is_err(),
            "{xml}"
        );
        assert_eq!(test_coverage_fingerprint(root.path()), before);
    }
}

#[test]
fn duplicate_jacoco_source_paths_and_corrupt_snapshots_are_explicit() {
    let (root, config, graph) = project();
    let source =
        "<sourcefile name='A.java'><counter type='LINE' missed='0' covered='1'/></sourcefile>";
    fs::write(
        root.path().join("report.xml"),
        format!("<report><package name='pkg'>{source}{source}</package></report>"),
    )
    .unwrap();
    assert!(
        import_test_coverage(root.path(), &config, &graph, Path::new("report.xml"), None).is_err()
    );
    fs::create_dir(root.path().join(".archguard")).unwrap();
    fs::write(root.path().join(".archguard/coverage.json"), "{}").unwrap();
    assert_eq!(
        read_test_coverage(root.path(), &config, &graph).status,
        "error"
    );
}

#[cfg(unix)]
#[test]
fn snapshot_symlinks_are_not_followed() {
    let (root, config, graph) = project();
    let outside = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join(".archguard")).unwrap();
    fs::write(outside.path().join("secret"), "{}").unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("secret"),
        root.path().join(".archguard/coverage.json"),
    )
    .unwrap();
    assert_eq!(
        read_test_coverage(root.path(), &config, &graph).status,
        "error"
    );
    fs::write(root.path().join("report.xml"), JACOCO).unwrap();
    assert!(
        import_test_coverage(root.path(), &config, &graph, Path::new("report.xml"), None).is_err()
    );
    assert_eq!(
        fs::read_to_string(outside.path().join("secret")).unwrap(),
        "{}"
    );
}

#[test]
fn ambiguous_source_roots_require_an_explicit_choice() {
    let (root, mut config, _) = project();
    fs::create_dir_all(root.path().join("lib/pkg")).unwrap();
    fs::write(
        root.path().join("lib/pkg/A.java"),
        "package other;\npublic class A {}\n",
    )
    .unwrap();
    config.project.source_roots = vec!["src".into(), "lib".into()];
    let mut builder = GraphBuilder::new();
    builder.ensure_path("pkg/A.java", Some("java"));
    builder.ensure_path("other/A.java", Some("java"));
    let graph = builder.build(root.path().to_string_lossy().into_owned());
    fs::write(root.path().join("report.xml"), JACOCO).unwrap();
    let ambiguous =
        import_test_coverage(root.path(), &config, &graph, Path::new("report.xml"), None).unwrap();
    assert_eq!(ambiguous.status, "partial");
    assert_eq!(ambiguous.unmatched_files, 1);
    assert!(ambiguous.files.is_empty());
    let explicit = import_test_coverage(
        root.path(),
        &config,
        &graph,
        Path::new("report.xml"),
        Some(Path::new("src")),
    )
    .unwrap();
    assert_eq!(explicit.files.len(), 1);
    assert_eq!(explicit.files[0].path, "src/pkg/A.java");
}

#[test]
fn unknown_lines_and_zero_branches_are_different_from_zero_percent() {
    let (root, config, graph) = project();
    fs::write(root.path().join("report.xml"), "<report><package name='pkg'><sourcefile name='A.java'><counter type='BRANCH' covered='0' missed='0'/></sourcefile></package></report>").unwrap();
    let report =
        import_test_coverage(root.path(), &config, &graph, Path::new("report.xml"), None).unwrap();
    assert_eq!(report.status, "partial");
    assert!(report.files[0].lines.is_none());
    assert_eq!(report.files[0].branches.as_ref().unwrap().total, 0);
    fs::write(root.path().join("report.xml"), "<report><package name='pkg'><sourcefile name='A.java'><counter type='LINE' covered='0' missed='3'/></sourcefile></package></report>").unwrap();
    let zero =
        import_test_coverage(root.path(), &config, &graph, Path::new("report.xml"), None).unwrap();
    assert_eq!(zero.files[0].lines.as_ref().unwrap().covered, 0);
    assert_eq!(zero.files[0].lines.as_ref().unwrap().total, 3);
    assert!(zero.files[0].branches.is_none());
}

#[test]
fn saved_counter_corruption_and_oversized_input_fail_explicitly() {
    let (root, config, graph) = project();
    fs::write(root.path().join("report.xml"), JACOCO).unwrap();
    let report =
        import_test_coverage(root.path(), &config, &graph, Path::new("report.xml"), None).unwrap();
    let original = serde_json::to_value(report).unwrap();
    for (field, value) in [
        ("path", serde_json::json!("../outside.java")),
        ("lines", serde_json::json!({"covered":5,"total":2})),
        ("source_sha256", serde_json::json!("invalid")),
    ] {
        let mut corrupt = original.clone();
        corrupt["files"][0][field] = value;
        fs::write(
            root.path().join(".archguard/coverage.json"),
            serde_json::to_vec(&corrupt).unwrap(),
        )
        .unwrap();
        assert_eq!(
            read_test_coverage(root.path(), &config, &graph).status,
            "error"
        );
    }
    let large = fs::File::create(root.path().join("oversized.xml")).unwrap();
    large.set_len(32 * 1024 * 1024 + 1).unwrap();
    assert!(
        import_test_coverage(
            root.path(),
            &config,
            &graph,
            Path::new("oversized.xml"),
            None
        )
        .is_err()
    );
}

#[test]
fn invalid_cobertura_branch_flags_are_not_silently_counted_as_no_branches() {
    let (root, config, graph) = project();
    fs::write(root.path().join("report.xml"), "<coverage><packages><package><classes><class filename='src/pkg/A.java'><lines><line number='1' hits='1' branch='sometimes'/></lines></class></classes></package></packages></coverage>").unwrap();
    assert!(
        import_test_coverage(root.path(), &config, &graph, Path::new("report.xml"), None).is_err()
    );
}

#[test]
fn line_only_cobertura_does_not_invent_branch_instrumentation() {
    let (root, config, graph) = project();
    fs::write(root.path().join("report.xml"), "<coverage><packages><package><classes><class filename='src/pkg/A.java'><lines><line number='1' hits='0'/></lines></class></classes></package></packages></coverage>").unwrap();
    let report =
        import_test_coverage(root.path(), &config, &graph, Path::new("report.xml"), None).unwrap();
    assert!(report.files[0].branches.is_none());
    fs::write(root.path().join("report.xml"), "<coverage><packages><package><classes><class filename='src/pkg/A.java'><lines><line number='1' hits='0' branch='false'/></lines></class></classes></package></packages></coverage>").unwrap();
    let explicit =
        import_test_coverage(root.path(), &config, &graph, Path::new("report.xml"), None).unwrap();
    assert_eq!(explicit.files[0].branches.as_ref().unwrap().total, 0);
}

#[test]
fn absolute_in_project_source_base_maps_without_declared_source_root() {
    let (root, config, graph) = project();
    let base = root.path().join("src");
    fs::write(root.path().join("report.xml"), format!("<coverage><sources><source>{}</source></sources><packages><package><classes><class filename='pkg/A.java'><lines><line number='1' hits='1' branch='false'/></lines></class></classes></package></packages></coverage>", base.display())).unwrap();
    let mut custom = config.clone();
    custom.project.source_roots = vec!["unused".into()];
    let report =
        import_test_coverage(root.path(), &custom, &graph, Path::new("report.xml"), None).unwrap();
    assert_eq!(report.files.len(), 1);
    assert_eq!(report.files[0].path, "src/pkg/A.java");
}

#[test]
fn failed_source_validation_does_not_leave_rows_claiming_unchanged() {
    let (root, mut config, graph) = project();
    fs::write(root.path().join("report.xml"), JACOCO).unwrap();
    import_test_coverage(root.path(), &config, &graph, Path::new("report.xml"), None).unwrap();
    config.project.exclude = vec!["[".into()];
    let report = read_test_coverage(root.path(), &config, &graph);
    assert_eq!(report.status, "error");
    assert!(report.files.is_empty());
}

const CLOVER: &str = r#"<?xml version="1.0"?><coverage generated="1730000000" clover="4.4.1"><project timestamp="1730000000" name="demo"><metrics statements="3" coveredstatements="2" conditionals="2" coveredconditionals="1"/><file name="A.java" path="src/pkg/A.java"><metrics statements="3" coveredstatements="2" conditionals="2" coveredconditionals="1"/><line num="1" type="stmt" count="3"/><line num="2" type="stmt" count="0"/><line num="3" type="cond" count="5" true="1" false="0"/></file></project></coverage>"#;

#[test]
fn clover_statements_and_conditions_map_to_lines_branches_and_hits() {
    let (root, config, graph) = project();
    fs::write(root.path().join("clover.xml"), CLOVER).unwrap();
    let report =
        import_test_coverage(root.path(), &config, &graph, Path::new("clover.xml"), None).unwrap();
    assert_eq!(report.status, "ready");
    assert_eq!(report.format.as_deref(), Some("clover"));
    assert_eq!(report.files[0].node, "pkg/A.java");
    assert_eq!(report.files[0].path, "src/pkg/A.java");
    let lines = report.files[0].lines.as_ref().unwrap();
    assert_eq!((lines.covered, lines.total), (2, 3));
    assert_eq!(
        serde_json::to_value(&report.files[0]).unwrap()["line_hits"],
        serde_json::json!([{"line":1,"covered":true},{"line":2,"covered":false},{"line":3,"covered":true}])
    );
    let branches = report.files[0].branches.as_ref().unwrap();
    assert_eq!((branches.covered, branches.total), (1, 2));
    fs::remove_file(root.path().join("clover.xml")).unwrap();
    assert_eq!(read_test_coverage(root.path(), &config, &graph), report);
}

#[test]
fn clover_unknown_cond_flags_leave_branches_unknown_but_keep_lines() {
    let (root, config, graph) = project();
    fs::write(root.path().join("clover.xml"), "<coverage><project><file path='src/pkg/A.java'><line num='1' type='stmt' count='3'/><line num='3' type='cond' count='5' true='1'/></file></project></coverage>").unwrap();
    let report =
        import_test_coverage(root.path(), &config, &graph, Path::new("clover.xml"), None).unwrap();
    assert_eq!(report.status, "partial");
    assert_eq!(report.files[0].lines.as_ref().unwrap().total, 2);
    assert_eq!(report.files[0].line_hits.as_ref().unwrap().len(), 2);
    assert!(
        report.files[0].branches.is_none(),
        "a condition without both flags is unknown, not 0/2"
    );
    assert!(report.diagnostics.iter().any(|d| d.contains("branch")));
}

#[test]
fn clover_conflicting_line_types_leave_branches_unknown() {
    let (root, config, graph) = project();
    fs::write(root.path().join("clover.xml"), "<coverage><project><file path='src/pkg/A.java'><line num='1' type='stmt' count='3'/><line num='3' type='cond' count='5' true='1' false='1'/></file><file path='src/pkg/A.java'><line num='3' type='stmt' count='2'/></file></project></coverage>").unwrap();
    let report =
        import_test_coverage(root.path(), &config, &graph, Path::new("clover.xml"), None).unwrap();
    assert!(report.files[0].branches.is_none());
    assert_eq!(report.files[0].lines.as_ref().unwrap().total, 2);
    assert!(report.diagnostics.iter().any(|d| d.contains("branch")));
}

#[test]
fn clover_rejects_unsupported_line_types_and_missing_attributes_without_touching_snapshot() {
    let (root, config, graph) = project();
    fs::write(root.path().join("report.xml"), JACOCO).unwrap();
    import_test_coverage(root.path(), &config, &graph, Path::new("report.xml"), None).unwrap();
    let before = test_coverage_fingerprint(root.path());
    for clover in [
        // missing count
        "<coverage><project><file path='src/pkg/A.java'><line num='1' type='stmt'/></file></project></coverage>",
        // unsupported line type
        "<coverage><project><file path='src/pkg/A.java'><line num='1' type='method_body' count='1'/></file></project></coverage>",
        // zero line number
        "<coverage><project><file path='src/pkg/A.java'><line num='0' type='stmt' count='1'/></file></project></coverage>",
        // file without path
        "<coverage><project><file><line num='1' type='stmt' count='1'/></file></project></coverage>",
        // traversal path
        "<coverage><project><file path='../A.java'><line num='1' type='stmt' count='1'/></file></project></coverage>",
    ] {
        fs::write(root.path().join("report.xml"), clover).unwrap();
        assert!(
            import_test_coverage(root.path(), &config, &graph, Path::new("report.xml"), None)
                .is_err(),
            "{clover}"
        );
        assert_eq!(test_coverage_fingerprint(root.path()), before);
    }
}

#[test]
fn clover_and_cobertura_are_told_apart_by_their_elements_and_bare_coverage_stays_cobertura() {
    let (root, config, graph) = project();
    fs::write(root.path().join("clover.xml"), CLOVER).unwrap();
    let clover =
        import_test_coverage(root.path(), &config, &graph, Path::new("clover.xml"), None).unwrap();
    assert_eq!(clover.format.as_deref(), Some("clover"));
    fs::write(root.path().join("cobertura.xml"), "<coverage><packages><package><classes><class filename='src/pkg/A.java'><lines><line number='1' hits='1' branch='false'/></lines></class></classes></package></packages></coverage>").unwrap();
    let cobertura = import_test_coverage(
        root.path(),
        &config,
        &graph,
        Path::new("cobertura.xml"),
        None,
    )
    .unwrap();
    assert_eq!(cobertura.format.as_deref(), Some("cobertura"));
    fs::write(root.path().join("mixed.xml"), "<coverage><project><file path='src/pkg/A.java'><line num='1' type='stmt' count='1'/></file></project><packages><package><classes><class filename='src/pkg/A.java'><lines><line number='1' hits='1'/></lines></class></classes></package></packages></coverage>").unwrap();
    assert!(
        import_test_coverage(root.path(), &config, &graph, Path::new("mixed.xml"), None).is_err()
    );
    fs::write(root.path().join("bare.xml"), "<coverage/>").unwrap();
    let bare =
        import_test_coverage(root.path(), &config, &graph, Path::new("bare.xml"), None).unwrap();
    assert_eq!(bare.format.as_deref(), Some("cobertura"));
    assert_eq!(bare.status, "partial");
    assert!(bare.files.is_empty());
}

#[test]
fn lcov_records_map_to_lines_branches_and_hits() {
    let (root, config, graph) = project();
    fs::write(root.path().join("lcov.info"), "TN:\nSF:src/pkg/A.java\nFN:1,f\nFNDA:2,f\nFNF:1\nFNH:1\nDA:1,2,abc123\nDA:2,0\nDA:3,5\nBRDA:3,0,0,1\nBRDA:3,0,1,0\nLF:3\nLH:2\nBRF:2\nBRH:1\nend_of_record\n").unwrap();
    let report =
        import_test_coverage(root.path(), &config, &graph, Path::new("lcov.info"), None).unwrap();
    assert_eq!(report.status, "ready");
    assert_eq!(report.format.as_deref(), Some("lcov"));
    assert_eq!(report.files[0].node, "pkg/A.java");
    assert_eq!(report.files[0].path, "src/pkg/A.java");
    let lines = report.files[0].lines.as_ref().unwrap();
    assert_eq!((lines.covered, lines.total), (2, 3));
    assert_eq!(
        serde_json::to_value(&report.files[0]).unwrap()["line_hits"],
        serde_json::json!([{"line":1,"covered":true},{"line":2,"covered":false},{"line":3,"covered":true}])
    );
    let branches = report.files[0].branches.as_ref().unwrap();
    assert_eq!((branches.covered, branches.total), (1, 2));
    fs::remove_file(root.path().join("lcov.info")).unwrap();
    assert_eq!(read_test_coverage(root.path(), &config, &graph), report);
}

#[test]
fn lcov_multiple_records_for_one_file_merge_and_drop_summary_counters() {
    let (root, config, graph) = project();
    fs::write(root.path().join("lcov.info"), "SF:src/pkg/A.java\nDA:1,1\nDA:2,0\nLF:2\nLH:1\nend_of_record\nTN:\nSF:src/pkg/A.java\nDA:2,3\nDA:3,0\nLF:2\nLH:1\nBRDA:1,0,0,-\nend_of_record\n").unwrap();
    let report =
        import_test_coverage(root.path(), &config, &graph, Path::new("lcov.info"), None).unwrap();
    assert_eq!(report.status, "partial");
    let lines = report.files[0].lines.as_ref().unwrap();
    assert_eq!(
        (lines.covered, lines.total),
        (2, 3),
        "merged DA detail decides; per-record LF/LH would double-count"
    );
    assert_eq!(
        serde_json::to_value(&report.files[0]).unwrap()["line_hits"],
        serde_json::json!([{"line":1,"covered":true},{"line":2,"covered":true},{"line":3,"covered":false}])
    );
    assert!(report.files[0].branches.is_none(), "'-' is unknown, not 0");
    assert!(report.diagnostics.iter().any(|d| d.contains("branch")));
}

#[test]
fn lcov_line_hits_mismatch_keeps_aggregate_but_not_the_changed_line_gate() {
    let (root, config, graph) = project();
    fs::write(
        root.path().join("lcov.info"),
        "SF:src/pkg/A.java\nDA:1,1\nDA:2,0\nLF:5\nLH:2\nend_of_record\n",
    )
    .unwrap();
    let report =
        import_test_coverage(root.path(), &config, &graph, Path::new("lcov.info"), None).unwrap();
    assert_eq!(report.status, "partial");
    assert_eq!(
        (
            report.files[0].lines.as_ref().unwrap().covered,
            report.files[0].lines.as_ref().unwrap().total
        ),
        (2, 5)
    );
    assert!(report.files[0].line_hits.is_none());
    assert!(
        report
            .diagnostics
            .iter()
            .any(|message| message.contains("differ"))
    );
}

#[test]
fn lcov_malformed_input_never_replaces_snapshot() {
    let (root, config, graph) = project();
    fs::write(root.path().join("report.xml"), JACOCO).unwrap();
    import_test_coverage(root.path(), &config, &graph, Path::new("report.xml"), None).unwrap();
    let before = test_coverage_fingerprint(root.path());
    for lcov in [
        "DA:1,1\nend_of_record\n",
        "SF:src/pkg/A.java\nDA:0,1\nend_of_record\n",
        "SF:src/pkg/A.java\nDA:1,x\nend_of_record\n",
        "SF:../A.java\nDA:1,1\nend_of_record\n",
        "SF:src/pkg/A.java\nDA:1,1\nLF:1\nLH:2\nend_of_record\n",
        "SF:src/pkg/A.java\nBRDA:1,0,0\nend_of_record\n",
        "TN:unit\nLF:3\nLH:1\nend_of_record\n",
        "",
    ] {
        fs::write(root.path().join("lcov.info"), lcov).unwrap();
        assert!(
            import_test_coverage(root.path(), &config, &graph, Path::new("lcov.info"), None)
                .is_err(),
            "{lcov:?}"
        );
        assert_eq!(test_coverage_fingerprint(root.path()), before);
    }
}
