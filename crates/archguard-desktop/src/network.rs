//! The web viewer's "Network" display: package cards in a grid and
//! dependency arrows between them, stroke width by reference count. The
//! arrows run through the gaps of the grid ([`route`]), not across cards.

mod draw;
mod grid;
mod route;

pub use grid::{Arrow, Network, place};
pub use route::route;

/// The grid with every gap widened to the lanes its arrows need, and the
/// arrows for `links` (from card, to card, reference count).
pub fn layout(n: usize, links: &[(usize, usize, u64)]) -> (Network, Vec<Arrow>) {
    let (cols, rows) = route::gaps(&place(n), links);
    let net = grid::grid(n, &cols, &rows);
    let arrows = route(&net, links);
    (net, arrows)
}

#[cfg(test)]
mod tests {
    use super::grid::{CARD_H, CARD_W, Card, TAB_H, stroke_width};
    use super::*;

    fn inside(c: &Card, x: f32, y: f32) -> bool {
        x > c.x + 1.0 && x < c.x + CARD_W - 1.0 && y > c.y + 1.0 && y < c.y + CARD_H - 1.0
    }

    /// Points along an SVG path of M/L/Q/C commands, every few pixels.
    fn samples(path: &str) -> Vec<Vec<(f32, f32)>> {
        let mut out: Vec<Vec<(f32, f32)>> = Vec::new();
        let toks: Vec<&str> = path.split_whitespace().collect();
        let (mut i, mut at) = (0, (0.0, 0.0));
        let num = |k: usize| toks[k].parse::<f32>().unwrap();
        while i < toks.len() {
            let n = match toks[i] {
                "M" => 1,
                "L" => 1,
                "Q" => 2,
                "C" => 3,
                t => panic!("command {t}"),
            };
            let pts: Vec<(f32, f32)> = (0..n)
                .map(|k| (num(i + 1 + 2 * k), num(i + 2 + 2 * k)))
                .collect();
            if toks[i] == "M" {
                out.push(vec![pts[0]]);
            } else {
                let mut ctl = vec![at];
                ctl.extend(&pts);
                // About one sample per 4 px of the control polygon.
                let len: f32 = ctl
                    .windows(2)
                    .map(|w| ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt())
                    .sum();
                let steps = ((len / 4.0).ceil() as usize).max(2);
                for s in 1..=steps {
                    let mut q = ctl.clone();
                    let t = s as f32 / steps as f32;
                    while q.len() > 1 {
                        q = q
                            .windows(2)
                            .map(|w| {
                                (
                                    w[0].0 + (w[1].0 - w[0].0) * t,
                                    w[0].1 + (w[1].1 - w[0].1) * t,
                                )
                            })
                            .collect();
                    }
                    out.last_mut().unwrap().push(q[0]);
                }
            }
            at = *pts.last().unwrap();
            i += 1 + 2 * n;
        }
        out
    }

    fn near_line(p: (f32, f32), line: &[(f32, f32)], d: f32) -> bool {
        line.windows(2).any(|w| {
            let ((ax, ay), (bx, by)) = (w[0], w[1]);
            let (vx, vy) = (bx - ax, by - ay);
            let l2 = (vx * vx + vy * vy).max(1e-6);
            let t = (((p.0 - ax) * vx + (p.1 - ay) * vy) / l2).clamp(0.0, 1.0);
            ((ax + vx * t - p.0).powi(2) + (ay + vy * t - p.1).powi(2)).sqrt() < d
        })
    }

    fn all_pairs(n: usize) -> Vec<(usize, usize, u64)> {
        (0..n)
            .flat_map(|a| (0..n).filter(move |&b| b != a).map(move |b| (a, b, 3)))
            .collect()
    }

    #[test]
    fn routes_avoid_foreign_cards() {
        let links = all_pairs(9);
        let (net, arrows) = layout(9, &links);
        for (arrow, &(a, b, _)) in arrows.iter().zip(&links) {
            for part in samples(&arrow.path) {
                for &(x, y) in &part {
                    for (k, c) in net.cards.iter().enumerate() {
                        assert!(
                            k == a || k == b || !inside(c, x, y),
                            "arrow {a}->{b} crosses card {k} at ({x}, {y})"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn parallel_routes_keep_apart() {
        // No two arrows share a stretch of line: crossings touch in a point,
        // lanes keep parallel runs at least a few pixels apart.
        let links = all_pairs(9);
        let lines: Vec<Vec<(f32, f32)>> = layout(9, &links)
            .1
            .iter()
            .map(|a| samples(&a.path).swap_remove(0))
            .collect();
        for (i, p) in lines.iter().enumerate() {
            for (j, q) in lines.iter().enumerate().skip(i + 1) {
                let close = p.iter().filter(|&&a| near_line(a, q, 1.5)).count();
                assert!(
                    close <= 6,
                    "arrows {:?} and {:?} overlap on {close} samples",
                    links[i],
                    links[j]
                );
            }
        }
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
        let line = samples(&route(&n, &[(0, 3, 10)])[0].path).swap_remove(0);
        let ((sx, sy), (tx, ty)) = (line[0], line[line.len() - 1]);
        // The head enters from above, beside the package tab: it must stay
        // out of the card body.
        let body = Card {
            x: b.x,
            y: b.y + TAB_H,
        };
        assert!(!inside(a, sx, sy) && !inside(&body, tx, ty));
        let near = |c: &Card, x: f32, y: f32| {
            // Heads keep a 2 px gap to the border (plus rounding).
            let t = 3.0;
            x >= c.x - t && x <= c.x + CARD_W + t && y >= c.y - t && y <= c.y + CARD_H + t
        };
        assert!(near(a, sx, sy) && near(b, tx, ty));
    }

    #[test]
    fn neighbour_arrows_run_straight() {
        let n = place(2);
        let path = &route(&n, &[(0, 1, 1)])[0].path;
        let line = &samples(path)[0];
        assert!(line.iter().all(|p| (p.1 - line[0].1).abs() < 0.1), "{path}");
        assert!(line[line.len() - 1].0 - line[0].0 >= 100.0, "{path}");
    }

    #[test]
    fn opposite_arrows_stay_apart() {
        let n = place(2);
        let both = route(&n, &[(0, 1, 1), (1, 0, 1)]);
        let (ab, ba) = (&samples(&both[0].path)[0], &samples(&both[1].path)[0]);
        assert!((ab[0].1 - ba[0].1).abs() >= 10.0);
    }

    #[test]
    fn labels_sit_on_the_route() {
        let n = place(9);
        let a = &route(&n, &[(0, 8, 5)])[0];
        assert!(near_line(a.label, &samples(&a.path)[0], 0.5));
    }
}
