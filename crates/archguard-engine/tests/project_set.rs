use archguard_core::ArchguardError;
use archguard_engine::{ProjectSet, read_project_set, write_project_set};
use std::{fs, path::Path};

fn dirs(root: &Path, names: &[&str]) {
    for n in names {
        fs::create_dir_all(root.join(n)).unwrap();
    }
}
fn set_file(root: &Path, text: &str) -> std::path::PathBuf {
    let f = root.join("x.archguard-set.toml");
    fs::write(&f, text).unwrap();
    f
}
fn err(root: &Path, text: &str) -> String {
    let f = set_file(root, text);
    match read_project_set(&f) {
        Err(ArchguardError::Config(m)) => {
            assert!(m.contains("x.archguard-set.toml"), "names the file: {m}");
            m
        }
        other => panic!("expected Config error, got {other:?}"),
    }
}

#[test]
fn relative_entries_resolve_against_the_file_folder() {
    let t = tempfile::tempdir().unwrap();
    dirs(t.path(), &["infra", "adapter"]);
    let f = set_file(
        t.path(),
        "name = \"search\"\nprojects = [\"infra\", \"./adapter\"]\n",
    );
    let set = read_project_set(&f).unwrap();
    let base = t.path().canonicalize().unwrap();
    assert_eq!(set.name, "search");
    assert_eq!(set.projects, vec![base.join("infra"), base.join("adapter")]);
}

#[test]
fn absolute_entries_stay_absolute() {
    let t = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    dirs(t.path(), &["a"]);
    let abs = other.path().canonicalize().unwrap();
    let text = format!(
        "name = \"s\"\nprojects = [\"a\", {:?}]\n",
        abs.to_str().unwrap()
    );
    let set = read_project_set(&set_file(t.path(), &text)).unwrap();
    assert_eq!(set.projects[1], abs);
}

#[test]
fn write_then_read_round_trips_and_has_stable_format() {
    let t = tempfile::tempdir().unwrap();
    dirs(t.path(), &["a", "b"]);
    let base = t.path().canonicalize().unwrap();
    let set = ProjectSet {
        name: "my \"set\"".into(),
        projects: vec![base.join("a"), base.join("b")],
    };
    let f = t.path().join("out.archguard-set.toml");
    write_project_set(&f, &set).unwrap();
    assert_eq!(read_project_set(&f).unwrap(), set);
    let first = fs::read_to_string(&f).unwrap();
    assert!(first.starts_with("name = "), "{first}");
    assert!(first.contains("projects = ["), "{first}");
    write_project_set(&f, &set).unwrap();
    assert_eq!(fs::read_to_string(&f).unwrap(), first);
    let leftovers = fs::read_dir(t.path()).unwrap().count();
    assert_eq!(leftovers, 3, "a, b, out only; no temp file");
}

#[test]
fn write_rejects_an_invalid_set_and_keeps_the_old_file() {
    let t = tempfile::tempdir().unwrap();
    dirs(t.path(), &["a"]);
    let f = set_file(t.path(), "old");
    let set = ProjectSet {
        name: "s".into(),
        projects: vec![t.path().join("a")],
    };
    assert!(matches!(
        write_project_set(&f, &set),
        Err(ArchguardError::Config(_))
    ));
    assert_eq!(fs::read_to_string(&f).unwrap(), "old");
}

#[test]
fn not_toml() {
    let t = tempfile::tempdir().unwrap();
    err(t.path(), "name = = =");
}

#[test]
fn unknown_key() {
    let t = tempfile::tempdir().unwrap();
    dirs(t.path(), &["a", "b"]);
    err(
        t.path(),
        "name = \"s\"\nprojects = [\"a\", \"b\"]\nextra = 1\n",
    );
}

#[test]
fn missing_key() {
    let t = tempfile::tempdir().unwrap();
    err(t.path(), "name = \"s\"\n");
}

#[test]
fn empty_and_overlong_name() {
    let t = tempfile::tempdir().unwrap();
    dirs(t.path(), &["a", "b"]);
    err(t.path(), "name = \"  \"\nprojects = [\"a\", \"b\"]\n");
    err(
        t.path(),
        &format!(
            "name = \"{}\"\nprojects = [\"a\", \"b\"]\n",
            "n".repeat(201)
        ),
    );
    let ok = format!(
        "name = \"{}\"\nprojects = [\"a\", \"b\"]\n",
        "n".repeat(200)
    );
    read_project_set(&set_file(t.path(), &ok)).unwrap();
}

#[test]
fn control_characters_in_the_name() {
    let t = tempfile::tempdir().unwrap();
    dirs(t.path(), &["a", "b"]);
    err(t.path(), "name = \"a\\nb\"\nprojects = [\"a\", \"b\"]\n");
}

#[test]
fn project_count_bounds() {
    let t = tempfile::tempdir().unwrap();
    let names: Vec<String> = (0..17).map(|i| format!("p{i}")).collect();
    for n in &names {
        fs::create_dir(t.path().join(n)).unwrap();
    }
    let list = |k: usize| {
        names[..k]
            .iter()
            .map(|n| format!("{n:?}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    err(t.path(), "name = \"s\"\nprojects = []\n");
    err(
        t.path(),
        &format!("name = \"s\"\nprojects = [{}]\n", list(1)),
    );
    err(
        t.path(),
        &format!("name = \"s\"\nprojects = [{}]\n", list(17)),
    );
    read_project_set(&set_file(
        t.path(),
        &format!("name = \"s\"\nprojects = [{}]\n", list(2)),
    ))
    .unwrap();
    read_project_set(&set_file(
        t.path(),
        &format!("name = \"s\"\nprojects = [{}]\n", list(16)),
    ))
    .unwrap();
}

#[test]
fn entry_missing_or_a_file() {
    let t = tempfile::tempdir().unwrap();
    dirs(t.path(), &["a"]);
    fs::write(t.path().join("file"), "x").unwrap();
    err(t.path(), "name = \"s\"\nprojects = [\"a\", \"missing\"]\n");
    err(t.path(), "name = \"s\"\nprojects = [\"a\", \"file\"]\n");
}

#[test]
fn duplicate_after_canonicalisation() {
    let t = tempfile::tempdir().unwrap();
    dirs(t.path(), &["a", "b"]);
    err(t.path(), "name = \"s\"\nprojects = [\"a\", \"b/../a\"]\n");
}

#[cfg(unix)]
#[test]
fn symlink_entry_resolves_to_its_target_and_duplicates_are_caught() {
    let t = tempfile::tempdir().unwrap();
    dirs(t.path(), &["a", "b"]);
    std::os::unix::fs::symlink(t.path().join("a"), t.path().join("link")).unwrap();
    let ok = set_file(t.path(), "name = \"s\"\nprojects = [\"link\", \"b\"]\n");
    assert_eq!(
        read_project_set(&ok).unwrap().projects[0],
        t.path().canonicalize().unwrap().join("a")
    );
    err(t.path(), "name = \"s\"\nprojects = [\"a\", \"link\"]\n");
}

#[test]
fn file_over_64_kib_and_missing_file() {
    let t = tempfile::tempdir().unwrap();
    err(
        t.path(),
        &format!("name = \"s\"\n# {}\n", "x".repeat(64 * 1024)),
    );
    let m = match read_project_set(&t.path().join("none.archguard-set.toml")) {
        Err(ArchguardError::Config(m)) => m,
        other => panic!("{other:?}"),
    };
    assert!(m.contains("none.archguard-set.toml"));
}

#[test]
fn errors_never_echo_the_content() {
    let t = tempfile::tempdir().unwrap();
    let m = err(
        t.path(),
        "name = \"topsecretvalue\"\nprojects = [\"nope-dir\", \"x\"]\n",
    );
    assert!(!m.contains("topsecretvalue"));
}

#[test]
fn fixture_file_has_the_expected_content() {
    let f = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/es-set/search.archguard-set.toml");
    let text = fs::read_to_string(f).unwrap();
    assert!(text.contains("name = \"search\""));
    assert!(text.contains("projects = [\"infra\", \"adapter\"]"));
}

#[test]
fn empty_entry_is_rejected() {
    let t = tempfile::tempdir().unwrap();
    dirs(t.path(), &["a"]);
    err(t.path(), "name = \"s\"\nprojects = [\"\", \"a\"]\n");
}

#[test]
fn set_file_that_is_a_directory_fails_cleanly() {
    let t = tempfile::tempdir().unwrap();
    let d = t.path().join("d.archguard-set.toml");
    fs::create_dir(&d).unwrap();
    match read_project_set(&d) {
        Err(ArchguardError::Config(m)) => assert!(m.contains("not a regular file"), "{m}"),
        other => panic!("{other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn set_file_that_is_a_fifo_does_not_hang() {
    let t = tempfile::tempdir().unwrap();
    let p = t.path().join("f.archguard-set.toml");
    let c = std::ffi::CString::new(p.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc_mkfifo(c.as_ptr(), 0o600) }, 0);
    match read_project_set(&p) {
        Err(ArchguardError::Config(m)) => assert!(m.contains("not a regular file"), "{m}"),
        other => panic!("{other:?}"),
    }
}

#[cfg(unix)]
unsafe extern "C" {
    #[link_name = "mkfifo"]
    fn libc_mkfifo(path: *const std::ffi::c_char, mode: u32) -> i32;
}

#[cfg(unix)]
#[test]
fn symlinked_set_file_is_read_but_a_link_to_a_directory_is_refused() {
    let t = tempfile::tempdir().unwrap();
    dirs(t.path(), &["a", "b"]);
    let real = set_file(t.path(), "name = \"s\"\nprojects = [\"a\", \"b\"]\n");
    let link = t.path().join("link.archguard-set.toml");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    assert_eq!(read_project_set(&link).unwrap().name, "s");
    let dir_link = t.path().join("dirlink.archguard-set.toml");
    std::os::unix::fs::symlink(t.path().join("a"), &dir_link).unwrap();
    match read_project_set(&dir_link) {
        Err(ArchguardError::Config(m)) => assert!(m.contains("not a regular file"), "{m}"),
        other => panic!("{other:?}"),
    }
}
