//! Runs a crate's extractors over a project tree made of broken variants.
#![allow(dead_code)]
use std::path::Path;

use crate::variants::variants;

/// For every variant index, writes each `(path, seed)` file with that variant of its seed
/// into a fresh directory and calls `run` on it. A panic in `run` fails the test.
pub fn each_broken_tree(files: &[(&str, &str)], run: impl Fn(&Path)) {
    let all: Vec<Vec<Vec<u8>>> = files.iter().map(|(_, s)| variants(s)).collect();
    let most = all.iter().map(Vec::len).max().unwrap_or(0);
    for i in 0..most {
        let dir = tempfile::tempdir().unwrap();
        for ((name, _), vs) in files.iter().zip(&all) {
            let path = dir.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, &vs[i.min(vs.len() - 1)]).unwrap();
        }
        eprintln!("variant {i}");
        run(dir.path());
    }
}
