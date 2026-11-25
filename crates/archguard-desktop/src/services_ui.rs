//! Services tab: the web viewer's service topology (`renderServices` in
//! `viewer_services.js`). The topology comes from
//! `archguard_engine::analyze_services`, as in the CLI's report.
//!
//! All tab state is one [`ServicesState`], built by [`build_from`] and shown by
//! [`push`] as one `ServicesModel` value.

mod contracts;
mod export;
mod follow;
pub(crate) mod layout;
pub(crate) mod paths;
mod patterns;
pub(crate) mod search;
#[cfg(test)]
mod search_check;
mod sections;
#[cfg(test)]
mod sections_check;
mod state;
mod sync;
#[cfg(test)]
mod tree_check;
pub(crate) mod view;

use std::path::Path;

use archguard_engine::AnalysisRun;
use archguard_engine::core::CallAnalysis;
use slint::ComponentHandle;

use crate::context::Loaded;
use crate::documents;
use crate::graph_ui::{self, simulation::Simulation};
use crate::panes_ui;
use crate::tab_support::{self, Actions, Calls, Shared, TabState, Texts, Topology, index};
use crate::ui::{AppWindow, ServicesData, ServicesModel};
use state::KINDS;
pub use state::{Mode, ServicesState};

/// Build the tab state from the document's topology and, with `run`,
/// its pattern candidates and API contracts.
fn build_from(
    topology: Topology,
    run: Option<(&Path, &AnalysisRun, &CallAnalysis)>,
) -> ServicesState {
    let mut state = state::build_from(topology);
    if let Some((root, run, calls)) = run {
        sections::read(&mut state, root, run, calls);
    }
    snapshot_overrides(&mut state);
    state
}

/// The active document's Graphs show simulated changes.
fn simulated() -> bool {
    documents::with_active_state(|s: &mut Simulation| !s.ops.is_empty()).unwrap_or(false)
}

/// The callbacks of Details → pattern candidates and API contracts.
fn wire_sections(ui: &AppWindow, d: &ServicesData, a: &Actions<ServicesState>) {
    d.on_select_pattern(a.on(|s, _, i: i32| s.patterns.choose(index(i).unwrap_or(usize::MAX))));
    d.on_more_patterns(a.on0(|s, _| s.patterns.more()));
    d.on_more_pattern_evidence(a.on0(|s, _| s.patterns.more_evidence()));
    d.on_select_contract(a.on(|s, _, i: i32| s.contracts.choose(index(i).unwrap_or(usize::MAX))));
    d.on_more_contracts(a.on0(|s, _| s.contracts.more()));
    d.on_more_contract_details(a.on0(|s, _| s.contracts.more_details()));
    d.on_select_search(a.on(|s, _, i: i32| {
        let len = s.topology().map_or(0, |t| t.elasticsearch.artifacts.len());
        s.search.choose(index(i).unwrap_or(usize::MAX), len);
    }));
    d.on_more_search(a.on0(|s, _| s.search.more()));
    d.on_clear_pattern_highlight(a.on0(|s, ui| {
        s.patterns.highlighted = false;
        graph_ui::mark(ui, &[], &[]);
    }));
    // The Graph is changed after the tab's state is put back: revealing
    // switches tabs, which may show this one again.
    let w = ui.as_weak();
    d.on_highlight_pattern(move || {
        let Some(ui) = w.upgrade() else { return };
        let mut marks = None;
        tab_support::update(&ui, |s: &mut ServicesState, _| {
            marks = s.patterns.highlight().filter(|_| !simulated());
            s.patterns.highlighted |= marks.is_some();
        });
        let Some((files, pairs)) = marks else { return };
        if let Some(n) = files.first().and_then(|f| graph_ui::node_named(f)) {
            panes_ui::reveal(&ui, n);
        }
        graph_ui::mark(&ui, &files, &pairs);
    });
}

/// Register the tab's callbacks once at start-up.
pub fn wire(ui: &AppWindow) {
    let d = ui.global::<ServicesData>();
    let a = Actions::<ServicesState>::new(ui);
    d.on_select_kind(a.on(|s, _, i: i32| {
        s.kind = index(i).unwrap_or(0).min(KINDS.len() - 1);
        s.clear_focus();
    }));
    d.on_select_mode(a.on(|s, _, i: i32| {
        s.mode =
            [Mode::All, Mode::Incoming, Mode::Outgoing, Mode::Both][index(i).unwrap_or(0).min(3)];
    }));
    d.on_select_routing(a.on(|s, _, i: i32| s.in_out = i == 1));
    d.on_toggle_cycles_only(a.on0(|s, _| {
        s.cycles_only = !s.cycles_only;
        s.cycle = None;
    }));
    let all = follow::navigation(ui, a.on(|s, _, ()| s.show_all()));
    d.on_show_all(move || all(()));
    d.on_close_details(a.on0(|s, _| s.details_hidden = true));
    d.on_show_more_projects(a.on0(|s, _| s.project_limit += 100));
    let service = a.on(|s, _, i: i32| {
        if let Some(i) = index(i) {
            s.select_service(i)
        }
    });
    d.on_select_service(follow::navigation(ui, service));
    let project = a.on(|s, _, i: i32| {
        if let Some(i) = index(i) {
            s.select_project(i)
        }
    });
    d.on_select_project(follow::navigation(ui, project));
    d.on_select_edge(a.on(|s, _, i: i32| {
        if let Some(i) = index(i) {
            s.select_edge(i)
        }
    }));
    d.on_select_cycle(a.on(|s, _, i: i32| {
        if let Some(i) = index(i) {
            s.select_cycle(i)
        }
    }));
    d.on_zoom_by(a.on(|s, _, f: f32| s.zoom_by(f)));
    d.on_fit(a.on2(|s, ui, w: f32, h: f32| {
        view::fit(s, w, h, Texts::of(ui).t("Unmapped service"));
    }));
    d.on_resized(a.on(|s, _, w: f32| s.viewport = w.floor().max(0.0)));
    wire_sections(ui, &d, &a);
    // `ARCHGUARD_DESKTOP_SVC_OPEN=patterns,contracts,search` opens those sections
    // for snapshot runs (Details itself opens by a click).
    #[cfg(feature = "snapshot")]
    if let Ok(open) = std::env::var("ARCHGUARD_DESKTOP_SVC_OPEN") {
        d.set_patterns_open(open.contains("patterns"));
        d.set_contracts_open(open.contains("contracts"));
        d.set_search_open(open.contains("search"));
    }
    follow::wire();
}

impl TabState for ServicesState {
    fn push(&mut self, ui: &AppWindow) {
        push(ui, self);
    }

    fn pending(ui: &AppWindow, error: Option<&str>) {
        let model = match error {
            None => ServicesModel::default(),
            Some(e) => ServicesModel {
                state: 2,
                error: e.into(),
                ..Default::default()
            },
        };
        ui.global::<ServicesData>().set_model(model);
    }

    /// The view may not have reported its width yet.
    fn arrive(&mut self, ui: &AppWindow) {
        if self.viewport == 0.0 {
            let window = ui.window();
            self.viewport = (window.size().width as f32 / window.scale_factor()).floor();
        }
    }
}

/// Show the active document's tab; its topology is read once per
/// document, on a worker thread.
pub fn show(ui: &AppWindow, data: &Loaded) {
    tab_support::show_async(ui, || {
        let (root, slot) = (data.root.to_path_buf(), Shared::<Topology>::active());
        let set = crate::documents_set::active_set();
        let analysis = documents::with_store(|s| s.active()?.analysis.clone());
        let calls = Calls::of(data);
        move || {
            let topology = tab_support::topology(&slot, &root, set.as_deref());
            let run = analysis
                .as_ref()
                .map(|a| (root.as_path(), &a.run, calls.get()));
            build_from(topology, run.as_ref().map(|(r, a, c)| (*r, *a, &**c)))
        }
    });
}

/// Show the state in the window, in the models the view already shows.
pub fn push(ui: &AppWindow, s: &ServicesState) {
    let d = ui.global::<ServicesData>();
    let mut m = view::build(s, Texts::of(ui).t("Unmapped service"));
    sync::keep(&d.get_model(), &mut m);
    m.patterns = s.patterns.model();
    m.patterns.simulated = simulated();
    m.patterns.highlighted &= !m.patterns.simulated;
    m.contracts = s.contracts.model();
    m.search = s
        .topology()
        .map(|t| s.search.model(t, view::evidence))
        .unwrap_or_default();
    d.set_model(m);
}

/// The active document's Services tab as the export's snapshot and its
/// drawn tree, once its topology is read.
pub fn export_data(ui: &AppWindow) -> Option<(crate::export::Snapshot, crate::export::Diagram)> {
    let project = ui.get_project();
    let unmapped = Texts::of(ui).t("Unmapped service").to_string();
    documents::with_active_state(|s: &mut ServicesState| {
        let snapshot =
            crate::export::services_snapshot(&project, s.topology()?, &export::filters(s));
        Some((snapshot, export::diagram(s, &unmapped)))
    })
    .flatten()
}

/// `ARCHGUARD_DESKTOP_SVC_KIND=<index>`, `ARCHGUARD_DESKTOP_SVC_SERVICE=<name>`
/// and `ARCHGUARD_DESKTOP_SVC_ROUTING=inout` set the view for snapshot runs;
/// `ARCHGUARD_DESKTOP_SVC_PATTERN=<row>`, `ARCHGUARD_DESKTOP_SVC_CONTRACT=<row>`
/// and `ARCHGUARD_DESKTOP_SVC_SEARCH=<artifact>` open a listed pattern
/// assessment, API contract or Elasticsearch artifact (its section opens
/// with `ARCHGUARD_DESKTOP_SVC_OPEN=patterns,contracts,search`, see `wire`).
#[cfg(feature = "snapshot")]
fn snapshot_overrides(s: &mut ServicesState) {
    if let Some(k) = std::env::var("ARCHGUARD_DESKTOP_SVC_KIND")
        .ok()
        .and_then(|k| k.parse::<usize>().ok())
    {
        s.kind = k.min(KINDS.len() - 1);
    }
    if let Ok(name) = std::env::var("ARCHGUARD_DESKTOP_SVC_SERVICE") {
        let i = s
            .topology()
            .and_then(|t| t.services.iter().position(|x| x.name == name));
        if let Some(i) = i {
            s.select_service(i);
        }
    }
    s.in_out = std::env::var("ARCHGUARD_DESKTOP_SVC_ROUTING").is_ok_and(|r| r == "inout");
    let row = |key: &str| {
        std::env::var(key)
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
    };
    if let Some(i) = row("ARCHGUARD_DESKTOP_SVC_PATTERN") {
        s.patterns.choose(i);
    }
    if let Some(i) = row("ARCHGUARD_DESKTOP_SVC_CONTRACT") {
        s.contracts.choose(i);
    }
    if let Some(i) = row("ARCHGUARD_DESKTOP_SVC_SEARCH") {
        let len = s.topology().map_or(0, |t| t.elasticsearch.artifacts.len());
        s.search.choose(i, len);
    }
}

#[cfg(not(feature = "snapshot"))]
fn snapshot_overrides(_: &mut ServicesState) {}
