//! The tooltip of the surface under the pointer (`world-tip` in
//! `viewer_world.js`): its entity or "N grouped entries", the lens value,
//! and the roof outline the stage draws around it.

use slint::ComponentHandle;

use super::scene;
use super::state::{Lens, StartState};
use crate::format::Locale;
use crate::tab_support::Texts;
use crate::ui::{AppWindow, StartData, StartHover};

/// What the tooltip shows for one surface.
#[derive(Debug, Clone, PartialEq)]
pub struct Hover {
    pub cell: usize,
    /// The class or file, or the island's name; empty for a group.
    pub title: String,
    /// Members of a grouped surface (0 for a single one).
    pub grouped: usize,
    /// The class metric shown (1..=6), 0 complexity.
    pub kpi: usize,
    pub value: Option<f64>,
    /// A service island's kind (as `StartPin.svc`), 0 for a package area.
    pub svc: i32,
}

impl StartState {
    /// The surface at a canvas point and what its tooltip says.
    pub(super) fn hover_at(&self, x: f64, y: f64) -> Option<Hover> {
        let cell = self.cell_at(x, y)?;
        let c = &self.layout.cells[cell];
        if let (Some(w), true) = (&self.services, self.services_mode()) {
            let island = w.island(c.area);
            return Some(Hover {
                cell,
                title: island.label().to_string(),
                grouped: 0,
                kpi: 0,
                value: None,
                svc: super::services_view::svc(island),
            });
        }
        let grouped = if c.entities.len() > 1 {
            c.entities.len()
        } else {
            0
        };
        let title = match c.entities.first() {
            Some(&e) if grouped == 0 => {
                let e = self.ev.entity(e);
                e.class_id.clone().unwrap_or_else(|| e.node.clone())
            }
            _ => String::new(),
        };
        Some(Hover {
            cell,
            title,
            grouped,
            kpi: if self.lens == Lens::Explore {
                self.kpi
            } else {
                0
            },
            value: c.value,
            svc: 0,
        })
    }
}

/// Show the tooltip for the point `(x, y)` of the stage, or hide it.
pub fn show(ui: &AppWindow, s: &StartState, at: Option<(f64, f64)>) {
    let d = ui.global::<StartData>();
    let hover = at.and_then(|(x, y)| s.hover_at(x, y));
    let (Some(h), Some(v), Some((x, y))) = (hover, s.view(), at) else {
        if d.get_hover().shown {
            d.set_hover(StartHover::default());
        }
        return;
    };
    let l: Locale = Texts::of(ui).locale();
    // An island is outlined whole, a package area's surface alone.
    let area = s.layout.cells[h.cell].area;
    let cells = (0..s.layout.cells.len())
        .filter(|&i| i == h.cell || (h.svc > 0 && s.layout.cells[i].area == area));
    let outline = scene::outline(&s.layout, &v.frame, s.cam.rotation, cells);
    d.set_hover(StartHover {
        shown: true,
        x: x as f32,
        y: y as f32,
        title: h.title.into(),
        grouped: h.grouped as i32,
        kpi: h.kpi as i32,
        value: h.value.map(|v| l.num(v, 2)).unwrap_or_default().into(),
        svc: h.svc,
        outline: outline.into(),
    });
}
