//! View wiring to Slint, ticket 12: the one place where the class models become Slint data.
//!
//! Glyphs, checked in the snapshot of 2026-11-26: ✎ 👁 ⌕ ◇ « » → ⚠ render in the app font, ⑂
//! (U+2442) does not. The multi-field mark is therefore "›" (`es_class_canvas.slint`); the
//! counters of the list use `boxes::WRITE_GLYPH` and `READ_GLYPH`.

use archguard_engine::SearchGraph;
use archguard_engine::core::ServiceTopology;
use slint::SharedString;

use super::boxes::{
    self, BoxContent, ChainClass, Chip, Compartment, Line, Prefix, Stage, Title, content,
};
use super::details::details;
use super::diagram::{BoxKind, DBox, Diagram, EdgeStyle, diagram};
use super::layout::{Placed, PlacedEdge, Scene, layout};
use super::list::{GroupTitle, ListGroup, ListRow, list, members};
use super::types::Key;
use super::view_details::slint_details;
use crate::diagram::dash_rect;
use crate::es_ui::state::EsState;
use crate::tab_support::model;
use crate::ui::{
    EsClassBox, EsClassChip, EsClassCompartment, EsClassEdge, EsClassGroup, EsClassLine,
    EsClassModel, EsClassRow,
};

/// Group header and row height of the list; `es_class_list.slint` draws the same pitch.
const GROUP_H: f32 = 30.0;
const ROW_H: f32 = 26.0;

/// The selection: the user's, else the first index of the list (else its first row).
fn current(s: &EsState, rows: &[Key]) -> Option<Key> {
    let index = |k: &&Key| matches!(k, Key::Node(i) if s.graph.nodes[*i].kind == "index");
    s.selected
        .clone()
        .or_else(|| rows.iter().find(index).or(rows.first()).cloned())
}

/// The diagram of `key` (the whole list with the cluster switch), laid out and as Slint boxes.
fn canvas(
    t: &ServiceTopology,
    s: &EsState,
    key: &Key,
    rows: &[Key],
) -> (Diagram, Scene, Vec<EsClassBox>) {
    let d = diagram(t, &s.graph, key, s.cluster.then_some(rows), &s.switches);
    let contents: Vec<BoxContent> = d
        .boxes
        .iter()
        .map(|b| content(t, &s.graph, &b.key, &s.switches))
        .collect();
    let scene = layout(&d, &contents);
    let boxes = (d.boxes.iter().zip(&contents).zip(&scene.boxes))
        .map(|((b, c), p)| slint_box(b, c, p))
        .collect();
    (d, scene, boxes)
}

/// The model of the whole view; remembers the keys the callbacks' indices stand for.
pub fn build(s: &mut EsState) -> EsClassModel {
    let topology = s.topology.clone();
    let Ok(t) = (*topology).as_ref() else {
        return EsClassModel {
            state: 2,
            ..Default::default()
        };
    };
    let groups = list(t, &s.graph, &s.query, s.group);
    let rows = members(&groups);
    let current = current(s, &rows);
    let key = current.clone().unwrap_or(Key::Node(usize::MAX));
    let (d, scene, boxes) = canvas(t, s, &key, &rows);
    let out = EsClassModel {
        state: 1,
        switches: model([
            s.switches.services,
            s.switches.analysis,
            s.switches.multi_fields,
            s.switches.collapse_fields,
        ]),
        cluster: s.cluster,
        crumb_name: current
            .as_ref()
            .map(|k| crumb(&s.graph, k))
            .unwrap_or_default()
            .into(),
        crumb_count: d.boxes.len() as i32,
        groups: model(slint_groups(&groups, current.as_ref())),
        query: SharedString::from(s.query.as_str()),
        kind: s.group as i32,
        count: rows.len() as i32,
        write_glyph: boxes::WRITE_GLYPH.into(),
        read_glyph: boxes::READ_GLYPH.into(),
        boxes: model(boxes),
        edges: model(scene.edges.iter().map(slint_edge)),
        canvas_w: scene.width,
        canvas_h: scene.height,
        zoom: s.zoom,
        details: current
            .as_ref()
            .map(|k| slint_details(&s.graph, k, &details(t, &s.graph, k)))
            .unwrap_or_default(),
        problem: SharedString::from(s.problem.as_str()),
    };
    s.row_keys = rows;
    s.box_keys = d.boxes.into_iter().map(|b| b.key).collect();
    s.canvas = (scene.width, scene.height);
    out
}

/// The selection's name in the scope line.
fn crumb(g: &SearchGraph, k: &Key) -> String {
    match k {
        Key::Node(i) => g.nodes.get(*i).map(|n| n.label.clone()).unwrap_or_default(),
        Key::Part { name, .. } => name.clone(),
    }
}

fn group_code(t: &GroupTitle) -> (i32, &str) {
    match t {
        GroupTitle::Services => (0, ""),
        GroupTitle::Indices(root) => (1, root),
        GroupTitle::IndicesUndeclared => (1, ""),
        GroupTitle::Aliases => (2, ""),
        GroupTitle::Templates => (3, ""),
        GroupTitle::Analysis => (4, ""),
        GroupTitle::Processing => (5, ""),
    }
}

/// Groups with flat row positions and the row tops (`GROUP_H` and `ROW_H` px).
fn slint_groups(groups: &[ListGroup], current: Option<&Key>) -> Vec<EsClassGroup> {
    let (mut index, mut y) = (0, 0.0);
    groups
        .iter()
        .map(|g| {
            y += GROUP_H;
            let rows: Vec<EsClassRow> = g
                .rows
                .iter()
                .map(|r| {
                    let row = slint_row(r, index, y, current);
                    index += 1;
                    y += ROW_H;
                    row
                })
                .collect();
            let (title, root) = group_code(&g.title);
            EsClassGroup {
                title,
                root: root.into(),
                rows: model(rows),
            }
        })
        .collect()
}

fn slint_row(r: &ListRow, index: i32, y: f32, current: Option<&Key>) -> EsClassRow {
    EsClassRow {
        label: r.label.as_str().into(),
        side: r.side.as_str().into(),
        writes: r.writes as i32,
        reads: r.reads as i32,
        warn: r.warn,
        selected: current == Some(&r.key),
        index,
        y,
    }
}

/// The box kind code of `es_class_canvas.slint`: 4 analysis block, the rest neutral.
fn box_code(k: BoxKind) -> i32 {
    match k {
        BoxKind::Service => 0,
        BoxKind::Index => 1,
        BoxKind::Template => 2,
        BoxKind::Alias => 3,
        BoxKind::Processing => 5,
        _ => 4,
    }
}

fn slint_box(b: &DBox, c: &BoxContent, p: &Placed) -> EsClassBox {
    let mut chips = p.chips.iter();
    let compartments = c
        .compartments
        .iter()
        .zip(&p.comps)
        .map(|(comp, &y)| slint_compartment(comp, y, &mut chips));
    EsClassBox {
        x: p.x,
        y: p.y,
        w: p.w,
        h: p.h,
        kind: box_code(b.kind),
        stereotype: c.stereotype.as_str().into(),
        name: c.name.as_str().into(),
        root: c.root.as_str().into(),
        ghost: b.ghost,
        warn: b.warn,
        selected: b.selected,
        border: if b.ghost {
            dash_rect(p.w, p.h).into()
        } else {
            SharedString::new()
        },
        compartments: model(compartments),
    }
}

/// `chips` yields the box's chip positions in compartment order.
fn slint_compartment<'a>(
    c: &Compartment,
    y: f32,
    chips: &mut impl Iterator<Item = &'a (f32, f32, f32)>,
) -> EsClassCompartment {
    let placed: Vec<EsClassChip> = c
        .chips
        .iter()
        .zip(chips)
        .map(|(chip, &(x, y, w))| slint_chip(chip, x, y, w))
        .collect();
    EsClassCompartment {
        title: title_code(c.title),
        y,
        lines: model(c.lines.iter().map(slint_line)),
        chips: model(placed),
    }
}

fn title_code(t: Title) -> i32 {
    match t {
        Title::Fields => 0,
        Title::Settings => 1,
        Title::Analysis => 2,
        Title::Chain => 3,
        Title::Parameters => 4,
        Title::UsedBy => 5,
        Title::PointsTo => 6,
        Title::Access => 7,
        Title::Patterns => 8,
    }
}

fn slint_chip(c: &Chip, x: f32, y: f32, w: f32) -> EsClassChip {
    EsClassChip {
        text: c.text.as_str().into(),
        stage: match c.stage {
            Stage::CharFilter => 0,
            Stage::Tokenizer => 1,
            Stage::Filter => 2,
        },
        // Built in and undeclared chips are outlines; declared and inline ones are filled.
        class: i32::from(matches!(
            c.class,
            ChainClass::Builtin | ChainClass::Undeclared
        )),
        x,
        y,
        w,
    }
}

fn slint_line(l: &Line) -> EsClassLine {
    EsClassLine {
        depth: l.depth as i32,
        prefix: match l.prefix {
            Prefix::None => 0,
            Prefix::Field => 1,
            Prefix::Multi => 2,
        },
        text: l.text.as_str().into(),
        kind: l.kind_text.as_str().into(),
        note: l.note.as_str().into(),
        summary: l.summary.unwrap_or(0) as i32,
    }
}

fn slint_edge(e: &PlacedEdge) -> EsClassEdge {
    EsClassEdge {
        line: e.line.as_str().into(),
        head: e.head.as_str().into(),
        shape: e.shape as i32,
        dashed: e.dashed,
        style: match e.style {
            EdgeStyle::Write => 1,
            EdgeStyle::Read => 2,
            _ => 0,
        },
        label: e.label.as_str().into(),
        lx: e.lx,
        ly: e.ly,
    }
}

/// "Fit": the whole canvas into `w` × `h` (the canvas's viewport).
pub fn fit(s: &mut EsState, w: f32, h: f32) {
    let (cw, ch) = s.canvas;
    if cw > 0.0 && ch > 0.0 {
        s.zoom = ((w - 24.0).max(1.0) / cw)
            .min((h - 24.0).max(1.0) / ch)
            .clamp(0.01, 3.0);
    }
}

/// The snapshot hooks of `es_ui.rs` (`ARCHGUARD_DESKTOP_ES_FOCUS`, `_ES_CLUSTER`, `_ES_KIND`).
#[cfg(feature = "snapshot")]
pub fn snapshot(s: &mut EsState) {
    use super::types::Group;
    const KINDS: [&str; 7] = [
        "all",
        "services",
        "indices",
        "aliases",
        "templates",
        "analysis",
        "processing",
    ];
    if let Ok(kind) = std::env::var("ARCHGUARD_DESKTOP_ES_KIND") {
        let at = KINDS.iter().position(|k| *k == kind.to_lowercase());
        s.group = Group::from_index(at.unwrap_or(0));
    }
    s.cluster = std::env::var("ARCHGUARD_DESKTOP_ES_CLUSTER").as_deref() == Ok("1");
    let Ok(want) = std::env::var("ARCHGUARD_DESKTOP_ES_FOCUS") else {
        return;
    };
    let Ok(t) = (*s.topology).as_ref() else {
        return;
    };
    let found = list(t, &s.graph, "", Group::All)
        .into_iter()
        .flat_map(|g| g.rows)
        .find(|r| r.label == want || format!("{}:{}", r.side, r.label) == want);
    s.selected = found.map(|r| r.key);
}
