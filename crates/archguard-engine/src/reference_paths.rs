//! Resolve source-root-stripped import references and logical node names
//! to unambiguous physical files.
use archguard_core::{ArchguardError, Config};
use archguard_extract::Lang;
use std::{collections::BTreeMap, path::Path};
#[tracing::instrument(skip_all)]
pub fn source_reference_paths(
    root: &Path,
    config: &Config,
) -> Result<BTreeMap<String, String>, ArchguardError> {
    let mut candidates = BTreeMap::<String, Vec<String>>::new();
    for file in crate::pipeline::walk_files(root, config)? {
        let stripped = archguard_extract::strip_root(&file.rel_path, &config.project.source_roots);
        // Java, Kotlin, C# and PHP nodes are named after their package (ADR 0010).
        let logical = matches!(
            file.lang,
            Lang::Java | Lang::Kotlin | Lang::CSharp | Lang::Php
        );
        if logical
            && let Some(name) = crate::pipeline::source_node_name(&file, &stripped, None, None)?
            && name != stripped
        {
            candidates
                .entry(name)
                .or_default()
                .push(file.rel_path.clone());
        }
        candidates.entry(stripped).or_default().push(file.rel_path);
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

#[cfg(test)]
mod tests {
    /// A Java file is also found by its node name, which names its package
    /// below its project (ADR 0010), not its directory.
    #[test]
    fn logical_node_names_resolve_to_their_file() {
        let dir = tempfile::tempdir().unwrap();
        let java = dir.path().join("functions/a/src/main/java/de/x");
        std::fs::create_dir_all(&java).unwrap();
        std::fs::write(dir.path().join("functions/a/pom.xml"), "<project/>").unwrap();
        std::fs::write(java.join("App.java"), "package de.x;\nclass App {}\n").unwrap();
        let paths =
            super::source_reference_paths(dir.path(), &archguard_core::Config::default()).unwrap();
        assert_eq!(
            paths.get("functions/a/de/x/App.java").map(String::as_str),
            Some("functions/a/src/main/java/de/x/App.java")
        );
    }
}
