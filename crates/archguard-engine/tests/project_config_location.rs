use std::fs;
use std::path::Path;

use archguard_engine::{Level, analyze_run_quiet, core::ArchguardError, load_config};

fn write(root: &Path, relative: &str, content: &[u8]) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

#[test]
fn project_local_config_precedes_legacy_root_config() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "archguard.toml", b"[modules]\ndepth = 3\n");
    write(
        dir.path(),
        ".archguard/archguard.toml",
        b"[modules]\ndepth = 7\n",
    );

    let loaded = load_config(None, dir.path()).unwrap();
    assert_eq!(loaded.config.modules.depth, 7);
    assert_eq!(
        loaded.sources["modules"],
        archguard_engine::core::ConfigSource::Declared
    );
}

#[test]
fn explicit_config_precedes_even_invalid_project_local_config() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "archguard.toml", b"invalid legacy TOML");
    write(dir.path(), ".archguard/archguard.toml", b"invalid TOML");
    write(dir.path(), "custom.toml", b"[modules]\ndepth = 5\n");

    let loaded = load_config(Some(&dir.path().join("custom.toml")), dir.path()).unwrap();
    assert_eq!(loaded.config.modules.depth, 5);
}

#[test]
fn legacy_root_config_remains_available_without_project_local_config() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join(".archguard")).unwrap();
    write(dir.path(), "archguard.toml", b"[modules]\ndepth = 3\n");
    assert_eq!(
        load_config(None, dir.path()).unwrap().config.modules.depth,
        3
    );
}

#[test]
fn invalid_project_local_config_never_falls_back_to_legacy() {
    for content in [
        b"invalid TOML".as_slice(),
        b"[modules]\ndepth = 'far'",
        &[0xff],
    ] {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "archguard.toml", b"[modules]\ndepth = 3\n");
        write(dir.path(), ".archguard/archguard.toml", content);
        let error = load_config(None, dir.path()).unwrap_err();
        assert!(matches!(error, ArchguardError::Config(_)), "{error:?}");
        assert!(error.to_string().contains(".archguard/archguard.toml"));
    }
}

#[test]
fn project_local_config_directory_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "archguard.toml", b"[modules]\ndepth = 3\n");
    fs::create_dir_all(dir.path().join(".archguard/archguard.toml")).unwrap();
    let error = load_config(None, dir.path()).unwrap_err();
    assert!(matches!(error, ArchguardError::Config(_)), "{error:?}");
    assert!(error.to_string().contains(".archguard/archguard.toml"));
}

#[test]
fn project_local_config_keeps_paths_relative_to_analysis_root() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        ".archguard/archguard.toml",
        b"derive_layers = false\n[project]\nsource_roots = ['application']\nlanguages = ['ts']\n[store]\ndb = '.archguard/custom.db'\n[rules]\nbaseline = '.archguard/accepted.json'\n[semantic.java]\nclasspath = ['lib/contracts.jar']\n[semantic.csharp]\nproject = 'application/App.csproj'\n",
    );
    write(dir.path(), "application/a.ts", b"import './b';\n");
    write(dir.path(), "application/b.ts", b"export const b = 1;\n");
    let loaded = load_config(None, dir.path()).unwrap();
    assert_eq!(loaded.config.project.source_roots, ["application"]);
    assert_eq!(loaded.config.store.db, ".archguard/custom.db");
    assert_eq!(loaded.config.rules.baseline, ".archguard/accepted.json");
    assert_eq!(loaded.config.semantic.java.classpath, ["lib/contracts.jar"]);
    assert_eq!(
        loaded.config.semantic.csharp.project.as_deref(),
        Some("application/App.csproj")
    );
    let run = analyze_run_quiet(dir.path(), &loaded, Level::File).unwrap();
    assert_eq!(run.graph.nodes.len(), 2);
    assert_eq!(run.graph.edges.len(), 1);
}

#[cfg(unix)]
#[test]
fn discovered_config_symlinks_are_rejected_without_echoing_their_target() {
    use std::os::unix::fs::symlink;

    let outside = tempfile::tempdir().unwrap();
    let secret = outside.path().join("archguard.toml");
    fs::write(&secret, "aws_secret_access_key = DUMMY_SECRET oops\n").unwrap();

    let file_link = tempfile::tempdir().unwrap();
    fs::create_dir(file_link.path().join(".archguard")).unwrap();
    symlink(&secret, file_link.path().join(".archguard/archguard.toml")).unwrap();
    let legacy_link = tempfile::tempdir().unwrap();
    symlink(&secret, legacy_link.path().join("archguard.toml")).unwrap();
    let directory_link = tempfile::tempdir().unwrap();
    symlink(outside.path(), directory_link.path().join(".archguard")).unwrap();

    for root in [&file_link, &legacy_link, &directory_link] {
        let error = load_config(None, root.path()).unwrap_err();
        assert!(matches!(error, ArchguardError::Config(_)), "{error:?}");
        assert!(!error.to_string().contains("DUMMY_SECRET"), "{error}");
    }

    // The operator chooses an explicit path, so it may be a link.
    let chosen = tempfile::tempdir().unwrap();
    write(chosen.path(), "real.toml", b"[modules]\ndepth = 5\n");
    symlink(
        chosen.path().join("real.toml"),
        chosen.path().join("link.toml"),
    )
    .unwrap();
    let loaded = load_config(Some(&chosen.path().join("link.toml")), chosen.path()).unwrap();
    assert_eq!(loaded.config.modules.depth, 5);
}
