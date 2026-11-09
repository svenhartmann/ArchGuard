//! ArchGuard records its own runtime (ADR 0004): `ARCHGUARD_PROBE=1` makes
//! the real binary write OTLP JSON Lines that its own importer reads back
//! as requests with the pipeline phases as method spans.
use archguard_engine::core::RuntimeRequest;
use std::path::Path;
use std::{fs, process::Command};

/// A two-file TypeScript project with one import.
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("src/app")).unwrap();
    fs::create_dir_all(dir.path().join("src/lib")).unwrap();
    fs::write(
        dir.path().join("src/app/main.ts"),
        "import { two } from \"../lib/two\";\nexport const one = (): number => two();\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("src/lib/two.ts"),
        "export const two = (): number => 2;\n",
    )
    .unwrap();
    dir
}

fn check(dir: &Path, env: &[(&str, &Path)]) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_archguard"));
    command
        .args(["check", "--format", "json"])
        .arg(dir)
        .env_remove("ARCHGUARD_PROBE")
        .env_remove("ARCHGUARD_PROBE_DIR");
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.code().is_some_and(|c| c <= 1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn probed(dir: &Path) {
    check(dir, &[("ARCHGUARD_PROBE", Path::new("1"))]);
}

fn file_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn requests(root: &Path) -> Vec<RuntimeRequest> {
    archguard_engine::runtime_profile(root, None)
        .expect("a runtime profile")
        .requests
}

#[test]
fn a_probed_check_is_a_request_with_the_pipeline_phases() {
    let dir = project();
    probed(dir.path());
    let runs = dir.path().join(".archguard/runtime/archguard");
    assert_eq!(file_names(&runs), ["traces-0001.jsonl"]);

    let requests = requests(dir.path());
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.name, "archguard check");
    assert_eq!(request.service, "archguard");
    assert!(!request.spans_truncated);
    let span = |name: &str| {
        let found = request.spans.iter().find(|s| s.name == name);
        found.unwrap_or_else(|| panic!("no span {name} in {:#?}", request.spans))
    };
    for name in [
        "config.load_config",
        "effective.run_inner",
        "pipeline.walk_files",
        "pipeline.build_graph_named",
        "extract ts",
        "pipeline.resolve_imports",
        "kpi_payload.compute_review_kpis",
        "pipeline.build_report",
        "scc.find_sccs",
        "layers.check_layers",
        "json.to_json",
    ] {
        let s = span(name);
        assert_eq!(s.kind, "internal");
        assert!(s.duration_ms >= 0.0 && s.duration_ms <= request.duration_ms);
        assert!(s.offset_ms >= 0.0 && s.offset_ms + s.duration_ms <= request.duration_ms + 0.01);
    }
    let pipeline = span("effective.run_inner");
    assert_eq!(pipeline.depth, 1);
    assert!(pipeline.duration_ms > 0.0);
    assert!(pipeline.duration_ms >= span("pipeline.build_graph_named").duration_ms);
    assert_eq!(span("extract ts").depth, pipeline.depth + 2);
    assert_eq!(
        pipeline.function.as_deref(),
        Some("archguard_engine.effective.run_inner")
    );
    assert_eq!(
        pipeline.file.as_deref(),
        Some("crates/archguard-engine/src/effective.rs")
    );
}

#[test]
fn every_run_writes_the_next_file_and_adds_a_request() {
    let dir = project();
    probed(dir.path());
    probed(dir.path());
    let runs = dir.path().join(".archguard/runtime/archguard");
    assert_eq!(
        file_names(&runs),
        ["traces-0001.jsonl", "traces-0002.jsonl"]
    );
    assert_eq!(requests(dir.path()).len(), 2);
}

#[test]
fn without_the_variable_nothing_is_written() {
    let dir = project();
    check(dir.path(), &[]);
    assert!(!dir.path().join(".archguard/runtime").exists());
}

#[test]
fn the_probe_dir_variable_moves_the_output_and_no_path_is_written() {
    let dir = project();
    let out = tempfile::tempdir().unwrap();
    let target = out.path().join("self");
    check(
        dir.path(),
        &[
            ("ARCHGUARD_PROBE", Path::new("1")),
            ("ARCHGUARD_PROBE_DIR", &target),
        ],
    );
    assert!(!dir.path().join(".archguard/runtime").exists());
    let text = fs::read_to_string(target.join("traces-0001.jsonl")).unwrap();
    let project = dir.path().to_string_lossy();
    assert!(!text.contains(project.as_ref()), "the project path leaked");
    assert!(!text.contains("/Users/") && !text.contains("/home/"));
    assert!(text.contains("\"archguard check\""));
}
