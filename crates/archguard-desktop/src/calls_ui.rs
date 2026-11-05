//! Calls tab: the web viewer's compiler call-site explorer (renderCalls in
//! viewer.js, viewer_calls_tree.js, viewer_call_rankings*.js,
//! viewer_call_cycles*.js, viewer_method_metrics.js). The snapshot comes
//! from `archguard_engine::analyze_calls`, as in the CLI's report.
//!
//! All tab state is one [`CallsState`], built by [`build`] and shown by
//! [`push`] as one `CallsModel` value.

mod actions;
mod cycles;
mod evidence;
mod evidence_view;
mod model;
mod rankings;
mod state;
mod tree;
mod view;

use std::path::Path;

use archguard_engine::core::Config;
use slint::{ComponentHandle, SharedString};

use crate::context::Loaded;
use crate::documents;
use crate::graph_ui::tab_support::{Actions, TabState, index, snapshot_mode};
use crate::ui::{AppWindow, CallsData, CallsModel};
use model::{ElementKind, Labels, Resolution};
use rankings::Sort;
use state::{MEMBER_PAGE, Mode, PAGE, RANK_PAGE, TREE_PAGE, WITNESS_PAGE};

pub use state::State as CallsState;

/// Build the tab state for an analysis (runs the call analysis).
pub fn build(data: &Loaded) -> CallsState {
    build_from(data.root, &data.run.config)
}

fn build_from(root: &Path, config: &Config) -> CallsState {
    let mut s = CallsState::new(archguard_engine::analyze_calls(root, config));
    snapshot_overrides(&mut s);
    s
}

/// Fill the tab after an analysis finished. The call analysis runs on a
/// worker thread; snapshot runs build it in place so the PNG shows it.
pub fn show(ui: &AppWindow, data: &Loaded) {
    // A document keeps its state; switching back only shows it again.
    if documents::with_active_state(|s: &mut CallsState| push(ui, s)).is_some() {
        return;
    }
    let Some(doc) = documents::active_id() else {
        return;
    };
    ui.global::<CallsData>().set_model(CallsModel::default());
    if snapshot_mode() {
        return install(ui, doc, build(data));
    }
    let (root, config) = (data.root.to_path_buf(), data.run.config.clone());
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let state = build_from(&root, &config);
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = weak.upgrade() {
                install(&ui, doc, state);
            }
        });
    });
}

/// Store the state for `doc`; show it only if `doc` is still active.
fn install(ui: &AppWindow, doc: documents::DocId, mut state: CallsState) {
    state.labels = labels(ui);
    state.refresh();
    if documents::set_state(doc, state) {
        documents::with_active_state(|s: &mut CallsState| push(ui, s));
    }
}

fn labels(ui: &AppWindow) -> Labels {
    let d = ui.global::<CallsData>();
    Labels {
        unresolved_target: d.get_label_unresolved_target().into(),
        initializer: d.get_label_initializer().into(),
        unreported_language: d.get_label_unreported_language().into(),
    }
}

fn ui_labels(ui: &AppWindow) -> view::UiLabels {
    let d = ui.global::<CallsData>();
    view::UiLabels {
        all_languages: d.get_label_all_languages(),
        all_packages: d.get_label_all_packages(),
        default_package: d.get_label_default_package(),
        external_group: d.get_label_external_group(),
        unresolved_group: d.get_label_unresolved_group(),
    }
}

/// Show the state in the window.
pub fn push(ui: &AppWindow, s: &CallsState) {
    ui.global::<CallsData>()
        .set_model(view::fill(s, &ui_labels(ui)));
}

impl TabState for CallsState {
    fn push(&mut self, ui: &AppWindow) {
        push(ui, self);
    }

    /// Apply the web's rules after a callback.
    fn changed(&mut self) {
        self.refresh();
    }
}

/// Picker index → value; 0 is "All".
fn pick(list: Vec<String>, i: i32) -> Option<String> {
    index(i)
        .and_then(|i| i.checked_sub(1))
        .and_then(|i| list.into_iter().nth(i))
}

/// Register the tab's callbacks once at start-up.
pub fn wire(ui: &AppWindow) {
    let d = ui.global::<CallsData>();
    let a = Actions::<CallsState>::new(ui);
    d.on_set_language(a.on(|s, _, i: i32| {
        let language = pick(s.idx.languages(), i);
        actions::set_language(s, language);
    }));
    d.on_set_package(a.on(|s, _, i: i32| {
        let package = pick(s.idx.packages(s.filter.language.as_deref()), i);
        actions::set_package(s, package);
    }));
    d.on_set_query(a.on(|s, _, q: SharedString| actions::set_query(s, q.into())));
    d.on_set_resolution(a.on(|s, _, i: i32| actions::set_resolution(s, Resolution::from_index(i))));
    d.on_toggle_internal(a.on0(|s, _| actions::toggle_internal(s)));
    d.on_toggle_analysis(a.on0(|s, _| s.analysis_open = !s.analysis_open));
    d.on_set_mode(a.on(|s, _, i: i32| {
        actions::set_mode(
            s,
            match i {
                1 => Mode::Rankings,
                2 => Mode::Cycles,
                _ => Mode::Connections,
            },
        )
    }));
    wire_tree(ui);
    wire_rankings(ui);
    wire_cycles(ui);
    wire_evidence(ui);
}

fn wire_tree(ui: &AppWindow) {
    let d = ui.global::<CallsData>();
    let a = Actions::<CallsState>::new(ui);
    d.on_tree_toggle(a.on(|s, _, n: i32| {
        if let Some(n) = index(n) {
            actions::tree_toggle(s, n)
        }
    }));
    d.on_tree_select(a.on(|s, _, n: i32| {
        if let Some(n) = index(n) {
            actions::tree_select(s, n)
        }
    }));
    d.on_tree_expand(a.on(|s, _, all: bool| actions::tree_expand(s, all)));
    d.on_tree_clear(a.on0(|s, _| {
        s.element = None;
        s.limit = PAGE;
        s.site_limit = PAGE;
    }));
    d.on_tree_more(a.on0(|s, _| s.tree_limit += TREE_PAGE));
    d.on_select_relation(a.on(|s, _, i: i32| {
        if let Some(r) = index(i).and_then(|i| s.view.relations.get(i)) {
            let key = r.key.clone();
            actions::select_relation(s, key);
        }
    }));
    d.on_relations_more(a.on0(|s, _| s.limit += PAGE));
}

fn wire_rankings(ui: &AppWindow) {
    let d = ui.global::<CallsData>();
    let a = Actions::<CallsState>::new(ui);
    d.on_set_rank_method(a.on(|s, _, m: bool| s.rank_method = m));
    d.on_set_rank_sort(a.on(|s, _, i: i32| s.rank_sort = Sort::from_index(i)));
    d.on_rank_select(a.on(|s, _, i: i32| {
        if let Some(i) = index(i) {
            actions::rank_select(s, i)
        }
    }));
    d.on_rank_more(a.on0(|s, _| s.rank_limit += RANK_PAGE));
    d.on_toggle_rank_help(a.on0(|s, _| s.rank_help = !s.rank_help));
}

fn cycle_kind(method: bool) -> ElementKind {
    if method {
        ElementKind::Method
    } else {
        ElementKind::Class
    }
}

fn wire_cycles(ui: &AppWindow) {
    let d = ui.global::<CallsData>();
    let a = Actions::<CallsState>::new(ui);
    d.on_set_cycle_method(a.on(|s, _, m: bool| actions::set_cycle_kind(s, cycle_kind(m))));
    d.on_cycle_select(a.on(|s, _, i: i32| {
        if let Some(i) = index(i) {
            actions::cycle_select(s, i)
        }
    }));
    d.on_cycles_show_all(a.on0(|s, _| actions::cycles_show_all(s)));
    d.on_cycles_more(a.on0(|s, _| s.cycle.groups += state::CYCLE_PAGE));
    d.on_toggle_cycle_help(a.on0(|s, _| s.cycle_help = !s.cycle_help));
    d.on_path_select(a.on(|s, _, i: i32| {
        let Some(i) = index(i) else { return };
        let step = s
            .group()
            .and_then(|g| Some((g.witness.get(i)?.clone(), g.witness.get(i + 1)?.clone())));
        if let Some((from, to)) = step {
            actions::cycle_edge(s, &from, &to);
        }
    }));
    d.on_path_more(a.on0(|s, _| s.cycle.witness += WITNESS_PAGE));
    d.on_member_select(a.on(|s, _, i: i32| {
        let kind = s.cycle_kind;
        if let Some(id) = index(i).and_then(|i| s.group()?.members.get(i).cloned()) {
            actions::open_in_tree(s, kind, &id);
            actions::select_element(s, kind, &id);
        }
    }));
    d.on_members_more(a.on0(|s, _| s.cycle.members += MEMBER_PAGE));
    d.on_edge_select(a.on(|s, _, i: i32| {
        let edge = index(i)
            .and_then(|i| s.view.cycle_relations.get(i))
            .and_then(|r| r.cycle.clone());
        if let Some((from, to)) = edge {
            actions::cycle_edge(s, &from, &to);
        }
    }));
    d.on_edges_more(a.on0(|s, _| s.cycle.edges += MEMBER_PAGE));
}

fn wire_evidence(ui: &AppWindow) {
    let d = ui.global::<CallsData>();
    let a = Actions::<CallsState>::new(ui);
    d.on_toggle_group(a.on(|s, _, i: i32| {
        let Some(r) = s.view.selected.as_ref() else {
            return;
        };
        let groups = evidence::method_groups(r, &s.idx, s.site_limit);
        let Some(pos) = index(i) else { return };
        if let Some(g) = groups.get(pos) {
            let open = s.open_groups.get(&g.key).copied().unwrap_or(pos == 0);
            s.open_groups.insert(g.key.clone(), !open);
        }
    }));
    d.on_sites_more(a.on0(|s, _| s.site_limit += PAGE));
    d.on_toggle_suggestion(a.on(|s, _, i: i32| {
        let Some(i) = index(i) else { return };
        if !s.open_suggestions.remove(&i) {
            s.open_suggestions.insert(i);
        }
    }));
    d.on_select_method(a.on(|s, _, id: SharedString| {
        actions::select_element(s, ElementKind::Method, &id);
    }));
}

/// Snapshot runs can open a view: `ARCHGUARD_DESKTOP_CALLS_MODE` (0
/// connections, 1 most calls, 2 cycles), `ARCHGUARD_DESKTOP_CALLS_RANK`
/// (`<method 0|1>,<sort 0..4>`), `ARCHGUARD_DESKTOP_CALLS_CYCLE` (0 types,
/// 1 methods), `ARCHGUARD_DESKTOP_CALLS_PACKAGE` (`*` for all) and
/// `ARCHGUARD_DESKTOP_CALLS_ANALYSIS=1` (open the analysis details).
#[cfg(feature = "snapshot")]
fn snapshot_overrides(s: &mut CallsState) {
    let var = |k: &str| std::env::var(k).ok();
    if let Some(p) = var("ARCHGUARD_DESKTOP_CALLS_PACKAGE") {
        actions::set_package(s, (p != "*").then_some(p));
    }
    match var("ARCHGUARD_DESKTOP_CALLS_MODE").as_deref() {
        Some("1") => actions::set_mode(s, Mode::Rankings),
        Some("2") => actions::set_mode(s, Mode::Cycles),
        _ => {}
    }
    if let Some((m, sort)) = var("ARCHGUARD_DESKTOP_CALLS_RANK")
        .as_deref()
        .and_then(|v| v.split_once(','))
    {
        s.rank_method = m == "1";
        s.rank_sort = Sort::from_index(sort.parse().unwrap_or(0));
    }
    if var("ARCHGUARD_DESKTOP_CALLS_CYCLE").as_deref() == Some("1") {
        actions::set_cycle_kind(s, ElementKind::Method);
    }
    s.analysis_open = var("ARCHGUARD_DESKTOP_CALLS_ANALYSIS").as_deref() == Some("1");
}

#[cfg(not(feature = "snapshot"))]
fn snapshot_overrides(_: &mut CallsState) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(name)
            .canonicalize()
            .expect("fixture")
    }

    fn state(root: &Path) -> CallsState {
        let loaded = archguard_engine::load_config(None, root).expect("config");
        build_from(root, &loaded.config)
    }

    /// The Python fixture resolves calls without an external compiler:
    /// package "api" opens with its two connections, as the web viewer.
    #[test]
    fn build_opens_the_first_package_of_a_real_project() {
        let s = state(&fixture("py-layers"));
        assert_eq!(s.idx.a.calls.len(), 14);
        assert_eq!(s.filter.package.as_deref(), Some("api"));
        let v = &s.view;
        assert_eq!(v.relations.len(), 2);
        assert_eq!(v.relations.iter().map(|r| r.calls.len()).sum::<usize>(), 2);
        assert_eq!((v.shown.len(), v.visible.len()), (9, 9));
    }
}
