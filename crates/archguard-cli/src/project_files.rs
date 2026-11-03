//! Generated project artifacts. Each changed file is replaced atomically;
//! identical snapshots retain their mtime. User policy is never generated here.
use std::io;
use std::path::{Path, PathBuf};

use archguard_engine::{LoadedConfig, core::Graph, write_project_file};

const GENERATED_IGNORE: &str = "# Regenerated local analysis artifacts\n/cache.json\n/graph.json\n/effective-config.json\n/security.json\n/coverage.json\n/quality.json\n/tests.json\n/mutation.json\n/duplicates.json\n/contracts.json\n/review.json\n/services.json\n/reports/\n/graph.db*\n/.*.tmp\n";

pub(crate) fn ensure_ignore_rules(root: &Path) -> io::Result<()> {
    // Existing policy wins. Generated defaults never hide intent, baselines,
    // historical metrics or portable refactoring scenarios.
    match write_project_file(
        root,
        Path::new(".gitignore"),
        GENERATED_IGNORE.as_bytes(),
        false,
    ) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error),
    }
}

fn write_artifact(root: &Path, relative: &Path, text: &str) -> io::Result<PathBuf> {
    ensure_ignore_rules(root)?;
    let mut bytes = text.as_bytes().to_vec();
    if !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    write_project_file(root, relative, &bytes, true)?;
    Ok(root.join(".archguard").join(relative))
}

pub(crate) fn write_report(root: &Path, name: &str, text: &str) -> io::Result<PathBuf> {
    write_artifact(root, &Path::new("reports").join(name), text)
}

pub(crate) fn write_project_graph(
    root: &Path,
    graph: &Graph,
    html: Option<&str>,
) -> io::Result<()> {
    let json = serde_json::to_string_pretty(graph)?;
    write_artifact(root, Path::new("graph.json"), &json)?;
    if let Some(html) = html {
        write_report(root, "graph.html", html)?;
    }
    Ok(())
}

pub(crate) fn write_effective_config(root: &Path, loaded: &LoadedConfig) -> io::Result<()> {
    let json = serde_json::to_string_pretty(
        &serde_json::json!({"config": loaded.config, "sources": loaded.sources}),
    )?;
    write_artifact(root, Path::new("effective-config.json"), &json)?;
    Ok(())
}
