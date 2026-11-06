//! `StartModel` from the tab state: the scene, the pins and the texts of
//! `renderWorld`, `worldStatus` and `worldDraw`.

use slint::{ComponentHandle, SharedString};

use super::details;
use super::layout::Cell;
use super::overlay::{center, place_pins};
use super::pins;
use super::scene::{self, Paint, Tone, View};
use super::services_view;
use super::stage::{Look, fill_camera, fill_scene};
use super::state::{Lens, Mode, StartState};
use crate::format::Locale;
use crate::kpi;
use crate::model_sync::sync_rows;
use crate::tab_support::Texts;
use crate::ui::{AppWindow, StartCrumb, StartData, StartModel, StartPin};

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
        Tone::Gap
    } else {
        Tone::Covered
    }
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
        search: s.search.as_str().into(),
        ..Default::default()
    };
    m.services.available = s.services.is_some();
    if let (Some(w), Mode::Services) = (&s.services, s.mode) {
        services_view::fill(&mut m, &old, s, w, &ui.get_project());
        services_view::texts(&mut m, &old, w, &d);
        d.set_model(m);
        return;
    }
    fill_texts(&mut m, &old, s, &Texts::of(ui), &ui.get_project());
    if let Some(v) = s.view() {
        let look = Look {
            paint: |c: &Cell| paint(s, c),
            flagged: |area: usize| {
                let a = &s.scope.areas[area];
                s.lens == Lens::Problems && (a.cyclic || !a.findings.is_empty())
            },
        };
        fill_scene(&mut m, &old, s, &v, look);
        fill_camera(&mut m, s, &v);
        fill_outline(&mut m, s, &v);
        fill_overlay(&mut m, &old, s, &v, Texts::of(ui).locale());
    } else {
        m.empty = true;
    }
    details::fill(&mut m, s, Texts::of(ui).locale());
    d.set_model(m);
}

/// The roof outline of the selected entity or area.
fn fill_outline(m: &mut StartModel, s: &StartState, v: &View) {
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

/// The pin's maximum as `start_world.slint` writes it (English, for the
/// width): "max 4" for complexity, "CBO max. 4" for a class KPI.
fn max_mark(kpi: usize, max: &str) -> String {
    const KPIS: [&str; 6] = ["CBO", "RFC", "DIT", "NOC", "WMC", "LCOM4"];
    match kpi.checked_sub(1).and_then(|k| KPIS.get(k)) {
        Some(label) => format!("{label} max. {max}"),
        None => format!("max {max}"),
    }
}

fn pin_row(l: Locale, s: &StartState, area: usize, max: Option<f64>) -> (StartPin, String) {
    let a = &s.scope.areas[area];
    let name = a.name.rsplit('/').next().unwrap_or(&a.name).to_string();
    let max = max
        .filter(|_| s.lens == Lens::Explore)
        .map(|v| l.num(v, 2))
        .unwrap_or_default();
    let mut marks = Vec::new();
    if a.cyclic {
        marks.push("Package cycle".to_string());
    }
    if !a.findings.is_empty() {
        marks.push(format!("{} findings", a.findings.len()));
    }
    if !max.is_empty() {
        marks.push(max_mark(s.kpi, &max));
    }
    let sub = if marks.is_empty() {
        format!("{} elements", l.count(a.entities.len()))
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

/// Pins placed clear of each other and of the blocks, their leader lines
/// and the cycle arrows of the Problems lens.
fn fill_overlay(m: &mut StartModel, old: &StartModel, s: &StartState, v: &View, l: Locale) {
    let center = |area: usize, z: f64| center(&s.layout, v, area, z);
    let max = area_max(s);
    let order = pinned(s, &max);
    let rows: Vec<StartPin> = order.iter().map(|&a| pin_row(l, s, a, max[a]).0).collect();
    let anchors: Vec<(f64, f64)> = order.iter().map(|&a| center(a, 0.0)).collect();
    place_pins(m, old, (&s.layout, s.scope.areas.len()), v, rows, anchors);
    if s.lens == Lens::Problems {
        let (mut curves, mut heads) = (String::new(), String::new());
        for c in s.scope.connections.iter().filter(|c| c.cyclic).take(60) {
            super::arrows::arrow(
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
fn fill_texts(m: &mut StartModel, old: &StartModel, s: &StartState, texts: &Texts, root: &str) {
    let crumbs = s.crumbs(root);
    let last = crumbs.len().saturating_sub(1);
    // With a services World, its crumb leads (`renderWorldPath`).
    let lead = s.services.is_some().then(|| StartCrumb {
        services: true,
        ..Default::default()
    });
    m.crumbs = sync_rows(
        &old.crumbs,
        lead.into_iter().chain(
            crumbs
                .into_iter()
                .enumerate()
                .map(|(i, (label, _))| StartCrumb {
                    label: label.into(),
                    last: i == last,
                    services: false,
                }),
        ),
    );
    m.cycle_groups = kpi::AREA_CYCLE_GROUPS.item(texts, s.scope.cycle_groups);
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
        m.top_value = texts.locale().num(v, 2).into();
        m.top_area = area.as_str().into();
    }
    let mut areas = vec![SharedString::from(texts.t("Select area…"))];
    areas.extend(
        s.scope
            .areas
            .iter()
            .map(|a| SharedString::from(a.name.as_str())),
    );
    m.areas = sync_rows(&old.areas, areas);
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

    /// The width estimate names the shown KPI, as the pin does.
    #[test]
    fn the_pin_maximum_names_the_kpi() {
        assert_eq!(max_mark(0, "4"), "max 4");
        assert_eq!(max_mark(6, "1.5"), "LCOM4 max. 1.5");
        assert_eq!(max_mark(2, "9"), "RFC max. 9");
    }

    #[test]
    fn complexity_colors_run_from_teal_to_orange() {
        // hsl(175 48% 55%) and the floor hsl(15 48% 55%).
        assert_eq!(complexity_color(0.0), Tone::Rgb(85, 195, 186));
        assert_eq!(complexity_color(1e9), Tone::Rgb(195, 113, 85));
    }
}
