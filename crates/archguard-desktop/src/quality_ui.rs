//! Quality tab: the web viewer's "Software quality" workspace
//! (`viewer_engineering_quality.js`, `viewer_class_heatmap.js`).
//!
//! All state lives in one [`QualityState`]; [`show`] builds it from an
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
use crate::deferred::{Coalesce, TYPING};
use crate::documents;
use crate::model_sync::{sync_rows, sync_rows_with};
use crate::tab_support::{self, Actions, Calls, TabState, Texts, index, model};
use crate::ui::{
    AppWindow, QualityCard, QualityCell, QualityData, QualityGroup, QualityHeatRow, QualityItem,
    QualityLine, QualityModel,
};
use inputs::Inputs;
use notes::Section;
pub use state::QualityState;
use state::{PAGE, Page, View};

impl TabState for QualityState {
    fn push(&mut self, ui: &AppWindow) {
        push(ui, self);
    }

    /// The loading page, or the failure in place of the records.
    fn pending(ui: &AppWindow, error: Option<&str>) {
        let lang = Texts::of(ui);
        let old = QualityModel::default();
        let mut m = page_model(&lang, QualityState::loading(lang.clone()).page(), &old);
        if let Some(e) = error {
            m.loading = false;
            m.empty = e.into();
        }
        ui.global::<QualityData>().set_model(m);
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
    let query = on_view(&a, |v, t: SharedString| {
        v.query = t.to_string();
        v.limit = PAGE;
    });
    q.on_query(Coalesce::debounce(TYPING, query).callback());
    let target_query = on_view(&a, |v, t: SharedString| v.target_query = t.to_string());
    q.on_target_query(Coalesce::debounce(TYPING, target_query).callback());
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
            crate::panes_ui::reveal(&ui, n);
        }
    });
}

/// Show the active document's tab. The measurements take seconds; they
/// run once per document, on a worker thread.
pub fn show(ui: &AppWindow, data: &Loaded) {
    tab_support::show_async(ui, || {
        let lang = Texts::of(ui);
        let calls = Calls::of(data);
        let (root, graph, config, sources) = (
            data.root.to_path_buf(),
            data.run.graph.clone(),
            data.run.config.clone(),
            data.sources.clone(),
        );
        move || {
            let mut s = QualityState::new(
                lang,
                Inputs::compute(&root, &graph, &config, &sources, &calls.get()),
            );
            snapshot_view(&mut s);
            s
        }
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
/// The active document's Quality tab as the export's snapshot: every
/// loaded report, with the tab's section, target and search.
pub fn export(ui: &AppWindow) -> Option<crate::export::Snapshot> {
    use crate::export::{QualityFilters, QualityInput, quality_snapshot};
    let project = ui.get_project();
    documents::with_active_state(|s: &mut QualityState| {
        let i = s.inputs.as_ref()?;
        let input = QualityInput {
            code: &i.code,
            knowledge: i.knowledge.as_ref(),
            measurements: &i.measurements,
            cohesion: &i.cohesion,
            history: &i.history,
            tests: &i.tests,
            mutations: &i.mutations,
            duplicates: &i.duplicates,
        };
        let filters = QualityFilters {
            section: s.view.section.key().into(),
            target: Some(s.view.target.clone()).filter(|t| !t.is_empty()),
            query: s.view.query.clone(),
        };
        Some(quality_snapshot(&project, &input, &filters))
    })
    .flatten()
}

/// The page in the models the view shows, changed in place: a keystroke
/// in a search field redraws only the cards that changed.
pub fn push(ui: &AppWindow, s: &mut QualityState) {
    let page = s.page();
    let old = ui.global::<QualityData>().get_model();
    let mut m = page_model(&s.lang, page, &old);
    m.query = s.view.query.as_str().into();
    m.target_query = s.view.target_query.as_str().into();
    ui.global::<QualityData>().set_model(m);
}

fn page_model(lang: &Texts, p: Page, old: &QualityModel) -> QualityModel {
    let mut notes = card_model(p.notes);
    adopt_card(&old.notes, &mut notes);
    QualityModel {
        loading: p.loading,
        section: p.section as i32,
        scope_summary: p.scope_summary.into(),
        targets: sync_rows(&old.targets, p.targets.into_iter().map(SharedString::from)),
        target_index: p.target_index as i32,
        target_note: p.target_note.into(),
        status: p.status.into(),
        status_slot: p.status_slot,
        status_detail: p.status_detail.into(),
        notes,
        heatmap: p.heatmap,
        filter: p.filter as i32,
        sort: p.sort as i32,
        heat: sync_rows_with(&old.heat, p.heat.into_iter().map(heat_row), |o, n| {
            n.cells = sync_rows(&o.cells, n.cells.iter());
        }),
        heat_help: sync_rows(
            &old.heat_help,
            (0..heatmap::METRICS.len()).map(|i| SharedString::from(heatmap_text::help(lang, i))),
        ),
        detail: sync_rows_with(
            &old.detail,
            p.detail.into_iter().map(card_model),
            adopt_card,
        ),
        cards: sync_rows_with(&old.cards, p.cards.into_iter().map(card_model), adopt_card),
        more: p.more,
        empty: p.empty.into(),
        ..Default::default()
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

/// `new` takes over the inner models of `old`, synced to its contents.
fn adopt_card(old: &QualityCard, new: &mut QualityCard) {
    new.items = sync_rows_with(&old.items, new.items.iter(), |o, n| {
        n.lines = sync_rows(&o.lines, n.lines.iter());
    });
    new.metrics = sync_rows(&old.metrics, new.metrics.iter());
    new.groups = sync_rows(&old.groups, new.groups.iter());
}

fn heat_row(r: state::HeatRow) -> QualityHeatRow {
    QualityHeatRow {
        class_id: r.class_id.into(),
        label: r.label.into(),
        file: r.file.into(),
        selected: r.selected,
        cells: model(r.cells.into_iter().map(|c| QualityCell {
            text: c.text.into(),
            state: c.state.slot(),
            tip: c.tip.into(),
        })),
    }
}
