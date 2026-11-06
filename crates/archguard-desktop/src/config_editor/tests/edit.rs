use std::cell::RefCell;
use std::rc::Rc;

use super::super::{ConfigEditor, FieldValue};
use super::{editor_with, fixture};

const COMMENTED: &str = "\
# Team policy, keep this header.
derive_layers = true   # measured

[rules]
deny_cycles = \"module\"   # intent
max_scc_size = 1         # intent (Default)
house_rule = \"keep me\"   # not an ArchGuard key

[custom]
owner = \"someone\"
";

#[test]
fn set_keeps_comments_formatting_and_unknown_keys() {
    let dir = fixture("");
    let mut editor = editor_with(dir.path(), COMMENTED);
    editor
        .set("rules.max_scc_size", FieldValue::Int(3))
        .unwrap();
    editor
        .set("rules.deny_cycles", FieldValue::Text("file".into()))
        .unwrap();
    assert_eq!(
        editor.text(),
        COMMENTED
            .replace("max_scc_size = 1 ", "max_scc_size = 3 ")
            .replace("\"module\"", "\"file\"")
    );
}

#[test]
fn set_creates_missing_sections_without_empty_headers() {
    let dir = fixture("");
    let mut editor = ConfigEditor::load(dir.path()).unwrap();
    editor
        .set("semantic.java.release", FieldValue::Int(21))
        .unwrap();
    editor.set("rules.max_acd", FieldValue::Float(2.5)).unwrap();
    editor
        .set("derive_layers", FieldValue::Bool(false))
        .unwrap();
    assert_eq!(
        editor.text(),
        "derive_layers = false\n\n[semantic.java]\nrelease = 21\n\n[rules]\nmax_acd = 2.5\n"
    );
}

#[test]
fn unset_removes_only_that_key() {
    let dir = fixture("");
    let mut editor = editor_with(dir.path(), COMMENTED);
    editor.unset("rules.max_scc_size").unwrap();
    editor.unset("store.db").unwrap(); // not written: nothing to do
    let expected: String = COMMENTED
        .lines()
        .filter(|l| !l.starts_with("max_scc_size"))
        .map(|l| format!("{l}\n"))
        .collect();
    assert_eq!(editor.text(), expected);
}

#[test]
fn set_rejects_values_of_the_wrong_kind() {
    let dir = fixture("");
    let mut editor = ConfigEditor::load(dir.path()).unwrap();
    let err = editor
        .set("rules.deny_cycles", FieldValue::Text("sometimes".into()))
        .unwrap_err();
    assert_eq!(err.path, "rules.deny_cycles");
    assert!(err.message.contains("module, file, off"), "{err}");
    let err = editor
        .set("modules.depth", FieldValue::Text("two".into()))
        .unwrap_err();
    assert_eq!(err.path, "modules.depth");
    assert!(editor.set("rules.nonsense", FieldValue::Int(1)).is_err());
    assert_eq!(editor.text(), "", "nothing written");
}

#[test]
fn parse_reads_form_input() {
    use super::super::Kind;
    assert_eq!(FieldValue::parse(Kind::Int, " 4 "), Ok(FieldValue::Int(4)));
    assert!(FieldValue::parse(Kind::Int, "-1").is_err());
    assert_eq!(
        FieldValue::parse(Kind::List, "src/**\n\n lib/{a,b}/** "),
        Ok(FieldValue::List(vec![
            "src/**".into(),
            "lib/{a,b}/**".into()
        ]))
    );
    let map = FieldValue::Map(vec![("hubs".into(), 2.0), ("sdp".into(), 0.5)]);
    assert_eq!(FieldValue::parse(Kind::Map, &map.to_string()), Ok(map));
}

const LAYERS: &str = "\
derive_layers = false

# bottom
[[layers]]
name = \"domain\"

# middle
[[layers]]
name = \"app\"

[rules]
max_scc_size = 1

# top
[[layers]]
name = \"api\"
";

fn names(editor: &ConfigEditor) -> Vec<String> {
    editor
        .entries("layers", None)
        .into_iter()
        .map(|e| e.fields[0].text.clone())
        .collect()
}

#[test]
fn table_array_entries_move_with_their_comments() {
    let dir = fixture("");
    let mut editor = editor_with(dir.path(), LAYERS);
    editor.move_entry("layers", 2, 0).unwrap();
    assert_eq!(names(&editor), ["api", "domain", "app"]);
    let text = editor.text();
    assert!(
        text.starts_with(
            "derive_layers = false\n\n# top\n[[layers]]\nname = \"api\"\n\n# bottom\n"
        ),
        "{text}"
    );
    assert!(text.contains("[rules]\nmax_scc_size = 1\n"), "{text}");
    editor.move_entry("layers", 0, 2).unwrap();
    assert_eq!(editor.text(), LAYERS, "moving back restores the file");
}

#[test]
fn table_array_entries_add_and_remove() {
    let dir = fixture("");
    let mut editor = editor_with(dir.path(), LAYERS);
    editor.remove_entry("layers", 1).unwrap();
    assert_eq!(names(&editor), ["domain", "api"]);
    assert!(
        !editor.text().contains("# middle"),
        "comment leaves with its entry"
    );
    let index = editor.add_entry("layers").unwrap();
    assert_eq!(index, 2);
    editor
        .set("layers[2].name", FieldValue::Text("infra".into()))
        .unwrap();
    editor
        .set(
            "layers[2].may_depend_on",
            FieldValue::List(vec!["domain".into()]),
        )
        .unwrap();
    assert_eq!(names(&editor), ["domain", "api", "infra"]);
    assert!(
        editor.text().ends_with(
            "name = \"api\"\n\n[[layers]]\nname = \"infra\"\nmay_depend_on = [\"domain\"]\n"
        ),
        "{}",
        editor.text()
    );
    assert!(editor.remove_entry("layers", 9).is_err());
    assert!(editor.move_entry("layers", 0, 9).is_err());
}

#[test]
fn edits_report_dirty_until_the_text_is_back() {
    let dir = fixture("");
    let mut editor = editor_with(dir.path(), COMMENTED);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let log = Rc::clone(&seen);
    editor.on_change(move |dirty, what| log.borrow_mut().push((dirty, what.to_string())));
    assert!(!editor.is_dirty());
    editor
        .set("rules.max_scc_size", FieldValue::Int(2))
        .unwrap();
    editor
        .set("rules.max_scc_size", FieldValue::Int(1))
        .unwrap();
    assert_eq!(
        *seen.borrow(),
        [
            (true, "Rules and thresholds".to_string()),
            (false, "Rules and thresholds".to_string())
        ]
    );
    assert!(!editor.is_dirty());
}
