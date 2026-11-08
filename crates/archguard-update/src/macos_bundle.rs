//! macOS: replaces the installed `.app` bundle with the `ArchGuard.app` from
//! a verified `.dmg`. The image is mounted read-only with `hdiutil`, the app is
//! copied beside the installed bundle and swapped in by renames; the old
//! bundle is deleted only after the swap succeeded.
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

const APP: &str = "ArchGuard.app";

/// Runs an external program; injectable so tests never call `hdiutil`.
pub trait Runner {
    /// Runs `program` with `args` and fails unless it exits successfully.
    fn run(&self, program: &str, args: &[OsString]) -> Result<(), String>;
}

/// Runs programs for real with [`std::process::Command`].
pub struct SystemRunner;

impl Runner for SystemRunner {
    fn run(&self, program: &str, args: &[OsString]) -> Result<(), String> {
        let output = Command::new(program)
            .args(args)
            .output()
            .map_err(|e| format!("cannot run {program}: {e}"))?;
        if output.status.success() {
            return Ok(());
        }
        Err(format!(
            "{program} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// The bundle `…/X.app` of an executable at `…/X.app/Contents/MacOS/<bin>`.
pub(crate) fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let is_bundle = macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app";
    is_bundle.then(|| bundle.to_path_buf())
}

/// Installs the app from `dmg` over `bundle`. `Ok(false)` means the folder
/// holding the bundle is not writable, so nothing was changed.
pub(crate) fn install(bundle: &Path, dmg: &[u8], runner: &dyn Runner) -> Result<bool, String> {
    let parent = bundle
        .parent()
        .ok_or("the app bundle has no parent folder")?;
    let work = parent.join(format!(".ArchGuard-update-{}", std::process::id()));
    match std::fs::create_dir(&work) {
        Err(e) if is_read_only(&e) => return Ok(false),
        other => other.map_err(|e| format!("cannot create {}: {e}", work.display()))?,
    }
    let result = stage(&work, dmg, runner).and_then(|new| swap(bundle, &new));
    let _ = std::fs::remove_dir_all(&work);
    result.map(|()| true)
}

fn is_read_only(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem
    )
}

/// Mounts the image, copies its app into `work` and detaches it again.
fn stage(work: &Path, dmg: &[u8], runner: &dyn Runner) -> Result<PathBuf, String> {
    let image = work.join("ArchGuard.dmg");
    let mount = work.join("mnt");
    let new = work.join(APP);
    std::fs::write(&image, dmg).map_err(|e| format!("cannot write {}: {e}", image.display()))?;
    std::fs::create_dir(&mount).map_err(|e| format!("cannot create {}: {e}", mount.display()))?;
    let attach = ["attach", "-nobrowse", "-readonly", "-mountpoint"].map(OsString::from);
    let mut args = attach.to_vec();
    args.extend([mount.clone().into_os_string(), image.into_os_string()]);
    runner.run("hdiutil", &args)?;
    let copied = copy_tree(&mount.join(APP), &new)
        .map_err(|e| format!("cannot copy {APP} from the disk image: {e}"));
    let detached = runner.run("hdiutil", &["detach".into(), mount.into_os_string()]);
    copied.and(detached).map(|()| new)
}

/// Renames `bundle` aside, moves `new` into its place and deletes the old
/// bundle; restores the old bundle if the second rename fails.
fn swap(bundle: &Path, new: &Path) -> Result<(), String> {
    let name = bundle.file_name().unwrap_or_default().to_string_lossy();
    let old = bundle.with_file_name(format!(".{name}.old-{}", std::process::id()));
    std::fs::rename(bundle, &old)
        .map_err(|e| format!("cannot move {} aside: {e}", bundle.display()))?;
    if let Err(e) = std::fs::rename(new, bundle) {
        let _ = std::fs::rename(&old, bundle);
        return Err(format!("cannot install {}: {e}", bundle.display()));
    }
    let _ = std::fs::remove_dir_all(&old);
    Ok(())
}

/// Copies a directory tree, keeping symlinks as links and file permissions.
fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    let kind = std::fs::symlink_metadata(from)?.file_type();
    if kind.is_symlink() {
        return link(&std::fs::read_link(from)?, to);
    }
    if !kind.is_dir() {
        return std::fs::copy(from, to).map(|_| ());
    }
    std::fs::create_dir(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        copy_tree(&entry.path(), &to.join(entry.file_name()))?;
    }
    Ok(())
}

#[cfg(unix)]
fn link(target: &Path, at: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, at)
}

#[cfg(not(unix))]
fn link(_target: &Path, at: &Path) -> io::Result<()> {
    Err(io::Error::other(format!(
        "cannot recreate the symlink {}",
        at.display()
    )))
}

#[cfg(test)]
mod tests {
    use super::bundle_of;
    use std::path::Path;

    #[test]
    fn only_an_executable_inside_contents_macos_of_an_app_has_a_bundle() {
        let exe = Path::new("/Applications/My.app/Contents/MacOS/archguard-desktop");
        assert_eq!(
            bundle_of(exe).as_deref(),
            Some(Path::new("/Applications/My.app"))
        );
        assert_eq!(
            bundle_of(Path::new("/usr/local/bin/archguard-desktop")),
            None
        );
        assert_eq!(bundle_of(Path::new("/x/My/Contents/MacOS/bin")), None);
    }
}
