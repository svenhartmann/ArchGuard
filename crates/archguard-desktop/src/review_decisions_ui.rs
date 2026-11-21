//! View → Review decisions… in the window (ui/review_decisions.slint):
//! [`review_decisions::Session`] behind `ReviewDecisionsData`. Generating
//! the review and recording run on a worker thread; the session is out
//! of [`SESSION`] meanwhile, so nothing else touches it. Besides the
//! decisions it shows the review as the web viewer's assessment did
//! ([`review_findings`]) and writes it as Markdown.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use slint::{ComponentHandle, VecModel};

use crate::documents::with_store;
use crate::review_decisions::{Form, KINDS, Session};
use crate::review_findings;
use crate::ui::{AppWindow, DecisionRow, ReviewDecisionsData};

mod view;

const GENERATING: i32 = 1;
const RECORDING: i32 = 2;
const RECORDED: i32 = 3;
const EXPORTED: i32 = 4;
const FAILED: i32 = 5;
const WRITTEN: i32 = 6;

thread_local! {
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
    /// The row to select once the review is generated (snapshot hook).
    static PENDING: Cell<Option<i32>> = const { Cell::new(None) };
}

pub fn wire(ui: &AppWindow) {
    let d = ui.global::<ReviewDecisionsData>();
    let w = ui.as_weak();
    d.on_show(move || with(&w, open));
    let w = ui.as_weak();
    d.on_close(move || {
        with(&w, |ui| ui.global::<ReviewDecisionsData>().set_open(false));
        SESSION.take();
    });
    let w = ui.as_weak();
    d.on_select(move |i| with(&w, |ui| select(ui, i)));
    let w = ui.as_weak();
    d.on_add(move || with(&w, add));
    let w = ui.as_weak();
    d.on_record(move || with(&w, record));
    let w = ui.as_weak();
    d.on_export(move || with(&w, export));
    let w = ui.as_weak();
    d.on_search(move || with(&w, push));
    let w = ui.as_weak();
    d.on_reveal(move |name| with(&w, |ui| view::reveal(ui, &name)));
    let w = ui.as_weak();
    d.on_export_markdown(move || with(&w, export_markdown));
}

fn with(w: &slint::Weak<AppWindow>, f: impl FnOnce(&AppWindow)) {
    if let Some(ui) = w.upgrade() {
        f(&ui);
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

fn set_state(ui: &AppWindow, state: i32, detail: &str) {
    let d = ui.global::<ReviewDecisionsData>();
    d.set_state(state);
    d.set_detail(detail.into());
}

/// Run `job` on a worker thread and `done` with its result in the window.
fn work<T: Send + 'static>(
    ui: &AppWindow,
    job: impl FnOnce() -> T + Send + 'static,
    done: impl FnOnce(&AppWindow, T) + Send + 'static,
) {
    let w = ui.as_weak();
    std::thread::spawn(move || {
        let result = job();
        let _ = slint::invoke_from_event_loop(move || with(&w, |ui| done(ui, result)));
    });
}

/// Open the dialog on the active document with a newly generated review;
/// a session left for "Open in graph" with unrecorded drafts is shown
/// again instead, so the drafts are not lost.
fn open(ui: &AppWindow) {
    let Some(root) = with_store(|s| s.active().map(|d| d.root.clone())) else {
        return;
    };
    let d = ui.global::<ReviewDecisionsData>();
    let kept = SESSION.with_borrow(|s| {
        s.as_ref()
            .is_some_and(|s| s.root == root && !s.drafts.is_empty())
    });
    d.set_open(true);
    if kept {
        return;
    }
    SESSION.take();
    d.set_rows(Rc::new(VecModel::<DecisionRow>::default()).into());
    d.set_selected(-1);
    d.set_drafts(0);
    d.set_revision("".into());
    d.set_query("".into());
    d.set_filter(0);
    d.set_page(0);
    set_state(ui, GENERATING, "");
    work(
        ui,
        move || Session::generate(&root, now()),
        |ui, result| match result {
            Ok(session) => {
                SESSION.set(Some(session));
                set_state(ui, 0, "");
                push(ui);
                if let Some(i) = PENDING.take() {
                    select(ui, i);
                    snapshot_markdown(ui);
                }
            }
            Err(e) => set_state(ui, FAILED, &e),
        },
    );
}

/// The rows the search and filter keep, the summary and the draft count
/// of the session.
fn push(ui: &AppWindow) {
    let d = ui.global::<ReviewDecisionsData>();
    let (query, filter) = (d.get_query(), d.get_filter().max(0) as usize);
    let rows = SESSION.with_borrow(|s| {
        let Some(s) = s else { return Vec::new() };
        d.set_total(s.report.findings.len() as i32);
        d.set_overview(view::overview(review_findings::overview(&s.report)));
        let revision: String = s.report.revision.chars().take(12).collect();
        d.set_revision(revision.into());
        d.set_base(s.report.base_ref.clone().unwrap_or_default().into());
        let index = |k| KINDS.iter().position(|&x| x == k).unwrap_or(0) as i32;
        review_findings::kept(&s.report, &query, filter)
            .into_iter()
            .map(|i| {
                let f = &s.report.findings[i];
                let decision = s.decision(i);
                DecisionRow {
                    index: i as i32,
                    title: f.title.as_str().into(),
                    meta: format!("{} · {}", f.severity, f.category).into(),
                    disposition: f.disposition.as_str().into(),
                    kind: decision.map_or(-1, |d| index(d.decision)),
                    author: decision.map_or("", |d| &d.author).into(),
                    note: decision.map_or("", |d| &d.note).into(),
                    draft: s.drafts.iter().any(|d| d.finding_ref == f.id),
                }
            })
            .collect()
    });
    d.set_rows(Rc::new(VecModel::from(rows)).into());
    d.set_drafts(SESSION.with_borrow(|s| s.as_ref().map_or(0, |s| s.drafts.len() as i32)));
}

/// Select finding `i` and fill the form with its decision.
fn select(ui: &AppWindow, i: i32) {
    let Some(form) = SESSION.with_borrow(|s| s.as_ref().map(|s| s.form(i as usize))) else {
        return;
    };
    SESSION.with_borrow(|s| {
        if let Some(f) = s.as_ref().and_then(|s| s.report.findings.get(i as usize)) {
            view::show_finding(ui, f);
        }
    });
    let d = ui.global::<ReviewDecisionsData>();
    d.set_selected(i);
    d.set_kind(KINDS.iter().position(|&k| k == form.kind).unwrap_or(0) as i32);
    d.set_note(form.note.into());
    d.set_adr(form.adr.into());
    d.set_expires(form.expires_on.into());
    d.set_author(form.author.into());
}

/// Keep the form as the selected finding's draft.
fn add(ui: &AppWindow) {
    let d = ui.global::<ReviewDecisionsData>();
    let form = Form {
        kind: KINDS[(d.get_kind().max(0) as usize).min(KINDS.len() - 1)],
        note: d.get_note().into(),
        adr: d.get_adr().into(),
        expires_on: d.get_expires().into(),
        author: d.get_author().into(),
    };
    let i = d.get_selected();
    SESSION.with_borrow_mut(|s| {
        if let (Some(s), Ok(i)) = (s, usize::try_from(i)) {
            s.add(i, &form);
        }
    });
    set_state(ui, 0, "");
    push(ui);
}

/// Import the drafts on a worker thread; the import validates them.
fn record(ui: &AppWindow) {
    let Some(mut session) = SESSION.take() else {
        return;
    };
    set_state(ui, RECORDING, "");
    work(
        ui,
        move || {
            let result = session.record(now());
            (session, result)
        },
        |ui, (session, result)| {
            SESSION.set(Some(session));
            push(ui);
            match result {
                Ok(n) => {
                    ui.global::<ReviewDecisionsData>().set_count(n as i32);
                    set_state(ui, RECORDED, "");
                }
                Err(e) => set_state(ui, FAILED, &e),
            }
        },
    );
}

/// Where the user chooses to save `suggested`; headless runs take it.
fn save_path(suggested: PathBuf) -> Option<PathBuf> {
    let dir = suggested.parent().map(std::path::Path::to_path_buf);
    if let Some(dir) = &dir {
        let _ = std::fs::create_dir_all(dir);
    }
    if crate::context::headless_run() {
        return Some(suggested);
    }
    let mut dialog = rfd::FileDialog::new();
    if let Some(name) = suggested.file_name() {
        dialog = dialog.set_file_name(name.to_string_lossy());
    }
    if let Some(dir) = dir {
        dialog = dialog.set_directory(dir);
    }
    dialog.save_file()
}

/// Write the review as Markdown (`archguard_engine::review_markdown`).
fn export_markdown(ui: &AppWindow) {
    let Some(suggested) = SESSION.with_borrow(|s| s.as_ref().map(Session::markdown_path)) else {
        return;
    };
    let Some(path) = save_path(suggested) else {
        return;
    };
    let result = SESSION.with_borrow(|s| s.as_ref().map(|s| s.write_markdown(&path)));
    match result {
        Some(Ok(())) => set_state(ui, WRITTEN, &path.display().to_string()),
        Some(Err(e)) => set_state(ui, FAILED, &e),
        None => {}
    }
}

/// Write the decision document where the user chooses; headless runs
/// write it to the suggested path.
fn export(ui: &AppWindow) {
    let Some(suggested) = SESSION.with_borrow(|s| s.as_ref().map(Session::export_path)) else {
        return;
    };
    let Some(path) = save_path(suggested) else {
        return;
    };
    let result = SESSION.with_borrow(|s| s.as_ref().map(|s| s.export(&path)));
    match result {
        Some(Ok(())) => set_state(ui, EXPORTED, &path.display().to_string()),
        Some(Err(e)) => set_state(ui, FAILED, &e),
        None => {}
    }
}

/// `ARCHGUARD_DESKTOP_REVIEW=<row>`: open View → Review decisions… on the
/// active document and select that finding once the review is generated
/// (wait with `ARCHGUARD_DESKTOP_SNAPSHOT_MS`). With it,
/// `ARCHGUARD_DESKTOP_REVIEW_VIEW=page=1&query=…&filter=1&evidence=1&markdown=1`
/// shows the impact page, searches, filters (index of the status filter),
/// opens the finding's evidence and writes the Markdown export to
/// `.archguard/review.md` of the project.
#[cfg(feature = "snapshot")]
pub fn snapshot_hook(ui: &AppWindow) {
    let Some(row) = std::env::var("ARCHGUARD_DESKTOP_REVIEW")
        .ok()
        .and_then(|v| v.parse().ok())
    else {
        return;
    };
    PENDING.set(Some(row));
    open(ui);
    let d = ui.global::<ReviewDecisionsData>();
    for (key, value) in snapshot_view() {
        match key.as_str() {
            "page" => d.set_page(value.parse().unwrap_or(0)),
            "query" => d.set_query(value.into()),
            "filter" => d.set_filter(value.parse().unwrap_or(0)),
            "evidence" => d.set_evidence_open(value == "1"),
            _ => {}
        }
    }
}

#[cfg(feature = "snapshot")]
fn snapshot_view() -> Vec<(String, String)> {
    let view = std::env::var("ARCHGUARD_DESKTOP_REVIEW_VIEW").unwrap_or_default();
    view.split('&')
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[cfg(feature = "snapshot")]
fn snapshot_markdown(ui: &AppWindow) {
    if snapshot_view()
        .iter()
        .any(|(k, v)| k == "markdown" && v == "1")
    {
        export_markdown(ui);
    }
}

#[cfg(not(feature = "snapshot"))]
fn snapshot_markdown(_: &AppWindow) {}
