//! Settings view (View → Settings): the active document's
//! `archguard.toml` as a form (ADR 0002 decision 8), built on the UI-free
//! core in [`crate::config_editor`].
//!
//! One [`SettingsState`] per document, built when its Settings tab is
//! first shown; it binds the editor to the document, so every edit marks
//! it dirty and the "Save changes?" prompt saves it. Text inputs are
//! debounced and flushed before any other action. Group names and field
//! descriptions come from the catalogue in English and are translated
//! through `ui/texts.slint` like other text Rust builds.

mod edits;
mod form;
mod preview_panel;
mod session;
#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::rc::Rc;

use slint::{ComponentHandle, SharedString};

use crate::config_editor::{FieldValue, Group};
use crate::deferred::{Coalesce, TYPING};
use crate::documents::{self, with_store};
use crate::model_sync::sync_rows;
use crate::panes::{Layout, SETTINGS};
use crate::panes_ui;
use crate::tab_support::{Actions, TabState, Texts, index};
use crate::ui::{AppWindow, SettingsData, SettingsEdit, SettingsModel};
use session::SettingsState;

type Typed = Coalesce<(SharedString, SharedString)>;

thread_local! {
    /// Field text waiting for a pause in typing, and the field it is for.
    static FIELD_TEXT: RefCell<Option<(Rc<Typed>, SharedString)>> = const { RefCell::new(None) };
    static PANE_TEXT: RefCell<Option<Rc<Coalesce<SharedString>>>> = const { RefCell::new(None) };
}

impl TabState for SettingsState {
    fn push(&mut self, ui: &AppWindow) {
        push(ui, self);
    }
}

/// Register the view's callbacks once at start-up.
pub fn wire(ui: &AppWindow) {
    let a = Actions::<SettingsState>::new(ui);
    wire_fields(ui, &a);
    let d = ui.global::<SettingsData>();
    let pane = Coalesce::debounce(TYPING, a.on(|s, _, t: SharedString| s.edit_text(&t)));
    PANE_TEXT.set(Some(pane.clone()));
    d.on_edit_text(pane.callback());
    let query = Coalesce::debounce(TYPING, a.on(|s, _, q: SharedString| s.query = q.into()));
    d.on_set_query(query.callback());
    d.on_select_group(flushed(a.on(|s, _, i: i32| {
        s.group = index(i)
            .and_then(|i| Group::ALL.get(i).copied())
            .unwrap_or(s.group);
        s.query.clear();
    })));
    let preview = flushed(a.on(|s, ui, ()| start_preview(ui, s)));
    d.on_preview(move || preview(()));
    let close = a.on0(|s, _| s.preview = Default::default());
    d.on_close_preview(close);
    let save = flushed(a.on(|s, _, ()| s.save()));
    d.on_save(move || save(()));
    let revert = flushed(a.on(|s, _, ()| s.reload()));
    d.on_revert(move || revert(()));
    let reload = flushed(a.on(|s, _, ()| s.reload()));
    d.on_reload(move || reload(()));
    let overwrite = flushed(a.on(|s, _, ()| s.overwrite()));
    d.on_overwrite(move || overwrite(()));
    let toggle = flushed(a.on(|s, _, ()| s.text_mode = !s.text_mode));
    d.on_toggle_text(move || toggle(()));
    panes_ui::on_sync(SETTINGS, sync);
}

/// The row callbacks of `SettingsEdit`.
fn wire_fields(ui: &AppWindow, a: &Actions<SettingsState>) {
    let e = ui.global::<SettingsEdit>();
    let typed = Coalesce::debounce(
        TYPING,
        a.on(|s, _, (p, t): (SharedString, SharedString)| s.edit(&p, &t)),
    );
    FIELD_TEXT.set(Some((typed, SharedString::new())));
    e.on_set_text(type_into);
    let f = flushed(a.on(|s, _, (p, b): (SharedString, bool)| s.set(&p, FieldValue::Bool(b))));
    e.on_set_bool(move |p, b| f((p, b)));
    let f = flushed(a.on(|s, _, (p, i): (SharedString, i32)| {
        if let Some(i) = index(i) {
            s.choose(&p, i);
        }
    }));
    e.on_set_option(move |p, i| f((p, i)));
    e.on_reset(flushed(a.on(|s, _, p: SharedString| s.reset(&p))));
    e.on_add_entry(flushed(a.on(|s, _, p: SharedString| s.add_entry(&p))));
    let f = flushed(a.on(|s, _, (p, i): (SharedString, i32)| {
        if let Some(i) = index(i) {
            s.remove_entry(&p, i);
        }
    }));
    e.on_remove_entry(move |p, i| f((p, i)));
    let f = flushed(a.on(|s, _, (p, from, to): (SharedString, i32, i32)| {
        if let (Some(from), Some(to)) = (index(from), index(to)) {
            s.move_entry(&p, from, to);
        }
    }));
    e.on_move_entry(move |p, from, to| f((p, from, to)));
}

/// Text typed into field `path`: applied after a pause, or at once when
/// another field gets text first.
fn type_into(path: SharedString, text: SharedString) {
    let Some(typed) = FIELD_TEXT.with_borrow_mut(|t| {
        let (typed, last) = t.as_mut()?;
        let other = *last != path;
        *last = path.clone();
        Some((typed.clone(), other))
    }) else {
        return;
    };
    if typed.1 {
        typed.0.flush();
    }
    typed.0.call((path, text));
}

/// Apply typed text that waits for its pause.
fn flush_typing() {
    if let Some((typed, _)) = FIELD_TEXT.with_borrow(Clone::clone) {
        typed.flush();
    }
    if let Some(pane) = PANE_TEXT.with_borrow(Clone::clone) {
        pane.flush();
    }
}

/// `f` after the typed text is applied, so Save and Preview see it.
fn flushed<A: 'static>(f: impl Fn(A) + 'static) -> impl Fn(A) + 'static {
    move |a| {
        flush_typing();
        f(a)
    }
}

fn start_preview(ui: &AppWindow, s: &mut SettingsState) {
    let Ok(rc) = &s.editor else { return };
    let request = rc.borrow().preview_request();
    s.preview = preview_panel::start(ui, s.doc, request, refresh);
}

/// Push the active document's state again (a preview arrived).
fn refresh(ui: &AppWindow) {
    documents::with_active_state(|s: &mut SettingsState| push(ui, s));
}

/// Show the active document's settings when its layout has the tab.
fn sync(ui: &AppWindow, layout: &Layout) -> Vec<u32> {
    if layout.find(SETTINGS).is_some() {
        show(ui);
    }
    Vec::new()
}

/// Build the active document's state once, then push it.
fn show(ui: &AppWindow) {
    let Some((doc, root, effective)) = with_store(|s| {
        let d = s.active()?;
        let config = d.analysis.as_ref().map(|a| a.run.config.clone());
        Some((d.id, d.root.clone(), config))
    }) else {
        return;
    };
    let build = || SettingsState::open(doc, root, effective.clone());
    documents::with_active_state_or(build, |s: &mut SettingsState| {
        if s.effective.is_none() {
            s.effective = effective.clone();
        }
        push(ui, s);
    });
}

fn push(ui: &AppWindow, s: &mut SettingsState) {
    s.texts = Texts::of(ui);
    preview_panel::collect(s.doc, &mut s.preview);
    let d = ui.global::<SettingsData>();
    let old = d.get_model();
    d.set_model(model(s, &old));
}

/// The view's data; list rows that did not change stay in `old`'s models.
fn model(s: &SettingsState, old: &SettingsModel) -> SettingsModel {
    let (items, placed) = s.items();
    let general_error = match &s.invalid {
        Some(e) if !placed => e.to_string(),
        _ => String::new(),
    };
    let (text, text_error) = match (&s.text_draft, &s.editor) {
        (Some(d), _) => (d.raw.clone(), d.error.clone().unwrap_or_default()),
        (None, Ok(rc)) => (rc.borrow().text(), String::new()),
        (None, Err(_)) => Default::default(),
    };
    SettingsModel {
        loaded: true,
        load_error: s.editor.as_ref().err().cloned().unwrap_or_default().into(),
        file: s.file().into(),
        groups: sync_rows(&old.groups, s.groups()),
        group: Group::ALL.iter().position(|g| *g == s.group).unwrap_or(0) as i32,
        query: s.query.as_str().into(),
        items: sync_rows(&old.items, items),
        dirty: s.is_dirty(),
        errors: s.errors() as i32,
        general_error: general_error.into(),
        text_mode: s.text_mode,
        text: text.into(),
        text_error: text_error.into(),
        changed_on_disk: s.changed_on_disk,
        save_error: s.save_error.as_str().into(),
        preview: preview_panel::view(&s.preview),
    }
}

/// `ARCHGUARD_DESKTOP_SETTINGS=<group>[,text][,preview]`: open View →
/// Settings on a group (its label or catalogue name, any case), in the
/// text pane, with a preview. `ARCHGUARD_DESKTOP_SETTINGS_EDIT=<edit>;…`
/// first edits a temp copy's configuration: `path=text` types into a
/// field, `+array` adds an entry, `-path` resets a field.
#[cfg(feature = "snapshot")]
pub fn snapshot_hook(ui: &AppWindow) {
    let Ok(spec) = std::env::var("ARCHGUARD_DESKTOP_SETTINGS") else {
        return;
    };
    ui.invoke_open_view_tab(SETTINGS.into());
    let edits = std::env::var("ARCHGUARD_DESKTOP_SETTINGS_EDIT").unwrap_or_default();
    crate::tab_support::update(ui, |s: &mut SettingsState, ui| {
        for edit in edits.split(';').filter(|e| !e.is_empty()) {
            snapshot_edit(s, edit);
        }
        for part in spec.split(',').map(str::trim) {
            match part {
                "text" => s.text_mode = true,
                "preview" => start_preview(ui, s),
                name => s.group = group_named(name).unwrap_or(s.group),
            }
        }
    });
    eprintln!(
        "settings: dirty {}, errors {}",
        documents::active_id().is_some_and(documents::is_dirty),
        documents::with_active_state(|s: &mut SettingsState| s.errors()).unwrap_or(0)
    );
}

#[cfg(feature = "snapshot")]
fn snapshot_edit(s: &mut SettingsState, edit: &str) {
    if let Some(array) = edit.strip_prefix('+') {
        s.add_entry(array);
    } else if let Some(path) = edit.strip_prefix('-') {
        s.reset(path);
    } else if let Some((path, text)) = edit.split_once('=') {
        s.edit(path, &text.replace("\\n", "\n"));
    }
}

#[cfg(feature = "snapshot")]
fn group_named(name: &str) -> Option<Group> {
    // `dependency_rules`, `Dependency rules`, `DependencyRules`.
    let key = |s: &str| s.to_lowercase().replace(['_', ' '], "");
    let name = key(name);
    Group::ALL
        .into_iter()
        .find(|g| key(g.label()) == name || key(&format!("{g:?}")) == name)
}
