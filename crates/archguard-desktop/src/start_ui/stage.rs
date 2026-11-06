//! The scene of both Worlds in the window: faces, plates and lattice in
//! unit coordinates, and the camera's viewbox onto them.

use std::f64::consts::PI;
use std::fmt::Write;

use super::layout::Cell;
use super::scene::{self, Face, Paint, Tone, View};
use super::state::{Mode, StartState};
use crate::model_sync::sync_rows;
use crate::ui::{StartFace, StartModel, StartPlate};

pub(super) fn face_row(f: Face) -> StartFace {
    let (tone, rgb) = match f.tone {
        Tone::Accent => (0, (0, 0, 0)),
        Tone::Muted => (1, (0, 0, 0)),
        Tone::Gap => (2, (0, 0, 0)),
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

/// Everything that changes the faces; pans and zooms do not.
fn scene_key(s: &StartState) -> String {
    let services = match (&s.services, s.mode) {
        (Some(w), Mode::Services) => format!("{:?}|{:?}", w.filter, w.selected),
        _ => String::new(),
    };
    format!(
        "{:?}|{:?}|{:?}|{}|{:.5}|{}|{services}",
        s.mode, s.within, s.lens, s.kpi, s.cam.rotation, s.query
    )
}

/// How the faces of a scene are painted: per cell, and whether a plate
/// (by area) is marked red.
pub(super) struct Look<P: Fn(&Cell) -> Paint, F: Fn(usize) -> bool> {
    pub paint: P,
    pub flagged: F,
}

/// Faces, plates and lattice: reused from the window when the key matches.
pub(super) fn fill_scene<P: Fn(&Cell) -> Paint, F: Fn(usize) -> bool>(
    m: &mut StartModel,
    old: &StartModel,
    s: &StartState,
    v: &View,
    look: Look<P, F>,
) {
    m.scene_key = scene_key(s).into();
    if old.scene_key == m.scene_key {
        m.faces = old.faces.clone();
        m.plates = old.plates.clone();
        m.lattice = old.lattice.clone();
        return;
    }
    let rot = s.cam.rotation;
    let faces = scene::faces(&s.layout, &v.frame, rot, look.paint);
    m.faces = sync_rows(&old.faces, faces.into_iter().map(face_row));
    let plates = s
        .layout
        .plates
        .iter()
        .map(|p| StartPlate {
            path: scene::plate(&v.frame, p, rot).into(),
            flagged: (look.flagged)(p.area),
        })
        .collect::<Vec<_>>();
    m.plates = sync_rows(&old.plates, plates);
    m.lattice = scene::lattice(&v.frame, rot).into();
}

pub(super) fn fill_camera(m: &mut StartModel, s: &StartState, v: &View) {
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
}
