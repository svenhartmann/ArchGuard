//! View wiring to Slint, ticket 12: the one place where the class models become Slint data.
//!
//! Glyphs, checked in the snapshots of 2026-11-26/27: ✎ ⌕ ◇ « » → ⚠ ↓ render in the app font, ⑂
//! (U+2442) and 👁 (U+1F441) do not. The multi-field mark is therefore "›" (`es_class_canvas.slint`); the
//! counters of the list use `boxes::WRITE_GLYPH` and `READ_GLYPH`.

use archguard_engine::SearchGraph;
use archguard_engine::core::ServiceTopology;
use slint::SharedString;

use super::boxes::{
    self, BoxContent, ChainClass, Chip, Compartment, Line, Prefix, Stage, Title, content,
};
use super::details::{details, field_details};
use super::diagram::{BoxKind, DBox, Diagram, EdgeStyle, diagram_field};
use super::layout::{Placed, PlacedEdge, Scene, field_rows, layout};
use super::list::{GroupTitle, ListGroup, ListRow, list, members};
use super::types::Key;
use super::view_details::slint_details;
use super::wrap::fit_box;
use crate::diagram::dash_rect;
use crate::es_ui::state::EsState;
use crate::tab_support::model;
use crate::ui::{
    EsClassBox, EsClassChip, EsClassCompartment, EsClassEdge, EsClassGroup, EsClassLine,
    EsClassModel, EsClassRow, EsDetails,
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

/// The field of the selected index in focus.
fn field_of<'a>(s: &'a EsState, key: &Key) -> Option<&'a str> {
    let (k, path) = s.field.as_ref()?;
    (k == key).then_some(path.as_str())
}

struct Canvas {
    d: Diagram,
    scene: Scene,
    boxes: Vec<EsClassBox>,
    /// Per box the (top, path) of its field lines.
    rows: Vec<Vec<(f32, String)>>,
}

/// The diagram of `key` (the whole list with the cluster switch), laid out and as Slint boxes.
fn canvas(t: &ServiceTopology, s: &EsState, key: &Key, rows: &[Key]) -> Canvas {
    let field = field_of(s, key);
    let cluster = s.cluster.then_some(rows);
    let d = diagram_field(t, &s.graph, key, cluster, &s.switches, field);
    let mut contents: Vec<BoxContent> = d
        .boxes
        .iter()
        .map(|b| content(t, &s.graph, &b.key, &s.switches))
        .collect();
    let scene = layout(&d, &contents);
    for (c, p) in contents.iter_mut().zip(&scene.boxes) {
        fit_box(c, p.w);
    }
    let placed = d.boxes.iter().zip(&contents).zip(&scene.boxes);
    let boxes = (placed.clone())
        .map(|((b, c), p)| slint_box(b, c, p, field))
        .collect();
    let rows = placed.map(|((_, c), p)| field_rows(c, p)).collect();
    Canvas {
        d,
        scene,
        boxes,
        rows,
    }
}

fn details_model(t: &ServiceTopology, s: &EsState, key: &Key) -> EsDetails {
    let field = field_of(s, key);
    let focus = match (key, field) {
        (Key::Node(i), Some(p)) => field_details(t, &s.graph, *i, p),
        _ => None,
    };
    slint_details(
        &s.graph,
        key,
        &details(t, &s.graph, key),
        focus.as_ref(),
        field,
    )
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
    let Canvas {
        d,
        scene,
        boxes,
        rows: box_rows,
    } = canvas(t, s, &key, &rows);
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
        fit_tick: s.fit_tick,
        details: current
            .as_ref()
            .map(|k| details_model(t, s, k))
            .unwrap_or_default(),
        problem: SharedString::from(s.problem.as_str()),
    };
    s.shown = current;
    s.box_rows = box_rows;
    s.row_keys = rows;
    s.box_keys = d.boxes.into_iter().map(|b| b.key).collect();
    s.canvas = (scene.width, scene.height);
    out
}

/// The selection's name in the scope line.
fn crumb(g: &SearchGraph, k: &Key) -> String {
    match k {
        Key::Node(i) => g
            .nodes
            .get(*i)
            .map(|n| n.short_label.clone())
            .unwrap_or_default(),
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

/// `field`: the path of the field in focus; only the selected box shows it.
fn slint_box(b: &DBox, c: &BoxContent, p: &Placed, field: Option<&str>) -> EsClassBox {
    let mut chips = p.chips.iter();
    let focus = field.filter(|_| b.selected);
    let compartments = (c.compartments.iter().zip(&p.comps))
        .map(|(comp, &y)| slint_compartment(comp, y, focus, &mut chips));
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
        dim: b.dim,
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
    focus: Option<&str>,
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
        lines: model(c.lines.iter().map(|l| slint_line(l, focus))),
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

fn slint_line(l: &Line, focus: Option<&str>) -> EsClassLine {
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
        focus: !l.path.is_empty() && focus == Some(l.path.as_str()),
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
        dim: e.dim,
    }
}

/// Smallest zoom "Fit" sets; a larger canvas scrolls instead of shrinking below legibility.
const MIN_FIT: f32 = 0.4;

/// "Fit" (never enlarges): the whole canvas into `w` × `h` (the canvas's viewport).
pub fn fit(s: &mut EsState, w: f32, h: f32) {
    let (cw, ch) = s.canvas;
    if cw > 0.0 && ch > 0.0 {
        s.zoom = ((w - 24.0).max(1.0) / cw)
            .min((h - 24.0).max(1.0) / ch)
            .clamp(MIN_FIT, 1.0);
    }
}

/// The snapshot hooks of `es_ui.rs` (`ARCHGUARD_DESKTOP_ES_FOCUS`, `_ES_FIELD`, `_ES_CLUSTER`, `_ES_KIND`).
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
    // `#<n>`: the n-th row of the list (the corpus probe prints n, so no names are needed).
    let rows: Vec<_> = list(t, &s.graph, "", Group::All)
        .into_iter()
        .flat_map(|g| g.rows)
        .collect();
    let by_number = want.strip_prefix('#').and_then(|n| n.parse::<usize>().ok());
    let found = match by_number {
        Some(n) => rows.get(n),
        None => rows
            .iter()
            .find(|r| r.label == want || format!("{}:{}", r.side, r.label) == want),
    };
    s.selected = found.map(|r| r.key.clone());
    if let (Some(key), Ok(path)) = (
        s.selected.clone(),
        std::env::var("ARCHGUARD_DESKTOP_ES_FIELD"),
    ) {
        // `#<k>`: the k-th field of the selected index.
        let by_number = path.strip_prefix('#').and_then(|k| k.parse::<usize>().ok());
        let named = match (&key, by_number) {
            (Key::Node(i), Some(k)) => (s.graph.nodes.get(*i))
                .and_then(|n| n.artifact)
                .and_then(|a| t.elasticsearch.artifacts.get(a))
                .and_then(|a| a.detail.as_ref())
                .and_then(|d| d.fields.get(k))
                .map(|f| f.path.clone()),
            _ => None,
        };
        s.field = Some((key, named.unwrap_or(path)));
    }
}
