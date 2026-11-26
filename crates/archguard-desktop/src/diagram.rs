//! SVG path commands for the layered diagrams of the Screens and Domain
//! views: curved or straight arrows with an open head, dashed lines and
//! dashed card borders (Slint's `Path` has no dash pattern).

use std::fmt::Write;

use crate::services_ui::{layout::Point as Pt, paths};

pub type Point = (f32, f32);

/// The end of an edge; the code is what the Slint canvas reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeadShape {
    Open = 0,
    Filled = 1,
    Hollow = 2,
    Diamond = 3,
}

/// A stroke without its head: path commands, the point halfway along it (for a label) and
/// the point the head comes from.
pub struct Stroke {
    pub path: String,
    pub mid: Point,
    pub tail: Point,
}

/// A curve from `a` to `b` that leaves and enters horizontally. Ends one above the other
/// bulge out to the left (an edge within one column of cards).
pub fn curve_stroke(a: Point, b: Point, dashed: bool) -> Stroke {
    let (c1, c2) = if (b.0 - a.0).abs() < 1.0 {
        let d = 40.0 + (b.1 - a.1).abs() * 0.15;
        ((a.0 - d, a.1), (b.0 - d, b.1))
    } else {
        let d = ((b.0 - a.0).abs() * 0.45).max(30.0).copysign(b.0 - a.0);
        ((a.0 + d, a.1), (b.0 - d, b.1))
    };
    let path = if dashed {
        let p = |q: Point| Pt { x: q.0, y: q.1 };
        paths::dashed(&[p(a), p(a), p(c1), p(c2), p(b), p(b)], 6.0, 4.0)
    } else {
        format!(
            "M {:.1} {:.1} C {:.1} {:.1} {:.1} {:.1} {:.1} {:.1}",
            a.0, a.1, c1.0, c1.1, c2.0, c2.1, b.0, b.1
        )
    };
    let mid = (
        (a.0 + 3.0 * c1.0 + 3.0 * c2.0 + b.0) / 8.0,
        (a.1 + 3.0 * c1.1 + 3.0 * c2.1 + b.1) / 8.0,
    );
    Stroke {
        path,
        mid,
        tail: c2,
    }
}

/// A straight stroke from `a` to `b`, dashed or solid.
pub fn line_stroke(a: Point, b: Point, dashed: bool) -> Stroke {
    let path = if dashed {
        dashes(a, b)
    } else {
        format!("M {:.1} {:.1} L {:.1} {:.1}", a.0, a.1, b.0, b.1)
    };
    Stroke {
        path,
        mid: ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0),
        tail: a,
    }
}

/// An arrow from `a` to `b` with an open head, and the point halfway along it.
pub fn curve(a: Point, b: Point) -> (String, Point) {
    let mut st = curve_stroke(a, b, false);
    head(&mut st.path, st.tail, b);
    (st.path, st.mid)
}

/// The ports of an arrow between two boxes `w` × `h` at `a` and `b`.
pub fn ends(a: Point, b: Point, w: f32, h: f32) -> (Point, Point) {
    let (ay, by) = (a.1 + h / 2.0, b.1 + h / 2.0);
    if b.0 > a.0 + 1.0 {
        ((a.0 + w, ay), (b.0, by))
    } else if b.0 < a.0 - 1.0 {
        ((a.0, ay), (b.0 + w, by))
    } else {
        ((a.0, ay), (b.0, by))
    }
}

/// A straight arrow, dashed or solid, and its middle.
pub fn line(a: Point, b: Point, dashed: bool) -> (String, Point) {
    let mut st = line_stroke(a, b, dashed);
    head(&mut st.path, a, b);
    (st.path, st.mid)
}

/// A small loop above `(x, y)` (a state that transitions into itself).
pub fn self_loop(x: f32, y: f32, w: f32) -> (String, Point) {
    let (l, r) = (x - w / 2.0, x + w / 2.0);
    let mut s = format!(
        "M {l:.1} {y:.1} C {l:.1} {:.1} {r:.1} {:.1} {r:.1} {y:.1}",
        y - 30.0,
        y - 30.0
    );
    head(&mut s, (r, y - 30.0), (r, y));
    (s, (x, y - 22.0))
}

/// The open head at `tip`, coming from `from`.
fn head(s: &mut String, from: Point, tip: Point) {
    let l = ((tip.0 - from.0).powi(2) + (tip.1 - from.1).powi(2))
        .sqrt()
        .max(1e-3);
    let (ux, uy) = ((tip.0 - from.0) / l, (tip.1 - from.1) / l);
    let wing = |k: f32| {
        (
            tip.0 - ux * 9.0 - uy * 5.0 * k,
            tip.1 - uy * 9.0 + ux * 5.0 * k,
        )
    };
    let (w1, w2) = (wing(1.0), wing(-1.0));
    let _ = write!(
        s,
        " M {:.1} {:.1} L {:.1} {:.1} L {:.1} {:.1}",
        w1.0, w1.1, tip.0, tip.1, w2.0, w2.1
    );
}

/// The head of `shape` with its tip at `tip`, coming from `from`. A filled tip and a hollow
/// triangle are closed (`Z`); a diamond is the tail end of an edge, so `tip` is its start.
pub fn head_path(shape: HeadShape, from: Point, tip: Point) -> String {
    let mut s = String::new();
    match shape {
        HeadShape::Open => head(&mut s, from, tip),
        HeadShape::Filled => closed(&mut s, from, tip, &[(9.0, 5.0)]),
        HeadShape::Hollow => closed(&mut s, from, tip, &[(12.0, 7.0)]),
        HeadShape::Diamond => closed(&mut s, from, tip, &[(7.0, 5.0), (14.0, 0.0)]),
    }
    s
}

/// A closed polygon: the tip, then each `(back, side)` offset on the left, and the mirrored
/// offsets on the right in reverse (a single offset gives a triangle).
fn closed(s: &mut String, from: Point, tip: Point, offs: &[(f32, f32)]) {
    let l = ((tip.0 - from.0).powi(2) + (tip.1 - from.1).powi(2))
        .sqrt()
        .max(1e-3);
    let (ux, uy) = ((tip.0 - from.0) / l, (tip.1 - from.1) / l);
    let at =
        |(back, side): (f32, f32)| (tip.0 - ux * back - uy * side, tip.1 - uy * back + ux * side);
    let left = offs.iter().map(|&o| at(o));
    let right = offs
        .iter()
        .rev()
        .skip_while(|o| o.1 == 0.0)
        .map(|&(b, sd)| at((b, -sd)));
    let _ = write!(s, "M {:.1} {:.1}", tip.0, tip.1);
    for p in left.chain(right) {
        let _ = write!(s, " L {:.1} {:.1}", p.0, p.1);
    }
    s.push_str(" Z");
}

/// Dashes of 6 with gaps of 4 from `a` to `b`.
fn dashes(a: Point, b: Point) -> String {
    let l = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
    let (ux, uy) = if l > 0.0 {
        ((b.0 - a.0) / l, (b.1 - a.1) / l)
    } else {
        (0.0, 0.0)
    };
    let mut s = String::new();
    let mut t = 0.0;
    while t < l {
        let e = (t + 6.0).min(l);
        let _ = write!(
            s,
            "M {:.1} {:.1} L {:.1} {:.1} ",
            a.0 + ux * t,
            a.1 + uy * t,
            a.0 + ux * e,
            a.1 + uy * e
        );
        t += 10.0;
    }
    s.trim_end().to_string()
}

/// A dashed rectangle `w` × `h` at the origin, for a card's border.
pub fn dash_rect(w: f32, h: f32) -> String {
    let corners = [
        (0.5, 0.5),
        (w - 0.5, 0.5),
        (w - 0.5, h - 0.5),
        (0.5, h - 0.5),
        (0.5, 0.5),
    ];
    let sides: Vec<String> = corners.windows(2).map(|c| dashes(c[0], c[1])).collect();
    sides.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn moves(s: &str) -> usize {
        s.matches('M').count()
    }

    #[test]
    fn a_dashed_line_is_broken_into_dashes() {
        let (s, mid) = line((0.0, 0.0), (100.0, 0.0), true);
        assert_eq!(moves(&s), 10 + 1, "ten dashes and the head");
        assert_eq!(mid, (50.0, 0.0));
        let (solid, _) = line((0.0, 0.0), (100.0, 0.0), false);
        assert_eq!(moves(&solid), 2);
    }

    #[test]
    fn a_curve_ends_at_its_target_with_a_head() {
        let (s, mid) = curve((0.0, 0.0), (200.0, 100.0));
        assert!(s.contains("C 90.0 0.0 110.0 100.0 200.0 100.0"), "{s}");
        assert!(s.contains("L 200.0 100.0 L"), "head at the tip: {s}");
        assert_eq!(mid, (100.0, 50.0));
        let (back, _) = curve((300.0, 0.0), (100.0, 0.0));
        assert!(back.contains("C 210.0 0.0 190.0 0.0 100.0 0.0"), "{back}");
        let (_, left) = curve((0.0, 100.0), (0.0, 0.0));
        assert!(left.0 < 0.0, "within a column it bulges left");
    }

    #[test]
    fn a_dashed_border_has_four_sides() {
        let s = dash_rect(20.0, 10.0);
        assert!(s.starts_with("M 0.5 0.5 L 6.5 0.5"), "{s}");
        assert!(moves(&s) >= 6);
    }

    #[test]
    fn each_head_shape_is_closed_with_its_tip_in_place() {
        let (from, tip) = ((0.0, 0.0), (100.0, 0.0));
        for shape in [HeadShape::Filled, HeadShape::Hollow] {
            let s = head_path(shape, from, tip);
            assert!(s.starts_with("M 100.0 0.0 L"), "tip first: {s}");
            assert!(s.ends_with('Z'), "{s}");
            assert_eq!(s.matches('L').count(), 2, "triangle: {s}");
        }
        let d = head_path(HeadShape::Diamond, from, tip);
        assert!(d.starts_with("M 100.0 0.0 L") && d.ends_with('Z'), "{d}");
        assert_eq!(d.matches('L').count(), 3, "four corners: {d}");
        assert!(d.contains("L 86.0 0.0"), "far corner behind the start: {d}");
        let open = head_path(HeadShape::Open, from, tip);
        assert!(
            open.contains("L 100.0 0.0 L") && !open.contains('Z'),
            "{open}"
        );
    }

    #[test]
    fn a_dashed_curve_is_broken_and_ends_at_the_target() {
        let st = curve_stroke((0.0, 0.0), (200.0, 100.0), true);
        assert!(st.path.matches('M').count() > 5, "{}", st.path);
        assert!(st.path.starts_with("M 0 0"), "{}", st.path);
        assert_eq!(st.mid, (100.0, 50.0));
    }
}
