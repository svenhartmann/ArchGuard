//! The chapter editor of the Architecture tab: Markdown beside its
//! preview, saved into the project's arc42 text (see [`crate::arc42_file`]).

use std::path::{Path, PathBuf};

use archguard_engine::report::Arc42Document;
use slint::{ComponentHandle, SharedString};

use super::{blocks, markdown};
use crate::arc42_file::{self, Arc42Files, Target};
use crate::tab_support::{self, Texts};
use crate::ui::{AppWindow, ArchitectureData, ArchitectureModel};

/// The project's arc42 text and the open chapter editor, if any.
pub struct Editor {
    root: PathBuf,
    /// `None` until the user chose where to create it.
    files: Option<Arc42Files>,
    /// Why the project's arc42 text could not be read.
    files_error: String,
    open: Option<Open>,
}

/// An open chapter editor.
struct Open {
    chapter: u8,
    /// No arc42 text in the project yet: the user is asked where to put it.
    ask_location: bool,
    error: String,
}

impl Editor {
    /// Find and read the project's arc42 text.
    pub fn new(root: &Path) -> Self {
        let mut editor = Editor {
            root: root.to_path_buf(),
            files: None,
            files_error: String::new(),
            open: None,
        };
        if let Some(target) = arc42_file::find(root) {
            match Arc42Files::load(target) {
                Ok(f) => editor.files = Some(f),
                Err(e) => editor.files_error = e,
            }
        }
        editor
    }

    pub fn is_open(&self) -> bool {
        self.open.is_some()
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
        // The file the open chapter is saved to; the editor shows it.
        m.file = match (&self.files, &self.open) {
            (Some(f), Some(e)) => {
                let path = f.path_of(e.chapter);
                let shown = path.strip_prefix(&self.root).unwrap_or(&path);
                shown.display().to_string().into()
            }
            _ => SharedString::new(),
        };
        m.file_error = self.files_error.as_str().into();
        if let Some(e) = &self.open {
            m.editing = true;
            m.ask_location = e.ask_location;
            m.edit_error = e.error.as_str().into();
        }
    }

    pub fn start(&mut self, ui: &AppWindow, doc: &Arc42Document, chapter: u8) {
        let text = self.chapter(doc, chapter);
        self.open = Some(Open {
            chapter,
            ask_location: false,
            error: String::new(),
        });
        ui.global::<ArchitectureData>()
            .set_draft(text.as_str().into());
        self.preview(ui, &text);
    }

    pub fn cancel(&mut self) {
        self.open = None;
    }

    /// Show the draft as the reader would.
    pub fn preview(&self, ui: &AppWindow, text: &str) {
        let Some(e) = &self.open else { return };
        let body = blocks::chapter_body(text, e.chapter, false);
        let note = Texts::of(ui)
            .t("Diagram source · this diagram syntax is not supported in the viewer.")
            .to_string();
        let rows = blocks::ui_blocks(&markdown::parse(&body), &note);
        ui.global::<ArchitectureData>()
            .set_preview(tab_support::model(rows));
    }

    /// Save the draft; the path written, or `None` while the editor stays
    /// open (an error, or the question where to create the file).
    pub fn save(&mut self, ui: &AppWindow, doc: &Arc42Document) -> Option<String> {
        let edit = self.open.as_mut()?;
        let Some(files) = &mut self.files else {
            edit.ask_location = true;
            return None;
        };
        let draft = ui.global::<ArchitectureData>().get_draft();
        match files.save_chapter(doc, edit.chapter, &draft) {
            Ok(path) => {
                self.files_error.clear();
                self.open = None;
                Some(path.display().to_string())
            }
            Err(e) => {
                edit.error = e;
                None
            }
        }
    }

    /// The answer to "where should the arc42 text go": 0 `arc42.md` in the
    /// project folder (as `archguard doc --arc42`), 1 another file in the
    /// project, else cancel. Saves into the chosen file.
    pub fn choose(&mut self, ui: &AppWindow, doc: &Arc42Document, choice: i32) -> Option<String> {
        let edit = self.open.as_mut()?;
        edit.ask_location = false;
        let path = match choice {
            0 => self.root.join("arc42.md"),
            1 => ask_path(ui, &self.root)?,
            _ => return None,
        };
        if !path.starts_with(&self.root) {
            edit.error = Texts::of(ui)
                .t("Choose a place inside the project folder, so the app finds the file next time.")
                .to_string();
            return None;
        }
        match Arc42Files::load(Target::File(path)) {
            Ok(f) => self.files = Some(f),
            Err(e) => {
                edit.error = e;
                return None;
            }
        }
        self.save(ui, doc)
    }

    /// `ARCHGUARD_DESKTOP_ARC_EDIT=1` opens the chapter editor, `=save`
    /// also saves it (asking where, in a project without arc42 text).
    #[cfg(feature = "snapshot")]
    pub fn snapshot(&mut self, ui: &AppWindow, doc: &Arc42Document, chapter: u8) -> Option<String> {
        let mode = std::env::var("ARCHGUARD_DESKTOP_ARC_EDIT").ok()?;
        self.start(ui, doc, chapter);
        (mode == "save").then(|| self.save(ui, doc)).flatten()
    }

    #[cfg(not(feature = "snapshot"))]
    pub fn snapshot(&mut self, _: &AppWindow, _: &Arc42Document, _: u8) -> Option<String> {
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
