//! Quality tab: the web viewer's "Software quality" workspace
//! (`viewer_engineering_quality.js`, `viewer_class_heatmap.js`).
//!
//! All state lives in one [`QualityState`]; [`build`] makes it from an
//! analysis, [`push`] writes its page into `QualityData.model`.

mod card;
mod cards;
mod cohesion;
mod evidence;
mod heatmap;
mod heatmap_text;
mod inputs;
mod lang;
mod notes;
mod rows;
mod state;

use slint::{ComponentHandle, Model, SharedString};

use crate::context::Loaded;
use crate::documents;
use crate::graph_ui::tab_support::{self, Actions, TabState, index, model, snapshot_mode};
use crate::ui::{
    AppWindow, QualityCard, QualityCell, QualityData, QualityGroup, QualityHeatRow, QualityItem,
    QualityLine, QualityModel,
};
use heatmap::State;
use inputs::Inputs;
use lang::Lang;
use notes::Section;
pub use state::QualityState;
use state::{PAGE, Page, View};

/// The tab's state for an analysis (runs the call analysis and Git reads).
pub fn build(data: &Loaded, lang: Lang) -> QualityState {
    let inputs = Inputs::compute(data.root, &data.run.graph, &data.run.config, data.sources);
    QualityState::new(lang, inputs)
}

fn language(ui: &AppWindow) -> Lang {
    Lang {
        de: ui.global::<QualityData>().get_lang() == "de",
    }
}

impl TabState for QualityState {
    fn push(&mut self, ui: &AppWindow) {
        push(ui, self);
    }
}

/// A callback that changes what the user chose.
fn on_view<A: 'static>(
    a: &Actions<QualityState>,
    f: impl Fn(&mut View, A) + 'static,
) -> impl Fn(A) + 'static {
    a.on(move |s, _, arg| f(&mut s.view, arg))
}

/// A picker or row index; negative values pick the first entry.
fn first_or(i: i32) -> usize {
    index(i).unwrap_or(0)
}

/// Register the tab's callbacks once at start-up.
pub fn wire(ui: &AppWindow) {
    let q = ui.global::<QualityData>();
    let a = Actions::<QualityState>::new(ui);
    q.on_section(on_view(&a, |v, i: i32| {
        v.section = Section::ALL[first_or(i).min(Section::ALL.len() - 1)];
        v.limit = PAGE;
    }));
    q.on_query(on_view(&a, |v, t: SharedString| {
        v.query = t.to_string();
        v.limit = PAGE;
    }));
    q.on_target_query(on_view(&a, |v, t: SharedString| {
        v.target_query = t.to_string()
    }));
    q.on_filter(on_view(&a, |v, i: i32| {
        v.filter = state::filter_of(first_or(i));
        v.limit = PAGE;
    }));
    q.on_sort(on_view(&a, |v, i: i32| {
        v.sort = state::sort_of(first_or(i));
        v.limit = PAGE;
    }));
    q.on_more(a.on0(|s, _| s.view.limit += PAGE));
    q.on_select_class(on_view(&a, |v, id: SharedString| {
        v.selected = Some(id.to_string())
    }));
    q.on_back(a.on0(|s, _| s.view.selected = None));
    q.on_package(on_view(&a, |v, node: SharedString| {
        v.cohesion = cohesion::Selection {
            package: Some(node.to_string()),
            ..Default::default()
        };
    }));
    q.on_group(on_view(&a, |v, i: i32| {
        v.cohesion.group = Some(first_or(i))
    }));
    q.on_more_groups(a.on0(|s, _| s.view.cohesion.limit += PAGE));
    wire_targets(ui);
}

fn wire_targets(ui: &AppWindow) {
    let q = ui.global::<QualityData>();
    let w = ui.as_weak();
    q.on_target(move |i| {
        let Some(ui) = w.upgrade() else { return };
        let name = ui
            .global::<QualityData>()
            .get_model()
            .targets
            .row_data(first_or(i));
        tab_support::update(&ui, |s: &mut QualityState, _| {
            // Row 0 is "Entire source scope".
            s.view.target = if i <= 0 {
                String::new()
            } else {
                name.map(|n| n.to_string()).unwrap_or_default()
            };
            s.view.limit = PAGE;
        });
    });
    let w = ui.as_weak();
    q.on_locate(move |name| {
        let Some(ui) = w.upgrade() else { return };
        // The store lock is released before reveal runs, so the Graph
        // tab can take its own state.
        let node = documents::with_active_state(|s: &mut QualityState| {
            let nodes = &s.inputs.as_ref()?.source.nodes;
            nodes.iter().position(|n| n.name == name.as_str())
        })
        .flatten();
        if let Some(n) = node {
            tab_support::reveal(&ui, n);
        }
    });
}

/// Fill the tab after an analysis finished. The measurements take seconds,
/// so they run on a worker thread; snapshot mode waits for them.
pub fn show(ui: &AppWindow, data: &Loaded) {
    // A document keeps its state; switching back only shows it again.
    if documents::with_active_state(|s: &mut QualityState| push(ui, s)).is_some() {
        return;
    }
    let Some(doc) = documents::active_id() else {
        return;
    };
    let lang = language(ui);
    if snapshot_mode() {
        documents::set_state(doc, build(data, lang));
        tab_support::update(ui, |s, _| snapshot_view(s));
        return;
    }
    documents::set_state(doc, QualityState::loading(lang));
    tab_support::update(ui, |_: &mut QualityState, _| {});
    let (root, graph, config, sources) = (
        data.root.to_path_buf(),
        data.run.graph.clone(),
        data.run.config.clone(),
        data.sources.clone(),
    );
    let w = ui.as_weak();
    std::thread::spawn(move || {
        let inputs = Inputs::compute(&root, &graph, &config, &sources);
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = w.upgrade() else { return };
            if documents::set_state(doc, QualityState::new(lang, inputs)) {
                tab_support::update(&ui, |_: &mut QualityState, _| {});
            }
        });
    });
}

/// Snapshot mode opens a section as the web's route does:
/// `ARCHGUARD_DESKTOP_QUALITY=<section>[&query=…][&class=…][&target=…]`.
#[cfg(feature = "snapshot")]
fn snapshot_view(s: &mut QualityState) {
    let Ok(route) = std::env::var("ARCHGUARD_DESKTOP_QUALITY") else {
        return;
    };
    let mut parts = route.split('&');
    let key = parts.next().unwrap_or("");
    if let Some(&section) = Section::ALL.iter().find(|s| s.key() == key) {
        s.view.section = section;
    }
    for (name, value) in parts.filter_map(|p| p.split_once('=')) {
        match name {
            "query" => s.view.query = value.into(),
            "class" => s.view.selected = Some(value.into()),
            "target" => s.view.target = value.into(),
            _ => {}
        }
    }
}

#[cfg(not(feature = "snapshot"))]
fn snapshot_view(_: &mut QualityState) {}

/// Write the state's page into `QualityData.model`.
pub fn push(ui: &AppWindow, s: &mut QualityState) {
    let page = s.page();
    ui.global::<QualityData>()
        .set_model(page_model(s.lang, page));
}

fn page_model(lang: Lang, p: Page) -> QualityModel {
    QualityModel {
        loading: p.loading,
        section: p.section as i32,
        scope_summary: p.scope_summary.into(),
        targets: model(p.targets.into_iter().map(SharedString::from)),
        target_index: p.target_index as i32,
        target_note: p.target_note.into(),
        status: p.status.into(),
        notes: card_model(p.notes),
        heatmap: p.heatmap,
        filter: p.filter as i32,
        sort: p.sort as i32,
        heat: model(p.heat.into_iter().map(heat_row)),
        heat_help: model(
            (0..heatmap::METRICS.len()).map(|i| SharedString::from(heatmap_text::help(lang, i))),
        ),
        detail: model(p.detail.into_iter().map(card_model)),
        cards: model(p.cards.into_iter().map(card_model)),
        more: p.more,
        empty: p.empty.into(),
    }
}

fn line_model(l: card::Line) -> QualityLine {
    QualityLine {
        kind: l.kind as i32,
        text: l.text.into(),
        node: l.node.into(),
    }
}

fn card_model(c: card::Card) -> QualityCard {
    QualityCard {
        key: c.key.into(),
        title: c.title.into(),
        full: c.full.into(),
        action: c.action.into(),
        pressed: c.pressed,
        items: model(c.items.into_iter().map(|i| QualityItem {
            kind: i.kind as i32,
            text: i.text.into(),
            node: i.node.into(),
            value: i.value,
            lines: model(i.lines.into_iter().map(line_model)),
        })),
        metrics: model(c.metrics.into_iter().map(SharedString::from)),
        groups: model(c.groups.into_iter().map(|g| QualityGroup {
            index: g.index as i32,
            label: g.label.into(),
            files: g.files.into(),
            active: g.active,
        })),
        more_groups: c.more_groups,
    }
}

/// `QualityCell.state` codes of `quality_cards.slint`.
fn state_code(s: State) -> i32 {
    match s {
        State::Unknown => 0,
        State::Low => 1,
        State::Medium => 2,
        State::High => 3,
        State::Info => 4,
        State::NotApplicable | State::Small | State::Equal => 5,
    }
}

fn heat_row(r: state::HeatRow) -> QualityHeatRow {
    QualityHeatRow {
        class_id: r.class_id.into(),
        label: r.label.into(),
        file: r.file.into(),
        selected: r.selected,
        cells: model(r.cells.into_iter().map(|c| QualityCell {
            text: c.text.into(),
            state: state_code(c.state),
            tip: c.tip.into(),
        })),
    }
}
