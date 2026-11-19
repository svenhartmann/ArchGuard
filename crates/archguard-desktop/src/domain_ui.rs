//! Domain tab (ADR 0022): entities by aggregate from
//! `archguard_engine::domain_model::analyze`, the selected entity's fields
//! and state machine, and the domain events.

mod chain;
#[cfg(test)]
mod chain_tests;
mod entities;
mod state;
mod states;
mod view;

use slint::ComponentHandle;

use crate::context::Loaded;
use crate::tab_support::{self, Actions, TabState, Texts, index};
use crate::ui::{AppWindow, DomainData, DomainViewModel};
use state::{DomainState, build_from};

pub fn wire(ui: &AppWindow) {
    let d = ui.global::<DomainData>();
    let a = Actions::<DomainState>::new(ui);
    d.on_select_entity(a.on(|s, _, i: i32| {
        if let Some(i) = index(i) {
            s.select_entity(i)
        }
    }));
    d.on_close_details(a.on0(|s, _| s.selected = None));
    d.on_select_state(a.on(|s, _, name: slint::SharedString| s.select_state(&name)));
    d.on_select_role(a.on(|s, _, i: i32| s.select_role(index(i).unwrap_or(0))));
    d.on_open_file(crate::screens_ui::open_file(ui));
}

impl TabState for DomainState {
    fn push(&mut self, ui: &AppWindow) {
        let m = view::build(self, &Texts::of(ui));
        ui.global::<DomainData>().set_model(m);
    }

    fn pending(ui: &AppWindow, error: Option<&str>) {
        let model = match error {
            None => DomainViewModel::default(),
            Some(e) => DomainViewModel {
                state: 2,
                error: e.into(),
                ..Default::default()
            },
        };
        ui.global::<DomainData>().set_model(model);
    }
}

/// Show the active document's tab; the model and the UI flow (for the
/// chains) are read once per document, on a worker thread.
pub fn show(ui: &AppWindow, data: &Loaded) {
    tab_support::show_async(ui, || {
        let root = data.root.to_path_buf();
        move || {
            let model = archguard_engine::domain_model::analyze(&root).map_err(|e| e.to_string());
            let flow = archguard_engine::ui_flow::analyze(&root).map_err(|e| e.to_string());
            let mut s = build_from(model, flow);
            snapshot_overrides(&mut s);
            s
        }
    });
}

/// `ARCHGUARD_DESKTOP_DOMAIN_ENTITY=<name>` selects an entity,
/// `ARCHGUARD_DESKTOP_DOMAIN_STATE=<state>` one of its states and
/// `ARCHGUARD_DESKTOP_FLOW_ROLE=<index>` the simulated user for snapshot runs.
#[cfg(feature = "snapshot")]
fn snapshot_overrides(s: &mut DomainState) {
    if let Some(r) = std::env::var("ARCHGUARD_DESKTOP_FLOW_ROLE")
        .ok()
        .and_then(|r| r.parse().ok())
    {
        s.select_role(r);
    }
    let Ok(name) = std::env::var("ARCHGUARD_DESKTOP_DOMAIN_ENTITY") else {
        return;
    };
    let i = (*s.model)
        .as_ref()
        .ok()
        .and_then(|d| d.entities.iter().position(|e| e.name == name));
    if let Some(i) = i {
        s.select_entity(i);
    }
    if let Ok(state) = std::env::var("ARCHGUARD_DESKTOP_DOMAIN_STATE") {
        s.select_state(&state);
    }
}

#[cfg(not(feature = "snapshot"))]
fn snapshot_overrides(_: &mut DomainState) {}
