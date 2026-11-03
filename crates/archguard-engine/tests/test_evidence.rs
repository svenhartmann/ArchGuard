use archguard_engine::{
    core::{Config, Graph, GraphBuilder, Ref},
    import_test_evidence, read_test_evidence, test_evidence_fingerprint, test_impact,
};
use std::{fs, path::Path};
fn project() -> (tempfile::TempDir, Config, Graph) {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("src")).unwrap();
    fs::write(root.path().join("src/A.ts"), "export const a = 1;\n").unwrap();
    fs::write(
        root.path().join("src/B.ts"),
        "import {a} from './A';\nexport const b = a;\n",
    )
    .unwrap();
    let mut builder = GraphBuilder::new();
    let a = builder.ensure_path("A.ts", Some("ts"));
    let b = builder.ensure_path("B.ts", Some("ts"));
    builder
        .edge(
            b,
            a,
            Ref {
                file: "src/B.ts".into(),
                line: 1,
                raw: "import".into(),
            },
        )
        .unwrap();
    let graph = builder.build(root.path().to_string_lossy().into_owned());
    (root, Config::default(), graph)
}
const JUNIT: &str = "<testsuites><testsuite name='unit' tests='3'><testcase classname='ATest' name='direct' time='0.125'/><testsuite name='nested'><testcase classname='BTest' name='dependent' time='1.5'><failure message='expected true'>details</failure></testcase><testcase name='unknown'><skipped/></testcase></testsuite></testsuite></testsuites>";
#[test]
fn imports_exact_test_identity_outcome_and_seconds_without_inventing_mapping() {
    let (root, config, graph) = project();
    fs::write(root.path().join("junit.xml"), JUNIT).unwrap();
    let before = test_evidence_fingerprint(root.path());
    let report = import_test_evidence(
        root.path(),
        &config,
        &graph,
        Path::new("junit.xml"),
        None,
        None,
    )
    .unwrap();
    assert_eq!(report.status, "partial");
    assert_eq!(report.tests.len(), 3);
    assert_eq!(report.tests[0].duration_seconds, Some(0.125));
    assert_eq!(report.tests[0].outcome, "passed");
    assert_eq!(report.tests[1].suite, "unit/nested");
    assert_eq!(report.tests[1].outcome, "failed");
    assert_eq!(report.tests[2].duration_seconds, None);
    assert_eq!(report.tests[2].outcome, "skipped");
    assert!(report.tests.iter().all(|t| t.mappings.is_empty()));
    assert_ne!(before, test_evidence_fingerprint(root.path()));
    fs::remove_file(root.path().join("junit.xml")).unwrap();
    assert_eq!(read_test_evidence(root.path(), &config, &graph), report);
    assert!(report.diagnostics.iter().any(|d| d.contains("unverified")));
}
#[test]
fn explicit_mappings_select_direct_and_static_dependent_tests_and_keep_unknowns() {
    let (root, config, graph) = project();
    fs::write(root.path().join("junit.xml"), JUNIT).unwrap();
    fs::write(root.path().join("map.json"),r#"{"schema_version":1,"tests":[{"class_name":"ATest","name":"direct","sources":[{"path":"src/A.ts","lines":[1]}]},{"class_name":"BTest","name":"dependent","sources":[{"path":"src/B.ts","lines":[2]}]}]}"#).unwrap();
    let report = import_test_evidence(
        root.path(),
        &config,
        &graph,
        Path::new("junit.xml"),
        Some(Path::new("map.json")),
        None,
    )
    .unwrap();
    let impact = test_impact(&graph, &report, &["src/A.ts".into()]);
    assert_eq!(impact.selected.len(), 2);
    assert_eq!(impact.unmapped_tests, 1);
    assert_eq!(impact.status, "partial");
    assert!(
        impact
            .diagnostics
            .iter()
            .any(|d| d.contains("not safe to omit"))
    );
    assert!(
        impact
            .selected
            .iter()
            .any(|t| t.name == "direct" && t.reason == "mapped_changed_source")
    );
    assert!(
        impact
            .selected
            .iter()
            .any(|t| t.name == "dependent" && t.reason == "mapped_static_dependent")
    );
    fs::write(root.path().join("src/A.ts"), "export const a = 2;\n").unwrap();
    let stale = read_test_evidence(root.path(), &config, &graph);
    assert_eq!(stale.status, "stale");
    let impact = test_impact(&graph, &stale, &["src/A.ts".into()]);
    assert_eq!(impact.stale_tests, 1);
    assert_eq!(impact.selected.len(), 1);
    assert_eq!(
        test_impact(&graph, &report, &["absent".into()]).unresolved_targets,
        ["absent"]
    );
}
#[test]
fn malformed_or_ambiguous_evidence_never_replaces_snapshot() {
    let (root, config, graph) = project();
    fs::write(root.path().join("junit.xml"), JUNIT).unwrap();
    import_test_evidence(
        root.path(),
        &config,
        &graph,
        Path::new("junit.xml"),
        None,
        None,
    )
    .unwrap();
    let before = fs::read(root.path().join(".archguard/tests.json")).unwrap();
    for xml in [
        "<not-tests/>",
        "<testsuite><testcase name='x' time='NaN'/></testsuite>",
        "<testsuite><testcase name='x' time='-1'/></testsuite>",
        "<!DOCTYPE x [<!ENTITY x SYSTEM 'file:///etc/passwd'>]><testsuite><testcase name='&x;'/></testsuite>",
    ] {
        fs::write(root.path().join("bad.xml"), xml).unwrap();
        assert!(
            import_test_evidence(
                root.path(),
                &config,
                &graph,
                Path::new("bad.xml"),
                None,
                None
            )
            .is_err()
        );
        assert_eq!(
            fs::read(root.path().join(".archguard/tests.json")).unwrap(),
            before
        );
    }
    fs::write(
        root.path().join("dupes.xml"),
        "<testsuite><testcase name='x'/><testcase name='x'/></testsuite>",
    )
    .unwrap();
    fs::write(
        root.path().join("map.json"),
        r#"{"schema_version":1,"tests":[{"name":"x","sources":[{"path":"src/A.ts"}]}]}"#,
    )
    .unwrap();
    assert!(
        import_test_evidence(
            root.path(),
            &config,
            &graph,
            Path::new("dupes.xml"),
            Some(Path::new("map.json")),
            None
        )
        .is_err()
    );
    assert_eq!(
        fs::read(root.path().join(".archguard/tests.json")).unwrap(),
        before
    );
}
#[test]
fn unsafe_paths_and_invalid_line_mappings_are_rejected() {
    let (root, config, graph) = project();
    fs::write(root.path().join("junit.xml"), JUNIT).unwrap();
    for source in [
        r#"{"path":"../outside.ts"}"#,
        r#"{"path":"src/A.ts","lines":[0]}"#,
        r#"{"path":"src/A.ts","lines":[999]}"#,
    ] {
        fs::write(
            root.path().join("map.json"),
            format!(r#"{{"schema_version":1,"tests":[{{"name":"direct","sources":[{source}]}}]}}"#),
        )
        .unwrap();
        assert!(
            import_test_evidence(
                root.path(),
                &config,
                &graph,
                Path::new("junit.xml"),
                Some(Path::new("map.json")),
                None
            )
            .is_err()
        );
    }
    assert!(!root.path().join(".archguard/tests.json").exists());
}

#[cfg(unix)]
#[test]
fn special_file_input_is_rejected_without_waiting_for_a_writer() {
    use std::{sync::mpsc, time::Duration};
    let (root, config, graph) = project();
    let fifo = root.path().join("report.fifo");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let (send, recv) = mpsc::channel();
    let path = root.path().to_owned();
    let worker = std::thread::spawn(move || {
        let result =
            import_test_evidence(&path, &config, &graph, Path::new("report.fifo"), None, None);
        send.send(result.is_err()).unwrap();
    });
    let result = recv.recv_timeout(Duration::from_millis(200));
    if result.is_err() {
        let writer = fs::OpenOptions::new().write(true).open(&fifo).unwrap();
        drop(writer);
    }
    worker.join().unwrap();
    assert_eq!(
        result,
        Ok(true),
        "regular-file validation must not block on a pipe"
    );
}

#[test]
fn xml_node_budget_bounds_non_test_metadata_too() {
    let (root, config, graph) = project();
    let xml = format!(
        "<testsuite><testcase name='x'>{}</testcase></testsuite>",
        "<property/>".repeat(500_000)
    );
    fs::write(root.path().join("large.xml"), xml).unwrap();
    assert!(
        import_test_evidence(
            root.path(),
            &config,
            &graph,
            Path::new("large.xml"),
            None,
            None
        )
        .is_err()
    );
    assert!(!root.path().join(".archguard/tests.json").exists());
}
