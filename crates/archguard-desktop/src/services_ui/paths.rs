//! SVG path strings for the service tree: the web's `roundedTreePath`,
//! `treeDependencyPath`, its arrow head polygon and the startup dash
//! (`stroke-dasharray: 6 3`), which Slint paths do not support.

use std::fmt::Write;

use super::layout::Point;

/// `roundedTreePath`: polyline with rounded non-collinear bends.
pub fn rounded(points: &[Point], radius: f32) -> String {
    let mut path: Vec<Point> = Vec::with_capacity(points.len());
    for &p in points {
        if path.last() != Some(&p) {
            path.push(p);
        }
    }
    let Some(first) = path.first() else {
        return String::new();
    };
    let mut out = format!("M {} {}", first.x, first.y);
    for w in path.windows(3) {
        let (before, corner, after) = (w[0], w[1], w[2]);
        let turn = (corner.x - before.x) * (after.y - corner.y)
            - (corner.y - before.y) * (after.x - corner.x);
        if turn == 0.0 || radius <= 0.0 {
            let _ = write!(out, " L {} {}", corner.x, corner.y);
            continue;
        }
        let inc = (corner.x - before.x).hypot(corner.y - before.y);
        let outg = (after.x - corner.x).hypot(after.y - corner.y);
        let r = radius.min(inc / 2.0).min(outg / 2.0);
        let entry = Point {
            x: corner.x - (corner.x - before.x) * r / inc,
            y: corner.y - (corner.y - before.y) * r / inc,
        };
        let exit = Point {
            x: corner.x + (after.x - corner.x) * r / outg,
            y: corner.y + (after.y - corner.y) * r / outg,
        };
        let _ = write!(
            out,
            " L {} {} Q {} {} {} {}",
            entry.x, entry.y, corner.x, corner.y, exit.x, exit.y
        );
    }
    if let (true, Some(last)) = (path.len() > 1, path.last()) {
        let _ = write!(out, " L {} {}", last.x, last.y);
    }
    out
}

/// `treeDependencyPath`: horizontal leads and one cubic bow.
pub fn curve(p: &[Point; 6]) -> String {
    format!(
        "M {} {} L {} {} C {} {} {} {} {} {} L {} {}",
        p[0].x,
        p[0].y,
        p[1].x,
        p[1].y,
        p[2].x,
        p[2].y,
        p[3].x,
        p[3].y,
        p[4].x,
        p[4].y,
        p[5].x,
        p[5].y
    )
}

/// The curve as a polyline (32 cubic samples, as the web's hit points).
fn flatten(p: &[Point; 6]) -> Vec<Point> {
    let mut out = vec![p[0], p[1]];
    for i in 1..=32 {
        let t = i as f32 / 32.0;
        let u = 1.0 - t;
        let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
        out.push(Point {
            x: a * p[1].x + b * p[2].x + c * p[3].x + d * p[4].x,
            y: a * p[1].y + b * p[2].y + c * p[3].y + d * p[4].y,
        });
    }
    out.push(p[5]);
    out
}

/// The curve dashed `on`/`off` along its length.
pub fn dashed(p: &[Point; 6], on: f32, off: f32) -> String {
    let line = flatten(p);
    let mut out = String::new();
    let (mut phase, mut drawing) = (0.0f32, true);
    for w in line.windows(2) {
        let (a, b) = (w[0], w[1]);
        let len = (b.x - a.x).hypot(b.y - a.y);
        let mut pos = 0.0;
        while pos < len {
            let limit = if drawing { on } else { off };
            let step = (limit - phase).min(len - pos);
            if drawing {
                let at = |s: f32| (a.x + (b.x - a.x) * s / len, a.y + (b.y - a.y) * s / len);
                let ((x0, y0), (x1, y1)) = (at(pos), at(pos + step));
                let _ = write!(out, "M {x0} {y0} L {x1} {y1} ");
            }
            pos += step;
            phase += step;
            if phase >= limit {
                phase = 0.0;
                drawing = !drawing;
            }
        }
    }
    out.trim_end().to_string()
}

/// The arrow head `0,0 -9,-4.5 -9,4.5` at `tip`, pointing from `prior`.
pub fn head(tip: Point, prior: Point) -> String {
    let angle = (tip.y - prior.y).atan2(tip.x - prior.x);
    let (sin, cos) = angle.sin_cos();
    let at = |x: f32, y: f32| (tip.x + x * cos - y * sin, tip.y + x * sin + y * cos);
    let ((ax, ay), (bx, by)) = (at(-9.0, -4.5), at(-9.0, 4.5));
    format!("M {} {} L {ax} {ay} L {bx} {by} Z", tip.x, tip.y)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(x: f32, y: f32) -> Point {
        Point { x, y }
    }

    /// The hierarchy connector of a child row: vertical rail, rounded
    /// corner, horizontal stub — as `roundedTreePath(points, 6)`.
    #[test]
    fn rounded_bends_like_the_web() {
        let d = rounded(&[pt(33.0, 46.0), pt(33.0, 68.0), pt(49.0, 68.0)], 6.0);
        assert_eq!(d, "M 33 46 L 33 62 Q 33 68 39 68 L 49 68");
    }

    #[test]
    fn head_points_along_the_lead() {
        // A left-side arc ends pointing right (+x) at its target.
        assert_eq!(
            head(pt(100.0, 50.0), pt(88.0, 50.0)),
            "M 100 50 L 91 45.5 L 91 54.5 Z"
        );
    }

    #[test]
    fn dashes_alternate_six_and_three() {
        let p = [
            pt(0.0, 0.0),
            pt(18.0, 0.0),
            pt(18.0, 0.0),
            pt(18.0, 0.0),
            pt(18.0, 0.0),
            pt(18.0, 0.0),
        ];
        let d = dashed(&p, 6.0, 3.0);
        assert!(d.starts_with("M 0 0 L 6 0 M 9 0 L 15 0"), "{d}");
    }
}
