//! Elasticsearch tab: the graph of services, indices and the other search objects
//! (`archguard_engine::search_graph`) as the Services tree's rows and arcs. The
//! topology is the Services tab's, read once per document.

mod class;
mod index_page;
#[cfg(test)]
mod index_page_check;
mod page_view;
mod state;
mod view;
#[cfg(test)]
mod view_check;

use slint::ComponentHandle;

use crate::context::Loaded;
use crate::tab_support::{self, Actions, Shared, TabState, Topology, index};
use crate::ui::{AppWindow, EsClassData, EsClassModel};
use class::types::{Group, Switch};
pub use state::EsState;

pub fn wire(ui: &AppWindow) {
    let d = ui.global::<EsClassData>();
    let a = Actions::<EsState>::new(ui);
    d.on_select_row(a.on(|s, _, i: i32| s.pick_row(index(i).unwrap_or(usize::MAX))));
    d.on_select_box(a.on2(|s, _, i: i32, y: f32| s.pick_box(index(i).unwrap_or(usize::MAX), y)));
    d.on_search(a.on(|s, _, q: slint::SharedString| s.set_query(&q)));
    d.on_choose_kind(a.on(|s, _, i: i32| s.set_group(Group::from_index(index(i).unwrap_or(0)))));
    d.on_toggle_cluster(a.on0(|s, _| s.toggle_cluster()));
    d.on_switch(a.on2(|s, _, i: i32, on: bool| {
        if let Some(switch) = index(i).and_then(Switch::from_index) {
            s.set_switch(switch, on);
        }
    }));
    d.on_zoom_by(a.on(|s, _, f: f32| s.zoom_by(f)));
    d.on_fit(a.on2(|s, _, w: f32, h: f32| class::view::fit(s, w, h)));
    d.on_open(a.on2(|s, _, file: slint::SharedString, line: i32| {
        s.open_usage(&file, line.max(0) as u32);
    }));
}

impl TabState for EsState {
    fn push(&mut self, ui: &AppWindow) {
        ui.global::<EsClassData>()
            .set_model(class::view::build(self));
    }

    fn pending(ui: &AppWindow, error: Option<&str>) {
        let state = if error.is_some() { 2 } else { 0 };
        ui.global::<EsClassData>().set_model(EsClassModel {
            state,
            ..Default::default()
        });
    }
}

/// Show the active document's tab; the topology is the Services tab's.
pub fn show(ui: &AppWindow, data: &Loaded) {
    tab_support::show_async(ui, || {
        let (root, slot) = (data.root.to_path_buf(), Shared::<Topology>::active());
        let set = crate::documents_set::active_set();
        move || {
            let mut s = state::build_from(tab_support::topology(&slot, &root, set.as_deref()));
            s.root = root.clone();
            snapshot_overrides(&mut s);
            s
        }
    });
}

/// Snapshot runs: `ARCHGUARD_DESKTOP_ES_FOCUS=<label>` selects the row with that label (an
/// analysis block as `<kind>:<name>`, e.g. `analyzer:german_custom`),
/// `ARCHGUARD_DESKTOP_ES_CLUSTER=1` shows the whole cluster and
/// `ARCHGUARD_DESKTOP_ES_KIND=<group>` filters the list (`all`, `services`, `indices`, `aliases`,
/// `templates`, `analysis`, `processing`).
#[cfg(feature = "snapshot")]
fn snapshot_overrides(s: &mut EsState) {
    class::view::snapshot(s);
}

#[cfg(not(feature = "snapshot"))]
fn snapshot_overrides(_: &mut EsState) {}
