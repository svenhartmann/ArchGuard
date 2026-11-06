use std::cell::RefCell;
use std::fs;
use std::rc::Rc;

use super::super::{ConfigEditor, FieldValue, SaveError, bind};
use super::{editor_with, fixture};
use crate::documents;

fn leftovers(dir: &std::path::Path) -> Vec<String> {
    fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n != "archguard.toml")
        .collect()
}

#[test]
fn saving_without_a_file_creates_the_canonical_one() {
    let dir = fixture("ts-cyclic");
    let mut editor = ConfigEditor::load(dir.path()).unwrap();
    editor
        .set("rules.max_scc_size", FieldValue::Int(3))
        .unwrap();
    let written = editor.save().unwrap();
    assert_eq!(written, dir.path().join(".archguard/archguard.toml"));
    assert_eq!(
        fs::read_to_string(&written).unwrap(),
        "[rules]\nmax_scc_size = 3\n"
    );
    assert!(!editor.is_dirty());
    assert_eq!(
        leftovers(&dir.path().join(".archguard")),
        Vec::<String>::new(),
        "no temp file, no .bak"
    );
}

#[test]
fn saving_replaces_the_file_atomically_and_keeps_permissions() {
    let dir = fixture("");
    let mut editor = editor_with(dir.path(), "# keep\n[rules]\nmax_scc_size = 1\n");
    let path = editor.path().to_path_buf();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    }
    editor
        .set("rules.max_scc_size", FieldValue::Int(2))
        .unwrap();
    editor.save().unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "# keep\n[rules]\nmax_scc_size = 2\n"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }
    assert_eq!(leftovers(path.parent().unwrap()), Vec::<String>::new());
}

#[test]
fn invalid_or_concurrently_changed_files_are_not_written() {
    let dir = fixture("");
    let mut editor = editor_with(dir.path(), "[modules]\ndepth = 2\n");
    let path = editor.path().to_path_buf();
    editor
        .set("performance.min_barrel_exports", FieldValue::Int(0))
        .unwrap();
    assert!(
        matches!(editor.save(), Err(SaveError::Invalid(e)) if e.path == "performance.min_barrel_exports")
    );
    editor.unset("performance.min_barrel_exports").unwrap();
    editor.set("modules.depth", FieldValue::Int(3)).unwrap();
    fs::write(&path, "[modules]\ndepth = 4\n").unwrap();
    assert_eq!(editor.save(), Err(SaveError::ChangedOnDisk));
    assert_eq!(fs::read_to_string(&path).unwrap(), "[modules]\ndepth = 4\n");
}

#[test]
fn bind_marks_the_document_dirty_and_saves_through_it() {
    let dir = fixture("ts-cyclic");
    let (doc, _) = documents::with_store(|s| s.open(dir.path()));
    let editor = Rc::new(RefCell::new(ConfigEditor::load(dir.path()).unwrap()));
    bind(doc, &editor);
    editor
        .borrow_mut()
        .set("modules.depth", FieldValue::Int(1))
        .unwrap();
    assert!(documents::is_dirty(doc));
    documents::save(doc).unwrap();
    assert!(!documents::is_dirty(doc));
    let written = fs::read_to_string(dir.path().join(".archguard/archguard.toml")).unwrap();
    assert_eq!(written, "[modules]\ndepth = 1\n");
    editor
        .borrow_mut()
        .set("modules.depth", FieldValue::Int(2))
        .unwrap();
    editor
        .borrow_mut()
        .set("modules.depth", FieldValue::Int(1))
        .unwrap();
    assert!(!documents::is_dirty(doc), "back to the saved text");
}
