//! Source review evidence and configured entry points, formerly read from
//! the web viewer's payload.
use archguard_engine::analysis::match_review_entry_points;
use archguard_engine::{Level, analyze_run, collect_review_sources, load_config};
use std::fs;

fn project(files: &[(&str, &str)], config: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (path, source) in files {
        let file = dir.path().join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, source).unwrap();
    }
    fs::write(dir.path().join("archguard.toml"), config).unwrap();
    dir
}

#[test]
fn configured_entry_points_match_their_source_file() {
    let dir = project(
        &[("entry.ts", "export const entry = 1;\n")],
        "derive_layers=false\n[project]\nsource_roots=[]\nlanguages=['ts']\n[review]\nentry_points=['entry.ts']\ngit_since='180.days'\n",
    );
    let loaded = load_config(None, dir.path()).unwrap();
    let run = analyze_run(dir.path(), &loaded, Level::Module).unwrap();
    let sources = collect_review_sources(
        dir.path(),
        &run.config,
        &run.graph,
        &run.config.review.git_since,
    )
    .unwrap();
    assert_eq!(sources.files[0].path.as_deref(), Some("entry.ts"));
    assert_eq!(sources.files[0].lines, Some(1));
    let ids =
        match_review_entry_points(&run.graph, &sources, &run.config.review.entry_points).unwrap();
    assert_eq!(ids.len(), 1);
}

#[test]
fn every_source_file_is_mapped_below_the_module_depth() {
    let dir = project(
        &[
            ("app/main.ts", "import '../lib/service';\n"),
            ("lib/service.ts", "export const service = 1;\n"),
            ("other/unused.ts", "export const unused = 1;\n"),
        ],
        "derive_layers=false\n[project]\nsource_roots=[]\nlanguages=['ts']\n[modules]\ndepth=0\n",
    );
    let loaded = load_config(None, dir.path()).unwrap();
    let run = analyze_run(dir.path(), &loaded, Level::Module).unwrap();
    let sources = collect_review_sources(
        dir.path(),
        &run.config,
        &run.graph,
        &run.config.review.git_since,
    )
    .unwrap();
    assert_eq!(sources.mapped_files, 3);
    assert!(
        sources
            .files
            .iter()
            .any(|s| s.path.as_deref() == Some("lib/service.ts") && s.lines == Some(1))
    );
}
