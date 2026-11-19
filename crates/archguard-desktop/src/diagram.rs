//! SVG path commands for the layered diagrams of the Screens and Domain
//! views: curved or straight arrows with an open head, dashed lines and
//! dashed card borders (Slint's `Path` has no dash pattern).

use std::fmt::Write;

pub type Point = (f32, f32);

/// An arrow from `a` to `b` that leaves and enters horizontally, and the
/// point halfway along it (for a label). Ends one above the other bulge
/// out to the left (an edge within one column of cards).
pub fn curve(a: Point, b: Point) -> (String, Point) {
    let (c1, c2) = if (b.0 - a.0).abs() < 1.0 {
        let d = 40.0 + (b.1 - a.1).abs() * 0.15;
        ((a.0 - d, a.1), (b.0 - d, b.1))
    } else {
        let d = ((b.0 - a.0).abs() * 0.45).max(30.0).copysign(b.0 - a.0);
        ((a.0 + d, a.1), (b.0 - d, b.1))
    };
    let mut s = format!(
        "M {:.1} {:.1} C {:.1} {:.1} {:.1} {:.1} {:.1} {:.1}",
        a.0, a.1, c1.0, c1.1, c2.0, c2.1, b.0, b.1
    );
    head(&mut s, c2, b);
    let mid = (
        (a.0 + 3.0 * c1.0 + 3.0 * c2.0 + b.0) / 8.0,
        (a.1 + 3.0 * c1.1 + 3.0 * c2.1 + b.1) / 8.0,
    );
    (s, mid)
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
    let mut s = if dashed {
        dashes(a, b)
    } else {
        format!("M {:.1} {:.1} L {:.1} {:.1}", a.0, a.1, b.0, b.1)
    };
    head(&mut s, a, b);
    (s, ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0))
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
}
