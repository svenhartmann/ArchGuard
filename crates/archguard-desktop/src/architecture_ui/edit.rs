//! The chapter editor of the Architecture tab, saved into the project's
//! arc42 text (see [`crate::arc42_file`]). Visual: the chapter as the
//! reader shows it, one block at a time open as Markdown in its place.
//! Markdown: the whole chapter beside its preview.

use std::ops::Range;
use std::path::{Path, PathBuf};

use archguard_engine::report::Arc42Document;
use slint::{ComponentHandle, SharedString};

use super::{blocks, markdown, sync};
use crate::arc42_file::{self, Arc42Files, Target};
use crate::documents::{self, DocId};
use crate::tab_support::Texts;
use crate::ui::{AppWindow, ArcBlock, ArchitectureData, ArchitectureModel};

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
    /// The file's text when editing began. Kept with the draft, so a new
    /// analysis that reads the file again cannot hide a change on disk.
    base: Option<String>,
    /// The block open in the visual editor: its bytes in `draft`.
    block: Option<Range<usize>>,
}

/// A top-level block of the draft as the visual editor shows it.
struct Piece {
    /// Its bytes in the draft.
    range: Range<usize>,
    blocks: Vec<markdown::Block>,
    /// Inside a generated archguard block: refreshed on save, so not edited.
    generated: bool,
}

impl Open {
    /// The chapter body in reading order; `None` is the open block. The
    /// text before and after the open block is parsed on its own, so the
    /// other blocks stay as they are while it is typed in.
    fn pieces(&self) -> Vec<Option<Piece>> {
        let end = self.draft.len();
        let body = end - blocks::chapter_body(&self.draft, self.chapter, false).len();
        let generated = &generated(&self.draft);
        let part = |from: usize, to: usize| {
            let chunks = markdown::chunks(&self.draft[from..to]).into_iter();
            chunks.map(move |(r, blocks)| {
                let range = r.start + from..r.end + from;
                Some(Piece {
                    generated: generated.iter().any(|g| g.contains(&range.start)),
                    range,
                    blocks,
                })
            })
        };
        match &self.block {
            None => part(body, end).collect(),
            // `min`: a chapter heading typed into the block moves the body.
            Some(b) => part(body.min(b.start), b.start)
                .chain([None])
                .chain(part(b.end, end))
                .collect(),
        }
    }

    /// The pieces as rows of the editor; a row knows its piece.
    fn rows(&self, note: &str) -> Vec<ArcBlock> {
        let mut rows = Vec::new();
        for (i, piece) in self.pieces().iter().enumerate() {
            let Some(p) = piece else {
                rows.push(blocks::editor_row());
                continue;
            };
            let piece = if p.generated { -1 } else { i as i32 };
            let shown = blocks::ui_blocks(&p.blocks, note).into_iter();
            rows.extend(shown.map(|row| ArcBlock { piece, ..row }));
        }
        rows
    }
}

/// The bytes of `text` inside generated `archguard:arc42` blocks; a block
/// without its end marker runs to the end.
fn generated(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut open = None;
    for (i, _) in text.match_indices(arc42_file::MARKER) {
        let tag = text[i..].split_once("-->").map_or("", |(tag, _)| tag);
        if tag.trim_end().ends_with(":start") {
            open.get_or_insert(i);
        } else if let Some(start) = open.take() {
            out.push(start..i);
        }
    }
    out.extend(open.map(|start| start..text.len()));
    out
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
        // LF in the draft, as the block ranges expect; saving keeps CRLF files CRLF.
        let text = self.chapter(doc, chapter).replace("\r\n", "\n");
        let open = Open {
            chapter,
            ask_location: false,
            error: String::new(),
            draft: text.clone(),
            original: text,
            base: self.files.as_ref().and_then(|f| f.saved(chapter)),
            block: None,
        };
        documents::set_state(self.doc, Some(open));
    }

    /// Show this document's draft in the window's one editor: the
    /// Markdown, the open block's text and the chapter as the reader
    /// would show it. On every keystroke: only the rows that changed are set.
    pub fn show(&self, ui: &AppWindow) {
        let note = Texts::of(ui)
            .t("Diagram source · this diagram syntax is not supported in the viewer.")
            .to_string();
        let shown = self.with_open(|o| {
            let e = o.as_ref()?;
            let block = e.block.clone().map_or("", |b| &e.draft[b]);
            Some((e.draft.clone(), block.to_string(), e.rows(&note)))
        });
        let Some((draft, block, rows)) = shown else {
            return;
        };
        let d = ui.global::<ArchitectureData>();
        if d.get_draft() != draft.as_str() {
            d.set_draft(draft.as_str().into());
        }
        if d.get_block_text() != block.as_str() {
            d.set_block_text(block.as_str().into());
        }
        d.set_preview(sync::keep_blocks(&d.get_preview(), rows));
    }

    /// Keep the edited Markdown; the document is unsaved while it differs.
    pub fn edit(&self, text: &str) {
        self.change(|e| {
            e.draft = text.to_string();
            e.block = None;
        });
    }

    /// Open piece `index` of the visual editor in its place; an index
    /// without a piece starts a new block at the end of the chapter.
    pub fn open_block(&self, index: i32) {
        self.with_open(|o| {
            let Some(e) = o else { return };
            let mut pieces = e.pieces();
            let index = usize::try_from(index).unwrap_or(usize::MAX);
            e.block = match pieces.get_mut(index).map(Option::take) {
                Some(Some(p)) if !p.generated => Some(p.range),
                // The open block itself, or a generated one.
                Some(_) => return,
                None => Some(e.draft.len()..e.draft.len()),
            };
        });
    }

    /// The open block shows as the reader shows it again.
    pub fn close_block(&self) {
        self.with_open(|o| o.as_mut().map(|e| e.block = None));
    }

    /// Keep the text typed into the open block.
    pub fn edit_block(&self, text: &str) {
        self.change(|e| {
            let Some(b) = e.block.clone() else { return };
            let text = text.replace("\r\n", "\n");
            // A new block starts after an empty line.
            let before = &e.draft[..b.start];
            let gap = match () {
                () if !b.is_empty() || text.is_empty() || before.is_empty() => "",
                () if before.ends_with("\n\n") => "",
                () if before.ends_with('\n') => "\n",
                () => "\n\n",
            };
            e.draft.replace_range(b.clone(), &format!("{gap}{text}"));
            let start = b.start + gap.len();
            e.block = Some(start..start + text.len());
        });
    }

    /// Change the open draft; the document is unsaved while it differs
    /// from the chapter the editor opened.
    fn change(&self, f: impl FnOnce(&mut Open)) {
        let changed = self.with_open(|o| {
            o.as_mut().map(|e| {
                f(e);
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

    /// Save the draft; the path written, or `None` while the editor stays
    /// open (an error, or the question where to create the file).
    pub fn save(&mut self, doc: &Arc42Document) -> Option<String> {
        let (chapter, draft, mut base) = self.with_open(|o| {
            o.as_ref()
                .map(|e| (e.chapter, e.draft.clone(), e.base.clone()))
        })?;
        let Some(files) = &mut self.files else {
            self.with_open(|o| o.as_mut().map(|e| e.ask_location = true));
            return None;
        };
        match files.save_chapter(doc, chapter, &draft, &mut base) {
            Ok(path) => {
                self.files_error.clear();
                self.cancel();
                Some(path.display().to_string())
            }
            Err(e) => {
                self.with_open(|o| o.as_mut().map(|o| o.base = base));
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

    /// `ARCHGUARD_DESKTOP_ARC_EDIT=1` opens the chapter editor, `=block`
    /// also opens its first editable block (a new one, if it has none),
    /// `=save` also saves it (asking where, in a project without arc42 text).
    #[cfg(feature = "snapshot")]
    pub fn snapshot(&mut self, doc: &Arc42Document, chapter: u8) -> Option<String> {
        let mode = std::env::var("ARCHGUARD_DESKTOP_ARC_EDIT").ok()?;
        self.start(doc, chapter);
        if mode == "block" {
            let own = |p: &Option<Piece>| p.as_ref().is_some_and(|p| !p.generated);
            let first = self.with_open(|o| o.as_ref()?.pieces().iter().position(own));
            self.open_block(first.map_or(-1, |i| i as i32));
        }
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

    /// The pieces of the open draft: their text and whether generated.
    fn pieces(editor: &Editor) -> Vec<Option<(String, bool)>> {
        editor.with_open(|o| {
            let e = o.as_ref().expect("open editor");
            let text = |p: Piece| (e.draft[p.range].to_string(), p.generated);
            e.pieces().into_iter().map(|p| p.map(text)).collect()
        })
    }

    fn open_text(editor: &Editor) -> Option<String> {
        editor.with_open(|o| {
            let e = o.as_ref()?;
            e.block.clone().map(|b| e.draft[b].to_string())
        })
    }

    #[test]
    fn the_visual_editor_changes_one_block_in_place_and_no_generated_one() {
        let (dir, a) = project();
        let new = doc("Old");
        let mut editor = Editor::new(a, dir.path());
        editor.start(&new, 5);
        let generated = Some(("Old 5".to_string(), true));
        assert_eq!(pieces(&editor), vec![generated.clone()]);
        editor.open_block(0);
        assert_eq!(open_text(&editor), None, "a generated block does not open");
        // "Add text": a new block at the end, after an empty line.
        editor.open_block(-1);
        assert!(!documents::is_dirty(a), "nothing typed yet");
        editor.edit_block("Eigener Text.");
        editor.edit_block("Eigener Text.\r\n\r\n- a\n- b");
        assert!(documents::is_dirty(a));
        assert_eq!(pieces(&editor), vec![generated.clone(), None]);
        editor.close_block();
        let own = |s: &str| Some((s.to_string(), false));
        assert_eq!(
            pieces(&editor),
            vec![generated.clone(), own("Eigener Text."), own("- a\n- b")]
        );
        // One of two own blocks changes; the other stays as typed.
        editor.open_block(1);
        assert_eq!(open_text(&editor).as_deref(), Some("Eigener Text."));
        editor.edit_block("Neuer Text,\nzwei Zeilen.");
        assert_eq!(
            pieces(&editor),
            vec![generated, None, own("- a\n- b")],
            "the blocks around the open one stay"
        );
        // Another block opens straight from the open one.
        editor.open_block(2);
        assert_eq!(open_text(&editor).as_deref(), Some("- a\n- b"));
        assert!(editor.save(&new).is_some());
        let text = std::fs::read_to_string(dir.path().join("arc42.md")).unwrap();
        assert!(
            text.contains("-->\n\nNeuer Text,\nzwei Zeilen.\n\n- a\n- b\n"),
            "{text}"
        );
        assert!(text.contains("Old 5"), "the generated block stays: {text}");
    }

    #[test]
    fn typing_markdown_in_the_whole_chapter_closes_the_open_block() {
        let (dir, a) = project();
        let mut editor = Editor::new(a, dir.path());
        editor.start(&doc("Old"), 5);
        editor.open_block(-1);
        editor.edit("## 5. Title\n\nAlles neu.\n");
        assert_eq!(open_text(&editor), None, "its range is no longer valid");
        assert_eq!(
            pieces(&editor),
            vec![Some(("Alles neu.".to_string(), false))]
        );
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

    /// `git pull` changes the chapter while it is edited, then a new
    /// analysis reads the file again: the save still sees the change.
    #[test]
    fn a_chapter_pulled_during_editing_survives_a_new_analysis_and_the_save() {
        let (dir, a) = project();
        let new = doc("Old");
        let path = dir.path().join("arc42.md");
        let mut editor = Editor::new(a, dir.path());
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
        let reason = rebuilt.unsaved_reason();
        assert!(
            reason.contains("changed on disk") && reason.contains("save again"),
            "{reason}"
        );
        assert!(rebuilt.is_open(), "the draft stays");
        // Refused once, the new text is loaded: saving again writes the draft.
        assert_eq!(rebuilt.save(&new), Some(path.display().to_string()));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("Mein Entwurf.") && !text.contains("Gepullter Text."));
    }
}
