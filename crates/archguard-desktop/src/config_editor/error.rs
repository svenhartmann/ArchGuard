//! Errors of the configuration editor.

use std::fmt;

use super::catalogue::{self, Field};

/// A problem with one field: the concrete path the engine or the parser
/// names (`layers[0].name`, `rules.foo`; empty when no field is known)
/// and a readable message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldError {
    pub path: String,
    pub message: String,
}

impl FieldError {
    pub fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }

    /// The catalogue field the form shows this error at.
    pub fn field(&self) -> Option<&'static Field> {
        catalogue::resolve(&self.path)
    }
}

impl fmt::Display for FieldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.path.is_empty() {
            f.write_str(&self.message)
        } else {
            write!(f, "{}: {}", self.path, self.message)
        }
    }
}

/// Why [`super::ConfigEditor::save`] did not write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveError {
    /// The edited configuration does not pass the engine's checks.
    Invalid(FieldError),
    /// The file changed on disk since it was loaded or saved here.
    ChangedOnDisk,
    Io(String),
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SaveError::Invalid(e) => write!(f, "not saved, invalid configuration: {e}"),
            SaveError::ChangedOnDisk => {
                f.write_str("not saved: the file changed on disk; reload it first")
            }
            SaveError::Io(e) => write!(f, "not saved: {e}"),
        }
    }
}
