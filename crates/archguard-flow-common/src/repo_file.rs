//! Reading repository files without following symlinks (shared by the flow readers).
use std::path::Path;

/// Read repository state without following any component below the chosen root.
pub fn read_repo_file(root: &Path, relative: &Path, limit: u64) -> std::io::Result<Vec<u8>> {
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
                    "symlinks in repository paths are not followed",
                ));
            }
        }
        std::fs::File::open(path)?
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > limit {
        return Err(std::io::Error::other(
            "repository file is not a regular file",
        ));
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(std::io::Error::other(
            "repository file exceeds its size limit",
        ));
    }
    Ok(bytes)
}
