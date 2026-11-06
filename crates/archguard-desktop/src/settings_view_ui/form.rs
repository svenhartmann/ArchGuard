//! The form as the view shows it: the selected group's fields from
//! [`ConfigEditor::fields`] and its table arrays from
//! [`ConfigEditor::entries`], flat, with drafts and errors on their rows;
//! with a search, the matching fields of every group.

use std::cell::RefCell;
use std::collections::HashMap;

use slint::{ModelRc, SharedString};

use crate::config_editor::{
    ConfigEditor, EntryState, FIELDS, Field, FieldError, FieldState, FieldValue, Group, Kind,
    Provenance, catalogue,
};
use crate::tab_support::model;
use crate::ui::{SettingsGroupItem, SettingsItem};

use super::session::SettingsState;

pub(super) const HEADING: i32 = 0;
pub(super) const FIELD: i32 = 1;
pub(super) const ARRAY: i32 = 2;
pub(super) const ENTRY: i32 = 3;
pub(super) const NOTE: i32 = 4;

thread_local! {
    /// One choice model per enum, so rows compare equal across pushes.
    static CHOICES: RefCell<HashMap<usize, ModelRc<SharedString>>> = RefCell::new(HashMap::new());
}

impl SettingsState {
    /// The items of the form, and whether the validation problem shows
    /// on one of them.
    pub fn items(&self) -> (Vec<SettingsItem>, bool) {
        let Ok(rc) = &self.editor else {
            return (Vec::new(), false);
        };
        let editor = rc.borrow();
        let rows: HashMap<String, FieldState> = editor
            .fields(self.effective.as_ref())
            .into_iter()
            .map(|f| (f.path.clone(), f))
            .collect();
        let query = self.query.trim().to_lowercase();
        let mut out = Vec::new();
        for group in Group::ALL {
            if query.is_empty() && group != self.group {
                continue;
            }
            let start = out.len();
            for f in FIELDS.iter().filter(|f| f.group == group) {
                self.push_field(&mut out, &editor, &rows, f, &query);
            }
            if !query.is_empty() && out.len() > start {
                out.insert(start, heading(group.label()));
            }
        }
        let placed = self.place_error(&mut out);
        (out, placed)
    }

    /// The items of catalogue field `f`, if it is shown.
    fn push_field(
        &self,
        out: &mut Vec<SettingsItem>,
        editor: &ConfigEditor,
        rows: &HashMap<String, FieldState>,
        f: &'static Field,
        query: &str,
    ) {
        let in_array = f.path.contains("[]");
        match f.kind {
            Kind::TableArray if query.is_empty() || array_matches(f, query) => {
                out.push(array_head(f));
                self.push_entries(out, &editor.entries(f.path, self.effective.as_ref()), f);
            }
            // Nested sections (`semantic.java`); top-level ones are the group.
            Kind::Table if query.is_empty() && f.path.contains('.') => out.push(heading(f.path)),
            _ if in_array || !f.kind.is_value() || !matches(f, query) => {}
            _ => out.extend(rows.get(f.path).map(|s| self.field_item(s, false))),
        }
    }

    fn push_entries(&self, out: &mut Vec<SettingsItem>, entries: &[EntryState], array: &Field) {
        if entries.is_empty() {
            out.push(SettingsItem {
                kind: NOTE,
                path: array.path.into(),
                ..Default::default()
            });
        }
        let written = entries.iter().filter(|e| e.index.is_some()).count();
        for entry in entries {
            out.push(entry_head(array, entry, written));
            out.extend(entry.fields.iter().map(|s| self.field_item(s, true)));
        }
    }

    /// One field row, showing what the user typed when that is pending.
    fn field_item(&self, s: &FieldState, nested: bool) -> SettingsItem {
        let draft = self.drafts.get(&s.path);
        let widget = widget(s.field.kind);
        let typed = draft.filter(|_| widget == 1 || widget == 2 || widget >= 4);
        let read_only = s.provenance == Provenance::Derived;
        let (options, option) = choices(s);
        SettingsItem {
            kind: FIELD,
            path: s.path.as_str().into(),
            label: last_segment(&s.path).into(),
            description: s.field.description.into(),
            widget,
            text: typed.map_or(s.text.as_str(), |d| d.raw.as_str()).into(),
            checked: s.value == Some(FieldValue::Bool(true)),
            options,
            option,
            provenance: provenance(s.provenance),
            read_only,
            can_reset: s.provenance == Provenance::File,
            error: draft.and_then(|d| d.error.as_deref()).unwrap_or("").into(),
            nested,
            index: -1,
            can_up: false,
            can_down: false,
        }
    }

    /// Put the engine's validation problem on the item it names (the
    /// longest matching path); whether one does.
    fn place_error(&self, items: &mut [SettingsItem]) -> bool {
        let Some(e) = &self.invalid else { return false };
        let path = error_path(e);
        let owner = items
            .iter_mut()
            .filter(|i| i.kind != HEADING && i.kind != NOTE)
            .map(|i| (entry_path(i), i))
            .filter(|(p, _)| owns(p, path))
            .max_by_key(|(p, _)| p.len());
        let Some((_, item)) = owner else { return false };
        // A field's own input error is the more specific one.
        if item.error.is_empty() {
            item.error = e.message.as_str().into();
        }
        true
    }

    /// The group list: edits and errors per group, matches of the search.
    pub fn groups(&self) -> Vec<SettingsGroupItem> {
        let edited = self.edited();
        let query = self.query.trim().to_lowercase();
        let failed = |g: Group| {
            let field = |p: &str| catalogue::resolve(p).map(|f| f.group);
            self.invalid
                .as_ref()
                .is_some_and(|e| field(error_path(e)) == Some(g))
                || self
                    .drafts
                    .iter()
                    .any(|(p, d)| d.error.is_some() && field(p) == Some(g))
        };
        Group::ALL
            .into_iter()
            .map(|g| SettingsGroupItem {
                label: g.label().into(),
                edited: edited.contains(&g),
                failed: failed(g),
                matches: if query.is_empty() {
                    -1
                } else {
                    FIELDS
                        .iter()
                        .filter(|f| f.group == g && f.kind.is_value() && matches(f, &query))
                        .count() as i32
                },
            })
            .collect()
    }
}

/// The path an error is about: its own, or for messages such as
/// `dependency_rules[0].to must contain …` (no `path: ` prefix, so the
/// core keeps no path) the message's first word.
fn error_path(e: &FieldError) -> &str {
    if !e.path.is_empty() {
        return &e.path;
    }
    let first = e.message.split_whitespace().next().unwrap_or("");
    if catalogue::resolve(first).is_some() {
        first
    } else {
        ""
    }
}

/// The path an item stands for: an entry's `array[i]`.
fn entry_path(i: &SettingsItem) -> String {
    if i.kind == ENTRY {
        format!("{}[{}]", i.path, i.index)
    } else {
        i.path.to_string()
    }
}

/// Whether an error at `error` belongs to the item at `path`.
fn owns(path: &str, error: &str) -> bool {
    error == path
        || error
            .strip_prefix(path)
            .is_some_and(|rest| rest.starts_with('[') || rest.starts_with('.'))
}

fn heading(label: &str) -> SettingsItem {
    SettingsItem {
        kind: HEADING,
        label: label.into(),
        ..Default::default()
    }
}

fn array_head(f: &Field) -> SettingsItem {
    SettingsItem {
        kind: ARRAY,
        path: f.path.into(),
        label: last_segment(f.path).into(),
        description: f.description.into(),
        index: -1,
        ..Default::default()
    }
}

fn entry_head(array: &Field, e: &EntryState, written: usize) -> SettingsItem {
    let index = e.index.map_or(-1, |i| i as i32);
    // The entry's name, id or first text value names the card.
    let title = e
        .fields
        .iter()
        .find(|f| matches!(f.field.kind, Kind::Text | Kind::Enum(_)) && !f.text.is_empty())
        .map(|f| f.text.as_str())
        .unwrap_or("");
    SettingsItem {
        kind: ENTRY,
        path: array.path.into(),
        text: title.into(),
        provenance: provenance(e.provenance),
        read_only: e.index.is_none(),
        index,
        can_up: index > 0,
        can_down: e.index.is_some_and(|i| i + 1 < written),
        ..Default::default()
    }
}

fn matches(f: &Field, query: &str) -> bool {
    query.is_empty()
        || f.path.to_lowercase().contains(query)
        || f.description.to_lowercase().contains(query)
}

/// A table array is shown when it or one of its entry fields matches.
fn array_matches(array: &Field, query: &str) -> bool {
    let scope = format!("{}[].", array.path);
    FIELDS
        .iter()
        .filter(|f| f.path == array.path || f.path.starts_with(&scope))
        .any(|f| matches(f, query))
}

fn last_segment(path: &str) -> &str {
    path.rsplit('.').next().unwrap_or(path)
}

fn provenance(p: Provenance) -> i32 {
    match p {
        Provenance::File => 1,
        Provenance::Default => 2,
        Provenance::Derived => 3,
    }
}

/// The widget of `settings_parts.slint` for a kind.
fn widget(kind: Kind) -> i32 {
    match kind {
        Kind::Bool => 0,
        Kind::Int => 1,
        Kind::Float => 2,
        Kind::Enum(_) => 3,
        Kind::Text | Kind::Table | Kind::TableArray => 4,
        Kind::List => 5,
        Kind::Map => 6,
    }
}

/// The choices of an enum field and the selected one.
fn choices(s: &FieldState) -> (ModelRc<SharedString>, i32) {
    let Kind::Enum(allowed) = s.field.kind else {
        return (ModelRc::default(), -1);
    };
    let key = allowed.as_ptr() as usize;
    let options = CHOICES.with_borrow_mut(|c| {
        c.entry(key)
            .or_insert_with(|| model(allowed.iter().map(|v| SharedString::from(*v))))
            .clone()
    });
    let current = match &s.value {
        Some(FieldValue::Text(t)) => t.as_str(),
        _ => "",
    };
    let selected = allowed.iter().position(|v| *v == current);
    (options, selected.map_or(-1, |i| i as i32))
}
