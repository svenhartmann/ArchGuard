//! Screens tab (ADR 0021): screens and navigations per application from
//! `archguard_engine::ui_flow::analyze`, each screen's reach for the
//! simulated user (`screen_reaches`), and the mismatch findings.
//!
//! All tab state is one [`ScreensState`]; [`push`] shows it as one
//! `ScreensModel` value.

mod card;
mod center;
mod focus;
mod inspector;
mod layout;
mod state;
#[cfg(test)]
mod tests;
mod tree;
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
            s.config = archguard_engine::load_config(None, &root)
                .ok()
                .map(|l| l.config);
            s.root = root;
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
}

#[cfg(not(feature = "snapshot"))]
fn snapshot_overrides(_: &mut ScreensState) {}
