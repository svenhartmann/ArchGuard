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

/// Writes the unbroken seeds and calls `check`: a seed no extractor reads would make the
/// broken variants test nothing, so `check` asserts real output on it.
pub fn intact_tree(files: &[(&str, &str)], check: impl Fn(&Path)) {
    let dir = tempfile::tempdir().unwrap();
    for (name, seed) in files {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, seed).unwrap();
    }
    check(dir.path());
}

/// Asserts `flow` has screens, transitions, actions or endpoints (not only diagnostics).
pub fn assert_flow(what: &str, flow: &archguard_core::UiFlow) {
    assert!(
        !(flow.screens.is_empty()
            && flow.transitions.is_empty()
            && flow.actions.is_empty()
            && flow.endpoints.is_empty()),
        "{what}: seed gives no output, diagnostics: {:?}",
        flow.diagnostics
    );
}

/// Asserts `model` has entities.
pub fn assert_domain(what: &str, model: &archguard_core::domain_model::DomainModel) {
    assert!(!model.entities.is_empty(), "{what}: seed gives no entities");
}
