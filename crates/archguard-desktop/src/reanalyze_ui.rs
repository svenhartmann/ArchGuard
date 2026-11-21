//! View → Analyze again (Cmd/Ctrl+R) and the notice that the active
//! project's sources changed since its analysis, as the web viewer's live
//! reload did. Analysing again replaces the views' data, so it waits while
//! the Graph has simulated changes or an editor holds unsaved edits.
//!
//! Each analysis keeps the source fingerprint it started from
//! ([`Baseline`]). A timer compares it with a fresh one on a worker
//! thread, at most every [`interval`], while the window is visible.
//! Notices (`ui/app.slint`): 12 sources changed, 13 blocked by the
//! simulation, 14 blocked by unsaved changes (`notice`: what), 15 running.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use archguard_engine::{load_config, source_content_fingerprint, source_stamp};
use slint::ComponentHandle;

use crate::documents::{self, DocId, Document, Status, with_store};
use crate::graph_ui::simulation::Simulation;
use crate::ui::AppWindow;

const CHANGED: i32 = 12;
const BLOCKED_SIM: i32 = 13;
const BLOCKED_UNSAVED: i32 = 14;
const RUNNING: i32 = 15;

/// The source fingerprint a document's newest analysis started from;
/// `None` when it could not be taken (e.g. an unreadable configuration).
pub struct Baseline(pub Option<String>);

/// Why a document cannot be analysed again now.
#[derive(Debug, PartialEq, Eq)]
pub enum Blocked {
    Simulation,
    /// What is unsaved, as the save prompt names it.
    Unsaved(String),
}

/// The content fingerprint of `root`'s configured sources (touching a file
/// keeps it); reads every file, so blocking, for workers.
pub fn fingerprint(root: &Path) -> Option<String> {
    let loaded = load_config(None, root).ok()?;
    source_content_fingerprint(root, &loaded.config).ok()
}

/// The last check of one document: the cheap stat stamp it saw and the
/// content fingerprint it found for that stamp.
#[derive(Clone)]
struct Seen {
    id: DocId,
    stamp: String,
    content: Option<String>,
}

/// [`fingerprint`] of `root`, but only reading files when the stat stamp
/// differs from `seen`'s; the check polls, so an idle project costs a walk.
fn check(root: &Path, id: DocId, seen: Option<Seen>) -> (Option<String>, Option<Seen>) {
    let Some(config) = load_config(None, root).ok().map(|l| l.config) else {
        return (None, None);
    };
    let Ok(stamp) = source_stamp(root, &config) else {
        return (None, None);
    };
    if let Some(s) = seen.filter(|s| s.id == id && s.stamp == stamp) {
        return (s.content.clone(), Some(s));
    }
    let content = source_content_fingerprint(root, &config).ok();
    let seen = Seen {
        id,
        stamp,
        content: content.clone(),
    };
    (content, Some(seen))
}

/// Simulated changes (confirmed or proposed) or unsaved edits of `id`.
pub fn blocked(id: DocId) -> Option<Blocked> {
    let simulating = documents::with_state(id, |s: &mut Simulation| {
        !s.ops.is_empty() || !s.pending.is_empty()
    });
    if simulating == Some(true) {
        return Some(Blocked::Simulation);
    }
    let unsaved = with_store(|s| s.get(id).map(Document::unsaved)).unwrap_or_default();
    (!unsaved.is_empty()).then_some(Blocked::Unsaved(unsaved))
}

/// View → Analyze again: the active document to analyse, with the
/// "running" notice, or `None` and the notice why not
/// (`documents_ui` starts the run).
pub fn admit(ui: &AppWindow) -> Option<(DocId, PathBuf)> {
    // Settings text still being typed counts as unsaved.
    crate::settings_view_ui::flush_typing();
    let (id, root) = with_store(|s| s.active().map(|d| (d.id, d.root.clone())))?;
    match blocked(id) {
        Some(Blocked::Simulation) => notice(ui, BLOCKED_SIM, " "),
        Some(Blocked::Unsaved(what)) => notice(ui, BLOCKED_UNSAVED, &what),
        None => {
            notice(ui, RUNNING, " ");
            return Some((id, root));
        }
    }
    None
}

/// An analysis of `id` landed: it is the new baseline; the "changed" and
/// "running" notices are done.
pub fn landed(ui: &AppWindow, id: DocId, baseline: Option<String>) {
    documents::set_state(id, Baseline(baseline));
    if matches!(ui.get_notice_kind(), CHANGED | RUNNING) {
        ui.set_notice("".into());
    }
}

fn notice(ui: &AppWindow, kind: i32, text: &str) {
    ui.set_notice_kind(kind);
    ui.set_notice(text.into());
}

#[derive(Default)]
struct Watch {
    busy: bool,
    next: Option<Instant>,
    /// The changed fingerprint already announced, so a dismissed notice
    /// stays dismissed until the sources change again.
    notified: Option<String>,
    seen: Option<Seen>,
}

thread_local! {
    static WATCH: RefCell<Watch> = RefCell::default();
}

/// Time until the next check: twice the last one's, between 3 and 30 s,
/// so a slow project is not scanned back to back.
fn interval(took: Duration) -> Duration {
    (took * 2).clamp(Duration::from_secs(3), Duration::from_secs(30))
}

/// Whether a check that found `current` announces a change.
fn announces(baseline: Option<&str>, current: Option<&str>, notified: Option<&str>) -> bool {
    match (baseline, current) {
        (Some(b), Some(c)) => c != b && Some(c) != notified,
        _ => false,
    }
}

/// Check the active document's sources from now on.
pub fn watch(ui: &AppWindow) {
    let weak = ui.as_weak();
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_secs(1),
        move || {
            if let Some(ui) = weak.upgrade() {
                poll(&ui);
            }
        },
    );
    // Runs as long as the app.
    std::mem::forget(timer);
}

/// The active, analysed document: id, root and baseline.
fn target() -> Option<(DocId, PathBuf, String)> {
    let (id, root) = with_store(|s| {
        let d = s.active().filter(|d| d.status == Status::Ready)?;
        Some((d.id, d.root.clone()))
    })?;
    let base = documents::with_state(id, |b: &mut Baseline| b.0.clone())??;
    Some((id, root, base))
}

fn poll(ui: &AppWindow) {
    let due = WATCH.with_borrow(|w| !w.busy && w.next.is_none_or(|n| Instant::now() >= n));
    let window = ui.window();
    if !due || !window.is_visible() || window.is_minimized() {
        return;
    }
    let Some((id, root, base)) = target() else {
        return;
    };
    let seen = WATCH.with_borrow_mut(|w| {
        w.busy = true;
        w.seen.clone()
    });
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let started = Instant::now();
        let (current, seen) = check(&root, id, seen);
        let took = started.elapsed();
        let _ = slint::invoke_from_event_loop(move || {
            WATCH.with_borrow_mut(|w| {
                w.busy = false;
                w.seen = seen;
                w.next = Some(Instant::now() + interval(took));
            });
            if let Some(ui) = weak.upgrade() {
                checked(&ui, id, &base, current);
            }
        });
    });
}

/// A check of `id` against `base` found `current`.
fn checked(ui: &AppWindow, id: DocId, base: &str, current: Option<String>) {
    // A newer analysis or another document meanwhile: its own check follows.
    let still = documents::with_state(id, |b: &mut Baseline| b.0.as_deref() == Some(base));
    if still != Some(true) || documents::active_id() != Some(id) {
        return;
    }
    let show =
        WATCH.with_borrow(|w| announces(Some(base), current.as_deref(), w.notified.as_deref()));
    // Another notice stays; the change is announced once it is gone.
    if show && ui.get_notice().is_empty() {
        WATCH.with_borrow_mut(|w| w.notified = current);
        notice(ui, CHANGED, " ");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::analysis::SimOp;

    #[test]
    fn a_change_is_announced_once_per_new_state() {
        assert!(!announces(Some("a"), Some("a"), None), "unchanged");
        assert!(announces(Some("a"), Some("b"), None));
        assert!(!announces(Some("a"), Some("b"), Some("b")), "already shown");
        assert!(announces(Some("a"), Some("c"), Some("b")), "changed again");
        assert!(!announces(None, Some("b"), None), "no baseline");
        assert!(!announces(Some("a"), None, None), "check failed");
    }

    #[test]
    fn checks_back_off_on_slow_projects() {
        assert_eq!(interval(Duration::from_millis(10)), Duration::from_secs(3));
        assert_eq!(interval(Duration::from_secs(4)), Duration::from_secs(8));
        assert_eq!(interval(Duration::from_secs(60)), Duration::from_secs(30));
    }

    /// Editing a source file changes the fingerprint; touching nothing
    /// keeps it.
    #[test]
    fn the_fingerprint_follows_source_edits() {
        let dir = tempfile::tempdir().expect("temp dir");
        let src = dir.path().join("src");
        std::fs::create_dir_all(&src).expect("src dir");
        std::fs::write(src.join("a.py"), "import b\n").expect("write");
        std::fs::write(src.join("b.py"), "x = 1\n").expect("write");
        let first = fingerprint(dir.path()).expect("fingerprint");
        assert_eq!(fingerprint(dir.path()).as_deref(), Some(first.as_str()));
        std::fs::write(src.join("b.py"), "x = 2\n").expect("write");
        assert_ne!(fingerprint(dir.path()), Some(first.clone()), "edited");
        std::fs::write(src.join("c.py"), "y = 1\n").expect("write");
        let added = fingerprint(dir.path());
        assert!(added.is_some() && added != Some(first), "file added");
    }

    #[test]
    fn simulated_changes_and_unsaved_edits_block_analysing_again() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (id, _) = with_store(|s| s.open(dir.path()));
        assert_eq!(blocked(id), None);
        documents::set_state(
            id,
            Simulation {
                pending: vec![SimOp::CutEdge {
                    from: "a".into(),
                    to: "b".into(),
                }],
                ..Simulation::default()
            },
        );
        assert_eq!(blocked(id), Some(Blocked::Simulation));
        documents::set_state(id, Simulation::default());
        documents::set_save_hook(id, "config", || Ok(()));
        documents::mark_dirty(id, "config", "Layer rules");
        assert_eq!(blocked(id), Some(Blocked::Unsaved("Layer rules".into())));
        documents::mark_clean(id, "config");
        assert_eq!(blocked(id), None);
    }
}
