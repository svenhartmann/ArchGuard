use std::{fs, process::Command};

#[test]
fn arc42_populates_twelve_chapters_and_preserves_manual_text_on_update() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("a.ts"), "import './b';\n").unwrap();
    fs::write(root.join("b.ts"), "export const b=1;\n").unwrap();
    fs::write(root.join("archguard.toml"), "derive_layers=false\n[project]\nsource_roots=[]\nlanguages=['ts']\n[[architecture.goals]]\nname='Modifiability'\npriority='high'\nrationale='Independent changes'\n[[architecture.decisions]]\nid='ADR-1'\ntitle='Separate modules'\nstatus='accepted'\nchosen_option='Dependency boundaries'\n").unwrap();
    let cli = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_archguard"))
            .current_dir(root)
            .args(args)
            .output()
            .unwrap()
    };
    let result = cli(&["doc", "--arc42"]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let target = root.join("arc42.md");
    let before = fs::read_to_string(&target).unwrap();
    assert_eq!(
        before
            .lines()
            .filter(|line| line.starts_with("## "))
            .count(),
        12
    );
    for text in [
        "Building Block View",
        "Modifiability",
        "ADR-1",
        "ACD",
        "Static",
        "CC BY-SA 4.0",
        "archguard:arc42:05:start",
    ] {
        assert!(before.contains(text), "missing {text}");
    }
    let manual = before.replace(
        "## 5. Building Block View",
        "## 5. Building Block View\n\nUnsere fachliche Erläuterung bleibt erhalten.",
    );
    fs::write(&target, &manual).unwrap();
    assert!(cli(&["doc", "--arc42", "--check"]).status.success());
    fs::write(root.join("c.ts"), "import './a';\n").unwrap();
    let stale = cli(&["doc", "--arc42", "--check"]);
    assert_eq!(stale.status.code(), Some(1));
    // The author must learn which chapters to rework, not only that one is off.
    assert!(
        String::from_utf8_lossy(&stale.stdout).contains("chapter 5"),
        "{}",
        String::from_utf8_lossy(&stale.stdout)
    );
    assert_eq!(fs::read_to_string(&target).unwrap(), manual);
    assert!(cli(&["doc", "--arc42"]).status.success());
    let updated = fs::read_to_string(&target).unwrap();
    assert!(updated.contains("Unsere fachliche Erläuterung bleibt erhalten."));
    assert_ne!(updated, manual);
    assert!(cli(&["doc", "--arc42", "--check"]).status.success());
    let chapter = cli(&["doc", "--arc42", "--chapter", "5", "--stdout"]);
    assert!(chapter.status.success());
    let chapter = String::from_utf8(chapter.stdout).unwrap();
    assert!(chapter.contains("## 5. Building Block View"));
    assert!(!chapter.contains("## 6."));
    assert!(cli(&["doc", "--arc42", "--chapter", "13"]).status.code() == Some(2));
}

#[test]
fn arc42_refuses_unmanaged_or_damaged_files_without_overwriting_them() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.ts"), "export const a=1;\n").unwrap();
    let cli = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_archguard"))
            .current_dir(dir.path())
            .args(args)
            .output()
            .unwrap()
    };
    let target = dir.path().join("arc42.md");
    let manual = "# Our existing arc42 document\nValuable manual text\n";
    fs::write(&target, manual).unwrap();
    assert!(!cli(&["doc", "--arc42"]).status.success());
    assert_eq!(fs::read_to_string(&target).unwrap(), manual);
    fs::remove_file(&target).unwrap();
    assert!(cli(&["doc", "--arc42"]).status.success());
    let broken = fs::read_to_string(&target)
        .unwrap()
        .replace("<!-- archguard:arc42:05:end -->", "");
    fs::write(&target, &broken).unwrap();
    assert!(!cli(&["doc", "--arc42"]).status.success());
    assert_eq!(fs::read_to_string(&target).unwrap(), broken);
}

#[test]
fn arc42_single_chapter_files_can_be_created_and_updated_independently() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.ts"), "export const a=1;").unwrap();
    let cli = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_archguard"))
            .current_dir(dir.path())
            .args(args)
            .output()
            .unwrap()
    };
    let args = ["doc", "--arc42", "--chapter", "5", "--out", "docs/05.md"];
    assert!(cli(&args).status.success());
    let path = dir.path().join("docs/05.md");
    let before = fs::read_to_string(&path).unwrap();
    assert!(before.contains("## 5."));
    assert!(!before.contains("## 6."));
    fs::write(&path, before + "\nOwn notes\n").unwrap();
    fs::write(dir.path().join("b.ts"), "import './a';").unwrap();
    assert!(cli(&args).status.success());
    assert!(
        fs::read_to_string(&path)
            .unwrap()
            .ends_with("\nOwn notes\n")
    );
    assert!(
        cli(&[
            "doc",
            "--arc42",
            "--chapter",
            "5",
            "--out",
            "docs/05.md",
            "--check"
        ])
        .status
        .success()
    );
}
#[test]
fn architecture_doc_check_detects_graph_drift_without_rewriting() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("a.ts"), "export const a=1;\n").unwrap();
    fs::write(
        root.join("archguard.toml"),
        "derive_layers=false\n[project]\nsource_roots=[]\nlanguages=['ts']\n",
    )
    .unwrap();
    let cli = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_archguard"))
            .current_dir(root)
            .args(args)
            .output()
            .unwrap()
    };
    let written = cli(&["doc"]);
    assert!(
        written.status.success(),
        "{}",
        String::from_utf8_lossy(&written.stderr)
    );
    let before = fs::read_to_string(root.join("architecture.md")).unwrap();
    assert!(before.contains("ACD"));
    assert!(cli(&["doc", "--check"]).status.success());
    fs::write(root.join("b.ts"), "import './a';\n").unwrap();
    assert_eq!(cli(&["doc", "--check"]).status.code(), Some(1));
    assert_eq!(
        fs::read_to_string(root.join("architecture.md")).unwrap(),
        before
    );
    assert!(cli(&["doc"]).status.success());
    assert!(cli(&["doc", "--check"]).status.success());
}

/// A committed `architecture.md -> ~/victim.txt` must not let `doc`
/// overwrite a file outside the project.
#[cfg(unix)]
#[test]
fn architecture_doc_refuses_to_write_through_a_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("a.ts"), "export const a=1;\n").unwrap();
    let victim = outside.path().join("victim.txt");
    fs::write(&victim, "precious\n").unwrap();
    std::os::unix::fs::symlink(&victim, root.join("architecture.md")).unwrap();
    std::os::unix::fs::symlink(&victim, root.join("chosen.md")).unwrap();
    for args in [&["doc"][..], &["doc", "--out", "chosen.md"][..]] {
        let result = Command::new(env!("CARGO_BIN_EXE_archguard"))
            .current_dir(root)
            .args(args)
            .output()
            .unwrap();
        assert!(!result.status.success(), "{args:?}");
        assert_eq!(
            fs::read_to_string(&victim).unwrap(),
            "precious\n",
            "{args:?}"
        );
    }
    assert!(
        fs::symlink_metadata(root.join("architecture.md"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    // A regular target is replaced as a whole, keeping no temporary file.
    fs::remove_file(root.join("architecture.md")).unwrap();
    fs::write(root.join("architecture.md"), "old\n").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_archguard"))
        .current_dir(root)
        .arg("doc")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(
        fs::read_to_string(root.join("architecture.md"))
            .unwrap()
            .contains("ACD")
    );
    let leftovers: Vec<_> = fs::read_dir(root)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

/// The web viewer draws and ticks the catalogue (ADR 0006), so the report
/// payload carries it next to the chapters.
#[test]
fn html_report_carries_the_arc42_diagram_catalogue() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("a.ts"), "import './b';\n").unwrap();
    fs::write(root.join("b.ts"), "export const b=1;\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_archguard"))
        .args(["graph", "--format", "html"])
        .arg(root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let html = String::from_utf8(output.stdout).unwrap();
    let payload = html
        .split("<script id=\"archguard-data\" type=\"application/json\">")
        .nth(1)
        .and_then(|rest| rest.split("</script>").next())
        .unwrap();
    let data: serde_json::Value = serde_json::from_str(payload).unwrap();
    let diagrams = data["insights"]["arc42"]["diagrams"].as_array().unwrap();
    let blocks = diagrams
        .iter()
        .find(|d| d["id"] == "building-blocks")
        .unwrap();
    assert_eq!(blocks["chapter"], 5);
    assert_eq!(blocks["shown"], true);
}

#[test]
fn arc42_check_accepts_a_current_document_the_desktop_wrote_in_german() {
    use archguard_engine::core::DocLang;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("a.ts"), "import './b';\n").unwrap();
    fs::write(root.join("b.ts"), "export const b=1;\n").unwrap();
    let loaded = archguard_engine::load_config(None, root).unwrap();
    let run =
        archguard_engine::analyze_run(root, &loaded, archguard_engine::Level::Module).unwrap();
    let mut config = run.config.clone();
    config.arc42.lang = DocLang::De;
    let german = archguard_engine::arc42_document(root, &config, &run.graph, &run.report).unwrap();
    let check = || {
        let out = Command::new(env!("CARGO_BIN_EXE_archguard"))
            .current_dir(root)
            .args(["doc", "--arc42", "--check"])
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stdout));
    };
    fs::write(root.join("arc42.md"), german.markdown(None)).unwrap();
    check();
    // ADR 0008: a German window saves one chapter into an English document.
    let english = archguard_engine::arc42_document(root, &run.config, &run.graph, &run.report)
        .unwrap()
        .markdown(None);
    let mixed = german.merge(&english, Some(5)).unwrap();
    assert_ne!(mixed, english);
    fs::write(root.join("arc42.md"), mixed).unwrap();
    check();
}

/// `doc --arc42 --format json` carries the chapters and the catalogue
/// the web viewer gets in its payload, without a browser.
#[test]
fn arc42_json_carries_the_chapters_and_the_diagram_catalogue() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::write(root.join("a.ts"), "import './b';\n").unwrap();
    fs::write(root.join("b.ts"), "export const b=1;\n").unwrap();
    let json = |args: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_archguard"))
            .current_dir(root)
            .args(["doc", "--arc42", "--format", "json"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()
    };
    let doc = json(&[]);
    assert_eq!(doc["chapters"].as_array().unwrap().len(), 12);
    assert_eq!(doc["chapters"][4]["number"], 5);
    let five = doc["chapters"][4]["markdown"].as_str().unwrap();
    assert!(five.starts_with("## 5."), "{five}");
    assert_eq!(doc["chapters"][4]["file"], serde_json::Value::Null);
    assert_eq!(doc["target"]["kind"], "file");
    assert_eq!(doc["target"]["exists"], false);
    let diagrams = doc["diagrams"].as_array().unwrap();
    let blocks = diagrams
        .iter()
        .find(|d| d["id"] == "building-blocks")
        .unwrap();
    assert_eq!(blocks["chapter"], 5);
    assert_eq!(blocks["kind"], "packages");
    assert_eq!(blocks["shown"], true);
    assert_eq!(blocks["chosen"], false);
    assert_eq!(blocks["in_text"], true, "the generated chapter has it");
    assert!(blocks["elements"].is_array() && blocks["relations"].is_array());
    assert!(!root.join("arc42.md").exists(), "json writes nothing");
    let only = json(&["--chapter", "5"]);
    assert_eq!(only["chapters"].as_array().unwrap().len(), 1);
    let only_diagrams = only["diagrams"].as_array().unwrap();
    assert!(only_diagrams.iter().all(|d| d["chapter"] == 5));
    // Own text without the diagram: found, and the diagram is not in it.
    fs::write(root.join("arc42.md"), "## 5. Bausteine\n\nUnser Text.\n").unwrap();
    let own = json(&["--chapter", "5"]);
    assert_eq!(own["target"]["exists"], true);
    let file = own["chapters"][0]["file"].as_str().unwrap();
    assert!(file.ends_with("arc42.md"), "{file}");
    assert_eq!(own["diagrams"][0]["in_text"], false);
}
