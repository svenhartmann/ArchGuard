//! Document tabs in the window: open a project, analyse it on a worker
//! thread, switch, close. The store itself is [`documents`]. Opening,
//! switching and closing update the recent list, the session and the
//! project's last view in the settings ([`settings_file`]).
//!
//! Every tab's `show(ui, &Loaded)` runs when a document with a finished
//! analysis becomes active: after its analysis and on every switch. Tabs
//! that keep their state in [`documents`] only push it again then.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use archguard_engine::{Level, analyze_run_quiet, collect_review_sources, load_config};
use slint::{ComponentHandle, SharedString};

use crate::context::{Analysis, Loaded};
use crate::documents::{self, DocId, Status, SubProject, with_store};
use crate::graph_ui;
use crate::panes::Layout;
use crate::panes_ui;
use crate::settings::{self, ProjectView, Session};
use crate::settings_file;
use crate::tab_support::{self, Shared, Texts, Topology, model as vec_model};
use crate::ui::{AppWindow, DocItem, RecentItem, SubProjectItem};
use crate::{
    architecture_ui, calls_ui, file_ui, instances, matrix_ui, quality_ui, runtime_ui, security_ui,
    services_ui, start_ui, web,
};

/// Register the callbacks of the document strip and of every tab.
pub fn wire(ui: &AppWindow) {
    graph_ui::wire(ui);
    matrix_ui::wire(ui);
    file_ui::wire(ui);
    start_ui::wire(ui);
    architecture_ui::wire(ui);
    calls_ui::wire(ui);
    services_ui::wire(ui);
    runtime_ui::wire(ui);
    quality_ui::wire(ui);
    security_ui::wire(ui);
    crate::export_ui::wire(ui);
    crate::follow_ui::wire(ui);
    panes_ui::wire(ui);
    ui.on_select_doc(with_ui(ui, |ui, i: i32| activate(ui, index(i))));
    ui.on_open_sub_project(with_ui(ui, |ui, i: i32| open_sub_project(ui, index(i))));
    let add = with_ui(ui, |ui, ()| pick_folder(ui));
    ui.on_add_doc(move || add(()));
    ui.on_open_web(with_ui(ui, |ui, tab: SharedString| open_web(ui, &tab)));
    let strip = with_ui(ui, |ui, ()| push_strip(ui));
    documents::on_dirty_changed(move || strip(()));
    // Notice 2 reads "Could not save the changes: …" (`ui/app.slint`).
    settings_file::on_error(with_ui(ui, |ui, e: String| {
        ui.set_notice_kind(2);
        ui.set_notice(e.into());
    }));
}

pub use crate::tab_support::with_ui;

pub fn index(i: i32) -> usize {
    usize::try_from(i).unwrap_or(usize::MAX)
}

/// Open `root` as the active document; analyse it if it is new.
pub fn open(ui: &AppWindow, root: PathBuf) {
    let root = root.canonicalize().unwrap_or(root);
    save_view(ui);
    let (id, new) = with_store(|s| s.open(&root));
    if new {
        seed_view(id, &root);
        spawn_analysis(ui, id, root.clone());
    }
    settings_file::edit(|s| s.add_recent(&root));
    record_session();
    push_recent(ui);
    refresh(ui);
}

/// A newly opened project shows the view it had when it was last open.
fn seed_view(id: DocId, root: &Path) {
    let Some(view) = settings_file::get(|s| s.project(root).cloned()) else {
        return;
    };
    // A file written before panes has the Graph's display per project.
    let layout = view.layout.unwrap_or_else(|| {
        let mut l = Layout::single(&view.tab);
        l.panes[0].tabs[0].display = view.display;
        l
    });
    documents::set_state(id, layout);
}

/// Store the open documents as the session.
fn record_session() {
    let session = with_store(|s| Session {
        documents: s.docs().iter().map(|d| d.root.clone()).collect(),
        active: s.active_index().unwrap_or(0),
    });
    settings_file::update(|s| s.session = Some(session));
}

/// Show the recent projects in the menu and on the start screen.
pub fn push_recent(ui: &AppWindow) {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let items: Vec<RecentItem> = settings_file::get(|s| {
        s.recent
            .iter()
            .map(|p| {
                let (name, path) = settings::recent_label(p, home.as_deref());
                RecentItem {
                    name: name.into(),
                    path: path.into(),
                }
            })
            .collect()
    });
    ui.set_recent(vec_model(items));
}

/// Show the document at tab `index`.
pub fn activate(ui: &AppWindow, index: usize) {
    save_view(ui);
    if with_store(|s| s.activate(index)) {
        record_session();
        refresh(ui);
    }
}

/// Close the document at tab `index` without asking; `shell_ui` asks
/// about unsaved changes first.
pub fn close(ui: &AppWindow, index: usize) {
    save_view(ui);
    let Some(doc) = with_store(|s| s.close(index)) else {
        return;
    };
    let root = doc.root.clone();
    // Drop the document's state before the window shows the next one.
    drop(doc);
    std::thread::spawn(move || web::stop(&root));
    record_session();
    refresh(ui);
}

fn open_sub_project(ui: &AppWindow, index: usize) {
    ui.set_projects_open(false);
    let root = with_store(|s| {
        let doc = s.active()?;
        Some(documents::project_root(&doc.root, doc.projects.get(index)?))
    });
    if let Some(root) = root {
        open(ui, root);
    }
}

fn pick_folder(ui: &AppWindow) {
    let start = with_store(|s| s.active().map(|d| d.root.clone()));
    let mut dialog = rfd::FileDialog::new().set_title(ui.get_open_title().as_str());
    if let Some(dir) = start {
        dialog = dialog.set_directory(dir);
    }
    if let Some(dir) = dialog.pick_folder() {
        open(ui, dir);
    }
}

/// The analysis of one document, as the CLI runs it, plus the monorepo's
/// sub-projects. Blocking: runs on a worker thread.
fn analyze(
    root: &Path,
    topology: &Shared<Topology>,
) -> Result<(Analysis, Vec<SubProject>), String> {
    let loaded = load_config(None, root).map_err(|e| e.to_string())?;
    let run = analyze_run_quiet(root, &loaded, Level::Module).map_err(|e| e.to_string())?;
    // Git evidence for review priorities; without it the churn component
    // is zero, as in the CLI.
    let named = (&run.graph, &run.source_names);
    let sources = collect_review_sources(root, &run.config, named, &run.config.review.git_since)
        .unwrap_or_default();
    // Read once per document; the Services tab reuses the same slot.
    let projects = match &*tab_support::topology(topology, root) {
        Ok(t) => documents::build_units(t),
        Err(_) => Vec::new(),
    };
    let analysis = Analysis {
        root: root.to_path_buf(),
        run,
        sources,
    };
    Ok((analysis, projects))
}

fn spawn_analysis(ui: &AppWindow, id: DocId, root: PathBuf) {
    let topology = Shared::<Topology>::new();
    documents::set_state(id, topology.clone());
    let weak = ui.as_weak();
    let name = "desktop: analyze project";
    let work = tab_support::traced(&root.clone(), name, move || analyze(&root, &topology));
    std::thread::spawn(move || {
        let result = guarded(work);
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = weak.upgrade() {
                finished(&ui, id, result);
            }
        });
    });
}

/// Run a document's analysis on its worker thread. A panic becomes the
/// document's error, so it leaves "Loading" either way.
fn guarded<T>(analyze: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    tab_support::catch(analyze).and_then(|r| r)
}

fn finished(ui: &AppWindow, id: DocId, result: Result<(Analysis, Vec<SubProject>), String>) {
    let active = with_store(|s| {
        let Some(doc) = s.get_mut(id) else {
            return false;
        };
        match result {
            Ok((analysis, projects)) => {
                doc.analysis = Some(Arc::new(analysis));
                doc.projects = projects;
                doc.status = Status::Ready;
            }
            Err(e) => doc.status = Status::Failed(e),
        }
        s.active().is_some_and(|d| d.id == id)
    });
    if active {
        refresh(ui);
    } else {
        push_strip(ui);
    }
    #[cfg(feature = "snapshot")]
    if with_store(|s| s.docs().iter().all(|d| d.status != Status::Loading)) {
        crate::snapshot::schedule(ui);
    }
}

/// Remember the active document's view as the project's last view in the
/// settings; `display` and `explorer_tab` are those of the Graph the view
/// bar shows (each Graph tab keeps its own, see `pane_store`).
pub fn save_view(_ui: &AppWindow) {
    let Some(root) = with_store(|s| s.active().map(|d| d.root.clone())) else {
        return;
    };
    let layout = panes_ui::layout();
    let graph = layout
        .find("graph")
        .map(|(p, t)| layout.panes[p].tabs[t].id);
    let (display, explorer_tab) = graph
        .and_then(|id| {
            instances::with(id, |s: &mut graph_ui::Graph| {
                (s.g.display, s.g.explorer_tab)
            })
        })
        .unwrap_or_default();
    let view = ProjectView {
        tab: layout.focused().to_string(),
        display,
        explorer_tab,
        layout: Some(layout),
    };
    settings_file::edit(|s| s.set_project(&root, view));
}

fn restore_view(ui: &AppWindow) {
    panes_ui::push(ui);
}

fn push_strip(ui: &AppWindow) {
    let (items, active) = with_store(|s| {
        let items: Vec<DocItem> = s
            .docs()
            .iter()
            .map(|d| DocItem {
                name: d.name.as_str().into(),
                path: d.root.display().to_string().into(),
                loading: d.status == Status::Loading,
                failed: matches!(d.status, Status::Failed(_)),
                dirty: d.is_dirty(),
            })
            .collect();
        (items, s.active_index())
    });
    ui.set_docs(vec_model(items));
    ui.set_active_doc(active.map_or(-1, |i| i as i32));
}

/// Show the active document: strip, header, status and every tab.
pub fn refresh(ui: &AppWindow) {
    push_strip(ui);
    ui.set_projects_open(false);
    ui.set_web_status(SharedString::new());
    let doc = with_store(|s| {
        s.active().map(|d| {
            let projects: Vec<SubProject> = d.projects.clone();
            (
                d.name.clone(),
                d.root.clone(),
                d.status.clone(),
                d.analysis.clone(),
                projects,
            )
        })
    });
    let Some((name, root, status, analysis, projects)) = doc else {
        ui.set_has_doc(false);
        ui.set_project(SharedString::new());
        ui.set_sub_projects(vec_model(Vec::new()));
        return;
    };
    ui.set_has_doc(true);
    ui.set_project(name.into());
    ui.set_root_path(root.display().to_string().into());
    ui.set_status(root.display().to_string().into());
    ui.set_loading(status == Status::Loading);
    ui.set_error(match status {
        Status::Failed(e) => e.into(),
        _ => SharedString::new(),
    });
    let texts = Texts::of(ui);
    ui.set_sub_projects(vec_model(projects.into_iter().map(|p| SubProjectItem {
        name: p.name.into(),
        kind: texts.t(&p.kind).into(),
        path: p.path.into(),
    })));
    restore_view(ui);
    if let Some(analysis) = analysis {
        show_tabs(ui, &analysis.loaded());
    }
}

fn show_tabs(ui: &AppWindow, data: &Loaded) {
    start_ui::show(ui, data);
    architecture_ui::show(ui, data);
    calls_ui::show(ui, data);
    services_ui::show(ui, data);
    runtime_ui::show(ui, data);
    quality_ui::show(ui, data);
    security_ui::show(ui, data);
    // Graph, Matrix and File tabs: their instances for the layout.
    panes_ui::sync(ui);
    graph_ui::show(ui);
    ui.invoke_fit_views();
}

fn open_web(ui: &AppWindow, tab: &str) {
    let Some(root) = with_store(|s| s.active().map(|d| d.root.clone())) else {
        return;
    };
    ui.set_web_status(SharedString::from("… archguard serve"));
    let (w, tab) = (ui.as_weak(), tab.to_string());
    std::thread::spawn(move || {
        let msg = match web::open(&root, &tab) {
            Ok(url) => url,
            Err(e) => format!("archguard serve: {e}"),
        };
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = w.upgrade() {
                ui.set_web_status(msg.into());
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A panicking analysis still ends in a result, so the document leaves
    /// "Loading" and shows the panic as its error.
    #[test]
    fn a_panicking_analysis_becomes_the_error() {
        let result: Result<(), String> = guarded(|| panic!("parser blew up"));
        assert_eq!(result, Err("parser blew up".to_string()));
    }

    #[test]
    fn an_analysis_error_stays_the_error() {
        let result: Result<(), String> = guarded(|| Err("no config".into()));
        assert_eq!(result, Err("no config".to_string()));
    }
}
