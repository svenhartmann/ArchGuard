//! Project-local files share the cache's descriptor-relative storage on Unix.
use std::io;
use std::path::{Component, Path};

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
    use crate::cache::safe;
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
