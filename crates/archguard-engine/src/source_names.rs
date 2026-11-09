//! Node names of the extracted files (Kap. 6 step 4), kept with the graph.
//!
//! Graph nodes carry no source path, so every consumer that joins walked
//! files to file nodes needs the name each file was given. The build records
//! it once; the graph cache stores it next to the graph, and consumers that
//! hold the map no longer read and extract every file a second time.
use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use archguard_core::{ArchguardError, Graph};
use archguard_extract::{Declaration, Extraction, Extractor, SourceFile, extractor_for};

/// Graph node name of every extracted file, keyed by its walked `rel_path`.
/// A `rel_path` the walk produced twice (lossy display paths) is left out,
/// so its files are named by extracting them again.
pub type SourceNames = BTreeMap<String, String>;

/// A full graph, plus the [`SourceNames`] of the run that built it when the
/// caller has them. A bare `&Graph` converts too: its names are re-extracted.
#[derive(Debug, Clone, Copy)]
pub struct NamedGraph<'a> {
    pub graph: &'a Graph,
    pub names: Option<&'a SourceNames>,
}

impl<'a> From<&'a Graph> for NamedGraph<'a> {
    fn from(graph: &'a Graph) -> Self {
        NamedGraph { graph, names: None }
    }
}

impl<'a> From<(&'a Graph, &'a SourceNames)> for NamedGraph<'a> {
    fn from((graph, names): (&'a Graph, &'a SourceNames)) -> Self {
        NamedGraph {
            graph,
            names: Some(names),
        }
    }
}

/// The [`SourceNames`] of `(rel_path, node_name)` pairs.
pub(super) fn names<'a>(files: impl Iterator<Item = (&'a str, &'a str)>) -> SourceNames {
    let mut names = SourceNames::new();
    let mut twice = BTreeSet::new();
    for (rel_path, node_name) in files {
        if names.insert(rel_path.into(), node_name.into()).is_some() {
            twice.insert(rel_path);
        }
    }
    names.retain(|path, _| !twice.contains(path.as_str()));
    names
}

/// The graph node name of a scanned file: its declared module path plus
/// the file name when the file declares a module, else its stripped
/// path (Kap. 6.1: logical hierarchies hang under their declaration,
/// physical ones under their directory).
pub(crate) fn file_node_name(stripped: &str, declarations: &[Declaration]) -> String {
    let file_name = stripped.rsplit('/').next().unwrap_or(stripped);
    match declarations
        .first()
        .map(|declaration| declaration.module_path.as_str())
        .filter(|module| !module.is_empty())
    {
        Some(module) => format!("{module}/{file_name}"),
        None => stripped.to_string(),
    }
}

/// The graph node name of one walked file, `None` without a registered
/// extractor. Taken from `names` when the build recorded it; otherwise the
/// file is extracted again from `content`, or from disk when that is `None`.
pub(crate) fn source_node_name(
    file: &SourceFile,
    stripped: &str,
    names: Option<&SourceNames>,
    content: Option<&str>,
) -> Result<Option<String>, ArchguardError> {
    let Some(extractor) = extractor_for(file.lang) else {
        return Ok(None);
    };
    if let Some(name) = names.and_then(|names| names.get(&file.rel_path)) {
        return Ok(Some(name.clone()));
    }
    let read;
    let content = match content {
        Some(content) => content,
        None => {
            read = String::from_utf8_lossy(&fs::read(&file.path)?).into_owned();
            &read
        }
    };
    let extraction = extract(extractor.as_ref(), stripped, content);
    Ok(Some(file_node_name(stripped, &extraction.declarations)))
}

/// One extractor pass; tests count the passes to prove the names are reused.
pub(super) fn extract(extractor: &dyn Extractor, stripped: &str, content: &str) -> Extraction {
    #[cfg(test)]
    extractions::bump();
    extractor.extract(stripped, content)
}

#[cfg(test)]
pub(crate) mod extractions {
    use std::cell::Cell;

    thread_local!(static PASSES: Cell<usize> = const { Cell::new(0) });

    pub(super) fn bump() {
        PASSES.with(|passes| passes.set(passes.get() + 1));
    }

    /// Extractor passes on this thread so far.
    pub(crate) fn count() -> usize {
        PASSES.with(Cell::get)
    }
}
