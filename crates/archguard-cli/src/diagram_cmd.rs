//! `archguard diagram` and `doc --arc42 --format json`: the arc42 diagram
//! catalogue as a table or data, one diagram drawn in the terminal
//! (`archguard_report::ascii`).

use std::io::IsTerminal;
use std::path::Path;

use archguard_engine::LoadedConfig;
use archguard_engine::arc42_file::{Arc42Files, Target};
use archguard_engine::core::{Arc42Config, Diagram, DiagramKind, Graph, Report};
use archguard_engine::report::Arc42Document;
use archguard_engine::report::ascii::{self, Style};
use archguard_engine::report::package_map::{SourceModel, levels};
use serde_json::Value;

use crate::args::{Cli, Command, Format};
use crate::output::{
    EXIT_OK, analysis_error, emit, emit_drawing, terminal_safe, to_pretty_json, usage,
};

/// `diagram`, and `doc --arc42 --format json` (without `--check`);
/// `None` for every other `doc`.
pub(crate) fn dispatch(
    cli: &Cli,
    root: &Path,
    loaded: &LoadedConfig,
    graph: &Graph,
    report: &Report,
) -> Option<i32> {
    match &cli.command {
        Command::Diagram { .. } => Some(run(cli, root, loaded, graph, report)),
        Command::Doc {
            out,
            arc42: true,
            check: false,
            chapter,
            ..
        } if cli.format == Format::Json => {
            let target = (out.as_deref(), *chapter);
            Some(doc_json(cli, root, loaded, graph, report, target))
        }
        _ => None,
    }
}

/// `doc --arc42 --format json`: the document (`out`, else found as the
/// desktop finds it) as data, nothing written.
fn doc_json(
    cli: &Cli,
    root: &Path,
    loaded: &LoadedConfig,
    graph: &Graph,
    report: &Report,
    (out, chapter): (Option<&Path>, Option<u8>),
) -> i32 {
    let (doc, files) = match open(root, &loaded.config.arc42, out, (graph, report), loaded) {
        Ok(found) => found,
        Err(code) => return code,
    };
    if !cli.quiet {
        let value = archguard_engine::arc42_data::document(&files, &doc, chapter);
        emit(&to_pretty_json(&value));
    }
    EXIT_OK
}

/// The arc42 document under `choice` and the project's text of it.
fn open(
    root: &Path,
    choice: &Arc42Config,
    out: Option<&Path>,
    (graph, report): (&Graph, &Report),
    loaded: &LoadedConfig,
) -> Result<(Arc42Document, Arc42Files), i32> {
    let mut config = loaded.config.clone();
    config.arc42 = choice.clone();
    let doc =
        archguard_engine::arc42_document(root, &config, graph, report).map_err(analysis_error)?;
    let files = match out {
        Some(out) => Arc42Files::load(Target::File(root.join(out))),
        None => Arc42Files::open(root, choice.diagram_format),
    };
    Ok((doc, files.map_err(|e| usage(&e))?))
}

fn run(cli: &Cli, root: &Path, loaded: &LoadedConfig, graph: &Graph, report: &Report) -> i32 {
    let Command::Diagram {
        path,
        id,
        level,
        ascii,
    } = &cli.command
    else {
        unreachable!("dispatched for diagram only")
    };
    if !matches!(cli.format, Format::Text | Format::Json) {
        return usage("diagram prints text or json");
    }
    // A single argument that is no folder is the id (`Command::path`).
    let id = id.clone().or_else(|| {
        let p = path.as_deref().filter(|p| !p.is_dir())?;
        Some(p.to_string_lossy().into_owned())
    });
    let id = match (level, id) {
        (Some(package), id) => {
            let map = id.as_deref().unwrap_or("building-blocks");
            Some(format!("{map}@{package}"))
        }
        (None, id) => id,
    };
    let analysis = (root, loaded, graph, report);
    let catalogue = match &id {
        Some(id) => one(analysis, id).map(|d| vec![d]),
        None => entries(analysis, &loaded.config.arc42),
    };
    let catalogue = match catalogue {
        Ok(c) => c,
        Err(code) => return code,
    };
    if !cli.quiet {
        match (cli.format, id.is_some()) {
            (Format::Json, true) => emit(&to_pretty_json(&catalogue[0])),
            (Format::Json, false) => emit(&to_pretty_json(&Value::from(catalogue))),
            (_, true) => emit_drawing(&drawing(&catalogue[0], *ascii)),
            (_, false) => emit(&table(&catalogue)),
        }
    }
    EXIT_OK
}

type Analysis<'a> = (&'a Path, &'a LoadedConfig, &'a Graph, &'a Report);

/// The catalogue under `choice`, as `doc --arc42 --format json` has it.
fn entries(
    (root, loaded, graph, report): Analysis<'_>,
    choice: &Arc42Config,
) -> Result<Vec<Value>, i32> {
    let (doc, files) = open(root, choice, None, (graph, report), loaded)?;
    Ok(archguard_engine::arc42_data::catalogue(&files, &doc, None))
}

/// The catalogue entry `id`. A package map level the project does not
/// show is made for this call only: neither shown nor chosen.
fn one(analysis: Analysis<'_>, id: &str) -> Result<Value, i32> {
    let (root, loaded, graph, _) = analysis;
    let mine = &loaded.config.arc42;
    let find = |all: Vec<Value>| all.into_iter().find(|d| d["id"] == id);
    let hint = "; `archguard diagram` lists the catalogue";
    let Some((map, package)) = Arc42Config::level_of(id) else {
        let found = find(entries(analysis, mine)?);
        return found.ok_or_else(|| usage(format!("no diagram `{id}`{hint}")));
    };
    let mut choice = mine.clone();
    if !choice.shown_diagrams.iter().any(|s| s == id) {
        choice.shown_diagrams.push(id.to_string());
    }
    let Some(mut entry) = find(entries(analysis, &choice)?) else {
        let known = level_packages(graph).join(", ");
        return Err(usage(format!(
            "no level `{package}` of `{map}`; packages with a level: {known}{hint}"
        )));
    };
    if !mine.shown_diagrams.iter().any(|s| s == id) {
        entry["shown"] = false.into();
        entry["chosen"] = false.into();
        // Drawn into a generated chapter only because this call asked.
        let files = Arc42Files::open(root, mine.diagram_format).map_err(|e| usage(&e))?;
        let chapter = entry["chapter"].as_u64().unwrap_or(0) as u8;
        if !files.has_chapter(chapter) {
            entry["in_text"] = false.into();
        }
    }
    Ok(entry)
}

/// The packages of the levels the desktop's map drills into.
fn level_packages(graph: &Graph) -> Vec<String> {
    let model = SourceModel::new(graph);
    levels(&model, graph, None)
        .entries
        .iter()
        .map(|e| graph.nodes[e.node].name.clone())
        .collect()
}

/// The catalogue as a table: chapter, kind, in the document, in the
/// project's text, title, id. The id comes last and whole (scenario ids
/// run to 180 characters), so the short columns stay aligned and it
/// stays copyable.
fn table(catalogue: &[Value]) -> String {
    let yes = |v: &Value| {
        if v.as_bool() == Some(true) {
            "yes"
        } else {
            "no"
        }
    };
    let text = |v: &Value| v.as_str().unwrap_or("").to_string();
    let rows: Vec<[String; 6]> = catalogue
        .iter()
        .map(|d| {
            [
                d["chapter"].to_string(),
                text(&d["kind"]),
                yes(&d["shown"]).into(),
                yes(&d["in_text"]).into(),
                title(&text(&d["title"])),
                text(&d["id"]),
            ]
        })
        .collect();
    let head = ["CHAPTER", "KIND", "IN DOC", "IN TEXT", "TITLE", "ID"].map(String::from);
    let mut widths = head.clone().map(|h| h.chars().count());
    for row in &rows {
        for (w, cell) in widths.iter_mut().zip(row) {
            *w = (*w).max(cell.chars().count());
        }
    }
    let line = |row: &[String; 6]| {
        let cells: Vec<String> = (row.iter().zip(widths))
            .map(|(c, w)| format!("{c:<w$}"))
            .collect();
        cells.join("  ").trim_end().to_string() + "\n"
    };
    std::iter::once(&head).chain(&rows).map(line).collect()
}

/// A title cut to [`TITLE`] characters, so the id column starts at a
/// fixed place.
fn title(t: &str) -> String {
    if t.chars().count() <= TITLE {
        return t.to_string();
    }
    t.chars().take(TITLE - 1).chain(['…']).collect()
}

/// The widest title in the list.
const TITLE: usize = 60;

/// Title line, the drawing, what the counts count.
/// Written past `emit`'s filter to keep the colours, so every string of
/// the entry (repository names, labels, title) is neutralised first.
fn drawing(entry: &Value, ascii: bool) -> String {
    let mut entry = entry.clone();
    neutralise(&mut entry);
    let d: Diagram = serde_json::from_value(entry.clone()).expect("a catalogue diagram");
    let stdout = std::io::stdout().is_terminal();
    let style = Style {
        unicode: !ascii,
        color: stdout && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty()),
        width: std::env::var("COLUMNS")
            .ok()
            .and_then(|c| c.parse().ok())
            .unwrap_or(100),
    };
    let kind = entry["kind"].as_str().unwrap_or("");
    let mut out = format!("{} [{}, chapter {}, {kind}]\n\n", d.title, d.id, d.chapter);
    out.push_str(&ascii::draw(&d, style));
    if d.kind != DiagramKind::Sequence && !d.unit.is_empty() && !d.relations.is_empty() {
        out.push_str(&format!("\ncounts: {}\n", d.unit));
    }
    out
}

/// Every string in `value` through [`terminal_safe`]; ids and the
/// relations' ends change alike, so they still match.
fn neutralise(value: &mut Value) {
    match value {
        Value::String(s) => *s = terminal_safe(s),
        Value::Array(items) => items.iter_mut().for_each(neutralise),
        Value::Object(fields) => fields.values_mut().for_each(neutralise),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_long_title_is_cut_to_the_column() {
        let cut = super::title(&"x".repeat(70));
        assert_eq!(cut.chars().count(), super::TITLE);
        assert!(cut.ends_with('…'));
        assert_eq!(super::title("Package overview"), "Package overview");
    }

    #[test]
    fn a_drawing_escapes_repository_control_chars() {
        use archguard_engine::core::{Diagram, DiagramKind};
        let payload = "x\x1b]0;X\x07\x1b[31m\ny";
        // A relation of a box to itself is listed below the picture, past
        // the canvas.
        let mut d: Diagram = serde_json::from_value(serde_json::json!({
            "id": payload, "chapter": 5, "title": payload, "kind": "packages",
            "note": "", "unit": payload, "shown": true,
            "elements": [{"id": payload, "name": payload, "kind": "package", "cycle": false}],
            "relations": [{"from": payload, "to": payload, "kind": "dependency", "count": 1,
                "cycle": false, "error": false, "reply": false}],
        }))
        .unwrap();
        d.kind = DiagramKind::Packages;
        let out = super::drawing(&serde_json::to_value(&d).unwrap(), true);
        assert!(!out.contains(['\x1b', '\x07']), "{out:?}");
        assert!(
            out.starts_with("x\\u{1b}]0;X\\u{7}\\u{1b}[31m\ny ["),
            "{out:?}"
        );
    }
}
