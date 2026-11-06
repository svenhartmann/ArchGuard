//! The callbacks of the services World's filters and actions
//! (`repositoryWorldControls`), and its snapshot hooks.

use slint::ComponentHandle;

use super::follow_place::navigation;
use super::services::{KINDS, ServicesWorld};
use super::services_view::{DIRECTIONS, SOURCES};
use super::state::{Mode, StartState};
use crate::tab_support::{Actions, index};
use crate::ui::{AppWindow, StartData};

/// A picker of the filter row; whether the shown islands changed.
fn set_filter(w: &mut ServicesWorld, which: i32, v: i32) -> bool {
    let i = index(v).unwrap_or(0);
    match which {
        0 => {
            let d = index(v - 1).and_then(|i| w.scopes.get(i).cloned());
            w.set_islands(d, w.filter.all_projects);
            return true;
        }
        1 => w.filter.source = SOURCES[i.min(SOURCES.len() - 1)],
        2 => w.filter.kind = index(v - 1).and_then(|i| KINDS.get(i).copied()),
        _ => w.filter.direction = DIRECTIONS[i.min(DIRECTIONS.len() - 1)],
    }
    false
}

/// A toggle of the filter row; whether the shown islands changed. All
/// projects shows every deployment variant, as the web.
fn toggle(w: &mut ServicesWorld, which: i32) -> bool {
    let f = &mut w.filter;
    match which {
        0 => f.cycles_only = !f.cycles_only,
        1 => f.flow = !f.flow,
        2 => {
            let all = !f.all_projects;
            let deployment = if all { None } else { f.deployment.clone() };
            w.set_islands(deployment, all);
            return true;
        }
        _ => w.selected = None,
    }
    false
}

/// Register the services World's callbacks.
pub fn wire(ui: &AppWindow, d: &StartData<'_>) {
    let a = Actions::<StartState>::new(ui);
    d.on_set_mode(navigation(
        ui,
        a.on(|s, _, mode: i32| {
            s.set_mode(if mode == 1 {
                Mode::Services
            } else {
                Mode::Architecture
            })
        }),
    ));
    d.on_services_filter(
        a.on2(|s, _, which: i32, v: i32| s.with_services(|w| set_filter(w, which, v))),
    );
    d.on_services_toggle(a.on(|s, _, which: i32| s.with_services(|w| toggle(w, which))));
    d.on_enter_area(navigation(
        ui,
        a.on(|s, _, area: i32| match (s.mode, index(area)) {
            (Mode::Services, Some(a)) => s.enter_area(a),
            (Mode::Architecture, Some(a)) => s.open_area(a),
            _ => {}
        }),
    ));
    let zoom = navigation(
        ui,
        a.on(|s, _, ()| {
            if let Some(i) = s.services.as_ref().and_then(|w| w.selected) {
                s.enter_island(i);
            }
        }),
    );
    d.on_zoom_into(move || zoom(()));
    // As the Projects menu, outside the tab state's borrow.
    let weak = ui.as_weak();
    d.on_open_project(move || {
        let unit = crate::documents::with_active_state(|s: &mut StartState| s.project_unit());
        if let (Some(ui), Some(Some(unit))) = (weak.upgrade(), unit) {
            ui.invoke_open_sub_project(unit as i32);
        }
    });
}

/// `ARCHGUARD_DESKTOP_START_MODE=services`, the filters
/// `ARCHGUARD_DESKTOP_ISLAND_FILTER=<deployment>:<source>:<kind>:<direction>`
/// (picker indices, empty keeps), `ARCHGUARD_DESKTOP_ISLAND_FLAGS=cycles,flow,all`,
/// a selection `ARCHGUARD_DESKTOP_ISLAND=<name or path>` (with
/// `ARCHGUARD_DESKTOP_ISLAND_DETAILS=1` its details) and zooming into it
/// (`ARCHGUARD_DESKTOP_ISLAND_ENTER=1`) for snapshot runs.
#[cfg(feature = "snapshot")]
pub fn snapshot_overrides(s: &mut StartState) {
    let var = |k: &str| std::env::var(k).ok();
    if var("ARCHGUARD_DESKTOP_START_MODE").as_deref() == Some("services") {
        s.set_mode(Mode::Services);
    }
    if let Some(spec) = var("ARCHGUARD_DESKTOP_ISLAND_FILTER") {
        for (which, v) in spec.split(':').enumerate() {
            if let Ok(v) = v.parse::<i32>() {
                s.with_services(|w| set_filter(w, which as i32, v));
            }
        }
    }
    let flags = var("ARCHGUARD_DESKTOP_ISLAND_FLAGS").unwrap_or_default();
    for (which, name) in ["cycles", "flow", "all"].iter().enumerate() {
        if flags.split(',').any(|f| f == *name) {
            s.with_services(|w| toggle(w, which as i32));
        }
    }
    let Some(name) = var("ARCHGUARD_DESKTOP_ISLAND") else {
        return;
    };
    let island = s.services.as_ref().and_then(|w| {
        w.inventory
            .iter()
            .position(|i| i.path == name || i.label() == name)
    });
    if let Some(i) = island {
        s.pick_entity(i);
        s.details_open = var("ARCHGUARD_DESKTOP_ISLAND_DETAILS").as_deref() == Some("1");
        if var("ARCHGUARD_DESKTOP_ISLAND_ENTER").as_deref() == Some("1") {
            s.enter_island(i);
        }
    }
}
