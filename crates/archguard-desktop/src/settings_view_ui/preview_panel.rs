//! The preview: `archguard check` with the saved and the edited
//! configuration (`config_editor::preview`), on a worker thread, and its
//! result as the panel shows it.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use archguard_engine::core::{Finding, Severity};
use slint::ComponentHandle;

use crate::config_editor::FieldError;
use crate::config_editor::preview::{self, CheckSummary, Preview, PreviewRequest};
use crate::documents::DocId;
use crate::tab_support::{model, snapshot_mode};
use crate::ui::{AppWindow, SettingsFinding, SettingsPreview};

/// Findings listed per side; the rest is counted.
pub const LISTED: usize = 6;

#[derive(Debug, Clone, PartialEq, Default)]
pub enum PreviewStatus {
    #[default]
    Idle,
    /// Waiting for the worker started as run `n`.
    Running(u64),
    Done(Box<Preview>),
    Failed(String),
}

type Outcome = Result<Preview, FieldError>;

thread_local! {
    /// Results that arrived per document, with their run number.
    static ARRIVED: RefCell<HashMap<DocId, (u64, Outcome)>> = RefCell::new(HashMap::new());
    static RUNS: Cell<u64> = const { Cell::new(0) };
}

/// Start a run of `request` for `doc` and return its status. `done`
/// runs on the UI thread when the result arrived; snapshot runs wait
/// for it instead and do not call `done`.
pub fn start(
    ui: &AppWindow,
    doc: DocId,
    request: PreviewRequest,
    done: fn(&AppWindow),
) -> PreviewStatus {
    let n = RUNS.get() + 1;
    RUNS.set(n);
    if snapshot_mode() {
        arrive(doc, n, preview::run(&request));
        return PreviewStatus::Running(n);
    }
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let outcome = preview::run(&request);
        let _ = slint::invoke_from_event_loop(move || {
            arrive(doc, n, outcome);
            if let Some(ui) = weak.upgrade() {
                done(&ui);
            }
        });
    });
    PreviewStatus::Running(n)
}

fn arrive(doc: DocId, n: u64, outcome: Outcome) {
    ARRIVED.with_borrow_mut(|a| a.insert(doc, (n, outcome)));
}

/// `status` with the result of its run for `doc`, if it arrived.
pub fn collect(doc: DocId, status: &mut PreviewStatus) {
    let PreviewStatus::Running(n) = *status else {
        return;
    };
    let Some((run, outcome)) = ARRIVED.with_borrow_mut(|a| a.remove(&doc)) else {
        return;
    };
    if run == n {
        *status = settle(outcome);
    }
}

fn settle(outcome: Outcome) -> PreviewStatus {
    match outcome {
        Ok(p) => PreviewStatus::Done(Box::new(p)),
        Err(e) => PreviewStatus::Failed(e.to_string()),
    }
}

/// The panel's data.
pub fn view(status: &PreviewStatus) -> SettingsPreview {
    match status {
        PreviewStatus::Idle => SettingsPreview::default(),
        PreviewStatus::Running(_) => SettingsPreview {
            state: 1,
            ..Default::default()
        },
        PreviewStatus::Failed(e) => SettingsPreview {
            state: 3,
            error: e.as_str().into(),
            ..Default::default()
        },
        PreviewStatus::Done(p) => done(p),
    }
}

fn done(p: &Preview) -> SettingsPreview {
    let (before, before_error) = match &p.before {
        Ok(s) => (counts(s), String::new()),
        Err(e) => ((false, 0, 0, 0), e.clone()),
    };
    let after = counts(&p.after);
    let more = |list: &[Finding]| list.len().saturating_sub(LISTED) as i32;
    SettingsPreview {
        state: 2,
        error: Default::default(),
        before_error: before_error.into(),
        before_pass: before.0,
        before_errors: before.1,
        before_warnings: before.2,
        before_infos: before.3,
        after_pass: after.0,
        after_errors: after.1,
        after_warnings: after.2,
        after_infos: after.3,
        changed: p.changed(),
        added: findings(&p.added),
        removed: findings(&p.removed),
        added_more: more(&p.added),
        removed_more: more(&p.removed),
    }
}

fn counts(s: &CheckSummary) -> (bool, i32, i32, i32) {
    (s.pass, s.errors as i32, s.warnings as i32, s.infos as i32)
}

fn findings(list: &[Finding]) -> slint::ModelRc<SettingsFinding> {
    model(list.iter().take(LISTED).map(|f| {
        SettingsFinding {
            severity: match f.severity {
                Severity::Error => "error",
                Severity::Warning => "warning",
                Severity::Info => "info",
            }
            .into(),
            id: f.id.as_str().into(),
            text: f
                .message
                .clone()
                .unwrap_or_else(|| f.nodes.join(", "))
                .into(),
        }
    }))
}
