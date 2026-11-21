//! File → Compare architecture… in the window (ui/compare_dialog.slint,
//! ui/compare_view.slint) over the UI-free [`crate::compare`]. The dialog
//! lists the document's local branches; the comparison runs on a worker
//! thread, and closing the dialog cancels it (the engine stops at its next
//! step and a late result is dropped). The result is kept per document and
//! shown in the document's Compare tab, narrowed by a search text and by
//! collapsed packages.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use slint::{ComponentHandle, SharedString};

use crate::compare::diff::{Comparison, Cycle, Relation, Status};
use crate::compare::filter;
use crate::compare::git::{self, Target};
use crate::documents::{self, DocId, with_store};
use crate::panes::{COMPARE, Layout};
use crate::panes_ui;
use crate::tab_support::{index, model, snapshot_mode, with_ui};
use crate::ui::{AppWindow, CompareCounts, CompareData, CompareRelation, CompareRow};

/// Evidence lines listed per relation; the rest is counted.
const EVIDENCE: usize = 5;

const RUNNING: i32 = 1;
const FAILED: i32 = 2;

type Outcome = Result<Comparison, String>;

thread_local! {
    /// The number of the comparison the dialog waits for; closing the
    /// dialog moves it on, so a late result is dropped.
    static RUN: Cell<u64> = const { Cell::new(0) };
    /// The running comparison's cancel flag, set when the dialog closes.
    static CANCEL: RefCell<Arc<AtomicBool>> = RefCell::new(Arc::new(AtomicBool::new(false)));
    /// The dialog's document, its root and the branches it lists.
    static DIALOG: RefCell<Option<(DocId, PathBuf, Vec<String>)>> = const { RefCell::new(None) };
}

/// A document's comparison and its selected row.
struct CompareState {
    result: Comparison,
    /// Index into `result.rows`.
    selected: Option<usize>,
    query: String,
    collapsed: BTreeSet<String>,
    /// `result.rows` indices the list shows, in order.
    shown: Vec<usize>,
}

impl CompareState {
    fn new(result: Comparison) -> Self {
        let mut s = CompareState {
            result,
            selected: None,
            query: String::new(),
            collapsed: BTreeSet::new(),
            shown: Vec::new(),
        };
        s.refilter();
        s
    }

    fn refilter(&mut self) {
        self.shown = filter::visible(&self.result.rows, &self.query, &self.collapsed);
    }
}

pub fn wire(ui: &AppWindow) {
    let d = ui.global::<CompareData>();
    let f = with_ui(ui, |ui, ()| show(ui));
    d.on_show(move || f(()));
    let f = with_ui(ui, |ui, ()| run(ui));
    d.on_run(move || f(()));
    let f = with_ui(ui, |ui, ()| close(ui));
    d.on_close(move || f(()));
    d.on_select(with_ui(ui, |ui, i: i32| {
        documents::with_active_state(|s: &mut CompareState| {
            s.selected = index(i).and_then(|i| s.shown.get(i).copied());
        });
        push(ui);
    }));
    d.on_query(with_ui(ui, |ui, t: SharedString| {
        change(ui, |s| s.query = t.to_string());
    }));
    d.on_toggle(with_ui(ui, |ui, i: i32| {
        change(ui, |s| {
            let Some(name) = index(i)
                .and_then(|i| s.shown.get(i))
                .map(|&r| s.result.rows[r].name.clone())
            else {
                return;
            };
            if !s.collapsed.remove(&name) {
                s.collapsed.insert(name);
            }
        });
    }));
    d.on_collapse_all(with_ui(ui, |ui, collapse: bool| {
        change(ui, |s| {
            s.collapsed = if collapse {
                filter::groups(&s.result.rows)
            } else {
                BTreeSet::new()
            };
        });
    }));
    panes_ui::on_sync(COMPARE, sync);
}

/// Change the active document's filter or collapse state and show it.
fn change(ui: &AppWindow, f: impl FnOnce(&mut CompareState)) {
    documents::with_active_state(|s: &mut CompareState| {
        f(s);
        s.refilter();
    });
    push(ui);
}

/// Open the dialog on the active document with its local branches.
fn show(ui: &AppWindow) {
    let Some((doc, root)) = with_store(|s| s.active().map(|d| (d.id, d.root.clone()))) else {
        return;
    };
    let d = ui.global::<CompareData>();
    let (refs, error) = match git::refs(&root) {
        Ok(r) => (r, String::new()),
        Err(e) => (git::preselect(Vec::new(), None), e),
    };
    let mut targets: Vec<SharedString> = refs.branches.iter().map(Into::into).collect();
    targets.push(d.invoke_index_label());
    d.set_branches(model(refs.branches.iter().map(SharedString::from)));
    d.set_targets(model(targets));
    d.set_from_index(refs.from as i32);
    d.set_to_index(refs.to.unwrap_or(refs.branches.len()) as i32);
    d.set_state(if error.is_empty() { 0 } else { FAILED });
    d.set_error(error.into());
    d.set_dialog(true);
    DIALOG.set(Some((doc, root, refs.branches)));
}

/// Start the comparison of the picked pair.
fn run(ui: &AppWindow) {
    let d = ui.global::<CompareData>();
    let Some((doc, root, from, to)) = DIALOG.with_borrow(|dialog| {
        let (doc, root, branches) = dialog.as_ref()?;
        let from = branches.get(index(d.get_from_index())?)?.clone();
        let to = match branches.get(index(d.get_to_index())?) {
            Some(b) => Target::Ref(b.clone()),
            None => Target::Staged,
        };
        Some((*doc, root.clone(), from, to))
    }) else {
        return;
    };
    let n = RUN.get() + 1;
    RUN.set(n);
    CANCEL.with_borrow_mut(|c| {
        c.store(true, Ordering::Relaxed);
        *c = Arc::new(AtomicBool::new(false));
    });
    let cancel = CANCEL.with_borrow(Arc::clone);
    d.set_state(RUNNING);
    if snapshot_mode() {
        return arrive(ui, doc, n, git::run(&root, &from, &to, &cancel));
    }
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let outcome = git::run(&root, &from, &to, &cancel);
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = weak.upgrade() {
                arrive(&ui, doc, n, outcome);
            }
        });
    });
}

/// Close the dialog; a running comparison stops at the engine's next step
/// and a result that still arrives is dropped.
fn close(ui: &AppWindow) {
    RUN.set(RUN.get() + 1);
    CANCEL.with_borrow(|c| c.store(true, Ordering::Relaxed));
    let d = ui.global::<CompareData>();
    d.set_dialog(false);
    d.set_state(0);
    DIALOG.take();
}

/// The result of run `n` for `doc`: shown in the document's Compare tab,
/// unless the dialog was closed or another document became active.
fn arrive(ui: &AppWindow, doc: DocId, n: u64, outcome: Outcome) {
    if RUN.get() != n {
        return;
    }
    let d = ui.global::<CompareData>();
    let result = match outcome {
        Ok(c) if documents::active_id() == Some(doc) => c,
        Ok(_) => return close(ui),
        Err(e) => {
            d.set_state(FAILED);
            d.set_error(e.into());
            return;
        }
    };
    let title = format!("{} → {}", result.from, label(&result.to, ui));
    documents::forget_active::<CompareState>();
    documents::with_active_state_or(|| CompareState::new(result), |_| ());
    close(ui);
    ui.invoke_open_view_tab(COMPARE.into());
    if let Some(id) = panes_ui::shown_id(COMPARE) {
        panes_ui::describe(ui, id, Some(title), 0);
    }
    push(ui);
}

fn label(to: &str, ui: &AppWindow) -> String {
    if to.is_empty() {
        ui.global::<CompareData>().invoke_index_label().into()
    } else {
        to.into()
    }
}

fn sync(ui: &AppWindow, layout: &Layout) -> Vec<u32> {
    if layout.find(COMPARE).is_some() {
        push(ui);
    }
    Vec::new()
}

/// Show the active document's comparison, or none.
fn push(ui: &AppWindow) {
    let d = ui.global::<CompareData>();
    let shown = documents::with_active_state(|s: &mut CompareState| {
        let c = &s.result;
        d.set_from(c.from.as_str().into());
        d.set_to(c.to.as_str().into());
        d.set_empty(c.empty);
        d.set_counts(counts(c));
        d.set_filter(s.query.as_str().into());
        d.set_groups(!filter::groups(&c.rows).is_empty());
        d.set_rows(model(s.shown.iter().map(|&i| {
            let r = &c.rows[i];
            CompareRow {
                name: r.name.as_str().into(),
                file: r.file,
                status: status(r.status),
                cycle: cycle(r.cycle),
                collapsible: !r.file && c.rows.get(i + 1).is_some_and(|n| n.file),
                collapsed: s.collapsed.contains(&r.name),
            }
        })));
        let selected = s.selected.filter(|&i| i < c.rows.len());
        d.set_selected(
            selected
                .and_then(|i| s.shown.iter().position(|&v| v == i))
                .map_or(-1, |p| p as i32),
        );
        let listed = selected.map(|i| c.relations_of(i)).unwrap_or_default();
        d.set_relations(model(listed.into_iter().map(relation)));
    });
    d.set_has_result(shown.is_some());
}

fn counts(c: &Comparison) -> CompareCounts {
    let n = c.counts();
    let i = |v: usize| v as i32;
    CompareCounts {
        packages_added: i(n.packages_added),
        packages_removed: i(n.packages_removed),
        added: i(n.added),
        removed: i(n.removed),
        changed: i(n.changed),
        cycles_new: i(n.cycles_new),
        cycles_removed: i(n.cycles_removed),
    }
}

fn relation(r: &Relation) -> CompareRelation {
    let lines: Vec<String> = r
        .evidence
        .iter()
        .take(EVIDENCE)
        .map(|e| format!("{}:{}  {}", e.file, e.line, e.raw.trim()))
        .collect();
    CompareRelation {
        from: r.from.as_str().into(),
        to: r.to.as_str().into(),
        status: status(r.status),
        cycle: cycle(r.cycle),
        evidence: lines.join("\n").into(),
        more: r.evidence.len().saturating_sub(EVIDENCE) as i32,
    }
}

fn status(s: Status) -> i32 {
    match s {
        Status::Added => 0,
        Status::Removed => 1,
        Status::Changed => 2,
        Status::Unchanged => 3,
    }
}

fn cycle(c: Cycle) -> i32 {
    match c {
        Cycle::None => 0,
        Cycle::Kept => 1,
        Cycle::New => 2,
        Cycle::Removed => 3,
    }
}

/// `ARCHGUARD_DESKTOP_COMPARE=<from>,<to>[,<row>[,<filter>]]`: File →
/// Compare architecture… on the active document with branch `from`
/// against branch `to` (`--staged`: the Git index), run in place, then
/// select the package or file named `row` (empty: none) and type `filter`
/// into the search field. Logs the dialog's state.
#[cfg(feature = "snapshot")]
pub fn snapshot_hook(ui: &AppWindow) {
    let Ok(spec) = std::env::var("ARCHGUARD_DESKTOP_COMPARE") else {
        return;
    };
    use slint::Model;
    let parts: Vec<&str> = spec.split(',').map(str::trim).collect();
    show(ui);
    let branches = DIALOG.with_borrow(|d| d.as_ref().map(|d| d.2.clone()).unwrap_or_default());
    let at = |name: &str| {
        branches
            .iter()
            .position(|b| b == name)
            .map_or(-1, |i| i as i32)
    };
    let d = ui.global::<CompareData>();
    d.set_from_index(at(parts.first().copied().unwrap_or("")));
    let to = parts.get(1).copied().unwrap_or("");
    d.set_to_index(if to == "--staged" {
        branches.len() as i32
    } else {
        at(to)
    });
    run(ui);
    if let Some(name) = parts.get(2) {
        let row = d.get_rows().iter().position(|r| r.name == *name);
        d.invoke_select(row.map_or(-1, |i| i as i32));
    }
    if let Some(text) = parts.get(3) {
        d.invoke_query((*text).into());
    }
    eprintln!(
        "compare: state {}, error {:?}, rows {}",
        d.get_state(),
        d.get_error(),
        d.get_rows().row_count()
    );
}
