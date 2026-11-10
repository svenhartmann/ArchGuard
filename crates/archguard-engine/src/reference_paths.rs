//! Resolve source-root-stripped import references to unambiguous physical files.
use archguard_core::{ArchguardError, Config};
use std::{collections::BTreeMap, path::Path};
#[tracing::instrument(skip_all)]
pub fn source_reference_paths(
    root: &Path,
    config: &Config,
) -> Result<BTreeMap<String, String>, ArchguardError> {
    let mut candidates = BTreeMap::<String, Vec<String>>::new();
    for file in crate::pipeline::walk_files(root, config)? {
        candidates
            .entry(archguard_extract::strip_root(
                &file.rel_path,
                &config.project.source_roots,
            ))
            .or_default()
            .push(file.rel_path);
    }
    Ok(candidates
        .into_iter()
        .filter_map(|(reference, mut files)| {
            files.sort();
            files.dedup();
            (files.len() == 1).then(|| (reference, files.remove(0)))
        })
        .collect())
}
