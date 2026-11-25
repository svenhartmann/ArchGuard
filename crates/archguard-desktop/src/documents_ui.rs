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
use std::sync::atomic::{AtomicU64, Ordering};

use archguard_engine::{Level, analyze_run_quiet, collect_review_sources, load_config};
use slint::{ComponentHandle, SharedString};

use crate::context::{Analysis, Loaded};
use crate::documents::{self, DocId, Status, Store, SubProject, with_store};
use crate::documents_set::{self, Target, target};
use crate::graph_ui;
use crate::panes::Layout;
use crate::panes_ui;
use crate::settings::{self, ProjectView, Session};
use crate::settings_file;
use crate::tab_support::{self, Shared, Texts, Topology, model as vec_model};
use crate::ui::{AppWindow, DocItem, RecentItem, SubProjectItem};
use crate::{
    architecture_ui, calls_ui, file_ui, instances, matrix_ui, quality_ui, reanalyze_ui, runtime_ui,
    security_ui, services_ui, start_ui,
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
    crate::access_ui::wire(ui);
    crate::screens_ui::wire(ui);
    crate::domain_ui::wire(ui);
    crate::es_ui::wire(ui);
    quality_ui::wire(ui);
    security_ui::wire(ui);
    crate::export_ui::wire(ui);
    crate::follow_ui::wire(ui);
    panes_ui::wire(ui);
    let again = with_ui(ui, |ui, ()| {
        if let Some((id, root)) = reanalyze_ui::admit(ui) {
            spawn_analysis(ui, id, root);
        }
    });
    ui.on_analyze_again(move || again(()));
    reanalyze_ui::watch(ui);
    ui.on_select_doc(with_ui(ui, |ui, i: i32| activate(ui, index(i))));
    ui.on_open_sub_project(with_ui(ui, |ui, i: i32| open_sub_project(ui, index(i))));
    let add = with_ui(ui, |ui, ()| pick_folder(ui));
    ui.on_add_doc(move || add(()));
    let strip = with_ui(ui, |ui, ()| push_strip(ui));
    documents::on_dirty_changed(move || strip(()));
    // The window opens without waiting for rust-analyzer. Once its run for
    // a root is done, that root's documents are analysed again; the views
    // keep showing the first analysis meanwhile.
    let weak = ui.as_weak();
    archguard_engine::index_rust_in_background(move |root| {
        let (weak, root) = (weak.clone(), root.to_path_buf());
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else {
                return;
            };
            let docs = with_store(|s| {
                s.docs()
                    .iter()
                    .map(|d| (d.id, d.root.clone()))
                    .collect::<Vec<_>>()
            });
            for (id, _) in docs.into_iter().filter(|(_, r)| *r == root) {
                spawn_analysis(&ui, id, root.clone());
            }
        });
    });
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

/// Open a project folder, or a project set file, as the active document;
/// analyse it if it is new. A set file that is not valid is a notice.
pub fn open(ui: &AppWindow, path: PathBuf) {
    crate::settings_view_ui::flush_typing();
    let target = match target(&path) {
        Ok(t) => t,
        Err(e) => {
            ui.set_notice_kind(2);
            ui.set_notice(e.into());
            return;
        }
    };
    save_view(ui);
    let (id, new, root, entry) = with_store(|s| match target {
        Target::Folder(root) => {
            let (id, new) = s.open(&root);
            (id, new, root.clone(), root)
        }
        Target::Set {
            file,
            primary,
            name,
        } => {
            let (id, new) = s.open_set(&file, &primary, &name);
            (id, new, primary, file)
        }
    });
    if new {
        seed_view(id, &entry);
        spawn_analysis(ui, id, root);
    }
    settings_file::edit(|s| s.add_recent(&entry));
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

/// The open documents as the session; a set is its file.
fn session_of(s: &Store) -> Session {
    Session {
        documents: s.docs().iter().map(|d| d.entry().to_path_buf()).collect(),
        active: s.active_index().unwrap_or(0),
    }
}

/// Store the open documents as the session.
fn record_session() {
    let session = with_store(|s| session_of(s));
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
    // Settings text typed for the document shown so far goes to it.
    crate::settings_view_ui::flush_typing();
    save_view(ui);
    if with_store(|s| s.activate(index)) {
        record_session();
        refresh(ui);
    }
}

/// Close the document at tab `index`, or every document (`None`), without
/// asking; `shell_ui` asks about unsaved changes first. The window shows
/// the next document once, not each one closed on the way.
pub fn close(ui: &AppWindow, index: Option<usize>) {
    save_view(ui);
    let docs: Vec<_> = with_store(|s| match index {
        Some(i) => s.close(i).into_iter().collect(),
        None => std::iter::from_fn(|| s.close(0)).collect(),
    });
    if docs.is_empty() {
        return;
    }
    // Drop the documents' state before the window shows the next one.
    drop(docs);
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
    set: Option<&Path>,
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
    let projects = match (&*tab_support::topology(topology, root, set), set) {
        (Ok(_), Some(set)) => archguard_engine::read_project_set(set)
            .map(|s| documents_set::members(&s.projects))
            .unwrap_or_default(),
        (Ok(t), None) => documents::build_units(t),
        (Err(_), _) => Vec::new(),
    };
    let analysis = Analysis {
        root: root.to_path_buf(),
        run,
        sources,
    };
    Ok((analysis, projects))
}

/// Numbers analysis runs across all documents.
static RUNS: AtomicU64 = AtomicU64::new(0);

/// The newest analysis run started for a document; only its result is
/// kept (a run started before the Rust index was done may finish last).
struct Run(u64);

impl Run {
    /// Number a new analysis run of document `id`; it is now the newest.
    fn start(id: DocId) -> u64 {
        let run = RUNS.fetch_add(1, Ordering::Relaxed) + 1;
        documents::set_state(id, Run(run));
        run
    }

    /// Give analysis `run`'s result to document `id`; whether `id` is the
    /// active document, `None` if the result was dropped.
    fn land(
        id: DocId,
        run: u64,
        result: Result<(Analysis, Vec<SubProject>), String>,
    ) -> Option<bool> {
        if documents::with_state(id, |r: &mut Run| r.0) != Some(run) {
            return None;
        }
        Some(with_store(|s| {
            let Some(doc) = s.get_mut(id) else {
                return false;
            };
            match result {
                Ok((analysis, projects)) => {
                    if doc.analysis.is_some() {
                        doc.forget_derived();
                    }
                    doc.analysis = Some(Arc::new(analysis));
                    doc.projects = projects;
                    doc.status = Status::Ready;
                }
                Err(e) => doc.status = Status::Failed(e),
            }
            s.active().is_some_and(|d| d.id == id)
        }))
    }
}

/// Analyse document `id` on a worker; the newest run's result lands.
fn spawn_analysis(ui: &AppWindow, id: DocId, root: PathBuf) {
    let run = Run::start(id);
    let topology = Shared::<Topology>::new();
    documents::set_state(id, topology.clone());
    let weak = ui.as_weak();
    let name = "desktop: analyze project";
    let at = root.clone();
    let set = documents_set::active_set_of(id);
    let work = tab_support::traced(&at, name, move || analyze(&root, set.as_deref(), &topology));
    std::thread::spawn(move || {
        // Taken first, so edits during the analysis count as changes.
        let baseline = reanalyze_ui::fingerprint(&at);
        let result = guarded(work);
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = weak.upgrade() {
                finished(&ui, id, run, (result, baseline));
            }
        });
    });
}

/// Run a document's analysis on its worker thread. A panic becomes the
/// document's error, so it leaves "Loading" either way.
fn guarded<T>(analyze: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    tab_support::catch(analyze).and_then(|r| r)
}

type Landing = (Result<(Analysis, Vec<SubProject>), String>, Option<String>);

fn finished(ui: &AppWindow, id: DocId, run: u64, (result, baseline): Landing) {
    let Some(active) = Run::land(id, run, result) else {
        return;
    };
    reanalyze_ui::landed(ui, id, baseline);
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
    let Some(root) = with_store(|s| s.active().map(|d| d.entry().to_path_buf())) else {
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
    crate::access_ui::show(ui, data);
    crate::screens_ui::show(ui, data);
    crate::domain_ui::show(ui, data);
    crate::es_ui::show(ui, data);
    quality_ui::show(ui, data);
    security_ui::show(ui, data);
    // Graph, Matrix and File tabs: their instances for the layout.
    panes_ui::sync(ui);
    graph_ui::show(ui);
    ui.invoke_fit_views();
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

    /// The Rust index starts a second analysis while the first runs; the
    /// first, without rust-analyzer data, finishes last.
    #[test]
    fn only_the_newest_analysis_lands_in_its_document() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (id, _) = with_store(|s| s.open(dir.path()));
        let first = Run::start(id);
        let second = Run::start(id);
        assert_eq!(Run::land(id, second, Err("second".into())), Some(true));
        assert_eq!(Run::land(id, first, Err("first".into())), None, "older run");
        let status = with_store(|s| s.get_mut(id).map(|d| d.status.clone()));
        assert_eq!(status, Some(Status::Failed("second".into())));
    }

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/es-set")
            .join(name)
            .canonicalize()
            .expect("fixture")
    }

    #[test]
    fn a_set_file_is_a_set_and_a_folder_a_folder() {
        let (set, infra) = (fixture("search.archguard-set.toml"), fixture("infra"));
        assert_eq!(target(&infra), Ok(Target::Folder(infra.clone())));
        assert_eq!(
            target(&set),
            Ok(Target::Set {
                file: set,
                primary: infra,
                name: "search".into()
            })
        );
    }

    #[test]
    fn a_broken_set_file_is_an_error_text_not_a_crash() {
        let dir = tempfile::tempdir().expect("temp dir");
        let bad = dir.path().join("bad.archguard-set.toml");
        std::fs::write(&bad, "projects = 3").expect("write");
        let Err(text) = target(&bad) else {
            panic!("expected an error");
        };
        assert!(!text.is_empty());
        assert!(target(&dir.path().join("missing.archguard-set.toml")).is_err());
    }

    #[test]
    fn the_analysis_of_a_set_lists_its_members_as_projects() {
        let (set, infra) = (fixture("search.archguard-set.toml"), fixture("infra"));
        let (_, projects) = analyze(&infra, Some(&set), &Shared::new()).expect("analysis");
        let names: Vec<&str> = projects.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["infra", "adapter"]);
        assert_eq!(
            documents::project_root(&infra, &projects[1]),
            fixture("adapter")
        );
    }

    #[test]
    fn the_session_and_recent_list_keep_the_set_file() {
        let (set, infra) = (fixture("search.archguard-set.toml"), fixture("infra"));
        let mut s = documents::Store::default();
        s.open_set(&set, &infra, "search");
        s.open(&infra);
        let session = session_of(&s);
        assert_eq!(session.documents, [set, infra]);
        assert_eq!(session.active, 1);
    }

    #[test]
    fn an_analysis_error_stays_the_error() {
        let result: Result<(), String> = guarded(|| Err("no config".into()));
        assert_eq!(result, Err("no config".to_string()));
    }

    /// Guard: "Analyze again" (a second landed analysis) drops the shared flow, so Screens, Domain
    /// and Access recompute it from the changed sources; there is no separate staleness check.
    #[test]
    fn a_new_analysis_drops_the_shared_flow() {
        let dir = tempfile::tempdir().expect("temp dir");
        let (id, _) = with_store(|s| s.open(dir.path()));
        let runs = std::cell::Cell::new(0);
        let flow = || {
            let slot = Shared::<Arc<Result<archguard_engine::core::UiFlow, String>>>::active();
            slot.get_or_init(|| {
                runs.set(runs.get() + 1);
                Arc::new(Err("flow".into()))
            });
        };
        for _ in 0..2 {
            let run = Run::start(id);
            let analysis = analyze(dir.path(), None, &Shared::new()).expect("analysis");
            assert_eq!(Run::land(id, run, Ok(analysis)), Some(true));
            flow();
            flow();
        }
        assert_eq!(runs.get(), 2, "once per analysis");
    }
}
