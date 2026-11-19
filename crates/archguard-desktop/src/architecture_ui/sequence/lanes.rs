//! Where a sequence figure puts its participants (columns) and messages
//! (rows): evenly for generated sequences; for own Mermaid wide enough for
//! the message texts, with room for block frames, their dividers and notes.

use archguard_engine::core::{Diagram, DiagramRelation};

use crate::architecture_ui::mermaid::Item;
use crate::ui::{ArcFrame, ArcLine};

pub const COLUMN: f32 = 170.0;
pub const CARD_W: f32 = 150.0;
pub const MARGIN: f32 = 20.0;
pub const TOP: f32 = 14.0;
pub const ROW: f32 = 34.0;
/// Message labels in px before zoom (other diagrams use 9).
pub const LABEL: f32 = 12.0;
/// shortcut: a guessed mean glyph width of `LABEL` text; measure the text
/// if labels end up too tight or too loose.
const GLYPH: f32 = 6.6;
/// How far a nested frame sits inside its parent.
const INSET: f32 = 7.0;

/// Participant centres, row lines (the last one the bottom lifeline end),
/// card height and figure width.
pub struct Lanes {
    pub xs: Vec<f32>,
    pub ys: Vec<f32>,
    pub card: f32,
    pub width: f32,
}

impl Lanes {
    pub fn centre(&self, i: usize) -> f32 {
        self.xs[i]
    }
    pub fn row(&self, k: usize) -> f32 {
        self.ys[k]
    }
    pub fn bottom(&self) -> f32 {
        *self.ys.last().unwrap_or(&0.0)
    }
}

fn first_row(card: f32) -> f32 {
    TOP + card + 30.0
}

/// `n` participants a column apart, `rows` rows a row apart.
pub fn even(n: usize, rows: usize, card: f32) -> Lanes {
    let xs: Vec<f32> = (0..n)
        .map(|i| MARGIN + i as f32 * COLUMN + CARD_W / 2.0)
        .collect();
    Lanes {
        width: xs.last().map_or(CARD_W, |x| x + CARD_W / 2.0) + MARGIN,
        ys: (0..=rows)
            .map(|k| first_row(card) + k as f32 * ROW)
            .collect(),
        xs,
        card,
    }
}

/// Own Mermaid: columns as wide as their messages need, rows with the
/// frames of blocks and notes between them, and the blocks' dividers.
pub fn own(d: &Diagram, items: &[Item], card: f32) -> (Lanes, Vec<ArcFrame>, Vec<ArcLine>) {
    let (xs, width) = columns(d);
    let mut rows = Rows {
        top: first_row(card) - 20.0,
        ys: Vec::new(),
        frames: Vec::new(),
        lines: Vec::new(),
        open: Vec::new(),
    };
    for item in items {
        rows.place(item, &xs, width);
    }
    rows.ys.push(rows.top + 20.0);
    let lanes = Lanes {
        xs,
        ys: rows.ys,
        card,
        width,
    };
    (lanes, rows.frames, rows.lines)
}

/// The width a message's label needs, with its arrow's ends.
fn wide(r: &DiagramRelation) -> f32 {
    r.label.as_deref().map_or(0, |l| l.chars().count()) as f32 * GLYPH + 24.0
}

/// Each centre at least a column right of the last, and far enough from
/// every lane its messages start at; a message to oneself needs the room
/// right of its lane.
fn columns(d: &Diagram) -> (Vec<f32>, f32) {
    let at = |id: &str| d.elements.iter().position(|e| e.id == id);
    let spans: Vec<(usize, usize, f32)> = d
        .relations
        .iter()
        .filter_map(|r| {
            let (a, b) = (at(&r.from)?, at(&r.to)?);
            // A loop to oneself reaches into the next lane's gap.
            let around = if a == b { 30.0 } else { 0.0 };
            Some((a.min(b), a.max(b).max(a.min(b) + 1), wide(r) + around))
        })
        .collect();
    // How far right of the lanes before it lane `i` must lie.
    let need = |i: usize, xs: &[f32]| {
        spans
            .iter()
            .filter(|&&(_, b, _)| b == i)
            .map(|&(a, _, w)| xs[a] + w)
            .fold(0.0, f32::max)
    };
    let mut xs: Vec<f32> = Vec::new();
    for i in 0..d.elements.len() {
        let next = xs.last().map_or(MARGIN + CARD_W / 2.0, |x| x + COLUMN);
        xs.push(next.max(need(i, &xs)));
    }
    // Loops to oneself on the last lane reach past it.
    let last = xs.last().map_or(0.0, |x| x + CARD_W / 2.0);
    let width = last.max(need(xs.len(), &xs)) + MARGIN;
    (xs, width)
}

/// The rows of own Mermaid, placed from `top`, the top of the free space.
struct Rows {
    top: f32,
    ys: Vec<f32>,
    frames: Vec<ArcFrame>,
    lines: Vec<ArcLine>,
    /// The frames of the blocks still open, innermost last.
    open: Vec<usize>,
}

impl Rows {
    fn place(&mut self, item: &Item, xs: &[f32], width: f32) {
        let top = self.top;
        match item {
            // The label above the line, a loop to oneself below it.
            Item::Message => {
                self.ys.push(top + 20.0);
                self.top += ROW;
            }
            Item::Open(label) => {
                let inset = INSET * (self.open.len() + 1) as f32;
                self.open.push(self.frames.len());
                self.frames
                    .push(frame(inset, top + 2.0, width - 2.0 * inset, label));
                self.top += 24.0;
            }
            Item::Divider(label) => {
                let Some(f) = self.open.last().map(|&i| &self.frames[i]) else {
                    return;
                };
                self.lines.push(divider(f.x, f.x + f.w, top + 2.0, label));
                self.top += 20.0;
            }
            Item::End => {
                if let Some(i) = self.open.pop() {
                    self.frames[i].h = top + 4.0 - self.frames[i].y;
                }
                self.top += 10.0;
            }
            Item::Note {
                from,
                to,
                side,
                text,
            } => {
                let (a, b) = (xs[*from], xs[*to]);
                let (x, r) = match side {
                    -1 => (a - 140.0, a - 10.0),
                    1 => (b + 10.0, b + 140.0),
                    _ => (a - 60.0, b + 60.0),
                };
                let mut note = frame(x.max(2.0), top + 2.0, 0.0, text);
                note.w = r - note.x;
                note.h = 22.0;
                self.frames.push(note);
                self.top += 28.0;
            }
        }
    }
}

fn frame(x: f32, y: f32, w: f32, label: &str) -> ArcFrame {
    ArcFrame {
        x,
        y,
        w,
        h: 0.0,
        label: label.into(),
    }
}

/// A dashed line across a block before its `else`, its condition below.
fn divider(from: f32, to: f32, y: f32, label: &str) -> ArcLine {
    ArcLine {
        path: dashes(from, to, y).into(),
        tone: 5,
        // A space hides the count label.
        text: if label.is_empty() { " " } else { label }.into(),
        lx: from + 10.0,
        ly: y + 16.0,
        size: 11.0,
        ..Default::default()
    }
}

/// A dashed horizontal line from `from` to `to`, either way.
pub fn dashes(from: f32, to: f32, y: f32) -> String {
    let (dir, len) = (if to > from { 1.0 } else { -1.0 }, (to - from).abs());
    let mut path = String::new();
    let mut pos = 0.0;
    while pos < len {
        let end = (pos + 6.0).min(len);
        path += &format!("M {} {y} H {} ", from + dir * pos, from + dir * end);
        pos += 10.0;
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::architecture_ui::mermaid::diagram;

    /// A nested block lies inside its parent; each message row inside the
    /// frames around it; a divider between the rows it separates; a long
    /// label widens its gap.
    #[test]
    fn blocks_frame_their_messages_and_nest() {
        let d = diagram(
            "sequenceDiagram\n  A->>B: eine recht lange Nachricht von A nach B\n  alt x\n    B->>A: a\n    loop y\n      A->>A: b\n    end\n  else z\n    B->>A: c\n  end\n",
        )
        .expect("a sequence");
        let (lanes, frames, lines) = own(&d, &d.items, 50.0);
        assert!(
            lanes.centre(1) - lanes.centre(0) > COLUMN,
            "widened for the label"
        );
        let (outer, inner) = (&frames[0], &frames[1]);
        assert_eq!(
            (outer.label.as_str(), inner.label.as_str()),
            ("alt [x]", "loop [y]")
        );
        let inside = |y: f32, f: &ArcFrame| f.y < y && y < f.y + f.h;
        assert!(inner.x > outer.x && inside(inner.y, outer) && inside(inner.y + inner.h, outer));
        assert!(!inside(lanes.row(0), outer));
        assert!((1..4).all(|k| inside(lanes.row(k), outer)));
        assert!(inside(lanes.row(2), inner) && !inside(lanes.row(3), inner));
        let divider = lines[0].ly - 16.0;
        assert!(lanes.row(2) < divider && divider < lanes.row(3));
        assert!(lanes.bottom() > outer.y + outer.h);
    }
}
