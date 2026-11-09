//! The rust-analyzer SCIP index on disk, so a new process (desktop start,
//! CLI run) does not run rust-analyzer again while sources and manifests are
//! unchanged. It lives in the user's cache directory, not in the project: an
//! index can be large and must not end up in a repository.
//!
//! One file per project root: `<cache>/rust-scip/<sha256(root)>.scip`, holding
//! a magic line, the 32-byte fingerprint, the rust-analyzer version, the
//! content hash of every indexed source and the raw SCIP bytes. A different
//! fingerprint means the file is stale; the hashes tell which of its
//! documents still match the sources.
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

const MAGIC: &[u8] = b"archguard-scip-2\n";
/// Larger files are neither written nor read.
const MAX_BYTES: u64 = 1024 * 1024 * 1024;

/// An index as kept on disk.
#[derive(Debug, PartialEq)]
pub(super) struct Kept {
    pub fingerprint: [u8; 32],
    pub version: String,
    /// Source path → SHA-256 of the text it had when indexed.
    pub documents: HashMap<String, [u8; 32]>,
    pub bytes: Vec<u8>,
}

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

/// A length-prefixed section and the rest.
fn section(raw: &[u8]) -> Option<(&[u8], &[u8])> {
    let (len, rest) = raw.split_at_checked(4)?;
    let len = u32::from_le_bytes(len.try_into().ok()?) as usize;
    rest.split_at_checked(len)
}

/// The index kept for `root`, whatever its fingerprint.
pub(super) fn load(dir: &Path, root: &Path) -> Option<Kept> {
    let path = file(dir, root);
    if std::fs::metadata(&path).ok()?.len() > MAX_BYTES {
        return None;
    }
    let raw = std::fs::read(path).ok()?;
    let rest = raw.strip_prefix(MAGIC)?;
    let (fingerprint, rest) = rest.split_at_checked(32)?;
    let (version, rest) = section(rest)?;
    let (listed, bytes) = section(rest)?;
    let mut documents = HashMap::new();
    for line in std::str::from_utf8(listed).ok()?.lines() {
        let (hash, path) = line.split_once('\t')?;
        documents.insert(path.to_owned(), unhex(hash)?);
    }
    Some(Kept {
        fingerprint: fingerprint.try_into().ok()?,
        version: String::from_utf8(version.to_vec()).ok()?,
        documents,
        bytes: bytes.to_vec(),
    })
}

fn unhex(text: &str) -> Option<[u8; 32]> {
    let mut out = [0u8; 32];
    if text.len() != 64 {
        return None;
    }
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(text.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

/// Keep `kept` for `root`, replacing the previous file atomically. Errors
/// only cost the next start a rust-analyzer run, so they are returned for
/// tests and otherwise ignored.
pub(super) fn store(dir: &Path, root: &Path, kept: &Kept) -> std::io::Result<()> {
    if kept.bytes.len() as u64 > MAX_BYTES {
        return Ok(());
    }
    let mut listed = String::new();
    for (path, hash) in &kept.documents {
        // A tab or newline in a path would break the list: leave it out,
        // so that document counts as changed.
        if path.contains(['\t', '\n']) {
            continue;
        }
        let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
        listed.push_str(&format!("{hex}\t{path}\n"));
    }
    let path = file(dir, root);
    let parent = path.parent().unwrap_or(dir);
    std::fs::create_dir_all(parent)?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(MAGIC)?;
    tmp.write_all(&kept.fingerprint)?;
    for part in [kept.version.as_bytes(), listed.as_bytes()] {
        tmp.write_all(&(part.len() as u32).to_le_bytes())?;
        tmp.write_all(part)?;
    }
    tmp.write_all(&kept.bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kept(fingerprint: u8, version: &str, bytes: &[u8]) -> Kept {
        Kept {
            fingerprint: [fingerprint; 32],
            version: version.into(),
            documents: HashMap::from([
                ("src/a.rs".to_string(), [1u8; 32]),
                ("src/b.rs".to_string(), [2u8; 32]),
            ]),
            bytes: bytes.to_vec(),
        }
    }

    #[test]
    fn an_index_round_trips_with_its_document_hashes() {
        let cache = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        assert!(load(cache.path(), root.path()).is_none());
        let first = kept(7, "rust-analyzer 1.0", b"scip-bytes");
        store(cache.path(), root.path(), &first).unwrap();
        assert_eq!(load(cache.path(), root.path()), Some(first));
        let second = kept(8, "v2", b"new");
        store(cache.path(), root.path(), &second).unwrap();
        assert_eq!(
            load(cache.path(), root.path()),
            Some(second),
            "replaced, one file per root"
        );
        let other = tempfile::tempdir().unwrap();
        assert!(load(cache.path(), other.path()).is_none(), "per root");
    }

    #[test]
    fn damaged_and_older_files_are_ignored() {
        let cache = tempfile::tempdir().unwrap();
        let root = tempfile::tempdir().unwrap();
        let path = file(cache.path(), root.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"archguard-scip-2\nshort").unwrap();
        assert!(load(cache.path(), root.path()).is_none());
        let mut old = b"archguard-scip-1\n".to_vec();
        old.extend([0u8; 32]);
        old.extend(3u32.to_le_bytes());
        old.extend(b"1.0bytes");
        std::fs::write(&path, old).unwrap();
        assert!(load(cache.path(), root.path()).is_none(), "format 1");
    }
}
