//! A catalogue diagram (ADR 0006) as the window's `ArcFigure`: cards and
//! curved edges from the layered layout, C4 shapes, one colour per
//! relation kind and a frame around each deployment node or layer.

use std::collections::HashMap;

use archguard_engine::core::{
    Diagram, DiagramElement, DiagramFormat, DiagramKind, ElementKind, RelationKind,
};
use archguard_engine::report::{diagram_mermaid, diagram_plantuml};
use slint::{ModelRc, VecModel};

use super::diagram::{self, Layout};
use crate::tab_support::Texts;
use crate::ui::{ArcCard, ArcFigure, ArcFrame, ArcLine};

fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items))
}

/// `chapter` names the chapter in the diagram view; empty in a chapter.
pub fn figure(d: &Diagram, format: DiagramFormat, chapter: String, texts: &Texts) -> ArcFigure {
    let at: HashMap<&str, usize> = d
        .elements
        .iter()
        .enumerate()
        .map(|(i, e)| (e.id.as_str(), i))
        .collect();
    let relations: Vec<_> = d
        .relations
        .iter()
        .filter_map(|r| Some((r, at.get(r.from.as_str())?, at.get(r.to.as_str())?)))
        .collect();
    let edges: Vec<(usize, usize)> = relations.iter().map(|&(_, a, b)| (*a, *b)).collect();
    let l = diagram::layout(d.elements.len(), &edges);
    let frames = frames(d, &l);
    let framed = |e: &DiagramElement| {
        frames
            .iter()
            .any(|f| Some(f.label.as_str()) == e.group.as_deref())
    };
    let cards = d
        .elements
        .iter()
        .zip(&l.cards)
        .enumerate()
        .map(|(i, (e, c))| card(i, e, c, framed(e), texts))
        .collect();
    let lines = relations
        .iter()
        .zip(&l.routes)
        .map(|((r, _, _), route)| ArcLine {
            path: route.path.as_str().into(),
            head: route.head.as_str().into(),
            count: r.count as i32,
            tone: tone(r.kind),
            text: line_text(r.kind, texts).into(),
            lx: route.label.0,
            ly: route.label.1,
            cycle: r.cycle,
            dim: false,
            strong: false,
        })
        .collect();
    let c4 = d.kind != DiagramKind::Packages;
    ArcFigure {
        id: d.id.as_str().into(),
        chapter: chapter.into(),
        title: d.title.as_str().into(),
        note: d.note.as_str().into(),
        shown: d.shown,
        cards: model(cards),
        lines: model(lines),
        frames: model(frames),
        dw: l.width,
        dh: l.height,
        source: match format {
            DiagramFormat::Mermaid => diagram_mermaid(d),
            DiagramFormat::Plantuml => diagram_plantuml(d),
        }
        .into(),
        legend: if c4 {
            texts.t(LEGEND).into()
        } else {
            Default::default()
        },
    }
}

const LEGEND: &str = "Blue: HTTP · cyan: gRPC · violet: events · green: datastore use · grey: startup order · light grey: configured reference. Counts are configuration evidence.";

fn tone(kind: RelationKind) -> i32 {
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

/// The label of a C4 relation; package maps show their count.
fn line_text(kind: RelationKind, texts: &Texts) -> &str {
    match kind {
        RelationKind::Dependency | RelationKind::Call => "",
        RelationKind::Startup => texts.t("starts first"),
        RelationKind::Http => "HTTP",
        RelationKind::Grpc => "gRPC",
        RelationKind::Event => texts.t("event"),
        RelationKind::Reference => texts.t("reference"),
        RelationKind::Data => texts.t("uses"),
    }
}

fn card(i: usize, e: &DiagramElement, c: &diagram::Card, framed: bool, texts: &Texts) -> ArcCard {
    let (name1, name2) = diagram::name_lines(&e.name);
    let (kind, what) = match e.kind {
        ElementKind::Package => (0, ""),
        ElementKind::Remainder => (2, texts.t("Further packages")),
        ElementKind::Container => (3, texts.t("Container")),
        ElementKind::Database => (4, texts.t("Database")),
        ElementKind::External => (5, texts.t("External container")),
    };
    let tag = match (&e.technology, what) {
        (Some(t), w) if !w.is_empty() => format!("{w} · {t}"),
        (_, w) => w.to_string(),
    };
    // A group without a frame is named on the card.
    let group = e.group.as_deref().filter(|_| !framed);
    let detail = [group, e.detail.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    ArcCard {
        index: i as i32,
        x: c.x,
        y: c.y,
        w: c.w,
        h: c.h,
        kind,
        name1: name1.into(),
        name2: name2.into(),
        files: -1,
        cycle: e.cycle,
        selected: false,
        tag: tag.into(),
        detail: detail.into(),
    }
}

/// A frame per group around its cards, when no other card lies inside.
fn frames(d: &Diagram, l: &Layout) -> Vec<ArcFrame> {
    let mut groups: Vec<&str> = Vec::new();
    for g in d.elements.iter().filter_map(|e| e.group.as_deref()) {
        if !groups.contains(&g) {
            groups.push(g);
        }
    }
    groups
        .into_iter()
        .filter_map(|g| {
            let member = |i: usize| d.elements[i].group.as_deref() == Some(g);
            let cards: Vec<&diagram::Card> = (0..d.elements.len())
                .filter(|&i| member(i))
                .map(|i| &l.cards[i])
                .collect();
            let x = cards.iter().map(|c| c.x).fold(f32::INFINITY, f32::min) - 14.0;
            let y = cards.iter().map(|c| c.y).fold(f32::INFINITY, f32::min) - 36.0;
            let r = cards.iter().map(|c| c.x + c.w).fold(0.0, f32::max) + 14.0;
            let b = cards.iter().map(|c| c.y + c.h).fold(0.0, f32::max) + 12.0;
            let inside = |c: &diagram::Card| c.x < r && c.x + c.w > x && c.y < b && c.y + c.h > y;
            let foreign = (0..d.elements.len()).any(|i| !member(i) && inside(&l.cards[i]));
            (!foreign).then(|| ArcFrame {
                x,
                y,
                w: r - x,
                h: b - y,
                label: g.into(),
            })
        })
        .collect()
}
