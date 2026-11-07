//! A recorded request (catalogue kind `sequence`) as the window's
//! `ArcFigure`: participants side by side, a lifeline below each and one
//! horizontal message per step, top to bottom in measured order.

use archguard_engine::core::{Diagram, RelationKind};
use slint::{ModelRc, VecModel};

use super::diagram;
use crate::tab_support::Texts;
use crate::ui::{ArcCard, ArcFigure, ArcLine};

const LEGEND: &str = "Blue: HTTP · cyan: gRPC · violet: events · red: recorded error. Top to bottom in measured start order.";
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
    let bottom = first + d.relations.len() as f32 * ROW;
    let cards: Vec<ArcCard> = d
        .elements
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let (name1, name2) = diagram::name_lines(&e.name);
            ArcCard {
                index: i as i32,
                x: centre(i) - CARD_W / 2.0,
                y: TOP,
                w: CARD_W,
                h: CARD_H,
                kind: 3,
                name1: name1.into(),
                name2: name2.into(),
                files: -1,
                tag: texts.t("Container").into(),
                ..Default::default()
            }
        })
        .collect();
    let lifelines = (0..d.elements.len()).map(|i| ArcLine {
        path: format!("M {} {} V {bottom}", centre(i), TOP + CARD_H).into(),
        tone: 5,
        // A space hides the count label.
        text: " ".into(),
        ..Default::default()
    });
    let messages = d.relations.iter().enumerate().filter_map(|(k, r)| {
        let (a, b) = (centre(at(&r.from)?), centre(at(&r.to)?));
        let y = first + k as f32 * ROW;
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
        legend: texts.t(LEGEND).into(),
    }
}

/// The line colour of a relation kind (`ArcLine.tone`).
pub fn tone(kind: RelationKind) -> i32 {
    match kind {
        RelationKind::Dependency | RelationKind::Call => 0,
        RelationKind::Startup => 1,
        RelationKind::Http => 2,
        RelationKind::Grpc => 3,
        RelationKind::Event => 4,
        RelationKind::Reference => 5,
        RelationKind::Data => 6,
    }
}
