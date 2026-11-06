//! Default values, read from the core types' `Default` impls so they are
//! stated only there.

use archguard_engine::core::{
    ArchitectureConstraint, ArchitectureDecision, ArchitectureGoal, TargetArchitectureConfig,
};
use archguard_engine::core::{Config, DependencyRule, LayerConfig, OwnershipRule};
use serde::Serialize;
use toml_edit::{DocumentMut, Table};

use super::catalogue::Field;
use super::toml_path::{self, Seg};
use super::value::FieldValue;

/// The default of `field`; `None` for optional fields and for entry
/// fields without a default (they must be written).
pub fn of(field: &Field) -> Option<FieldValue> {
    if field.optional || !field.kind.is_value() {
        return None;
    }
    let (scope, rest) = split_scope(field.path);
    let table = scope_defaults(scope)?;
    let segs: Vec<Seg> = rest.split('.').map(|k| Seg::Key(k.into())).collect();
    FieldValue::from_item(field.kind, toml_path::get(&table, &segs)?)
}

/// `layers[].name` → (`layers[]`, `name`); `rules.max_acd` → (``, `rules.max_acd`);
/// `architecture.model.name` → (`architecture.model`, `name`).
fn split_scope(path: &str) -> (&str, &str) {
    if let Some(i) = path.rfind("[].") {
        return (&path[..i + 2], &path[i + 3..]);
    }
    match path.strip_prefix("architecture.model.") {
        Some(rest) => ("architecture.model", rest),
        None => ("", path),
    }
}

/// The serialized default of the struct behind `scope`.
fn scope_defaults(scope: &str) -> Option<Table> {
    match scope {
        "" => table(&Config::default()),
        "layers[]" => table(&LayerConfig::default()),
        "dependency_rules[]" => table(&DependencyRule::default()),
        "ownership_rules[]" => table(&OwnershipRule::default()),
        "architecture.goals[]" => table(&ArchitectureGoal::default()),
        "architecture.constraints[]" => table(&ArchitectureConstraint::default()),
        "architecture.decisions[]" => table(&ArchitectureDecision::default()),
        "architecture.model" => table(&TargetArchitectureConfig::default()),
        // Components and directions have required fields, no defaults.
        _ => None,
    }
}

/// `value` serialized as a TOML table.
pub fn table(value: &impl Serialize) -> Option<Table> {
    let text = toml::to_string(value).ok()?;
    Some(text.parse::<DocumentMut>().ok()?.as_table().clone())
}
