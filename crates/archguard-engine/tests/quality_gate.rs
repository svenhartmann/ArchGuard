use archguard_engine::core::{Config, Graph, GraphBuilder};
use archguard_engine::{QualityGateOptions, evaluate_quality_gate, import_test_coverage};
use std::{fs, path::Path, process::Command};

fn git(root: &Path, args: &[&str]) {
    let result = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
fn project() -> (tempfile::TempDir, Config, Graph) {
    let root = tempfile::tempdir().unwrap();
    git(root.path(), &["init", "--quiet", "--template="]);
    git(
        root.path(),
        &["config", "user.email", "test@example.invalid"],
    );
    git(root.path(), &["config", "user.name", "Test"]);
    fs::create_dir(root.path().join("src")).unwrap();
    fs::write(
        root.path().join("src/a.py"),
        "def answer():\n    return 1\n",
    )
    .unwrap();
    git(root.path(), &["add", "src/a.py"]);
    git(root.path(), &["commit", "--quiet", "-m", "base"]);
    let mut config = Config::default();
    config.project.source_roots = vec!["src".into()];
    let mut builder = GraphBuilder::new();
    builder.ensure_path("a.py", Some("python"));
    let graph = builder.build(root.path().to_string_lossy().into_owned());
    (root, config, graph)
}
fn coverage(root: &Path, config: &Config, graph: &Graph, hits: u32) {
    fs::write(root.join("coverage.xml"),format!("<coverage><packages><package><classes><class filename='src/a.py'><lines><line number='1' hits='1'/><line number='2' hits='{hits}'/></lines></class></classes></package></packages></coverage>")).unwrap();
    import_test_coverage(root, config, graph, Path::new("coverage.xml"), None).unwrap();
}
#[test]
fn lcov_detail_feeds_the_changed_line_gate_like_cobertura() {
    let (root, config, graph) = project();
    fs::write(
        root.path().join("src/a.py"),
        "def answer():\n    return 2\n",
    )
    .unwrap();
    for hits in ["0", "1"] {
        fs::write(
            root.path().join("lcov.info"),
            format!("TN:\nSF:src/a.py\nDA:1,1\nDA:2,{hits}\nend_of_record\n"),
        )
        .unwrap();
        import_test_coverage(root.path(), &config, &graph, Path::new("lcov.info"), None).unwrap();
        let options = QualityGateOptions {
            min_changed_coverage: Some(80.0),
            ..Default::default()
        };
        let report = evaluate_quality_gate(root.path(), &config, &graph, "HEAD", &options).unwrap();
        let measured = report.coverage.as_ref().unwrap();
        assert_eq!(
            (measured.covered_lines, measured.total_lines),
            if hits == "0" { (0, 1) } else { (1, 1) },
            "LCOV line hits must reach the gate, not degrade it to coverage_unavailable"
        );
        assert_eq!(report.status, if hits == "0" { "fail" } else { "pass" });
    }
}

#[test]
fn changed_line_coverage_counts_actual_added_executable_lines_and_preserves_git() {
    let (root, config, graph) = project();
    fs::write(
        root.path().join("src/a.py"),
        "def answer():\n    return 2\n",
    )
    .unwrap();
    coverage(root.path(), &config, &graph, 0);
    let before = fs::read(root.path().join(".git/index")).unwrap();
    let options = QualityGateOptions {
        min_changed_coverage: Some(80.0),
        ..Default::default()
    };
    let report = evaluate_quality_gate(root.path(), &config, &graph, "HEAD", &options).unwrap();
    assert_eq!(report.status, "fail");
    assert_eq!(report.changed_files, 1);
    assert_eq!(report.changed_lines, 1);
    let value = serde_json::to_value(&report).unwrap();
    assert_eq!(value["coverage"]["files"][0]["path"], "src/a.py");
    assert_eq!(
        value["coverage"]["files"][0]["uncovered_lines"],
        serde_json::json!([2])
    );
    let measured = report.coverage.unwrap();
    assert_eq!((measured.covered_lines, measured.total_lines), (0, 1));
    assert_eq!(measured.percent, Some(0.0));
    assert!(report.findings.iter().any(|f| f.kind == "changed_coverage"));
    assert_eq!(before, fs::read(root.path().join(".git/index")).unwrap());
    coverage(root.path(), &config, &graph, 1);
    assert_eq!(
        evaluate_quality_gate(root.path(), &config, &graph, "HEAD", &options)
            .unwrap()
            .status,
        "pass"
    );
}

#[test]
fn git_scope_includes_staged_unstaged_new_sources_and_literal_paths() {
    let (root, mut config, _) = project();
    fs::create_dir_all(root.path().join("app/src")).unwrap();
    let path = "app/src/a [x]\nline.py";
    fs::write(root.path().join(path), "one = 1\ntwo = 2\n").unwrap();
    git(root.path(), &["add", "--", path]);
    git(root.path(), &["commit", "--quiet", "-m", "nested"]);
    fs::write(root.path().join(path), "one = 10\ntwo = 2\n").unwrap();
    git(root.path(), &["add", "--", path]);
    fs::write(root.path().join(path), "one = 10\ntwo = 20\n").unwrap();
    fs::write(root.path().join("app/src/new.py"), "new_value = 1\n").unwrap();
    fs::write(root.path().join("src/a.py"), "outside = 99\n").unwrap();
    fs::write(root.path().join("app/.gitignore"), "src/ignored.py\n").unwrap();
    fs::write(root.path().join("app/src/ignored.py"), "ignored = 1\n").unwrap();
    config.project.source_roots = vec!["src".into()];
    let mut builder = GraphBuilder::new();
    builder.ensure_path("a [x]\nline.py", Some("python"));
    builder.ensure_path("new.py", Some("python"));
    let app = root.path().join("app");
    let graph = builder.build(app.to_string_lossy().into_owned());
    let report = evaluate_quality_gate(
        &app,
        &config,
        &graph,
        "HEAD",
        &QualityGateOptions {
            min_changed_coverage: Some(80.0),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(report.changed_files, 2);
    assert_eq!(report.changed_lines, 3);
    assert_eq!(report.status, "incomplete");
    assert_eq!(report.coverage.unwrap().unknown_files, 2);
    assert!(
        report.findings.iter().all(|finding| !finding
            .path
            .as_deref()
            .unwrap_or("")
            .contains("ignored"))
    );
}
#[test]
fn missing_stale_or_legacy_line_records_never_pass_a_coverage_gate() {
    let (root, config, graph) = project();
    fs::write(
        root.path().join("src/a.py"),
        "def answer():\n    return 2\n",
    )
    .unwrap();
    let options = QualityGateOptions {
        min_changed_coverage: Some(0.0),
        ..Default::default()
    };
    assert_eq!(
        evaluate_quality_gate(root.path(), &config, &graph, "HEAD", &options)
            .unwrap()
            .status,
        "incomplete"
    );
    coverage(root.path(), &config, &graph, 1);
    let path = root.path().join(".archguard/coverage.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["files"][0]
        .as_object_mut()
        .unwrap()
        .remove("line_hits");
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(
        evaluate_quality_gate(root.path(), &config, &graph, "HEAD", &options)
            .unwrap()
            .status,
        "incomplete"
    );
    coverage(root.path(), &config, &graph, 1);
    fs::write(
        root.path().join("src/a.py"),
        "def answer():\n    return 3\n",
    )
    .unwrap();
    assert_eq!(
        evaluate_quality_gate(root.path(), &config, &graph, "HEAD", &options)
            .unwrap()
            .status,
        "incomplete"
    );
}
#[test]
fn deletion_only_and_empty_changes_have_no_fabricated_coverage_percentage() {
    let (root, config, graph) = project();
    let options = QualityGateOptions {
        min_changed_coverage: Some(90.0),
        ..Default::default()
    };
    let report = evaluate_quality_gate(root.path(), &config, &graph, "HEAD", &options).unwrap();
    assert_eq!(report.status, "pass");
    assert_eq!(report.coverage.unwrap().percent, None);
    fs::remove_file(root.path().join("src/a.py")).unwrap();
    let report = evaluate_quality_gate(root.path(), &config, &graph, "HEAD", &options).unwrap();
    assert_eq!(report.status, "pass");
    assert_eq!(report.changed_lines, 0);
    assert_eq!(report.coverage.unwrap().status, "not_applicable");
}
#[test]
fn syntax_only_language_is_incomplete_for_method_limits() {
    let (root, config, graph) = project();
    fs::write(
        root.path().join("src/a.py"),
        "def answer():\n    return 2\n",
    )
    .unwrap();
    let options = QualityGateOptions {
        max_complexity: Some(5),
        ..Default::default()
    };
    let report = evaluate_quality_gate(root.path(), &config, &graph, "HEAD", &options).unwrap();
    // Python is measured from syntax only, which never counts as a complete
    // compiler analysis: the measured method is listed, the gate stays open.
    assert_eq!(report.status, "incomplete");
    assert!(report.findings.iter().any(|finding| {
        finding.kind == "methods_unavailable" && finding.message.contains("python")
    }));
    assert!(
        report
            .methods
            .iter()
            .any(|method| method.id.ends_with("#answer@1"))
    );
}
#[test]
fn invalid_thresholds_and_refs_are_errors_without_command_execution() {
    let (root, config, graph) = project();
    for options in [
        QualityGateOptions::default(),
        QualityGateOptions {
            min_changed_coverage: Some(f64::NAN),
            ..Default::default()
        },
        QualityGateOptions {
            min_changed_coverage: Some(101.0),
            ..Default::default()
        },
        QualityGateOptions {
            max_complexity: Some(0),
            ..Default::default()
        },
    ] {
        assert!(evaluate_quality_gate(root.path(), &config, &graph, "HEAD", &options).is_err());
    }
    assert!(
        evaluate_quality_gate(
            root.path(),
            &config,
            &graph,
            "--output=owned",
            &QualityGateOptions {
                max_nesting: Some(0),
                ..Default::default()
            }
        )
        .is_err()
    );
    assert!(!root.path().join("owned").exists());
}

#[test]
fn out_of_range_report_lines_are_unknown_not_non_executable_changed_code() {
    let (root, config, graph) = project();
    fs::write(
        root.path().join("src/a.py"),
        "def answer():\n    return 2\n",
    )
    .unwrap();
    fs::write(root.path().join("coverage.xml"),"<coverage><packages><package><classes><class filename='src/a.py'><lines><line number='999' hits='1'/></lines></class></classes></package></packages></coverage>").unwrap();
    import_test_coverage(
        root.path(),
        &config,
        &graph,
        Path::new("coverage.xml"),
        None,
    )
    .unwrap();
    let report = evaluate_quality_gate(
        root.path(),
        &config,
        &graph,
        "HEAD",
        &QualityGateOptions {
            min_changed_coverage: Some(80.0),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(report.status, "incomplete");
    assert_eq!(report.coverage.unwrap().unknown_files, 1);
}

#[cfg(unix)]
#[test]
fn git_clean_filters_are_not_executed_by_quality_diff() {
    let (root, config, graph) = project();
    fs::write(
        root.path().join(".gitattributes"),
        "*.py filter=gateprobe\n",
    )
    .unwrap();
    git(
        root.path(),
        &[
            "config",
            "filter.gateprobe.clean",
            "touch gate-filter-ran; cat",
        ],
    );
    git(
        root.path(),
        &["config", "filter.gateprobe.required", "true"],
    );
    fs::write(
        root.path().join("src/a.py"),
        "def answer():\n    return 2\n",
    )
    .unwrap();
    let report = evaluate_quality_gate(
        root.path(),
        &config,
        &graph,
        "HEAD",
        &QualityGateOptions {
            min_changed_coverage: Some(80.0),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(report.status, "incomplete");
    assert!(
        !root.path().join("gate-filter-ran").exists(),
        "quality comparison must not execute repository filter commands"
    );
    git(
        root.path(),
        &[
            "config",
            "filter.gateprobe.process",
            "touch gate-filter-ran; exit 1",
        ],
    );
    let again = evaluate_quality_gate(
        root.path(),
        &config,
        &graph,
        "HEAD",
        &QualityGateOptions {
            min_changed_coverage: Some(80.0),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(again.changed_lines, 1);
    assert!(
        !root.path().join("gate-filter-ran").exists(),
        "long-running process filters must also remain disabled"
    );
}
