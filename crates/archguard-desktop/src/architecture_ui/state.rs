//! The Architecture tab's state ([`ArchitectureState`]) and the map of
//! its open chapter.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use archguard_engine::core::{
    Arc42Config, CallAnalysis, Config, DiagramFormat, DocLang, Graph, Report,
};
use archguard_engine::report::Arc42Document;

use super::visual::{self, Kind, Map, MapState, RELATIONS_PAGE};
use super::{catalogue, edit};
use crate::documents::{self, DocId};
use crate::tab_support::Texts;
use crate::ui::AppWindow;
use archguard_engine::report::package_map::{self as projection, Labels, SourceModel};
use slint::ComponentHandle;

/// Everything the tab knows: the document, the source graph and its
/// projections indexes, and the reader's position.
pub struct ArchitectureState {
    pub(super) root: PathBuf,
    pub(super) graph: Graph,
    /// The arc42 document, or why it could not be written.
    pub(super) doc: Result<Arc42Document, String>,
    pub(super) has_model: bool,
    pub(super) model: SourceModel,
    pub(super) deps: Vec<(usize, usize)>,
    pub(super) calls: Vec<(usize, usize)>,
    pub(super) calls_status: String,
    pub(super) chapter: u8,
    /// Per map kind (dependencies, calls, cycles).
    pub(super) maps: [MapState; 3],
    /// Path of the last export, or its error.
    pub(super) saved: String,
    /// The project's own arc42 text and its chapter editor.
    pub(super) editor: edit::Editor,
    /// `[arc42]` of the project: diagram format and hidden catalogue ids.
    pub(super) format: DiagramFormat,
    pub(super) hidden: Vec<String>,
    /// The diagram view is open instead of a chapter.
    pub(super) catalogue: bool,
    /// Why the last tick could not be written.
    pub(super) figure_error: String,
    /// Package map levels in the document (`[arc42] shown_diagrams`).
    pub(super) shown: Vec<String>,
    /// What the document is written from, to write it again after a tick.
    pub(super) config: Config,
    pub(super) report: Report,
    /// The newest rewrite started on this state (0: none); only its
    /// document is kept.
    write: u64,
}

/// Numbers rewrites across all states, so a rebuilt state (0) never takes
/// the document of a rewrite started before.
static WRITES: AtomicU64 = AtomicU64::new(0);

/// Build the tab state (writes the arc42 document, runs the call analysis).
pub(super) fn build_from(
    root: &Path,
    config: &Config,
    graph: Graph,
    report: &Report,
    calls: &CallAnalysis,
    lang: DocLang,
) -> ArchitectureState {
    let _span = archguard_engine::self_trace::tracing::info_span!("build_from").entered();
    // The document is written in the window's language (ADR 0008).
    let config = &Config {
        arc42: Arc42Config {
            lang,
            ..config.arc42.clone()
        },
        ..config.clone()
    };
    let doc =
        archguard_engine::arc42_document(root, config, &graph, report).map_err(|e| e.to_string());
    let model = SourceModel::new(&graph);
    let chapter = match &doc {
        Ok(d) if !d.chapters.iter().any(|c| c.number == 5) => {
            d.chapters.first().map_or(5, |c| c.number)
        }
        _ => 5,
    };
    let mut state = ArchitectureState {
        root: root.to_path_buf(),
        deps: projection::dependency_edges(&model, &graph),
        calls: projection::call_edges(&graph, calls),
        calls_status: calls.status.clone(),
        has_model: config.architecture.model.is_some(),
        graph,
        doc,
        model,
        chapter,
        maps: Default::default(),
        saved: String::new(),
        editor: edit::Editor::new(0, root).with_format(config.arc42.diagram_format),
        format: config.arc42.diagram_format,
        hidden: config.arc42.hidden_diagrams.clone(),
        catalogue: false,
        figure_error: String::new(),
        shown: config.arc42.shown_diagrams.clone(),
        config: config.clone(),
        report: report.clone(),
        write: 0,
    };
    snapshot_overrides(&mut state);
    state
}

/// `ARCHGUARD_DESKTOP_CHAPTER=<n>` and `ARCHGUARD_DESKTOP_ARC_SCOPE=<package>`
/// open a chapter and a building-block scope for snapshot runs.
#[cfg(feature = "snapshot")]
pub(super) fn snapshot_overrides(s: &mut ArchitectureState) {
    if let Some(n) = std::env::var("ARCHGUARD_DESKTOP_CHAPTER")
        .ok()
        .and_then(|n| n.parse().ok())
    {
        s.chapter = n;
    }
    s.catalogue = std::env::var("ARCHGUARD_DESKTOP_ARC_CATALOGUE").is_ok();
    // `ARCHGUARD_DESKTOP_ARC_TOGGLE=<id>:<0|1>@<project root>` ticks a
    // diagram off or on; only in that project, never in others the
    // snapshot run restores.
    if let Ok(spec) = std::env::var("ARCHGUARD_DESKTOP_ARC_TOGGLE")
        && let Some((tick, root)) = spec.rsplit_once('@')
        && let Some((id, on)) = tick.rsplit_once(':')
        && std::path::Path::new(root) == s.root
    {
        toggle_diagram(s, None, id, on == "1");
    }
    if let Ok(name) = std::env::var("ARCHGUARD_DESKTOP_ARC_SCOPE") {
        s.maps[Kind::Dependencies as usize].scope =
            s.graph.nodes.iter().position(|n| n.name == name);
    }
}

#[cfg(not(feature = "snapshot"))]
pub(super) fn snapshot_overrides(_: &mut ArchitectureState) {}

/// The derived visual of a chapter, as `renderArc42Derived`. Chapter 7
/// (deployment) and 4 (target model) are not native yet.
pub(super) fn derived(s: &ArchitectureState, chapter: u8) -> Option<Kind> {
    match chapter {
        5 if !s.graph.nodes.is_empty() => Some(Kind::Dependencies),
        6 if !s.calls.is_empty() => Some(Kind::Calls),
        11 if !s.graph.edges.is_empty() => Some(Kind::Cycles),
        _ => None,
    }
}

pub(super) fn labels(ui: &AppWindow) -> Labels {
    labels_in(&Texts::of(ui))
}

pub(super) fn labels_in(texts: &Texts) -> Labels {
    Labels {
        direct_root: texts.t("Files without a package").into(),
        direct: texts.t("direct files").into(),
        outside: texts.t("Other project packages").into(),
    }
}

/// The map of the open chapter, if it has one.
pub(super) fn current(s: &ArchitectureState, labels: &Labels) -> Option<(Kind, Map)> {
    let kind = derived(s, s.chapter)?;
    let edges = if kind == Kind::Calls {
        &s.calls
    } else {
        &s.deps
    };
    let map = visual::build(
        kind,
        &s.maps[kind as usize],
        &s.model,
        &s.graph,
        edges,
        labels,
    );
    Some((kind, map))
}

/// Show (`show`) or hide catalogue diagram `id`: writes `archguard.toml`,
/// then the document as the CLI would write it.
pub(super) fn toggle_diagram(
    s: &mut ArchitectureState,
    ui: Option<&AppWindow>,
    id: &str,
    show: bool,
) {
    if Arc42Config::level_of(id).is_some() {
        return toggle_level(s, ui, id, show);
    }
    let hidden = catalogue::hidden_after(&s.hidden, id, show);
    s.figure_error = match catalogue::write(&s.root, "hidden_diagrams", &hidden) {
        Ok(()) => String::new(),
        Err(e) => e,
    };
    if s.figure_error.is_empty() {
        if let Ok(doc) = &mut s.doc {
            catalogue::rebase(doc, &hidden, s.format);
        }
        s.config.arc42.hidden_diagrams = hidden.clone();
        s.hidden = hidden;
    }
}

/// A package map level is written by the engine: list it in
/// `shown_diagrams`, then write the document again (off the UI thread).
fn toggle_level(s: &mut ArchitectureState, ui: Option<&AppWindow>, id: &str, show: bool) {
    let shown = catalogue::hidden_after(&s.shown, id, !show);
    if let Err(e) = catalogue::write(&s.root, "shown_diagrams", &shown) {
        s.figure_error = e;
        return;
    }
    s.figure_error = String::new();
    s.shown = shown;
    s.config.arc42.shown_diagrams = s.shown.clone();
    s.config.arc42.hidden_diagrams = s.hidden.clone();
    rewrite(s, ui);
}

/// After a language switch the document is written again in the window's
/// language.
pub(super) fn follow_language(s: &mut ArchitectureState, ui: &AppWindow, lang: DocLang) {
    if s.config.arc42.lang != lang {
        s.config.arc42.lang = lang;
        rewrite(s, Some(ui));
        // Drawn diagrams name their kinds in the window's language.
        if !s.editor.is_dirty() {
            s.editor.redraw();
        }
    }
}

/// Write the document again (off the UI thread unless snapshotting).
fn rewrite(s: &mut ArchitectureState, ui: Option<&AppWindow>) {
    s.write = WRITES.fetch_add(1, Ordering::Relaxed) + 1;
    let (root, config, graph, report) = (
        s.root.clone(),
        s.config.clone(),
        s.graph.clone(),
        s.report.clone(),
    );
    let write = move || {
        archguard_engine::arc42_document(&root, &config, &graph, &report).map_err(|e| e.to_string())
    };
    match ui {
        Some(ui) if !crate::tab_support::snapshot_mode() => {
            write_later(ui, s.editor.doc, s.write, write);
        }
        _ => s.doc = write(),
    }
}

/// Run `write` on a worker thread and give its document to document `doc`
/// ([`deliver`]).
fn write_later(
    ui: &AppWindow,
    doc: DocId,
    number: u64,
    write: impl FnOnce() -> Result<Arc42Document, String> + Send + 'static,
) {
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let result = write();
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            if deliver(doc, number, result) && documents::active_id() == Some(doc) {
                crate::tab_support::update(&ui, |_: &mut ArchitectureState, _| ());
            }
        });
    });
}

/// Give rewrite `number`'s document to document `doc`'s state, active or
/// not, if it is that state's newest rewrite; true if it took it.
fn deliver(doc: DocId, number: u64, result: Result<Arc42Document, String>) -> bool {
    documents::with_state(doc, |st: &mut ArchitectureState| {
        let took = st.write == number;
        if took {
            st.doc = result;
            // A tick since the rewrite started is in `hidden`, not in its document.
            if let Ok(doc) = &mut st.doc {
                catalogue::rebase(doc, &st.hidden, st.format);
            }
        }
        took
    })
    .unwrap_or(false)
}

pub(super) fn open_scope(s: &mut ArchitectureState, kind: Kind, scope: Option<usize>) {
    s.maps[kind as usize] = MapState {
        scope,
        selected: None,
        relations: RELATIONS_PAGE,
    };
}

/// The tab state of the small layered fixture `py-layers`; with `dir`, its
/// analysis but the arc42 text of the project `dir`.
#[cfg(test)]
pub(super) fn fixture_state(dir: Option<&Path>) -> ArchitectureState {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/py-layers")
        .canonicalize()
        .expect("fixture");
    let loaded = archguard_engine::load_config(None, &root).expect("config");
    let run = archguard_engine::analyze_run_quiet(&root, &loaded, archguard_engine::Level::Module)
        .expect("analysis");
    let calls = archguard_engine::analyze_calls(&root, &run.config);
    build_from(
        dir.unwrap_or(&root),
        &run.config,
        run.graph,
        &run.report,
        &calls,
        DocLang::De,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::report::diagram_section;

    /// Two ticks in quick succession whose first write finishes last,
    /// another project tab active meanwhile, then a re-analysis.
    #[test]
    fn only_the_newest_rewrite_lands_in_its_document_active_or_not() {
        let mut state = fixture_state(None);
        let (a, _) = documents::with_store(|s| s.open(&state.root));
        state.editor.doc = a;
        rewrite(&mut state, None);
        let older = state.write;
        rewrite(&mut state, None);
        let newer = state.write;
        let base = state.doc.clone().expect("document");
        let written = |scope: &str| {
            Ok(Arc42Document {
                scope: scope.into(),
                ..base.clone()
            })
        };
        documents::set_state(a, state);
        let other = tempfile::tempdir().expect("temp dir");
        documents::with_store(|s| s.open(other.path()));
        let scope = || {
            documents::with_state(a, |s: &mut ArchitectureState| {
                s.doc.as_ref().map(|d| d.scope.clone()).unwrap_or_default()
            })
        };
        assert!(deliver(a, newer, written("newer")), "not the active tab");
        assert!(!deliver(a, older, written("older")), "older, finished last");
        assert_eq!(scope().as_deref(), Some("newer"));
        // A re-analysis builds the state again.
        let mut rebuilt = fixture_state(None);
        rebuilt.editor.doc = a;
        documents::set_state(a, rebuilt);
        assert!(!deliver(a, newer, written("stale")), "started before it");
        assert_ne!(scope().as_deref(), Some("stale"));
    }

    /// What a map level shows is what its tick writes into the document:
    /// the same boxes with the same labels, overview and package level.
    #[test]
    fn a_ticked_map_level_writes_the_boxes_the_map_shows() {
        let dir = tempfile::tempdir().expect("temp dir");
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/py-layers");
        let root = dir.path();
        // py-layers: archguard.toml and src/<package>/<file>.py.
        std::fs::copy(fixture.join("archguard.toml"), root.join("archguard.toml")).expect("copy");
        for package in std::fs::read_dir(fixture.join("src"))
            .expect("src")
            .flatten()
        {
            let target = root.join("src").join(package.file_name());
            std::fs::create_dir_all(&target).expect("package dir");
            for file in std::fs::read_dir(package.path())
                .expect("package")
                .flatten()
            {
                std::fs::copy(file.path(), target.join(file.file_name())).expect("copy");
            }
        }
        let loaded = archguard_engine::load_config(None, root).expect("config");
        let run =
            archguard_engine::analyze_run_quiet(root, &loaded, archguard_engine::Level::Module)
                .expect("analysis");
        let calls = archguard_engine::analyze_calls(root, &run.config);
        let mut config = run.config.clone();
        config.arc42.shown_diagrams = vec!["building-blocks@app".into()];
        let mut s = build_from(root, &config, run.graph, &run.report, &calls, DocLang::De);
        let labels = Labels::of(DocLang::De);
        let app = s.graph.nodes.iter().position(|n| n.name == "app");
        assert!(app.is_some(), "the fixture has a package app");
        for (scope, id) in [(None, "building-blocks"), (app, "building-blocks@app")] {
            s.maps[Kind::Dependencies as usize].scope = scope;
            let (_, map) = current(&s, &labels).expect("chapter 5 map");
            let shown: Vec<&str> = map.shown.nodes.iter().map(|n| n.label.as_str()).collect();
            let doc = s.doc.as_ref().expect("document");
            let diagram = doc.diagrams.iter().find(|d| d.id == id).expect(id);
            let written: Vec<&str> = diagram.elements.iter().map(|e| e.name.as_str()).collect();
            assert_eq!(written, shown, "{id}");
            let mut written: Vec<(String, String, usize)> = diagram
                .relations
                .iter()
                .map(|r| (r.from.clone(), r.to.clone(), r.count))
                .collect();
            let mut shown: Vec<(String, String, usize)> = map
                .shown
                .edges
                .iter()
                .map(|e| (format!("e{}", e.from), format!("e{}", e.to), e.count))
                .collect();
            written.sort();
            shown.sort();
            assert_eq!(written, shown, "{id}");
        }
    }

    /// A rewrite (language switch) starts, a diagram is ticked off, then
    /// the rewrite's document, written before the tick, arrives.
    #[test]
    fn a_rewrite_finished_after_a_tick_keeps_the_tick() {
        let mut state = fixture_state(None);
        rewrite(&mut state, None);
        let number = state.write;
        let before = state.doc.clone().expect("document");
        let diagram = before
            .diagrams
            .iter()
            .find(|d| d.shown && Arc42Config::level_of(&d.id).is_none())
            .expect("a shown catalogue diagram")
            .clone();
        // The tick writes archguard.toml: not into the fixture.
        let dir = tempfile::tempdir().expect("temp dir");
        state.root = dir.path().to_path_buf();
        let (a, _) = documents::with_store(|s| s.open(dir.path()));
        state.editor.doc = a;
        toggle_diagram(&mut state, None, &diagram.id, false);
        assert_eq!(state.figure_error, "");
        documents::set_state(a, state);
        assert!(deliver(a, number, Ok(before)), "newest rewrite");
        documents::with_state(a, |s: &mut ArchitectureState| {
            let doc = s.doc.as_ref().expect("document");
            let d = doc.diagrams.iter().find(|d| d.id == diagram.id).unwrap();
            assert!(!d.shown, "{} shown again", d.id);
            let section = diagram_section(&diagram, s.format);
            assert!(doc.chapters.iter().all(|c| !c.markdown.contains(&section)));
        })
        .expect("state");
    }
}
