//! Switching between the architecture and the services World, and what
//! clicks, pins and pickers do in each (`repositorySelectWorld`,
//! `repositoryOpenWorld`, `repositoryTapWorld` in the web).

use super::scene::{self, Camera};
use super::services::ServicesWorld;
use super::state::{Mode, Selection, StartState};
use crate::tab_support::index;

impl StartState {
    pub(super) fn back(&mut self) {
        if self.history.is_empty() && self.entered {
            self.set_mode(Mode::Services);
        }
        if let Some(v) = self.history.pop() {
            self.within = v.within;
            self.cam = v.cam;
            self.selection = v.selection;
            self.limit = v.limit;
            self.refresh();
        }
    }

    /// Wheel zoom at the cursor. At the maximum, zooming on over a package
    /// opens it; at the minimum, zooming out returns to the previous view.
    pub(super) fn wheel(&mut self, x: f64, y: f64, factor: f64) {
        let under = self.cell_at(x, y).map(|c| self.layout.cells[c].area);
        let point = (x - self.canvas.0 / 2.0, y - self.canvas.1 / 2.0);
        self.cam = scene::zoom(self.cam, factor, point);
        let services = self.mode == Mode::Services;
        let drill = under.filter(|&a| {
            factor > 1.0 && self.cam.zoom >= 8.0 && (services || self.scope.areas[a].module)
        });
        let out = factor < 1.0 && self.cam.zoom <= 0.4 && !services;
        if let Some(area) = drill {
            if services {
                self.enter_area(area);
            } else {
                self.open_area(area);
            }
        } else if out && (self.within.is_some() || self.entered) {
            self.back();
        }
    }

    pub(super) fn services_mode(&self) -> bool {
        self.mode == Mode::Services && self.services.is_some()
    }

    /// Show the other World with a fitted camera; the architecture World
    /// keeps its place.
    pub(super) fn set_mode(&mut self, mode: Mode) {
        if mode == Mode::Services && self.services.is_none() {
            return;
        }
        if mode == Mode::Services {
            self.history.clear();
        }
        self.mode = mode;
        self.entered = false;
        self.cam = Camera {
            rotation: self.cam.rotation,
            ..Camera::default()
        };
        self.refresh();
    }

    /// Change the services World's filters; `islands` when the shown
    /// islands change, which fits the camera again (as the web).
    pub(super) fn with_services(&mut self, f: impl FnOnce(&mut ServicesWorld) -> bool) {
        let Some(w) = &mut self.services else { return };
        let islands = f(w);
        w.refresh();
        if islands {
            self.cam = Camera {
                rotation: self.cam.rotation,
                ..Camera::default()
            };
        }
        self.refresh();
    }

    /// "Zoom into service": the architecture World of the island's source
    /// project, as a package of this document's analysis (the root
    /// project: the whole overview). Back returns to the services. An
    /// island without local sources shows its details instead.
    pub(super) fn enter_island(&mut self, island: usize) {
        let Some(w) = &mut self.services else { return };
        let Some(source) = w.inventory.get(island).map(|i| i.source.clone()) else {
            return;
        };
        let Some(source) = source else {
            if w.area(island).is_some() {
                w.selected = Some(island);
                self.details_open = true;
            }
            return;
        };
        let module = |name: &str| {
            self.ev
                .graph
                .nodes
                .iter()
                .any(|n| n.name == name && n.kind == archguard_engine::core::NodeKind::Module)
        };
        self.within = Some(source).filter(|p| module(p));
        self.set_mode(Mode::Architecture);
        self.entered = true;
        self.selection = None;
        self.limit = 20;
        self.details_open = false;
    }

    pub(super) fn enter_area(&mut self, area: usize) {
        if let Some(island) = self.services.as_ref().and_then(|w| w.shown.get(area)) {
            self.enter_island(*island);
        }
    }

    /// The selected island's source project, to open as its own document:
    /// its index among the document's build units (the header's Projects
    /// menu), so the root project, this document, has none.
    pub(super) fn project_unit(&self) -> Option<usize> {
        let w = self.services.as_ref()?;
        let source = w.inventory[w.selected?].source.as_deref()?;
        crate::documents::build_units(w.topology())
            .iter()
            .position(|p| p.path == source)
    }

    /// A click on the canvas: a surface selects its island or entity.
    pub(super) fn click_at(&mut self, x: f64, y: f64) {
        let Some(c) = self.cell_at(x, y) else { return };
        if self.services_mode() {
            let area = self.layout.cells[c].area;
            self.with_services(|w| {
                w.select_area(Some(area));
                false
            });
        } else {
            self.select_cell(c);
        }
    }

    /// A double click opens a package, or zooms into a service.
    pub(super) fn double_at(&mut self, x: f64, y: f64) {
        let Some(c) = self.cell_at(x, y) else { return };
        let area = self.layout.cells[c].area;
        if self.services_mode() {
            self.enter_area(area);
        } else {
            self.open_area(area);
        }
    }

    /// A pin: selects an island, opens a package.
    pub(super) fn pin(&mut self, area: usize) {
        if self.services_mode() {
            self.with_services(|w| {
                w.select_area(Some(area));
                false
            });
        } else {
            self.open_area(area);
        }
    }

    /// The picker below the World: entry 0 is its placeholder. Services:
    /// "Open project…" over the whole inventory.
    pub(super) fn pick(&mut self, i: i32) {
        if self.services_mode() {
            if let Some(island) = index(i - 1) {
                self.enter_island(island);
            }
            return;
        }
        self.selection = index(i - 1)
            .filter(|&a| a < self.scope.areas.len())
            .map(|area| Selection {
                area,
                entity: None,
                members: None,
            });
    }

    /// A member of the details: an entity, or a neighbouring island.
    pub(super) fn pick_entity(&mut self, e: usize) {
        if self.services_mode() {
            self.with_services(|w| {
                if w.area(e).is_some() {
                    w.selected = Some(e);
                }
                false
            });
            return;
        }
        if let Some(sel) = &mut self.selection {
            sel.entity = Some(e);
            self.limit = 20;
        }
    }

    pub(super) fn has_selection(&self) -> bool {
        if self.services_mode() {
            self.services.as_ref().is_some_and(|w| w.selected.is_some())
        } else {
            self.selection.is_some()
        }
    }

    /// `world-reset` on the services World: no selection, no search.
    pub(super) fn reset_services(&mut self) {
        self.set_search(String::new());
        self.details_open = false;
        self.cam = Camera::default();
        self.with_services(|w| {
            w.select_area(None);
            false
        });
    }
}
