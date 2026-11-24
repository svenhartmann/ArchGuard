//! Project set files (`.archguard-set.toml`), the one place that reads and writes them (ADR 0024).

use archguard_core::ArchguardError;
use std::path::{Path, PathBuf};

/// A named group of 2 to 16 project folders; the first is the primary project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSet {
    pub name: String,
    pub projects: Vec<PathBuf>,
}

/// Reads and validates a set file; relative entries resolve against its folder and are canonicalised.
pub fn read_project_set(_path: &Path) -> Result<ProjectSet, ArchguardError> {
    Err(ArchguardError::Config("not implemented".into()))
}

/// Validates `set` and writes it atomically (temp file + rename) in a stable format.
pub fn write_project_set(_path: &Path, _set: &ProjectSet) -> Result<(), ArchguardError> {
    Err(ArchguardError::Config("not implemented".into()))
}
