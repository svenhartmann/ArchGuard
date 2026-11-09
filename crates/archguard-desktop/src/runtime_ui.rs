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

/// Re-read the active document's profile when its input changed.
fn tick(ui: &AppWindow) {
    let Some(doc) = documents::active_id() else {
        return;
    };
    let root = documents::with_state(doc, |s: &mut RuntimeState| {
        if !s.live || s.reloading || stamp(&s.root) == s.stamp {
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
        let profile = archguard_engine::self_trace::untraced(|| {
            archguard_engine::runtime_profile(&root, None)
        });
        let new = stamp(&root);
        let _ = slint::invoke_from_event_loop(move || arrive(&weak, doc, profile, new));
    });
}

fn arrive(weak: &slint::Weak<AppWindow>, doc: DocId, profile: Option<RuntimeProfile>, new: Stamp) {
    let shown = documents::active_id() == Some(doc);
    documents::with_state(doc, |s: &mut RuntimeState| {
        s.profile = profile.map(Arc::new);
        s.stamp = new;
        s.reloading = false;
        if let (true, Some(ui)) = (shown, weak.upgrade()) {
            push(&ui, s);
        }
    });
}

impl TabState for RuntimeState {
    fn push(&mut self, ui: &AppWindow) {
        push(ui, self);
    }

    fn pending(ui: &AppWindow, _error: Option<&str>) {
        ui.global::<RuntimeData>()
            .set_model(RuntimeModel::default());
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
