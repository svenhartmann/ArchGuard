//! `measure_review_zones`: the opt-in compiler chain that fills the Martin
//! A/I zones the pipeline leaves unmeasured. Like the other Java tests in
//! this crate it needs a JDK on PATH (javac backs the abstractness).
use archguard_engine::core::ZoneStats;
use archguard_engine::{Level, analyze_run_quiet, load_config, measure_review_zones};
use std::path::Path;

fn put(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

fn zones(root: &Path) -> Option<ZoneStats> {
    let loaded = load_config(None, root).unwrap();
    let run = analyze_run_quiet(root, &loaded, Level::Module).unwrap();
    assert!(
        run.report.review_kpis.as_ref().unwrap().zones.is_none(),
        "the pipeline itself never measures zones"
    );
    measure_review_zones(root, &run.config, &run.graph, &run.view).unwrap()
}

#[test]
fn java_abstractness_fills_the_zones() {
    let dir = tempfile::tempdir().unwrap();
    put(
        dir.path(),
        "src/a/Port.java",
        "package a; public interface Port { void run(); }\n",
    );
    put(
        dir.path(),
        "src/b/Adapter.java",
        "package b; public class Adapter implements a.Port { public void run() {} }\n",
    );
    let zones = zones(dir.path()).expect("complete javac coverage measures a boundary");
    // a: interface only, depended on (A=1, I=0); b: concrete, depends (A=0, I=1).
    assert_eq!(
        zones,
        ZoneStats {
            main_sequence: 2,
            ..Default::default()
        }
    );
}

#[test]
fn languages_without_abstractness_stay_not_measured() {
    let dir = tempfile::tempdir().unwrap();
    put(
        dir.path(),
        "src/app/one.ts",
        "import { two } from '../lib/two';\nexport const one = two;\n",
    );
    put(dir.path(), "src/lib/two.ts", "export const two = 2;\n");
    assert_eq!(
        zones(dir.path()),
        None,
        "no measurable boundary is None, never zero"
    );
}
