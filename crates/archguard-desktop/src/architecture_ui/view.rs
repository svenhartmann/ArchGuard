//! The tab state as the window's `ArchitectureModel` values.

use archguard_engine::core::Graph;
use archguard_engine::report::Arc42Document;
use slint::{ModelRc, SharedString, VecModel};

use super::projection::Group;
use super::visual::{self, Action, Kind, Map, RELATIONS_PAGE};
use super::{blocks, diagram, markdown};
use crate::ui::{
    ArcCard, ArcChapter, ArcLevelItem, ArcLine, ArcMap, ArcRelation, ArchitectureModel,
};

fn model<T: Clone + 'static>(items: impl IntoIterator<Item = T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items.into_iter().collect::<Vec<_>>()))
}

pub fn fill_document(
    m: &mut ArchitectureModel,
    doc: &Arc42Document,
    chapter: u8,
    note: &str,
    derived: bool,
) {
    m.chapters = model(doc.chapters.iter().map(|c| ArcChapter {
        number: c.number.into(),
        label: format!("{:02}", c.number).into(),
        title: c.title.into(),
    }));
    let about = markdown::parse(blocks::preamble(&doc.markdown(None)));
    m.about = model(blocks::ui_blocks(&about, note));
    let Some(c) = doc
        .chapters
        .iter()
        .find(|c| c.number == chapter)
        .or(doc.chapters.first())
    else {
        return;
    };
    let markdown = doc.markdown(Some(c.number));
    let body = blocks::chapter_body(&markdown, c.number, derived);
    m.chapter = c.number.into();
    m.chapter_title = format!("{}. {}", c.number, c.title).into();
    m.blocks = model(blocks::ui_blocks(&markdown::parse(&body), note));
    m.source = markdown.into();
}

/// The map value; `relations` is how many relationships are listed.
pub fn map_model(
    graph: &Graph,
    calls_status: &str,
    relations: usize,
    kind: Kind,
    map: &Map,
    overview: &SharedString,
) -> ArcMap {
    let mut m = ArcMap {
        kind: kind as i32,
        total: map.projection.total as i32,
        internal: map.projection.internal as i32,
        calls_status: calls_status.into(),
        scopes: model(map.scopes.iter().map(|sc| match sc {
            None => overview.clone(),
            Some(n) => graph.nodes[*n].name.as_str().into(),
        })),
        scope_index: map
            .scopes
            .iter()
            .position(|sc| *sc == map.scope)
            .unwrap_or(0) as i32,
        ..Default::default()
    };
    fill_levels(&mut m, map, graph, overview);
    fill_diagram(&mut m, map);
    fill_relations(&mut m, map, relations.max(RELATIONS_PAGE));
    m
}

fn fill_levels(m: &mut ArcMap, map: &Map, graph: &Graph, overview: &SharedString) {
    let Some(levels) = &map.levels else { return };
    let item = |scope: Option<usize>, level: u8, label: String| ArcLevelItem {
        scope: scope.map_or(-1, |s| s as i32),
        level: level.into(),
        label: if scope.is_none() {
            overview.clone()
        } else {
            label.into()
        },
        current: scope == levels.scope,
    };
    m.level = levels.level.into();
    m.levels = model(
        visual::level_items(levels, graph)
            .into_iter()
            .map(|(s, l, label)| item(s, l, label)),
    );
    let short = |n: usize| {
        graph.nodes[n]
            .name
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_string()
    };
    m.crumbs = model(
        std::iter::once(item(None, 1, String::new())).chain(
            levels
                .path
                .iter()
                .map(|e| item(Some(e.node), e.level, short(e.node))),
        ),
    );
}

fn card(map: &Map, i: usize) -> ArcCard {
    let (n, c) = (&map.shown.nodes[i], &map.layout.cards[i]);
    let (name1, name2) = diagram::name_lines(&n.name);
    ArcCard {
        index: i as i32,
        x: c.x,
        y: c.y,
        w: c.w,
        h: c.h,
        kind: match n.group {
            Group::Package(_) => 0,
            Group::Direct => 1,
            Group::Outside => 2,
        },
        name1: name1.into(),
        name2: name2.into(),
        files: n.files as i32,
        cycle: n.cycle,
        selected: map.selected == Some(i),
    }
}

fn fill_diagram(m: &mut ArcMap, map: &Map) {
    let l = &map.layout;
    m.dw = l.width;
    m.dh = l.height;
    m.cards = model((0..map.shown.nodes.len()).map(|i| card(map, i)));
    let touches = |from: usize, to: usize| map.selected.map(|s| from == s || to == s);
    m.lines = model(map.shown.edges.iter().zip(&l.routes).map(|(e, r)| ArcLine {
        path: r.path.as_str().into(),
        head: r.head.as_str().into(),
        count: e.count as i32,
        lx: r.label.0,
        ly: r.label.1,
        cycle: e.cycle,
        dim: touches(e.from, e.to) == Some(false),
        strong: touches(e.from, e.to) == Some(true),
    }));
    let selected = map.selected.map(|s| &map.shown.nodes[s]);
    m.selected_name = selected.map(|n| n.name.as_str()).unwrap_or_default().into();
    m.selected_action = match map.action {
        Action::None => 0,
        Action::OpenLevel(_) => 1,
        Action::OpenInGraph(_) => 2,
        Action::OpenPackage(_) => 3,
    };
}

fn fill_relations(m: &mut ArcMap, map: &Map, limit: usize) {
    let all = visual::relations(map);
    m.relation_total = map.shown.edges.len() as i32;
    m.relations_more = all.len() > limit;
    m.relations = model(all.iter().take(limit).map(|e| ArcRelation {
        from: map.shown.nodes[e.from].name.as_str().into(),
        to: map.shown.nodes[e.to].name.as_str().into(),
        count: e.count as i32,
    }));
}
