//! The module instance an expression is evaluated in, and the `file()` trust boundary (ticket 30).
use crate::context::Reader;
use archguard_terraform::module::{ModuleCtx, Resource};
use std::path::Path;

/// Largest JSON file `file()` reads.
const MAX_FILE: u64 = 1024 * 1024;

/// A module instance with what `file()` needs: the project root, the safe reader and the module
/// directory relative to the root (`path.module`).
pub(super) struct Scope<'a, 'c> {
    pub(super) module: &'c ModuleCtx<'a>,
    pub(super) root: &'c Path,
    pub(super) read: &'c Reader,
    pub(super) dir: &'c str,
}

impl<'a, 'c> Scope<'a, 'c> {
    /// The scope of a resource in the project at `root`.
    pub(super) fn of(r: &'c Resource<'a>, root: &'c Path, read: &'c Reader) -> Self {
        let dir = r.file.path.rsplit_once('/').map_or("", |(d, _)| d);
        Scope {
            module: &r.module,
            root,
            read,
            dir,
        }
    }

    /// Text of the file `rel`, relative to the root as `path.module` is; `None` when it resolves outside the root
    /// (`..`, symlink), is not a regular file or exceeds `MAX_FILE`.
    pub(super) fn file(&self, rel: &str) -> Option<String> {
        let root = self.root.canonicalize().ok()?;
        let path = self.root.join(rel).canonicalize().ok()?;
        let inside = path.strip_prefix(&root).ok()?;
        let meta = std::fs::metadata(&path).ok()?;
        (meta.is_file() && meta.len() <= MAX_FILE).then_some(())?;
        (self.read)(self.root, &self.root.join(inside))
    }
}
