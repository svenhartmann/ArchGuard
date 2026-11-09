//! A source file that is not valid UTF-8 (here a Latin-1 Python comment)
//! must not abort the analysis: it is decoded lossily and still analysed
//! next to the other files.

use std::fs;

use archguard_core::Config;
use archguard_engine::{Level, analyze};

#[test]
fn a_latin1_python_file_is_analysed_next_to_a_utf8_one() {
    let dir = tempfile::tempdir().expect("temp dir");
    let src = dir.path().join("src");
    fs::create_dir_all(&src).expect("create src");
    // `# café` in Latin-1: 0xE9 is not valid UTF-8.
    fs::write(src.join("legacy.py"), b"# caf\xe9\nimport fresh\n").expect("write legacy.py");
    fs::write(src.join("fresh.py"), "VALUE = 1\n").expect("write fresh.py");

    let (graph, _report) =
        analyze(dir.path(), &Config::default(), Level::Module).expect("analyze succeeds");

    let names: Vec<&str> = graph.nodes.iter().map(|node| node.name.as_str()).collect();
    assert!(
        names.iter().any(|name| name.contains("legacy")),
        "nodes: {names:?}"
    );
    assert!(
        names.iter().any(|name| name.contains("fresh")),
        "nodes: {names:?}"
    );
}
