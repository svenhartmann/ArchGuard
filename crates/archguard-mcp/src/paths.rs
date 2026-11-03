//! F02 — one path policy for every MCP file access.
//!
//! The server root is canonicalised once when the server starts
//! ([`crate::run_stdio_server`]) and re-verified per call through
//! [`server_root`], so direct [`crate::server::ArchguardTools::new`] users get the
//! same policy. Every caller- or config-supplied path goes through
//! [`resolve`]:
//!
//! * relative arguments are joined onto the server root — never resolved
//!   against the process working directory;
//! * absolute arguments are accepted only when they canonicalise back
//!   inside the root (sibling roots and `/etc/passwd` style paths fail);
//! * the canonicalised result must `starts_with` the canonical root, which
//!   also closes `..` climbing and symlink escapes (canonicalisation
//!   follows symlinks, the check then sees where they really lead);
//! * create-if-missing targets (`existing == false`, e.g. a database the
//!   store opens) may be absent, but their parent must already exist
//!   inside the root and the target itself must not be a symlink — a
//!   dangling symlink would otherwise be followed by the creator and write
//!   outside the root.
//!
//! Rejections happen before any file is read or created. The CLI keeps its
//! own CWD-relative path selection (Kap. 9); this policy is MCP-only.

use std::path::{Path, PathBuf};

/// Canonicalise the configured server root; an unreadable root is an error.
pub(crate) fn server_root(root: &Path) -> Result<PathBuf, String> {
    root.canonicalize()
        .map_err(|err| format!("server root {} not readable: {err}", root.display()))
}

/// Resolve `arg` against the canonical `root` and prove the result stays
/// inside the server root. `existing == true` is for read paths (the
/// target must exist); `existing == false` additionally accepts a missing
/// target whose parent is verified inside the root (see the module docs).
pub(crate) fn resolve(root: &Path, arg: &Path, existing: bool) -> Result<PathBuf, String> {
    let joined = if arg.is_absolute() {
        arg.to_path_buf()
    } else {
        root.join(arg)
    };
    match joined.canonicalize() {
        Ok(canonical) => {
            if !canonical.starts_with(root) {
                Err(outside(arg, root))
            } else {
                Ok(canonical)
            }
        }
        Err(err) => {
            if existing {
                return Err(format!("path {} not readable: {err}", joined.display()));
            }
            // Not resolvable: either absent (a legal create target) or a
            // dangling symlink (never legal — the creator would follow it).
            if std::fs::symlink_metadata(&joined).is_ok_and(|meta| meta.file_type().is_symlink()) {
                return Err(format!(
                    "path {} is a symlink that does not resolve inside the server root",
                    joined.display()
                ));
            }
            let parent = joined
                .parent()
                .ok_or_else(|| format!("path {} has no parent", joined.display()))?;
            let parent = parent
                .canonicalize()
                .map_err(|err| format!("parent {} not readable: {err}", parent.display()))?;
            if !parent.starts_with(root) {
                return Err(outside(arg, root));
            }
            let name = joined
                .file_name()
                .ok_or_else(|| format!("path {} has no file name", joined.display()))?;
            Ok(parent.join(name))
        }
    }
}

fn outside(arg: &Path, root: &Path) -> String {
    format!(
        "path {} is outside the server root {}",
        arg.display(),
        root.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let base = tempfile::tempdir().expect("temp base");
        let root = base.path().join("root");
        let outside = base.path().join("outside");
        fs::create_dir_all(root.join("sub")).expect("mkdir sub");
        fs::create_dir_all(&outside).expect("mkdir outside");
        fs::write(root.join("inside.txt"), "in").expect("write inside");
        fs::write(outside.join("secret.txt"), "out").expect("write secret");
        (base, root.canonicalize().expect("canonical root"), outside)
    }

    #[test]
    fn relative_arguments_resolve_under_the_root_not_the_cwd() {
        let (_base, root, _outside) = fixture();
        // "sub" and "inside.txt" do not exist relative to the process CWD;
        // accepting them proves the root is the base, not the CWD.
        assert_eq!(
            resolve(&root, Path::new("inside.txt"), true).unwrap(),
            root.join("inside.txt")
        );
        assert_eq!(
            resolve(&root, Path::new("sub"), true).unwrap(),
            root.join("sub")
        );
    }

    #[test]
    fn absolute_and_parent_dir_escapes_are_rejected() {
        let (_base, root, outside) = fixture();
        let err = resolve(&root, &outside.join("secret.txt"), true).unwrap_err();
        assert!(err.contains("outside the server root"), "{err}");
        let err = resolve(&root, Path::new("../outside/secret.txt"), true).unwrap_err();
        assert!(err.contains("outside the server root"), "{err}");
    }

    #[test]
    #[cfg(unix)]
    fn symlink_escapes_are_rejected_after_canonicalisation() {
        let (_base, root, outside) = fixture();
        std::os::unix::fs::symlink(&outside, root.join("link")).expect("symlink");
        let err = resolve(&root, Path::new("link"), true).unwrap_err();
        assert!(err.contains("outside the server root"), "{err}");
    }

    #[test]
    fn create_targets_need_a_verified_parent_inside_the_root() {
        let (_base, root, outside) = fixture();
        // Legal: a missing file below an existing in-root directory.
        assert_eq!(
            resolve(&root, Path::new("sub/graph.db"), false).unwrap(),
            root.join("sub/graph.db")
        );
        // Illegal: missing parent, and a parent that escapes via `..`.
        let err = resolve(&root, Path::new("missing/graph.db"), false).unwrap_err();
        assert!(err.contains("parent"), "{err}");
        assert!(!root.join("missing").exists());
        let err = resolve(&root, Path::new("../outside/new.db"), false).unwrap_err();
        assert!(err.contains("outside the server root"), "{err}");
        assert!(!outside.join("new.db").exists());
    }

    #[test]
    #[cfg(unix)]
    fn dangling_symlink_targets_are_rejected_not_followed() {
        let (_base, root, outside) = fixture();
        std::os::unix::fs::symlink(outside.join("mirror.db"), root.join("link.db")).expect("link");
        let err = resolve(&root, Path::new("link.db"), false).unwrap_err();
        assert!(err.contains("symlink"), "{err}");
    }
}
