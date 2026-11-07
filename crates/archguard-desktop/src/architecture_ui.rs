//! Architecture tab: the web viewer's arc42 reader (renderArc42Documentation
//! and the derived package maps in viewer_architecture.js). The document
//! comes from `archguard_engine::arc42_document`, as in the CLI's report.
//!
//! All tab state is one [`ArchitectureState`] per document (in
//! [`documents`]), built by [`state::build_from`] and shown by [`push`]
//! as one `ArchitectureModel` value.

mod blocks;
mod catalogue;
mod diagram;
mod edit;
mod figure;
mod follow;
mod markdown;
mod names;
mod projection;
mod state;
mod view;
mod visual;

use std::path::{Path, PathBuf};

use slint::{ComponentHandle, SharedString};

use crate::context::Loaded;
use crate::format::Locale;
use crate::kpi;
use crate::tab_support::{self, Actions, Calls, TabState, Texts, index};
use crate::ui::{AppWindow, ArchitectureData, ArchitectureModel};
use archguard_engine::report::Arc42Document;
use follow::navigation;
use projection::Group;
pub use state::ArchitectureState;
use state::{build_from, current, derived, labels, open_scope, toggle_diagram};
use visual::{Action, Kind, RELATIONS_PAGE};

/// Register the tab's callbacks once at start-up.
pub fn wire(ui: &AppWindow) {
    let d = ui.global::<ArchitectureData>();
    let a = Actions::<ArchitectureState>::new(ui);
    d.on_select_chapter(a.on(|s, _, n: i32| {
        if !s.editor.is_open() {
            s.chapter = u8::try_from(n).unwrap_or(5);
            s.catalogue = false;
        }
    }));
    d.on_open_catalogue(a.on0(|s, _| s.catalogue = !s.editor.is_open()));
    let toggle = a.on(|s, _, (id, show): (SharedString, bool)| toggle_diagram(s, &id, show));
    d.on_toggle_diagram(move |id, show| toggle((id, show)));
    d.on_start_edit(a.on0(|s, ui| {
        if let Ok(doc) = &s.doc {
            s.editor.start(ui, doc, s.chapter);
        }
    }));
    d.on_draft_changed(tab_support::with_ui(ui, |ui, text: SharedString| {
        crate::documents::with_active_state(|s: &mut ArchitectureState| {
            s.editor.preview(ui, &text);
        });
    }));
    d.on_save_edit(a.on0(|s, ui| {
        if let Ok(doc) = &s.doc {
            s.saved = s
                .editor
                .save(ui, doc)
                .unwrap_or(std::mem::take(&mut s.saved));
        }
    }));
    d.on_cancel_edit(a.on0(|s, _| s.editor.cancel()));
    d.on_choose_location(a.on(|s, ui, choice: i32| {
        if let Ok(doc) = &s.doc {
            s.saved = s
                .editor
                .choose(ui, doc, choice)
                .unwrap_or(std::mem::take(&mut s.saved));
        }
    }));
    d.on_navigate(navigation(
        ui,
        a.on(|s, _, scope: i32| {
            open_scope(s, Kind::Dependencies, index(scope));
        }),
    ));
    d.on_scope_selected(navigation(
        ui,
        a.on(|s, ui, i: i32| {
            let Some((kind, map)) = current(s, &labels(ui)) else {
                return;
            };
            let scope = index(i).and_then(|i| map.scopes.get(i).copied().flatten());
            open_scope(s, kind, scope);
        }),
    ));
    d.on_select_card(navigation(ui, a.on(select_card)));
    d.on_more_relations(a.on0(|s, _| {
        if let Some(kind) = derived(s, s.chapter) {
            let state = &mut s.maps[kind as usize];
            state.relations = state.relations.max(RELATIONS_PAGE) + RELATIONS_PAGE;
        }
    }));
    let action = navigation(ui, a.on(|s, ui, ()| card_action(s, ui)));
    d.on_card_action(move || action(()));
    d.on_download(a.on(|s, _, all: bool| download(s, all)));
    d.on_plantuml(a.on(|s, ui, unit: SharedString| plantuml(s, ui, &unit)));
    follow::wire();
}

impl TabState for ArchitectureState {
    fn push(&mut self, ui: &AppWindow) {
        push(ui, self);
    }

    fn arrive(&mut self, ui: &AppWindow) {
        if let Ok(doc) = &self.doc
            && let Some(path) = self.editor.snapshot(ui, doc, self.chapter)
        {
            self.saved = path;
        }
    }

    fn pending(ui: &AppWindow, error: Option<&str>) {
        let mut m = ArchitectureModel::default();
        if let Some(e) = error {
            m.state = 2;
            m.error = e.into();
        }
        ui.global::<ArchitectureData>().set_model(m);
    }
}

/// Show the active document's tab. Its state is built once per document,
/// on a worker thread.
pub fn show(ui: &AppWindow, data: &Loaded) {
    tab_support::show_async(ui, || {
        let calls = Calls::of(data);
        let (root, config, graph, report) = (
            data.root.to_path_buf(),
            data.run.config.clone(),
            data.run.graph.clone(),
            data.run.report.clone(),
        );
        move || build_from(&root, &config, graph, &report, &calls.get())
    });
}

fn select_card(s: &mut ArchitectureState, ui: &AppWindow, i: i32) {
    let Some((kind, map)) = current(s, &labels(ui)) else {
        return;
    };
    let Some(node) = index(i).and_then(|i| map.shown.nodes.get(i)) else {
        return;
    };
    let state = &mut s.maps[kind as usize];
    state.scope = map.scope;
    state.selected = (state.selected != Some(node.group)).then_some(node.group);
    state.relations = RELATIONS_PAGE;
}

fn card_action(s: &mut ArchitectureState, ui: &AppWindow) {
    let Some((kind, map)) = current(s, &labels(ui)) else {
        return;
    };
    match map.action {
        Action::None => {}
        Action::OpenLevel(n) | Action::OpenPackage(n) => open_scope(s, kind, Some(n)),
        // Graph-tab nodes are the graph's nodes.
        Action::OpenInGraph(n) => crate::panes_ui::reveal(ui, n),
    }
}

/// Show the state in the window.
pub fn push(ui: &AppWindow, s: &ArchitectureState) {
    let d = ui.global::<ArchitectureData>();
    let mut m = ArchitectureModel {
        has_model: s.has_model,
        saved: s.saved.as_str().into(),
        ..Default::default()
    };
    m.map.kind = -1;
    match &s.doc {
        Err(e) => {
            m.state = 2;
            m.error = e.as_str().into();
        }
        Ok(doc) if doc.chapters.is_empty() => m.state = 2,
        Ok(doc) => {
            m.state = 1;
            let drawn: Vec<_> = doc
                .diagrams
                .iter()
                .filter(|d| d.chapter == s.chapter)
                .cloned()
                .collect();
            let text = catalogue::strip(&s.editor.chapter(doc, s.chapter), &drawn, s.format);
            view::fill_document(
                &mut m,
                doc,
                view::Sources {
                    chapter: &text,
                    whole: &s.editor.preamble(doc),
                },
                s.chapter,
                Texts::of(ui)
                    .t("Diagram source · this diagram syntax is not supported in the viewer."),
                derived(s, s.chapter).is_some(),
            );
            let texts = Texts::of(ui);
            (m.language_note, m.chapter_note) = notes(&texts, m.chapter);
            if let Some((kind, map)) = current(s, &labels(ui)) {
                m.map = view::map_model(
                    &Texts::of(ui),
                    &s.graph,
                    &s.calls_status,
                    s.maps[kind as usize].relations,
                    kind,
                    &map,
                );
            }
            fill_figures(&mut m, s, doc, &texts);
        }
    }
    m.figure_error = s.figure_error.as_str().into();
    s.editor.fill(&mut m);
    d.set_model(m);
}

/// The catalogue diagrams: the open chapter's (its package map gets only
/// a tick), or all of them in the diagram view.
fn fill_figures(
    m: &mut ArchitectureModel,
    s: &ArchitectureState,
    doc: &Arc42Document,
    texts: &Texts,
) {
    let map = derived(s, s.chapter).map(catalogue::map_id);
    if s.catalogue {
        m.catalogue = true;
        m.chapter_title = texts.t("Diagrams").into();
        m.map.kind = -1;
        m.blocks = Default::default();
        m.source = Default::default();
    } else if let Some(d) = doc.diagrams.iter().find(|d| Some(d.id.as_str()) == map) {
        m.map_id = d.id.as_str().into();
        m.map_title = d.title.as_str().into();
        m.map_shown = d.shown;
    }
    let figures = doc
        .diagrams
        .iter()
        .filter(|d| s.catalogue || (d.chapter == s.chapter && Some(d.id.as_str()) != map))
        .map(|d| {
            let chapter = if s.catalogue {
                format!("{} {}", texts.t("Chapter"), d.chapter)
            } else {
                String::new()
            };
            figure::figure(d, s.format, chapter, texts)
        });
    m.figures = slint::ModelRc::new(slint::VecModel::from(figures.collect::<Vec<_>>()));
}

/// What the desktop says around the engine's German arc42 text: its
/// language (in an English window) and, in chapter 10, the level its
/// figures are counted on.
fn notes(texts: &Texts, chapter: i32) -> (SharedString, SharedString) {
    let language = if texts.locale() == Locale::En {
        texts.t("Document language: German. The analysis engine writes the arc42 text in German.")
    } else {
        ""
    };
    let level = if chapter == 10 {
        texts.f(
            "{acd} and the cycle figures of this chapter are counted on modules (depth 2). {shown} is in the Graph tab; package cycles are in the chapter 5 diagram.",
            &[
                ("acd", texts.t(kpi::MODULE_ACD.label).to_string()),
                ("shown", texts.t(kpi::SHOWN_ACD.label).to_string()),
            ],
        )
    } else {
        String::new()
    };
    (language.into(), level.into())
}

fn download(s: &mut ArchitectureState, all: bool) {
    let Ok(doc) = &s.doc else { return };
    let (name, content) = if all {
        ("arc42.md".to_string(), doc.markdown(None))
    } else {
        (
            format!("arc42-{:02}.md", s.chapter),
            doc.markdown(Some(s.chapter)),
        )
    };
    s.saved = save(&s.root, &name, &content);
}

fn plantuml(s: &mut ArchitectureState, ui: &AppWindow, unit: &str) {
    let Some((kind, map)) = current(s, &labels(ui)) else {
        return;
    };
    let nodes: Vec<diagram::UmlNode<'_>> = map
        .shown
        .nodes
        .iter()
        .map(|n| diagram::UmlNode {
            name: &n.name,
            shape: match n.group {
                Group::Package(_) => diagram::Shape::Package,
                _ => diagram::Shape::Rectangle,
            },
            cycle: n.cycle,
        })
        .collect();
    let edges: Vec<diagram::UmlEdge> = map
        .shown
        .edges
        .iter()
        .map(|e| diagram::UmlEdge {
            from: e.from,
            to: e.to,
            count: e.count,
            cycle: e.cycle,
        })
        .collect();
    let name = if kind == Kind::Calls {
        "arc42-calls.puml"
    } else {
        "arc42-dependencies.puml"
    };
    s.saved = save(&s.root, name, &diagram::plantuml(&nodes, &edges, unit));
}

/// Write an export to `~/Downloads` (else the project root), as the web's
/// download; returns the path, or the path and the error.
fn save(root: &Path, name: &str, content: &str) -> String {
    let downloads = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Downloads"));
    let dir = downloads
        .filter(|d| d.is_dir())
        .unwrap_or_else(|| root.to_path_buf());
    let path = dir.join(name);
    match std::fs::write(&path, content) {
        Ok(()) => path.display().to_string(),
        Err(e) => format!("{}: {e}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The engine's arc42 is German: an English window says so, and
    /// chapter 10 names the level of its ACD.
    #[test]
    fn arc42_chrome_names_language_and_level() {
        let en = Texts::default();
        let (language, level) = notes(&en, 10);
        assert!(language.starts_with("Document language: German"));
        assert!(
            level.starts_with("ACD (module level) and the cycle figures"),
            "{level}"
        );
        assert!(level.contains("ACD (shown level) is in the Graph tab"));
        assert_eq!(notes(&en, 5).1, "");
        let de = Texts::german();
        let (language, level) = notes(&de, 10);
        assert_eq!(language, "", "a German window needs no note");
        assert!(level.starts_with("ACD (Modulebene)"), "{level}");
    }

    /// The tab state for a small layered fixture: twelve chapters, the
    /// package map on chapter 5 and the cycle map on chapter 11, as the web
    /// viewer.
    #[test]
    fn build_writes_the_document_and_derives_maps() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/py-layers")
            .canonicalize()
            .expect("fixture");
        let loaded = archguard_engine::load_config(None, &root).expect("config");
        let run =
            archguard_engine::analyze_run_quiet(&root, &loaded, archguard_engine::Level::Module)
                .expect("analysis");
        let calls = archguard_engine::analyze_calls(&root, &run.config);
        let state = build_from(&root, &run.config, run.graph.clone(), &run.report, &calls);
        let doc = state.doc.as_ref().expect("arc42 document");
        assert_eq!(doc.chapters.len(), 12);
        assert_eq!(state.chapter, 5);
        assert_eq!(derived(&state, 5), Some(Kind::Dependencies));
        assert_eq!(derived(&state, 11), Some(Kind::Cycles));
        assert_eq!(derived(&state, 1), None);
        assert_eq!(state.deps.len(), run.graph.edges.len());
    }
}
