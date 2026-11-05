//! The Start tab's state: navigation, lenses, selection and camera, as
//! `worldUi` in `viewer_world.js`. Views read it; they never write it.

use std::collections::HashMap;

use super::evidence::{self, Evidence, Scope};
use super::layout::{self, Layout, Region};
use super::scene::{self, Camera};

/// The three lenses of the web (`problems`, `complexity`, `coverage`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lens {
    Problems,
    Explore,
    Coverage,
}

/// A selected area, optionally one entity or the members of one surface.
#[derive(Debug, Clone, PartialEq)]
pub struct Selection {
    pub area: usize,
    pub entity: Option<usize>,
    pub members: Option<Vec<usize>>,
}

/// One entry of the Back history.
#[derive(Debug, Clone)]
pub(super) struct Visit {
    pub(super) within: Option<String>,
    pub(super) cam: Camera,
    pub(super) selection: Option<Selection>,
    pub(super) limit: usize,
}

/// Everything the tab knows: the evidence and the viewer's position.
pub struct StartState {
    pub(super) ev: Evidence,
    /// The scope of the overview (after skipping single-package chains).
    pub(super) home: Option<String>,
    pub(super) within: Option<String>,
    pub(super) history: Vec<Visit>,
    pub(super) lens: Lens,
    /// 0 complexity, 1..=6 the class metrics of [`evidence::KPIS`].
    pub(super) kpi: usize,
    pub(super) cam: Camera,
    pub(super) selection: Option<Selection>,
    /// The search field's text, as typed.
    pub(super) search: String,
    /// Lower-case search term (`search`, trimmed).
    pub(super) query: String,
    /// Canvas size in pixels, reported by the view.
    pub(super) canvas: (f64, f64),
    pub(super) details_open: bool,
    pub(super) limit: usize,
    // Derived from the above by `refresh`.
    pub(super) scope: Scope,
    pub(super) layout: Layout,
}

pub(super) fn from_evidence(ev: Evidence) -> StartState {
    let mut s = StartState {
        ev,
        home: None,
        within: None,
        history: Vec::new(),
        lens: Lens::Problems,
        kpi: 0,
        cam: Camera::default(),
        selection: None,
        search: String::new(),
        query: String::new(),
        canvas: (1200.0, 520.0),
        details_open: false,
        limit: 20,
        scope: Scope::default(),
        layout: Layout::default(),
    };
    s.refresh();
    s
}

impl StartState {
    /// Recompute the scope and the ground plan after a navigation or a lens
    /// change (`renderWorld`).
    pub(super) fn refresh(&mut self) {
        let scope = evidence::scope(&self.ev, self.within.as_deref());
        self.scope = match scope {
            Some(s) => s,
            None => {
                self.within = None;
                evidence::scope(&self.ev, None).unwrap_or_default()
            }
        };
        if self.within.is_none() {
            self.home = self.scope.name.clone();
        }
        self.layout = plan(self);
    }

    pub(super) fn kpi_active(&self) -> bool {
        self.lens == Lens::Explore && self.kpi > 0
    }

    /// `worldEntityValue`: the metric of the current lens for an entity.
    pub(super) fn value(&self, entity: usize) -> Option<f64> {
        if self.kpi_active() {
            self.ev.kpi(entity, self.kpi).0
        } else {
            self.ev.entity(entity).max_complexity.map(f64::from)
        }
    }

    /// `worldNavigate`: open a package scope, remembering the current view.
    pub(super) fn navigate(&mut self, within: Option<String>) {
        if within == self.within {
            return;
        }
        let is_module = |name: &str| {
            self.ev
                .graph
                .nodes
                .iter()
                .any(|n| n.name == name && n.kind == archguard_engine::core::NodeKind::Module)
        };
        if within.as_deref().is_some_and(|w| !is_module(w)) {
            return;
        }
        self.history.push(Visit {
            within: self.within.take(),
            cam: self.cam,
            selection: self.selection.take(),
            limit: self.limit,
        });
        self.within = within;
        self.limit = 20;
        self.cam = Camera {
            rotation: self.cam.rotation,
            ..Camera::default()
        };
        self.refresh();
    }

    pub(super) fn back(&mut self) {
        if let Some(v) = self.history.pop() {
            self.within = v.within;
            self.cam = v.cam;
            self.selection = v.selection;
            self.limit = v.limit;
            self.refresh();
        }
    }

    /// `worldOpenArea`: packages open, files show their details.
    pub(super) fn open_area(&mut self, area: usize) {
        let Some(a) = self.scope.areas.get(area) else {
            return;
        };
        if a.module {
            self.navigate(Some(a.name.clone()));
        } else {
            self.select(Selection {
                area,
                entity: None,
                members: None,
            });
        }
    }

    /// Select something; entities and files open the details as in the web.
    pub(super) fn select(&mut self, sel: Selection) {
        let module = self.scope.areas.get(sel.area).is_some_and(|a| a.module);
        if sel.entity.is_some() || !module {
            self.details_open = true;
        }
        self.selection = Some(sel);
        self.limit = 20;
    }

    /// Select a surface: its single entity, or all its members.
    pub(super) fn select_cell(&mut self, cell: usize) {
        let c = &self.layout.cells[cell];
        let sel = Selection {
            area: c.area,
            entity: (c.entities.len() == 1).then(|| c.entities[0]),
            members: (c.entities.len() > 1).then(|| c.entities.clone()),
        };
        self.select(sel);
    }

    pub(super) fn set_lens(&mut self, lens: Lens) {
        self.lens = lens;
        self.refresh();
    }

    pub(super) fn set_kpi(&mut self, kpi: usize) {
        self.kpi = kpi.min(evidence::KPIS.len());
        self.lens = Lens::Explore;
        self.refresh();
    }

    /// `world-reset`: overview, default lens and camera, no search.
    pub(super) fn reset(&mut self) {
        self.within = None;
        self.history.clear();
        self.selection = None;
        self.limit = 20;
        self.cam = Camera::default();
        self.lens = Lens::Problems;
        self.kpi = 0;
        self.set_search(String::new());
        self.details_open = false;
        self.refresh();
    }

    pub(super) fn set_search(&mut self, text: String) {
        self.query = text.trim().to_lowercase();
        self.search = text;
    }

    /// `worldMatches`: the class id or file name contains the term.
    pub(super) fn matches(&self, cell: &layout::Cell) -> bool {
        self.query.is_empty()
            || cell.entities.iter().any(|&e| {
                let e = self.ev.entity(e);
                e.class_id
                    .as_deref()
                    .unwrap_or(&e.node)
                    .to_lowercase()
                    .contains(&self.query)
            })
    }

    /// Enter in the search: select the first match by area name.
    pub(super) fn accept_search(&mut self) {
        let first = (0..self.layout.cells.len())
            .filter(|&i| self.matches(&self.layout.cells[i]))
            .min_by(|&a, &b| {
                let name = |i: usize| &self.scope.areas[self.layout.cells[i].area].name;
                name(a).cmp(name(b))
            });
        if let Some(i) = first {
            self.select_cell(i);
        }
    }

    pub(super) fn view(&self) -> Option<scene::View> {
        Some(scene::View {
            frame: scene::Frame::of(&self.layout)?,
            w: self.canvas.0,
            h: self.canvas.1,
            cam: self.cam,
        })
    }

    /// The surface below a canvas point, front-most first.
    pub(super) fn cell_at(&self, x: f64, y: f64) -> Option<usize> {
        let v = self.view()?;
        scene::hit(&self.layout, &v.frame, self.cam.rotation, v.to_unit((x, y)))
    }

    /// Wheel zoom at the cursor. At the maximum, zooming on over a package
    /// opens it; at the minimum, zooming out returns to the previous view.
    pub(super) fn wheel(&mut self, x: f64, y: f64, factor: f64) {
        let under = self.cell_at(x, y).map(|c| self.layout.cells[c].area);
        let point = (x - self.canvas.0 / 2.0, y - self.canvas.1 / 2.0);
        self.cam = scene::zoom(self.cam, factor, point);
        let drill =
            under.filter(|&a| factor > 1.0 && self.cam.zoom >= 8.0 && self.scope.areas[a].module);
        if let Some(area) = drill {
            self.open_area(area);
        } else if factor < 1.0 && self.cam.zoom <= 0.4 && self.within.is_some() {
            self.back();
        }
    }

    /// The package path from the overview to the current scope.
    pub(super) fn crumbs(&self, root: &str) -> Vec<(String, Option<String>)> {
        let index: HashMap<_, _> = self.ev.graph.nodes.iter().map(|n| (n.id, n)).collect();
        let mut chain = Vec::new();
        let mut node = self
            .ev
            .graph
            .nodes
            .iter()
            .find(|n| Some(&n.name) == self.scope.name.as_ref());
        while let Some(n) = node.filter(|n| Some(&n.name) != self.home.as_ref()) {
            chain.push((
                n.name.rsplit('/').next().unwrap_or(&n.name).to_string(),
                Some(n.name.clone()),
            ));
            node = n.parent.and_then(|p| index.get(&p).copied());
        }
        chain.push((root.to_string(), None));
        chain.reverse();
        chain
    }
}

/// The ground plan for the current scope and lens (`renderWorld`): heights
/// in the metric lenses normalise to the highest value in view.
fn plan(s: &StartState) -> Layout {
    let regions: Vec<Region<'_>> = s
        .scope
        .areas
        .iter()
        .map(|a| Region {
            name: &a.name,
            entities: &a.entities,
        })
        .collect();
    if s.kpi_active() {
        let max = s
            .scope
            .areas
            .iter()
            .flat_map(|a| a.entities.iter().filter_map(|&e| s.value(e)))
            .fold(0.0, f64::max);
        layout::layout(&regions, |e| s.value(e), |v| layout::metric_height(v, max))
    } else {
        layout::layout(&regions, |e| s.value(e), layout::complexity_height)
    }
}

/// A measured value as the web prints it: integers without decimals.
pub(super) fn number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v:.2}")
    }
}
