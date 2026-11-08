//! Linux: replaces the running AppImage file (the `APPIMAGE` path) in place.
use std::io;
use std::path::{Path, PathBuf};

/// Writes `image` beside `installed`, marks it executable and renames it over
/// `installed`. On any error the staged file is removed and `installed` stays
/// as it was.
pub(crate) fn replace(installed: &Path, image: &[u8]) -> io::Result<()> {
    let staged = staged_path(installed);
    let result =
        write_executable(&staged, image).and_then(|()| std::fs::rename(&staged, installed));
    if result.is_err() {
        let _ = std::fs::remove_file(&staged);
    }
    result
}

fn staged_path(installed: &Path) -> PathBuf {
    let name = installed.file_name().unwrap_or_default().to_string_lossy();
    installed.with_file_name(format!(".{name}.new"))
}

fn write_executable(path: &Path, data: &[u8]) -> io::Result<()> {
    std::fs::write(path, data)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}
