//! Grid geometry of the network display: card size, gaps and the
//! placement of the cards in input order.

pub const CARD_W: f32 = 270.0;
pub const CARD_H: f32 = 124.0;
/// Height of the package tab above the card body (network_view.slint).
pub(super) const TAB_H: f32 = 24.0;
// Gaps wide enough for a few lanes plus room for the arrow heads.
pub(super) const GAP_X: f32 = 110.0;
pub(super) const GAP_Y: f32 = 100.0;
pub(super) const MARGIN: f32 = 48.0;

#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Arrow {
    pub path: String,
    pub width: f32,
    /// Where the reference count goes: middle of the longest run.
    pub label: (f32, f32),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Network {
    pub cards: Vec<Card>,
    pub cols: usize,
    /// Left edge of every column and top edge of every row.
    pub(super) xs: Vec<f32>,
    pub(super) ys: Vec<f32>,
    pub width: f32,
    pub height: f32,
}

/// Grid placement in input order, `ceil(sqrt(n))` columns.
pub fn place(n: usize) -> Network {
    grid(n, &[], &[])
}

/// Edges of `count` cells of `size`; gap `k` lies before cell `k` (the
/// last one after the last cell). Missing gaps get the default.
fn edges(count: usize, size: f32, gaps: &[f32], default: f32) -> (Vec<f32>, f32) {
    let gap = |k: usize| {
        let base = if k == 0 || k == count {
            MARGIN
        } else {
            default
        };
        gaps.get(k).map_or(base, |&g| g.max(base))
    };
    let mut at = 0.0;
    let mut out = Vec::with_capacity(count);
    for k in 0..count {
        at += gap(k);
        out.push(at);
        at += size;
    }
    (out, at + gap(count))
}

/// Grid with the given column and row gaps (margins first and last).
pub(super) fn grid(n: usize, col_gaps: &[f32], row_gaps: &[f32]) -> Network {
    let cols = ((n as f32).sqrt().ceil() as usize).max(1);
    let rows = n.div_ceil(cols);
    let (xs, width) = edges(cols, CARD_W, col_gaps, GAP_X);
    let (ys, height) = edges(rows, CARD_H, row_gaps, GAP_Y);
    let cards = (0..n)
        .map(|i| Card {
            x: xs[i % cols],
            y: ys[i / cols],
        })
        .collect();
    Network {
        cards,
        cols,
        xs,
        ys,
        width,
        height: if n == 0 { 0.0 } else { height },
    }
}

/// Stroke width for `count` references, as the web legend
/// (`min(8, 1.5 + log2(count))`).
pub fn stroke_width(count: u64) -> f32 {
    (1.5 + (count.max(1) as f32).log2()).min(8.0)
}
