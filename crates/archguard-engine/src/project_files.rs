//! Project-local files below `<root>/.archguard`, written through descriptor-relative
//! storage on Unix that the graph cache shares.
use std::io;
use std::path::{Component, Path};

/// The project-local ArchGuard directory below the analysed root.
pub(crate) const PROJECT_DIR: &str = ".archguard";

/// Descriptor-relative, no-follow primitives for `<root>/.archguard` on Unix,
/// shared by project files and the graph cache.
#[cfg(unix)]
pub(crate) mod safe {
    use std::fs::File;
    use std::io::Write;
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};

    use rustix::fd::{AsFd, OwnedFd};
    use rustix::fs::{AtFlags, Mode, OFlags, fsync, linkat, mkdirat, openat, renameat, unlinkat};
    use rustix::io::Errno;

    use super::PROJECT_DIR;

    /// Disambiguates temporary names from the same process.
    static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

    /// `openat` flags for a directory we traverse *into*; `follow` is
    /// only ever true for the caller-chosen root itself — everything
    /// below it is repository state and must not be a symlink.
    pub(crate) fn dir_flags(follow: bool) -> OFlags {
        let mut flags = OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY;
        if !follow {
            flags |= OFlags::NOFOLLOW;
        }
        flags
    }

    /// Atomically commit bytes in an already opened directory. Create-only
    /// publication uses a hard link so an existing entry can never be replaced.
    pub(crate) fn write_atomic(
        dir_fd: &impl AsFd,
        name: &Path,
        bytes: &[u8],
        overwrite: bool,
    ) -> std::io::Result<()> {
        let tmp = format!(
            ".archguard-{}-{}.tmp",
            std::process::id(),
            TMP_SEQ.fetch_add(1, Ordering::Relaxed)
        );
        // O_EXCL | O_NOFOLLOW: the fresh name can never be a pre-planted
        // symlink, and renameat below replaces the *target entry* — a
        // symlinked target is swapped for the new regular file
        // instead of being written through.
        let fd = openat(
            dir_fd,
            &tmp,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::from_bits_retain(0o600),
        )?;
        let result = (|| {
            let mut file = File::from(fd);
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);
            if overwrite {
                // POSIX rename replaces the target entry itself; a symlinked
                // cache.json is swapped, never written through one.
                renameat(dir_fd, &tmp, dir_fd, name)?;
            } else {
                linkat(dir_fd, &tmp, dir_fd, name, AtFlags::empty())?;
            }
            Ok::<_, std::io::Error>(())
        })();
        // Rename consumed the name on success; create-only publication and
        // failures both need to remove it explicitly.
        let _ = unlinkat(dir_fd, &tmp, AtFlags::empty());
        result?;
        // Durability of the rename itself; the visible state is already
        // consistent without it (rename is atomic for readers).
        let _ = fsync(dir_fd);
        Ok(())
    }

    pub(crate) fn open_subdirectory(parent: &impl AsFd, name: &Path) -> std::io::Result<OwnedFd> {
        match mkdirat(parent, name, Mode::from_bits_retain(0o755)) {
            Ok(()) | Err(Errno::EXIST) => {}
            Err(error) => return Err(error.into()),
        }
        Ok(openat(parent, name, dir_flags(false), Mode::empty())?)
    }

    pub(crate) fn open_project_directory(dir: &Path) -> std::io::Result<OwnedFd> {
        // The root is chosen by the caller (CLI/MCP), so following
        // symlinks on the way there is fine — e.g. /tmp-style symlinks
        // on macOS. Only components *below* the root are untrusted.
        let root = openat(rustix::fs::CWD, dir, dir_flags(true), Mode::empty())?;
        // ELOOP on a symlink, ENOTDIR on a file: a repository-controlled
        // redirect is refused here, before anything is opened or created.
        open_subdirectory(&root, Path::new(PROJECT_DIR))
    }
}

/// Store bytes below `<root>/.archguard`, creating ordinary parent directories.
///
/// `relative` must contain only normal relative path components. With
/// `overwrite`, identical content retains its mtime and returns `false`;
/// otherwise an existing entry returns `AlreadyExists`. Changed content is
/// published atomically. Unix access uses held directory descriptors and never
/// follows repository-controlled symlinks, including during directory swaps.
/// Other platforms reject existing symlinks and use an atomic temporary file.
pub fn write_project_file(
    root: &Path,
    relative: &Path,
    bytes: &[u8],
    overwrite: bool,
) -> io::Result<bool> {
    write_project_file_with(root, relative, bytes, overwrite, || {})
}

fn validate_relative(relative: &Path) -> io::Result<()> {
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "project file path must stay inside .archguard",
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn write_project_file_with(
    root: &Path,
    relative: &Path,
    bytes: &[u8],
    overwrite: bool,
    before_write: impl FnOnce(),
) -> io::Result<bool> {
    use rustix::fs::{Mode, OFlags, openat};
    use rustix::io::Errno;
    use std::fs::File;
    use std::io::Read;

    validate_relative(relative)?;
    let mut parent = safe::open_project_directory(root)?;
    for component in relative
        .parent()
        .expect("relative path has parent")
        .components()
    {
        parent = safe::open_subdirectory(&parent, Path::new(component.as_os_str()))?;
    }
    let name = Path::new(relative.file_name().expect("validated normal path"));
    // Boundary tests can swap a directory after traversal. Production passes
    // an empty closure; neither path nor callback is part of the public API.
    before_write();
    if overwrite {
        match openat(
            &parent,
            name,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
            Mode::empty(),
        ) {
            Ok(fd) => {
                let file = File::from(fd);
                let metadata = file.metadata()?;
                if !metadata.is_file() {
                    return Err(io::Error::other("project file is not a regular file"));
                }
                if metadata.len() == bytes.len() as u64 {
                    let mut previous = Vec::with_capacity(bytes.len());
                    file.take(bytes.len() as u64 + 1)
                        .read_to_end(&mut previous)?;
                    if previous == bytes {
                        return Ok(false);
                    }
                }
            }
            Err(Errno::NOENT) => {}
            Err(error) => return Err(error.into()),
        }
    }
    safe::write_atomic(&parent, name, bytes, overwrite)?;
    Ok(true)
}

#[cfg(not(unix))]
fn write_project_file_with(
    root: &Path,
    relative: &Path,
    bytes: &[u8],
    overwrite: bool,
    before_write: impl FnOnce(),
) -> io::Result<bool> {
    use std::fs;
    use std::io::Write;

    validate_relative(relative)?;
    let mut parent = root.to_path_buf();
    for component in Path::new(".archguard").components().chain(
        relative
            .parent()
            .expect("relative path has parent")
            .components(),
    ) {
        parent.push(component);
        match fs::create_dir(&parent) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        let metadata = fs::symlink_metadata(&parent)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(io::Error::other("project parent is not a real directory"));
        }
    }
    before_write();
    let target = parent.join(relative.file_name().expect("validated normal path"));
    match fs::symlink_metadata(&target) {
        Ok(_) if !overwrite => return Err(io::ErrorKind::AlreadyExists.into()),
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(io::Error::other("project file is not a regular file"));
            }
            if fs::read(&target)? == bytes {
                return Ok(false);
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    let mut temporary = tempfile::NamedTempFile::new_in(&parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    if overwrite {
        temporary.persist(target)
    } else {
        temporary.persist_noclobber(target)
    }
    .map_err(|error| error.error)?;
    Ok(true)
}

/// Read repository state without following any component below the chosen root.
pub(crate) fn read_repo_file(root: &Path, relative: &Path, limit: u64) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    if relative
        .components()
        .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err(std::io::ErrorKind::InvalidInput.into());
    }
    #[cfg(unix)]
    let file = {
        use rustix::fs::{Mode, OFlags, openat};
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
        let mut parent = openat(rustix::fs::CWD, root, flags, Mode::empty())?;
        for component in relative.parent().unwrap_or(Path::new("")).components() {
            parent = openat(
                &parent,
                Path::new(component.as_os_str()),
                flags | OFlags::NOFOLLOW,
                Mode::empty(),
            )?;
        }
        std::fs::File::from(openat(
            &parent,
            Path::new(
                relative
                    .file_name()
                    .ok_or(std::io::ErrorKind::InvalidInput)?,
            ),
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
            Mode::empty(),
        )?)
    };
    #[cfg(not(unix))]
    let file = {
        let mut path = root.to_path_buf();
        for component in relative.components() {
            path.push(component);
            if std::fs::symlink_metadata(&path)?.file_type().is_symlink() {
                return Err(std::io::Error::other(
                    "symlink is outside the security scan scope",
                ));
            }
        }
        std::fs::File::open(path)?
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(std::io::Error::other(
            "dependency input is not a regular bounded file",
        ));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(std::io::Error::other(
            "dependency input exceeded its size limit",
        ));
    }
    Ok(bytes)
}

/// Upper bound of a project manifest read by [`read_manifest`].
const MANIFEST_MAX_BYTES: u64 = 1024 * 1024;

/// Read a UTF-8 project manifest without following any symlink below `root`.
pub(crate) fn read_manifest(root: &Path, relative: &Path) -> std::io::Result<String> {
    #[cfg(unix)]
    let file = {
        use rustix::fs::{Mode, OFlags, openat};
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW;
        let mut parent = openat(rustix::fs::CWD, root, flags, Mode::empty())?;
        for part in relative.parent().unwrap_or(Path::new("")).components() {
            parent = openat(&parent, Path::new(part.as_os_str()), flags, Mode::empty())?;
        }
        std::fs::File::from(openat(
            &parent,
            Path::new(
                relative
                    .file_name()
                    .ok_or(std::io::ErrorKind::InvalidInput)?,
            ),
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
            Mode::empty(),
        )?)
    };
    #[cfg(not(unix))]
    let file = {
        let mut path = root.to_path_buf();
        for component in relative.components() {
            path.push(component);
            if std::fs::symlink_metadata(&path)?.file_type().is_symlink() {
                return Err(std::io::ErrorKind::PermissionDenied.into());
            }
        }
        std::fs::File::open(path)?
    };
    if !file.metadata()?.is_file() || file.metadata()?.len() > MANIFEST_MAX_BYTES {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    use std::io::Read;
    let mut source = String::new();
    file.take(MANIFEST_MAX_BYTES + 1)
        .read_to_string(&mut source)?;
    if source.len() as u64 > MANIFEST_MAX_BYTES {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    Ok(source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{Duration, SystemTime};

    #[test]
    fn writes_stay_inside_project_and_identical_content_keeps_mtime() {
        let root = tempfile::tempdir().unwrap();
        let relative = Path::new("reports/graph.html");
        assert!(write_project_file(root.path(), relative, b"first", true).unwrap());
        let target = root.path().join(".archguard").join(relative);
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(100);
        fs::File::options()
            .write(true)
            .open(&target)
            .unwrap()
            .set_modified(old)
            .unwrap();
        assert!(!write_project_file(root.path(), relative, b"first", true).unwrap());
        assert_eq!(fs::metadata(&target).unwrap().modified().unwrap(), old);
        assert!(write_project_file(root.path(), relative, b"changed", true).unwrap());
        assert_eq!(fs::read(target).unwrap(), b"changed");
    }

    #[test]
    fn create_only_never_replaces_existing_policy_or_leaves_temporary_files() {
        let root = tempfile::tempdir().unwrap();
        let relative = Path::new("archguard.toml");
        assert!(write_project_file(root.path(), relative, b"policy", false).unwrap());
        for bytes in [b"policy".as_slice(), b"replacement"] {
            let error = write_project_file(root.path(), relative, bytes, false).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        }
        assert_eq!(
            fs::read(root.path().join(".archguard/archguard.toml")).unwrap(),
            b"policy"
        );
        assert_eq!(
            fs::read_dir(root.path().join(".archguard"))
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn invalid_paths_do_not_create_project_state() {
        let root = tempfile::tempdir().unwrap();
        for path in [
            "",
            ".",
            "..",
            "../escape",
            "reports/../../escape",
            "/escape",
        ] {
            let error =
                write_project_file(root.path(), Path::new(path), b"bytes", true).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{path}");
        }
        assert!(!root.path().join(".archguard").exists());
    }

    #[test]
    fn directories_cannot_be_replaced_by_artifacts() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join(".archguard/graph.json")).unwrap();
        assert!(write_project_file(root.path(), Path::new("graph.json"), b"data", true).is_err());
        assert!(root.path().join(".archguard/graph.json").is_dir());
    }

    #[cfg(unix)]
    fn swap_boundary(component: &str, overwrite: bool) {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(outside.path().join("reports")).unwrap();
        let replaced = root.path().join(component);
        let held = replaced.with_file_name("held-directory");
        let saved = if component == ".archguard" {
            held.join("reports/graph.html")
        } else {
            held.join("graph.html")
        };
        let result = write_project_file_with(
            root.path(),
            Path::new("reports/graph.html"),
            b"project contents",
            overwrite,
            || {
                fs::rename(&replaced, &held).unwrap();
                symlink(outside.path(), &replaced).unwrap();
            },
        );
        assert!(result.unwrap());
        assert!(
            !outside.path().join("graph.html").exists(),
            "write escaped via reports swap"
        );
        assert!(
            !outside.path().join("reports/graph.html").exists(),
            "write escaped via .archguard swap"
        );
        assert_eq!(fs::read(saved).unwrap(), b"project contents");
    }

    #[cfg(unix)]
    #[test]
    fn opened_project_directory_cannot_be_redirected_by_swap() {
        for overwrite in [true, false] {
            swap_boundary(".archguard", overwrite);
        }
    }

    #[cfg(unix)]
    #[test]
    fn opened_reports_directory_cannot_be_redirected_by_swap() {
        for overwrite in [true, false] {
            swap_boundary(".archguard/reports", overwrite);
        }
    }

    #[cfg(unix)]
    #[test]
    fn existing_symlink_boundaries_and_targets_are_refused() {
        use std::os::unix::fs::symlink;
        for component in [
            ".archguard",
            ".archguard/reports",
            ".archguard/reports/graph.html",
        ] {
            let root = tempfile::tempdir().unwrap();
            let outside = tempfile::tempdir().unwrap();
            let link = root.path().join(component);
            fs::create_dir_all(link.parent().unwrap()).unwrap();
            let sentinel = outside.path().join("sentinel");
            fs::write(&sentinel, b"unchanged").unwrap();
            symlink(
                if component.ends_with("html") {
                    &sentinel
                } else {
                    outside.path()
                },
                link,
            )
            .unwrap();
            assert!(
                write_project_file(
                    root.path(),
                    Path::new("reports/graph.html"),
                    b"project",
                    true
                )
                .is_err()
            );
            assert_eq!(fs::read(sentinel).unwrap(), b"unchanged");
            assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 1);
        }
    }
}
