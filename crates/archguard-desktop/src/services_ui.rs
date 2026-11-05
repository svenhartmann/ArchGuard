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

use std::cell::RefCell;

use slint::ComponentHandle;

use crate::context::Loaded;
use crate::ui::{AppWindow, ServicesData, ServicesModel};
use state::KINDS;
pub use state::{Mode, ServicesState};

thread_local! {
    static STATE: RefCell<Option<ServicesState>> = const { RefCell::new(None) };
}

/// Build the tab state for an analysis (reads the service topology).
pub fn build(data: &Loaded) -> ServicesState {
    let mut state = state::build_from(data.root);
    snapshot_overrides(&mut state);
    state
}

/// Register the tab's callbacks once at start-up.
pub fn wire(ui: &AppWindow) {
    let d = ui.global::<ServicesData>();
    d.on_select_kind(action(ui, |s, _, i: i32| {
        s.kind = usize::try_from(i).unwrap_or(0).min(KINDS.len() - 1);
        s.clear_focus();
    }));
    d.on_select_mode(action(ui, |s, _, i: i32| {
        s.mode = [Mode::All, Mode::Incoming, Mode::Outgoing, Mode::Both]
            [usize::try_from(i).unwrap_or(0).min(3)];
    }));
    d.on_select_routing(action(ui, |s, _, i: i32| s.in_out = i == 1));
    let toggle = action(ui, |s, _, ()| {
        s.cycles_only = !s.cycles_only;
        s.cycle = None;
    });
    d.on_toggle_cycles_only(move || toggle(()));
    let all = action(ui, |s, _, ()| s.show_all());
    d.on_show_all(move || all(()));
    let close = action(ui, |s, _, ()| s.details_hidden = true);
    d.on_close_details(move || close(()));
    let more = action(ui, |s, _, ()| s.project_limit += 100);
    d.on_show_more_projects(move || more(()));
    d.on_select_service(action(ui, |s, _, i: i32| {
        if let Ok(i) = usize::try_from(i) {
            s.select_service(i);
        }
    }));
    d.on_select_project(action(ui, |s, _, i: i32| {
        if let Ok(i) = usize::try_from(i) {
            s.select_project(i);
        }
    }));
    d.on_select_edge(action(ui, |s, _, i: i32| {
        if let Ok(i) = usize::try_from(i) {
            s.select_edge(i);
        }
    }));
    d.on_select_cycle(action(ui, |s, _, i: i32| {
        if let Ok(i) = usize::try_from(i) {
            s.select_cycle(i);
        }
    }));
    d.on_zoom_by(action(ui, |s, _, f: f32| s.zoom_by(f)));
    d.on_fit(action2(ui, |s, ui, w: f32, h: f32| {
        view::fit(s, w, h, &ui.global::<ServicesData>().get_label_unmapped());
    }));
    d.on_resized(action(ui, |s, _, w: f32| s.viewport = w.floor().max(0.0)));
}

/// A callback that runs `f` on the tab state and refreshes the window.
fn action<A: 'static>(
    ui: &AppWindow,
    f: impl Fn(&mut ServicesState, &AppWindow, A) + 'static,
) -> impl Fn(A) + 'static {
    let w = ui.as_weak();
    move |arg| {
        let Some(ui) = w.upgrade() else { return };
        STATE.with_borrow_mut(|s| {
            if let Some(s) = s {
                f(s, &ui, arg);
                push(&ui, s);
            }
        });
    }
}

fn action2<A: 'static, B: 'static>(
    ui: &AppWindow,
    f: impl Fn(&mut ServicesState, &AppWindow, A, B) + 'static,
) -> impl Fn(A, B) + 'static {
    let g = action(ui, move |s, ui, (a, b)| f(s, ui, a, b));
    move |a, b| g((a, b))
}

/// Fill the tab after an analysis finished. The topology is read on a
/// worker thread; snapshot runs build it in place so the PNG shows it.
pub fn show(ui: &AppWindow, data: &Loaded) {
    ui.global::<ServicesData>()
        .set_model(ServicesModel::default());
    if std::env::var_os("ARCHGUARD_DESKTOP_SNAPSHOT").is_some() {
        return install(ui, build(data));
    }
    let root = data.root.to_path_buf();
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let state = state::build_from(&root);
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = weak.upgrade() {
                install(&ui, state);
            }
        });
    });
}

fn install(ui: &AppWindow, mut state: ServicesState) {
    if state.viewport == 0.0 {
        let window = ui.window();
        state.viewport = (window.size().width as f32 / window.scale_factor()).floor();
    }
    push(ui, &state);
    STATE.set(Some(state));
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
