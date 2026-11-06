//! SVG paths of the routed arrows: rounded corners, the arrow head and
//! the place of the reference count.

/// Points without repeats and straight-through bends.
pub(super) fn simplify(points: Vec<(f32, f32)>) -> Vec<(f32, f32)> {
    let mut out: Vec<(f32, f32)> = Vec::with_capacity(points.len());
    for p in points {
        if out
            .last()
            .is_some_and(|q| (q.0 - p.0).abs() < 0.5 && (q.1 - p.1).abs() < 0.5)
        {
            continue;
        }
        if let [.., a, b] = out[..] {
            let straight = ((a.0 - b.0).abs() < 0.5 && (b.0 - p.0).abs() < 0.5)
                || ((a.1 - b.1).abs() < 0.5 && (b.1 - p.1).abs() < 0.5);
            if straight {
                out.pop();
            }
        }
        out.push(p);
    }
    out
}

/// SVG path of the polyline with rounded corners, then the arrow head.
pub(super) fn path(points: &[(f32, f32)], width: f32) -> String {
    use std::fmt::Write;
    let dist = |a: (f32, f32), b: (f32, f32)| ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
    let toward = |a: (f32, f32), b: (f32, f32), r: f32| {
        let l = dist(a, b).max(1e-3);
        (a.0 + (b.0 - a.0) * r / l, a.1 + (b.1 - a.1) * r / l)
    };
    let mut s = format!("M {:.1} {:.1}", points[0].0, points[0].1);
    for w in points.windows(3) {
        let r = 8.0f32
            .min(dist(w[0], w[1]) / 2.0)
            .min(dist(w[1], w[2]) / 2.0);
        let (p, q) = (toward(w[1], w[0], r), toward(w[1], w[2], r));
        let _ = write!(
            s,
            " L {:.1} {:.1} Q {:.1} {:.1} {:.1} {:.1}",
            p.0, p.1, w[1].0, w[1].1, q.0, q.1
        );
    }
    let (t, prev) = (points[points.len() - 1], points[points.len() - 2]);
    let _ = write!(s, " L {:.1} {:.1}", t.0, t.1);
    let l = dist(prev, t).max(1.0);
    let (ux, uy) = ((t.0 - prev.0) / l, (t.1 - prev.1) / l);
    let (back, side) = (8.0 + width, 4.5 + width / 2.0);
    let wing = |k: f32| {
        (
            t.0 - ux * back - uy * side * k,
            t.1 - uy * back + ux * side * k,
        )
    };
    let ((w1x, w1y), (w2x, w2y)) = (wing(1.0), wing(-1.0));
    let _ = write!(
        s,
        " M {w1x:.1} {w1y:.1} L {:.1} {:.1} L {w2x:.1} {w2y:.1}",
        t.0, t.1
    );
    s
}

/// Midpoint of the longest segment: where the count label goes.
pub(super) fn label_at(points: &[(f32, f32)]) -> (f32, f32) {
    let len = |w: &[(f32, f32)]| (w[1].0 - w[0].0).abs() + (w[1].1 - w[0].1).abs();
    let w = points
        .windows(2)
        .max_by(|a, b| len(a).total_cmp(&len(b)))
        .expect("a route has two points");
    ((w[0].0 + w[1].0) / 2.0, (w[0].1 + w[1].1) / 2.0)
}
