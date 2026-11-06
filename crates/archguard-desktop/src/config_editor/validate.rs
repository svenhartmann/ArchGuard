//! Validation before save and preview: the text must deserialize into
//! `Config` (unknown keys rejected) and pass the engine's own checks in
//! `load_config`. Errors name the field path.

use std::io::Write as _;
use std::path::Path;

use archguard_engine::core::Config;
use archguard_engine::{LoadedConfig, load_config};
use toml_edit::{Document, Item, TableLike};

use super::document::ConfigEditor;
use super::error::FieldError;

impl ConfigEditor {
    /// The edited configuration as the engine loads it, or the first problem.
    pub fn validate(&self) -> Result<LoadedConfig, FieldError> {
        validate(self.root(), &self.text())
    }
}

/// Validate `text` as the configuration of the project at `root`.
pub fn validate(root: &Path, text: &str) -> Result<LoadedConfig, FieldError> {
    if let Err(e) = toml::from_str::<Config>(text) {
        let path = e
            .span()
            .and_then(|s| path_at(text, s.start))
            .unwrap_or_default();
        return Err(FieldError::new(path, e.message().trim()));
    }
    // The engine validates only files: check a temporary copy.
    let mut file =
        tempfile::NamedTempFile::new().map_err(|e| FieldError::new("", e.to_string()))?;
    file.write_all(text.as_bytes())
        .map_err(|e| FieldError::new("", e.to_string()))?;
    load_config(Some(file.path()), root).map_err(|e| engine_error(&e.to_string(), file.path()))
}

/// `config error: <file>: config error: layers[0].name: must be …` →
/// path `layers[0].name` and the message.
fn engine_error(message: &str, file: &Path) -> FieldError {
    let prefix = "config error: ";
    let file = format!("{}: ", file.display());
    let rest = message.strip_prefix(prefix).unwrap_or(message);
    let rest = rest.strip_prefix(&file).unwrap_or(rest);
    let rest = rest.strip_prefix(prefix).unwrap_or(rest);
    match rest.split_once(": ") {
        Some((path, text)) if is_path(path) => FieldError::new(path, text),
        _ => FieldError::new("", rest),
    }
}

fn is_path(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.[]-".contains(c))
}

/// The concrete path of the key or value at byte `offset` of `text`.
pub fn path_at(text: &str, offset: usize) -> Option<String> {
    let doc = Document::parse(text).ok()?;
    find(doc.as_table(), offset, "")
}

fn find(table: &dyn TableLike, offset: usize, prefix: &str) -> Option<String> {
    for (key, item) in table.iter() {
        let path = if prefix.is_empty() {
            key.to_string()
        } else {
            format!("{prefix}.{key}")
        };
        let hit = |span: Option<std::ops::Range<usize>>| span.is_some_and(|s| s.contains(&offset));
        if let Some(found) = find_inside(item, offset, &path) {
            return Some(found);
        }
        if hit(table.key(key).and_then(|k| k.span())) || hit(item.span()) {
            return Some(path);
        }
    }
    None
}

/// A deeper match inside the tables of `item`.
fn find_inside(item: &Item, offset: usize, path: &str) -> Option<String> {
    if let Some(entries) = item.as_array_of_tables() {
        return entries.iter().enumerate().find_map(|(i, t)| {
            let entry = format!("{path}[{i}]");
            find(t, offset, &entry).or_else(|| {
                t.span()
                    .is_some_and(|s| s.contains(&offset))
                    .then_some(entry)
            })
        });
    }
    find(item.as_table_like()?, offset, path)
}
