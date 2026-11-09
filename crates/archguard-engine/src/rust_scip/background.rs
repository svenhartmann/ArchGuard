//! rust-analyzer in the background, for the desktop app: a missing index is
//! built on its own thread while the analysis goes on without it, and the
//! app is told when it is there. The CLI and the MCP server never set the
//! hook and keep waiting for the index.
//!
//! One job per fingerprint at a time; a failed job is not repeated for the
//! same fingerprint, so its error is reported instead of a pending note.
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// The note while the index is being built.
pub(crate) const PENDING: &str =
    "index is being built in the background; call targets follow when it is done";

type Ready = Arc<dyn Fn(&Path) + Send + Sync>;
static HOOK: OnceLock<Ready> = OnceLock::new();

#[derive(Default)]
struct Jobs {
    running: HashSet<[u8; 32]>,
    /// The last failure; a newer fingerprint starts a new job.
    failed: Option<([u8; 32], String)>,
    /// Finished jobs per root, so kept results of that root count as old.
    generations: HashMap<PathBuf, u64>,
}
static JOBS: OnceLock<Mutex<Jobs>> = OnceLock::new();

fn jobs() -> std::sync::MutexGuard<'static, Jobs> {
    JOBS.get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Build missing rust-analyzer indexes in the background from now on and
/// call `ready` with the project root when one is done or has failed.
/// The first hook set in a process stays.
pub fn index_rust_in_background(ready: impl Fn(&Path) + Send + Sync + 'static) {
    let _ = HOOK.set(Arc::new(ready));
}

pub(super) fn hook() -> Option<Ready> {
    HOOK.get().cloned()
}

/// Background jobs finished for `root`: part of the call analysis key, so
/// a result computed without the index is replaced once it is there.
pub(crate) fn generation(root: &Path) -> u64 {
    jobs().generations.get(root).copied().unwrap_or(0)
}

/// Start `job` for `fingerprint` unless it runs already or failed; returns
/// the note to report meanwhile.
pub(super) fn start(
    root: &Path,
    fingerprint: [u8; 32],
    job: impl FnOnce() -> Result<(), String> + Send + 'static,
    ready: Ready,
) -> String {
    {
        let mut jobs = jobs();
        if let Some((failed, error)) = &jobs.failed
            && *failed == fingerprint
        {
            return error.clone();
        }
        if !jobs.running.insert(fingerprint) {
            return PENDING.into();
        }
    }
    let root = root.to_path_buf();
    std::thread::spawn(move || {
        let result = job();
        {
            let mut jobs = jobs();
            jobs.running.remove(&fingerprint);
            if let Err(error) = result {
                jobs.failed = Some((fingerprint, error));
            }
            *jobs.generations.entry(root.clone()).or_default() += 1;
        }
        ready(&root);
    });
    PENDING.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    fn ready_channel() -> (Ready, mpsc::Receiver<PathBuf>) {
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let ready: Ready = Arc::new(move |root: &Path| {
            let _ = tx.lock().unwrap().send(root.to_path_buf());
        });
        (ready, rx)
    }

    #[test]
    fn a_missing_index_is_built_once_and_reported_when_done() {
        let root = tempfile::tempdir().unwrap();
        let fingerprint = [1; 32];
        let (ready, done) = ready_channel();
        let (release, gate) = mpsc::channel::<()>();
        let runs = Arc::new(AtomicUsize::new(0));
        let job = {
            let runs = runs.clone();
            move || {
                runs.fetch_add(1, Ordering::SeqCst);
                gate.recv().map_err(|e| e.to_string())
            }
        };
        assert_eq!(start(root.path(), fingerprint, job, ready.clone()), PENDING);
        let again = start(root.path(), fingerprint, || Ok(()), ready.clone());
        assert_eq!(again, PENDING, "a running job is not started twice");
        assert_eq!(generation(root.path()), 0);
        release.send(()).unwrap();
        let reported = done.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(reported, root.path());
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert_eq!(generation(root.path()), 1);
    }

    #[test]
    fn a_failed_job_reports_its_error_instead_of_running_again() {
        let root = tempfile::tempdir().unwrap();
        let fingerprint = [2; 32];
        let (ready, done) = ready_channel();
        start(
            root.path(),
            fingerprint,
            || Err("no toolchain".into()),
            ready.clone(),
        );
        done.recv_timeout(Duration::from_secs(10)).unwrap();
        let runs = Arc::new(AtomicUsize::new(0));
        let counted = {
            let runs = runs.clone();
            move || {
                runs.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        };
        assert_eq!(
            start(root.path(), fingerprint, counted, ready),
            "no toolchain"
        );
        assert_eq!(runs.load(Ordering::SeqCst), 0);
    }
}
