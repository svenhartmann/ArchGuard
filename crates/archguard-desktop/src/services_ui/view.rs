//! The window model of the Services tab: the web's `renderServices`,
//! `renderServiceProjects` and `renderServiceEvidence` as one
//! `ServicesModel` value. Texts are composed in Slint (translated); this
//! module supplies numbers, names and geometry.

use archguard_engine::core::{
    ServiceCycle, ServiceDependency, ServiceDependencyKind, ServiceEvidence, ServiceTopology,
};
use slint::{ModelRc, SharedString, VecModel};

use super::layout::{self, Link, Node, Side};
use super::paths;
use super::state::{KINDS, Mode, ServicesState};
use crate::ui::{
    ServicesModel, SvcCycle, SvcEdge, SvcEvidence, SvcInspector, SvcLink, SvcProject, SvcRow,
};

fn kind_name(kind: ServiceDependencyKind) -> &'static str {
    match kind {
        ServiceDependencyKind::Startup => "startup",
        ServiceDependencyKind::ConfiguredHttp => "configured_http",
        ServiceDependencyKind::HttpCall => "http_call",
        ServiceDependencyKind::GrpcCall => "grpc_call",
        ServiceDependencyKind::Event => "event",
        ServiceDependencyKind::ConfiguredReference => "configured_reference",
    }
}

fn kind_index(kind: ServiceDependencyKind) -> i32 {
    KINDS.iter().position(|&k| k == kind).unwrap_or(0) as i32
}

/// `serviceProjectKind` index: application, library, tooling, workspace, unknown.
fn project_kind(kind: &str) -> i32 {
    match kind {
        "application" => 0,
        "library" => 1,
        "tooling" => 2,
        "workspace" => 3,
        _ => 4,
    }
}

/// The web's `serviceKey`: `JSON.stringify([from, to, kind])`.
pub fn key(e: &ServiceDependency) -> String {
    let quote = |s: &str| {
        let mut out = String::from("\"");
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                c => out.push(c),
            }
        }
        out.push('"');
        out
    };
    format!(
        "[{},{},{}]",
        quote(&e.from),
        quote(&e.to),
        quote(kind_name(e.kind))
    )
}

fn model<T: Clone + 'static>(items: impl IntoIterator<Item = T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items.into_iter().collect::<Vec<_>>()))
}

/// What the current relation kind and filters select.
pub struct Scope<'a> {
    pub t: &'a ServiceTopology,
    /// Dependencies of the chosen kind (`serviceDependencies`).
    pub all: Vec<usize>,
    /// Cyclic groups of the chosen kind (`serviceCycles`).
    pub cycles: Vec<&'a ServiceCycle>,
    pub selected: Option<&'a str>,
}

impl<'a> Scope<'a> {
    pub fn new(s: &ServicesState, t: &'a ServiceTopology) -> Self {
        let kind = KINDS[s.kind];
        Scope {
            t,
            all: (0..t.dependencies.len())
                .filter(|&d| t.dependencies[d].kind == kind)
                .collect(),
            cycles: t.cycles.iter().filter(|c| c.kind == kind).collect(),
            selected: s.selected.map(|i| t.services[i].id.as_str()),
        }
    }

    /// `serviceEdgeInCycle`.
    pub fn in_cycle(&self, d: usize) -> bool {
        let e = &self.t.dependencies[d];
        self.cycles
            .iter()
            .any(|c| c.nodes.contains(&e.from) && c.nodes.contains(&e.to))
    }

    /// `serviceMatches`: the drawn subset of `all`.
    pub fn shown(&self, s: &ServicesState) -> Vec<usize> {
        self.all
            .iter()
            .copied()
            .filter(|&d| self.matches(s, d))
            .collect()
    }

    fn matches(&self, s: &ServicesState, d: usize) -> bool {
        let e = &self.t.dependencies[d];
        if s.cycles_only && !self.in_cycle(d) {
            return false;
        }
        if let Some(c) = &s.cycle
            && (!c.contains(&e.from) || !c.contains(&e.to))
        {
            return false;
        }
        let Some(sel) = self.selected.filter(|_| s.mode != Mode::All) else {
            return true;
        };
        let (incoming, outgoing) = (e.to == sel, e.from == sel);
        match s.mode {
            Mode::Incoming => incoming,
            Mode::Outgoing => outgoing,
            _ => incoming || outgoing,
        }
    }

    /// `serviceName`: the service's name, else the id.
    pub fn name<'b>(&'b self, id: &'b str) -> &'b str {
        self.t
            .services
            .iter()
            .find(|s| s.id == id)
            .map_or(id, |s| s.name.as_str())
    }

    fn project_of(&self, service: usize) -> Option<usize> {
        let id = self.t.services[service].project.as_deref()?;
        self.t.projects.iter().position(|p| p.id == id)
    }
}

/// Build the whole window model for the state.
pub fn build(s: &ServicesState, unmapped: &str) -> ServicesModel {
    let t = match &*s.topology {
        Err(e) => {
            return ServicesModel {
                state: 2,
                error: e.as_str().into(),
                ..controls(s, None)
            };
        }
        Ok(t) => t,
    };
    let scope = Scope::new(s, t);
    let mut m = controls(s, Some(&scope));
    status(&mut m, &scope, s);
    if t.compose_files.is_empty() && t.services.is_empty() && t.projects.is_empty() {
        m.state = 3;
        return m;
    }
    m.state = 1;
    tree(&mut m, &scope, s, unmapped);
    m.inspector = inspector(&scope, s);
    m
}

fn controls(s: &ServicesState, scope: Option<&Scope>) -> ServicesModel {
    ServicesModel {
        kind: s.kind as i32,
        mode: s.mode as i32,
        routing: i32::from(s.in_out),
        cycles_only: s.cycles_only,
        has_selection: s.selected.is_some(),
        reference: scope
            .and_then(|sc| sc.selected.map(|id| sc.name(id).to_string()))
            .unwrap_or_default()
            .into(),
        zoom: s.zoom,
        ..Default::default()
    }
}

/// The counts the engine's monorepo reason states (`analyze_services`):
/// manifest project roots without workspace aggregators and local Compose
/// build contexts. The tab words them in the window's language.
pub fn monorepo_evidence(t: &archguard_engine::core::ServiceTopology) -> (usize, usize) {
    let projects: std::collections::BTreeSet<&str> = t
        .projects
        .iter()
        .filter(|p| p.kind != "workspace")
        .map(|p| p.path.as_str())
        .collect();
    let contexts: std::collections::BTreeSet<&str> = t
        .services
        .iter()
        .filter(|s| !s.compose_file.is_empty())
        .filter_map(|s| s.build_context.as_deref())
        .collect();
    (projects.len(), contexts.len())
}

fn status(m: &mut ServicesModel, sc: &Scope, s: &ServicesState) {
    let t = sc.t;
    m.complete = t.complete;
    m.projects = t.projects.len() as i32;
    m.applications = t
        .projects
        .iter()
        .filter(|p| p.kind == "application")
        .count() as i32;
    m.containers = t
        .services
        .iter()
        .filter(|x| !x.compose_file.is_empty())
        .count() as i32;
    m.services = t.services.len() as i32;
    m.compose_files = t.compose_files.len() as i32;
    m.all = sc.all.len() as i32;
    m.shown = sc.shown(s).len() as i32;
    m.monorepo = t.monorepo;
    m.build_units = crate::documents::build_units(t).len() as i32;
    let (projects, contexts) = monorepo_evidence(t);
    (m.reason_projects, m.reason_contexts) = (projects as i32, contexts as i32);
    m.diagnostics = model(t.diagnostics.iter().map(|d| {
        let service = d
            .service
            .as_ref()
            .map_or(String::new(), |x| format!(" · {x}"));
        SharedString::from(format!("{}{service}: {}", d.file, d.message))
    }));
    m.cycles = model(sc.cycles.iter().map(|c| {
        SvcCycle {
            names: c
                .nodes
                .iter()
                .map(|n| sc.name(n))
                .collect::<Vec<_>>()
                .join(", ")
                .into(),
        }
    }));
    m.project_list = model(s.project_order.iter().take(s.project_limit).map(|&i| {
        let p = &t.projects[i];
        SvcProject {
            path: p.path.as_str().into(),
            name: p.name.as_str().into(),
            kind: project_kind(&p.kind),
            ecosystem: p.ecosystem.as_str().into(),
            pressed: s.project == Some(i),
        }
    }));
    m.more_projects = s.project_order.len() > s.project_limit;
}

/// A tree node: a group (project path or Compose file) or a service.
#[derive(Clone, Copy, PartialEq)]
pub enum Item {
    Group,
    Service(usize),
}

/// The tree's nodes in the web's order (`renderServices`).
pub fn nodes(sc: &Scope, unmapped: &str) -> (Vec<Node>, Vec<Item>) {
    let (mut nodes, mut items) = (Vec::new(), Vec::new());
    let mut groups: Vec<(String, usize)> = Vec::new();
    for (i, service) in sc.t.services.iter().enumerate() {
        let project = sc.project_of(i).map(|p| &sc.t.projects[p]);
        let (key, name) = match project {
            Some(p) => (format!("project-group:{}", p.id), p.path.clone()),
            None if service.compose_file.is_empty() => {
                ("compose:".to_string(), unmapped.to_string())
            }
            None => (
                format!("compose:{}", service.compose_file),
                service.compose_file.clone(),
            ),
        };
        let group = match groups.iter().find(|(k, _)| *k == key) {
            Some(&(_, g)) => g,
            None => {
                nodes.push(Node {
                    id: key.clone(),
                    name,
                    parent: None,
                });
                items.push(Item::Group);
                groups.push((key, nodes.len() - 1));
                nodes.len() - 1
            }
        };
        nodes.push(Node {
            id: service.id.clone(),
            name: service.name.clone(),
            parent: Some(group),
        });
        items.push(Item::Service(i));
    }
    (nodes, items)
}

/// The relation links of the chosen kind, with "In/Out" sides, and the
/// dependency index of each link.
pub fn links(sc: &Scope, s: &ServicesState, nodes: &[Node]) -> (Vec<Link>, Vec<usize>) {
    let at = |id: &str| nodes.iter().position(|n| n.id == id);
    sc.all
        .iter()
        .filter_map(|&d| {
            let e = &sc.t.dependencies[d];
            let side = s.in_out.then(|| {
                let out = sc.selected == Some(e.from.as_str()) && sc.selected != Some(&e.to);
                if out { Side::Right } else { Side::Left }
            });
            let link = Link {
                from: at(&e.from)?,
                to: at(&e.to)?,
                key: key(e),
                side,
            };
            Some((link, d))
        })
        .unzip()
}

/// The placed tree for the state (also used by "Fit").
pub struct Placed {
    pub lay: layout::Layout,
    pub nodes: Vec<Node>,
    pub items: Vec<Item>,
    /// Dependency index per layout link.
    pub deps: Vec<usize>,
}

pub fn place(sc: &Scope, s: &ServicesState, unmapped: &str) -> Placed {
    let (nodes, items) = nodes(sc, unmapped);
    let (links, deps) = links(sc, s, &nodes);
    Placed {
        lay: layout::layout(&nodes, &links, s.viewport),
        nodes,
        items,
        deps,
    }
}

/// `shorten(name, max(6, floor(labelWidth / 7.5)))`.
fn shorten(text: &str, label_width: f32) -> String {
    let max = ((label_width / 7.5).floor() as usize).max(6);
    if text.chars().count() > max {
        text.chars().take(max - 1).chain(['…']).collect()
    } else {
        text.to_string()
    }
}

fn tree(m: &mut ServicesModel, sc: &Scope, s: &ServicesState, unmapped: &str) {
    let placed = place(sc, s, unmapped);
    let (lay, nodes, items) = (&placed.lay, &placed.nodes, &placed.items);
    let in_cycle = |id: &str| sc.cycles.iter().any(|c| c.nodes.iter().any(|n| n == id));
    let count = |f: &dyn Fn(&ServiceDependency) -> bool| {
        sc.all.iter().filter(|&&d| f(&sc.t.dependencies[d])).count() as i32
    };
    m.rows = model(lay.rows.iter().map(|r| {
        let id = nodes[r.node].id.as_str();
        let service = match items[r.node] {
            Item::Service(i) => i as i32,
            Item::Group => -1,
        };
        // Icon and padding, plus the In/Out column (bare counts when compact).
        let counts = if lay.compact { 90.0 } else { 145.0 };
        let label_width = lay.w - r.indent - if service >= 0 { counts } else { 36.0 };
        SvcRow {
            name: shorten(&nodes[r.node].name, label_width).into(),
            context: service < 0,
            service,
            left: r.x - lay.w / 2.0,
            y: r.y,
            w: lay.w,
            h: lay.h,
            indent: r.indent,
            label_width,
            relation_x: lay.relation_x,
            compact: lay.compact,
            selected: service >= 0 && sc.selected == Some(id),
            in_cycle: in_cycle(id),
            incoming: if service >= 0 {
                count(&|e| e.to == id)
            } else {
                0
            },
            outgoing: if service >= 0 {
                count(&|e| e.from == id)
            } else {
                0
            },
        }
    }));
    m.hierarchy = lay
        .hierarchy
        .iter()
        .map(|p| paths::rounded(p, 6.0))
        .collect::<Vec<_>>()
        .join(" ")
        .into();
    m.edges = model(edges(sc, s, lay, &placed.deps));
    m.width = lay.width;
    m.height = lay.height;
}

/// The drawn relations (`shown`), coloured as `serviceColor`.
fn edges(sc: &Scope, s: &ServicesState, lay: &layout::Layout, deps: &[usize]) -> Vec<SvcEdge> {
    let shown = sc.shown(s);
    let mut out = Vec::new();
    for route in &lay.routes {
        let d = deps[route.link];
        if !shown.contains(&d) {
            continue;
        }
        let e = &sc.t.dependencies[d];
        let color = if route.side == Side::Right && sc.in_cycle(d) {
            3
        } else if sc.selected == Some(e.to.as_str()) {
            1
        } else if sc.selected == Some(e.from.as_str()) {
            2
        } else {
            0
        };
        let p = &route.points;
        out.push(SvcEdge {
            line: if e.kind == ServiceDependencyKind::Startup {
                paths::dashed(p, 6.0, 3.0)
            } else {
                paths::curve(p)
            }
            .into(),
            head: paths::head(p[5], p[4]).into(),
            color,
            selected: s.edge == Some(d),
            edge: d as i32,
            tip_x: p[5].x,
            tip_y: p[5].y,
            top: p[0].y.min(p[5].y),
            bottom: p[0].y.max(p[5].y),
        });
    }
    out
}

fn evidence(items: &[ServiceEvidence]) -> ModelRc<SvcEvidence> {
    model(items.iter().map(|e| {
        let line = e.line.map_or(String::new(), |l| format!(":{l}"));
        let pointer = if e.pointer.is_empty() {
            String::new()
        } else {
            format!(" · {}", e.pointer)
        };
        SvcEvidence {
            location: format!("{}{line}{pointer}", e.file).into(),
            description: e.description.as_str().into(),
        }
    }))
}

/// `renderServiceEvidence`: relation, project or deployment details.
fn inspector(sc: &Scope, s: &ServicesState) -> SvcInspector {
    let t = sc.t;
    if s.details_hidden {
        return SvcInspector::default();
    }
    if let Some(d) = s.edge {
        let e = &t.dependencies[d];
        return SvcInspector {
            mode: 1,
            title: format!("{} → {}", sc.name(&e.from), sc.name(&e.to)).into(),
            kind: kind_index(e.kind),
            evidence: evidence(&e.evidence),
            ..Default::default()
        };
    }
    let project = s
        .project
        .or_else(|| s.selected.and_then(|i| sc.project_of(i)));
    if s.selected.is_none() && project.is_none() {
        return SvcInspector::default();
    }
    let mut out = SvcInspector {
        kind: s.kind as i32,
        ..Default::default()
    };
    if let Some(p) = project.map(|p| &t.projects[p]) {
        out.title = p.name.as_str().into();
        out.has_project = true;
        out.project_kind = project_kind(&p.kind);
        out.ecosystem = p.ecosystem.as_str().into();
        out.path = p.path.as_str().into();
        out.manifest = p.manifest.as_str().into();
        out.runtime = p.runtime.as_deref().unwrap_or_default().into();
        out.project_evidence = evidence(&p.evidence);
    }
    match s.selected {
        None => deployments(&mut out, sc, project),
        Some(i) => deployment(&mut out, sc, i),
    }
    out
}

/// A project without a chosen deployment: its deployments as buttons.
fn deployments(out: &mut SvcInspector, sc: &Scope, project: Option<usize>) {
    let id = project.map(|p| sc.t.projects[p].id.as_str());
    out.mode = 2;
    out.links = model(
        sc.t.services
            .iter()
            .enumerate()
            .filter(|(_, x)| x.project.as_deref() == id)
            .map(|(i, x)| SvcLink {
                label: x.name.as_str().into(),
                edge: -1,
                service: i as i32,
            }),
    );
}

/// A deployment: Compose facts and its relations of the chosen kind.
fn deployment(out: &mut SvcInspector, sc: &Scope, i: usize) {
    let x = &sc.t.services[i];
    out.mode = 3;
    if !out.has_project {
        out.title = x.name.as_str().into();
    }
    out.compose_file = x.compose_file.as_str().into();
    out.build_context = x.build_context.as_deref().unwrap_or_default().into();
    out.networks = x.networks.join(", ").into();
    out.profiles = x.profiles.join(", ").into();
    out.links = model(
        sc.all
            .iter()
            .copied()
            .filter(|&d| {
                let e = &sc.t.dependencies[d];
                e.from == x.id || e.to == x.id
            })
            .map(|d| {
                let e = &sc.t.dependencies[d];
                SvcLink {
                    label: format!("{} → {}", sc.name(&e.from), sc.name(&e.to)).into(),
                    edge: d as i32,
                    service: -1,
                }
            }),
    );
}

/// `fitServices` for a viewport of `w` × `h`.
pub fn fit(s: &mut ServicesState, w: f32, h: f32, unmapped: &str) {
    let Some(t) = (*s.topology).as_ref().ok() else {
        return;
    };
    let placed = place(&Scope::new(s, t), s, unmapped);
    let (cw, ch) = (placed.lay.width, placed.lay.height);
    if cw > 0.0 && ch > 0.0 {
        let z = ((w - 24.0).max(1.0) / cw).min((h - 24.0).max(1.0) / ch);
        s.zoom = z.clamp(0.01, 3.0);
    }
}

#[cfg(test)]
mod tests {
    // The Services tab on `examples/service-shop`; expected values are read
    // from the web viewer's Services tab for the same project (1480 px wide).

    use std::path::PathBuf;

    use slint::Model;

    use super::super::state::build_from;
    use super::*;
    use crate::ui::SvcRow;

    fn shop() -> ServicesState {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/service-shop");
        let mut s = build_from(std::sync::Arc::new(
            archguard_engine::analyze_services(&root).map_err(|e| e.to_string()),
        ));
        s.viewport = 1480.0;
        s
    }

    fn rows(m: &ServicesModel) -> Vec<SvcRow> {
        m.rows.iter().collect()
    }

    fn service(s: &ServicesState, name: &str) -> usize {
        let t = (*s.topology).as_ref().expect("topology");
        t.services.iter().position(|x| x.name == name).expect(name)
    }

    #[test]
    fn status_counts_match_the_web() {
        let m = build(&shop(), "Unmapped service");
        assert_eq!(m.state, 1);
        assert_eq!((m.projects, m.applications, m.containers), (1, 1, 14));
        // The header chip's set: the ten local build contexts.
        assert_eq!(m.build_units, 10);
        assert_eq!((m.reason_projects, m.reason_contexts), (1, 10));
        assert_eq!((m.shown, m.all), (15, 15));
        assert_eq!(m.cycles.row_count(), 2);
        assert!(m.monorepo);
        assert_eq!(m.kind, 0, "configured HTTP relations exist");
    }

    /// The counts shown in place of the engine's English monorepo reason
    /// are the ones that reason states.
    #[test]
    fn monorepo_counts_restate_the_engine_reason() {
        let s = shop();
        let t = (*s.topology).as_ref().expect("topology");
        let (projects, contexts) = monorepo_evidence(t);
        assert_eq!(
            t.monorepo_reason,
            format!(
                "{projects} distinct manifest project roots (excluding workspace aggregators), {contexts} verified local Compose build contexts; at least two project roots or build contexts establish monorepo evidence."
            )
        );
    }

    #[test]
    fn rows_follow_the_web_order_and_counts() {
        let m = build(&shop(), "Unmapped service");
        let rows = rows(&m);
        let seen: Vec<(String, i32, i32)> = rows
            .iter()
            .take(15)
            .map(|r| (r.name.to_string(), r.incoming, r.outgoing))
            .collect();
        let expected = [
            (".", 0, 0),
            ("gateway", 0, 3),
            ("orders", 2, 4),
            ("payments", 1, 1),
            ("risk", 1, 1),
            ("catalog", 2, 2),
            ("pricing", 1, 1),
            ("promotions", 1, 1),
            ("shipping", 1, 2),
            ("inventory", 4, 0),
            ("notifications", 2, 0),
            ("stacks/checkout/compose.yaml", 0, 0),
            ("orders-db", 0, 0),
            ("stacks/fulfillment/compose.yaml", 0, 0),
            ("rabbitmq", 0, 0),
        ];
        let expected: Vec<(String, i32, i32)> = expected
            .iter()
            .map(|&(n, i, o)| (n.to_string(), i, o))
            .collect();
        assert_eq!(seen, expected);
        assert_eq!(rows.len(), 18, "4 groups and 14 services");
        // Geometry of the web at 1480 px: rows 24..672, relation axis 1076.
        assert_eq!(
            (rows[0].left, rows[0].w, rows[0].relation_x),
            (24.0, 648.0, 1076.0)
        );
        assert_eq!((rows[1].y, rows[1].indent), (68.0, 18.0));
        let cyclic: Vec<&str> = rows
            .iter()
            .filter(|r| r.in_cycle)
            .map(|r| r.name.as_str())
            .collect();
        assert_eq!(
            cyclic,
            [
                "orders",
                "payments",
                "risk",
                "catalog",
                "pricing",
                "promotions"
            ]
        );
    }

    #[test]
    fn only_right_hand_cycle_edges_are_red() {
        let m = build(&shop(), "Unmapped service");
        let red = m.edges.iter().filter(|e| e.color == 3).count();
        assert_eq!(m.edges.row_count(), 15);
        assert_eq!(red, 2, "risk → orders and promotions → catalog go up");
        // Each arc spans its two rows; the view draws only arcs near the
        // visible band.
        for e in m.edges.iter() {
            assert!(e.top < e.bottom, "{} .. {}", e.top, e.bottom);
            assert!(e.tip_y == e.top || e.tip_y == e.bottom);
        }
    }

    #[test]
    fn selecting_a_service_focuses_its_connections() {
        let mut s = shop();
        s.select_service(service(&s, "gateway"));
        assert_eq!(s.mode, Mode::Both);
        let m = build(&s, "Unmapped service");
        assert_eq!((m.shown, m.all), (3, 15));
        assert!(m.edges.iter().all(|e| e.color == 2), "outgoing violet");
        assert_eq!(m.inspector.mode, 3);
        assert_eq!(m.inspector.title, "archguard-service-shop-example");
        assert_eq!(m.inspector.links.row_count(), 3);
        assert_eq!(m.reference, "gateway");

        s.select_service(service(&s, "inventory"));
        s.mode = Mode::Incoming;
        assert_eq!(build(&s, "").shown, 4);
    }

    #[test]
    fn a_cyclic_group_shows_only_its_edges() {
        let mut s = shop();
        s.select_service(service(&s, "gateway"));
        s.select_cycle(0);
        assert!(s.cycles_only && s.selected.is_none() && s.mode == Mode::All);
        let m = build(&s, "");
        assert_eq!(m.shown, 3, "orders → payments → risk → orders");
        assert_eq!(m.inspector.mode, 0);
    }

    #[test]
    fn an_edge_shows_its_evidence() {
        let mut s = shop();
        let t = (*s.topology).as_ref().expect("topology");
        let d = t
            .dependencies
            .iter()
            .position(|e| e.kind == ServiceDependencyKind::ConfiguredHttp)
            .expect("edge");
        s.select_edge(d);
        let m = build(&s, "");
        assert_eq!(m.inspector.mode, 1);
        assert_eq!(m.inspector.title, "orders → payments");
        let ev = m.inspector.evidence.row_data(0).expect("evidence");
        assert_eq!(
            ev.location,
            "stacks/checkout/compose.yaml · /services/orders/environment/PAYMENTS_API"
        );
    }

    #[test]
    fn startup_relations_are_dashed_and_switch_counts() {
        let mut s = shop();
        s.kind = 1;
        let m = build(&s, "");
        assert_eq!((m.shown, m.all), (7, 7));
        let line = m.edges.row_data(0).expect("edge").line;
        assert!(line.matches('M').count() > 3, "dashed: {line}");
    }

    /// In a split pane the rows and their arcs fit the viewport: no
    /// count or arc is drawn beyond its right edge.
    #[test]
    fn the_tree_fits_a_narrow_pane() {
        let mut s = shop();
        s.viewport = 356.0;
        let t = (*s.topology).as_ref().expect("topology");
        let placed = place(&Scope::new(&s, t), &s, "");
        let lay = &placed.lay;
        assert!(lay.width <= 356.0, "canvas {} wide", lay.width);
        for r in &placed.lay.routes {
            for p in &r.points {
                assert!(p.x >= 24.0 + lay.w, "{p:?} over the rows");
                assert!(p.x <= lay.width, "{p:?} beyond the canvas");
            }
        }
        let m = build(&s, "");
        for r in rows(&m) {
            assert!(r.compact, "{}: compact counts", r.name);
            assert!(r.label_width >= 40.0, "{}: label {}", r.name, r.label_width);
            assert!(
                r.left + r.w < r.relation_x,
                "{}: row left of the axis",
                r.name
            );
        }
    }

    /// From the web's minimum width on, nothing changes.
    #[test]
    fn a_wide_pane_keeps_the_web_geometry() {
        let mut s = shop();
        s.viewport = 860.0;
        let m = build(&s, "");
        let r = rows(&m).remove(1);
        assert!(!r.compact);
        assert_eq!((r.w, r.relation_x), (400.0, 642.0));
    }

    #[test]
    fn fit_scales_the_canvas_into_the_viewport() {
        let mut s = shop();
        fit(&mut s, 1480.0, 400.0, "");
        // 18 rows: height 24 + 14 + 17·30 + 14 + 24 = 586; width 1480.
        assert!((s.zoom - 376.0 / 586.0).abs() < 1e-4, "{}", s.zoom);
    }
}
