//! Elasticsearch tab: the graph of services, indices and the other search objects
//! (`archguard_engine::search_graph`) as the Services tree's rows and arcs. The
//! topology is the Services tab's, read once per document.

mod state;
mod view;
#[cfg(test)]
mod view_check;

use slint::ComponentHandle;

use crate::context::Loaded;
use crate::tab_support::{self, Actions, Shared, TabState, Topology, index};
use crate::ui::{AppWindow, ElasticsearchData, EsModel};
pub use state::EsState;

pub fn wire(ui: &AppWindow) {
    let d = ui.global::<ElasticsearchData>();
    let a = Actions::<EsState>::new(ui);
    d.on_toggle_kind(a.on(|s, _, i: i32| s.toggle_kind(index(i).unwrap_or(usize::MAX))));
    d.on_focus(a.on(|s, _, i: i32| s.focus_on(index(i).unwrap_or(usize::MAX))));
    d.on_clear_focus(a.on0(|s, _| s.focus = None));
    d.on_zoom_by(a.on(|s, _, f: f32| s.zoom_by(f)));
    d.on_fit(a.on2(|s, _, w: f32, h: f32| view::fit(s, w, h)));
    d.on_resized(a.on(|s, _, w: f32| s.viewport = w.floor().max(0.0)));
}

impl TabState for EsState {
    fn push(&mut self, ui: &AppWindow) {
        ui.global::<ElasticsearchData>()
            .set_model(view::build(self));
    }

    fn pending(ui: &AppWindow, error: Option<&str>) {
        let state = if error.is_some() { 2 } else { 0 };
        ui.global::<ElasticsearchData>().set_model(EsModel {
            state,
            ..Default::default()
        });
    }

    /// The view may not have reported its width yet.
    fn arrive(&mut self, ui: &AppWindow) {
        if self.viewport == 0.0 {
            let window = ui.window();
            self.viewport = (window.size().width as f32 / window.scale_factor()).floor();
        }
    }
}

/// Show the active document's tab; the topology is the Services tab's.
pub fn show(ui: &AppWindow, data: &Loaded) {
    tab_support::show_async(ui, || {
        let (root, slot) = (data.root.to_path_buf(), Shared::<Topology>::active());
        let set = crate::documents_set::active_set();
        move || {
            let mut s = state::build_from(tab_support::topology(&slot, &root, set.as_deref()));
            snapshot_overrides(&mut s);
            s
        }
    });
}

/// `ARCHGUARD_DESKTOP_ES_KINDS=index,alias,ingest_pipeline` shows exactly these kinds,
/// `ARCHGUARD_DESKTOP_ES_FOCUS=<label>` focuses the node with that label (snapshot runs).
#[cfg(feature = "snapshot")]
fn snapshot_overrides(s: &mut EsState) {
    if let Ok(list) = std::env::var("ARCHGUARD_DESKTOP_ES_KINDS") {
        let want: Vec<&str> = list.split(',').collect();
        for (i, kind) in s.kinds().into_iter().enumerate() {
            if s.kind_shown(&kind) != want.contains(&kind.as_str()) {
                s.toggle_kind(i);
            }
        }
    }
    if let Ok(label) = std::env::var("ARCHGUARD_DESKTOP_ES_FOCUS")
        && let Some(i) = s.graph.nodes.iter().position(|n| n.label == label)
    {
        s.focus_on(i);
    }
}

#[cfg(not(feature = "snapshot"))]
fn snapshot_overrides(_: &mut EsState) {}
