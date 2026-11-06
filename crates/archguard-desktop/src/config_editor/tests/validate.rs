use super::super::FieldValue;
use super::{editor_with, fixture};

fn error(text: &str) -> (String, String) {
    let dir = fixture("");
    let err = editor_with(dir.path(), text).validate().unwrap_err();
    (err.path, err.message)
}

#[test]
fn unknown_keys_are_rejected_at_their_path() {
    let (path, message) = error("[rules]\nmax_scc_size = 1\nfoo = 1\n");
    assert_eq!(path, "rules.foo");
    assert!(message.contains("unknown field `foo`"), "{message}");
    assert_eq!(error("[custom]\nx = 1\n").0, "custom");
    assert_eq!(
        error("[[layers]]\nname = \"a\"\n\n[[layers]]\nname = \"b\"\nextra = 1\n").0,
        "layers[1].extra"
    );
}

#[test]
fn wrong_types_are_rejected_at_their_path() {
    assert_eq!(error("[modules]\ndepth = \"two\"\n").0, "modules.depth");
    let (path, message) = error("[[dependency_rules]]\nname = \"r\"\nseverity = \"fatal\"\n");
    assert_eq!(path, "dependency_rules[0].severity");
    assert!(message.contains("fatal"), "{message}");
}

#[test]
fn engine_checks_name_the_field() {
    let (path, message) = error("[performance]\nmin_barrel_exports = 0\n");
    assert_eq!(
        (path.as_str(), message.as_str()),
        (
            "performance.min_barrel_exports",
            "must be greater than zero"
        )
    );
    let dir = fixture("");
    let mut editor = editor_with(dir.path(), "");
    editor.add_entry("layers").unwrap();
    let err = editor.validate().unwrap_err();
    assert_eq!(err.path, "layers[0].name");
    assert_eq!(err.field().unwrap().path, "layers[].name");
    editor
        .set("layers[0].name", FieldValue::Text("core".into()))
        .unwrap();
    editor
        .set("rules.max_instability_core", FieldValue::Float(1.5))
        .unwrap();
    assert_eq!(
        editor.validate().unwrap_err().path,
        "rules.max_instability_core"
    );
    editor.unset("rules.max_instability_core").unwrap();
    let loaded = editor.validate().unwrap();
    assert_eq!(loaded.config.layers[0].name, "core");
}
