//! Saving: validate, refuse when the file changed on disk, then replace
//! it atomically (temporary file in the same directory, then rename).
//!
//! No `.bak` copy: `archguard.toml` is project content under version
//! control, the rename never leaves a half-written file, and an invalid
//! configuration is never written. (`desktop.toml` keeps a `.bak` only
//! of a file it could not read, which cannot happen here: an unreadable
//! file does not load into the editor.)

use std::cell::RefCell;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use super::document::ConfigEditor;
use super::error::SaveError;
use crate::documents::{self, DocId};

impl ConfigEditor {
    /// Write the edited text to [`ConfigEditor::path`], creating
    /// `.archguard/` when needed. Returns the written file.
    pub fn save(&mut self) -> Result<PathBuf, SaveError> {
        let text = self.text();
        self.validate().map_err(SaveError::Invalid)?;
        let on_disk = match fs::read_to_string(self.path()) {
            Ok(text) => Some(text),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(SaveError::Io(format!("{}: {e}", self.path().display()))),
        };
        if on_disk != self.disk {
            return Err(SaveError::ChangedOnDisk);
        }
        write_atomic(self.path(), &text)
            .map_err(|e| SaveError::Io(format!("{}: {e}", self.path().display())))?;
        self.disk = Some(text);
        Ok(self.path().to_path_buf())
    }
}

/// Replace `path` with `text` so readers see the old or the new file,
/// never a part. Keeps the permissions of an existing file.
pub fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::other("no parent directory"))?;
    fs::create_dir_all(dir)?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let result = fs::write(&tmp, text).and_then(|()| {
        if let Ok(metadata) = fs::metadata(path) {
            fs::set_permissions(&tmp, metadata.permissions())?;
        }
        fs::rename(&tmp, path)
    });
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Connect `editor` to document `doc`: every edit marks the document
/// dirty (or clean when the text is back to the file), and
/// `documents::save(doc)` saves through the editor. A Save button in the
/// view should call `documents::save(doc)` too, so the document is
/// marked clean in one place.
pub fn bind(doc: DocId, editor: &Rc<RefCell<ConfigEditor>>) {
    editor.borrow_mut().on_change(move |dirty, what| {
        if dirty {
            documents::mark_dirty(doc, what);
        } else {
            documents::mark_clean(doc);
        }
    });
    documents::set_save_hook(doc, save_hook(Rc::clone(editor)));
}

/// The hook `documents::set_save_hook` takes.
pub fn save_hook(editor: Rc<RefCell<ConfigEditor>>) -> impl FnMut() -> Result<(), String> {
    move || {
        editor
            .borrow_mut()
            .save()
            .map(drop)
            .map_err(|e| e.to_string())
    }
}
