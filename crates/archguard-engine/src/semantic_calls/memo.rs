//! The last call analysis per project root, reused while the configuration and
//! the source set are unchanged. The desktop app, the MCP server and `serve`
//! ask for it several times per project (pipeline, Start, arc42, patterns,
//! performance); each run re-parses every source and may start compiler
//! tools. Kept in memory only; an unavailable result is never kept, as a
//! missing tool or a timeout may be transient.
use archguard_core::{CallAnalysis, Config};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Roots kept; the oldest goes first.
const MAX_ROOTS: usize = 8;

type Entry = (String, CallAnalysis);
/// Roots in insertion order (oldest first) and their kept results.
type Kept = (Vec<PathBuf>, HashMap<PathBuf, Entry>);
static MEMO: OnceLock<Mutex<Kept>> = OnceLock::new();

/// Identity of what the analysis reads: configuration and source set
/// (contents and Rust manifests). `None` when the sources cannot be listed.
fn key(root: &Path, config: &Config) -> Option<String> {
    let sources = crate::cache::source_fingerprint(root, config).ok()?;
    let config = serde_json::to_string(config).ok()?;
    Some(format!("{sources}\n{config}"))
}

/// `compute` for `root`, or the kept result of an unchanged project.
pub(super) fn cached(
    root: &Path,
    config: &Config,
    compute: impl FnOnce() -> CallAnalysis,
) -> CallAnalysis {
    let Some(key) = key(root, config) else {
        return compute();
    };
    let memo = MEMO.get_or_init(Mutex::default);
    let root = root.to_path_buf();
    {
        let guard = memo.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((kept, calls)) = guard.1.get(&root)
            && *kept == key
        {
            return calls.clone();
        }
    }
    let calls = compute();
    if calls.status != "unavailable" {
        let mut guard = memo.lock().unwrap_or_else(|e| e.into_inner());
        let (order, entries) = &mut *guard;
        order.retain(|r| *r != root);
        order.push(root.clone());
        if order.len() > MAX_ROOTS {
            let oldest = order.remove(0);
            entries.remove(&oldest);
        }
        entries.insert(root, (key, calls.clone()));
    }
    calls
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/a.py"), "def f():\n    return 1\n").unwrap();
        dir
    }

    fn analysis(status: &str) -> CallAnalysis {
        CallAnalysis {
            status: status.into(),
            ..Default::default()
        }
    }

    #[test]
    fn an_unchanged_project_is_analysed_once() {
        let dir = project();
        let config = Config::default();
        let runs = Cell::new(0);
        let compute = || {
            runs.set(runs.get() + 1);
            analysis("complete")
        };
        cached(dir.path(), &config, compute);
        cached(dir.path(), &config, compute);
        assert_eq!(runs.get(), 1);
        std::fs::write(dir.path().join("src/a.py"), "def f():\n    return 2\n").unwrap();
        cached(dir.path(), &config, compute);
        assert_eq!(runs.get(), 2, "a changed source runs the analysis again");
    }

    #[test]
    fn unavailable_results_are_not_kept() {
        let dir = project();
        let config = Config::default();
        let runs = Cell::new(0);
        let compute = || {
            runs.set(runs.get() + 1);
            analysis("unavailable")
        };
        cached(dir.path(), &config, compute);
        cached(dir.path(), &config, compute);
        assert_eq!(runs.get(), 2);
    }
}
