//! The Services tab's state: the topology and the web's `serviceUi`.

use crate::tab_support::Topology;

use archguard_engine::core::{ServiceDependencyKind, ServiceTopology};

/// The relation list in the toolbar's order.
pub const KINDS: [ServiceDependencyKind; 6] = [
    ServiceDependencyKind::ConfiguredHttp,
    ServiceDependencyKind::Startup,
    ServiceDependencyKind::HttpCall,
    ServiceDependencyKind::GrpcCall,
    ServiceDependencyKind::Event,
    ServiceDependencyKind::ConfiguredReference,
];

/// The direction filter (`serviceUi.mode`), in the toolbar's order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    All = 0,
    Incoming = 1,
    Outgoing = 2,
    Both = 3,
}

/// Everything the tab knows: the topology and the web's `serviceUi`.
pub struct ServicesState {
    pub(super) topology: Topology,
    /// Projects by path, then manifest (indices into `projects`).
    pub(super) project_order: Vec<usize>,
    /// Index into [`KINDS`].
    pub(super) kind: usize,
    pub(super) mode: Mode,
    /// "In/Out" routing instead of "Up / down".
    pub(super) in_out: bool,
    /// Selected service and project (indices into the topology).
    pub(super) selected: Option<usize>,
    pub(super) project: Option<usize>,
    pub(super) project_limit: usize,
    /// Inspected dependency (index into `dependencies`).
    pub(super) edge: Option<usize>,
    pub(super) cycles_only: bool,
    /// Service ids of the chosen cyclic group.
    pub(super) cycle: Option<Vec<String>>,
    /// The details panel was closed for the current selection.
    pub(super) details_hidden: bool,
    pub(super) zoom: f32,
    /// Width of the tree viewport (the web's `treeAvailableWidth`).
    pub(super) viewport: f32,
    /// Runtime profile from `.archguard/runtime` (ADR 0004), if recorded.
    pub(super) runtime: Option<std::sync::Arc<archguard_engine::core::RuntimeProfile>>,
    /// The "Runtime" view instead of the topology.
    pub(super) runtime_view: bool,
}

pub fn build_from(topology: Topology) -> ServicesState {
    let mut project_order = Vec::new();
    let mut kind = ServiceDependencyKind::ConfiguredHttp;
    if let Ok(t) = &*topology {
        project_order = (0..t.projects.len()).collect();
        project_order.sort_by(|&a, &b| {
            let (x, y) = (&t.projects[a], &t.projects[b]);
            (&x.path, &x.manifest).cmp(&(&y.path, &y.manifest))
        });
        kind = initial_kind(t);
    }
    ServicesState {
        topology,
        project_order,
        kind: KINDS.iter().position(|&k| k == kind).unwrap_or(0),
        mode: Mode::All,
        in_out: false,
        selected: None,
        project: None,
        project_limit: 100,
        edge: None,
        cycles_only: false,
        cycle: None,
        details_hidden: false,
        zoom: 1.0,
        viewport: 0.0,
        runtime: None,
        runtime_view: false,
    }
}

/// Configured HTTP unless absent; then the first call, event or startup
/// relation (`initServices`).
fn initial_kind(t: &ServiceTopology) -> ServiceDependencyKind {
    use ServiceDependencyKind::*;
    if t.dependencies.iter().any(|e| e.kind == ConfiguredHttp) {
        return ConfiguredHttp;
    }
    t.dependencies
        .iter()
        .map(|e| e.kind)
        .find(|k| matches!(k, HttpCall | GrpcCall | Event | Startup))
        .unwrap_or(ConfiguredHttp)
}

impl ServicesState {
    pub(super) fn topology(&self) -> Option<&ServiceTopology> {
        (*self.topology).as_ref().ok()
    }

    pub(super) fn clear_focus(&mut self) {
        self.edge = None;
        self.cycle = None;
        self.details_hidden = false;
    }

    /// `selectService`.
    pub(super) fn select_service(&mut self, i: usize) {
        let Some(t) = self.topology() else { return };
        let Some(service) = t.services.get(i) else {
            return;
        };
        let project = service
            .project
            .as_deref()
            .and_then(|id| t.projects.iter().position(|p| p.id == id));
        self.selected = Some(i);
        self.project = project;
        self.clear_focus();
        if self.mode == Mode::All {
            self.mode = Mode::Both;
        }
    }

    /// `selectServiceProject` (row `i` of the sorted inventory).
    pub(super) fn select_project(&mut self, i: usize) {
        let Some(&p) = self.project_order.get(i) else {
            return;
        };
        let Some(t) = self.topology() else { return };
        let id = t.projects[p].id.as_str();
        let matches: Vec<usize> = (0..t.services.len())
            .filter(|&s| t.services[s].project.as_deref() == Some(id))
            .collect();
        self.project = Some(p);
        self.selected = (matches.len() == 1).then(|| matches[0]);
        self.clear_focus();
        if self.selected.is_some() && self.mode == Mode::All {
            self.mode = Mode::Both;
        }
    }

    /// A cyclic group button: show only that group.
    pub(super) fn select_cycle(&mut self, i: usize) {
        let Some(t) = self.topology() else { return };
        let kind = KINDS[self.kind];
        let Some(c) = t.cycles.iter().filter(|c| c.kind == kind).nth(i) else {
            return;
        };
        let nodes = c.nodes.clone();
        self.show_all();
        self.cycle = Some(nodes);
        self.cycles_only = true;
    }

    /// "All services".
    pub(super) fn show_all(&mut self) {
        self.selected = None;
        self.project = None;
        self.clear_focus();
        self.cycles_only = false;
        self.mode = Mode::All;
    }

    pub(super) fn select_edge(&mut self, d: usize) {
        let known = self.topology().is_some_and(|t| d < t.dependencies.len());
        if known {
            self.edge = Some(d);
            self.details_hidden = false;
        }
    }

    pub(super) fn zoom_by(&mut self, factor: f32) {
        self.zoom = if factor == 0.0 {
            1.0
        } else {
            (self.zoom * factor).clamp(0.01, 3.0)
        };
    }
}
