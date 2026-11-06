use std::path::Path;

use archguard_engine::core::Config;
use archguard_engine::{Level, analyze_run_quiet, load_config};

use super::super::{ConfigEditor, FieldState, FieldValue, Provenance};
use super::{editor_with, fixture};

fn row<'a>(rows: &'a [FieldState], path: &str) -> &'a FieldState {
    rows.iter()
        .find(|r| r.path == path)
        .unwrap_or_else(|| panic!("{path}"))
}

/// The config an analysis of `root` ran with, as the app holds it.
fn effective(root: &Path) -> Config {
    let loaded = load_config(None, root).unwrap();
    analyze_run_quiet(root, &loaded, Level::Module)
        .unwrap()
        .config
}

#[test]
fn written_values_come_from_the_file_others_are_defaults() {
    let dir = fixture("");
    let editor = editor_with(
        dir.path(),
        "[rules]\nmax_scc_size = 4\n[review]\nmax_tangledness = \"high\"\n",
    );
    let rows = editor.fields(None);
    let written = row(&rows, "rules.max_scc_size");
    assert_eq!(
        (written.provenance, &written.value),
        (Provenance::File, &Some(FieldValue::Int(4)))
    );
    let default = row(&rows, "rules.deny_cycles");
    assert_eq!(default.provenance, Provenance::Default);
    assert_eq!(default.text, "module");
    let off = row(&rows, "review.max_propagation_cost");
    assert_eq!((off.provenance, &off.value), (Provenance::Default, &None));
    let wrong = row(&rows, "review.max_tangledness");
    assert_eq!((wrong.provenance, &wrong.value), (Provenance::File, &None));
    assert_eq!(
        wrong.text, "\"high\"",
        "raw TOML of a value of the wrong type"
    );
    assert!(
        rows.iter().all(|r| !r.path.contains('[')),
        "no entry fields"
    );
}

#[test]
fn unwritten_project_and_depth_are_derived() {
    let dir = fixture("ts-cyclic");
    let editor = ConfigEditor::load(dir.path()).unwrap();
    let rows = editor.fields(None);
    for path in ["project.source_roots", "project.languages", "modules.depth"] {
        let r = row(&rows, path);
        assert_eq!(
            (r.provenance, &r.value),
            (Provenance::Derived, &None),
            "{path}: not analysed yet"
        );
    }
    assert_eq!(
        row(&rows, "project.exclude").provenance,
        Provenance::Default
    );
    let effective = effective(dir.path());
    let rows = editor.fields(Some(&effective));
    let languages = row(&rows, "project.languages");
    assert_eq!(languages.provenance, Provenance::Derived);
    assert_eq!(
        languages.value,
        Some(FieldValue::List(effective.project.languages.clone()))
    );
    let depth = row(&rows, "modules.depth");
    assert_eq!(
        depth.value,
        Some(FieldValue::Int(effective.modules.depth.into()))
    );
}

#[test]
fn a_written_section_is_not_derived() {
    let dir = fixture("");
    // An empty header declares nothing, as in the engine.
    let editor = editor_with(
        dir.path(),
        "[project]\nexclude = [\"**/gen/**\"]\n[modules]\n",
    );
    let rows = editor.fields(None);
    assert_eq!(row(&rows, "project.exclude").provenance, Provenance::File);
    let roots = row(&rows, "project.source_roots");
    assert_eq!(
        (roots.provenance, roots.text.as_str()),
        (Provenance::Default, "src")
    );
    assert_eq!(row(&rows, "modules.depth").provenance, Provenance::Derived);
}

#[test]
fn derived_layers_follow_the_declared_ones() {
    let dir = fixture("ts-derive");
    let mut editor = editor_with(
        dir.path(),
        "[[layers]]\nname = \"core\"\ncontains = [\"core/**\"]\n",
    );
    let effective = effective(dir.path());
    let entries = editor.entries("layers", Some(&effective));
    assert_eq!(
        (entries[0].index, entries[0].provenance),
        (Some(0), Provenance::File)
    );
    assert_eq!(entries[0].fields[0].path, "layers[0].name");
    let derived: Vec<_> = entries
        .iter()
        .filter(|e| e.provenance == Provenance::Derived)
        .collect();
    assert!(!derived.is_empty(), "ts-derive derives layers: {entries:?}");
    assert!(derived.iter().all(|e| e.index.is_none()));
    assert!(
        derived[0]
            .fields
            .iter()
            .all(|f| f.provenance == Provenance::Derived)
    );
    editor
        .set("derive_layers", FieldValue::Bool(false))
        .unwrap();
    assert_eq!(
        editor.entries("layers", Some(&effective)).len(),
        1,
        "derivation switched off"
    );
}

#[test]
fn the_legacy_file_is_read_and_written_in_place() {
    let dir = fixture("py-layers");
    let editor = ConfigEditor::load(dir.path()).unwrap();
    assert_eq!(editor.path(), dir.path().join("archguard.toml"));
    let entries = editor.entries("layers", None);
    assert_eq!(entries.len(), 3);
    assert_eq!(
        entries[2].fields[2].value,
        Some(FieldValue::List(vec!["app".into()]))
    );
    let missing = fixture("ts-cyclic");
    let editor = ConfigEditor::load(missing.path()).unwrap();
    assert_eq!(
        editor.path(),
        missing.path().join(".archguard/archguard.toml")
    );
    assert_eq!(editor.text(), "");
}
