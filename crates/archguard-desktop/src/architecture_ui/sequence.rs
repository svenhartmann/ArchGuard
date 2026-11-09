//! A recorded request (catalogue kind `sequence`) as the window's
//! `ArcFigure`: participants side by side, a lifeline below each and one
//! horizontal message per step, top to bottom in measured order. Each
//! synchronous call gets a dashed return to its caller once the calls made
//! within it are done.

use archguard_engine::core::{Diagram, DiagramElement, DiagramRelation, ElementKind, RelationKind};
use slint::{ModelRc, VecModel};

use super::diagram;
use crate::tab_support::Texts;
use crate::ui::{ArcCard, ArcFigure, ArcLine};

const LEGEND: &str = "Blue: HTTP · cyan: gRPC · violet: events · red: recorded error · dashed: return to the caller. Top to bottom in measured start order.";
/// Scenarios derived from the code (only calls): no measured order.
const STATIC_LEGEND: &str = "Blue: calls · violet: events (async) · red: error path · dashed: return to the caller. Top to bottom in source order, derived from the code, not measured.";
const COLUMN: f32 = 230.0;
const CARD_W: f32 = 180.0;
const CARD_H: f32 = 62.0;
const MARGIN: f32 = 30.0;
const TOP: f32 = 20.0;
const ROW: f32 = 46.0;

pub fn figure(d: &Diagram, source: String, chapter: String, texts: &Texts) -> ArcFigure {
    let centre = |i: usize| MARGIN + i as f32 * COLUMN + CARD_W / 2.0;
    let at = |id: &str| d.elements.iter().position(|e| e.id == id);
    let first = TOP + CARD_H + 34.0;
    let classes = d.id.starts_with("runtime-scenario-classes-");
    let rows = steps(&d.relations);
    let bottom = first + rows.len() as f32 * ROW;
    let cards: Vec<ArcCard> = d
        .elements
        .iter()
        .enumerate()
        .map(|(i, e)| participant(i, e, centre(i), classes, texts))
        .collect();
    let lifelines = (0..d.elements.len()).map(|i| ArcLine {
        path: format!("M {} {} V {bottom}", centre(i), TOP + CARD_H).into(),
        tone: 5,
        // A space hides the count label.
        text: " ".into(),
        ..Default::default()
    });
    let messages = rows.iter().enumerate().filter_map(|(k, step)| {
        let y = first + k as f32 * ROW;
        let (Step::Call(i) | Step::Return(i)) = *step;
        let r = &d.relations[i];
        let (a, b) = (centre(at(&r.from)?), centre(at(&r.to)?));
        if let Step::Return(_) = step {
            return Some(answer(b, a, y, r));
        }
        let back = if b > a { b - 9.0 } else { b + 9.0 };
        Some(ArcLine {
            path: format!("M {a} {y} H {b}").into(),
            head: format!("M {b} {y} L {back} {} L {back} {} Z", y - 4.5, y + 4.5).into(),
            tone: tone(r.kind),
            text: r.label.clone().unwrap_or_default().into(),
            lx: a.min(b) + 10.0,
            ly: y - 5.0,
            cycle: r.error,
            ..Default::default()
        })
    });
    let lines: Vec<ArcLine> = lifelines.chain(messages).collect();
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
        dh: bottom + 24.0,
        source: source.into(),
        legend: legend(d, texts).into(),
        sequence: true,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Step {
    /// Index into `relations`.
    Call(usize),
    /// The return of that call to its caller.
    Return(usize),
}

/// Messages in order, with a return for each call once the calls made
/// within it (`caller`) are done. Events and explicit replies get none.
fn steps(relations: &[DiagramRelation]) -> Vec<Step> {
    let (mut out, mut open): (Vec<Step>, Vec<usize>) = (Vec::new(), Vec::new());
    for (i, r) in relations.iter().enumerate() {
        // The nearest enclosing message still open: a collapsed scenario
        // can send from a call that already returned. Callers only point
        // back, so the walk ends.
        let mut within = r.caller.filter(|&c| c < i);
        while let Some(c) = within
            && !open.contains(&c)
        {
            within = relations[c].caller.filter(|&p| p < c);
        }
        while let Some(&top) = open.last()
            && Some(top) != within
        {
            open.pop();
            if relations[top].kind != RelationKind::Event {
                out.push(Step::Return(top));
            }
        }
        out.push(Step::Call(i));
        if !r.reply {
            open.push(i);
        } else if within.is_some() && within == r.caller {
            open.pop();
        }
    }
    let rest = open.into_iter().rev();
    out.extend(
        rest.filter(|&i| relations[i].kind != RelationKind::Event)
            .map(Step::Return),
    );
    out
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

/// A participant's card above its lifeline at `centre`. Building blocks of
/// a scenario are packages, or classes in a class scenario; traces show
/// services.
fn participant(i: usize, e: &DiagramElement, centre: f32, classes: bool, texts: &Texts) -> ArcCard {
    let (name1, name2) = diagram::name_lines(&e.name);
    let package = e.kind == ElementKind::Package;
    let tag = match e.kind {
        ElementKind::Package if classes => texts.t("Class"),
        ElementKind::Package => "",
        ElementKind::Person => texts.t("Person"),
        ElementKind::External => texts.t("External"),
        _ => texts.t("Container"),
    };
    ArcCard {
        index: i as i32,
        x: centre - CARD_W / 2.0,
        y: TOP,
        w: CARD_W,
        h: CARD_H,
        kind: if package {
            0
        } else if matches!(e.kind, ElementKind::Person | ElementKind::External) {
            5
        } else {
            3
        },
        name1: name1.into(),
        name2: name2.into(),
        files: -1,
        tag: tag.into(),
        ..Default::default()
    }
}

/// Only a recorded trace (`runtime-trace-…`) has a measured order; code
/// scenarios and service flows are derived from the source.
fn legend<'a>(d: &Diagram, texts: &'a Texts) -> &'a str {
    if d.id.starts_with("runtime-trace-") {
        texts.t(LEGEND)
    } else {
        texts.t(STATIC_LEGEND)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Step::{Call, Return};

    fn rel(from: &str, to: &str, caller: Option<usize>) -> DiagramRelation {
        DiagramRelation {
            from: from.into(),
            to: to.into(),
            kind: RelationKind::Http,
            count: 1,
            cycle: false,
            label: None,
            error: false,
            caller,
            reply: false,
        }
    }

    /// service-shop: catalog asks pricing, pricing asks promotions,
    /// promotions asks catalog again; only then catalog asks inventory.
    #[test]
    fn a_call_returns_to_the_activation_that_made_it() {
        let r = [
            rel("user", "gateway", None),
            rel("gateway", "catalog", Some(0)),
            rel("catalog", "pricing", Some(1)),
            rel("pricing", "promotions", Some(2)),
            rel("promotions", "catalog", Some(3)),
            rel("catalog", "inventory", Some(1)),
        ];
        let want = [
            Call(0),
            Call(1),
            Call(2),
            Call(3),
            Call(4),
            Return(4),
            Return(3),
            Return(2),
            Call(5),
            Return(5),
            Return(1),
            Return(0),
        ];
        assert_eq!(steps(&r), want);
    }

    #[test]
    fn an_explicit_reply_closes_its_call_without_a_drawn_return() {
        let reply = |from, to, caller| DiagramRelation {
            reply: true,
            error: true,
            ..rel(from, to, Some(caller))
        };
        let r = [
            rel("user", "a", None),
            rel("a", "b", Some(0)),
            reply("b", "a", 1),
            reply("a", "user", 0),
        ];
        assert_eq!(steps(&r), [Call(0), Call(1), Call(2), Call(3)]);
    }

    #[test]
    fn an_event_never_returns_but_calls_made_on_it_do() {
        let r = [
            DiagramRelation {
                kind: RelationKind::Event,
                ..rel("a", "b", None)
            },
            rel("b", "c", Some(0)),
        ];
        assert_eq!(steps(&r), [Call(0), Call(1), Return(1)]);
    }
}
