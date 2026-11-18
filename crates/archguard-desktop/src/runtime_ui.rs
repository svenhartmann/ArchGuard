//! Runtime tab (ADR 0004): single requests of one application from
//! `.archguard/runtime/`, newest first, each with its timeline, and the
//! summary over all of them. With Live on, a timer compares the input files'
//! sizes and times once a second and re-reads the profile on a worker thread
//! when they changed.

mod model;
mod summary;
mod time;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use archguard_engine::core::RuntimeProfile;
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::context::Loaded;
use crate::documents::{self, DocId};
use crate::tab_support::{self, Actions, TabState, index};
use crate::ui::{AppWindow, RtDetail, RuntimeData, RuntimeModel};

/// The page the tab opens on: "Time", where the slow spots are.
const TIME_PAGE: usize = 2;

/// Input files' count, total size and newest change: cheap to read each second.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Stamp {
    files: u64,
    bytes: u64,
    newest: Option<SystemTime>,
}

fn stamp(root: &Path) -> Stamp {
    fn walk(dir: &Path, depth: usize, s: &mut Stamp) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(meta) = std::fs::symlink_metadata(entry.path()) else {
                continue;
            };
            if meta.is_dir() && depth < 4 {
                walk(&entry.path(), depth + 1, s);
            } else if meta.is_file() {
                add(&meta, s);
            }
        }
    }
    fn add(meta: &std::fs::Metadata, s: &mut Stamp) {
        s.files += 1;
        s.bytes += meta.len();
        s.newest = s.newest.max(meta.modified().ok());
    }
    let mut s = Stamp::default();
    walk(&root.join(".archguard/runtime"), 1, &mut s);
    if let Ok(meta) = std::fs::symlink_metadata(root.join(".archguard/otel-traces.json")) {
        add(&meta, &mut s);
    }
    s
}

/// The tab's state for one document.
pub struct RuntimeState {
    root: PathBuf,
    profile: Option<Arc<RuntimeProfile>>,
    stamp: Stamp,
    /// The selected request by (trace, span); `None` follows the newest.
    selected: Option<(String, String)>,
    page: usize,
    live: bool,
    reloading: bool,
}

fn build_from(root: PathBuf) -> RuntimeState {
    let stamp = stamp(&root);
    let mut state = RuntimeState {
        profile: archguard_engine::runtime_profile(&root, None).map(Arc::new),
        root,
        stamp,
        selected: None,
        // Open where the slow spots are; the request list is one click away.
        page: TIME_PAGE,
        live: true,
        reloading: false,
    };
    snapshot_overrides(&mut state);
    state
}

/// Register the tab's callbacks and the live timer once at start-up.
pub fn wire(ui: &AppWindow) {
    let d = ui.global::<RuntimeData>();
    let a = Actions::<RuntimeState>::new(ui);
    d.on_select_page(a.on(|s, _, i: i32| s.page = index(i).unwrap_or(0).min(2)));
    d.on_toggle_live(a.on0(|s, _| s.live = !s.live));
    d.on_select_request(a.on(|s, _, i: i32| {
        let chosen = index(i).and_then(|i| s.profile.as_ref()?.requests.get(i));
        // The newest request keeps following new ones; any other stays put.
        s.selected = chosen
            .filter(|_| i > 0)
            .map(|r| (r.trace_id.clone(), r.span_id.clone()));
    }));
    start_live(ui);
}

fn start_live(ui: &AppWindow) {
    thread_local! {
        static TIMER: slint::Timer = slint::Timer::default();
    }
    let weak = ui.as_weak();
    TIMER.with(|t| {
        t.start(
            slint::TimerMode::Repeated,
            Duration::from_secs(1),
            move || {
                if let Some(ui) = weak.upgrade() {
                    tick(&ui);
                }
            },
        );
    });
}

/// Re-read the active document's profile when it is [`due`]; a change while
/// hidden is read once the tab shows again.
fn tick(ui: &AppWindow) {
    let Some(doc) = documents::active_id() else {
        return;
    };
    let shown = runtime_shown(&crate::panes_ui::layout());
    let root = documents::with_state(doc, |s: &mut RuntimeState| {
        if !due(s, shown) {
            return None;
        }
        s.reloading = true;
        Some(s.root.clone())
    })
    .flatten();
    let Some(root) = root else { return };
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        // Untraced: reading its own recording would record itself and
        // change the file it watches, so it would reload every second.
        let (new, profile) = reload(&root, || {
            archguard_engine::self_trace::untraced(|| {
                archguard_engine::runtime_profile(&root, None)
            })
        });
        let _ = slint::invoke_from_event_loop(move || arrive(&weak, doc, profile, new));
    });
}

fn arrive(
    weak: &slint::Weak<AppWindow>,
    doc: DocId,
    profile: Result<Option<RuntimeProfile>, String>,
    new: Stamp,
) {
    let shown = documents::active_id() == Some(doc);
    documents::with_state(doc, |s: &mut RuntimeState| {
        land(s, profile, new);
        if let (true, Some(ui)) = (shown, weak.upgrade()) {
            push(&ui, s);
        }
    });
}

/// The stamp before `read` runs, so files that change while it reads count
/// as new on the next tick, and what `read` returned or why it panicked.
fn reload(
    root: &Path,
    read: impl FnOnce() -> Option<RuntimeProfile>,
) -> (Stamp, Result<Option<RuntimeProfile>, String>) {
    let new = stamp(root);
    (new, tab_support::catch(read))
}

/// A failed reload keeps the last profile; the next change tries again.
fn land(s: &mut RuntimeState, profile: Result<Option<RuntimeProfile>, String>, new: Stamp) {
    if let Ok(profile) = profile {
        s.profile = profile.map(Arc::new);
    }
    s.stamp = new;
    s.reloading = false;
}

/// Whether `tick` re-reads now: Live on, no read running, the input changed,
/// and a pane shows the tab or the document has no recording yet, so the
/// first one un-dims the tab while another view shows.
fn due(s: &RuntimeState, shown: bool) -> bool {
    (shown || s.profile.is_none()) && s.live && !s.reloading && stamp(&s.root) != s.stamp
}

fn runtime_shown(l: &crate::panes::Layout) -> bool {
    (0..l.panes.len()).any(|p| l.view(p).is_some_and(|t| t.kind == "runtime"))
}

impl TabState for RuntimeState {
    /// It reads only the trace folder; the stamp keeps it current, so a new
    /// analysis keeps the chosen request and Live off.
    const DERIVED: bool = false;

    fn push(&mut self, ui: &AppWindow) {
        push(ui, self);
    }

    fn pending(ui: &AppWindow, error: Option<&str>) {
        ui.global::<RuntimeData>().set_model(pending_model(error));
    }
}

/// The loading text, or why the first read failed.
fn pending_model(error: Option<&str>) -> RuntimeModel {
    RuntimeModel {
        state: if error.is_some() { 3 } else { 0 },
        error: error.unwrap_or_default().into(),
        ..Default::default()
    }
}

/// Show the active document's tab; its profile is read on a worker thread.
pub fn show(ui: &AppWindow, data: &Loaded) {
    tab_support::show_async(ui, || {
        let root = data.root.to_path_buf();
        move || build_from(root)
    });
}

/// The index of the selected request, the newest if none or gone.
fn selected(s: &RuntimeState, p: &RuntimeProfile) -> usize {
    s.selected
        .as_ref()
        .and_then(|(trace, span)| {
            p.requests
                .iter()
                .position(|r| &r.trace_id == trace && &r.span_id == span)
        })
        .unwrap_or(0)
}

pub fn push(ui: &AppWindow, s: &RuntimeState) {
    ui.global::<RuntimeData>().set_model(build(s));
}

fn build(s: &RuntimeState) -> RuntimeModel {
    let summary = summary::model(s.profile.as_deref());
    let Some(p) = s.profile.as_deref() else {
        return RuntimeModel {
            state: 2,
            live: s.live,
            summary,
            ..Default::default()
        };
    };
    let threshold = p.thresholds.n_plus_one_min_calls;
    let chosen = selected(s, p);
    let requests: Vec<_> = p
        .requests
        .iter()
        .enumerate()
        .map(|(i, r)| model::request(r, threshold, i == chosen))
        .collect();
    RuntimeModel {
        state: 1,
        page: i32::try_from(s.page).unwrap_or(0),
        live: s.live,
        requests_total: i32::try_from(p.requests_total).unwrap_or(i32::MAX),
        requests: ModelRc::new(VecModel::from(requests)),
        detail: p
            .requests
            .get(chosen)
            .map_or_else(RtDetail::default, |r| model::detail(r, threshold)),
        summary,
        breakdown: time::breakdown(p),
        hot: time::hot(p),
        legend: time::legend(p),
        sampled: time::sampled(p.requests.get(chosen)),
        error: Default::default(),
    }
}

/// `ARCHGUARD_DESKTOP_RUNTIME_PAGE=requests|summary|time` and
/// `ARCHGUARD_DESKTOP_RUNTIME_REQUEST=<index>` set the view for snapshot runs.
#[cfg(feature = "snapshot")]
fn snapshot_overrides(s: &mut RuntimeState) {
    s.page = match std::env::var("ARCHGUARD_DESKTOP_RUNTIME_PAGE").as_deref() {
        Ok("requests") => 0,
        Ok("summary") => 1,
        _ => TIME_PAGE,
    };
    let request = std::env::var("ARCHGUARD_DESKTOP_RUNTIME_REQUEST")
        .ok()
        .and_then(|i| i.parse::<usize>().ok());
    if let (Some(i), Some(p)) = (request, s.profile.as_deref()) {
        s.selected = p
            .requests
            .get(i)
            .map(|r| (r.trace_id.clone(), r.span_id.clone()));
    }
}

#[cfg(not(feature = "snapshot"))]
fn snapshot_overrides(_: &mut RuntimeState) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stamp_changes_when_a_file_grows() {
        let dir = tempfile::tempdir().unwrap();
        let run = dir.path().join(".archguard/runtime/run-1");
        std::fs::create_dir_all(&run).unwrap();
        let file = run.join("traces-0.jsonl");
        std::fs::write(&file, "{}\n").unwrap();
        let before = stamp(dir.path());
        assert_eq!((before.files, before.bytes), (1, 3));
        std::fs::write(&file, "{}\n{}\n").unwrap();
        assert_ne!(stamp(dir.path()), before);
    }

    #[test]
    fn a_reload_stamps_before_it_reads_so_appends_meanwhile_show_next() {
        let dir = tempfile::tempdir().unwrap();
        let run = dir.path().join(".archguard/runtime/run-1");
        std::fs::create_dir_all(&run).unwrap();
        let file = run.join("traces-0.jsonl");
        std::fs::write(&file, "{}\n").unwrap();
        let (new, _) = reload(dir.path(), || {
            std::fs::write(&file, "{}\n{}\n").unwrap();
            None
        });
        assert_ne!(new, stamp(dir.path()), "the next tick reloads the append");
    }

    #[test]
    fn a_panicking_reload_keeps_the_profile_and_stays_live() {
        let dir = tempfile::tempdir().unwrap();
        let (new, profile) = reload(dir.path(), || panic!("reader failed"));
        assert!(profile.is_err());
        let old = Arc::new(RuntimeProfile::default());
        let mut s = build_from(dir.path().to_path_buf());
        s.profile = Some(old.clone());
        s.reloading = true;
        land(&mut s, profile, new);
        assert!(!s.reloading, "the next change reloads again");
        assert_eq!(s.stamp, new);
        assert!(Arc::ptr_eq(s.profile.as_ref().unwrap(), &old));
    }

    #[test]
    fn live_reload_runs_only_while_a_pane_shows_the_runtime_view() {
        use crate::panes::Layout;
        assert!(!runtime_shown(&Layout::single("graph")));
        assert!(runtime_shown(&Layout::single("runtime")));
        let mut l = Layout::single("graph");
        l.split_right(Some("runtime"), None);
        assert!(runtime_shown(&l));
    }

    /// The tab is dimmed without recordings; the first one, made while
    /// another view shows, must reach it without a click.
    #[test]
    fn without_recordings_live_reload_runs_also_while_hidden() {
        let dir = tempfile::tempdir().unwrap();
        let s = build_from(dir.path().to_path_buf());
        let run = dir.path().join(".archguard/runtime/run-1");
        std::fs::create_dir_all(&run).unwrap();
        let file = run.join("traces-0.jsonl");
        let fixture = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/runtime/shop.jsonl"
        );
        std::fs::copy(fixture, &file).unwrap();
        assert!(due(&s, false), "the first recording un-dims the tab");
        let s = build_from(dir.path().to_path_buf());
        let mut more = std::fs::read(&file).unwrap();
        more.push(b'\n');
        std::fs::write(&file, more).unwrap();
        assert!(!due(&s, false), "with data it idles while hidden");
        assert!(due(&s, true));
    }

    /// A failed first read is kept as the tab's error and not retried, so
    /// the tab must say so instead of loading forever.
    #[test]
    fn a_failed_first_read_shows_its_error() {
        assert_eq!(pending_model(None).state, 0, "loading");
        let m = pending_model(Some("boom"));
        assert_eq!((m.state, m.error.as_str()), (3, "boom"));
    }

    /// The tab reads only the trace folder: a new analysis (rust-analyzer
    /// finished indexing) keeps the chosen request, page and Live off;
    /// closing the document drops them.
    #[test]
    fn a_new_analysis_keeps_the_runtime_state() {
        use tab_support::Start;
        let dir = tempfile::tempdir().unwrap();
        let doc = documents::with_store(|s| {
            *s = documents::Store::default();
            s.open(dir.path()).0
        });
        assert_eq!(tab_support::start::<RuntimeState>(), Start::Build(doc));
        let mut built = build_from(dir.path().to_path_buf());
        built.selected = Some(("t".into(), "s".into()));
        (built.page, built.live) = (0, false);
        assert!(tab_support::finish(
            doc,
            documents::generation(doc),
            Ok(built)
        ));
        documents::with_store(|s| s.get_mut(doc).unwrap().forget_derived());
        assert_eq!(
            tab_support::start::<RuntimeState>(),
            Start::Ready,
            "not rebuilt"
        );
        let kept = documents::with_state(doc, |s: &mut RuntimeState| {
            (s.selected.clone(), s.page, s.live)
        });
        assert_eq!(kept, Some((Some(("t".into(), "s".into())), 0, false)));
        documents::with_store(|s| s.close(0));
        assert!(documents::with_state(doc, |_: &mut RuntimeState| ()).is_none());
    }

    #[test]
    fn without_input_the_tab_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let s = build_from(dir.path().to_path_buf());
        assert_eq!(build(&s).state, 2);
    }

    #[test]
    fn the_newest_request_is_shown_until_another_is_chosen() {
        let dir = tempfile::tempdir().unwrap();
        let run = dir.path().join(".archguard/runtime/run-1");
        std::fs::create_dir_all(&run).unwrap();
        let fixture = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/runtime/shop.jsonl"
        );
        std::fs::copy(fixture, run.join("traces-0.jsonl")).unwrap();
        let mut s = build_from(dir.path().to_path_buf());
        let m = build(&s);
        assert_eq!((m.state, m.requests_total), (1, 3));
        assert_eq!(m.detail.db_calls, 3, "the newest request (trace b)");
        let p = s.profile.clone().unwrap();
        s.selected = Some((
            p.requests[1].trace_id.clone(),
            p.requests[1].span_id.clone(),
        ));
        let m = build(&s);
        assert_eq!(m.detail.db_calls, 4);
        assert_eq!(m.detail.own, "45", "70 ms minus 25 ms in DB calls");
    }
}
