//! The rust-analyzer SCIP index on disk, so a new process (desktop start,
//! CLI run) does not run rust-analyzer again while sources and manifests are
//! unchanged. It lives in the user's cache directory, not in the project: an
//! index can be large and must not end up in a repository.
//!
//! One file per project root: `<cache>/rust-scip/<sha256(root)>.scip`, holding
//! a magic line, the 32-byte fingerprint, the rust-analyzer version and the
//! raw SCIP bytes. A different fingerprint means the file is stale.
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};

const MAGIC: &[u8] = b"archguard-scip-1\n";
/// Larger files are neither written nor read.
const MAX_BYTES: u64 = 1024 * 1024 * 1024;

/// `ARCHGUARD_CACHE_DIR`, else the platform's per-user cache directory.
pub(super) fn cache_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("ARCHGUARD_CACHE_DIR").filter(|d| !d.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        return home.map(|h| h.join("Library/Caches/archguard"));
    }
    if cfg!(windows) {
        return std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("archguard/cache"));
    }
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| home.map(|h| h.join(".cache")))
        .map(|d| d.join("archguard"))
}

fn file(dir: &Path, root: &Path) -> PathBuf {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let name: String = Sha256::digest(root.to_string_lossy().as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    dir.join("rust-scip").join(format!("{name}.scip"))
}

/// Version and SCIP bytes kept for `root` under `fingerprint`, if any.
pub(super) fn load(dir: &Path, root: &Path, fingerprint: &[u8; 32]) -> Option<(String, Vec<u8>)> {
    let path = file(dir, root);
    if std::fs::metadata(&path).ok()?.len() > MAX_BYTES {
        return None;
    }
    let raw = std::fs::read(path).ok()?;
    let rest = raw.strip_prefix(MAGIC)?;
    let (kept, rest) = rest.split_at_checked(32)?;
    if kept != fingerprint {
        return None;
    }
    let (len, rest) = rest.split_at_checked(4)?;
    let len = u32::from_le_bytes(len.try_into().ok()?) as usize;
    let (version, bytes) = rest.split_at_checked(len)?;
    Some((String::from_utf8(version.to_vec()).ok()?, bytes.to_vec()))
}

/// Keep `bytes` for `root`, replacing the previous file atomically. Errors
/// only cost the next start a rust-analyzer run, so they are returned for
/// tests and otherwise ignored.
pub(super) fn store(
    dir: &Path,
    root: &Path,
    fingerprint: &[u8; 32],
    version: &str,
    bytes: &[u8],
) -> std::io::Result<()> {
    if bytes.len() as u64 > MAX_BYTES {
        return Ok(());
    }
    let path = file(dir, root);
    let parent = path.parent().unwrap_or(dir);
    std::fs::create_dir_all(parent)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(MAGIC)?;
    tmp.write_all(fingerprint)?;
    tmp.write_all(&(version.len() as u32).to_le_bytes())?;
    tmp.write_all(version.as_bytes())?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_index_round_trips_for_its_fingerprint_only() {
        let cache = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let fp = [7u8; 32];
        assert!(load(cache.path(), root.path(), &fp).is_none());
        store(
            cache.path(),
            root.path(),
            &fp,
            "rust-analyzer 1.0",
            b"scip-bytes",
        )
        .unwrap();
        assert_eq!(
            load(cache.path(), root.path(), &fp),
            Some(("rust-analyzer 1.0".into(), b"scip-bytes".to_vec()))
        );
        assert!(
            load(cache.path(), root.path(), &[8u8; 32]).is_none(),
            "stale fingerprint"
        );
        store(cache.path(), root.path(), &[8u8; 32], "v2", b"new").unwrap();
        assert_eq!(
            load(cache.path(), root.path(), &[8u8; 32]),
            Some(("v2".into(), b"new".to_vec())),
            "replaced, one file per root"
        );
        let other = tempfile::tempdir().unwrap();
        assert!(
            load(cache.path(), other.path(), &[8u8; 32]).is_none(),
            "per root"
        );
    }

    #[test]
    fn a_damaged_file_is_ignored() {
        let cache = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let path = file(cache.path(), root.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"archguard-scip-1\nshort").unwrap();
        assert!(load(cache.path(), root.path(), &[0u8; 32]).is_none());
    }
}
