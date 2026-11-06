//! What both Worlds draw over the scene in canvas pixels: pins placed
//! clear of each other, of the blocks and of the floating controls, with
//! their leader lines.

use std::fmt::Write;

use slint::Model;

use super::layout::Layout;
use super::pins::{self, Rect, Spot};
use super::scene::View;
use crate::model_sync::sync_rows;
use crate::ui::{StartModel, StartPin};

/// The floating controls, which pins must not cover.
pub fn controls(w: f64, h: f64) -> Vec<Rect> {
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

/// Screen rectangles of every block and, per area (of `n`), the one
/// around its blocks.
pub fn blocks(layout: &Layout, n: usize, v: &View) -> (Vec<Rect>, Vec<Rect>) {
    let mut per_area: Vec<Vec<(f64, f64)>> = vec![Vec::new(); n];
    let cells = layout
        .cells
        .iter()
        .map(|c| {
            let (x, y) = (c.x as f64, c.y as f64);
            let pts = [
                v.screen(x, y, 0.0),
                v.screen(x + 1.0, y, 0.0),
                v.screen(x, y + 1.0, 0.0),
                v.screen(x + 1.0, y + 1.0, 0.0),
                v.screen(x + 0.5, y + 0.5, c.z),
            ];
            per_area[c.area].extend(pts);
            Rect::around(pts)
        })
        .collect();
    (cells, per_area.into_iter().map(Rect::around).collect())
}

/// What the pins of `order` want: anchor, width, their blocks and the
/// offset they had in the window's last frame.
fn wants(
    old: &StartModel,
    rows: &[StartPin],
    anchors: &[(f64, f64)],
    areas: &[Rect],
) -> Vec<pins::Want> {
    let last = |area: i32| {
        let p = old.pins.iter().find(|p| p.area == area && !p.collapsed)?;
        let (x, y) = (
            f64::from(p.x + p.w / 2.0),
            f64::from(p.y) + pins::PIN_H / 2.0,
        );
        Some((x - f64::from(p.ax), y - f64::from(p.ay)))
    };
    rows.iter()
        .zip(anchors)
        .map(|(p, &anchor)| pins::Want {
            anchor,
            width: f64::from(p.w),
            blocks: areas[p.area as usize],
            last: last(p.area),
        })
        .collect()
}

/// The screen point above the centre of an area's plate at height `z`.
pub fn center(layout: &Layout, v: &View, area: usize, z: f64) -> (f64, f64) {
    let p = layout
        .plates
        .iter()
        .find(|p| p.area == area)
        .expect("plate per area");
    v.screen(p.x + p.w / 2.0, p.y + p.h / 2.0, z)
}

/// Place the pin `rows` at their `anchors`, clear of the blocks of the
/// `plan` (its layout and number of areas) and of each other.
pub fn place_pins(
    m: &mut StartModel,
    old: &StartModel,
    plan: (&Layout, usize),
    v: &View,
    rows: Vec<StartPin>,
    anchors: Vec<(f64, f64)>,
) {
    let (cells, areas) = blocks(plan.0, plan.1, v);
    let cover = pins::Cover::new(&cells, (v.w, v.h));
    let wants = wants(old, &rows, &anchors, &areas);
    let placed = pins::place(&wants, (v.w, v.h), controls(v.w, v.h), &cover);
    let mut leaders = String::new();
    let mut shown = Vec::new();
    for ((mut pin, spot), (ax, ay)) in rows.into_iter().zip(placed).zip(anchors) {
        (pin.ax, pin.ay) = (ax as f32, ay as f32);
        match spot {
            Spot::Off => continue,
            Spot::Collapsed => pin.collapsed = true,
            Spot::Shown(r) => {
                let (fx, fy) = r.nearest((ax, ay));
                let _ = write!(leaders, "M {fx:.1} {fy:.1} L {ax:.1} {ay:.1} ");
                (pin.x, pin.y) = ((r.x - r.w / 2.0) as f32, (r.y - r.h / 2.0) as f32);
            }
        }
        shown.push(pin);
    }
    // Dots first: the shown pins stay on top of them.
    shown.sort_by_key(|p| !p.collapsed);
    m.pins = sync_rows(&old.pins, shown);
    m.leaders = leaders.into();
}
