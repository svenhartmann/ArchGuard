#![cfg(unix)]

use archguard_engine::{
    Level, analyze_run_quiet, load_config, read_snapshots, snapshot_scope,
    write_snapshot_with_scope,
};
use std::fs;
use std::os::unix::fs::symlink;

#[test]
fn snapshot_does_not_write_through_history_or_archguard_symlinks() {
    for redirect_history in [true, false] {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(root.path().join("a.ts"), "export const a=1;\n").unwrap();
        fs::write(outside.path().join("sentinel"), "unchanged").unwrap();
        let run = analyze_run_quiet(
            root.path(),
            &load_config(None, root.path()).unwrap(),
            Level::Module,
        )
        .unwrap();
        // Remove only the test-created cache to install either redirect before
        // exercising the snapshot writer. No user checkout is involved.
        fs::remove_dir_all(root.path().join(".archguard")).unwrap();
        if redirect_history {
            fs::create_dir(root.path().join(".archguard")).unwrap();
            symlink(outside.path(), root.path().join(".archguard/history")).unwrap();
        } else {
            symlink(outside.path(), root.path().join(".archguard")).unwrap();
        }
        let result = write_snapshot_with_scope(
            root.path(),
            &run.report,
            None,
            Some(&snapshot_scope(&run.config)),
        );
        let entries: Vec<_> = fs::read_dir(outside.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(
            entries,
            vec![std::ffi::OsString::from("sentinel")],
            "snapshot writing must not create files or directories through a symlink outside the analyzed root"
        );
        assert!(
            result.is_err(),
            "an unsafe snapshot destination must surface as an error"
        );
        assert_eq!(
            fs::read_to_string(outside.path().join("sentinel")).unwrap(),
            "unchanged"
        );
    }
}

#[test]
fn snapshot_reader_ignores_symlinks_out_of_the_project() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.ts"), "export const a=1;\n").unwrap();
    let run = analyze_run_quiet(
        root.path(),
        &load_config(None, root.path()).unwrap(),
        Level::Module,
    )
    .unwrap();
    write_snapshot_with_scope(root.path(), &run.report, None, None).unwrap();
    let history = root.path().join(".archguard/history");
    assert_eq!(read_snapshots(root.path()).len(), 1);

    let outside = tempfile::tempdir().unwrap();
    let real = fs::read_dir(&history).unwrap().next().unwrap().unwrap();
    let foreign = outside.path().join("foreign.json");
    fs::copy(real.path(), &foreign).unwrap();

    symlink(&foreign, history.join("zz-link.json")).unwrap();
    assert_eq!(
        read_snapshots(root.path()).len(),
        1,
        "a linked snapshot is not this project's history"
    );

    let linked_history = tempfile::tempdir().unwrap();
    fs::create_dir(linked_history.path().join(".archguard")).unwrap();
    symlink(
        outside.path(),
        linked_history.path().join(".archguard/history"),
    )
    .unwrap();
    assert!(read_snapshots(linked_history.path()).is_empty());

    let linked_state = tempfile::tempdir().unwrap();
    symlink(
        root.path().join(".archguard"),
        linked_state.path().join(".archguard"),
    )
    .unwrap();
    assert!(read_snapshots(linked_state.path()).is_empty());
}
