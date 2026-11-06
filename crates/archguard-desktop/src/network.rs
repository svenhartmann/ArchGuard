//! The web viewer's "Network" display: package cards in a grid and
//! dependency arrows between them, stroke width by reference count.

pub const CARD_W: f32 = 270.0;
pub const CARD_H: f32 = 124.0;
const GAP_X: f32 = 90.0;
const GAP_Y: f32 = 80.0;
const MARGIN: f32 = 40.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Arrow {
    pub path: String,
    pub width: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Network {
    pub cards: Vec<Card>,
    pub width: f32,
    pub height: f32,
}

/// Grid placement in input order, `ceil(sqrt(n))` columns.
pub fn place(n: usize) -> Network {
    let cols = ((n as f32).sqrt().ceil() as usize).max(1);
    let rows = n.div_ceil(cols);
    let cards = (0..n)
        .map(|i| Card {
            x: MARGIN + (i % cols) as f32 * (CARD_W + GAP_X),
            y: MARGIN + (i / cols) as f32 * (CARD_H + GAP_Y),
        })
        .collect();
    let span = |k: usize, size: f32, gap: f32| {
        2.0 * MARGIN + k as f32 * size + k.saturating_sub(1) as f32 * gap
    };
    Network {
        cards,
        width: span(cols, CARD_W, GAP_X),
        height: if n == 0 {
            0.0
        } else {
            span(rows, CARD_H, GAP_Y)
        },
    }
}

/// Stroke width for `count` references, as the web legend
/// (`min(8, 1.5 + log2(count))`).
pub fn stroke_width(count: u64) -> f32 {
    (1.5 + (count.max(1) as f32).log2()).min(8.0)
}

/// Arrow from card `a` to card `b`, leaving and entering at the card
/// borders, bent to the left of its direction so that `a→b` and `b→a`
/// stay apart. Ends with an arrow head at `b`.
pub fn arrow(a: &Card, b: &Card, count: u64) -> Arrow {
    let center = |c: &Card| (c.x + CARD_W / 2.0, c.y + CARD_H / 2.0);
    let ((ax, ay), (bx, by)) = (center(a), center(b));
    let len = ((bx - ax).powi(2) + (by - ay).powi(2)).sqrt().max(1.0);
    let (dx, dy) = ((bx - ax) / len, (by - ay) / len);
    // Distance from a card centre to its border along (dx, dy).
    let exit = {
        let sx = if dx.abs() > 1e-6 {
            CARD_W / 2.0 / dx.abs()
        } else {
            f32::MAX
        };
        let sy = if dy.abs() > 1e-6 {
            CARD_H / 2.0 / dy.abs()
        } else {
            f32::MAX
        };
        sx.min(sy) + 4.0
    };
    let (sx, sy) = (ax + dx * exit, ay + dy * exit);
    let (tx, ty) = (bx - dx * exit, by - dy * exit);
    let (nx, ny) = (-dy, dx);
    // Bend relative to the visible segment, not the centre distance.
    let seg = ((tx - sx).powi(2) + (ty - sy).powi(2)).sqrt();
    let bend = 0.15 * seg;
    let at = |f: f32| {
        (
            sx + (tx - sx) * f + nx * bend,
            sy + (ty - sy) * f + ny * bend,
        )
    };
    let ((c1x, c1y), (c2x, c2y)) = (at(1.0 / 3.0), at(2.0 / 3.0));
    // Arrow head along the last segment of the curve.
    let (hx, hy) = (tx - c2x, ty - c2y);
    let hl = (hx * hx + hy * hy).sqrt().max(1.0);
    let (ux, uy) = (hx / hl, hy / hl);
    let w = stroke_width(count);
    let (back, side) = (8.0 + w, 4.5 + w / 2.0);
    let wing = |s: f32| {
        (
            tx - ux * back - uy * side * s,
            ty - uy * back + ux * side * s,
        )
    };
    let ((w1x, w1y), (w2x, w2y)) = (wing(1.0), wing(-1.0));
    Arrow {
        path: format!(
            "M {sx:.1} {sy:.1} C {c1x:.1} {c1y:.1} {c2x:.1} {c2y:.1} {tx:.1} {ty:.1} \
             M {w1x:.1} {w1y:.1} L {tx:.1} {ty:.1} L {w2x:.1} {w2y:.1}"
        ),
        width: w,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inside(c: &Card, x: f32, y: f32) -> bool {
        x > c.x + 1.0 && x < c.x + CARD_W - 1.0 && y > c.y + 1.0 && y < c.y + CARD_H - 1.0
    }

    fn numbers(path: &str) -> Vec<f32> {
        path.split(' ').filter_map(|t| t.parse().ok()).collect()
    }

    #[test]
    fn nine_cards_form_three_rows_of_three() {
        let n = place(9);
        let xs: Vec<f32> = n.cards.iter().map(|c| c.x).collect();
        assert_eq!(xs[0], xs[3]);
        assert!(xs[0] < xs[1] && xs[1] < xs[2]);
        assert!(n.cards[2].y < n.cards[3].y);
        let right = n.cards.iter().map(|c| c.x + CARD_W).fold(0.0, f32::max);
        assert!(right < n.width && n.cards[8].y + CARD_H < n.height);
    }

    #[test]
    fn stroke_width_follows_the_web_legend() {
        assert_eq!(stroke_width(1), 1.5);
        assert_eq!(stroke_width(4), 3.5);
        assert_eq!(stroke_width(100), 8.0);
        assert_eq!(stroke_width(0), 1.5);
    }

    #[test]
    fn arrows_start_and_end_on_card_borders() {
        let n = place(4);
        let (a, b) = (&n.cards[0], &n.cards[3]);
        let p = numbers(&arrow(a, b, 10).path);
        let (sx, sy) = (p[0], p[1]);
        let (tx, ty) = (p[6], p[7]);
        assert!(!inside(a, sx, sy) && !inside(b, tx, ty));
        let near = |c: &Card, x: f32, y: f32| {
            // Arrows keep a 4 px gap to the border (plus rounding).
            let t = 5.0;
            x >= c.x - t && x <= c.x + CARD_W + t && y >= c.y - t && y <= c.y + CARD_H + t
        };
        assert!(near(a, sx, sy) && near(b, tx, ty));
    }

    #[test]
    fn neighbour_arrows_stay_gentle() {
        // Adjacent cards: the bend must stay small against the visible
        // segment, or the arrow turns into a hook.
        let n = place(2);
        let p = numbers(&arrow(&n.cards[0], &n.cards[1], 1).path);
        let seg = ((p[6] - p[0]).powi(2) + (p[7] - p[1]).powi(2)).sqrt();
        let bend = (p[3] - p[1]).abs();
        assert!(seg >= 40.0, "visible segment {seg}");
        assert!(bend <= seg * 0.3, "bend {bend} on segment {seg}");
    }

    #[test]
    fn opposite_arrows_bend_apart() {
        let n = place(2);
        let ab = numbers(&arrow(&n.cards[0], &n.cards[1], 1).path);
        let ba = numbers(&arrow(&n.cards[1], &n.cards[0], 1).path);
        // Control points of a horizontal pair lie on different sides.
        assert!((ab[3] - ab[1]) * (ba[3] - ba[1]) < 0.0);
    }
}
