//! View → Review decisions… in the window (ui/review_decisions.slint):
//! [`review_decisions::Session`] behind `ReviewDecisionsData`. Generating
//! the review and recording run on a worker thread; the session is out
//! of [`SESSION`] meanwhile, so nothing else touches it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use slint::{ComponentHandle, VecModel};

use crate::documents::with_store;
use crate::review_decisions::{Form, KINDS, Session};
use crate::ui::{AppWindow, DecisionRow, ReviewDecisionsData};

const GENERATING: i32 = 1;
const RECORDING: i32 = 2;
const RECORDED: i32 = 3;
const EXPORTED: i32 = 4;
const FAILED: i32 = 5;

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

/// Open the dialog on the active document with a newly generated review.
fn open(ui: &AppWindow) {
    let Some(root) = with_store(|s| s.active().map(|d| d.root.clone())) else {
        return;
    };
    SESSION.take();
    let d = ui.global::<ReviewDecisionsData>();
    d.set_open(true);
    d.set_rows(Rc::new(VecModel::<DecisionRow>::default()).into());
    d.set_selected(-1);
    d.set_drafts(0);
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
                }
            }
            Err(e) => set_state(ui, FAILED, &e),
        },
    );
}

/// The rows and the draft count of the session.
fn push(ui: &AppWindow) {
    let d = ui.global::<ReviewDecisionsData>();
    let rows = SESSION.with_borrow(|s| {
        let Some(s) = s else { return Vec::new() };
        let index = |k| KINDS.iter().position(|&x| x == k).unwrap_or(0) as i32;
        s.report
            .findings
            .iter()
            .enumerate()
            .map(|(i, f)| {
                let decision = s.decision(i);
                DecisionRow {
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

/// Write the decision document where the user chooses; headless runs
/// write it to the suggested path.
fn export(ui: &AppWindow) {
    let Some(suggested) = SESSION.with_borrow(|s| s.as_ref().map(Session::export_path)) else {
        return;
    };
    let dir = suggested.parent().map(std::path::Path::to_path_buf);
    if let Some(dir) = &dir {
        let _ = std::fs::create_dir_all(dir);
    }
    let path = if crate::context::headless_run() {
        Some(suggested.clone())
    } else {
        let mut dialog = rfd::FileDialog::new().set_file_name("review-decisions-draft.json");
        if let Some(dir) = dir {
            dialog = dialog.set_directory(dir);
        }
        dialog.save_file()
    };
    let Some(path) = path else { return };
    let result = SESSION.with_borrow(|s| s.as_ref().map(|s| s.export(&path)));
    match result {
        Some(Ok(())) => set_state(ui, EXPORTED, &path.display().to_string()),
        Some(Err(e)) => set_state(ui, FAILED, &e),
        None => {}
    }
}

/// `ARCHGUARD_DESKTOP_REVIEW=<row>`: open View → Review decisions… on the
/// active document and select that finding once the review is generated
/// (wait with `ARCHGUARD_DESKTOP_SNAPSHOT_MS`).
#[cfg(feature = "snapshot")]
pub fn snapshot_hook(ui: &AppWindow) {
    if let Some(row) = std::env::var("ARCHGUARD_DESKTOP_REVIEW")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        PENDING.set(Some(row));
        open(ui);
    }
}
