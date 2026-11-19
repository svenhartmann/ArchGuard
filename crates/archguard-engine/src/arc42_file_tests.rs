//! Tests of [`crate::arc42_file`], moved from the desktop app with the code.
use std::path::Path;

use archguard_core::DiagramFormat;
use archguard_report::arc42_file::diagram_block;
use archguard_report::{Arc42Chapter, Arc42Document};

use crate::arc42_file::*;

/// A diagram block in own text is written with the analysis' current
/// section; one the analysis no longer has stays as written.
#[test]
fn saving_puts_in_shown_diagrams_and_takes_out_hidden_ones() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("arc42.md");
    let mut new = doc("New");
    let diagram = |id: &str, shown| archguard_core::Diagram {
        id: id.into(),
        chapter: 5,
        title: id.into(),
        note: String::new(),
        kind: archguard_core::DiagramKind::Packages,
        unit: String::new(),
        elements: Vec::new(),
        relations: Vec::new(),
        shown,
    };
    new.diagrams = vec![diagram("shown", true), diagram("hidden", false)];
    let section = |d| archguard_report::diagram_section(d, DiagramFormat::Plantuml);
    let hidden = diagram_block("hidden", &section(&new.diagrams[1]));
    std::fs::write(
        &path,
        format!("## 5. Bausteine\n\nEigener Text.\n\n{hidden}\n\nDanach.\n"),
    )
    .unwrap();
    let mut files = Arc42Files::load(Target::File(path.clone())).unwrap();
    files.format = DiagramFormat::Plantuml;
    let chapter = files.chapter(&new, 5);
    files
        .save_chapter(&new, 5, &chapter, &mut files.saved(5))
        .unwrap();
    let shown = diagram_block("shown", &section(&new.diagrams[0]));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        format!("## 5. Bausteine\n\nEigener Text.\n\nDanach.\n\n{shown}\n"),
    );
}

/// A chapter of a file with Windows line ends reads with LF, as the
/// diagram sections it is compared with.
#[test]
fn a_chapter_reads_with_lf_line_ends() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("arc42.md");
    std::fs::write(&path, "## 5. Bausteine\r\n\r\nText.\r\n").unwrap();
    let files = Arc42Files::load(Target::File(path)).unwrap();
    assert_eq!(files.chapter(&doc("New"), 5), "## 5. Bausteine\n\nText.\n");
}

fn shown_diagram(id: &str) -> archguard_core::Diagram {
    archguard_core::Diagram {
        id: id.into(),
        chapter: 5,
        title: id.into(),
        note: String::new(),
        kind: archguard_core::DiagramKind::Packages,
        unit: String::new(),
        elements: Vec::new(),
        relations: Vec::new(),
        shown: true,
    }
}

/// A shown diagram the user removed from the chapter, or turned into
/// own text, stays out; saving names it, so the view can hide it.
#[test]
fn saving_keeps_a_removed_or_detached_diagram_out() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("arc42.md");
    let mut new = doc("New");
    new.diagrams = vec![shown_diagram("gone"), shown_diagram("own")];
    let section = |d| archguard_report::diagram_section(d, DiagramFormat::Mermaid);
    let gone = diagram_block("gone", &section(&new.diagrams[0]));
    let own = diagram_block("own", &section(&new.diagrams[1]));
    std::fs::write(
        &path,
        format!("## 5. Bausteine\n\nText.\n\n{gone}\n\n{own}\n"),
    )
    .unwrap();
    let mut files = Arc42Files::load(Target::File(path.clone())).unwrap();
    let edited = "## 5. Bausteine\n\nText.\n\n```mermaid\nflowchart LR\n  a --> b\n```\n";
    files
        .save_chapter(&new, 5, edited, &mut files.saved(5))
        .unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);
    assert_eq!(files.dropped, ["gone", "own"]);
}

#[test]
fn saving_refreshes_diagram_blocks_from_the_analysis() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("arc42.md");
    let mut new = doc("New");
    let diagram = archguard_core::Diagram {
        id: "calls".into(),
        chapter: 6,
        title: "Aufrufe".into(),
        note: "Neu.".into(),
        kind: archguard_core::DiagramKind::Packages,
        unit: String::new(),
        elements: Vec::new(),
        relations: Vec::new(),
        shown: true,
    };
    new.diagrams.push(diagram.clone());
    let old = diagram_block(
        "calls",
        "### Aufrufe\n\nAlt.\n\n```mermaid\nflowchart LR\n```",
    );
    let gone = diagram_block("gone", "### Weg\n\n```mermaid\nflowchart LR\n```");
    std::fs::write(&path, format!("## 6. Laufzeit\n\n{old}\n\n{gone}\n")).unwrap();
    let mut files = Arc42Files::load(Target::File(path.clone())).unwrap();
    let chapter = files.chapter(&new, 6);
    files
        .save_chapter(&new, 6, &chapter, &mut files.saved(6))
        .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let fresh = diagram_block(
        "calls",
        &archguard_report::diagram_section(&diagram, DiagramFormat::Mermaid),
    );
    assert!(text.contains(&fresh) && !text.contains("Alt."), "{text}");
    assert!(text.contains(&gone), "{text}");
}

fn doc(evidence: &str) -> Arc42Document {
    Arc42Document {
        schema_version: 1,
        lang: archguard_core::DocLang::De,
        scope: "shop".into(),
        notice: "Notice.\n",
        chapters: (1..=12)
            .map(|n| Arc42Chapter {
                number: n,
                title: "Title",
                markdown: format!("{evidence} {n}"),
                questions: vec![],
            })
            .collect(),
        diagrams: vec![],
    }
}

fn dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("temp dir")
}

#[test]
fn find_takes_the_shallowest_file_or_folder_and_skips_build_folders() {
    let d = dir();
    let root = d.path();
    assert_eq!(find(root), None, "nothing to find asks the user");
    std::fs::create_dir_all(root.join("target/arc42")).unwrap();
    std::fs::create_dir_all(root.join(".git/arc42")).unwrap();
    assert_eq!(
        find(root),
        None,
        "build and hidden folders are not searched"
    );
    std::fs::create_dir_all(root.join("docs/deep/arc42")).unwrap();
    assert_eq!(
        find(root),
        Some(Target::Folder(root.join("docs/deep/arc42")))
    );
    std::fs::write(root.join("docs/ARC42.md"), "").unwrap();
    assert_eq!(find(root), Some(Target::File(root.join("docs/ARC42.md"))));
    std::fs::create_dir(root.join("arc42")).unwrap();
    assert_eq!(find(root), Some(Target::Folder(root.join("arc42"))));
    std::fs::write(root.join("arc42/arc42.md"), "").unwrap();
    assert_eq!(
        find(root),
        Some(Target::File(root.join("arc42/arc42.md"))),
        "a folder that holds the whole document is that file"
    );
    std::fs::write(root.join("arc42.md"), "").unwrap();
    assert_eq!(find(root), Some(Target::File(root.join("arc42.md"))));
}

#[test]
fn a_file_shows_own_text_with_fresh_blocks_and_falls_back_per_chapter() {
    let d = dir();
    let path = d.path().join("arc42.md");
    let saved = doc("Old").markdown(None).replace(
        "<!-- Eigene Erläuterungen zu Kapitel 5 hier ergänzen. -->",
        "Unser Shop trennt Bestellung und Lager.",
    );
    let saved = saved.replace("\n## 7.", "\n## 7-removed");
    std::fs::write(&path, &saved).unwrap();
    let files = Arc42Files::load(Target::File(path)).unwrap();
    let new = doc("New");
    let five = files.chapter(&new, 5);
    assert!(five.starts_with("## 5. Title\n"), "{five}");
    assert!(five.contains("Unser Shop trennt Bestellung und Lager."));
    assert!(five.contains("New 5") && !five.contains("Old 5"), "{five}");
    assert!(!five.contains("## 6."), "one chapter only: {five}");
    assert!(files.preamble(&new).starts_with("# arc42"));
    // Chapter 7's heading is gone from the file: the generated one shows.
    assert!(
        files.chapter(&new, 6).contains("7-removed"),
        "6 runs to the next heading"
    );
    assert!(files.chapter(&new, 7).starts_with("## 7. Title"));
}

#[test]
fn saving_a_chapter_keeps_other_chapters_and_refreshes_blocks() {
    let d = dir();
    let path = d.path().join("arc42.md");
    let saved = doc("Old").markdown(None) + "\nAnhang des Teams.\n";
    std::fs::write(&path, saved.replace('\n', "\r\n")).unwrap();
    let mut files = Arc42Files::load(Target::File(path.clone())).unwrap();
    let new = doc("New");
    let edited = files
        .chapter(&new, 5)
        .replace("New 5", "edited inside the block")
        .replace(
            "<!-- Eigene Erläuterungen zu Kapitel 5 hier ergänzen. -->",
            "Eigene Notiz zu Bausteinen.",
        );
    assert_eq!(
        files.save_chapter(&new, 5, &edited, &mut files.saved(5)),
        Ok(path.clone())
    );
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.replace("\r\n", "").contains('\n'), "CRLF stays");
    let text = text.replace("\r\n", "\n");
    assert!(text.contains("Eigene Notiz zu Bausteinen."));
    assert!(text.contains("New 5") && !text.contains("edited inside the block"));
    assert!(
        text.contains("Old 4") && text.contains("Old 6"),
        "other chapters untouched"
    );
    assert!(
        text.contains("\n\n## 6. Title"),
        "blank line before the next chapter"
    );
    assert!(text.ends_with("Anhang des Teams.\n"));
    assert!(
        files
            .chapter(&new, 5)
            .contains("Eigene Notiz zu Bausteinen.")
    );
}

#[test]
fn a_broken_block_marker_refuses_the_save() {
    let d = dir();
    let path = d.path().join("arc42.md");
    let saved = doc("Old").markdown(None);
    std::fs::write(&path, &saved).unwrap();
    let mut files = Arc42Files::load(Target::File(path.clone())).unwrap();
    let new = doc("New");
    let edited = files
        .chapter(&new, 5)
        .replace("<!-- archguard:arc42:05:end -->", "");
    assert!(
        files
            .save_chapter(&new, 5, &edited, &mut files.saved(5))
            .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        saved,
        "no file was changed"
    );
    // Removing both markers makes the chapter free text.
    let free = "## 5. Title\n\nNur eigener Text.\n";
    assert!(
        files
            .save_chapter(&new, 5, free, &mut files.saved(5))
            .is_ok()
    );
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("Nur eigener Text.")
    );
}

/// In one file a chapter is found by its `## N.` heading: an edit
/// without it would land in the chapter before and duplicate chapter N
/// on the next save. The save is refused; the file and its base stay.
#[test]
fn a_file_refuses_a_chapter_without_its_own_heading() {
    let d = dir();
    let path = d.path().join("arc42.md");
    let saved = doc("Old").markdown(None);
    std::fs::write(&path, &saved).unwrap();
    let mut files = Arc42Files::load(Target::File(path.clone())).unwrap();
    let new = doc("New");
    let five = files.chapter(&new, 5) + "Eigener Text.\n";
    let cases = [
        ("deleted", five.replacen("## 5. Title\n", "", 1)),
        ("dotless", five.replacen("## 5. Title", "## 5 Title", 1)),
        ("renumbered", five.replacen("## 5. Title", "## 6. Title", 1)),
        ("text before", format!("Vorab.\n\n{five}")),
        ("two chapters", five.clone() + "\n## 6. Title\n"),
    ];
    for (case, edited) in cases {
        let mut base = files.saved(5);
        let err = files
            .save_chapter(&new, 5, &edited, &mut base)
            .expect_err(case);
        assert!(err.contains("## 5."), "{case}: {err}");
        assert_eq!(base, files.saved(5), "{case}: base kept");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), saved, "{case}");
    }
    assert!(
        files
            .save_chapter(&new, 5, &five, &mut files.saved(5))
            .is_ok(),
        "its own heading saves"
    );
}

#[test]
fn a_new_file_starts_from_the_generated_document() {
    let d = dir();
    let path = d.path().join("docs/arc42.md");
    let mut files = Arc42Files::load(Target::File(path.clone())).unwrap();
    assert_eq!(files.path_of(3), path, "one file holds every chapter");
    let new = doc("New");
    let edited = files.chapter(&new, 3) + "Kontext vom Team.\n";
    files
        .save_chapter(&new, 3, &edited, &mut files.saved(3))
        .unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.starts_with("# arc42"));
    assert!(text.contains("Kontext vom Team.\n\n## 4. Title"), "{text}");
    assert_eq!(text.matches("## 3. Title").count(), 1);
    assert_eq!(text.matches("## ").count(), 12);
    assert_eq!(find(d.path()), Some(Target::File(path)), "found next time");
}

#[test]
fn a_folder_writes_the_chapter_file_or_creates_one() {
    let d = dir();
    let folder = d.path().join("arc42");
    std::fs::create_dir(&folder).unwrap();
    std::fs::write(folder.join("05_bausteine.md"), "## 5. Bausteine\n\nAlt.\n").unwrap();
    std::fs::write(folder.join("readme.md"), "not a chapter").unwrap();
    let mut files = Arc42Files::load(Target::Folder(folder.clone())).unwrap();
    let new = doc("New");
    assert_eq!(files.chapter(&new, 5), "## 5. Bausteine\n\nAlt.\n");
    assert!(files.chapter(&new, 9).starts_with("## 9. Title"));
    assert_eq!(files.path_of(5), folder.join("05_bausteine.md"));
    assert_eq!(
        files.path_of(9),
        folder.join("arc42-09.md"),
        "the file a save creates"
    );
    assert_eq!(
        files.save_chapter(&new, 5, "## 5. Bausteine\n\nNeu.", &mut files.saved(5)),
        Ok(folder.join("05_bausteine.md"))
    );
    assert_eq!(
        std::fs::read_to_string(folder.join("05_bausteine.md")).unwrap(),
        "## 5. Bausteine\n\nNeu.\n"
    );
    let nine = files.chapter(&new, 9) + "Entscheidung A.\n";
    assert_eq!(
        files.save_chapter(&new, 9, &nine, &mut files.saved(9)),
        Ok(folder.join("arc42-09.md"))
    );
    let written = std::fs::read_to_string(folder.join("arc42-09.md")).unwrap();
    assert!(written.starts_with("## 9. Title") && written.contains("New 9"));
    assert!(written.ends_with("Entscheidung A.\n"));
    assert!(!written.contains("## 10."));
}

/// A file that starts with a UTF-8 BOM right before `## 1.`: chapter 1
/// is the project's, a save replaces it and the BOM stays.
#[test]
fn a_byte_order_mark_before_chapter_one_is_kept() {
    let d = dir();
    let path = d.path().join("arc42.md");
    let saved = "\u{feff}## 1. Einführung\n\nAlt.\n\n## 2. Rand\n\nZwei.\n";
    std::fs::write(&path, saved).unwrap();
    let mut files = Arc42Files::load(Target::File(path.clone())).unwrap();
    let new = doc("New");
    assert!(files.has_chapter(1));
    assert_eq!(files.chapter(&new, 1), "## 1. Einführung\n\nAlt.\n\n");
    let edited = "## 1. Einführung\n\nNeu.\n";
    assert_eq!(
        files.save_chapter(&new, 1, edited, &mut files.saved(1)),
        Ok(path.clone())
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "\u{feff}## 1. Einführung\n\nNeu.\n\n## 2. Rand\n\nZwei.\n"
    );
    // A chapter file of a folder keeps its BOM too.
    let folder = d.path().join("arc42");
    std::fs::create_dir(&folder).unwrap();
    let one = folder.join("01_einfuehrung.md");
    std::fs::write(&one, "\u{feff}## 1. Einführung\n\nAlt.\n").unwrap();
    let mut files = Arc42Files::load(Target::Folder(folder)).unwrap();
    assert_eq!(files.chapter(&new, 1), "## 1. Einführung\n\nAlt.\n");
    assert_eq!(
        files.save_chapter(&new, 1, edited, &mut files.saved(1)),
        Ok(one.clone())
    );
    assert_eq!(
        std::fs::read_to_string(&one).unwrap(),
        "\u{feff}## 1. Einführung\n\nNeu.\n"
    );
}

#[test]
fn a_file_changed_on_disk_refuses_the_save_and_keeps_the_change() {
    let d = dir();
    let path = d.path().join("arc42.md");
    std::fs::write(&path, doc("Old").markdown(None)).unwrap();
    let mut files = Arc42Files::load(Target::File(path.clone())).unwrap();
    let new = doc("New");
    let edited = files.chapter(&new, 5) + "Mein Kapitel 5.\n";
    let mut base = files.saved(5);
    // Someone else edits chapter 9 meanwhile (git pull, an editor).
    let theirs = std::fs::read_to_string(&path)
        .unwrap()
        .replace("Old 9", "Old 9 und Text von außen");
    std::fs::write(&path, &theirs).unwrap();
    assert!(files.save_chapter(&new, 5, &edited, &mut base).is_err());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        theirs,
        "not written"
    );
    // The new text is loaded: saving again keeps the other change.
    files.save_chapter(&new, 5, &edited, &mut base).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("Text von außen") && text.contains("Mein Kapitel 5."));
}

#[test]
fn a_chapter_file_changed_on_disk_refuses_the_save() {
    let d = dir();
    let folder = d.path().join("arc42");
    std::fs::create_dir(&folder).unwrap();
    let mut files = Arc42Files::load(Target::Folder(folder.clone())).unwrap();
    let new = doc("New");
    let mut base = files.saved(9);
    std::fs::write(folder.join("arc42-09.md"), "## 9. Von außen\n").unwrap();
    assert!(
        files
            .save_chapter(&new, 9, "## 9. Meins\n", &mut base)
            .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(folder.join("arc42-09.md")).unwrap(),
        "## 9. Von außen\n"
    );
}

/// A pull adds `09_entscheidungen.md` while chapter 9 (no file yet) is
/// edited: the save must not write `arc42-09.md`, which the next load
/// ignores, but refuse and load the pulled file.
#[test]
fn a_chapter_file_added_under_another_name_refuses_the_save() {
    let d = dir();
    let folder = d.path().join("arc42");
    std::fs::create_dir(&folder).unwrap();
    let mut files = Arc42Files::load(Target::Folder(folder.clone())).unwrap();
    let new = doc("New");
    let mut base = files.saved(9);
    let pulled = folder.join("09_entscheidungen.md");
    std::fs::write(&pulled, "## 9. Von außen\n").unwrap();
    let err = files
        .save_chapter(&new, 9, "## 9. Meins\n", &mut base)
        .expect_err("refused");
    assert!(err.contains("Chapter 9 itself was changed"), "{err}");
    assert!(!folder.join("arc42-09.md").exists(), "no ignored file");
    assert_eq!(files.path_of(9), pulled, "the pulled file is loaded");
    // Saving again writes the file the next load reads.
    assert_eq!(
        files.save_chapter(&new, 9, "## 9. Meins\n", &mut base),
        Ok(pulled.clone())
    );
    let next = Arc42Files::load(Target::Folder(folder)).unwrap();
    assert_eq!(next.chapter(&new, 9), "## 9. Meins\n");
}

#[test]
fn finds_tells_whether_a_chosen_file_is_found_next_time() {
    let d = dir();
    let root = d.path();
    assert!(finds(root, &root.join("arc42.md")));
    assert!(finds(root, &root.join("docs/ARC42.md")));
    assert!(finds(root, &root.join("docs/arc42/arc42.md")));
    assert!(
        finds(root, &root.join("a/b/c/d/arc42.md")),
        "four levels deep"
    );
    assert!(!finds(root, &root.join("a/b/c/d/e/arc42.md")), "too deep");
    assert!(
        !finds(root, &root.join("docs/architecture.md")),
        "other name"
    );
    assert!(
        !finds(root, &root.join(".github/arc42.md")),
        "hidden folder"
    );
    assert!(!finds(root, &root.join("build/arc42.md")), "build folder");
    assert!(
        !finds(root, &root.join("arc42/sub/arc42.md")),
        "the folder wins"
    );
    assert!(!finds(root, Path::new("/elsewhere/arc42.md")), "outside");
    std::fs::create_dir_all(root.join("docs/arc42")).unwrap();
    assert!(
        !finds(root, &root.join("arc42.md")),
        "other arc42 text is found"
    );
}
