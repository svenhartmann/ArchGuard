//! The panel widths every view of the window shares
//! (`ui/graph_panels.slint`): the Graph's explorer and details panel, the
//! Screens and Domain views' tree and details panel. Shown as the
//! settings' `[graph]`, `[screens]` and `[domain]` say, saved when a drag
//! at a panel edge ends or a double click resets one.

use slint::ComponentHandle;

use crate::settings::{GraphPanels, SidePanels};
use crate::settings_file;
use crate::ui::{AppWindow, GraphPanels as Panels};

/// Show the stored widths (after the settings are loaded).
pub fn apply(ui: &AppWindow) {
    let (g, screens, domain) = settings_file::get(|s| (s.graph, s.screens, s.domain));
    let p = ui.global::<Panels>();
    p.set_explorer_w(g.explorer_width as f32);
    p.set_details_w(g.details_width as f32);
    show(&p, 0, screens);
    show(&p, 1, domain);
}

/// View 0 Screens, 1 Domain.
fn show(p: &Panels, view: i32, s: SidePanels) {
    let (tree, details) = (s.tree_width as f32, s.details_width as f32);
    if view == 0 {
        p.set_screens_tree_w(tree);
        p.set_screens_details_w(details);
    } else {
        p.set_domain_tree_w(tree);
        p.set_domain_details_w(details);
    }
}

fn shown(p: &Panels, view: i32) -> SidePanels {
    let (tree, details) = if view == 0 {
        (p.get_screens_tree_w(), p.get_screens_details_w())
    } else {
        (p.get_domain_tree_w(), p.get_domain_details_w())
    };
    SidePanels {
        tree_width: tree.round() as u32,
        details_width: details.round() as u32,
    }
}

fn save(p: &Panels) {
    let graph = GraphPanels {
        explorer_width: p.get_explorer_w().round() as u32,
        details_width: p.get_details_w().round() as u32,
    };
    let (screens, domain) = (shown(p, 0), shown(p, 1));
    settings_file::update(|s| {
        s.graph = graph;
        s.screens = screens;
        s.domain = domain;
    });
}

pub fn wire(ui: &AppWindow) {
    let p = ui.global::<Panels>();
    let weak = ui.as_weak();
    p.on_changed(move || {
        if let Some(ui) = weak.upgrade() {
            save(&ui.global::<Panels>());
        }
    });
    let weak = ui.as_weak();
    p.on_side_drag(move |view, details, w| {
        let Some(ui) = weak.upgrade() else { return };
        let p = ui.global::<Panels>();
        let mut s = shown(&p, view);
        s.set(details, w);
        show(&p, view, s);
    });
    let weak = ui.as_weak();
    p.on_side_reset(move |view, details| {
        let Some(ui) = weak.upgrade() else { return };
        let p = ui.global::<Panels>();
        let mut s = shown(&p, view);
        s.reset(details);
        show(&p, view, s);
        save(&p);
    });
}
