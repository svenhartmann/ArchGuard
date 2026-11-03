use archguard_engine::core::{FindingKind, Status};
use archguard_engine::{Level, analyze_run, load_config};
use std::fs;

#[test]
fn thresholds_have_strict_boundaries_and_cycle_independence() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/a.ts"), "import './b';").unwrap();
    fs::write(dir.path().join("src/b.ts"), "import './c';").unwrap();
    fs::write(dir.path().join("src/c.ts"), "export const c = 1;").unwrap();
    let run = |rules: &str, layer: &str| {
        fs::write(
            dir.path().join("archguard.toml"),
            format!("derive_layers = false\n{layer}\n[rules]\ndeny_cycles = 'off'\n{rules}"),
        )
        .unwrap();
        analyze_run(
            dir.path(),
            &load_config(None, dir.path()).unwrap(),
            Level::Module,
        )
        .unwrap()
        .report
    };
    for (max, status) in [
        (0.0, Status::Pass),
        (0.1, Status::Fail),
        (2.0, Status::Pass),
        (1.99, Status::Fail),
    ] {
        let report = run(&format!("max_acd = {max}"), "");
        assert_eq!(report.summary.acd, 2.0);
        assert_eq!(report.summary.status, status, "max_acd={max}");
    }
    let layer = "[[layers]]\nname = 'not_named_core'\ncontains = ['a.ts']\nmay_depend_on = []";
    for (max, status) in [
        (0.0, Status::Pass),
        (1.0, Status::Pass),
        (0.9, Status::Fail),
    ] {
        let report = run(&format!("max_instability_core = {max}"), layer);
        assert_eq!(report.summary.status, status);
    }
    fs::write(dir.path().join("src/b.ts"), "import './a';").unwrap();
    for (max, status) in [
        (0, Status::Fail),
        (1, Status::Fail),
        (2, Status::Pass),
        (3, Status::Pass),
    ] {
        let report = run(&format!("max_scc_size = {max}"), "");
        assert_eq!(report.summary.status, status);
        assert_eq!(
            report
                .findings
                .iter()
                .filter(|f| f.kind == FindingKind::MetricThreshold)
                .count(),
            usize::from(status == Status::Fail)
        );
    }
}

#[test]
fn module_metrics_include_collapsed_module_nodes() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["a", "b"] {
        fs::create_dir_all(dir.path().join(format!("src/{name}"))).unwrap();
    }
    fs::write(dir.path().join("src/a/x.ts"), "import '../b/y';").unwrap();
    fs::write(dir.path().join("src/b/y.ts"), "export const y = 1;").unwrap();
    fs::write(dir.path().join("archguard.toml"), "derive_layers = false\n[modules]\ndepth = 0\n[[layers]]\nname = 'base'\ncontains = ['a']\n[rules]\nmax_instability_core = 0.5").unwrap();
    let report = analyze_run(
        dir.path(),
        &load_config(None, dir.path()).unwrap(),
        Level::Module,
    )
    .unwrap()
    .report;
    assert_eq!(report.metrics["a"].instability, 1.0);
    assert_eq!(report.summary.status, Status::Fail);
}
