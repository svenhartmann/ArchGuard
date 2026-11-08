//! Windows: saves the installer so the app can start `msiexec` right before
//! it quits. The MSI's fixed UpgradeCode and MajorUpgrade make Windows
//! Installer replace the installed version.
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

/// Writes `msi` as `temp_dir/name` and returns that path.
pub(crate) fn save(temp_dir: &Path, name: &str, msi: &[u8]) -> io::Result<PathBuf> {
    let path = temp_dir.join(name);
    std::fs::write(&path, msi)?;
    Ok(path)
}

/// `msiexec /i <msi> /passive`: progress bar only, no questions.
pub(crate) fn command(msi: &Path) -> (OsString, Vec<OsString>) {
    (
        "msiexec".into(),
        vec!["/i".into(), msi.as_os_str().to_owned(), "/passive".into()],
    )
}
