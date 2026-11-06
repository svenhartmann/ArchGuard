//! The services World's state: islands, connections and filters
//! (`repositoryWorldState` and `renderRepositoryWorld` in the web).

use archguard_engine::core::{ServiceDependencyKind as Kind, ServiceTopology};

use super::super::layout::{Cell, Layout, Plate};
use super::inventory::{self, Island};
use super::links::{self, Direction, Link, Source};
use crate::tab_support::Topology;

/// The filters of the web's `repositoryWorldState`.
#[derive(Debug, Clone, PartialEq)]
pub struct Filter {
    /// One deployment variant; `None`: all.
    pub deployment: Option<String>,
    pub source: Source,
    /// One kind; `None`: communication (all but startup).
    pub kind: Option<Kind>,
    pub direction: Direction,
    pub cycles_only: bool,
    pub all_projects: bool,
    /// Animated dots along calls and events.
    pub flow: bool,
}

/// How a connection is drawn (`worldDraw`): cyclic red, into the selected
/// island in the accent colour, otherwise green.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeTone {
    Cycle,
    Into,
    Other,
}

/// The kinds of the "Connections" filter after "Communication".
pub const KINDS: [Kind; 6] = [
    Kind::HttpCall,
    Kind::GrpcCall,
    Kind::Event,
    Kind::ConfiguredHttp,
    Kind::ConfiguredReference,
    Kind::Startup,
];

pub struct ServicesWorld {
    topology: Topology,
    pub(crate) inventory: Vec<Island>,
    /// The deployment variants (`repositoryDeploymentScopes`).
    pub(crate) scopes: Vec<String>,
    pub(crate) filter: Filter,
    /// Inventory indices of the islands shown, in layout order.
    pub(crate) shown: Vec<usize>,
    pub(crate) links: Vec<Link>,
    /// The selected island (inventory index).
    pub(crate) selected: Option<usize>,
    pub(crate) layout: Layout,
}

impl ServicesWorld {
    /// The services World of a topology, if it applies (see
    /// [`inventory::available`]).
    pub fn new(topology: Topology) -> Option<Self> {
        let t = topology.as_ref().as_ref().ok()?;
        let inventory = inventory::inventory(t);
        if !inventory::available(t, &inventory) {
            return None;
        }
        let scopes = inventory::deployment_scopes(t);
        let filter = Filter {
            deployment: inventory::default_deployment(&scopes),
            source: Source::All,
            kind: None,
            direction: Direction::Both,
            cycles_only: false,
            all_projects: false,
            flow: false,
        };
        let mut w = ServicesWorld {
            topology,
            inventory,
            scopes,
            filter,
            shown: Vec::new(),
            links: Vec::new(),
            selected: None,
            layout: Layout::default(),
        };
        w.refresh();
        Some(w)
    }

    pub(crate) fn topology(&self) -> &ServiceTopology {
        match &*self.topology {
            Ok(t) => t,
            Err(_) => unreachable!("built from a topology"),
        }
    }

    /// Recompute islands, connections and the ground plan after a filter
    /// change; a selection that is no longer shown is dropped.
    pub(crate) fn refresh(&mut self) {
        let f = &self.filter;
        let t = self.topology();
        let shown = inventory::shown(t, &self.inventory, f.deployment.as_deref(), f.all_projects);
        // As the web: all variants link through the whole inventory.
        let pool: Vec<usize> = match f.deployment {
            None => (0..self.inventory.len()).collect(),
            Some(_) => shown.clone(),
        };
        self.links = links::connections(t, &self.inventory, &pool, f.source);
        self.layout = islands(shown.len());
        self.selected = self.selected.filter(|s| shown.contains(s));
        self.shown = shown;
    }

    /// The connections the filters let through (indices into `links`).
    pub(crate) fn visible(&self) -> Vec<usize> {
        let f = &self.filter;
        links::visible(
            &self.links,
            f.kind,
            f.cycles_only,
            f.direction,
            self.selected,
        )
    }

    /// The layout area of an island.
    pub(crate) fn area(&self, island: usize) -> Option<usize> {
        self.shown.iter().position(|&i| i == island)
    }

    pub(crate) fn island(&self, area: usize) -> &Island {
        &self.inventory[self.shown[area]]
    }

    pub(crate) fn tone(&self, link: &Link) -> EdgeTone {
        if link.cyclic {
            EdgeTone::Cycle
        } else if Some(link.to) == self.selected {
            EdgeTone::Into
        } else {
            EdgeTone::Other
        }
    }

    /// Per area: whether a visible connection of it is cyclic (red plate).
    pub(crate) fn cyclic_areas(&self, visible: &[usize]) -> Vec<bool> {
        let mut out = vec![false; self.shown.len()];
        for l in visible.iter().map(|&i| &self.links[i]).filter(|l| l.cyclic) {
            for end in [l.from, l.to] {
                if let Some(a) = self.area(end) {
                    out[a] = true;
                }
            }
        }
        out
    }

    /// `worldMatches`: the island's path contains the term.
    pub(crate) fn matches(&self, area: usize, query: &str) -> bool {
        query.is_empty() || self.island(area).path.to_lowercase().contains(query)
    }

    /// Select the island of a layout area, or none.
    pub(crate) fn select_area(&mut self, area: Option<usize>) {
        self.selected = area.and_then(|a| self.shown.get(a).copied());
    }

    /// A filter that changes which islands are shown: selection off.
    pub(crate) fn set_islands(&mut self, deployment: Option<String>, all_projects: bool) {
        self.filter.deployment = deployment;
        self.filter.all_projects = all_projects;
        self.selected = None;
        self.refresh();
    }
}

/// One 2×2 island per project in a square grid, five units apart, four
/// flat facets each (`renderRepositoryWorld`).
fn islands(n: usize) -> Layout {
    let columns = ((n as f64).sqrt().ceil() as usize).max(1);
    let mut out = Layout::default();
    for area in 0..n {
        let (x, y) = ((area % columns * 5) as i64, (area / columns * 5) as i64);
        out.plates.push(Plate {
            area,
            x: x as f64,
            y: y as f64,
            w: 2.0,
            h: 2.0,
        });
        for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
            out.cells.push(Cell {
                x: x + dx,
                y: y + dy,
                z: 0.08,
                value: None,
                area,
                entities: vec![area],
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Five islands: three columns, a gap of three tiles, four facets each.
    #[test]
    fn islands_are_two_by_two_in_a_square_grid() {
        let l = islands(5);
        let at: Vec<(f64, f64)> = l.plates.iter().map(|p| (p.x, p.y)).collect();
        assert_eq!(
            at,
            [(0.0, 0.0), (5.0, 0.0), (10.0, 0.0), (0.0, 5.0), (5.0, 5.0)]
        );
        assert_eq!(l.plates[1].x - l.plates[0].x - l.plates[0].w, 3.0);
        assert_eq!(l.cells.len(), 20);
        let first: Vec<(i64, i64)> = l.cells[..4].iter().map(|c| (c.x, c.y)).collect();
        assert_eq!(first, [(0, 0), (1, 0), (0, 1), (1, 1)]);
        assert!(l.cells.iter().all(|c| c.z == 0.08 && c.value.is_none()));
        assert_eq!(l.cells[19].area, 4);
    }

    fn link(from: usize, to: usize, cyclic: bool) -> Link {
        Link {
            from,
            to,
            kind: Kind::HttpCall,
            evidence: Vec::new(),
            cyclic,
        }
    }

    /// Without a selection connections are green; with one, those into it
    /// take the accent; cyclic ones stay red either way.
    #[test]
    fn selection_tones() {
        let topology = std::sync::Arc::new(Ok(inventory::tests::topology(
            vec![
                inventory::tests::project("a", "application"),
                inventory::tests::project("b", "application"),
            ],
            Vec::new(),
        )));
        let mut w = ServicesWorld::new(topology).expect("two projects");
        let (into, out, cyc) = (link(0, 1, false), link(1, 0, false), link(0, 1, true));
        assert_eq!(w.tone(&into), EdgeTone::Other);
        w.select_area(Some(1));
        assert_eq!(w.selected, Some(1));
        assert_eq!(
            [w.tone(&into), w.tone(&out), w.tone(&cyc)],
            [EdgeTone::Into, EdgeTone::Other, EdgeTone::Cycle]
        );
        w.links = vec![into, cyc];
        assert_eq!(w.cyclic_areas(&[0]), [false, false]);
        assert_eq!(w.cyclic_areas(&[0, 1]), [true, true]);
        assert!(w.matches(0, "a") && !w.matches(0, "zz"));
        w.set_islands(None, true);
        assert_eq!(w.selected, None, "changing the islands drops the selection");
    }
}
