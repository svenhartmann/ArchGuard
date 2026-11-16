//! Architecture tab: the web viewer's arc42 reader (renderArc42Documentation
//! and the derived package maps in viewer_architecture.js). The document
//! comes from `archguard_engine::arc42_document`, as in the CLI's report.
//!
//! All tab state is one [`ArchitectureState`] per document (in
//! [`documents`]), built by [`state::build_from`] and shown by [`push`]
//! as one `ArchitectureModel` value.

mod blocks;
mod catalogue;
#[cfg(test)]
mod check;
mod diagram;
mod edit;
mod figure;
mod follow;
mod markdown;
mod projection;
mod rich;
pub mod sequence;
mod state;
mod sync;
mod view;
mod visual;
mod window;

use std::path::Path;

use slint::{ComponentHandle, SharedString};

use crate::context::Loaded;
use crate::documents;
use crate::tab_support::{self, Actions, Calls, TabState, Texts, index};
use crate::ui::{AppWindow, ArchitectureData, ArchitectureModel, RichData};
use follow::navigation;
use projection::Group;
use rich::view::{Key, Rich};
pub use state::ArchitectureState;
use state::{build_from, current, derived, follow_language, labels, open_scope, toggle_diagram};
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
    let toggle =
        a.on(|s, ui, (id, show): (SharedString, bool)| toggle_diagram(s, Some(ui), &id, show));
    d.on_toggle_diagram(move |id, show| toggle((id, show)));
    d.on_start_edit(a.on0(|s, _| start_edit(s)));
    d.on_draft_changed(tab_support::with_ui(ui, |ui, text: SharedString| {
        crate::documents::with_active_state(|s: &mut ArchitectureState| {
            s.editor.edit(&text);
            s.editor.show(ui);
        });
    }));
    d.on_visual_changed(tab_support::with_ui(ui, |ui, _: bool| {
        crate::documents::with_active_state(|s: &mut ArchitectureState| s.editor.show(ui));
    }));
    wire_rich(ui);
    // Errors show in the editor itself.
    d.on_save_edit(a.on0(|s, _| drop(save_draft(s))));
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
    d.on_download(a.on(|s, ui, all: bool| download(s, ui, all)));
    d.on_plantuml(a.on(|s, ui, unit: SharedString| plantuml(s, ui, &unit)));
    follow::wire();
}

impl TabState for ArchitectureState {
    fn push(&mut self, ui: &AppWindow) {
        follow_language(self, ui, Texts::of(ui).doc_lang());
        push(ui, self);
    }

    fn arrive(&mut self, _: &AppWindow) {
        if let Ok(doc) = &self.doc
            && let Some(path) = self.editor.snapshot(doc, self.chapter)
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
    let lang = Texts::of(ui).doc_lang();
    tab_support::show_async(ui, || {
        let id = crate::documents::active_id().unwrap_or_default();
        let calls = Calls::of(data);
        let (root, config, graph, report) = (
            data.root.to_path_buf(),
            data.run.config.clone(),
            data.run.graph.clone(),
            data.run.report.clone(),
        );
        move || {
            let mut state = build_from(&root, &config, graph, &report, &calls.get(), lang);
            state.editor.doc = id;
            state
        }
    });
}

/// Open the chapter editor. Save in the close and quit prompt saves its
/// draft as the Save button does, on the document's tab state as it is then.
fn start_edit(s: &mut ArchitectureState) {
    let Ok(doc) = &s.doc else { return };
    s.editor.start(doc, s.chapter);
    let id = s.editor.doc;
    documents::set_save_hook(id, edit::PART, move || {
        documents::with_state(id, save_draft)
            .unwrap_or_else(|| Err("The Architecture tab is still loading; save again.".into()))
    });
}

/// The Save button: write the open draft and remember the path. `Err`
/// keeps the editor open (its error, or the question where to save).
fn save_draft(s: &mut ArchitectureState) -> Result<(), String> {
    if !s.editor.is_open() {
        return Ok(());
    }
    let doc = s.doc.as_ref().map_err(Clone::clone)?;
    match s.editor.save(doc) {
        Some(path) => {
            s.saved = path;
            Ok(())
        }
        None => Err(s.editor.unsaved_reason()),
    }
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

/// The formatted editor's callbacks: each edits the active document's
/// draft and shows only the editor again.
fn wire_rich(ui: &AppWindow) {
    let r = ui.global::<RichData>();
    // Run `f` on the open editor and show it; `None` without one.
    fn edit<R>(ui: &AppWindow, f: impl FnOnce(&mut Rich) -> R) -> Option<R> {
        crate::documents::with_active_state(|s: &mut ArchitectureState| {
            let out = s.editor.rich(f);
            s.editor.show(ui);
            out
        })
        .flatten()
    }
    let weak = ui.as_weak();
    r.on_key(move |name, shift, alt, control| {
        let Some(ui) = weak.upgrade() else {
            return false;
        };
        match edit(&ui, |r| r.key(&name, shift, alt, control)) {
            Some(Key::Done) => true,
            Some(Key::Link) => {
                ui.global::<RichData>().set_linking(true);
                true
            }
            Some(Key::Ignored) | None => false,
        }
    });
    let weak = ui.as_weak();
    r.on_type(move |text| {
        if let Some(ui) = weak.upgrade() {
            edit(&ui, |r| r.type_text(&text));
        }
    });
    let weak = ui.as_weak();
    r.on_press(move |piece, block, x, y, shift| {
        if let Some(ui) = weak.upgrade() {
            edit(&ui, |r| {
                r.press(piece as usize, block as usize, x, y, shift)
            });
        }
    });
    let weak = ui.as_weak();
    r.on_drag(move |piece, block, x, y| {
        if let Some(ui) = weak.upgrade() {
            edit(&ui, |r| r.drag(piece as usize, block as usize, x, y));
        }
    });
    let weak = ui.as_weak();
    r.on_word(move |piece, block, x, y| {
        if let Some(ui) = weak.upgrade() {
            edit(&ui, |r| r.word(piece as usize, block as usize, x, y));
        }
    });
    let weak = ui.as_weak();
    r.on_width(move |w| {
        if let Some(ui) = weak.upgrade() {
            edit(&ui, |r| r.set_width(w));
        }
    });
    let weak = ui.as_weak();
    r.on_tool(move |name| {
        if let Some(ui) = weak.upgrade() {
            edit(&ui, |r| r.tool(&name));
        }
    });
    let weak = ui.as_weak();
    r.on_link(move |target| {
        if let Some(ui) = weak.upgrade() {
            edit(&ui, |r| r.link(&target));
        }
    });
}

/// Show the state in the window, keeping the list models it shows.
pub fn push(ui: &AppWindow, s: &ArchitectureState) {
    let _span = archguard_engine::self_trace::tracing::info_span!("push").entered();
    let mut m = window::model(s, &Texts::of(ui));
    let d = ui.global::<ArchitectureData>();
    sync::keep(&d.get_model(), &mut m);
    d.set_model(m);
    s.editor.show(ui);
}

fn download(s: &mut ArchitectureState, ui: &AppWindow, all: bool) {
    let Ok(doc) = &s.doc else { return };
    let (name, content) = if all {
        ("arc42.md".to_string(), doc.markdown(None))
    } else {
        (
            format!("arc42-{:02}.md", s.chapter),
            doc.markdown(Some(s.chapter)),
        )
    };
    if let Some(saved) = save(ui, &name, &content) {
        s.saved = saved;
    }
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
    if let Some(saved) = save(ui, name, &diagram::plantuml(&nodes, &edges, unit)) {
        s.saved = saved;
    }
}

/// Write an export where the save dialog points (never in headless
/// runs); `None` when cancelled.
fn save(ui: &AppWindow, name: &str, content: &str) -> Option<String> {
    if crate::context::headless_run() {
        return None;
    }
    let mut dialog = rfd::FileDialog::new()
        .set_title(ui.get_export_title().as_str())
        .set_file_name(name);
    if let Some(dir) = start_dir(std::env::home_dir().as_deref()) {
        dialog = dialog.set_directory(dir);
    }
    let path = dialog.save_file()?;
    Some(write(&path, content))
}

/// Where the save dialog starts: the user's Downloads folder if it exists,
/// else `None` (the dialog chooses). Never the project, where Enter would
/// replace its own arc42.md with the generated text.
fn start_dir(home: Option<&Path>) -> Option<std::path::PathBuf> {
    Some(home?.join("Downloads")).filter(|d| d.is_dir())
}

/// Write `content` in one step; the path written, or the error.
fn write(path: &Path, content: &str) -> String {
    match crate::export::write(path, content.as_bytes()) {
        Ok(()) => path.display().to_string(),
        Err(e) => e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let state = build_from(
            &root,
            &run.config,
            run.graph.clone(),
            &run.report,
            &calls,
            archguard_engine::core::DocLang::De,
        );
        let doc = state.doc.as_ref().expect("arc42 document");
        assert_eq!(doc.chapters.len(), 12);
        assert_eq!(state.chapter, 5);
        assert_eq!(derived(&state, 5), Some(Kind::Dependencies));
        assert_eq!(derived(&state, 11), Some(Kind::Cycles));
        assert_eq!(derived(&state, 1), None);
        assert_eq!(state.deps.len(), run.graph.edges.len());
    }

    /// Save in the close or quit prompt with Settings also unsaved writes
    /// the open arc42 draft too, instead of dropping it.
    #[test]
    fn saving_from_the_prompt_writes_the_arc42_draft_too() {
        let dir = tempfile::tempdir().expect("temp dir");
        let old = crate::arc42_file::tests::doc("Old");
        std::fs::write(dir.path().join("arc42.md"), old.markdown(None)).unwrap();
        let (a, _) = documents::with_store(|s| s.open(dir.path()));
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/py-layers")
            .canonicalize()
            .expect("fixture");
        let loaded = archguard_engine::load_config(None, &root).expect("config");
        let run =
            archguard_engine::analyze_run_quiet(&root, &loaded, archguard_engine::Level::Module)
                .expect("analysis");
        let calls = archguard_engine::analyze_calls(&root, &run.config);
        // The fixture's analysis, but the arc42 text of the temporary project.
        let mut state = build_from(
            dir.path(),
            &run.config,
            run.graph.clone(),
            &run.report,
            &calls,
            archguard_engine::core::DocLang::De,
        );
        state.doc = Ok(old);
        state.editor.doc = a;
        documents::set_state(a, state);
        documents::with_state(a, |s: &mut ArchitectureState| {
            start_edit(s);
            let text = s.editor.chapter(s.doc.as_ref().unwrap(), 5) + "Neu.\n";
            s.editor.edit(&text);
        });
        // Settings is open with unsaved edits and its own save hook.
        documents::set_save_hook(a, crate::config_editor::PART, || Ok(()));
        documents::mark_dirty(a, crate::config_editor::PART, "Layer rules");
        assert_eq!(documents::save(a), Ok(()));
        let text = std::fs::read_to_string(dir.path().join("arc42.md")).unwrap();
        assert!(text.contains("Neu.\n"), "the draft is written");
        assert!(!documents::is_dirty(a));
    }

    /// A download goes only where the save dialog points, in one step;
    /// a missing folder is an error, never a fallback into the project.
    #[test]
    fn a_download_writes_only_the_chosen_file() {
        let d = tempfile::tempdir().expect("temp dir");
        let chosen = d.path().join("arc42.md");
        assert_eq!(write(&chosen, "# arc42\n"), chosen.display().to_string());
        assert_eq!(std::fs::read_to_string(&chosen).unwrap(), "# arc42\n");
        let missing = d.path().join("gone/arc42.md");
        assert!(write(&missing, "x").starts_with("Could not write"));
        assert!(!d.path().join("gone").exists());
    }

    /// The save dialog never starts in the project, where Enter would
    /// replace its own arc42.md: Downloads, else the dialog's choice.
    #[test]
    fn a_download_starts_in_the_downloads_folder_or_where_the_dialog_likes() {
        let home = tempfile::tempdir().expect("temp dir");
        assert_eq!(start_dir(Some(home.path())), None, "no Downloads folder");
        assert_eq!(start_dir(None), None, "no home folder");
        std::fs::create_dir(home.path().join("Downloads")).unwrap();
        assert_eq!(
            start_dir(Some(home.path())),
            Some(home.path().join("Downloads"))
        );
    }
}
