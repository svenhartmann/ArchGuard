//! `StartModel` from the tab state: the scene, the pins and the texts of
//! `renderWorld`, `worldStatus` and `worldDraw`.

use std::f64::consts::PI;
use std::fmt::Write;

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use super::details;
use super::layout::Cell;
use super::pins::{self, Rect};
use super::scene::{self, Face, Paint, Tone, View};
use super::state::{Lens, StartState, number};
use crate::ui::{AppWindow, StartCrumb, StartData, StartFace, StartModel, StartPin, StartPlate};

/// `worldComplexityColor`: hsl(175 → 15, 48 %, 55 %) on a log scale.
pub fn complexity_color(value: f64) -> Tone {
    let hue = (175.0 - (value + 1.0).log2() * 24.0).max(15.0);
    let (s, l) = (0.48, 0.55);
    let c = (1.0 - (2.0 * l - 1.0f64).abs()) * s;
    let x = c * (1.0 - ((hue / 60.0) % 2.0 - 1.0).abs());
    let (r, g, b) = match (hue / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        _ => (0.0, x, c),
    };
    let m = l - c / 2.0;
    let byte = |v: f64| ((v + m) * 255.0).round() as u8;
    Tone::Rgb(byte(r), byte(g), byte(b))
}

/// The surface colour of a cell in the current lens.
fn paint(s: &StartState, cell: &Cell) -> Paint {
    let tone = match s.lens {
        Lens::Problems => Tone::Accent,
        Lens::Explore => match cell.value {
            None => Tone::Muted,
            Some(_) if s.kpi_active() => kpi_tone(s, cell),
            Some(v) => complexity_color(v),
        },
        Lens::Coverage => coverage_tone(s, cell),
    };
    let side_alpha = if s.lens == Lens::Explore && cell.value.is_none() {
        0.35
    } else {
        0.85
    };
    Paint {
        tone,
        side_alpha,
        dimmed: !s.matches(cell),
    }
}

/// `worldKpiCellState`: the rating of the member with the highest value.
fn kpi_tone(s: &StartState, cell: &Cell) -> Tone {
    use super::evidence::Rating;
    let best = cell
        .entities
        .iter()
        .filter_map(|&e| {
            let (v, r) = s.ev.kpi(e, s.kpi);
            v.map(|v| (v, r))
        })
        .fold(None, |best: Option<(f64, Rating)>, (v, r)| match best {
            Some((b, _)) if v <= b => best,
            _ => Some((v, r)),
        });
    match best.map(|b| b.1) {
        Some(Rating::Low) => Tone::Low,
        Some(Rating::Medium) => Tone::Medium,
        Some(Rating::High) => Tone::High,
        Some(Rating::Info) => Tone::Accent,
        _ => Tone::Muted,
    }
}

/// Red with a gap, green when fully covered, gray unless every member has
/// usable file coverage.
fn coverage_tone(s: &StartState, cell: &Cell) -> Tone {
    let known: Vec<_> = cell
        .entities
        .iter()
        .filter_map(|&e| s.ev.entity(e).coverage.as_ref()?.lines.clone())
        .collect();
    if known.is_empty() || known.len() != cell.entities.len() {
        return Tone::Muted;
    }
    if known.iter().any(|l| l.covered < l.total) {
        Tone::Cycle
    } else {
        Tone::Covered
    }
}

fn face_row(f: Face) -> StartFace {
    let (tone, rgb) = match f.tone {
        Tone::Accent => (0, (0, 0, 0)),
        Tone::Muted => (1, (0, 0, 0)),
        Tone::Cycle => (2, (0, 0, 0)),
        Tone::Covered => (3, (0, 0, 0)),
        Tone::Low => (4, (0, 0, 0)),
        Tone::Medium => (5, (0, 0, 0)),
        Tone::High => (6, (0, 0, 0)),
        Tone::Rgb(r, g, b) => (7, (r, g, b)),
    };
    StartFace {
        path: f.path.into(),
        tone,
        rgb: slint::Color::from_rgb_u8(rgb.0, rgb.1, rgb.2),
        shade: f.shade,
        alpha: f.alpha,
        stroke: f.stroke,
    }
}

/// Replace the rows of a model in place, so unchanged rows keep their
/// items and the repeater never rebuilds all faces at once.
fn sync<T: Clone + 'static>(old: &ModelRc<T>, rows: Vec<T>) -> ModelRc<T> {
    let Some(vec) = old.as_any().downcast_ref::<VecModel<T>>() else {
        return ModelRc::new(VecModel::from(rows));
    };
    let keep = vec.row_count().min(rows.len());
    while vec.row_count() > rows.len() {
        vec.remove(vec.row_count() - 1);
    }
    let mut rows = rows.into_iter();
    for i in 0..keep {
        vec.set_row_data(i, rows.next().expect("row"));
    }
    for row in rows {
        vec.push(row);
    }
    old.clone()
}

/// Everything that changes the faces; pans and zooms do not.
fn scene_key(s: &StartState) -> String {
    format!(
        "{:?}|{:?}|{}|{:.5}|{}",
        s.within, s.lens, s.kpi, s.cam.rotation, s.query
    )
}

/// Faces, plates and lattice: reused from the window when the key matches.
fn fill_scene(m: &mut StartModel, old: &StartModel, s: &StartState, v: &View) {
    m.scene_key = scene_key(s).into();
    if old.scene_key == m.scene_key {
        m.faces = old.faces.clone();
        m.plates = old.plates.clone();
        m.lattice = old.lattice.clone();
        return;
    }
    let rot = s.cam.rotation;
    let faces = scene::faces(&s.layout, &v.frame, rot, |c| paint(s, c));
    m.faces = sync(&old.faces, faces.into_iter().map(face_row).collect());
    let plates = s
        .layout
        .plates
        .iter()
        .map(|p| {
            let a = &s.scope.areas[p.area];
            StartPlate {
                path: scene::plate(&v.frame, p, rot).into(),
                flagged: s.lens == Lens::Problems && (a.cyclic || !a.findings.is_empty()),
            }
        })
        .collect();
    m.plates = sync(&old.plates, plates);
    m.lattice = scene::lattice(&v.frame, rot).into();
}

/// Show the state in the window.
pub fn push(ui: &AppWindow, s: &StartState) {
    let d = ui.global::<StartData>();
    let old = d.get_model();
    let mut m = StartModel {
        ready: true,
        lens: match s.lens {
            Lens::Problems => 0,
            Lens::Explore => 1,
            Lens::Coverage => 2,
        },
        kpi: s.kpi as i32,
        canvas_w: s.canvas.0 as f32,
        canvas_h: s.canvas.1 as f32,
        can_back: !s.history.is_empty(),
        query: s.query.as_str().into(),
        ..Default::default()
    };
    fill_texts(&mut m, s, &d.get_label_select_area(), &ui.get_project());
    if let Some(v) = s.view() {
        fill_scene(&mut m, &old, s, &v);
        fill_camera(&mut m, s, &v);
        fill_overlay(&mut m, s, &v);
    } else {
        m.empty = true;
    }
    details::fill(&mut m, s);
    d.set_model(m);
}

fn fill_camera(m: &mut StartModel, s: &StartState, v: &View) {
    let [x, y, w, h] = v.viewbox();
    (m.vb_x, m.vb_y, m.vb_w, m.vb_h) = (x as f32, y as f32, w as f32, h as f32);
    m.zoom = (s.cam.zoom * 100.0).round() as i32;
    m.angle = ((s.cam.rotation * 180.0 / PI).round() as i32).rem_euclid(360);
    // The bearing arrow of the orientation control, turned with the world.
    let (sin, cos) = s.cam.rotation.sin_cos();
    let mut bearing = String::new();
    for (i, (px, py)) in [(12.0, 2.0), (19.0, 21.0), (12.0, 17.0), (5.0, 21.0)]
        .iter()
        .enumerate()
    {
        let (dx, dy) = (px - 12.0, py - 12.0);
        let (rx, ry) = (12.0 + dx * cos - dy * sin, 12.0 + dx * sin + dy * cos);
        let _ = write!(
            bearing,
            "{} {rx:.2} {ry:.2} ",
            if i == 0 { "M" } else { "L" }
        );
    }
    bearing.push('Z');
    m.bearing = bearing.into();
    if let Some(sel) = &s.selection {
        let cells = (0..s.layout.cells.len()).filter(|&i| {
            let c = &s.layout.cells[i];
            match sel.entity {
                Some(e) => c.entities.contains(&e),
                None => c.area == sel.area,
            }
        });
        m.selection = scene::outline(&s.layout, &v.frame, s.cam.rotation, cells).into();
    }
}

/// Highest cell value per area (`areaMax`).
fn area_max(s: &StartState) -> Vec<Option<f64>> {
    let mut max = vec![None; s.scope.areas.len()];
    for c in &s.layout.cells {
        if let Some(v) = c.value {
            let m: &mut Option<f64> = &mut max[c.area];
            *m = Some(m.map_or(v, |m| m.max(v)));
        }
    }
    max
}

/// Pins in the web's order: in Explore by highest value, else cycles and
/// findings first, then by size.
fn pinned(s: &StartState, max: &[Option<f64>]) -> Vec<usize> {
    let areas = &s.scope.areas;
    let mut order: Vec<usize> = (0..areas.len())
        .filter(|&a| !areas[a].entities.is_empty())
        .collect();
    if s.lens == Lens::Explore {
        order.sort_by(|&a, &b| {
            let v = |i: usize| max[i].unwrap_or(-1.0);
            v(b).total_cmp(&v(a))
                .then_with(|| areas[a].name.cmp(&areas[b].name))
        });
    } else {
        let flagged = |i: usize| areas[i].cyclic || !areas[i].findings.is_empty();
        order.sort_by(|&a, &b| {
            flagged(b)
                .cmp(&flagged(a))
                .then(areas[b].entities.len().cmp(&areas[a].entities.len()))
        });
    }
    order
}

/// The floating controls, which pins must not cover.
fn controls(w: f64, h: f64) -> Vec<Rect> {
    vec![
        Rect {
            x: w - 12.0 - 80.0,
            y: 12.0 + 23.0,
            w: 160.0,
            h: 46.0,
        },
        Rect {
            x: w / 2.0,
            y: h - 12.0 - 23.0,
            w: 330.0,
            h: 46.0,
        },
    ]
}

fn pin_row(s: &StartState, area: usize, max: Option<f64>) -> (StartPin, String) {
    let a = &s.scope.areas[area];
    let name = a.name.rsplit('/').next().unwrap_or(&a.name).to_string();
    let max = max
        .filter(|_| s.lens == Lens::Explore)
        .map(number)
        .unwrap_or_default();
    let mut marks = Vec::new();
    if a.cyclic {
        marks.push("Package cycle".to_string());
    }
    if !a.findings.is_empty() {
        marks.push(format!("{} findings", a.findings.len()));
    }
    if !max.is_empty() {
        marks.push(format!("CBO max. {max}"));
    }
    let sub = if marks.is_empty() {
        format!("{} entries", a.entities.len())
    } else {
        marks.join(" · ")
    };
    let pin = StartPin {
        area: area as i32,
        name: name.as_str().into(),
        module: a.module,
        entries: a.entities.len() as i32,
        cyclic: a.cyclic,
        findings: a.findings.len() as i32,
        max: max.into(),
        w: pins::width(&name, &sub, a.module) as f32,
        ..Default::default()
    };
    (pin, name)
}

/// Pins with their leader lines, and the cycle arrows of the Problems lens.
fn fill_overlay(m: &mut StartModel, s: &StartState, v: &View) {
    let center = |area: usize, z: f64| {
        let p = s
            .layout
            .plates
            .iter()
            .find(|p| p.area == area)
            .expect("plate per area");
        v.screen(p.x + p.w / 2.0, p.y + p.h / 2.0, z)
    };
    let max = area_max(s);
    let order = pinned(s, &max);
    let rows: Vec<StartPin> = order.iter().map(|&a| pin_row(s, a, max[a]).0).collect();
    let anchors: Vec<(f64, f64)> = order.iter().map(|&a| center(a, 0.0)).collect();
    let widths: Vec<f64> = rows.iter().map(|p| f64::from(p.w)).collect();
    let placed = pins::place(&anchors, &widths, (v.w, v.h), controls(v.w, v.h));
    let mut leaders = String::new();
    let mut shown = Vec::new();
    for ((mut pin, slot), anchor) in rows.into_iter().zip(placed).zip(anchors) {
        let Some(r) = slot else { continue };
        let _ = write!(
            leaders,
            "M {:.1} {:.1} L {:.1} {:.1} ",
            r.x,
            r.y + r.h / 2.0,
            anchor.0,
            anchor.1
        );
        (pin.x, pin.y) = ((r.x - r.w / 2.0) as f32, (r.y - r.h / 2.0) as f32);
        shown.push(pin);
    }
    m.pins = ModelRc::new(VecModel::from(shown));
    m.leaders = leaders.into();
    if s.lens == Lens::Problems {
        let (mut curves, mut heads) = (String::new(), String::new());
        for c in s.scope.connections.iter().filter(|c| c.cyclic).take(60) {
            scene::arrow(
                center(c.from, 0.2),
                center(c.to, 0.2),
                &mut curves,
                &mut heads,
            );
        }
        (m.arrows, m.heads) = (curves.into(), heads.into());
    }
}

/// Breadcrumbs, the status line, counts and the area picker.
fn fill_texts(m: &mut StartModel, s: &StartState, select_area: &str, root: &str) {
    let crumbs = s.crumbs(root);
    let last = crumbs.len().saturating_sub(1);
    m.crumbs = ModelRc::new(VecModel::from(
        crumbs
            .into_iter()
            .enumerate()
            .map(|(i, (label, _))| StartCrumb {
                label: label.into(),
                last: i == last,
            })
            .collect::<Vec<_>>(),
    ));
    m.cycle_groups = s.scope.cycle_groups as i32;
    let cells = &s.layout.cells;
    m.count = cells.iter().map(|c| c.entities.len()).sum::<usize>() as i32;
    m.tiles = cells.len() as i32;
    m.matching = cells.iter().filter(|c| s.matches(c)).count() as i32;
    fill_coverage(m, s);
    let top = cells
        .iter()
        .filter_map(|c| c.value.map(|v| (v, &s.scope.areas[c.area].name)))
        .max_by(|a, b| a.0.total_cmp(&b.0).then_with(|| b.1.cmp(a.1)));
    if let Some((v, area)) = top {
        m.top_value = number(v).into();
        m.top_area = area.as_str().into();
    }
    let mut areas = vec![SharedString::from(select_area)];
    areas.extend(
        s.scope
            .areas
            .iter()
            .map(|a| SharedString::from(a.name.as_str())),
    );
    m.areas = ModelRc::new(VecModel::from(areas));
    m.area_index = s.selection.as_ref().map_or(0, |sel| sel.area as i32 + 1);
}

/// `worldStatus` for the Test gaps lens: surfaces with gaps and lines.
fn fill_coverage(m: &mut StartModel, s: &StartState) {
    let known: Vec<_> = s
        .layout
        .cells
        .iter()
        .flat_map(|c| &c.entities)
        .filter_map(|&e| s.ev.entity(e).coverage.as_ref()?.lines.clone())
        .collect();
    m.coverage_known = !known.is_empty();
    m.covered_lines = known.iter().map(|l| l.covered).sum::<u64>() as i32;
    m.total_lines = known.iter().map(|l| l.total).sum::<u64>() as i32;
    m.gaps = known.iter().filter(|l| l.covered < l.total).count() as i32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complexity_colors_run_from_teal_to_orange() {
        // hsl(175 48% 55%) and the floor hsl(15 48% 55%).
        assert_eq!(complexity_color(0.0), Tone::Rgb(85, 195, 186));
        assert_eq!(complexity_color(1e9), Tone::Rgb(195, 113, 85));
        assert_eq!(number(12.0), "12");
        assert_eq!(number(0.5), "0.50");
    }
}
