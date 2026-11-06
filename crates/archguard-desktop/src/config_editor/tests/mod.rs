//! Tests of the configuration editor. Fixtures are copied into a temp
//! dir; the originals are never edited.

mod catalogue;
mod edit;
mod preview;
mod provenance;
mod save;
mod validate;

use std::fs;
use std::path::Path;

use super::ConfigEditor;

/// A copy of `fixtures/<name>` (empty name: an empty dir).
pub fn fixture(name: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    if !name.is_empty() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(name);
        copy_dir(&src, dir.path());
    }
    dir
}

fn copy_dir(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap().flatten() {
        let to = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &to);
        } else {
            fs::copy(entry.path(), &to).unwrap();
        }
    }
}

/// `text` as `.archguard/archguard.toml` of `root`, loaded.
pub fn editor_with(root: &Path, text: &str) -> ConfigEditor {
    fs::create_dir_all(root.join(".archguard")).unwrap();
    fs::write(root.join(".archguard/archguard.toml"), text).unwrap();
    ConfigEditor::load(root).unwrap()
}
