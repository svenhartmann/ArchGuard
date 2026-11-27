//! Source files with a byte order mark: the main graph build reads the text,
//! not the mark (UTF-8) and not mojibake (UTF-16).

use std::fs;

use archguard_core::Config;
use archguard_engine::{Level, analyze};

fn utf16(text: &str, bom: [u8; 2], le: bool) -> Vec<u8> {
    let mut out = bom.to_vec();
    for unit in text.encode_utf16() {
        out.extend(if le {
            unit.to_le_bytes()
        } else {
            unit.to_be_bytes()
        });
    }
    out
}

/// Writes `bytes` as `src/importer.py` next to `src/fresh.py`; true when the
/// graph has an edge importer -> fresh.
fn imports_fresh(bytes: &[u8]) -> bool {
    let dir = tempfile::tempdir().expect("temp dir");
    let src = dir.path().join("src");
    fs::create_dir_all(&src).expect("create src");
    fs::write(src.join("importer.py"), bytes).expect("write importer.py");
    fs::write(src.join("fresh.py"), "VALUE = 1\n").expect("write fresh.py");
    let (graph, _) = analyze(dir.path(), &Config::default(), Level::Module).expect("analyze");
    let id = |part: &str| {
        let node = graph.nodes.iter().find(|n| n.name.contains(part));
        node.map(|n| n.id.clone())
            .unwrap_or_else(|| panic!("no node {part}"))
    };
    let (from, to) = (id("importer"), id("fresh"));
    graph.edges.iter().any(|e| e.from == from && e.to == to)
}

#[test]
fn plain_utf8_import_is_an_edge() {
    assert!(imports_fresh(b"import fresh\n"));
}

#[test]
fn utf8_bom_does_not_reach_the_first_token() {
    assert!(imports_fresh(b"\xEF\xBB\xBFimport fresh\n"));
}

#[test]
fn utf16le_file_is_decoded() {
    assert!(imports_fresh(&utf16("import fresh\n", [0xFF, 0xFE], true)));
}

#[test]
fn utf16be_file_is_decoded() {
    assert!(imports_fresh(&utf16("import fresh\n", [0xFE, 0xFF], false)));
}
