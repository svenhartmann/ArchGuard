//! The chapter editor of the Architecture tab: Markdown beside its
//! preview, saved into the project's arc42 text (see [`crate::arc42_file`]).

use std::path::{Path, PathBuf};

use archguard_engine::report::Arc42Document;
use slint::{ComponentHandle, SharedString};

use super::{blocks, markdown, sync};
use crate::arc42_file::{self, Arc42Files, Target};
use crate::documents::{self, DocId};
use crate::tab_support::Texts;
use crate::ui::{AppWindow, ArchitectureData, ArchitectureModel};

/// The part of a document the chapter editor owns (`documents::mark_dirty`).
pub(super) const PART: &str = "arc42";

/// The project's arc42 text and the open chapter editor, if any.
pub struct Editor {
    /// The document whose open editor ([`Open`]) this shows; `show` sets
    /// it, as the state is built on a worker that does not know it.
    pub(super) doc: DocId,
    root: PathBuf,
    /// `None` until the user chose where to create it.
    files: Option<Arc42Files>,
    /// Why the project's arc42 text could not be read.
    files_error: String,
}

/// An open chapter editor. Kept per document as its own state, not in the
/// derived `ArchitectureState`: a new analysis rebuilds that without
/// dropping the draft, and another document's editor never shows here.
struct Open {
    chapter: u8,
    /// No arc42 text in the project yet: the user is asked where to put it.
    ask_location: bool,
    error: String,
    /// The edited Markdown.
    draft: String,
    /// The chapter as the editor opened it: an equal draft is unsaved nothing.
    original: String,
}

impl Editor {
    /// Find and read the project's arc42 text.
    pub fn new(doc: DocId, root: &Path) -> Self {
        let mut editor = Editor {
            doc,
            root: root.to_path_buf(),
            files: None,
            files_error: String::new(),
        };
        if let Some(target) = arc42_file::find(root) {
            match Arc42Files::load(target) {
                Ok(f) => editor.files = Some(f),
                Err(e) => editor.files_error = e,
            }
        }
        editor
    }

    fn with_open<R: Default>(&self, f: impl FnOnce(&mut Option<Open>) -> R) -> R {
        documents::with_state(self.doc, f).unwrap_or_default()
    }

    pub fn is_open(&self) -> bool {
        self.with_open(|o| o.is_some())
    }

    /// The chapter as the reader and the editor show it: the project's
    /// text if it has one, else the generated chapter.
    pub fn chapter(&self, doc: &Arc42Document, n: u8) -> String {
        match &self.files {
            Some(f) => f.chapter(doc, n),
            None => arc42_file::generated(doc, n),
        }
    }

    /// The whole document, or the project's text before its chapters.
    pub fn preamble(&self, doc: &Arc42Document) -> String {
        self.files
            .as_ref()
            .map_or_else(|| doc.markdown(None), |f| f.preamble(doc))
    }

    pub fn fill(&self, m: &mut ArchitectureModel) {
        self.with_open(|open| {
            // The file the open chapter is saved to; the editor shows it.
            m.file = match (&self.files, &*open) {
                (Some(f), Some(e)) => {
                    let path = f.path_of(e.chapter);
                    let shown = path.strip_prefix(&self.root).unwrap_or(&path);
                    shown.display().to_string().into()
                }
                _ => SharedString::new(),
            };
            m.file_error = self.files_error.as_str().into();
            if let Some(e) = open {
                m.editing = true;
                m.ask_location = e.ask_location;
                m.edit_error = e.error.as_str().into();
            }
        });
    }

    pub fn start(&mut self, doc: &Arc42Document, chapter: u8) {
        let text = self.chapter(doc, chapter);
        let open = Open {
            chapter,
            ask_location: false,
            error: String::new(),
            draft: text.clone(),
            original: text,
        };
        documents::set_state(self.doc, Some(open));
    }

    /// Show this document's draft in the window's one editor.
    pub fn show(&self, ui: &AppWindow) {
        let Some(draft) = self.with_open(|o| o.as_ref().map(|e| e.draft.clone())) else {
            return;
        };
        let d = ui.global::<ArchitectureData>();
        if d.get_draft() != draft.as_str() {
            d.set_draft(draft.as_str().into());
        }
        self.preview(ui, &draft);
    }

    /// Keep the edited text; the document is unsaved while it differs.
    pub fn edit(&self, text: &str) {
        let changed = self.with_open(|o| {
            o.as_mut().map(|e| {
                e.draft = text.to_string();
                (e.chapter, e.draft != e.original)
            })
        });
        if let Some((chapter, dirty)) = changed {
            self.mark(chapter, dirty);
        }
    }

    pub fn cancel(&self) {
        if let Some(e) = self.with_open(Option::take) {
            self.mark(e.chapter, false);
        }
    }

    /// Mark this editor's part of the document dirty or clean.
    fn mark(&self, chapter: u8, dirty: bool) {
        if dirty {
            documents::mark_dirty(self.doc, PART, &format!("arc42 chapter {chapter}"));
        } else {
            documents::mark_clean(self.doc, PART);
        }
    }

    /// Why the open draft was not saved.
    pub(super) fn unsaved_reason(&self) -> String {
        self.with_open(|o| match o {
            Some(e) if !e.error.is_empty() => e.error.clone(),
            _ => "Choose where the arc42 text goes in the Architecture tab.".into(),
        })
    }

    fn set_error(&self, error: String) {
        self.with_open(|o| o.as_mut().map(|e| e.error = error));
    }

    /// Show the draft as the reader would.
    pub fn preview(&self, ui: &AppWindow, text: &str) {
        let Some(chapter) = self.with_open(|o| o.as_ref().map(|e| e.chapter)) else {
            return;
        };
        let body = blocks::chapter_body(text, chapter, false);
        let note = Texts::of(ui)
            .t("Diagram source · this diagram syntax is not supported in the viewer.")
            .to_string();
        let rows = blocks::ui_blocks(&markdown::parse(&body), &note);
        // Every keystroke: only the blocks that changed are set.
        let d = ui.global::<ArchitectureData>();
        d.set_preview(sync::keep_blocks(&d.get_preview(), rows));
    }

    /// Save the draft; the path written, or `None` while the editor stays
    /// open (an error, or the question where to create the file).
    pub fn save(&mut self, doc: &Arc42Document) -> Option<String> {
        let (chapter, draft) =
            self.with_open(|o| o.as_ref().map(|e| (e.chapter, e.draft.clone())))?;
        let Some(files) = &mut self.files else {
            self.with_open(|o| o.as_mut().map(|e| e.ask_location = true));
            return None;
        };
        match files.save_chapter(doc, chapter, &draft) {
            Ok(path) => {
                self.files_error.clear();
                self.cancel();
                Some(path.display().to_string())
            }
            Err(e) => {
                self.set_error(e);
                None
            }
        }
    }

    /// The answer to "where should the arc42 text go": 0 `arc42.md` in the
    /// project folder (as `archguard doc --arc42`), 1 another file in the
    /// project, else cancel. Saves into the chosen file.
    pub fn choose(&mut self, ui: &AppWindow, doc: &Arc42Document, choice: i32) -> Option<String> {
        self.with_open(|o| o.as_mut().map(|e| e.ask_location = false))?;
        let path = match choice {
            0 => self.root.join("arc42.md"),
            1 => ask_path(ui, &self.root)?,
            _ => return None,
        };
        if !arc42_file::finds(&self.root, &path) {
            self.set_error(
                Texts::of(ui)
                    .t("Choose a place inside the project folder, so the app finds the file next time.")
                    .to_string(),
            );
            return None;
        }
        match Arc42Files::load(Target::File(path)) {
            Ok(f) => self.files = Some(f),
            Err(e) => {
                self.set_error(e);
                return None;
            }
        }
        self.save(doc)
    }

    /// `ARCHGUARD_DESKTOP_ARC_EDIT=1` opens the chapter editor, `=save`
    /// also saves it (asking where, in a project without arc42 text).
    #[cfg(feature = "snapshot")]
    pub fn snapshot(&mut self, doc: &Arc42Document, chapter: u8) -> Option<String> {
        let mode = std::env::var("ARCHGUARD_DESKTOP_ARC_EDIT").ok()?;
        self.start(doc, chapter);
        (mode == "save").then(|| self.save(doc)).flatten()
    }

    #[cfg(not(feature = "snapshot"))]
    pub fn snapshot(&mut self, _: &Arc42Document, _: u8) -> Option<String> {
        None
    }
}

/// The save dialog for a new arc42 file; never in headless runs.
fn ask_path(ui: &AppWindow, root: &Path) -> Option<PathBuf> {
    if crate::context::headless_run() {
        return None;
    }
    rfd::FileDialog::new()
        .set_title(Texts::of(ui).t("Create the arc42 documentation"))
        .set_file_name("arc42.md")
        .add_filter("Markdown", &["md"])
        .set_directory(root)
        .save_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arc42_file::tests::doc;

    /// A project with an `arc42.md`, open as a document.
    fn project() -> (tempfile::TempDir, DocId) {
        let d = tempfile::tempdir().expect("temp dir");
        std::fs::write(d.path().join("arc42.md"), doc("Old").markdown(None)).unwrap();
        let (id, _) = documents::with_store(|s| s.open(d.path()));
        (d, id)
    }

    #[test]
    fn each_document_keeps_its_own_draft() {
        let ((a_dir, a), (b_dir, b)) = (project(), project());
        let new = doc("Old");
        let mut in_a = Editor::new(a, a_dir.path());
        in_a.start(&new, 5);
        in_a.edit("## 5. Title\n\nText von A.\n");
        // The other document opens an editor meanwhile.
        let mut in_b = Editor::new(b, b_dir.path());
        in_b.start(&new, 3);
        in_b.edit("## 3. Title\n\nText von B.\n");
        let path = a_dir.path().join("arc42.md");
        assert_eq!(in_a.save(&new), Some(path.display().to_string()));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("Text von A.") && !text.contains("Text von B."));
        assert!(!in_a.is_open() && in_b.is_open());
    }

    #[test]
    fn an_open_draft_survives_a_new_analysis_and_marks_the_document_dirty() {
        let (dir, a) = project();
        let new = doc("Old");
        let mut editor = Editor::new(a, dir.path());
        editor.start(&new, 5);
        let original = editor.chapter(&new, 5);
        editor.edit(&(original.clone() + "Neu.\n"));
        assert!(documents::is_dirty(a), "the close and quit prompt asks");
        editor.edit(&original);
        assert!(!documents::is_dirty(a), "back to the file: nothing unsaved");
        editor.edit(&(original + "Neu.\n"));
        // A new analysis drops the derived state; its editor is built again.
        documents::with_store(|s| s.get_mut(a).map(documents::Document::forget_derived));
        let mut rebuilt = Editor::new(a, dir.path());
        assert!(rebuilt.is_open());
        documents::mark_dirty(a, crate::config_editor::PART, "Layer rules");
        assert!(rebuilt.save(&new).is_some());
        let text = std::fs::read_to_string(dir.path().join("arc42.md")).unwrap();
        assert!(text.contains("Neu.\n"));
        assert!(
            documents::is_dirty(a),
            "the other unsaved change stays marked"
        );
    }
}
