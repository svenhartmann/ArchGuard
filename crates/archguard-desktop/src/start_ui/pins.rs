//! Package name pins over the world, placed so that they cover neither
//! each other nor the 3D blocks.
//!
//! Greedy in priority order: every pin tries a few positions around its
//! area's blocks (above, below, right, left, diagonals, stacked above), and
//! the one it had in the last frame. A position must stay clear of placed
//! pins and the floating controls; among those, the one covering the
//! fewest blocks (then the nearest) wins. A pin that only finds positions
//! over blocks collapses to a dot at its area and shows on hover.
//! Block cover comes from a summed-area table, O(1) per position.

/// A screen rectangle by centre and size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn overlaps(&self, o: &Rect) -> bool {
        (o.x - self.x).abs() < (o.w + self.w) / 2.0 + 6.0
            && (o.y - self.y).abs() < (o.h + self.h) / 2.0 + 6.0
    }

    /// The rectangle around `points` (none: an empty one at the origin).
    pub fn around(points: impl IntoIterator<Item = (f64, f64)>) -> Rect {
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for (x, y) in points {
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
        if x0 > x1 {
            return Rect {
                x: 0.0,
                y: 0.0,
                w: 0.0,
                h: 0.0,
            };
        }
        Rect {
            x: (x0 + x1) / 2.0,
            y: (y0 + y1) / 2.0,
            w: x1 - x0,
            h: y1 - y0,
        }
    }

    /// The point of the rectangle nearest to `p`.
    pub fn nearest(&self, p: (f64, f64)) -> (f64, f64) {
        let (hw, hh) = (self.w / 2.0, self.h / 2.0);
        (
            p.0.clamp(self.x - hw, self.x + hw),
            p.1.clamp(self.y - hh, self.y + hh),
        )
    }
}

/// Height of a pin (two text lines with padding) in pixels.
pub const PIN_H: f64 = 50.0;
/// Distance between a pin and the blocks it labels.
const GAP: f64 = 14.0;
/// A pin may cover at most this share of its area with blocks.
const COVER_MAX: f64 = 0.15;

/// Estimated pin width for its two lines; at most 180 px as in the web.
pub fn width(name: &str, sub: &str, module: bool) -> f64 {
    let text = (name.chars().count() as f64 * 7.8).max(sub.chars().count() as f64 * 6.2);
    (text + 26.0 + if module { 18.0 } else { 0.0 }).min(180.0)
}

/// Which screen pixels show blocks, on a coarse grid, as a summed-area
/// table: the covered share of any rectangle in O(1).
pub struct Cover {
    cols: usize,
    rows: usize,
    sum: Vec<u32>,
}

const COVER_CELL: f64 = 6.0;

impl Cover {
    /// From block rectangles on a `w` × `h` canvas; O(blocks × rows + grid).
    pub fn new(blocks: &[Rect], (w, h): (f64, f64)) -> Cover {
        let cols = (w / COVER_CELL).ceil().max(1.0) as usize;
        let rows = (h / COVER_CELL).ceil().max(1.0) as usize;
        // Per row a difference array of block spans, then prefix sums.
        let mut diff = vec![0i32; (cols + 1) * rows];
        let cell = |v: f64, n: usize| ((v / COVER_CELL).max(0.0) as usize).min(n);
        for b in blocks {
            let (x0, x1) = (cell(b.x - b.w / 2.0, cols), cell(b.x + b.w / 2.0, cols));
            let (y0, y1) = (cell(b.y - b.h / 2.0, rows), cell(b.y + b.h / 2.0, rows));
            for r in y0..y1 {
                diff[r * (cols + 1) + x0] += 1;
                diff[r * (cols + 1) + x1] -= 1;
            }
        }
        let mut sum = vec![0u32; (cols + 1) * (rows + 1)];
        for r in 0..rows {
            let (mut run, mut line) = (0i32, 0u32);
            for c in 0..cols {
                run += diff[r * (cols + 1) + c];
                line += u32::from(run > 0);
                sum[(r + 1) * (cols + 1) + c + 1] = sum[r * (cols + 1) + c + 1] + line;
            }
        }
        Cover { cols, rows, sum }
    }

    /// Share of `r` that shows blocks, 0 to 1.
    pub fn share(&self, r: &Rect) -> f64 {
        let cell = |v: f64, n: usize| ((v / COVER_CELL).round().max(0.0) as usize).min(n);
        let (x0, x1) = (
            cell(r.x - r.w / 2.0, self.cols),
            cell(r.x + r.w / 2.0, self.cols),
        );
        let (y0, y1) = (
            cell(r.y - r.h / 2.0, self.rows),
            cell(r.y + r.h / 2.0, self.rows),
        );
        if x1 <= x0 || y1 <= y0 {
            return 0.0;
        }
        let at = |r: usize, c: usize| i64::from(self.sum[r * (self.cols + 1) + c]);
        let n = at(y1, x1) - at(y0, x1) - at(y1, x0) + at(y0, x0);
        n as f64 / ((x1 - x0) * (y1 - y0)) as f64
    }
}

/// One pin to place: its anchor (the area's centre on screen), width, the
/// rectangle around its area's blocks and its offset from the anchor in
/// the last frame.
#[derive(Debug, Clone, Copy)]
pub struct Want {
    pub anchor: (f64, f64),
    pub width: f64,
    pub blocks: Rect,
    pub last: Option<(f64, f64)>,
}

/// Where a pin went.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Spot {
    Shown(Rect),
    /// A dot at the anchor; the pin shows on hover.
    Collapsed,
    /// The anchor is off the canvas.
    Off,
}

/// Centres to try for a pin, around its blocks, nearest kinds first.
fn candidates(want: &Want) -> Vec<(f64, f64)> {
    let (ax, ay) = want.anchor;
    let b = want.blocks;
    let (hw, hh) = (want.width / 2.0, PIN_H / 2.0);
    let (left, right) = (b.x - b.w / 2.0 - GAP - hw, b.x + b.w / 2.0 + GAP + hw);
    let (top, bottom) = (b.y - b.h / 2.0 - GAP - hh, b.y + b.h / 2.0 + GAP + hh);
    let mut out = vec![
        (ax, top),
        (right, ay),
        (left, ay),
        (ax, bottom),
        (right - hw, top),
        (left + hw, top),
        (right, top + hh),
        (left, top + hh),
        (right - hw, bottom),
        (left + hw, bottom),
    ];
    out.extend((1..4).map(|k| (ax, top - k as f64 * (PIN_H + 8.0))));
    out
}

/// `Math.max(lo, Math.min(hi, v))`: `lo` wins when the canvas is too small.
fn fit(v: f64, lo: f64, hi: f64) -> f64 {
    lo.max(hi.min(v))
}

/// Place the pins of `wants` in priority order. `occupied` starts with the
/// floating controls.
pub fn place(
    wants: &[Want],
    canvas: (f64, f64),
    mut occupied: Vec<Rect>,
    cover: &Cover,
) -> Vec<Spot> {
    let (w, h) = canvas;
    let diag = w.hypot(h).max(1.0);
    wants
        .iter()
        .map(|want| {
            let (px, py) = want.anchor;
            if !(0.0..=w).contains(&px) || !(0.0..=h).contains(&py) {
                return Spot::Off;
            }
            let (hw, hh) = (want.width / 2.0, PIN_H / 2.0);
            let rect = |(x, y): (f64, f64)| Rect {
                x: fit(x, hw + 8.0, w - hw - 8.0),
                y: fit(y, hh + 8.0, h - hh - 8.0),
                w: want.width,
                h: PIN_H,
            };
            let free = |r: &Rect| !occupied.iter().any(|o| o.overlaps(r));
            let clear = |r: &Rect| free(r) && cover.share(r) <= COVER_MAX;
            let cost = |r: &Rect| cover.share(r) * 4.0 + (r.x - px).hypot(r.y - py) / diag;
            // Last frame's place wins while it stays clear: no jumps while
            // the world turns or zooms.
            let kept = want
                .last
                .map(|(dx, dy)| rect((px + dx, py + dy)))
                .filter(clear);
            let best = kept.or_else(|| {
                candidates(want)
                    .into_iter()
                    .map(rect)
                    .filter(clear)
                    .min_by(|a, b| cost(a).total_cmp(&cost(b)))
            });
            match best {
                Some(r) => {
                    occupied.push(r);
                    Spot::Shown(r)
                }
                None => Spot::Collapsed,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sq(x: f64, y: f64, s: f64) -> Rect {
        Rect { x, y, w: s, h: s }
    }

    fn want(anchor: (f64, f64), blocks: Rect) -> Want {
        Want {
            anchor,
            width: 120.0,
            blocks,
            last: None,
        }
    }

    #[test]
    fn cover_measures_the_share_of_blocks() {
        let c = Cover::new(&[sq(60.0, 60.0, 120.0)], (600.0, 600.0));
        assert_eq!(c.share(&sq(60.0, 60.0, 60.0)), 1.0);
        assert_eq!(c.share(&sq(400.0, 400.0, 60.0)), 0.0);
        let half = c.share(&Rect {
            x: 120.0,
            y: 60.0,
            w: 60.0,
            h: 60.0,
        });
        assert!((half - 0.5).abs() < 0.1, "{half}");
    }

    #[test]
    fn pins_leave_their_blocks_free() {
        // A block in the middle: the pin goes beside it, not on it.
        let block = sq(500.0, 400.0, 160.0);
        let cover = Cover::new(&[block], (1000.0, 800.0));
        let spots = place(
            &[want((500.0, 400.0), block)],
            (1000.0, 800.0),
            Vec::new(),
            &cover,
        );
        let Spot::Shown(r) = spots[0] else {
            panic!("{spots:?}")
        };
        assert_eq!(cover.share(&r), 0.0, "{r:?}");
    }

    #[test]
    fn dense_pins_neither_overlap_nor_cover_blocks() {
        // A 6 × 5 field of blocks, one pin per block, as a busy world.
        let mut blocks = Vec::new();
        for i in 0..6 {
            for j in 0..5 {
                blocks.push(sq(150.0 + i as f64 * 140.0, 160.0 + j as f64 * 120.0, 90.0));
            }
        }
        let canvas = (1000.0, 800.0);
        let cover = Cover::new(&blocks, canvas);
        let wants: Vec<Want> = blocks.iter().map(|b| want((b.x, b.y), *b)).collect();
        let spots = place(&wants, canvas, Vec::new(), &cover);
        let shown: Vec<Rect> = spots
            .iter()
            .filter_map(|s| {
                if let Spot::Shown(r) = s {
                    Some(*r)
                } else {
                    None
                }
            })
            .collect();
        assert!(shown.len() >= 8, "only {} of 30 shown", shown.len());
        for (i, a) in shown.iter().enumerate() {
            assert!(cover.share(a) <= 0.15, "{a:?} covers blocks");
            for b in &shown[i + 1..] {
                assert!(!a.overlaps(b), "{a:?} overlaps {b:?}");
            }
        }
        // The rest collapse to dots rather than vanish.
        assert!(spots.iter().all(|s| *s != Spot::Off));
    }

    #[test]
    fn controls_block_slots_and_pins_stay_inside() {
        let blocker = Rect {
            x: 500.0,
            y: 335.0,
            w: 400.0,
            h: 60.0,
        };
        let cover = Cover::new(&[], (1000.0, 800.0));
        let free = want((500.0, 400.0), sq(500.0, 400.0, 0.0));
        let placed = place(&[free], (1000.0, 800.0), vec![blocker], &cover);
        let Spot::Shown(r) = placed[0] else { panic!() };
        assert!(!blocker.overlaps(&r));
        let edge = place(
            &[want((2.0, 2.0), sq(2.0, 2.0, 0.0))],
            (1000.0, 800.0),
            Vec::new(),
            &cover,
        );
        let Spot::Shown(r) = edge[0] else { panic!() };
        assert!(r.x - r.w / 2.0 >= 8.0 && r.y - r.h / 2.0 >= 8.0);
        let off = place(
            &[want((-5.0, 10.0), sq(-5.0, 10.0, 0.0))],
            (1000.0, 800.0),
            Vec::new(),
            &cover,
        );
        assert_eq!(off[0], Spot::Off);
        assert_eq!(width("x", "", true), 7.8 + 26.0 + 18.0);
        assert_eq!(width(&"n".repeat(40), "", false), 180.0);
    }

    #[test]
    fn a_pin_keeps_its_place_while_it_is_free() {
        let cover = Cover::new(&[], (1000.0, 800.0));
        let mut w = want((500.0, 400.0), sq(500.0, 400.0, 40.0));
        w.last = Some((150.0, 0.0));
        let Spot::Shown(r) = place(&[w], (1000.0, 800.0), Vec::new(), &cover)[0] else {
            panic!()
        };
        assert_eq!((r.x, r.y), (650.0, 400.0));
    }
}
