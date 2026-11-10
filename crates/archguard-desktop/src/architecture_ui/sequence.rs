//! A sequence (catalogue kind `sequence`) as the window's `ArcFigure`, drawn
//! like a UML sequence diagram: participants above and below their dashed
//! lifelines, one horizontal message per step from top to bottom, an
//! activation bar on the callee while a call runs, and a dashed return to
//! the caller once the calls made within it are done.

use archguard_engine::core::{
    Diagram, DiagramElement, DiagramRelation, ElementKind, RelationKind, SequenceStep as Step,
    sequence_steps as steps,
};
use slint::{ModelRc, VecModel};

use super::diagram;
use crate::tab_support::Texts;
use crate::ui::{ArcCard, ArcFigure, ArcLine};

const LEGEND: &str = "Blue: HTTP · cyan: gRPC · violet: events · red: recorded error · dashed: return to the caller. Top to bottom in measured start order.";
/// Scenarios derived from the code (only calls): no measured order.
const STATIC_LEGEND: &str = "Blue: calls · violet: events (async) · red: error path · dashed: return to the caller. Top to bottom in source order, derived from the code, not measured.";
const COLUMN: f32 = 170.0;
const CARD_W: f32 = 150.0;
const MARGIN: f32 = 20.0;
const TOP: f32 = 14.0;
const ROW: f32 = 34.0;
/// Half the width of an activation bar; nested bars shift by `NEST`.
const BAR: f32 = 5.0;
const NEST: f32 = 6.0;
/// Message labels in px before zoom (other diagrams use 9).
const LABEL: f32 = 12.0;

/// The lanes of one figure: participant centres and step rows.
struct Lanes {
    first: f32,
}

impl Lanes {
    fn centre(i: usize) -> f32 {
        MARGIN + i as f32 * COLUMN + CARD_W / 2.0
    }
    fn row(&self, k: usize) -> f32 {
        self.first + k as f32 * ROW
    }
}

pub fn figure(d: &Diagram, source: String, chapter: String, texts: &Texts) -> ArcFigure {
    let two_lines = d
        .elements
        .iter()
        .any(|e| !diagram::name_lines(&e.name).1.is_empty());
    let card_h = if two_lines { 66.0 } else { 50.0 };
    let lanes = Lanes {
        first: TOP + card_h + 30.0,
    };
    let rows = steps(&d.relations);
    let bottom = lanes.row(rows.len());
    let classes = d.id.starts_with("runtime-scenario-classes-");
    let cards: Vec<ArcCard> = [TOP, bottom]
        .into_iter()
        .flat_map(|y| {
            d.elements.iter().enumerate().map(move |(i, e)| ArcCard {
                y,
                h: card_h,
                ..participant(i, e, classes, texts)
            })
        })
        .collect();
    let lifelines = (0..d.elements.len()).map(|i| lifeline(Lanes::centre(i), TOP + card_h, bottom));
    let lines: Vec<ArcLine> = lifelines
        .chain(bars(d, &rows, &lanes))
        .chain(messages(d, &rows, &lanes))
        .collect();
    ArcFigure {
        id: d.id.as_str().into(),
        chapter: chapter.into(),
        title: d.title.as_str().into(),
        note: d.note.as_str().into(),
        shown: d.shown,
        cards: ModelRc::new(VecModel::from(cards)),
        lines: ModelRc::new(VecModel::from(lines)),
        frames: Default::default(),
        dw: 2.0 * MARGIN + d.elements.len().max(1) as f32 * COLUMN - (COLUMN - CARD_W),
        dh: bottom + card_h + 14.0,
        source: source.into(),
        legend: legend(d, texts).into(),
        sequence: true,
    }
}

fn at(d: &Diagram, id: &str) -> Option<usize> {
    d.elements.iter().position(|e| e.id == id)
}

/// A dashed vertical line below a participant.
fn lifeline(x: f32, from: f32, to: f32) -> ArcLine {
    let mut path = String::new();
    let mut y = from;
    while y < to {
        path += &format!("M {x} {y} V {} ", (y + 5.0).min(to));
        y += 9.0;
    }
    ArcLine {
        path: path.into(),
        tone: 5,
        // A space hides the count label.
        text: " ".into(),
        ..Default::default()
    }
}

/// `(participant, first row, last row)` of each call that keeps its callee
/// busy: until its return, or until the explicit reply to it.
fn activations(d: &Diagram, rows: &[Step]) -> Vec<(usize, usize, usize)> {
    let row_of = |want: &Step| rows.iter().position(|s| s == want);
    let reply_to = |i: usize| {
        let j = d
            .relations
            .iter()
            .position(|r| r.reply && r.caller == Some(i))?;
        row_of(&Step::Call(j))
    };
    (0..d.relations.len())
        .filter(|&i| d.relations[i].kind != RelationKind::Event && !d.relations[i].reply)
        .filter_map(|i| {
            let start = row_of(&Step::Call(i))?;
            let end = row_of(&Step::Return(i)).or_else(|| reply_to(i))?;
            Some((at(d, &d.relations[i].to)?, start, end))
        })
        .collect()
}

/// How many bars of `p` are still open at `row` (the nesting there).
fn depth(bars: &[(usize, usize, usize)], p: usize, row: usize) -> usize {
    bars.iter()
        .filter(|&&(q, s, e)| q == p && s < row && row <= e)
        .count()
}

/// The activation bars, light grey, nested ones shifted right.
fn bars<'a>(d: &Diagram, rows: &[Step], lanes: &'a Lanes) -> impl Iterator<Item = ArcLine> + 'a {
    let all = activations(d, rows);
    let placed: Vec<_> = all
        .iter()
        .map(|&(p, s, e)| (p, s, e, depth(&all, p, s)))
        .collect();
    placed.into_iter().map(move |(p, s, e, n)| {
        let x = Lanes::centre(p) - BAR + n as f32 * NEST;
        let (top, low) = (lanes.row(s), lanes.row(e));
        let rect = format!("M {x} {top} H {} V {low} H {x} Z", x + 2.0 * BAR);
        ArcLine {
            path: rect.clone().into(),
            head: rect.into(),
            tone: 5,
            text: " ".into(),
            dim: true,
            ..Default::default()
        }
    })
}

/// Calls and returns, each on its own row, ending at the bars' edges.
fn messages<'a>(
    d: &'a Diagram,
    rows: &'a [Step],
    lanes: &'a Lanes,
) -> impl Iterator<Item = ArcLine> + 'a {
    rows.iter().enumerate().filter_map(|(k, step)| {
        let y = lanes.row(k);
        let (Step::Call(i) | Step::Return(i)) = *step;
        let r = &d.relations[i];
        let (a, b) = (Lanes::centre(at(d, &r.from)?), Lanes::centre(at(d, &r.to)?));
        let own = a == b;
        let dir = if b > a { 1.0 } else { -1.0 };
        let (a, b) = if own {
            (a + BAR, b)
        } else {
            (a + dir * BAR, b - dir * BAR)
        };
        if let Step::Return(_) = step {
            // A call to oneself returns where it was made.
            return (!own).then(|| answer(b, a, y, r));
        }
        let (path, head) = if own {
            self_call(a, y)
        } else {
            let back = b - dir * 9.0;
            let head = format!("M {b} {y} L {back} {} L {back} {} Z", y - 4.5, y + 4.5);
            (format!("M {a} {y} H {b}"), head)
        };
        Some(ArcLine {
            path: path.into(),
            head: head.into(),
            tone: tone(r.kind),
            text: r.label.clone().unwrap_or_default().into(),
            lx: a.min(b) + 8.0,
            ly: y - 4.0,
            cycle: r.error,
            size: LABEL,
            ..Default::default()
        })
    })
}

/// A small loop from the bar's right edge `x` back onto the nested bar.
fn self_call(x: f32, y: f32) -> (String, String) {
    let (out, low, end) = (x + 30.0, y + 14.0, x + NEST);
    let path = format!("M {x} {y} H {out} V {low} H {end}");
    let head = format!(
        "M {end} {low} L {} {} L {} {} Z",
        end + 9.0,
        low - 4.5,
        end + 9.0,
        low + 4.5
    );
    (path, head)
}

/// A dashed return from `from` back to the caller at `to`, open arrowhead.
fn answer(from: f32, to: f32, y: f32, call: &DiagramRelation) -> ArcLine {
    let (dir, len) = (if to > from { 1.0 } else { -1.0 }, (to - from).abs());
    let mut path = String::new();
    let mut pos = 0.0;
    while pos < len {
        let end = (pos + 6.0).min(len);
        path += &format!("M {} {y} H {} ", from + dir * pos, from + dir * end);
        pos += 10.0;
    }
    let back = to - dir * 9.0;
    path += &format!("M {back} {} L {to} {y} L {back} {}", y - 4.5, y + 4.5);
    ArcLine {
        path: path.into(),
        tone: 5,
        text: " ".into(),
        cycle: call.error,
        ..Default::default()
    }
}

/// The line colour of a relation kind (`ArcLine.tone`).
pub fn tone(kind: RelationKind) -> i32 {
    match kind {
        RelationKind::Dependency | RelationKind::Call => 0,
        RelationKind::Startup | RelationKind::Runs => 1,
        RelationKind::Http => 2,
        RelationKind::Grpc => 3,
        RelationKind::Event => 4,
        RelationKind::Reference => 5,
        RelationKind::Data => 6,
    }
}

/// A participant's head card (placed by the caller). Its namespace
/// (`group`, e.g. the crate of a module) is the card's header line;
/// building blocks of a scenario are packages, or classes in a class
/// scenario; traces show services.
fn participant(i: usize, e: &DiagramElement, classes: bool, texts: &Texts) -> ArcCard {
    let (name1, name2) = diagram::name_lines(&e.name);
    let tag = match (&e.group, e.kind) {
        (Some(space), _) => space.as_str(),
        (None, ElementKind::Package) if classes => texts.t("Class"),
        (None, ElementKind::Package) => "",
        (None, ElementKind::Person) => texts.t("Person"),
        (None, ElementKind::External) => texts.t("External"),
        _ => texts.t("Container"),
    };
    let kind = match e.kind {
        ElementKind::Person | ElementKind::External => 5,
        // A plain box: the namespace replaces the package header.
        ElementKind::Package if e.group.is_some() => 2,
        ElementKind::Package => 0,
        _ => 3,
    };
    ArcCard {
        index: i as i32,
        x: Lanes::centre(i) - CARD_W / 2.0,
        w: CARD_W,
        kind,
        name1: name1.into(),
        name2: name2.into(),
        files: -1,
        tag: tag.into(),
        ..Default::default()
    }
}

/// Only a recorded trace (`runtime-trace-…`, `runtime-recorded-…`) has a
/// measured order; code scenarios and service flows are derived from the source.
fn legend<'a>(d: &Diagram, texts: &'a Texts) -> &'a str {
    if d.id.starts_with("runtime-trace-") || d.id.starts_with("runtime-recorded-") {
        texts.t(LEGEND)
    } else {
        texts.t(STATIC_LEGEND)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(from: &str, to: &str, caller: Option<usize>) -> DiagramRelation {
        DiagramRelation {
            from: from.into(),
            to: to.into(),
            kind: RelationKind::Call,
            count: 1,
            cycle: false,
            label: None,
            error: false,
            caller,
            reply: false,
        }
    }

    #[test]
    fn a_bar_runs_from_each_call_to_its_return_and_nests_on_reentry() {
        let element = |id: &str| DiagramElement {
            id: id.into(),
            name: id.into(),
            kind: ElementKind::Package,
            technology: None,
            detail: None,
            group: None,
            cycle: false,
        };
        let d = Diagram {
            id: "x".into(),
            chapter: 6,
            title: String::new(),
            kind: archguard_engine::core::DiagramKind::Sequence,
            note: String::new(),
            unit: String::new(),
            elements: vec![element("a"), element("b"), element("c")],
            relations: vec![
                call("a", "b", None),
                call("b", "c", Some(0)),
                call("c", "b", Some(1)),
            ],
            shown: true,
        };
        let rows = steps(&d.relations);
        // Call 0, Call 1, Call 2, Return 2, Return 1, Return 0.
        let bars = activations(&d, &rows);
        assert_eq!(bars, [(1, 0, 5), (2, 1, 4), (1, 2, 3)]);
        assert_eq!(depth(&bars, 1, 2), 1, "b called again while busy: nested");
        assert_eq!(depth(&bars, 2, 1), 0);
    }
}
