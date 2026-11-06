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

/// A copy of the tracked files of `fixtures/<name>` (empty name: an empty
/// dir). Analysis outputs other tests leave in the fixture (ignored by git,
/// e.g. `.archguard/cache.json`) are not copied.
pub fn fixture(name: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    if name.is_empty() {
        return dir;
    }
    let src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(name);
    let listed = std::process::Command::new("git")
        .arg("-C")
        .arg(&src)
        .args(["ls-files", "-z", "."])
        .output()
        .unwrap();
    assert!(listed.status.success(), "git ls-files in {}", src.display());
    for file in listed.stdout.split(|&b| b == 0).filter(|f| !f.is_empty()) {
        let file = Path::new(std::str::from_utf8(file).unwrap());
        let to = dir.path().join(file);
        fs::create_dir_all(to.parent().unwrap()).unwrap();
        fs::copy(src.join(file), &to).unwrap();
    }
    dir
}

/// `text` as `.archguard/archguard.toml` of `root`, loaded.
pub fn editor_with(root: &Path, text: &str) -> ConfigEditor {
    fs::create_dir_all(root.join(".archguard")).unwrap();
    fs::write(root.join(".archguard/archguard.toml"), text).unwrap();
    ConfigEditor::load(root).unwrap()
}
