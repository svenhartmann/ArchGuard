use std::collections::BTreeSet;

use archguard_engine::core::Config;

use super::super::catalogue::{self, FIELDS, Group, Kind};
use super::super::{ConfigEditor, FieldValue};
use super::fixture;

/// The scopes holding fields: the root, every section, every entry.
fn scopes() -> Vec<String> {
    let mut scopes = vec![String::new()];
    for f in FIELDS {
        match f.kind {
            Kind::Table => scopes.push(f.path.to_string()),
            Kind::TableArray => scopes.push(format!("{}[]", f.path)),
            _ => {}
        }
    }
    scopes
}

/// The field names `Config` accepts in `scope`, read from its
/// `deny_unknown_fields` error for an unknown key.
fn accepted(scope: &str) -> BTreeSet<String> {
    let header = match scope.strip_suffix("[]") {
        Some(array) => format!("[[{array}]]\n"),
        None if scope.is_empty() => String::new(),
        None => format!("[{scope}]\n"),
    };
    let err = toml::from_str::<Config>(&format!("{header}__probe__ = 1\n")).unwrap_err();
    let message = err.message().to_string();
    let expected = message
        .split_once("expected")
        .unwrap_or_else(|| panic!("{scope}: {message}"))
        .1;
    expected
        .split('`')
        .skip(1)
        .step_by(2)
        .map(String::from)
        .collect()
}

#[test]
fn catalogue_lists_every_field_config_accepts() {
    for scope in scopes() {
        let listed: BTreeSet<String> = catalogue::children(&scope)
            .map(|f| f.path[if scope.is_empty() { 0 } else { scope.len() + 1 }..].to_string())
            .collect();
        assert_eq!(listed, accepted(&scope), "fields of {scope:?}");
    }
}

/// Every key of the serialized default config, as a pattern path.
fn keys(table: &toml::Table, prefix: &str, out: &mut BTreeSet<String>) {
    for (key, value) in table {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        if let toml::Value::Table(inner) = value {
            keys(inner, &path, out);
        }
        out.insert(path);
    }
}

#[test]
fn catalogue_covers_the_serialized_default_config() {
    let default: toml::Table =
        toml::from_str(&toml::to_string(&Config::default()).unwrap()).unwrap();
    let mut serialized = BTreeSet::new();
    keys(&default, "", &mut serialized);
    let listed: BTreeSet<String> = FIELDS.iter().map(|f| f.path.to_string()).collect();
    let missing: Vec<_> = serialized.difference(&listed).collect();
    assert!(missing.is_empty(), "not in the catalogue: {missing:?}");
}

fn sample(kind: Kind) -> FieldValue {
    match kind {
        Kind::Bool => FieldValue::Bool(true),
        Kind::Int => FieldValue::Int(7),
        Kind::Float => FieldValue::Float(0.5),
        Kind::Text => FieldValue::Text("x".into()),
        Kind::Enum(allowed) => FieldValue::Text(allowed[0].into()),
        Kind::List => FieldValue::List(vec!["a/**".into()]),
        Kind::Map => FieldValue::Map(vec![("hubs".into(), 2.0)]),
        Kind::Table | Kind::TableArray => unreachable!(),
    }
}

#[test]
fn every_field_takes_a_value_of_its_kind() {
    let dir = fixture("");
    let mut editor = ConfigEditor::load(dir.path()).unwrap();
    for f in FIELDS {
        let path = f.path.replace("[]", "[0]");
        match f.kind {
            Kind::Table => {}
            Kind::TableArray => assert_eq!(editor.add_entry(&path), Ok(0), "{path}"),
            kind => editor.set(&path, sample(kind)).unwrap(),
        }
    }
    let text = editor.text();
    toml::from_str::<Config>(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
}

#[test]
fn every_allowed_enum_value_parses() {
    for f in FIELDS {
        let Kind::Enum(allowed) = f.kind else {
            continue;
        };
        for value in allowed {
            let dir = fixture("");
            let mut editor = ConfigEditor::load(dir.path()).unwrap();
            let path = f.path.replace("[]", "[0]");
            if let Some(array) = path.split_once("[0]").map(|(a, _)| a) {
                editor.add_entry(array).unwrap();
            }
            editor
                .set(&path, FieldValue::Text((*value).into()))
                .unwrap();
            toml::from_str::<Config>(&editor.text())
                .unwrap_or_else(|e| panic!("{path}={value}: {e}"));
        }
    }
}

#[test]
fn defaults_come_from_the_core_types() {
    let default = |path| super::super::defaults::of(catalogue::field(path).unwrap());
    assert_eq!(default("rules.max_scc_size"), Some(FieldValue::Int(1)));
    assert_eq!(
        default("rules.deny_cycles"),
        Some(FieldValue::Text("module".into()))
    );
    assert_eq!(default("derive_layers"), Some(FieldValue::Bool(true)));
    assert_eq!(
        default("project.source_roots"),
        Some(FieldValue::List(vec!["src".into()]))
    );
    assert_eq!(
        default("layers[].may_depend_on"),
        Some(FieldValue::List(vec![]))
    );
    assert_eq!(
        default("dependency_rules[].severity"),
        Some(FieldValue::Text("error".into()))
    );
    assert_eq!(
        default("architecture.model.severity"),
        Some(FieldValue::Text("error".into()))
    );
    assert_eq!(
        default("semantic.csharp.configuration"),
        Some(FieldValue::Text("Debug".into()))
    );
    assert_eq!(
        default("review.max_tangledness"),
        None,
        "optional gate is off"
    );
}

#[test]
fn every_group_has_fields_and_paths_resolve() {
    for group in Group::ALL {
        assert!(FIELDS.iter().any(|f| f.group == group), "{group:?}");
    }
    let unique: BTreeSet<_> = FIELDS.iter().map(|f| f.path).collect();
    assert_eq!(unique.len(), FIELDS.len(), "paths are unique");
    assert_eq!(
        catalogue::resolve("layers[2].contains[0]").unwrap().path,
        "layers[].contains"
    );
    assert_eq!(
        catalogue::resolve("layers[app].contains").unwrap().path,
        "layers[].contains"
    );
    assert_eq!(
        catalogue::resolve("review.health_weights[hubs]")
            .unwrap()
            .path,
        "review.health_weights"
    );
    assert!(catalogue::resolve("nonsense").is_none());
}
