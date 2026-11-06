//! The project's `archguard.toml` as an editable `toml_edit` document.
//! Edits touch only the addressed key or entry, so comments, formatting
//! and keys the editor does not know stay as written.

use std::fs;
use std::path::{Path, PathBuf};

use archguard_engine::DEFAULT_CONFIG_PATH;
use toml_edit::{DocumentMut, Item, Table};

use super::catalogue::{self, Field, Kind};
use super::error::FieldError;
use super::toml_path::{self, Seg};
use super::value::FieldValue;

/// The engine's legacy location, read when the canonical one is absent.
const LEGACY_PATH: &str = "archguard.toml";

type Listener = Box<dyn FnMut(bool, &str)>;

/// One project's configuration file being edited.
pub struct ConfigEditor {
    root: PathBuf,
    /// The file that is read and written.
    path: PathBuf,
    /// The text on disk as loaded or last saved; `None`: no file yet.
    pub(super) disk: Option<String>,
    doc: DocumentMut,
    listener: Option<Listener>,
}

impl ConfigEditor {
    /// Load the configuration of `root` from the file the engine reads:
    /// `.archguard/archguard.toml`, else the legacy `archguard.toml`.
    /// Without a file every field is at its default and saving creates
    /// `.archguard/archguard.toml`. A file that is no valid TOML is an
    /// error (it is project content and is never replaced unseen); a
    /// file that is TOML but not a valid configuration loads, and
    /// [`ConfigEditor::validate`] names the problem.
    pub fn load(root: &Path) -> Result<Self, String> {
        let (path, disk) = locate(root)?;
        let doc = match &disk {
            Some(text) => text
                .parse::<DocumentMut>()
                .map_err(|e| format!("{}: {e}", path.display()))?,
            None => DocumentMut::new(),
        };
        Ok(Self {
            root: root.to_path_buf(),
            path,
            disk,
            doc,
            listener: None,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The file saving writes.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The edited TOML text, for the text pane and for saving.
    pub fn text(&self) -> String {
        self.doc.to_string()
    }

    pub(super) fn table(&self) -> &Table {
        self.doc.as_table()
    }

    /// Whether the edited text differs from the file.
    pub fn is_dirty(&self) -> bool {
        self.text() != self.disk.as_deref().unwrap_or("")
    }

    /// Called after every edit with the dirty state and the form group
    /// that changed (see [`super::bind`]).
    pub fn on_change(&mut self, f: impl FnMut(bool, &str) + 'static) {
        self.listener = Some(Box::new(f));
    }

    fn changed(&mut self, what: &str) {
        let dirty = self.is_dirty();
        if let Some(f) = &mut self.listener {
            f(dirty, what);
        }
    }

    /// Replace the whole text (the text pane); it must be valid TOML.
    pub fn set_text(&mut self, text: &str) -> Result<(), FieldError> {
        self.doc = text
            .parse::<DocumentMut>()
            .map_err(|e| FieldError::new("", e.to_string()))?;
        self.changed("Configuration text");
        Ok(())
    }

    /// Write `value` at the concrete `path` (`rules.max_acd`,
    /// `layers[1].name`), keeping the comments around an existing value.
    pub fn set(&mut self, path: &str, value: FieldValue) -> Result<(), FieldError> {
        let field = value_field(path)?;
        if !value.fits(field.kind) {
            return Err(FieldError::new(path, mismatch(field.kind)));
        }
        let segs = segs(path)?;
        let key = toml_path::leaf(&segs).map_err(|e| FieldError::new(path, e))?;
        let parent = toml_path::parent_mut(self.doc.as_table_mut(), &segs)
            .map_err(|e| FieldError::new(path, e))?;
        let mut new = value.to_value();
        if let Some(old) = parent.get(key).and_then(Item::as_value) {
            *new.decor_mut() = old.decor().clone();
        }
        parent.insert(key, Item::Value(new));
        self.changed(field.group.label());
        Ok(())
    }

    /// Remove `path` from the file, so the default (or derivation) applies.
    pub fn unset(&mut self, path: &str) -> Result<(), FieldError> {
        let field = value_field(path)?;
        let segs = segs(path)?;
        if toml_path::get(self.doc.as_table(), &segs).is_none() {
            return Ok(());
        }
        let key = toml_path::leaf(&segs).map_err(|e| FieldError::new(path, e))?;
        toml_path::parent_mut(self.doc.as_table_mut(), &segs)
            .map_err(|e| FieldError::new(path, e))?
            .remove(key);
        self.changed(field.group.label());
        Ok(())
    }

    /// Append an empty `[[array]]` entry; returns its index.
    pub fn add_entry(&mut self, array: &str) -> Result<usize, FieldError> {
        let group = array_field(array)?.group;
        let entries = toml_path::array_mut(self.doc.as_table_mut(), &segs(array)?)
            .map_err(|e| FieldError::new(array, e))?;
        entries.push(Table::new());
        let index = entries.len() - 1;
        self.changed(group.label());
        Ok(index)
    }

    /// Remove entry `index` of `array` with its comments.
    pub fn remove_entry(&mut self, array: &str, index: usize) -> Result<(), FieldError> {
        let group = array_field(array)?.group;
        let entries = toml_path::array_mut(self.doc.as_table_mut(), &segs(array)?)
            .map_err(|e| FieldError::new(array, e))?;
        if index >= entries.len() {
            return Err(FieldError::new(array, format!("no entry {index}")));
        }
        entries.remove(index);
        self.changed(group.label());
        Ok(())
    }

    /// Move entry `from` of `array` to position `to`; entries keep their
    /// comments, the file keeps its table order around them.
    pub fn move_entry(&mut self, array: &str, from: usize, to: usize) -> Result<(), FieldError> {
        let group = array_field(array)?.group;
        let entries = toml_path::array_mut(self.doc.as_table_mut(), &segs(array)?)
            .map_err(|e| FieldError::new(array, e))?;
        let len = entries.len();
        if from >= len || to >= len {
            return Err(FieldError::new(array, format!("no entry {}", from.max(to))));
        }
        let mut tables: Vec<Table> = (0..len).map(|_| entries.remove(0)).collect();
        let positions: Vec<_> = tables.iter().map(Table::position).collect();
        let moved = tables.remove(from);
        tables.insert(to, moved);
        for (mut table, position) in tables.into_iter().zip(positions) {
            table.set_position(position);
            entries.push(table);
        }
        self.changed(group.label());
        Ok(())
    }
}

/// The file the engine would read, and its text if it exists.
fn locate(root: &Path) -> Result<(PathBuf, Option<String>), String> {
    let canonical = root.join(DEFAULT_CONFIG_PATH);
    for path in [canonical.clone(), root.join(LEGACY_PATH)] {
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let linked_dir = path == canonical
            && path.parent().is_some_and(|dir| {
                fs::symlink_metadata(dir).is_ok_and(|m| m.file_type().is_symlink())
            });
        if metadata.file_type().is_symlink() || linked_dir {
            return Err(format!(
                "{}: a discovered configuration must not be a symlink",
                path.display()
            ));
        }
        let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        return Ok((path, Some(text)));
    }
    Ok((canonical, None))
}

fn segs(path: &str) -> Result<Vec<Seg>, FieldError> {
    toml_path::parse(path).map_err(|e| FieldError::new(path, e))
}

fn value_field(path: &str) -> Result<&'static Field, FieldError> {
    catalogue::field(path)
        .filter(|f| f.kind.is_value())
        .ok_or_else(|| FieldError::new(path, "not a configuration value"))
}

fn array_field(path: &str) -> Result<&'static Field, FieldError> {
    catalogue::field(path)
        .filter(|f| f.kind == Kind::TableArray)
        .ok_or_else(|| FieldError::new(path, "not a table array"))
}

fn mismatch(kind: Kind) -> String {
    match kind {
        Kind::Enum(allowed) => format!("expected one of {}", allowed.join(", ")),
        Kind::Bool => "expected true or false".into(),
        Kind::Int => "expected a whole number".into(),
        Kind::Float => "expected a number".into(),
        Kind::Text => "expected text".into(),
        Kind::List => "expected a list of strings".into(),
        Kind::Map => "expected name = number pairs".into(),
        Kind::Table | Kind::TableArray => "a section has no value".into(),
    }
}
