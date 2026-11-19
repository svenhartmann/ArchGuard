//! `doc --arc42` applies the desktop's rules for the project's arc42 text
//! (`archguard_engine::arc42_file`): it finds the document where the app
//! does, refreshes the diagram blocks "Insert into chapter" put into own
//! text, puts in every shown diagram, and exports the merged document.
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

/// Two TypeScript files, one importing the other.
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.ts"), "import './b';\n").unwrap();
    fs::write(dir.path().join("b.ts"), "export const b=1;\n").unwrap();
    dir
}

fn cli(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_archguard"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}

fn ok(output: &Output) -> String {
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Exit 1 with `expected` on stdout.
fn outdated(output: &Output, expected: &str) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(1),
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains(expected), "{stdout}");
}

/// `text` with chapter 5 (its heading up to `## 6.`) replaced by `five`.
fn with_chapter_five(text: &str, five: &str) -> String {
    let start = text.find("## 5.").unwrap();
    let end = text.find("## 6.").unwrap();
    format!("{}{five}\n{}", &text[..start], &text[end..])
}

const STALE_BLOCK: &str = "<!-- archguard:diagram:building-blocks:start -->\n### Old map\n\n```mermaid\nflowchart LR\n  old --> gone\n```\n<!-- archguard:diagram:building-blocks:end -->";

#[test]
fn a_stale_inserted_diagram_block_is_reported_and_refreshed() {
    let dir = project();
    let root = dir.path();
    ok(&cli(root, &["doc", "--arc42"]));
    let path = root.join("arc42.md");
    let own = format!("## 5. Building Block View\n\nOur own words.\n\n{STALE_BLOCK}\n");
    let text = with_chapter_five(&fs::read_to_string(&path).unwrap(), &own);
    fs::write(&path, &text).unwrap();
    outdated(&cli(root, &["doc", "--arc42", "--check"]), "chapter 5");
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        text,
        "check writes nothing"
    );
    ok(&cli(root, &["doc", "--arc42"]));
    let updated = fs::read_to_string(&path).unwrap();
    assert!(!updated.contains("old --> gone"), "{updated}");
    assert!(updated.contains("Our own words."));
    assert!(updated.contains("<!-- archguard:diagram:building-blocks:end -->"));
    ok(&cli(root, &["doc", "--arc42", "--check"]));
}

#[test]
fn a_moved_document_is_found_checked_and_updated_in_place() {
    let dir = project();
    let root = dir.path();
    ok(&cli(root, &["doc", "--arc42"]));
    fs::create_dir(root.join("docs")).unwrap();
    let moved = root.join("docs/arc42.md");
    let text = fs::read_to_string(root.join("arc42.md")).unwrap().replace(
        "## 5. Building Block View",
        "## 5. Building Block View\n\nOwn note.",
    );
    fs::write(&moved, text).unwrap();
    fs::remove_file(root.join("arc42.md")).unwrap();
    ok(&cli(root, &["doc", "--arc42", "--check"]));
    fs::write(root.join("c.ts"), "import './a';\n").unwrap();
    outdated(&cli(root, &["doc", "--arc42", "--check"]), "docs/arc42.md");
    ok(&cli(root, &["doc", "--arc42"]));
    assert!(!root.join("arc42.md").exists(), "no second document");
    assert!(fs::read_to_string(&moved).unwrap().contains("Own note."));
    ok(&cli(root, &["doc", "--arc42", "--check"]));
}

#[test]
fn a_chapter_folder_is_checked_and_updated_per_chapter_file() {
    let dir = project();
    let root = dir.path();
    let chapter = |n: &str| {
        let text = ok(&cli(root, &["doc", "--arc42", "--chapter", n, "--stdout"]));
        text[text.find("## ").unwrap()..].to_string()
    };
    let folder = root.join("docs/arc42");
    fs::create_dir_all(&folder).unwrap();
    let five = folder.join("05-bausteinsicht.md");
    fs::write(&five, chapter("5") + "\nEigene Bausteine.\n").unwrap();
    fs::write(folder.join("arc42-09.md"), chapter("9")).unwrap();
    ok(&cli(root, &["doc", "--arc42", "--check"]));
    fs::write(root.join("c.ts"), "import './a';\n").unwrap();
    outdated(&cli(root, &["doc", "--arc42", "--check"]), "chapter 5");
    let nine = fs::read_to_string(folder.join("arc42-09.md")).unwrap();
    let before = fs::read_to_string(&five).unwrap();
    ok(&cli(root, &["doc", "--arc42"]));
    assert!(!root.join("arc42.md").exists(), "no second document");
    let written = fs::read_to_string(&five).unwrap();
    assert!(written.ends_with("\nEigene Bausteine.\n"), "{written}");
    assert_ne!(written, before, "chapter 5 refreshed");
    assert_eq!(
        fs::read_to_string(folder.join("arc42-09.md")).unwrap(),
        nine
    );
    ok(&cli(root, &["doc", "--arc42", "--check"]));
}

#[test]
fn the_export_converts_the_found_document_with_its_own_text() {
    let dir = project();
    let root = dir.path();
    ok(&cli(root, &["doc", "--arc42"]));
    fs::create_dir(root.join("docs")).unwrap();
    let text = fs::read_to_string(root.join("arc42.md")).unwrap();
    let own = format!("## 5. Building Block View\n\nUnsere Notiz.\n\n{STALE_BLOCK}\n");
    fs::write(root.join("docs/arc42.md"), with_chapter_five(&text, &own)).unwrap();
    fs::remove_file(root.join("arc42.md")).unwrap();
    ok(&cli(root, &["doc", "--arc42", "--format", "asciidoc"]));
    let adoc = fs::read_to_string(root.join("arc42.adoc")).unwrap();
    assert!(adoc.contains("Unsere Notiz."), "{adoc}");
    assert!(!adoc.contains("old --> gone"), "current diagrams: {adoc}");
    // The export writes next to the project, never over the document.
    let refused = cli(
        root,
        &[
            "doc",
            "--arc42",
            "--format",
            "asciidoc",
            "--out",
            "docs/arc42.md",
        ],
    );
    assert_eq!(refused.status.code(), Some(2));
}

/// The document holds every shown diagram: an own-text chapter without
/// its diagram gets it as a diagram block, as a desktop save does.
#[test]
fn a_shown_diagram_missing_from_own_text_is_put_in() {
    let dir = project();
    let root = dir.path();
    ok(&cli(root, &["doc", "--arc42"]));
    let path = root.join("arc42.md");
    let own = "## 5. Building Block View\n\nOnly our words.\n";
    let text = with_chapter_five(&fs::read_to_string(&path).unwrap(), own);
    fs::write(&path, &text).unwrap();
    outdated(&cli(root, &["doc", "--arc42", "--check"]), "chapter 5");
    ok(&cli(root, &["doc", "--arc42"]));
    let updated = fs::read_to_string(&path).unwrap();
    let five = &updated[updated.find("## 5.").unwrap()..updated.find("## 6.").unwrap()];
    assert!(five.starts_with(own), "{five}");
    assert!(
        five.contains("<!-- archguard:diagram:building-blocks:start -->"),
        "{five}"
    );
    ok(&cli(root, &["doc", "--arc42", "--check"]));
    // A hidden diagram stays out.
    fs::write(
        root.join("archguard.toml"),
        "[arc42]\nhidden_diagrams = [\"building-blocks\"]\n",
    )
    .unwrap();
    ok(&cli(root, &["doc", "--arc42"]));
    let hidden = fs::read_to_string(&path).unwrap();
    assert!(
        !hidden.contains("archguard:diagram:building-blocks"),
        "{hidden}"
    );
}
