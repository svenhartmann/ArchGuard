//! The Start tab's state: navigation, lenses, selection and camera, as
//! `worldUi` in `viewer_world.js`. Views read it; they never write it.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::coverage_detail::Files;
use super::evidence::{self, Evidence, Scope};
use super::kpis::Glance;
use super::layout::{self, Layout, Region};
use super::scene::{self, Camera};
use super::services::ServicesWorld;

/// Which World the tab shows: the packages of the analysed source, or the
/// services of a repository (`isRepositoryOverview` in the web).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Architecture,
    Services,
}

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
    /// The project root as the MCP requests of "AI context" name it.
    pub(super) root: String,
    /// The services World, when the topology has one.
    pub(super) services: Option<ServicesWorld>,
    pub(super) mode: Mode,
    /// The architecture World was entered from a service island: Back and
    /// zooming out at the minimum return to the services.
    pub(super) entered: bool,
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
    /// Connection entries opened in the details, by area names.
    pub(super) open_connections: BTreeSet<(String, String)>,
    /// Detail sections the user opened (`true`) or closed, by key
    /// (method entries, the coverage review).
    pub(super) sections: BTreeMap<String, bool>,
    /// The project's review KPIs below the World (independent of scope).
    pub(super) glance: Glance,
    /// Imported test coverage per source file.
    pub(super) files: Files,
    // Derived from the above by `refresh`.
    pub(super) scope: Scope,
    pub(super) layout: Layout,
}

pub(super) fn from_evidence(ev: Evidence, services: Option<ServicesWorld>) -> StartState {
    let _span = archguard_engine::self_trace::tracing::info_span!("from_evidence").entered();
    let mut s = StartState {
        ev,
        root: String::new(),
        // As the web, a repository opens on its services.
        mode: if services.is_some() {
            Mode::Services
        } else {
            Mode::Architecture
        },
        services,
        entered: false,
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
        open_connections: BTreeSet::new(),
        sections: BTreeMap::new(),
        glance: Glance::default(),
        files: Files::default(),
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
        self.layout = match (&self.services, self.mode) {
            (Some(w), Mode::Services) => w.layout.clone(),
            _ => plan(self),
        };
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
            self.details_open = true;
        }
    }

    /// Select something. Unlike the web, which opens the details of an
    /// entity or a file at once, the details open only on request
    /// (Details, Enter, [`Self::open_area`]); open ones stay open.
    pub(super) fn select(&mut self, sel: Selection) {
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

    /// `worldMatches`: the class id or file name contains the term; on
    /// the services World the island's path.
    pub(super) fn matches(&self, cell: &layout::Cell) -> bool {
        if let (Some(w), Mode::Services) = (&self.services, self.mode) {
            return w.matches(cell.area, &self.query);
        }
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
        match (first, self.mode) {
            (Some(i), Mode::Services) => {
                let area = self.layout.cells[i].area;
                if let Some(w) = &mut self.services {
                    w.select_area(Some(area));
                }
            }
            (Some(i), Mode::Architecture) => self.select_cell(i),
            (None, _) => {}
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
