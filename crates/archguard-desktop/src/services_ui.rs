//! Services tab: the web viewer's service topology (`renderServices` in
//! `viewer_services.js`). The topology comes from
//! `archguard_engine::analyze_services`, as in the CLI's report.
//!
//! All tab state is one [`ServicesState`], built by [`build`] and shown by
//! [`push`] as one `ServicesModel` value.

mod layout;
mod paths;
mod state;
mod view;

use slint::ComponentHandle;

use crate::context::Loaded;
use crate::documents;
use crate::graph_ui::tab_support::{Actions, TabState, index, snapshot_mode};
use crate::ui::{AppWindow, ServicesData, ServicesModel};
use state::KINDS;
pub use state::{Mode, ServicesState};

/// Build the tab state for an analysis (reads the service topology).
pub fn build(data: &Loaded) -> ServicesState {
    let mut state = state::build_from(data.root);
    snapshot_overrides(&mut state);
    state
}

/// Register the tab's callbacks once at start-up.
pub fn wire(ui: &AppWindow) {
    let d = ui.global::<ServicesData>();
    let a = Actions::<ServicesState>::new(ui);
    d.on_select_kind(a.on(|s, _, i: i32| {
        s.kind = index(i).unwrap_or(0).min(KINDS.len() - 1);
        s.clear_focus();
    }));
    d.on_select_mode(a.on(|s, _, i: i32| {
        s.mode =
            [Mode::All, Mode::Incoming, Mode::Outgoing, Mode::Both][index(i).unwrap_or(0).min(3)];
    }));
    d.on_select_routing(a.on(|s, _, i: i32| s.in_out = i == 1));
    d.on_toggle_cycles_only(a.on0(|s, _| {
        s.cycles_only = !s.cycles_only;
        s.cycle = None;
    }));
    d.on_show_all(a.on0(|s, _| s.show_all()));
    d.on_close_details(a.on0(|s, _| s.details_hidden = true));
    d.on_show_more_projects(a.on0(|s, _| s.project_limit += 100));
    d.on_select_service(a.on(|s, _, i: i32| {
        if let Some(i) = index(i) {
            s.select_service(i)
        }
    }));
    d.on_select_project(a.on(|s, _, i: i32| {
        if let Some(i) = index(i) {
            s.select_project(i)
        }
    }));
    d.on_select_edge(a.on(|s, _, i: i32| {
        if let Some(i) = index(i) {
            s.select_edge(i)
        }
    }));
    d.on_select_cycle(a.on(|s, _, i: i32| {
        if let Some(i) = index(i) {
            s.select_cycle(i)
        }
    }));
    d.on_zoom_by(a.on(|s, _, f: f32| s.zoom_by(f)));
    d.on_fit(a.on2(|s, ui, w: f32, h: f32| {
        view::fit(s, w, h, &ui.global::<ServicesData>().get_label_unmapped());
    }));
    d.on_resized(a.on(|s, _, w: f32| s.viewport = w.floor().max(0.0)));
}

impl TabState for ServicesState {
    fn push(&mut self, ui: &AppWindow) {
        push(ui, self);
    }
}

/// Fill the tab after an analysis finished. The topology is read on a
/// worker thread; snapshot runs build it in place so the PNG shows it.
pub fn show(ui: &AppWindow, data: &Loaded) {
    // A document keeps its state; switching back only shows it again.
    if documents::with_active_state(|s: &mut ServicesState| push(ui, s)).is_some() {
        return;
    }
    let Some(doc) = documents::active_id() else {
        return;
    };
    ui.global::<ServicesData>()
        .set_model(ServicesModel::default());
    if snapshot_mode() {
        return install(ui, doc, build(data));
    }
    let root = data.root.to_path_buf();
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let state = state::build_from(&root);
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = weak.upgrade() {
                install(&ui, doc, state);
            }
        });
    });
}

/// Store the state for `doc`; show it only if `doc` is still active.
fn install(ui: &AppWindow, doc: documents::DocId, mut state: ServicesState) {
    if state.viewport == 0.0 {
        let window = ui.window();
        state.viewport = (window.size().width as f32 / window.scale_factor()).floor();
    }
    if documents::set_state(doc, state) {
        documents::with_active_state(|s: &mut ServicesState| push(ui, s));
    }
}

/// Show the state in the window.
pub fn push(ui: &AppWindow, s: &ServicesState) {
    let d = ui.global::<ServicesData>();
    d.set_model(view::build(s, &d.get_label_unmapped()));
}

/// `ARCHGUARD_DESKTOP_SVC_KIND=<index>`, `ARCHGUARD_DESKTOP_SVC_SERVICE=<name>`
/// and `ARCHGUARD_DESKTOP_SVC_ROUTING=inout` set the view for snapshot runs.
#[cfg(feature = "snapshot")]
fn snapshot_overrides(s: &mut ServicesState) {
    if let Some(k) = std::env::var("ARCHGUARD_DESKTOP_SVC_KIND")
        .ok()
        .and_then(|k| k.parse::<usize>().ok())
    {
        s.kind = k.min(KINDS.len() - 1);
    }
    if let Ok(name) = std::env::var("ARCHGUARD_DESKTOP_SVC_SERVICE") {
        let i = s
            .topology()
            .and_then(|t| t.services.iter().position(|x| x.name == name));
        if let Some(i) = i {
            s.select_service(i);
        }
    }
    s.in_out = std::env::var("ARCHGUARD_DESKTOP_SVC_ROUTING").is_ok_and(|r| r == "inout");
}

#[cfg(not(feature = "snapshot"))]
fn snapshot_overrides(_: &mut ServicesState) {}
