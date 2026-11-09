//! The arc42 diagram catalogue in the reader (ADR 0006): which catalogue
//! entry a package map is, the generated sections the reader draws itself
//! instead of showing their source, and the tick that writes
//! `[arc42] hidden_diagrams` to `archguard.toml`.

use std::path::Path;

use archguard_engine::core::{Diagram, DiagramFormat, Graph};
use archguard_engine::report::{Arc42Document, diagram_section};

use super::visual::Kind;
use crate::config_editor::{ConfigEditor, FieldValue};

/// The catalogue id of a package map.
pub fn map_id(kind: Kind) -> &'static str {
    match kind {
        Kind::Dependencies => "building-blocks",
        Kind::Calls => "runtime-calls",
        Kind::Cycles => "cycles",
    }
}

/// `text` without the generated sections of the shown `diagrams`.
pub fn strip(text: &str, diagrams: &[Diagram], format: DiagramFormat) -> String {
    diagrams
        .iter()
        .filter(|d| d.shown)
        .fold(text.to_string(), |t, d| {
            t.replacen(&diagram_section(d, format), "", 1)
        })
}

/// `hidden` with `id` removed (`show`) or added.
pub fn hidden_after(hidden: &[String], id: &str, show: bool) -> Vec<String> {
    let mut out: Vec<String> = hidden.iter().filter(|h| *h != id).cloned().collect();
    if !show {
        out.push(id.to_string());
    }
    out
}

/// The catalogue id of a map view: the map's own id for the overview,
/// `<map>@<package>` for a package's level.
pub fn view_id(kind: Kind, scope: Option<usize>, graph: &Graph) -> String {
    match scope.and_then(|n| graph.nodes.get(n)) {
        Some(n) => format!("{}@{}", map_id(kind), n.name),
        None => map_id(kind).to_string(),
    }
}

/// Write `ids` as `[arc42] <key>` (`hidden_diagrams`, `shown_diagrams`) of
/// the project at `root`; the rest of the file stays as written.
pub fn write(root: &Path, key: &str, ids: &[String]) -> Result<(), String> {
    let mut editor = ConfigEditor::load(root)?;
    editor
        .set(&format!("arc42.{key}"), FieldValue::List(ids.to_vec()))
        .map_err(|e| e.to_string())?;
    editor.save().map(drop).map_err(|e| e.to_string())
}

/// The document as `archguard doc --arc42` writes it with `hidden`: the
/// diagrams' `shown` flags and the sections at the start of each chapter.
pub fn rebase(doc: &mut Arc42Document, hidden: &[String], format: DiagramFormat) {
    for c in &mut doc.chapters {
        let mine: Vec<Diagram> = doc
            .diagrams
            .iter()
            .filter(|d| d.chapter == c.number)
            .cloned()
            .collect();
        let mut text = strip(&c.markdown, &mine, format);
        let prefix: String = mine
            .iter()
            .filter(|d| !hidden.contains(&d.id))
            .map(|d| diagram_section(d, format))
            .collect();
        text.insert_str(0, &prefix);
        c.markdown = text;
    }
    for d in &mut doc.diagrams {
        d.shown = !hidden.contains(&d.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::DiagramKind;
    use archguard_engine::report::Arc42Chapter;

    fn diagram(id: &str, chapter: u8) -> Diagram {
        Diagram {
            id: id.into(),
            chapter,
            title: format!("Title {id}"),
            kind: DiagramKind::Packages,
            note: "Note.".into(),
            unit: "Quellverbindungen".into(),
            elements: vec![],
            relations: vec![],
            shown: true,
        }
    }

    fn doc() -> Arc42Document {
        let diagrams = vec![diagram("a", 5), diagram("b", 5)];
        let f = DiagramFormat::Mermaid;
        let body = format!(
            "{}{}Text.",
            diagram_section(&diagrams[0], f),
            diagram_section(&diagrams[1], f)
        );
        Arc42Document {
            schema_version: 1,
            lang: archguard_engine::core::DocLang::De,
            scope: "p".into(),
            notice: "",
            chapters: vec![Arc42Chapter {
                number: 5,
                title: "Bausteinsicht",
                markdown: body,
                questions: vec![],
            }],
            diagrams,
        }
    }

    #[test]
    fn a_map_level_has_its_own_catalogue_id() {
        let mut b = archguard_engine::core::GraphBuilder::new();
        b.ensure_path("app/core/a.rs", Some("rust"));
        let g = b.build("p".into());
        let core = g.nodes.iter().position(|n| n.name == "app/core").unwrap();
        assert_eq!(view_id(Kind::Dependencies, None, &g), "building-blocks");
        assert_eq!(view_id(Kind::Cycles, Some(core), &g), "cycles@app/core");
    }

    #[test]
    fn the_reader_drops_the_sections_it_draws() {
        let d = doc();
        assert_eq!(
            strip(&d.chapters[0].markdown, &d.diagrams, DiagramFormat::Mermaid),
            "Text."
        );
    }

    #[test]
    fn hiding_and_showing_rewrites_the_chapter_like_the_cli() {
        let mut d = doc();
        let f = DiagramFormat::Mermaid;
        let hidden = hidden_after(&["x".into()], "a", false);
        assert_eq!(hidden, ["x", "a"]);
        rebase(&mut d, &hidden, f);
        assert!(!d.diagrams[0].shown && d.diagrams[1].shown);
        assert_eq!(
            d.chapters[0].markdown,
            format!("{}Text.", diagram_section(&d.diagrams[1], f))
        );
        rebase(&mut d, &hidden_after(&hidden, "a", true), f);
        assert_eq!(d.chapters[0].markdown, doc().chapters[0].markdown);
    }

    #[test]
    fn the_tick_writes_hidden_diagrams_and_keeps_the_rest_of_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(".archguard/archguard.toml");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "# mine\n[modules]\ndepth = 3\n").unwrap();
        write(dir.path(), "hidden_diagrams", &["cycles".into()]).unwrap();
        write(dir.path(), "shown_diagrams", &["cycles@app".into()]).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.starts_with("# mine\n[modules]\ndepth = 3\n"), "{text}");
        let config: archguard_engine::core::Config = toml::from_str(&text).unwrap();
        assert_eq!(config.arc42.hidden_diagrams, ["cycles"]);
        assert_eq!(config.arc42.shown_diagrams, ["cycles@app"]);
    }
}
