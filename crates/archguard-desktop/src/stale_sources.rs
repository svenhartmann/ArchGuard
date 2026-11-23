//! There is no file watcher: a tab calls this when it is activated.

use std::path::Path;
use std::time::SystemTime;

use crate::documents;

/// Drop the active document's derived state (the shared flow included) when a source file
/// changed since it was built.
pub fn forget_stale_sources() {
    documents::with_store(|s| {
        let Some(d) = s.active() else { return };
        if newer_than(&d.root, d.built, 0) {
            let id = d.id;
            if let Some(d) = s.get_mut(id) {
                d.forget_derived();
            }
        }
    });
}

/// Whether a file under `dir` (not hidden, not a build/dependency directory) is newer than `since`.
/// shortcut: bounded depth and no .gitignore, a deeper or ignored change shows after a reload.
fn newer_than(dir: &Path, since: SystemTime, depth: usize) -> bool {
    const SKIP: [&str; 6] = ["target", "node_modules", "build", "dist", "bin", "obj"];
    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    entries.flatten().any(|e| {
        let name = e.file_name();
        let name = name.to_string_lossy();
        let Ok(meta) = std::fs::symlink_metadata(e.path()) else {
            return false;
        };
        if meta.is_dir() {
            depth < 12
                && !name.starts_with('.')
                && !SKIP.contains(&&*name)
                && newer_than(&e.path(), since, depth + 1)
        } else {
            meta.is_file() && meta.modified().is_ok_and(|m| m > since)
        }
    })
}
