//! The form's edits on [`SettingsState`]: field values, resets and the
//! entries of table arrays, each followed by validation.

use crate::config_editor::{ConfigEditor, FieldError, FieldValue, Kind, catalogue};

use super::session::{Draft, SettingsState};

impl SettingsState {
    /// Run `f` on the editor, then validate; an edit the editor refuses
    /// is kept as the error of `path`.
    fn apply(&mut self, path: &str, f: impl FnOnce(&mut ConfigEditor) -> Result<(), FieldError>) {
        let Ok(rc) = &self.editor else { return };
        let result = f(&mut rc.borrow_mut());
        if let Err(e) = result {
            let draft = self.drafts.entry(path.to_string()).or_insert(Draft {
                raw: String::new(),
                error: None,
            });
            draft.error = Some(e.message);
        }
        self.save_error.clear();
        self.revalidate();
    }

    /// Text typed into the field at `path`: one line, or one entry per
    /// line for lists and maps. Empty text unsets an optional field.
    pub fn edit(&mut self, path: &str, raw: &str) {
        let Some(field) = catalogue::field(path) else {
            return;
        };
        let parsed = if raw.trim().is_empty() && field.optional {
            None
        } else {
            Some(FieldValue::parse(field.kind, raw))
        };
        let error = parsed.as_ref().and_then(|p| p.clone().err());
        let draft = Draft {
            raw: raw.into(),
            error,
        };
        self.drafts.insert(path.into(), draft);
        match parsed {
            None => self.apply(path, |e| e.unset(path)),
            Some(Ok(value)) => self.apply(path, |e| e.set(path, value)),
            Some(Err(_)) => self.revalidate(),
        }
    }

    /// A switch or a choice: the value directly.
    pub fn set(&mut self, path: &str, value: FieldValue) {
        self.drafts.remove(path);
        self.apply(path, |e| e.set(path, value));
    }

    /// The choice `index` of the enum field at `path`.
    pub fn choose(&mut self, path: &str, index: usize) {
        let allowed = match catalogue::field(path).map(|f| f.kind) {
            Some(Kind::Enum(allowed)) => allowed,
            _ => return,
        };
        if let Some(v) = allowed.get(index) {
            self.set(path, FieldValue::Text(v.to_string()));
        }
    }

    /// Remove the value from the file: the default or derivation applies.
    pub fn reset(&mut self, path: &str) {
        self.drafts.remove(path);
        self.apply(path, |e| e.unset(path));
    }

    /// Entry edits renumber the entries: their drafts go.
    fn entries_changed(&mut self, array: &str) {
        let prefix = format!("{array}[");
        self.drafts
            .retain(|p, _| p != array && !p.starts_with(&prefix));
    }

    pub fn add_entry(&mut self, array: &str) {
        self.entries_changed(array);
        self.apply(array, |e| e.add_entry(array).map(drop));
    }

    pub fn remove_entry(&mut self, array: &str, index: usize) {
        self.entries_changed(array);
        self.apply(array, |e| e.remove_entry(array, index));
    }

    pub fn move_entry(&mut self, array: &str, from: usize, to: usize) {
        self.entries_changed(array);
        self.apply(array, |e| e.move_entry(array, from, to));
    }
}
