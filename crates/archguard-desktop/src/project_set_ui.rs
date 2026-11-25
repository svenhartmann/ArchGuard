//! File → Compose projects… (ui/project_set_dialog.slint): the dialog's
//! model, writing the set file and opening it (ADR 0024).

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use archguard_engine::{ProjectSet, read_project_set, write_project_set};

    use super::{Block, Dialog, file_name, relative, write};
    use crate::documents_set::{Target, target};

    fn dir(root: &Path, name: &str) -> PathBuf {
        let d = root.join(name);
        std::fs::create_dir_all(&d).expect("dir");
        d.canonicalize().expect("canonical")
    }

    fn two() -> Dialog {
        let mut d = Dialog::new();
        d.name = "search".into();
        d.add("/p/a".into());
        d.add("/p/b".into());
        d
    }

    #[test]
    fn adding_skips_duplicates_and_keeps_order() {
        let mut d = Dialog::new();
        assert!(d.add("/p/a".into()));
        assert!(d.add("/p/b".into()));
        assert!(!d.add("/p/a".into()));
        assert_eq!(d.folders, [PathBuf::from("/p/a"), PathBuf::from("/p/b")]);
    }

    #[test]
    fn remove_and_up_move_the_primary_project() {
        let mut d = two();
        d.add("/p/c".into());
        d.up(2);
        assert_eq!(d.folders[1], PathBuf::from("/p/c"));
        d.up(1);
        assert_eq!(d.folders[0], PathBuf::from("/p/c"), "up from 1 = primary");
        d.up(0);
        assert_eq!(d.folders[0], PathBuf::from("/p/c"), "the first stays");
        d.remove(0);
        assert_eq!(d.folders, [PathBuf::from("/p/a"), PathBuf::from("/p/b")]);
        d.remove(9);
        assert_eq!(d.folders.len(), 2, "out of range is ignored");
    }

    #[test]
    fn saving_is_blocked_with_a_reason() {
        let mut d = Dialog::new();
        assert_eq!(d.block(), Some(Block::Name));
        d.name = "  ".into();
        assert_eq!(d.block(), Some(Block::Name));
        d.name = "search".into();
        assert_eq!(d.block(), Some(Block::TooFew));
        d.add("/p/a".into());
        assert_eq!(d.block(), Some(Block::TooFew));
        d.add("/p/b".into());
        assert_eq!(d.block(), None);
        d.folders.push("/p/a".into());
        assert_eq!(d.block(), Some(Block::Duplicate));
        d.folders = (0..17).map(|i| PathBuf::from(format!("/p/{i}"))).collect();
        assert_eq!(d.block(), Some(Block::TooMany));
        d.folders.truncate(16);
        assert_eq!(d.block(), None);
        d.name = "x".repeat(201);
        assert_eq!(d.block(), Some(Block::Name));
    }

    #[test]
    fn a_set_prefills_the_dialog() {
        let d = Dialog::from_set("search".into(), vec!["/p/a".into(), "/p/b".into()]);
        assert_eq!((d.name.as_str(), d.folders.len()), ("search", 2));
        assert_eq!(d.block(), None);
    }

    #[test]
    fn paths_are_relative_when_close_and_absolute_when_far() {
        let base = Path::new("/r/repo/sets");
        assert_eq!(relative(base, Path::new("/r/repo/infra")), Path::new("../infra"));
        assert_eq!(relative(base, Path::new("/r/repo/sets/x")), Path::new("x"));
        assert_eq!(relative(base, Path::new("/r/repo/a/b/c")), Path::new("../a/b/c"));
        let far = Path::new("/home/me/.config/archguard/sets");
        assert_eq!(relative(far, Path::new("/home/me/p/a")), Path::new("/home/me/p/a"));
    }

    #[test]
    fn the_file_name_is_the_set_name() {
        assert_eq!(file_name("Search"), "Search.archguard-set.toml");
        assert_eq!(file_name("a/b:c"), "a_b_c.archguard-set.toml");
    }

    #[test]
    fn saving_writes_what_write_project_set_writes_with_relative_paths() {
        let t = tempfile::tempdir().expect("temp");
        let root = t.path().canonicalize().expect("root");
        let (a, b) = (dir(&root, "a"), dir(&root, "b"));
        let sets = dir(&root, "sets");
        let mut d = Dialog::new();
        d.name = "search".into();
        d.add(a.clone());
        d.add(b.clone());
        let file = sets.join(file_name(&d.name));
        write(&d, &file).expect("saved");
        let expected = sets.join("expected.archguard-set.toml");
        let set = ProjectSet {
            name: "search".into(),
            projects: vec!["../a".into(), "../b".into()],
        };
        write_project_set(&expected, &set).expect("expected");
        assert_eq!(
            std::fs::read_to_string(&file).expect("file"),
            std::fs::read_to_string(&expected).expect("expected text")
        );
        assert_eq!(read_project_set(&file).expect("read").projects, [a, b]);
    }

    #[test]
    fn far_folders_are_written_absolute() {
        let t = tempfile::tempdir().expect("temp");
        let root = t.path().canonicalize().expect("root");
        let (a, b) = (dir(&root, "a"), dir(&root, "b"));
        let sets = dir(&root, "x/y/z/sets");
        let mut d = Dialog::new();
        d.name = "far".into();
        d.add(a.clone());
        d.add(b);
        let file = sets.join(file_name("far"));
        write(&d, &file).expect("saved");
        let text = std::fs::read_to_string(&file).expect("file");
        assert!(text.contains(a.to_str().expect("utf8")), "{text}");
    }

    #[test]
    fn a_saved_set_opens_as_a_document() {
        let t = tempfile::tempdir().expect("temp");
        let root = t.path().canonicalize().expect("root");
        let (a, b) = (dir(&root, "a"), dir(&root, "b"));
        let mut d = Dialog::new();
        d.name = "search".into();
        d.add(a.clone());
        d.add(b);
        let file = root.join(file_name(&d.name));
        write(&d, &file).expect("saved");
        let Ok(Target::Set {
            primary, name, ..
        }) = target(&file)
        else {
            panic!("a set");
        };
        assert_eq!((primary, name.as_str()), (a, "search"));
    }

    #[test]
    fn save_errors_name_the_problem_without_file_contents() {
        let t = tempfile::tempdir().expect("temp");
        let root = t.path().canonicalize().expect("root");
        let a = dir(&root, "a");
        let mut d = Dialog::new();
        d.name = "search".into();
        d.add(a);
        d.add(root.join("gone"));
        let e = write(&d, &root.join(file_name("search"))).expect_err("missing folder");
        assert!(e.contains("project 2 does not exist"), "{e}");
        assert!(!root.join(file_name("search")).exists());
    }
}
