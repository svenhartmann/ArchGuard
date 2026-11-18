//! The chapter editor of the Architecture tab, saved into the project's
//! arc42 text (see [`crate::arc42_file`]). Visual: the chapter as
//! formatted text to type in ([`super::rich`]). Markdown: the whole
//! chapter beside its preview.

use std::path::{Path, PathBuf};

use archguard_engine::core::DiagramFormat;
use archguard_engine::report::Arc42Document;
use slint::ComponentHandle;

use super::rich::layout::Style;
#[cfg(test)]
use super::rich::view::Key;
use super::rich::view::Rich;
use super::{blocks, markdown, sync};
use crate::arc42_file::{self, Arc42Files, Target, diagram_block};
use crate::documents::{self, DocId};
use crate::tab_support::Texts;
use crate::ui::{
    AppWindow, ArcFigure, ArchitectureData, ArchitectureModel, RichData, RichItem, Theme,
};

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
    /// The format diagrams the chapter lacks go in with (`[arc42]`).
    format: DiagramFormat,
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
    /// The file's text when editing began. Kept with the draft, so a new
    /// analysis that reads the file again cannot hide a change on disk.
    base: Option<String>,
    /// The formatted editor while the visual editor shows; made from the
    /// draft when it opens, and it writes the draft on every edit.
    rich: Option<Rich>,
}

impl Editor {
    /// Find and read the project's arc42 text.
    pub fn new(doc: DocId, root: &Path) -> Self {
        let mut editor = Editor {
            doc,
            root: root.to_path_buf(),
            files: None,
            files_error: String::new(),
            format: DiagramFormat::Mermaid,
        };
        if let Some(target) = arc42_file::find(root) {
            match Arc42Files::load(target) {
                Ok(f) => editor.files = Some(f),
                Err(e) => editor.files_error = e,
            }
        }
        editor
    }

    pub fn with_format(self, format: DiagramFormat) -> Self {
        Editor { format, ..self }
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
            m.file = open
                .as_ref()
                .and_then(|e| self.file_of(e.chapter))
                .unwrap_or_default()
                .into();
            m.file_error = self.files_error.as_str().into();
            if let Some(e) = open {
                m.editing = true;
                m.dirty = e.draft != e.original;
                m.ask_location = e.ask_location;
                m.edit_error = e.error.as_str().into();
            }
        });
    }

    /// The file chapter `n` is saved to, relative to the project; `None`
    /// while the project has no arc42 text.
    fn file_of(&self, n: u8) -> Option<String> {
        let path = self.files.as_ref()?.path_of(n);
        let shown = path.strip_prefix(&self.root).unwrap_or(&path);
        Some(shown.display().to_string())
    }

    /// What the open chapter is called in the save notice and the close
    /// prompt: "arc42 chapter 5 (docs/arc42.md)".
    pub fn label(&self, chapter: u8) -> String {
        match self.file_of(chapter) {
            Some(f) => format!("arc42 chapter {chapter} ({f})"),
            None => format!("arc42 chapter {chapter}"),
        }
    }

    /// Open chapter `chapter` as it is now, unless an open draft has
    /// unsaved changes (they are never dropped). An unchanged open chapter
    /// stays as it is, with its caret.
    pub fn ensure_open(&self, doc: &Arc42Document, chapter: u8) {
        let text = self.chapter(doc, chapter);
        let keep = self.with_open(|o| {
            o.as_ref().is_some_and(|e| {
                e.draft != e.original
                    || (e.chapter == chapter && e.original.trim_end() == text.trim_end())
            })
        });
        if !keep {
            self.open(chapter, text);
        }
    }

    /// The chapter open in the editor.
    pub fn open_chapter(&self) -> Option<u8> {
        self.with_open(|o| o.as_ref().map(|e| e.chapter))
    }

    /// Whether the open chapter has unsaved changes.
    pub fn is_dirty(&self) -> bool {
        self.with_open(|o| o.as_ref().is_some_and(|e| e.draft != e.original))
    }

    #[cfg(any(test, feature = "snapshot"))]
    pub fn start(&self, doc: &Arc42Document, chapter: u8) {
        // LF in the draft, as the block ranges expect; saving keeps CRLF files CRLF.
        self.open(chapter, self.chapter(doc, chapter));
    }

    fn open(&self, chapter: u8, text: String) {
        let open = Open {
            chapter,
            ask_location: false,
            error: String::new(),
            draft: text.clone(),
            original: text,
            base: self.files.as_ref().and_then(|f| f.saved(chapter)),
            rich: None,
        };
        documents::set_state(self.doc, Some(open));
    }

    /// Show this document's draft in the window's one editor: as
    /// formatted text, or as Markdown beside the chapter as the reader
    /// would show it. On every keystroke: only the rows that changed are set.
    pub fn show(
        &self,
        ui: &AppWindow,
        figure: &dyn Fn(&str) -> Option<ArcFigure>,
        own: &dyn Fn(&str) -> Option<ArcFigure>,
        drawn: &[(String, String)],
    ) {
        let note = Texts::of(ui)
            .t("Diagram source · this diagram syntax is not supported in the viewer.")
            .to_string();
        let d = ui.global::<ArchitectureData>();
        let visual = d.get_visual();
        let style = style(ui);
        // Unsaved changes stay open to edit.
        let r = ui.global::<RichData>();
        if r.get_reading() && self.is_dirty() {
            r.set_reading(false);
        }
        let reading = r.get_reading();
        let shown = self.with_open(|o| {
            let e = o.as_mut()?;
            if !visual && !reading {
                e.rich = None;
                let body = blocks::chapter_body(&e.draft, e.chapter, false);
                let rows = blocks::ui_blocks(&markdown::parse(&body), &note);
                return Some((e.draft.clone(), Err(rows)));
            }
            let rich = e.rich.get_or_insert_with(|| Rich::new(&e.draft));
            rich.set_style(style);
            rich.reading = reading;
            let items = rich.items(&note, figure, own, drawn);
            Some((e.draft.clone(), Ok((items, rich.tools()))))
        });
        let Some((draft, shown)) = shown else {
            return;
        };
        if d.get_draft() != draft.as_str() {
            d.set_draft(draft.as_str().into());
        }
        match shown {
            Err(rows) => d.set_preview(sync::keep_blocks(&d.get_preview(), rows)),
            Ok((items, tools)) => {
                let r = ui.global::<RichData>();
                r.set_items(keep_items(&r.get_items(), items));
                r.set_tools(tools);
            }
        }
    }

    /// Draw the chapter anew, its diagrams too (a tick changed them).
    pub fn redraw(&self) {
        self.with_open(|o| o.as_mut().map(|e| e.rich = None));
    }

    /// Whether chapter `n` is in the project's arc42 text.
    pub fn in_file(&self, n: u8) -> bool {
        self.files.as_ref().is_some_and(|f| f.has_chapter(n))
    }

    /// Keep the edited Markdown; the document is unsaved while it differs.
    pub fn edit(&self, text: &str) {
        self.change(|e| {
            e.draft = text.to_string();
            e.rich = None;
        });
    }

    /// Insert diagram `id` (its Markdown `section`) as a diagram block: at
    /// the caret in the formatted editor, at the end of the Markdown.
    pub fn insert_diagram(&self, id: &str, section: &str) {
        self.change(|e| match &mut e.rich {
            Some(rich) => {
                rich.insert_diagram(id, section);
                e.draft = rich.markdown();
            }
            None => {
                let block = diagram_block(id, section);
                e.draft = format!("{}\n\n{block}\n", e.draft.trim_end());
            }
        });
    }

    /// Run `f` on the formatted editor, then keep what it wrote as the draft.
    pub fn rich<R>(&self, f: impl FnOnce(&mut Rich) -> R) -> Option<R> {
        let mut out = None;
        self.change(|e| {
            if let Some(rich) = &mut e.rich {
                out = Some(f(rich));
                e.draft = rich.markdown();
            }
        });
        out
    }

    /// Change the open draft; the document is unsaved while it differs
    /// from the chapter the editor opened.
    fn change(&self, f: impl FnOnce(&mut Open)) {
        let changed = self.with_open(|o| {
            o.as_mut().map(|e| {
                let old = e.draft.clone();
                f(e);
                // A refused save's reason holds until the text changes.
                if e.draft != old {
                    e.error.clear();
                }
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
            documents::mark_dirty(self.doc, PART, &self.label(chapter));
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

    pub(super) fn set_error(&self, error: String) {
        self.with_open(|o| o.as_mut().map(|e| e.error = error));
    }

    /// Save the draft; the path written, or `None` (an error, or the
    /// question where to create the file). The chapter stays open as
    /// saved, with the formatted editor and its caret unless the save
    /// refreshed generated text.
    pub fn save(&mut self, doc: &Arc42Document) -> Option<String> {
        let (chapter, draft, mut base) = self.with_open(|o| {
            o.as_ref()
                .map(|e| (e.chapter, e.draft.clone(), e.base.clone()))
        })?;
        let Some(files) = &mut self.files else {
            // The project has arc42 text that could not be read: no new place.
            if !self.files_error.is_empty() {
                self.set_error(self.files_error.clone());
                return None;
            }
            self.with_open(|o| o.as_mut().map(|e| e.ask_location = true));
            return None;
        };
        files.format = self.format;
        match files.save_chapter(doc, chapter, &draft, &mut base) {
            Ok(path) => {
                self.files_error.clear();
                let text = self.chapter(doc, chapter);
                let base = self.files.as_ref().and_then(|f| f.saved(chapter));
                self.with_open(|o| {
                    let e = o.as_mut()?;
                    // The file ends a chapter with its own blank lines.
                    if e.draft.trim_end() != text.trim_end() {
                        e.draft = text;
                        e.rich = None;
                    }
                    (e.original, e.base, e.error) = (e.draft.clone(), base, String::new());
                    Some(())
                });
                self.mark(chapter, false);
                Some(path.display().to_string())
            }
            Err(e) => {
                self.with_open(|o| o.as_mut().map(|o| o.base = base));
                self.set_error(e);
                None
            }
        }
    }

    /// The shown diagrams the last save took out of their chapter.
    pub fn take_dropped(&mut self) -> Vec<String> {
        self.files
            .as_mut()
            .map(|f| std::mem::take(&mut f.dropped))
            .unwrap_or_default()
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

    /// `ARCHGUARD_DESKTOP_ARC_EDIT=1` opens the chapter editor with the
    /// caret in its first editable block (`ARCHGUARD_DESKTOP_TYPE` types
    /// there), `=take` also takes its first generated block over as own
    /// text, `=save` also saves it (asking where, in a project without
    /// arc42 text).
    #[cfg(feature = "snapshot")]
    pub fn snapshot(&mut self, doc: &Arc42Document, chapter: u8) -> Option<String> {
        let mode = std::env::var("ARCHGUARD_DESKTOP_ARC_EDIT").ok()?;
        self.start(doc, chapter);
        if mode == "take" {
            self.with_open(|o| {
                let e = o.as_mut()?;
                let mut rich = Rich::new(&e.draft);
                let pieces = rich.editor.doc().pieces.len();
                (0..pieces).find(|&p| rich.editor.take_over(p, &[]))?;
                e.draft = rich.markdown();
                e.rich = Some(rich);
                Some(())
            });
        }
        (mode == "save").then(|| self.save(doc)).flatten()
    }

    #[cfg(not(feature = "snapshot"))]
    pub fn snapshot(&mut self, _: &Arc42Document, _: u8) -> Option<String> {
        None
    }
}

/// Families, sizes and weights of the window's theme, as the reader uses them.
fn style(ui: &AppWindow) -> Style {
    let t = ui.global::<Theme>();
    Style {
        family: t.get_font_family().into(),
        mono: t.get_mono_family().into(),
        body: t.get_font_body(),
        small: t.get_font_small(),
        section: t.get_font_section(),
        page: t.get_font_page(),
        title: t.get_weight_title() as f32,
        strong: t.get_weight_strong() as f32,
        scale: ui.window().scale_factor(),
    }
}

/// The new rows in the old model where it has as many: only rows that
/// changed are set, so the others keep their elements (and the caret's
/// input its focus).
fn keep_items(old: &slint::ModelRc<RichItem>, new: Vec<RichItem>) -> slint::ModelRc<RichItem> {
    use slint::Model;
    if old.row_count() != new.len() {
        return crate::tab_support::model(new);
    }
    for (i, item) in new.into_iter().enumerate() {
        if old.row_data(i).is_none_or(|o| !same_item(&o, &item)) {
            old.set_row_data(i, item);
        }
    }
    old.clone()
}

fn rows<T: PartialEq + Clone + 'static>(x: &slint::ModelRc<T>, y: &slint::ModelRc<T>) -> bool {
    use slint::Model;
    x.iter().eq(y.iter())
}

fn same_item(a: &RichItem, b: &RichItem) -> bool {
    use slint::Model;
    (a.piece, a.block, a.kind, a.caret) == (b.piece, b.block, b.kind, b.caret)
        && (a.top, a.height, a.indent, a.marker_y, a.marker_h)
            == (b.top, b.height, b.indent, b.marker_y, b.marker_h)
        && a.marker == b.marker
        && a.caret_rect == b.caret_rect
        && a.fixed.row_count() == b.fixed.row_count()
        && (a.empty, a.generated) == (b.empty, b.generated)
        && a.figure.id == b.figure.id
        && rows(&a.cells, &b.cells)
        && rows(&a.runs, &b.runs)
        && rows(&a.selection, &b.selection)
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

    /// Why the open draft's last save failed, as the save bar shows it.
    fn error(editor: &Editor) -> String {
        let mut m = ArchitectureModel::default();
        editor.fill(&mut m);
        m.edit_error.to_string()
    }

    #[test]
    fn each_document_keeps_its_own_draft() {
        let ((a_dir, a), (b_dir, b)) = (project(), project());
        let new = doc("Old");
        let mut in_a = Editor::new(a, a_dir.path());
        in_a.start(&new, 5);
        in_a.edit("## 5. Title\n\nText von A.\n");
        // The other document opens an editor meanwhile.
        let in_b = Editor::new(b, b_dir.path());
        in_b.start(&new, 3);
        in_b.edit("## 3. Title\n\nText von B.\n");
        let path = a_dir.path().join("arc42.md");
        assert_eq!(in_a.save(&new), Some(path.display().to_string()));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("Text von A.") && !text.contains("Text von B."));
        assert!(!in_a.is_dirty() && in_b.is_dirty());
    }

    /// A diagram inserted into a chapter of own text is saved as a diagram
    /// block, which the next save refreshes from the analysis.
    #[test]
    fn a_diagram_inserted_into_a_chapter_of_own_text_is_saved() {
        let d = tempfile::tempdir().expect("temp dir");
        let path = d.path().join("arc42.md");
        std::fs::write(&path, "## 5. Bausteine\n\nEigener Text.\n").unwrap();
        let (a, _) = documents::with_store(|s| s.open(d.path()));
        let new = doc("Old");
        let mut editor = Editor::new(a, d.path());
        editor.start(&new, 5);
        let section = "### Paketübersicht\n\nQuelle: Analyse.\n\n```mermaid\nflowchart LR\n```\n";
        editor.with_open(|o| {
            let e = o.as_mut().unwrap();
            e.rich = Some(Rich::new(&e.draft));
        });
        // The caret at the end of the own text.
        editor.rich(|r| {
            r.key("down", false, false, false);
            r.key("end", false, false, false);
        });
        editor.insert_diagram("unknown", section);
        assert!(documents::is_dirty(a) && editor.is_dirty());
        assert!(editor.save(&new).is_some());
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            text,
            format!(
                "## 5. Bausteine\n\nEigener Text.\n\n{}\n",
                diagram_block("unknown", section)
            ),
            "a diagram the analysis does not know stays as inserted"
        );
    }

    /// Typing in the formatted editor: nothing changes until a key does,
    /// the text lands after the generated block and the save keeps it.
    #[test]
    fn the_formatted_editor_writes_the_draft_and_keeps_generated_blocks() {
        let (dir, a) = project();
        let new = doc("Old");
        let mut editor = Editor::new(a, dir.path());
        editor.start(&new, 5);
        editor.with_open(|o| {
            let e = o.as_mut().unwrap();
            e.rich = Some(Rich::new(&e.draft));
        });
        assert!(!documents::is_dirty(a), "opening changes nothing");
        assert_eq!(
            editor.rich(|r| r.key("end", false, false, false)),
            Some(Key::Done)
        );
        assert!(!documents::is_dirty(a), "a move changes nothing");
        editor.rich(|r| {
            r.type_text("Eigener ");
            r.key("b", false, false, true);
            r.type_text("Text");
            r.key("enter", false, false, false);
            r.type_text("- eins");
        });
        assert!(documents::is_dirty(a));
        assert!(editor.save(&new).is_some());
        let text = std::fs::read_to_string(dir.path().join("arc42.md")).unwrap();
        assert!(text.contains("Old 5"), "the generated block stays: {text}");
        assert!(
            text.contains("\n\nEigener **Text**\n\n\\- eins\n"),
            "{text}"
        );
    }

    /// The chapter stays open after a save, with its caret; an unchanged
    /// one follows the file, a changed one is never replaced.
    #[test]
    fn the_chapter_stays_open_and_follows_the_file_until_it_is_changed() {
        let (dir, a) = project();
        let new = doc("Old");
        let editor = Editor::new(a, dir.path());
        editor.ensure_open(&new, 5);
        editor.with_open(|o| {
            let e = o.as_mut().unwrap();
            e.rich = Some(Rich::new(&e.draft));
        });
        editor.rich(|r| {
            r.key("end", false, false, false);
            r.type_text("Neu");
        });
        let mut editor = editor;
        assert!(editor.save(&new).is_some());
        assert!(editor.is_open() && !editor.is_dirty());
        assert!(
            editor.rich(|_| ()).is_some(),
            "the formatted editor and its caret stay"
        );
        // Another chapter while unchanged: it opens.
        editor.ensure_open(&new, 3);
        assert_eq!(editor.open_chapter(), Some(3));
        // Changed: neither another chapter nor the file replaces the draft.
        editor.edit("## 3. Title\n\nEntwurf.\n");
        editor.ensure_open(&new, 5);
        assert_eq!(editor.open_chapter(), Some(3));
        assert!(editor.is_dirty());
        // Discarded: the chapter opens again as the file has it.
        editor.cancel();
        editor.ensure_open(&new, 3);
        assert!(editor.is_open() && !editor.is_dirty());
    }

    /// Markdown typed in the whole chapter makes a new formatted editor.
    #[test]
    fn markdown_edits_make_the_formatted_editor_anew() {
        let (dir, a) = project();
        let editor = Editor::new(a, dir.path());
        editor.start(&doc("Old"), 5);
        editor.with_open(|o| {
            let e = o.as_mut().unwrap();
            e.rich = Some(Rich::new(&e.draft));
        });
        editor.edit("## 5. Title\n\nAlles *neu*.\n");
        assert_eq!(editor.rich(|_| ()), None, "the old one is gone");
        let text = editor.with_open(|o| {
            let e = o.as_mut().unwrap();
            let rich = e.rich.insert(Rich::new(&e.draft));
            rich.markdown()
        });
        assert_eq!(text, "## 5. Title\n\nAlles *neu*.\n");
    }

    #[test]
    fn an_open_draft_survives_a_new_analysis_and_marks_the_document_dirty() {
        let (dir, a) = project();
        let new = doc("Old");
        let editor = Editor::new(a, dir.path());
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

    /// `git pull` changes the chapter while it is edited, then a new
    /// analysis reads the file again: the save still sees the change.
    #[test]
    fn a_chapter_pulled_during_editing_survives_a_new_analysis_and_the_save() {
        let (dir, a) = project();
        let new = doc("Old");
        let path = dir.path().join("arc42.md");
        let editor = Editor::new(a, dir.path());
        editor.start(&new, 5);
        editor.edit(&(editor.chapter(&new, 5) + "Mein Entwurf.\n"));
        let pulled = std::fs::read_to_string(&path).unwrap().replace(
            "<!-- Eigene Erläuterungen zu Kapitel 5 hier ergänzen. -->",
            "Gepullter Text.",
        );
        std::fs::write(&path, &pulled).unwrap();
        documents::with_store(|s| s.get_mut(a).map(documents::Document::forget_derived));
        let mut rebuilt = Editor::new(a, dir.path());
        assert_eq!(rebuilt.save(&new), None, "refused");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            pulled,
            "not written"
        );
        // The pulled change is in this very chapter: the message says that
        // saving again replaces it, so the second save is a conscious choice.
        let reason = rebuilt.unsaved_reason();
        assert!(
            reason.contains("Chapter 5 itself was changed")
                && reason.contains("saving again replaces that change"),
            "{reason}"
        );
        assert!(rebuilt.is_open(), "the draft stays");
        // Saving again anyway writes the draft over the pulled chapter.
        assert_eq!(rebuilt.save(&new), Some(path.display().to_string()));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("Mein Entwurf.") && !text.contains("Gepullter Text."));
    }

    /// A refused save shows why until the text changes: fixed, or back to
    /// the chapter as opened, the message goes; a caret move keeps it.
    #[test]
    fn a_save_error_clears_when_the_text_changes() {
        let (dir, a) = project();
        let new = doc("Old");
        let mut editor = Editor::new(a, dir.path());
        editor.start(&new, 5);
        let original = editor.chapter(&new, 5);
        editor.edit("Kein Kapitelkopf.\n");
        assert_eq!(editor.save(&new), None, "a chapter without heading");
        assert!(!error(&editor).is_empty());
        editor.with_open(|o| {
            let e = o.as_mut().unwrap();
            e.rich = Some(Rich::new(&e.draft));
        });
        editor.rich(|r| r.key("end", false, false, false));
        assert!(!error(&editor).is_empty(), "a caret move keeps it");
        editor.edit(&original);
        assert_eq!(error(&editor), "", "the fixed text drops it");
        assert!(!editor.is_dirty());
    }

    /// An arc42.md that is not UTF-8 cannot be read: the save says why,
    /// instead of asking where to create a file the project already has.
    #[test]
    fn an_unreadable_arc42_text_is_the_save_error_not_a_location_question() {
        let d = tempfile::tempdir().expect("temp dir");
        let path = d.path().join("arc42.md");
        std::fs::write(&path, b"## 5. Bausteine\n\nGr\xfc\xdfe.\n").unwrap();
        let (a, _) = documents::with_store(|s| s.open(d.path()));
        let new = doc("Old");
        let mut editor = Editor::new(a, d.path());
        editor.start(&new, 5);
        editor.edit("## 5. Bausteine\n\nNeu.\n");
        assert_eq!(editor.save(&new), None);
        let asked = editor.with_open(|o| o.as_ref().map(|e| e.ask_location));
        assert_eq!(asked, Some(false), "no question where to create it");
        let reason = editor.unsaved_reason();
        assert!(reason.contains("arc42.md") && reason.contains("UTF-8"), "{reason}");
        assert_eq!(std::fs::read(&path).unwrap(), b"## 5. Bausteine\n\nGr\xfc\xdfe.\n");
    }

    /// A pull that changes only another chapter: refused once, then the
    /// draft is saved into the new file and the other change stays.
    #[test]
    fn a_pull_into_another_chapter_is_kept_by_saving_again() {
        let (dir, a) = project();
        let new = doc("Old");
        let path = dir.path().join("arc42.md");
        let mut editor = Editor::new(a, dir.path());
        editor.start(&new, 5);
        editor.edit(&(editor.chapter(&new, 5) + "Mein Entwurf.\n"));
        let pulled = std::fs::read_to_string(&path).unwrap().replace(
            "<!-- Eigene Erläuterungen zu Kapitel 9 hier ergänzen. -->",
            "Gepullter Text.",
        );
        std::fs::write(&path, &pulled).unwrap();
        assert_eq!(editor.save(&new), None, "refused");
        let reason = editor.unsaved_reason();
        assert!(
            reason.contains("save again to write this chapter into it")
                && !reason.contains("itself"),
            "{reason}"
        );
        assert_eq!(editor.save(&new), Some(path.display().to_string()));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("Mein Entwurf.") && text.contains("Gepullter Text."));
    }
}
