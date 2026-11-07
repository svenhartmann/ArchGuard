//! The Architecture tab's state ([`ArchitectureState`]) and the map of
//! its open chapter.

use std::path::{Path, PathBuf};

use archguard_engine::core::{CallAnalysis, Config, DiagramFormat, Graph, Report};
use archguard_engine::report::Arc42Document;

use super::projection::{self, Labels, SourceModel};
use super::visual::{self, Kind, Map, MapState, RELATIONS_PAGE};
use super::{catalogue, edit};
use crate::tab_support::Texts;
use crate::ui::AppWindow;

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
}

/// Build the tab state (writes the arc42 document, runs the call analysis).
pub(super) fn build_from(
    root: &Path,
    config: &Config,
    graph: Graph,
    report: &Report,
    calls: &CallAnalysis,
) -> ArchitectureState {
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
        editor: edit::Editor::new(root),
        format: config.arc42.diagram_format,
        hidden: config.arc42.hidden_diagrams.clone(),
        catalogue: false,
        figure_error: String::new(),
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
    // `ARCHGUARD_DESKTOP_ARC_TOGGLE=<id>:<0|1>` ticks a diagram off or on.
    if let Ok(spec) = std::env::var("ARCHGUARD_DESKTOP_ARC_TOGGLE")
        && let Some((id, on)) = spec.rsplit_once(':')
    {
        toggle_diagram(s, id, on == "1");
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
    let texts = Texts::of(ui);
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
pub(super) fn toggle_diagram(s: &mut ArchitectureState, id: &str, show: bool) {
    let hidden = catalogue::hidden_after(&s.hidden, id, show);
    s.figure_error = match catalogue::write(&s.root, &hidden) {
        Ok(()) => String::new(),
        Err(e) => e,
    };
    if s.figure_error.is_empty() {
        if let Ok(doc) = &mut s.doc {
            catalogue::rebase(doc, &hidden, s.format);
        }
        s.hidden = hidden;
    }
}

pub(super) fn open_scope(s: &mut ArchitectureState, kind: Kind, scope: Option<usize>) {
    s.maps[kind as usize] = MapState {
        scope,
        selected: None,
        relations: RELATIONS_PAGE,
    };
}
