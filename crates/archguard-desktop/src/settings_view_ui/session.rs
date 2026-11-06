//! The Settings view's state for one document: the bound
//! [`ConfigEditor`], what the user typed that is not (yet) a value, the
//! engine's validation and the edits per group since the last save.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;

use archguard_engine::core::Config;

use crate::config_editor::SaveError;
use crate::config_editor::{self, ConfigEditor, FIELDS, FieldError, Group, Kind};
use crate::documents::{self, DocId};

use super::preview_panel::PreviewStatus;

/// Input of a text field as typed; `error` when it is no value of the
/// field's kind (the file keeps its last value then).
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    pub raw: String,
    pub error: Option<String>,
}

pub struct SettingsState {
    pub(super) doc: DocId,
    root: PathBuf,
    /// The bound editor, or why the file could not be read.
    pub(super) editor: Result<Rc<RefCell<ConfigEditor>>, String>,
    /// The analysed configuration: the derived values.
    pub(super) effective: Option<Config>,
    pub(super) group: Group,
    pub(super) query: String,
    pub(super) drafts: BTreeMap<String, Draft>,
    pub(super) invalid: Option<FieldError>,
    /// Each group's values as last loaded or saved.
    baseline: BTreeMap<Group, String>,
    pub(super) text_mode: bool,
    /// The raw text while it does not parse, and the parse error.
    pub(super) text_draft: Option<Draft>,
    pub(super) changed_on_disk: bool,
    pub(super) save_error: String,
    pub(super) preview: PreviewStatus,
}

impl SettingsState {
    /// Load the configuration of `root` and bind it to document `doc`.
    pub fn open(doc: DocId, root: PathBuf, effective: Option<Config>) -> Self {
        let mut s = SettingsState {
            doc,
            root,
            editor: Err(String::new()),
            effective,
            group: Group::Project,
            query: String::new(),
            drafts: BTreeMap::new(),
            invalid: None,
            baseline: BTreeMap::new(),
            text_mode: false,
            text_draft: None,
            changed_on_disk: false,
            save_error: String::new(),
            preview: PreviewStatus::Idle,
        };
        s.reload();
        s
    }

    /// Read the file again, dropping every edit (Revert, Reload).
    pub fn reload(&mut self) {
        let loaded = ConfigEditor::load(&self.root);
        self.install(loaded);
        documents::mark_clean(self.doc);
    }

    /// Use `loaded` as the editor, keeping the `Rc` the save hook holds.
    fn install(&mut self, loaded: Result<ConfigEditor, String>) {
        self.drafts.clear();
        self.text_draft = None;
        self.changed_on_disk = false;
        self.save_error.clear();
        self.preview = PreviewStatus::Idle;
        self.editor = match (loaded, &self.editor) {
            (Ok(new), Ok(rc)) => {
                *rc.borrow_mut() = new;
                Ok(rc.clone())
            }
            (Ok(new), Err(_)) => Ok(Rc::new(RefCell::new(new))),
            (Err(e), _) => Err(e),
        };
        if let Ok(rc) = &self.editor {
            config_editor::bind(self.doc, rc);
            self.baseline = fingerprints(&rc.borrow());
        }
        self.revalidate();
    }

    pub(super) fn revalidate(&mut self) {
        self.invalid = self
            .editor
            .as_ref()
            .ok()
            .and_then(|rc| rc.borrow().validate().err());
    }

    /// The text pane: the whole file. Text that does not parse is kept
    /// with its error; the form keeps the last text that did.
    pub fn edit_text(&mut self, text: &str) {
        let Ok(rc) = &self.editor else { return };
        let result = rc.borrow_mut().set_text(text);
        self.text_draft = result.err().map(|e| Draft {
            raw: text.into(),
            error: Some(e.message),
        });
        if self.text_draft.is_none() {
            self.drafts.clear();
        }
        self.save_error.clear();
        self.revalidate();
    }

    /// Field errors plus the engine's validation problem.
    pub fn errors(&self) -> usize {
        let fields = self.drafts.values().filter(|d| d.error.is_some()).count();
        let text = usize::from(self.text_draft.is_some());
        fields + text + usize::from(self.invalid.is_some())
    }

    /// The file saving writes, relative to the project.
    pub fn file(&self) -> String {
        let Ok(rc) = &self.editor else {
            return String::new();
        };
        let editor = rc.borrow();
        let path = editor.path();
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .display()
            .to_string()
    }

    pub fn is_dirty(&self) -> bool {
        self.editor.as_ref().is_ok_and(|rc| rc.borrow().is_dirty())
    }

    /// The groups whose values differ from the file as loaded or saved.
    pub fn edited(&self) -> Vec<Group> {
        let Ok(rc) = &self.editor else {
            return Vec::new();
        };
        let now = fingerprints(&rc.borrow());
        Group::ALL
            .into_iter()
            .filter(|g| now.get(g) != self.baseline.get(g))
            .collect()
    }

    /// Save through the document, so it is marked clean in one place.
    pub fn save(&mut self) {
        if self.errors() > 0 {
            return;
        }
        match documents::save(self.doc) {
            Ok(()) => self.saved(),
            Err(e) if e == SaveError::ChangedOnDisk.to_string() => self.changed_on_disk = true,
            Err(e) => self.save_error = e,
        }
    }

    fn saved(&mut self) {
        self.changed_on_disk = false;
        self.save_error.clear();
        self.drafts.clear();
        if let Ok(rc) = &self.editor {
            self.baseline = fingerprints(&rc.borrow());
        }
    }

    /// Replace the file that changed on disk with the edited text.
    pub fn overwrite(&mut self) {
        let Ok(rc) = &self.editor else { return };
        let text = rc.borrow().text();
        let loaded = ConfigEditor::load(&self.root).and_then(|mut e| {
            e.set_text(&text).map_err(|e| e.to_string())?;
            Ok(e)
        });
        let baseline = std::mem::take(&mut self.baseline);
        self.install(loaded);
        // The edits are still unsaved against the file as it was.
        self.baseline = baseline;
        documents::mark_dirty(self.doc, "Configuration text");
        self.save();
    }
}

/// Each group's values and entries as one comparable text.
fn fingerprints(editor: &ConfigEditor) -> BTreeMap<Group, String> {
    let mut out: BTreeMap<Group, String> = BTreeMap::new();
    for f in editor.fields(None) {
        let text = out.entry(f.field.group).or_default();
        text.push_str(&format!("{}={:?}\n", f.path, f.value));
    }
    for array in FIELDS.iter().filter(|f| f.kind == Kind::TableArray) {
        let text = out.entry(array.group).or_default();
        text.push_str(&format!("{:?}\n", editor.entries(array.path, None)));
    }
    out
}
