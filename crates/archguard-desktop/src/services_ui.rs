//! Services tab: the web viewer's service topology (`renderServices` in
//! `viewer_services.js`). The topology comes from
//! `archguard_engine::analyze_services`, as in the CLI's report.
//!
//! All tab state is one [`ServicesState`], built by [`build_from`] and shown by
//! [`push`] as one `ServicesModel` value.

mod layout;
mod paths;
mod state;
mod view;

use slint::ComponentHandle;

use crate::context::Loaded;
use crate::tab_support::{self, Actions, Shared, TabState, Topology, index};
use crate::ui::{AppWindow, ServicesData, ServicesModel};
use state::KINDS;
pub use state::{Mode, ServicesState};

/// Build the tab state from the document's topology.
fn build_from(topology: Topology) -> ServicesState {
    let mut state = state::build_from(topology);
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

    fn pending(ui: &AppWindow, error: Option<&str>) {
        let model = match error {
            None => ServicesModel::default(),
            Some(e) => ServicesModel {
                state: 2,
                error: e.into(),
                ..Default::default()
            },
        };
        ui.global::<ServicesData>().set_model(model);
    }

    /// The view may not have reported its width yet.
    fn arrive(&mut self, ui: &AppWindow) {
        if self.viewport == 0.0 {
            let window = ui.window();
            self.viewport = (window.size().width as f32 / window.scale_factor()).floor();
        }
    }
}

/// Show the active document's tab; its topology is read once per
/// document, on a worker thread.
pub fn show(ui: &AppWindow, data: &Loaded) {
    tab_support::show_async(ui, || {
        let (root, slot) = (data.root.to_path_buf(), Shared::<Topology>::active());
        move || build_from(tab_support::topology(&slot, &root))
    });
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
