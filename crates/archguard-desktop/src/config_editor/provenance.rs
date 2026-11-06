//! What each field holds and where the value comes from: written in the
//! file, derived by the engine (`[project]` roots and languages,
//! `[modules].depth`, layers when `derive_layers` is on), or default.

use archguard_engine::core::{Config, LayerOrigin};
use toml_edit::{Item, Table};

use super::catalogue::{self, FIELDS, Field, Kind};
use super::defaults;
use super::document::ConfigEditor;
use super::toml_path;
use super::value::FieldValue;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    /// Written in the file.
    File,
    /// Not written; the engine derives it from the project.
    Derived,
    /// Not written; the documented default applies.
    Default,
}

/// One form row.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldState {
    /// Concrete path for [`ConfigEditor::set`]; for derived layer entries,
    /// which have no place in the file, the pattern (`layers[].name`).
    pub path: String,
    pub field: &'static Field,
    /// `None`: optional and not set, a derived value not yet analysed, or
    /// a written value of the wrong type (see `text`).
    pub value: Option<FieldValue>,
    /// What to show: the value, or the raw TOML when it has the wrong type.
    pub text: String,
    pub provenance: Provenance,
}

/// One entry of a table array.
#[derive(Debug, Clone, PartialEq)]
pub struct EntryState {
    /// Index in the file; `None` for a derived layer.
    pub index: Option<usize>,
    pub provenance: Provenance,
    pub fields: Vec<FieldState>,
}

impl ConfigEditor {
    /// Every value field outside table arrays, in catalogue order.
    /// `effective` is the config of the document's last analysis
    /// (`AnalysisRun::config`); it supplies the derived values.
    pub fn fields(&self, effective: Option<&Config>) -> Vec<FieldState> {
        let effective = effective.and_then(defaults::table);
        FIELDS
            .iter()
            .filter(|f| f.kind.is_value() && !f.path.contains("[]"))
            .map(|f| {
                let derived = derives(self.table(), f.path).then_some(effective.as_ref());
                state(self.table(), f, f.path.to_string(), derived)
            })
            .collect()
    }

    /// The entries of the table array `array` (`layers`,
    /// `architecture.model.components`). For `layers`, the derived layers
    /// of `effective` follow the written ones while derivation is on.
    pub fn entries(&self, array: &str, effective: Option<&Config>) -> Vec<EntryState> {
        let Ok(segs) = toml_path::parse(array) else {
            return Vec::new();
        };
        let written = toml_path::get(self.table(), &segs).and_then(Item::as_array_of_tables);
        let mut out: Vec<EntryState> = (0..written.map_or(0, |a| a.len()))
            .map(|i| EntryState {
                index: Some(i),
                provenance: Provenance::File,
                fields: entry_fields(self.table(), array, i),
            })
            .collect();
        if array == "layers" && self.derives_layers() {
            out.extend(derived_layers(effective));
        }
        out
    }

    /// `derive_layers` as written, else its default.
    fn derives_layers(&self) -> bool {
        self.table()
            .get("derive_layers")
            .and_then(Item::as_bool)
            .unwrap_or_else(|| Config::default().derive_layers)
    }
}

/// Rows for the fields of entry `index` of `array`.
fn entry_fields(doc: &Table, array: &str, index: usize) -> Vec<FieldState> {
    let scope = format!("{}[]", catalogue::pattern_of(array));
    catalogue::children(&scope)
        .filter(|f| f.kind.is_value())
        .map(|f| {
            let name = &f.path[scope.len() + 1..];
            state(doc, f, format!("{array}[{index}].{name}"), None)
        })
        .collect()
}

/// The derived layers of `effective` (declared ones are in the file).
fn derived_layers(effective: Option<&Config>) -> Vec<EntryState> {
    let Some(config) = effective else {
        return Vec::new();
    };
    let derived = config
        .layers
        .iter()
        .filter(|l| l.origin == LayerOrigin::Derived);
    derived
        .filter_map(defaults::table)
        .map(|layer| EntryState {
            index: None,
            provenance: Provenance::Derived,
            fields: catalogue::children("layers[]")
                .map(|f| {
                    let item = layer.get(&f.path["layers[].".len()..]);
                    row(f, f.path.to_string(), item, Provenance::Derived)
                })
                .collect(),
        })
        .collect()
}

/// Whether the engine derives `path` when it is not written: it derives
/// whole sections only, and only when the section declares nothing.
fn derives(doc: &Table, path: &str) -> bool {
    let section = match path {
        "project.source_roots" | "project.languages" => "project",
        "modules.depth" => "modules",
        _ => return false,
    };
    !doc.get(section).is_some_and(declares_value)
}

/// The engine's rule: a header without keys declares nothing.
fn declares_value(item: &Item) -> bool {
    match item.as_table_like() {
        Some(table) => table.iter().any(|(_, item)| declares_value(item)),
        None => !item.is_none(),
    }
}

/// The row of `field` at `path` in `doc`; `derived` is `Some` when the
/// engine derives the field, holding the analysed config if known.
fn state(
    doc: &Table,
    field: &'static Field,
    path: String,
    derived: Option<Option<&Table>>,
) -> FieldState {
    let segs = toml_path::parse(&path).unwrap_or_default();
    if let Some(item) = toml_path::get(doc, &segs) {
        return row(field, path, Some(item), Provenance::File);
    }
    match derived {
        Some(effective) => {
            let item = effective.and_then(|t| toml_path::get(t, &segs));
            row(field, path, item, Provenance::Derived)
        }
        None => {
            let value = defaults::of(field);
            let text = value.as_ref().map(ToString::to_string).unwrap_or_default();
            FieldState {
                path,
                field,
                value,
                text,
                provenance: Provenance::Default,
            }
        }
    }
}

fn row(
    field: &'static Field,
    path: String,
    item: Option<&Item>,
    provenance: Provenance,
) -> FieldState {
    let value = item.and_then(|i| FieldValue::from_item(field.kind, i));
    let text = match (&value, item) {
        (Some(v), _) => v.to_string(),
        (None, Some(item)) if field.kind != Kind::Map => raw(item),
        _ => String::new(),
    };
    FieldState {
        path,
        field,
        value,
        text,
        provenance,
    }
}

/// The TOML of a value without its surrounding whitespace and comments.
fn raw(item: &Item) -> String {
    match item.as_value() {
        Some(value) => {
            let mut value = value.clone();
            value.decor_mut().clear();
            value.to_string()
        }
        None => item.to_string().trim().to_string(),
    }
}
