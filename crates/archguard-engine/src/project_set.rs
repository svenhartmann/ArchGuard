//! Project set files (`.archguard-set.toml`), the one place that reads and writes them (ADR 0024).

use archguard_core::ArchguardError;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const MAX_BYTES: u64 = 64 * 1024;
const MAX_NAME_CHARS: usize = 200;
const PROJECTS: std::ops::RangeInclusive<usize> = 2..=16;

/// A named group of 2 to 16 project folders; the first is the primary project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSet {
    pub name: String,
    pub projects: Vec<PathBuf>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SetFile {
    name: String,
    projects: Vec<String>,
}

fn fail(file: &Path, reason: impl std::fmt::Display) -> ArchguardError {
    ArchguardError::Config(format!("{}: {reason}", file.display()))
}

fn check_shape(file: &Path, name: &str, count: usize) -> Result<(), ArchguardError> {
    let chars = name.chars().count();
    if name.trim().is_empty() || chars > MAX_NAME_CHARS || name.chars().any(char::is_control) {
        return Err(fail(
            file,
            format!("name must be 1 to {MAX_NAME_CHARS} characters without control characters"),
        ));
    }
    if !PROJECTS.contains(&count) {
        return Err(fail(
            file,
            format!("a set needs 2 to 16 projects, found {count}"),
        ));
    }
    Ok(())
}

/// Existing directory, canonical (symlinks followed: the user picked the entry), no duplicates.
fn resolve(file: &Path, base: &Path, entries: &[PathBuf]) -> Result<Vec<PathBuf>, ArchguardError> {
    let mut out: Vec<PathBuf> = Vec::with_capacity(entries.len());
    for (i, entry) in entries.iter().enumerate() {
        let n = i + 1;
        let dir = base
            .join(entry)
            .canonicalize()
            .map_err(|_| fail(file, format!("project {n} does not exist")))?;
        if !dir.is_dir() {
            return Err(fail(file, format!("project {n} is not a folder")));
        }
        if out.contains(&dir) {
            return Err(fail(file, format!("project {n} is a duplicate")));
        }
        out.push(dir);
    }
    Ok(out)
}

/// Reads and validates a set file; relative entries resolve against its folder and are canonicalised.
pub fn read_project_set(path: &Path) -> Result<ProjectSet, ArchguardError> {
    let mut text = String::new();
    let opened = std::fs::File::open(path).map_err(|e| fail(path, e.kind()))?;
    opened
        .take(MAX_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|e| fail(path, e.kind()))?;
    if text.len() as u64 > MAX_BYTES {
        return Err(fail(path, "file is larger than 64 KiB"));
    }
    // The toml error text quotes the source, so only its position is reported.
    let parsed: SetFile = toml::from_str(&text).map_err(|e| {
        let at = e.span().map_or(0, |s| s.start);
        fail(
            path,
            format!("not a valid set file (syntax, unknown or missing key) near byte {at}"),
        )
    })?;
    check_shape(path, &parsed.name, parsed.projects.len())?;
    let entries: Vec<PathBuf> = parsed.projects.into_iter().map(PathBuf::from).collect();
    let base = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok(ProjectSet {
        name: parsed.name,
        projects: resolve(path, base, &entries)?,
    })
}

/// Validates `set` and writes it atomically (temp file + rename) in a stable format.
pub fn write_project_set(path: &Path, set: &ProjectSet) -> Result<(), ArchguardError> {
    check_shape(path, &set.name, set.projects.len())?;
    let base = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    resolve(path, base, &set.projects)?;
    let projects = set
        .projects
        .iter()
        .map(|p| {
            p.to_str()
                .map(str::to_owned)
                .ok_or_else(|| fail(path, "project path is not UTF-8"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let text = toml::to_string(&SetFile {
        name: set.name.clone(),
        projects,
    })
    .map_err(|e| fail(path, e))?;
    let mut temp = tempfile::NamedTempFile::new_in(base).map_err(|e| fail(path, e.kind()))?;
    temp.write_all(text.as_bytes())
        .map_err(|e| fail(path, e.kind()))?;
    temp.persist(path).map_err(|e| fail(path, e.error.kind()))?;
    Ok(())
}
