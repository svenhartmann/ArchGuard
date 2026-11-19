//! The Graph panels every Graph of the window shares
//! (`ui/graph_panels.slint`): shown as the settings' `[graph]` say, saved
//! when a drag at a panel edge ends.

use slint::ComponentHandle;

use crate::settings::GraphPanels;
use crate::settings_file;
use crate::ui::{AppWindow, GraphPanels as Panels};

/// Show the stored widths (after the settings are loaded).
pub fn apply(ui: &AppWindow) {
    let g = settings_file::get(|s| s.graph);
    let p = ui.global::<Panels>();
    p.set_explorer_w(g.explorer_width as f32);
    p.set_details_w(g.details_width as f32);
}

pub fn wire(ui: &AppWindow) {
    let weak = ui.as_weak();
    ui.global::<Panels>().on_changed(move || {
        let Some(ui) = weak.upgrade() else { return };
        let p = ui.global::<Panels>();
        let graph = GraphPanels {
            explorer_width: p.get_explorer_w().round() as u32,
            details_width: p.get_details_w().round() as u32,
        };
        settings_file::update(|s| s.graph = graph);
    });
}
