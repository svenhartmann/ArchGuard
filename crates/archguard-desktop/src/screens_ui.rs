//! Screens tab (ADR 0021): screens and navigations per application from
//! `archguard_engine::ui_flow::analyze`, each screen's reach for the
//! simulated user (`screen_reaches`), and the mismatch findings.
//!
//! All tab state is one [`ScreensState`]; [`push`] shows it as one
//! `ScreensModel` value.

mod card;
mod center;
pub mod code;
mod drill;
mod focus;
pub mod inspector;
mod layout;
pub mod notes;
mod state;
#[cfg(test)]
mod tests;
mod tree;
mod view;
pub mod words;

use slint::ComponentHandle;

use crate::context::Loaded;
use crate::tab_support::{self, Actions, Flow, TabState, Texts, index};
use crate::ui::{AppWindow, ScreensData, ScreensModel, ScreensNote};
pub use state::{ScreensState, role_assignment, role_names, share_role, shared_role};
pub use view::role_picker;

/// Register the tab's callbacks once at start-up.
pub fn wire(ui: &AppWindow) {
    let d = ui.global::<ScreensData>();
    let a = Actions::<ScreensState>::new(ui);
    d.on_select_role(a.on(|s, _, i: i32| {
        let i = index(i).unwrap_or(0);
        s.select_role(i);
        state::share_role(i);
    }));
    d.on_set_switch(a.on2(|s, _, i: i32, v: i32| {
        if let Some(i) = index(i) {
            s.set_switch(i, v)
        }
    }));
    d.on_select_screen(a.on(|s, _, i: i32| {
        if let Some(i) = index(i) {
            s.select_screen(i)
        }
    }));
    d.on_open_list(a.on(|s, _, i: i32| {
        if let Some(i) = index(i) {
            s.open_list(i)
        }
    }));
    d.on_open_group(a.on(|s, _, i: i32| {
        if let Some(i) = index(i) {
            s.open_group(i)
        }
    }));
    d.on_toggle_band(a.on(|s, _, i: i32| {
        if let Some(i) = index(i) {
            s.toggle_band(i)
        }
    }));
    d.on_close_details(a.on0(|s, _| s.close_details()));
    d.on_set_details_open(a.on(|s, _, open: bool| s.details_open = open));
    d.on_close_code(a.on0(|s, _| s.code.close()));
    d.on_select_app(a.on(|s, _, i: i32| s.select_app(index(i).unwrap_or(0))));
    d.on_set_unknown(a.on(|s, _, i: i32| s.set_unknown(index(i).unwrap_or(0))));
    d.on_set_mode(a.on(|s, _, i: i32| s.set_mode(index(i).unwrap_or(0))));
    d.on_set_filter(a.on(|s, _, i: i32| s.set_filter(index(i).unwrap_or(0))));
    d.on_search(a.on(|s, _, t: slint::SharedString| s.search = t.to_string()));
    d.on_toggle_group(a.on(|s, _, k: slint::SharedString| s.toggle_group(&k)));
    d.on_go_back(a.on0(|s, _| s.go_back()));
    d.on_open_mismatches(a.on0(|s, _| s.open_mismatches()));
    d.on_open_browser(a.on(|s, _, url: slint::SharedString| {
        s.browser_note = crate::web::open_in_browser(&url).err().unwrap_or_default();
    }));
    wire_details(ui);
}

/// The details' jumps, drill-down, notes and the editor setting.
fn wire_details(ui: &AppWindow) {
    let d = ui.global::<ScreensData>();
    let a = Actions::<ScreensState>::new(ui);
    d.on_show_line(
        a.on2(|s, _, file: slint::SharedString, line: i32| s.show_code(&file, line.max(0) as u32)),
    );
    d.on_open_editor(a.on2(|s, _, file: slint::SharedString, line: i32| {
        let r = crate::editor::open(&s.root, &file, line.max(0) as u32);
        s.problem = r.err().unwrap_or_default();
    }));
    d.on_drill(a.on(|s, _, i: i32| {
        if let Some(i) = index(i) {
            s.open_drill(i)
        }
    }));
    d.on_open_note(a.on2(
        |s, ui, id: slint::SharedString, what: slint::SharedString| {
            s.open_note(&id, &what);
            fill_form(ui, s.notes.get(id.as_str()));
        },
    ));
    d.on_save_note(a.on0(|s, ui| {
        s.problem = save_note(s, &form_of(ui))
            .err()
            .map(|e| Texts::of(ui).t(&e).to_string())
            .unwrap_or_default();
    }));
    d.on_show_calls(tab_support::with_ui(ui, |ui, name: slint::SharedString| {
        crate::calls_ui::show_name(ui, &name);
    }));
    d.on_set_editor(a.on2(|_, _, preset: i32, custom: slint::SharedString| {
        let t = index(preset)
            .and_then(|i| crate::editor::PRESETS.get(i))
            .map_or(custom.as_str(), |(_, t)| t);
        crate::editor::set_template(t);
    }));
}

/// Record the open note form's item with `form` and read the notes again.
fn save_note(s: &mut ScreensState, form: &notes::Form) -> Result<(), String> {
    let Some((id, _)) = s.note_for.clone() else {
        return Ok(());
    };
    s.notes = notes::save_here(&s.root, s.config.as_ref(), &id, form)?;
    s.note_for = None;
    s.list = s.list.or(Some(state::List::Gaps));
    Ok(())
}

/// The note form's fields as the window holds them.
pub fn form_of(ui: &AppWindow) -> notes::Form {
    let n = ui.global::<ScreensNote>();
    notes::Form {
        kind: index(n.get_kind()).unwrap_or(0),
        text: n.get_text().into(),
        change: n.get_change().into(),
        adr: n.get_adr().into(),
    }
}

/// Fill the note form with the stored note of `stored`, or empty it.
pub fn fill_form(ui: &AppWindow, stored: Option<&archguard_engine::core::ReviewDecision>) {
    let f = notes::form(stored);
    let n = ui.global::<ScreensNote>();
    n.set_kind(f.kind as i32);
    n.set_text(f.text.into());
    n.set_change(f.change.into());
    n.set_adr(f.adr.into());
}

impl TabState for ScreensState {
    fn push(&mut self, ui: &AppWindow) {
        // The Domain tab may have changed the simulated user.
        if let Some(r) = state::shared_role() {
            self.select_role(r);
        }
        push(ui, self);
    }

    fn pending(ui: &AppWindow, error: Option<&str>) {
        let model = match error {
            None => ScreensModel::default(),
            Some(e) => ScreensModel {
                state: 2,
                error: e.into(),
                ..Default::default()
            },
        };
        ui.global::<ScreensData>().set_model(model);
    }
}

/// Show the active document's tab; the flow is read once per document, on
/// a worker thread.
pub fn show(ui: &AppWindow, data: &Loaded) {
    tab_support::show_async(ui, || {
        let root = data.root.to_path_buf();
        let flow = Flow::of(data);
        move || load(root, &flow)
    });
}

/// The tab's state; the flow comes from the document's shared slot.
pub(crate) fn load(root: std::path::PathBuf, flow: &Flow) -> ScreensState {
    let mut s = state::build_from((*flow.get()).clone());
    s.config = archguard_engine::load_config(None, &root)
        .ok()
        .map(|l| l.config);
    s.domain = archguard_engine::domain_model::analyze(&root).ok();
    s.notes = notes::load(&root);
    s.root = root;
    snapshot_overrides(&mut s);
    s
}

pub fn push(ui: &AppWindow, s: &ScreensState) {
    let m = view::build(s, &Texts::of(ui));
    ui.global::<ScreensData>().set_model(m);
}

/// `ARCHGUARD_DESKTOP_FLOW_ROLE=<index>` (0 anonymous, 1 signed in, then
/// the roles), `ARCHGUARD_DESKTOP_FLOW_APP=<name>`,
/// `ARCHGUARD_DESKTOP_FLOW_SCREEN=<name>` or `…_FLOW_ROUTE=<route>` (in the
/// shown application), `ARCHGUARD_DESKTOP_FLOW_UNKNOWN=<0 open, 1 all
/// hold, 2 none hold>`, `ARCHGUARD_DESKTOP_FLOW_MODE=<0 focus, 1 two
/// steps, 2 whole graph>`, `ARCHGUARD_DESKTOP_FLOW_FILTER=<0 all, 1
/// denied, 2 no path>` and `ARCHGUARD_DESKTOP_FLOW_SEARCH=<text>` set the
/// view for snapshot runs.
#[cfg(feature = "snapshot")]
fn snapshot_overrides(s: &mut ScreensState) {
    if let Some(r) = std::env::var("ARCHGUARD_DESKTOP_FLOW_ROLE")
        .ok()
        .and_then(|r| r.parse().ok())
    {
        s.select_role(r);
    }
    let var = |k: &str| std::env::var(format!("ARCHGUARD_DESKTOP_FLOW_{k}")).ok();
    let num = |k: &str| var(k).and_then(|v| v.parse::<usize>().ok());
    if let Some(name) = var("APP") {
        let apps = s.flow().map(state::applications).unwrap_or_default();
        if let Some(i) = apps.iter().position(|a| *a == name) {
            s.select_app(i);
        }
    }
    let app = s.app.clone();
    let pick = |f: &dyn Fn(&archguard_engine::core::Screen) -> bool| {
        let of_app = |x: &&archguard_engine::core::Screen| {
            x.application.as_deref().unwrap_or("") == app && f(x)
        };
        (s.flow()?.screens.iter()).position(|x| of_app(&x))
    };
    let screen = (var("SCREEN").and_then(|n| pick(&|x| x.name == n)))
        .or_else(|| var("ROUTE").and_then(|r| pick(&|x| x.route.as_deref() == Some(&r))));
    if let Some(i) = screen {
        s.select_screen(i);
    }
    if let Some(i) = num("UNKNOWN") {
        s.set_unknown(i);
    }
    if let Some(i) = num("MODE") {
        s.set_mode(i);
    }
    if let Some(i) = num("FILTER") {
        s.set_filter(i);
    }
    if let Some(q) = var("SEARCH") {
        s.search = q;
    }
    snapshot_details(s);
}

#[cfg(feature = "snapshot")]
fn snapshot_details(s: &mut ScreensState) {
    let var = |k: &str| std::env::var(format!("ARCHGUARD_DESKTOP_FLOW_{k}")).ok();
    let num = |k: &str| var(k).and_then(|v| v.parse::<usize>().ok());
    // `…_FLOW_LIST=<0 gaps, 1, 2, 3 undeclared>` opens a list,
    // `…_FLOW_NOTE=<row>[|<0 fix, 1 intentional, 2 later>|<text>]` opens
    // the note form of that row of it, with kind and text records the note
    // (as Save does), `…_FLOW_DRILL=<trigger>[|<call
    // text>]` the drill-down of the first action with that trigger whose
    // call contains the text,
    // `…_FLOW_CODE=<file>:<line>` the code panel.
    if let Some(i) = num("LIST") {
        s.open_list(i);
    }
    if let Some(spec) = var("NOTE") {
        snapshot_note(s, &spec);
    }
    if let Some(spec) = var("DRILL") {
        let (t, call) = spec.split_once('|').unwrap_or((&spec, ""));
        let at = (s.sim.actions.iter())
            .position(|a| a.trigger.as_deref() == Some(t) && words::call(a).contains(call));
        if let Some(a) = at {
            s.open_drill(a);
        }
    }
    if let Some((file, line)) = var("CODE").as_deref().and_then(|c| c.rsplit_once(':')) {
        s.show_code(file, line.parse().unwrap_or(1));
    }
}

#[cfg(feature = "snapshot")]
fn snapshot_note(s: &mut ScreensState, spec: &str) {
    let mut parts = spec.splitn(3, '|');
    let rows = view::build(s, &Texts::default()).inspector.rows;
    use slint::Model;
    let row = parts.next().and_then(|i| i.parse().ok());
    let Some(r) = row.and_then(|i| rows.row_data(i)) else {
        return;
    };
    s.open_note(&r.id, &r.text);
    let (Some(kind), Some(text)) = (parts.next(), parts.next()) else {
        return;
    };
    let form = notes::Form {
        kind: kind.parse().unwrap_or(0),
        text: text.into(),
        ..Default::default()
    };
    if let Err(e) = save_note(s, &form) {
        eprintln!("snapshot note: {e}");
    }
}

#[cfg(not(feature = "snapshot"))]
fn snapshot_overrides(_: &mut ScreensState) {}
