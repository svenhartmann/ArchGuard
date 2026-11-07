//! `measure_review_zones`: the opt-in compiler chain that fills the Martin
//! A/I zones the pipeline leaves unmeasured. Like the other Java tests in
//! this crate the Java case needs a JDK on PATH (javac backs the
//! abstractness); Rust abstractness comes from syntax and needs no tool.
use archguard_engine::core::ZoneStats;
use archguard_engine::{
    Level, analyze_run_quiet, load_config, measure_review_zones, review_performance,
};
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

fn rust_crate(root: &Path, port: &str) {
    put(root, "src/lib.rs", "pub mod a;\npub mod b;\n");
    put(root, "src/a/mod.rs", "pub mod port;\n");
    put(root, "src/a/port.rs", port);
    put(root, "src/b/mod.rs", "pub mod adapter;\n");
    put(
        root,
        "src/b/adapter.rs",
        "use crate::a::port::Port;\npub struct Adapter;\nimpl Port for Adapter { fn run(&self) {} }\n",
    );
}

/// Abstractness per module of the opt-in review chain, by module name.
fn abstractness(root: &Path) -> Vec<(String, Option<f64>)> {
    let loaded = load_config(None, root).unwrap();
    let run = analyze_run_quiet(root, &loaded, Level::Module).unwrap();
    let report = review_performance(root, &run.config, &run.graph, &run.view).unwrap();
    let mut metrics: Vec<_> = report
        .abstractions
        .iter()
        .filter(|metric| metric.scope == "node" && metric.types > 0)
        .map(|metric| (metric.name.clone(), metric.abstractness))
        .collect();
    metrics.sort_by(|a, b| a.0.cmp(&b.0));
    metrics
}

#[test]
fn rust_traits_fill_the_zones_from_syntax() {
    let dir = tempfile::tempdir().unwrap();
    rust_crate(dir.path(), "pub trait Port { fn run(&self); }\n");
    // a declares only a trait, b only a struct; impl blocks and modules are no types.
    assert_eq!(
        abstractness(dir.path()),
        [
            ("a/port.rs".to_string(), Some(1.0)),
            ("b/adapter.rs".to_string(), Some(0.0))
        ]
    );
    assert!(
        zones(dir.path()).is_some(),
        "Rust declarations measure boundaries without opt-in"
    );
}

#[test]
fn rust_item_macros_keep_their_module_unmeasured() {
    let dir = tempfile::tempdir().unwrap();
    rust_crate(
        dir.path(),
        "pub trait Port { fn run(&self); }\nsome_crate::declare_types!();\n",
    );
    let measured = abstractness(dir.path());
    assert_eq!(
        measured[0],
        ("a/port.rs".to_string(), None),
        "a macro may declare more types"
    );
    assert_eq!(measured[1], ("b/adapter.rs".to_string(), Some(0.0)));
}
