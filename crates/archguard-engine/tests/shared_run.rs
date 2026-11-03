use archguard_engine::{Level, analyze_effective, analyze_run, analyze_run_quiet, load_config};

fn warning_fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/a.ts"),
        "import './b';\nimport 'external';",
    )
    .unwrap();
    std::fs::write(dir.path().join("src/b.ts"), "import './a';").unwrap();
    dir
}

#[test]
fn quiet_run_preserves_cold_and_warm_results_and_warnings() {
    let dir = warning_fixture();
    let loaded = load_config(None, dir.path()).unwrap();
    let cold = analyze_run_quiet(dir.path(), &loaded, Level::Module).unwrap();
    let warm = analyze_run_quiet(dir.path(), &loaded, Level::Module).unwrap();
    let ordinary = analyze_run(dir.path(), &loaded, Level::Module).unwrap();

    assert!(!cold.cache_hit);
    assert!(warm.cache_hit && ordinary.cache_hit);
    assert!(!cold.warnings.is_empty());
    for run in [&warm, &ordinary] {
        assert_eq!(cold.graph, run.graph);
        assert_eq!(cold.view, run.view);
        assert_eq!(cold.report, run.report);
        assert_eq!(cold.effective_config(), run.effective_config());
        assert_eq!(cold.warnings, run.warnings);
    }
}

#[test]
fn quiet_run_does_not_write_warnings() {
    const CHILD: &str = "ARCHGUARD_QUIET_ANALYSIS_TEST_CHILD";
    if let Ok(mode) = std::env::var(CHILD) {
        let dir = warning_fixture();
        let loaded = load_config(None, dir.path()).unwrap();
        for _ in 0..2 {
            let run = if mode == "ordinary" {
                analyze_run(dir.path(), &loaded, Level::Module)
            } else {
                analyze_run_quiet(dir.path(), &loaded, Level::Module)
            }
            .unwrap();
            assert!(
                !run.warnings.is_empty(),
                "diagnostics remain available to the caller"
            );
        }
        return;
    }

    // Capture the OS stderr stream in a subprocess: Rust's default test capture
    // would otherwise hide eprintln! and let a noisy polling API pass this test.
    for mode in ["quiet", "ordinary"] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "quiet_run_does_not_write_warnings",
                "--nocapture",
            ])
            .env(CHILD, mode)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        if mode == "quiet" {
            assert!(
                output.stderr.is_empty(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        } else {
            assert!(String::from_utf8_lossy(&output.stderr).contains("unresolved external import"));
        }
    }
}

#[test]
fn derived_view_and_config_agree_cold_and_warm() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..10 {
        let path = dir.path().join(format!("src/g{i:02}/nested"));
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("a.ts"), "import './b';").unwrap();
        std::fs::write(path.join("b.ts"), "import './a';").unwrap();
    }
    for config in [
        "",
        "[modules]\ndepth = 3\n[[layers]]\nname = 'custom'\ncontains = ['g00/**']",
    ] {
        std::fs::write(dir.path().join("archguard.toml"), config).unwrap();
        let loaded = load_config(None, dir.path()).unwrap();
        for level in [Level::Module, Level::File] {
            let uncached = analyze_effective(dir.path(), &loaded, level).unwrap();
            let first = analyze_run(dir.path(), &loaded, level).unwrap();
            let second = analyze_run(dir.path(), &loaded, level).unwrap();
            assert!(second.cache_hit);
            assert_eq!(uncached, (first.graph.clone(), first.report.clone()));
            assert_eq!(first.report, second.report);
            assert_eq!(first.view, second.view);
            assert_eq!(first.effective_config(), second.effective_config());
            assert_eq!(
                first.report.metrics,
                archguard_engine::analysis::compute_metrics(
                    &first.view,
                    &archguard_engine::analysis::find_sccs(&first.view)
                )
            );
        }
    }
}
