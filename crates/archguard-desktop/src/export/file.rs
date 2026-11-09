//! Write an export to the chosen path in one step
//! ([`crate::config_editor::write_atomic`]), so a failed or interrupted
//! export never leaves half a file. Unlike the settings, an export never
//! creates the folder: the save dialog chose an existing one.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// Why `path` could not be written.
#[derive(Debug)]
pub struct WriteError {
    pub path: PathBuf,
    pub source: io::Error,
}

impl fmt::Display for WriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Could not write {}: {}",
            self.path.display(),
            self.source
        )
    }
}

impl std::error::Error for WriteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// Write `bytes` to `path` atomically, keeping an existing file's
/// permissions. The folder must exist.
pub fn write(path: &Path, bytes: &[u8]) -> Result<(), WriteError> {
    let fail = |source| WriteError {
        path: path.to_path_buf(),
        source,
    };
    path.file_name()
        .ok_or_else(|| fail(io::Error::new(io::ErrorKind::InvalidInput, "no file name")))?;
    let dir = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if !dir.is_dir() {
        let message = format!("the folder {} does not exist", dir.display());
        return Err(fail(io::Error::new(io::ErrorKind::NotFound, message)));
    }
    crate::config_editor::write_atomic(path, bytes).map_err(fail)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh, empty folder for one test.
    fn folder(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("archguard-export-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn write_replaces_the_file_and_leaves_no_temp_file() {
        let dir = folder("write");
        let path = dir.join("archguard-graph.md");
        write(&path, b"first").expect("write");
        write(&path, b"second").expect("overwrite");
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        assert_eq!(entries(&dir), ["archguard-graph.md"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn write_fails_cleanly_on_a_missing_folder() {
        let dir = folder("missing");
        let path = dir.join("absent").join("archguard-graph.md");
        let err = write(&path, b"x").unwrap_err();
        assert_eq!(err.source.kind(), io::ErrorKind::NotFound);
        assert!(err.to_string().starts_with("Could not write "), "{err}");
        assert!(err.to_string().contains("does not exist"), "{err}");
        assert!(entries(&dir).is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
