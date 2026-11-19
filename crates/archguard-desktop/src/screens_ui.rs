//! Screens tab (ADR 0021): screens and navigations per application from
//! `archguard_engine::ui_flow::analyze`, each screen's reach for the
//! simulated user (`screen_reaches`), and the mismatch findings.
//!
//! All tab state is one [`ScreensState`]; [`push`] shows it as one
//! `ScreensModel` value.

mod inspector;
mod layout;
mod state;
mod view;
pub mod words;

use slint::ComponentHandle;

use crate::context::Loaded;
use crate::tab_support::{self, Actions, TabState, Texts, index};
use crate::ui::{AppWindow, ScreensData, ScreensModel};
pub use state::{ScreensState, atoms_of, role_assignment};
pub use view::role_rows;

/// Register the tab's callbacks once at start-up.
pub fn wire(ui: &AppWindow) {
    let d = ui.global::<ScreensData>();
    let a = Actions::<ScreensState>::new(ui);
    d.on_select_role(a.on(|s, _, i: i32| s.select_role(index(i).unwrap_or(0))));
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
    d.on_open_file(open_file(ui));
}

/// A callback that opens a project-relative file in a File view.
pub fn open_file(ui: &AppWindow) -> impl Fn(slint::SharedString) + 'static {
    tab_support::with_ui(ui, |ui, file: slint::SharedString| {
        if !file.is_empty() {
            crate::panes_ui::open_node(ui, crate::panes::FILE, &file, false);
        }
    })
}

impl TabState for ScreensState {
    fn push(&mut self, ui: &AppWindow) {
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
        move || {
            let flow = archguard_engine::ui_flow::analyze(&root).map_err(|e| e.to_string());
            let mut s = state::build_from(flow);
            snapshot_overrides(&mut s);
            s
        }
    });
}

pub fn push(ui: &AppWindow, s: &ScreensState) {
    let m = view::build(s, &Texts::of(ui));
    ui.global::<ScreensData>().set_model(m);
}

/// `ARCHGUARD_DESKTOP_FLOW_ROLE=<index>` (0 anonymous, 1 signed in, then
/// the roles) and `ARCHGUARD_DESKTOP_FLOW_SCREEN=<name>` set the view for
/// snapshot runs.
#[cfg(feature = "snapshot")]
fn snapshot_overrides(s: &mut ScreensState) {
    if let Some(r) = std::env::var("ARCHGUARD_DESKTOP_FLOW_ROLE")
        .ok()
        .and_then(|r| r.parse().ok())
    {
        s.select_role(r);
    }
    if let Ok(name) = std::env::var("ARCHGUARD_DESKTOP_FLOW_SCREEN") {
        let i = (s.flow()).and_then(|f| f.screens.iter().position(|x| x.name == name));
        if let Some(i) = i {
            s.select_screen(i);
        }
    }
}

#[cfg(not(feature = "snapshot"))]
fn snapshot_overrides(_: &mut ScreensState) {}
