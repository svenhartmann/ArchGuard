//! Connection arrows over the world in canvas pixels, as `worldDraw`: a
//! quadratic curve bent to the left of its direction with its head at
//! 72 % of the way. Slint paths have no dash pattern, so dashed curves
//! are emitted as their segments.

use std::fmt::Write;

pub type Point = (f64, f64);

/// The control point of the curve from `p` to `q`.
pub fn control(p: Point, q: Point) -> Point {
    let (dx, dy) = (q.0 - p.0, q.1 - p.1);
    let length = dx.hypot(dy).max(1.0);
    let bend = 55f64.min(length * 0.2);
    (
        (p.0 + q.0) / 2.0 - dy / length * bend,
        (p.1 + q.1) / 2.0 + dx / length * bend,
    )
}

/// The point at `t` of the quadratic curve p, c, q.
pub fn at(p: Point, c: Point, q: Point, t: f64) -> Point {
    let s = 1.0 - t;
    (
        s * s * p.0 + 2.0 * s * t * c.0 + t * t * q.0,
        s * s * p.1 + 2.0 * s * t * c.1 + t * t * q.1,
    )
}

/// The arrow head at 72 % of the curve, eight pixels long.
fn head(p: Point, c: Point, q: Point, heads: &mut String) {
    let (t, s) = (0.72, 0.28);
    let tip = at(p, c, q, t);
    let angle = (s * (c.1 - p.1) + t * (q.1 - c.1)).atan2(s * (c.0 - p.0) + t * (q.0 - c.0));
    let wing = |a: f64| (tip.0 - 8.0 * a.cos(), tip.1 - 8.0 * a.sin());
    let (a, b) = (wing(angle - 0.45), wing(angle + 0.45));
    let _ = write!(
        heads,
        "M {:.3} {:.3} L {:.3} {:.3} L {:.3} {:.3} Z ",
        tip.0, tip.1, a.0, a.1, b.0, b.1
    );
}

/// A dependency arrow between two screen points: its curve and its head.
pub fn arrow(p: Point, q: Point, curves: &mut String, heads: &mut String) {
    let c = control(p, q);
    let _ = write!(
        curves,
        "M {:.1} {:.1} Q {:.1} {:.1} {:.1} {:.1} ",
        p.0, p.1, c.0, c.1, q.0, q.1
    );
    head(p, c, q, heads);
}

/// The arrow with its curve dashed `on`/`off` pixels along its length
/// (the web's `setLineDash([7, 5])`), sampled as 32 straight pieces.
pub fn dashed(p: Point, q: Point, on: f64, off: f64, curves: &mut String, heads: &mut String) {
    let c = control(p, q);
    let points: Vec<Point> = (0..=32).map(|i| at(p, c, q, f64::from(i) / 32.0)).collect();
    // A dash runs on across the pieces of the curve: one move per dash.
    let (mut phase, mut drawing, mut open) = (0.0, true, false);
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = (b.0 - a.0).hypot(b.1 - a.1);
        let along = |d: f64| (a.0 + (b.0 - a.0) * d / len, a.1 + (b.1 - a.1) * d / len);
        let mut pos = 0.0;
        while pos < len {
            let limit = if drawing { on } else { off };
            let step = (limit - phase).min(len - pos);
            if drawing {
                if !open {
                    let s = along(pos);
                    let _ = write!(curves, "M {:.1} {:.1} ", s.0, s.1);
                    open = true;
                }
                let e = along(pos + step);
                let _ = write!(curves, "L {:.1} {:.1} ", e.0, e.1);
            }
            pos += step;
            phase += step;
            if phase >= limit {
                (phase, drawing, open) = (0.0, !drawing, false);
            }
        }
    }
    head(p, c, q, heads);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrows_bend_left_of_their_direction() {
        let (mut c, mut h) = (String::new(), String::new());
        arrow((0.0, 0.0), (100.0, 0.0), &mut c, &mut h);
        assert_eq!(c.trim(), "M 0.0 0.0 Q 50.0 20.0 100.0 0.0");
        assert_eq!(h.matches('L').count(), 2);
        assert_eq!(
            at((0.0, 0.0), (50.0, 20.0), (100.0, 0.0), 0.5),
            (50.0, 10.0)
        );
    }

    /// Seven pixels drawn, five left out, along a straight line.
    #[test]
    fn dashes_alternate_seven_and_five() {
        let (mut c, mut h) = (String::new(), String::new());
        // Too short to bend: control at the midpoint, a straight line.
        dashed((0.0, 0.0), (0.0, 0.0), 7.0, 5.0, &mut c, &mut h);
        assert_eq!(c, "", "no length, no dash");
        let (mut c, mut h) = (String::new(), String::new());
        dashed((0.0, 0.0), (24.0, 0.0), 7.0, 5.0, &mut c, &mut h);
        // About 26 px of curve: dashes at 0–7, 12–19 and from 24 on.
        assert_eq!(c.matches('M').count(), 3, "{c}");
        assert!(c.starts_with("M 0.0 0.0 L "), "{c}");
        assert_eq!(h.matches('Z').count(), 1);
    }
}
