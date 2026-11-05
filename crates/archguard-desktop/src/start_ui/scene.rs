//! The isometric projection of the world (`worldProject` and `worldDraw` in
//! the web viewer), computed here because Slint has no 3D. Geometry is
//! emitted as SVG path commands in "unit" coordinates (projected ground
//! units, centred on the plan); the view maps them to pixels with a
//! viewbox, so panning and zooming never rebuild the faces.

use std::collections::HashMap;
use std::f64::consts::{FRAC_PI_2, SQRT_2};
use std::fmt::Write;

use super::layout::{Cell, Layout};

/// `worldProject`: ground (x, y) and height z to the screen plane.
pub fn project(x: f64, y: f64, z: f64, rotation: f64) -> (f64, f64) {
    let (s, c) = rotation.sin_cos();
    let rx = x * c - y * s;
    let ry = x * s + y * c;
    ((rx - ry) * 0.86, (rx + ry) * 0.44 - z)
}

/// Camera relative to the fitted view's centre, as `worldUi.camera`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    pub x: f64,
    pub y: f64,
    pub zoom: f64,
    pub rotation: f64,
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            x: 0.0,
            y: 0.0,
            zoom: 1.0,
            rotation: 0.0,
        }
    }
}

/// `worldZoom`: zoom by `factor` keeping the point below the cursor, given
/// relative to the canvas centre. The zoom stays within 40 % and 800 %.
pub fn zoom(cam: Camera, factor: f64, point: (f64, f64)) -> Camera {
    let zoom = (cam.zoom * factor).clamp(0.4, 8.0);
    let ratio = zoom / cam.zoom;
    Camera {
        x: point.0 - (point.0 - cam.x) * ratio,
        y: point.1 - (point.1 - cam.y) * ratio,
        zoom,
        ..cam
    }
}

/// `worldRotate`: turn by `delta`, normalised to [0, 2π).
pub fn rotate(cam: Camera, delta: f64) -> Camera {
    Camera {
        rotation: (cam.rotation + delta).rem_euclid(std::f64::consts::TAU),
        ..cam
    }
}

/// The plan's extent and the scale that fits it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub minx: f64,
    pub maxx: f64,
    pub miny: f64,
    pub maxy: f64,
    pub cx: f64,
    pub cy: f64,
    pub radius: f64,
    pub max_z: f64,
}

impl Frame {
    pub fn of(layout: &Layout) -> Option<Frame> {
        let first = layout.plates.first()?;
        let mut f = Frame {
            minx: first.x,
            maxx: first.x + first.w,
            miny: first.y,
            maxy: first.y + first.h,
            cx: 0.0,
            cy: 0.0,
            radius: 1.0,
            max_z: layout.cells.iter().map(|c| c.z).fold(0.0, f64::max),
        };
        for p in &layout.plates {
            f.minx = f.minx.min(p.x);
            f.maxx = f.maxx.max(p.x + p.w);
            f.miny = f.miny.min(p.y);
            f.maxy = f.maxy.max(p.y + p.h);
        }
        f.cx = (f.minx + f.maxx) / 2.0;
        f.cy = (f.miny + f.maxy) / 2.0;
        f.radius = ((f.maxx - f.minx).hypot(f.maxy - f.miny) / 2.0).max(1.0);
        Some(f)
    }

    /// Pixels per unit at zoom 1: a fixed enclosing radius keeps the scale
    /// stable through a full turn.
    pub fn base(&self, w: f64, h: f64) -> f64 {
        let a = (w - 60.0) / (self.radius * SQRT_2 * 1.72);
        let b = (h - 150.0) / (self.radius * SQRT_2 * 0.88 + self.max_z);
        a.min(b).max(1e-3)
    }

    /// Unit coordinates of a ground point.
    pub fn unit(&self, x: f64, y: f64, z: f64, rotation: f64) -> (f64, f64) {
        let (px, py) = project(x - self.cx, y - self.cy, z, rotation);
        (px, py + self.max_z / 2.0)
    }
}

/// A canvas of `w` × `h` pixels looking at a frame through a camera.
#[derive(Debug, Clone, Copy)]
pub struct View {
    pub frame: Frame,
    pub w: f64,
    pub h: f64,
    pub cam: Camera,
}

impl View {
    pub fn scale(&self) -> f64 {
        self.frame.base(self.w, self.h) * self.cam.zoom
    }

    pub fn to_screen(self, u: (f64, f64)) -> (f64, f64) {
        let s = self.scale();
        (
            self.w / 2.0 + self.cam.x + u.0 * s,
            self.h / 2.0 + self.cam.y + u.1 * s,
        )
    }

    pub fn to_unit(self, p: (f64, f64)) -> (f64, f64) {
        let s = self.scale();
        (
            (p.0 - self.w / 2.0 - self.cam.x) / s,
            (p.1 - self.h / 2.0 - self.cam.y) / s,
        )
    }

    pub fn screen(&self, x: f64, y: f64, z: f64) -> (f64, f64) {
        self.to_screen(self.frame.unit(x, y, z, self.cam.rotation))
    }

    /// The unit rectangle the canvas shows: x, y, width, height.
    pub fn viewbox(&self) -> [f64; 4] {
        let s = self.scale();
        let (x, y) = self.to_unit((0.0, 0.0));
        [x, y, self.w / s, self.h / s]
    }
}

/// The colour role of a face; resolved against the theme in the view.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tone {
    Accent,
    Muted,
    Cycle,
    Covered,
    Low,
    Medium,
    High,
    /// A measured value on the continuous complexity scale.
    Rgb(u8, u8, u8),
}

/// One filled polygon (or several with the same colour) in unit space.
#[derive(Debug, Clone, PartialEq)]
pub struct Face {
    pub path: String,
    pub tone: Tone,
    /// Positive lightens, negative darkens (the web's light overlays).
    pub shade: f32,
    /// Opacity of the tone over the panel background.
    pub alpha: f32,
    /// Top facets carry the lattice stroke; sides do not.
    pub stroke: bool,
}

/// How a cell is painted: its tone and whether a search dims it.
pub struct Paint {
    pub tone: Tone,
    pub side_alpha: f64,
    pub dimmed: bool,
}

const GROUND: f64 = -0.12;

fn poly(out: &mut String, pts: &[(f64, f64)]) {
    for (i, (x, y)) in pts.iter().enumerate() {
        let _ = write!(out, "{} {x:.3} {y:.3} ", if i == 0 { "M" } else { "L" });
    }
    out.push_str("Z ");
}

/// Cells in painter order: far (small projected depth) first.
pub fn depth_order(layout: &Layout, rotation: f64) -> Vec<usize> {
    let depth = |c: &Cell| project(c.x as f64 + 0.5, c.y as f64 + 0.5, 0.0, rotation).1;
    let mut order: Vec<usize> = (0..layout.cells.len()).collect();
    order.sort_by(|&a, &b| depth(&layout.cells[a]).total_cmp(&depth(&layout.cells[b])));
    order
}

/// Grid position -> cell index.
fn grid(layout: &Layout) -> HashMap<(i64, i64), usize> {
    layout
        .cells
        .iter()
        .enumerate()
        .map(|(i, c)| ((c.x, c.y), i))
        .collect()
}

/// The four ground corners of a cell with interpolated heights, and its peak.
struct Roof {
    corners: [(f64, f64); 4],
    peak: (f64, f64),
}

fn roof(
    layout: &Layout,
    at: &HashMap<(i64, i64), usize>,
    frame: &Frame,
    cell: &Cell,
    rotation: f64,
) -> Roof {
    let corner = |x: i64, y: i64| {
        let mut sum = 0.0;
        for (dx, dy) in [(-1, -1), (-1, 0), (0, -1), (0, 0)] {
            if let Some(&n) = at.get(&(x + dx, y + dy))
                && layout.cells[n].area == cell.area
            {
                sum += layout.cells[n].z;
            }
        }
        sum / 4.0
    };
    let (x, y) = (cell.x, cell.y);
    let corners = [(x, y), (x + 1, y), (x + 1, y + 1), (x, y + 1)]
        .map(|(cx, cy)| frame.unit(cx as f64, cy as f64, corner(cx, cy), rotation));
    let peak = frame.unit(x as f64 + 0.5, y as f64 + 0.5, cell.z, rotation);
    Roof { corners, peak }
}

/// Every face of the world in painter order. Sides that face away from the
/// viewer or touch a cell of the same area are culled; the visible sides of
/// a cell share one face.
pub fn faces(
    layout: &Layout,
    frame: &Frame,
    rotation: f64,
    paint: impl Fn(&Cell) -> Paint,
) -> Vec<Face> {
    let at = grid(layout);
    let order = depth_order(layout, rotation);
    let depth = |c: &Cell| project(c.x as f64 + 0.5, c.y as f64 + 0.5, 0.0, rotation).1;
    let (min_d, max_d) = order
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &i| {
            let d = depth(&layout.cells[i]);
            (lo.min(d), hi.max(d))
        });
    let span = if max_d > min_d { max_d - min_d } else { 1.0 };
    let mut out = Vec::with_capacity(layout.cells.len() * 5);
    for i in order {
        let cell = &layout.cells[i];
        let p = paint(cell);
        let fog = 0.72 + 0.28 * (depth(cell) - min_d) / span;
        let r = roof(layout, &at, frame, cell, rotation);
        if let Some(side) = sides(layout, &at, frame, cell, &r, rotation) {
            let alpha = if p.dimmed { 0.12 } else { p.side_alpha } * fog;
            out.push(Face {
                path: side,
                tone: p.tone,
                shade: -0.16,
                alpha: alpha as f32,
                stroke: false,
            });
        }
        for k in 0..4 {
            let light = (rotation + (k as f64 - 0.5) * FRAC_PI_2).cos();
            let shade = if light > 0.0 {
                0.05 + 0.14 * light
            } else {
                -(0.05 - 0.18 * light)
            };
            let mut path = String::new();
            poly(&mut path, &[r.corners[k], r.corners[(k + 1) % 4], r.peak]);
            out.push(Face {
                path,
                tone: p.tone,
                shade: shade as f32,
                alpha: if p.dimmed { 0.12 } else { 1.0 },
                stroke: true,
            });
        }
    }
    out
}

fn sides(
    layout: &Layout,
    at: &HashMap<(i64, i64), usize>,
    frame: &Frame,
    cell: &Cell,
    r: &Roof,
    rotation: f64,
) -> Option<String> {
    let (x, y) = (cell.x, cell.y);
    let ground = [(x, y), (x + 1, y), (x + 1, y + 1), (x, y + 1)]
        .map(|(cx, cy)| frame.unit(cx as f64, cy as f64, GROUND, rotation));
    let mut path = String::new();
    for (i, j, dx, dy) in [(0, 1, 0, -1), (1, 2, 1, 0), (2, 3, 0, 1), (3, 0, -1, 0)] {
        let facing = project(dx as f64, dy as f64, 0.0, rotation).1 > 1e-8;
        let open = at
            .get(&(x + dx, y + dy))
            .is_none_or(|&n| layout.cells[n].area != cell.area);
        if facing && open {
            poly(
                &mut path,
                &[r.corners[i], r.corners[j], ground[j], ground[i]],
            );
        }
    }
    (!path.is_empty()).then_some(path)
}

/// `worldContains`: even-odd point-in-polygon test.
pub fn contains(p: (f64, f64), poly: &[(f64, f64)]) -> bool {
    let mut inside = false;
    let mut j = poly.len().wrapping_sub(1);
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[j]);
        if (a.1 > p.1) != (b.1 > p.1) && p.0 < (b.0 - a.0) * (p.1 - a.1) / (b.1 - a.1) + a.0 {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// The front-most cell whose roof contains the unit point.
pub fn hit(layout: &Layout, frame: &Frame, rotation: f64, u: (f64, f64)) -> Option<usize> {
    let at = grid(layout);
    depth_order(layout, rotation).into_iter().rev().find(|&i| {
        let r = roof(layout, &at, frame, &layout.cells[i], rotation);
        (0..4).any(|k| contains(u, &[r.corners[k], r.corners[(k + 1) % 4], r.peak]))
    })
}

/// The roof outline of the given cells (selection highlight).
pub fn outline(
    layout: &Layout,
    frame: &Frame,
    rotation: f64,
    cells: impl Iterator<Item = usize>,
) -> String {
    let at = grid(layout);
    let mut path = String::new();
    for i in cells {
        let r = roof(layout, &at, frame, &layout.cells[i], rotation);
        poly(&mut path, &r.corners);
    }
    path
}

/// Area plates slightly larger than their grid, on the ground.
pub fn plate(frame: &Frame, p: &super::layout::Plate, rotation: f64) -> String {
    let (x0, y0, x1, y1) = (p.x - 0.25, p.y - 0.25, p.x + p.w + 0.25, p.y + p.h + 0.25);
    let mut path = String::new();
    poly(
        &mut path,
        &[(x0, y0), (x1, y0), (x1, y1), (x0, y1)].map(|(x, y)| frame.unit(x, y, GROUND, rotation)),
    );
    path
}

/// The decorative ground lattice: a line every two units.
pub fn lattice(frame: &Frame, rotation: f64) -> String {
    let mut path = String::new();
    let mut line = |a: (f64, f64), b: (f64, f64)| {
        let _ = write!(path, "M {:.3} {:.3} L {:.3} {:.3} ", a.0, a.1, b.0, b.1);
    };
    let mut gx = frame.minx.floor();
    while gx <= frame.maxx.ceil() {
        line(
            frame.unit(gx, frame.miny - 1.0, GROUND, rotation),
            frame.unit(gx, frame.maxy + 1.0, GROUND, rotation),
        );
        gx += 2.0;
    }
    let mut gy = frame.miny.floor();
    while gy <= frame.maxy.ceil() {
        line(
            frame.unit(frame.minx - 1.0, gy, GROUND, rotation),
            frame.unit(frame.maxx + 1.0, gy, GROUND, rotation),
        );
        gy += 2.0;
    }
    path
}

/// A dependency arrow between two screen points: the quadratic curve and
/// its head at 72 % of the way, as `worldDraw`.
pub fn arrow(p: (f64, f64), q: (f64, f64), curves: &mut String, heads: &mut String) {
    let (dx, dy) = (q.0 - p.0, q.1 - p.1);
    let length = dx.hypot(dy).max(1.0);
    let bend = 55f64.min(length * 0.2);
    let c = (
        (p.0 + q.0) / 2.0 - dy / length * bend,
        (p.1 + q.1) / 2.0 + dx / length * bend,
    );
    let _ = write!(
        curves,
        "M {:.1} {:.1} Q {:.1} {:.1} {:.1} {:.1} ",
        p.0, p.1, c.0, c.1, q.0, q.1
    );
    let (t, s) = (0.72, 0.28);
    let tip = (
        s * s * p.0 + 2.0 * s * t * c.0 + t * t * q.0,
        s * s * p.1 + 2.0 * s * t * c.1 + t * t * q.1,
    );
    let angle = (s * (c.1 - p.1) + t * (q.1 - c.1)).atan2(s * (c.0 - p.0) + t * (q.0 - c.0));
    let wing = |a: f64| (tip.0 - 8.0 * a.cos(), tip.1 - 8.0 * a.sin());
    poly(heads, &[tip, wing(angle - 0.45), wing(angle + 0.45)]);
}

#[cfg(test)]
mod tests {
    use super::super::layout::{Plate, complexity_height};
    use super::*;

    fn cell(x: i64, y: i64, area: usize) -> Cell {
        Cell {
            x,
            y,
            z: complexity_height(Some(3.0)),
            value: Some(3.0),
            area,
            entities: vec![0],
        }
    }

    fn plan(cells: Vec<Cell>) -> Layout {
        Layout {
            plates: vec![Plate {
                area: 0,
                x: 0.0,
                y: 0.0,
                w: 2.0,
                h: 2.0,
            }],
            cells,
        }
    }

    fn paint(_: &Cell) -> Paint {
        Paint {
            tone: Tone::Accent,
            side_alpha: 0.85,
            dimmed: false,
        }
    }

    fn close(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9
    }

    #[test]
    fn projection_matches_world_project() {
        assert!(close(project(1.0, 0.0, 0.0, 0.0), (0.86, 0.44)));
        assert!(close(project(0.0, 1.0, 0.5, 0.0), (-0.86, -0.06)));
        // A quarter turn maps the x axis onto the y axis.
        assert!(close(project(1.0, 0.0, 0.0, FRAC_PI_2), (-0.86, 0.44)));
    }

    #[test]
    fn zoom_keeps_the_point_under_the_cursor_and_clamps() {
        let cam = zoom(Camera::default(), 2.0, (100.0, 50.0));
        assert!(close((cam.x, cam.y), (-100.0, -50.0)));
        assert_eq!(cam.zoom, 2.0);
        assert_eq!(zoom(cam, 100.0, (0.0, 0.0)).zoom, 8.0);
        assert_eq!(zoom(cam, 0.01, (0.0, 0.0)).zoom, 0.4);
        let turned = rotate(Camera::default(), -FRAC_PI_2);
        assert!((turned.rotation - 3.0 * FRAC_PI_2).abs() < 1e-12);
    }

    #[test]
    fn frame_fits_the_plan_into_the_canvas() {
        let l = plan(vec![cell(0, 0, 0)]);
        let f = Frame::of(&l).unwrap();
        assert_eq!((f.cx, f.cy), (1.0, 1.0));
        assert!((f.radius - SQRT_2).abs() < 1e-12);
        let base = f.base(1000.0, 800.0);
        let a = 940.0 / (SQRT_2 * SQRT_2 * 1.72);
        let b = 650.0 / (SQRT_2 * SQRT_2 * 0.88 + l.cells[0].z);
        assert!((base - a.min(b)).abs() < 1e-9);
        let view = View {
            frame: f,
            w: 1000.0,
            h: 800.0,
            cam: Camera {
                x: 30.0,
                y: -20.0,
                zoom: 2.0,
                rotation: 0.0,
            },
        };
        let p = view.screen(1.5, 0.5, 0.2);
        assert!(close(view.to_screen(view.to_unit(p)), p));
        let vb = view.viewbox();
        assert!(close(view.to_unit((0.0, 0.0)), (vb[0], vb[1])));
        assert!((vb[2] * view.scale() - 1000.0).abs() < 1e-9);
    }

    /// At 0° the +x and +y sides face the viewer; a neighbour of the same
    /// area hides the shared side, one of another area does not.
    #[test]
    fn hidden_sides_are_culled() {
        let one = plan(vec![cell(0, 0, 0)]);
        let f = Frame::of(&one).unwrap();
        let faces = faces(&one, &f, 0.0, paint);
        assert_eq!(faces.len(), 5);
        assert_eq!(faces[0].path.matches('M').count(), 2);
        assert!(!faces[0].stroke && faces[1].stroke);

        let same = plan(vec![cell(0, 0, 0), cell(1, 0, 0)]);
        let sides: Vec<usize> = super::faces(&same, &f, 0.0, paint)
            .iter()
            .filter(|f| !f.stroke)
            .map(|f| f.path.matches('M').count())
            .collect();
        assert_eq!(sides, vec![1, 2]);
        let other = plan(vec![cell(0, 0, 0), cell(1, 0, 1)]);
        let sides = super::faces(&other, &f, 0.0, paint)
            .iter()
            .filter(|f| !f.stroke)
            .count();
        assert_eq!(sides, 2);
        // Turned by 180°, the -x and -y sides face the viewer instead.
        let turned = super::faces(&same, &f, std::f64::consts::PI, paint);
        let sides: Vec<usize> = turned
            .iter()
            .filter(|f| !f.stroke)
            .map(|f| f.path.matches('M').count())
            .collect();
        assert_eq!(sides, vec![1, 2]);
    }

    #[test]
    fn painter_order_and_hits_follow_depth() {
        let l = plan(vec![cell(1, 1, 0), cell(0, 0, 0), cell(1, 0, 0)]);
        assert_eq!(depth_order(&l, 0.0), vec![1, 2, 0]);
        assert_eq!(depth_order(&l, std::f64::consts::PI), vec![0, 2, 1]);
        let f = Frame::of(&l).unwrap();
        // The centroid of the front cell's first roof facet.
        let r = roof(&l, &grid(&l), &f, &l.cells[0], 0.0);
        let (a, b, peak) = (r.corners[0], r.corners[1], r.peak);
        let inside = ((a.0 + b.0 + peak.0) / 3.0, (a.1 + b.1 + peak.1) / 3.0);
        assert_eq!(hit(&l, &f, 0.0, inside), Some(0));
        assert_eq!(hit(&l, &f, 0.0, (40.0, 40.0)), None);
    }

    #[test]
    fn arrows_bend_left_of_their_direction() {
        let (mut c, mut h) = (String::new(), String::new());
        arrow((0.0, 0.0), (100.0, 0.0), &mut c, &mut h);
        assert_eq!(c.trim(), "M 0.0 0.0 Q 50.0 20.0 100.0 0.0");
        assert_eq!(h.matches('L').count(), 2);
    }
}
