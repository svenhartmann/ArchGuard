//! `StartModel` for the services World: islands, pins, connections, the
//! status line and the connection details of `renderRepositoryWorld`,
//! `repositoryWorldControls` and `repositoryWorldDetails`.

use std::collections::BTreeSet;

use slint::{ModelRc, SharedString, VecModel};

use super::details::{kind, line};
use super::overlay::{center, place_pins};
use super::scene::{self, Paint, Tone};
use super::services::{Direction, Drawn, KINDS, ServicesWorld, Source, draw, tone_index};
use super::stage::{Look, fill_camera, fill_scene};
use super::state::StartState;
use crate::model_sync::sync_rows;
use crate::ui::{StartCrumb, StartData, StartEdges, StartFlow, StartLine, StartModel, StartPin};

pub(super) const SOURCES: [Source; 4] = [
    Source::All,
    Source::Code,
    Source::Configuration,
    Source::Observed,
];
pub(super) const DIRECTIONS: [Direction; 5] = [
    Direction::Both,
    Direction::Incoming,
    Direction::Outgoing,
    Direction::Downstream,
    Direction::Upstream,
];

pub(super) fn position<T: PartialEq>(all: &[T], v: &T) -> i32 {
    all.iter().position(|x| x == v).map_or(0, |i| i as i32)
}

/// The scene, pins, connections and details of the services World.
pub fn fill(m: &mut StartModel, old: &StartModel, s: &StartState, w: &ServicesWorld, root: &str) {
    let visible = w.visible();
    status(m, s, w, &visible);
    m.crumbs = sync_rows(
        &old.crumbs,
        [StartCrumb {
            label: root.into(),
            last: true,
            services: false,
        }],
    );
    let cells = &s.layout.cells;
    m.count = w.shown.len() as i32;
    m.tiles = cells.len() as i32;
    m.matching = cells.iter().filter(|c| s.matches(c)).count() as i32;
    let Some(v) = s.view() else {
        m.empty = true;
        return;
    };
    let cyclic = w.cyclic_areas(&visible);
    let look = Look {
        paint: |c: &super::layout::Cell| Paint {
            tone: Tone::Accent,
            side_alpha: 0.85,
            dimmed: !w.matches(c.area, &s.query),
        },
        flagged: |area: usize| cyclic[area],
    };
    fill_scene(m, old, s, &v, look);
    fill_camera(m, s, &v);
    if let Some(area) = w.selected.and_then(|i| w.area(i)) {
        let cells = (0..cells.len()).filter(|&i| cells[i].area == area);
        m.selection = scene::outline(&s.layout, &v.frame, s.cam.rotation, cells).into();
    }
    pins(m, old, s, w, &v, &cyclic);
    let drawn = draw(w, &visible, |a| center(&s.layout, &v, a, 0.2));
    edges(m, old, drawn);
    details(m, s, w, &visible);
}

/// Filters and the status line (`repositoryWorldControls`).
fn status(m: &mut StartModel, s: &StartState, w: &ServicesWorld, visible: &[usize]) {
    let f = &w.filter;
    let selected = w.selected.map(|i| &w.inventory[i]);
    let local: BTreeSet<&str> = w
        .shown
        .iter()
        .map(|&i| &w.inventory[i])
        .filter(|i| !i.external)
        .filter_map(|i| i.source.as_deref())
        .collect();
    let sv = &mut m.services;
    sv.shown = true;
    sv.deployment = f
        .deployment
        .as_ref()
        .and_then(|d| w.scopes.iter().position(|x| x == d))
        .map_or(0, |i| i as i32 + 1);
    sv.source = position(&SOURCES, &f.source);
    sv.kind = f.kind.map_or(0, |k| position(&KINDS, &k) + 1);
    sv.direction = position(&DIRECTIONS, &f.direction);
    (sv.cycles_only, sv.all_projects, sv.flow) = (f.cycles_only, f.all_projects, f.flow);
    sv.selected = selected
        .map_or(String::new(), |i| i.title().to_string())
        .into();
    sv.external = selected.is_some_and(|i| i.external);
    sv.openable = s.project_unit().is_some();
    sv.connections = visible.len() as i32;
    sv.cyclic = visible.iter().any(|&i| w.links[i].cyclic);
    sv.complete = w.topology().complete;
    sv.islands = w.shown.len() as i32;
    sv.local = local.len() as i32;
}

/// The pickers' entries, with their translated first entries.
pub fn texts(m: &mut StartModel, old: &StartModel, w: &ServicesWorld, d: &StartData<'_>) {
    let none = d.get_no_sources();
    let entries = w.inventory.iter().map(|i| {
        let at = if i.external {
            none.as_str()
        } else {
            i.path.as_str()
        };
        SharedString::from(format!("{} · {at}", i.title()))
    });
    m.areas = sync_rows(
        &old.areas,
        std::iter::once(d.get_open_project_entry()).chain(entries),
    );
    m.area_index = 0;
    let variants = std::iter::once(d.get_all_variants())
        .chain(w.scopes.iter().map(|s| SharedString::from(s.as_str())));
    m.services.deployments = sync_rows(&old.services.deployments, variants);
}

/// An island's kind as `StartPin.svc`: 1 application, 2 library, 3 test /
/// tooling, 4 unclassified, 5 no local sources.
pub(super) fn svc(i: &super::services::Island) -> i32 {
    match i.kind.as_str() {
        _ if i.external => 5,
        "application" => 1,
        "library" => 2,
        "tooling" => 3,
        _ => 4,
    }
}

/// One pin per island: its name and kind with its source path, or that it
/// has no local sources.
fn pins(
    m: &mut StartModel,
    old: &StartModel,
    s: &StartState,
    w: &ServicesWorld,
    v: &scene::View,
    cyclic: &[bool],
) {
    let rows = (0..w.shown.len())
        .map(|area| {
            let i = w.island(area);
            let svc = svc(i);
            let detail = i.detail();
            // English widths; German labels are about as long.
            let label = [
                "",
                "Application",
                "Library",
                "Test / tooling",
                "Unclassified project",
            ];
            let sub = match svc {
                5 => "No local sources".to_string(),
                k if detail.is_empty() => label[k as usize].to_string(),
                k => format!("{} · {detail}", label[k as usize]),
            };
            StartPin {
                area: area as i32,
                name: i.label().into(),
                module: !i.external,
                entries: 1,
                cyclic: cyclic[area],
                svc,
                path: detail.as_str().into(),
                w: super::pins::width(i.label(), &sub, !i.external) as f32,
                ..Default::default()
            }
        })
        .collect();
    let anchors = (0..w.shown.len())
        .map(|a| center(&s.layout, v, a, 0.0))
        .collect();
    place_pins(m, old, (&s.layout, w.shown.len()), v, rows, anchors);
}

/// The connections, one row per tone, and the flow curves.
fn edges(m: &mut StartModel, old: &StartModel, drawn: Drawn) {
    let strokes = drawn
        .strokes
        .into_iter()
        .enumerate()
        .map(|(tone, s)| StartEdges {
            curves: s.curves.into(),
            heads: s.heads.into(),
            tone: tone as i32,
        });
    m.services.edges = sync_rows(&old.services.edges, strokes);
    let flows = drawn.flows.into_iter().map(|f| StartFlow {
        px: f.p.0 as f32,
        py: f.p.1 as f32,
        cx: f.c.0 as f32,
        cy: f.c.1 as f32,
        qx: f.q.0 as f32,
        qy: f.q.1 as f32,
        length: f.length as f32,
        offset: f.offset as f32,
        count: f.count,
        tone: tone_index(f.tone) as i32,
    });
    m.services.flows = sync_rows(&old.services.flows, flows);
}

/// `repositoryWorldDetails`: every visible connection with its evidence
/// and its neighbour.
fn details(m: &mut StartModel, s: &StartState, w: &ServicesWorld, visible: &[usize]) {
    let Some(selected) = w.selected else { return };
    let island = &w.inventory[selected];
    m.has_selection = true;
    m.details_open = s.details_open;
    m.detail_title = island.name.as_str().into();
    let mut lines = Vec::new();
    if island.external {
        lines.push(line(kind::SVC_EXTERNAL, "", "", ""));
    }
    lines.push(line(kind::SVC_NOTE, "", "", ""));
    for &i in visible.iter().take(s.limit) {
        let l = &w.links[i];
        lines.push(StartLine {
            entity: position(&KINDS, &l.kind),
            flag: l.cyclic,
            ..line(
                kind::SVC_LINK,
                w.inventory[l.from].title(),
                w.inventory[l.to].title(),
                "",
            )
        });
        for proof in &l.evidence {
            let at = proof.line.map_or(String::new(), |n| format!(":{n}"));
            let pointer = Some(&proof.pointer)
                .filter(|p| !p.is_empty())
                .map_or(String::new(), |p| format!(" · {p}"));
            lines.push(line(
                kind::SVC_AT,
                format!("{}{at}{pointer}", proof.file),
                "",
                "",
            ));
            lines.push(line(kind::MUTED, proof.description.as_str(), "", ""));
        }
        let other = if l.from == selected { l.to } else { l.from };
        lines.push(StartLine {
            entity: other as i32,
            ..line(kind::SVC_NEIGHBOR, "", "", "")
        });
    }
    if visible.len() > s.limit {
        lines.push(line(
            kind::MORE,
            (visible.len() - s.limit).to_string(),
            "",
            "",
        ));
    }
    if visible.is_empty() {
        lines.push(line(kind::SVC_NONE, "", "", ""));
    }
    m.detail_lines = ModelRc::new(VecModel::from(lines));
}
