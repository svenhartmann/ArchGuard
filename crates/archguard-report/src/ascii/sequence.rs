//! A sequence in the terminal: participants as headers with lifelines,
//! one arrow per message in [`sequence_steps`] order, dashed returns,
//! calls to oneself as a loop. Events get a double head (asynchronous),
//! errors an `x` before their text.

use std::collections::HashMap;

use archguard_core::{Diagram, DiagramRelation, RelationKind, SequenceStep, sequence_steps};

use super::canvas::{BLUE, Canvas, E, MAGENTA, N, RED, S, Style, W, YELLOW, fit};

/// Lifeline columns and the drawing so far.
struct Board {
    canvas: Canvas,
    x: Vec<usize>,
    spacing: usize,
    y: usize,
}

pub fn draw(d: &Diagram, style: Style) -> String {
    let index: HashMap<&str, usize> = (d.elements.iter().enumerate())
        .map(|(i, e)| (e.id.as_str(), i))
        .collect();
    let texts: Vec<String> = (d.relations.iter().enumerate())
        .map(|(i, r)| text(d, i, r, style.unicode))
        .collect();
    let mut board = header(d, &texts, style);
    for step in sequence_steps(&d.relations) {
        let (SequenceStep::Call(i) | SequenceStep::Return(i)) = step;
        let r = &d.relations[i];
        let (Some(&a), Some(&b)) = (index.get(r.from.as_str()), index.get(r.to.as_str())) else {
            continue;
        };
        match step {
            SequenceStep::Call(_) if a == b => board.self_call(a, &texts[i], color(r)),
            SequenceStep::Call(_) => board.call(a, b, &texts[i], r),
            SequenceStep::Return(_) if a != b => board.answer(b, a),
            SequenceStep::Return(_) => {}
        }
    }
    board.finish()
}

/// The message text: its label, else its number, kind and count; an `x`
/// before an error.
fn text(d: &Diagram, i: usize, r: &DiagramRelation, unicode: bool) -> String {
    let base = r.label.clone().unwrap_or_else(|| {
        let n = d.relations[..=i]
            .iter()
            .filter(|m| m.label.is_none())
            .count();
        let kind = serde_json::to_value(r.kind).ok();
        let kind = kind.as_ref().and_then(|k| k.as_str()).unwrap_or("");
        format!("{n}. {kind} {}", r.count)
    });
    if r.error {
        format!("{} {base}", if unicode { '✗' } else { 'x' })
    } else {
        base
    }
}

fn color(r: &DiagramRelation) -> u8 {
    match r.kind {
        _ if r.error => RED,
        RelationKind::Http => BLUE,
        RelationKind::Grpc => MAGENTA,
        RelationKind::Event => YELLOW,
        _ => 0,
    }
}

/// Participants side by side, as far apart as the longest message needs
/// and the width allows.
fn header(d: &Diagram, texts: &[String], style: Style) -> Board {
    let n = d.elements.len().max(1);
    let len = |s: &str| s.chars().count();
    let names = d.elements.iter().map(|e| len(&e.name)).max().unwrap_or(0);
    let longest = texts.iter().map(|t| len(t)).max().unwrap_or(0);
    let spacing = (names + 4).max(longest + 4).min(style.width / n).max(10);
    let mut canvas = Canvas::new(style);
    let x: Vec<usize> = (0..n).map(|i| spacing / 2 + i * spacing).collect();
    for (e, &cx) in d.elements.iter().zip(&x) {
        let name = fit(&e.name, spacing - 5, style.unicode);
        let w = len(&name) + 4;
        let left = cx - w / 2;
        canvas.frame(left, 0, w, 3, 0);
        canvas.text(left + 2, 1, &name, 0);
        canvas.line(cx, 2, S, 0);
    }
    Board {
        canvas,
        x,
        spacing,
        y: 3,
    }
}

impl Board {
    fn unicode(&self) -> bool {
        self.canvas.style.unicode
    }

    /// Lifelines down to row `to` (excluded).
    fn lifelines(&mut self, to: usize) {
        for i in 0..self.x.len() {
            for y in self.y..to {
                self.canvas
                    .line(self.x[i], y, if y > 0 { N | S } else { S }, 0);
            }
        }
    }

    /// A horizontal arrow on row `y` from lifeline `a` to `b`.
    fn arrow(&mut self, (a, b): (usize, usize), y: usize, dashed: bool, double: bool, color: u8) {
        let (xa, xb) = (self.x[a], self.x[b]);
        let body = match (self.unicode(), dashed) {
            (true, false) => '─',
            (true, true) => '╌',
            (false, false) => '-',
            (false, true) => '.',
        };
        let right = xb > xa;
        let (lo, hi) = (xa.min(xb) + 1, xa.max(xb) - 1);
        for x in lo..=hi {
            self.canvas.put(x, y, body, color);
        }
        self.canvas.line(xa, y, if right { E } else { W }, color);
        let head = if right { '>' } else { '<' };
        let tip = if right { hi } else { lo };
        self.canvas.put(tip, y, head, color);
        if double {
            let next = if right { tip - 1 } else { tip + 1 };
            self.canvas.put(next, y, head, color);
        }
    }

    fn call(&mut self, a: usize, b: usize, text: &str, r: &DiagramRelation) {
        self.lifelines(self.y + 2);
        let (lo, hi) = (self.x[a].min(self.x[b]), self.x[a].max(self.x[b]));
        let room = (hi - lo).saturating_sub(3);
        let text = fit(text, room.max(1), self.unicode());
        let color = color(r);
        self.canvas.text(lo + 2, self.y, &text, color);
        let event = r.kind == RelationKind::Event;
        self.arrow((a, b), self.y + 1, false, event, color);
        self.y += 2;
    }

    fn answer(&mut self, a: usize, b: usize) {
        self.lifelines(self.y + 1);
        self.arrow((a, b), self.y, true, false, 0);
        self.y += 1;
    }

    /// A loop to the right of the lifeline, the text beside it.
    fn self_call(&mut self, a: usize, text: &str, color: u8) {
        self.lifelines(self.y + 3);
        let x = self.x[a];
        let (top, bottom) = (self.y, self.y + 2);
        self.canvas.hline(top, x, x + 3, color);
        self.canvas.vline(x + 3, top, bottom, color);
        self.canvas.hline(bottom, x, x + 3, color);
        self.canvas.put(x + 1, bottom, '<', color);
        let room = self.spacing.saturating_sub(6).max(1);
        let text = fit(text, room, self.unicode());
        self.canvas.text(x + 5, top, &text, color);
        self.y += 3;
    }

    fn finish(mut self) -> String {
        self.lifelines(self.y + 1);
        self.canvas.render()
    }
}
