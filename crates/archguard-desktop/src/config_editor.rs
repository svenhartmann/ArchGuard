//! UI-free core of the `archguard.toml` editor (ADR 0002 decision 8).
//!
//! * [`catalogue`]: every field with path, kind, group and description.
//! * [`ConfigEditor`]: the project's file as a `toml_edit` document;
//!   [`ConfigEditor::fields`] and [`ConfigEditor::entries`] give each
//!   value with its [`Provenance`]; `set`/`unset` and the entry edits keep
//!   comments, formatting and unknown keys.
//! * [`ConfigEditor::validate`]: the engine's own checks, errors by path.
//! * [`preview::run`]: `archguard check` with the saved and the edited
//!   configuration, on a worker thread.
//! * [`ConfigEditor::save`] and [`bind`]: atomic save and the document's
//!   dirty/save hooks.
//!
//! Not wired into the window yet; the Settings view will use it.
#![allow(dead_code)]

pub mod catalogue;
mod defaults;
mod document;
mod error;
pub mod preview;
mod provenance;
mod save;
#[cfg(test)]
mod tests;
mod toml_path;
mod validate;
mod value;

// The public API; unused until the Settings view is wired.
#[allow(unused_imports)]
pub use {
    catalogue::{FIELDS, Field, Group, Kind},
    document::ConfigEditor,
    error::{FieldError, SaveError},
    preview::{CheckSummary, Preview, PreviewRequest},
    provenance::{EntryState, FieldState, Provenance},
    save::{bind, save_hook},
    value::FieldValue,
};
