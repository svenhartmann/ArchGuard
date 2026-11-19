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
use crate::output::{EXIT_OK, analysis_error, emit, to_pretty_json, usage};

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
        emit(&match (cli.format, id.is_some()) {
            (Format::Json, true) => to_pretty_json(&catalogue[0]),
            (Format::Json, false) => to_pretty_json(&Value::from(catalogue)),
            (_, true) => drawing(&catalogue[0], *ascii),
            (_, false) => table(&catalogue),
        });
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

/// The catalogue as a table: id, chapter, kind, in the document, in the
/// project's text, title.
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
                text(&d["id"]),
                d["chapter"].to_string(),
                text(&d["kind"]),
                yes(&d["shown"]).into(),
                yes(&d["in_text"]).into(),
                text(&d["title"]),
            ]
        })
        .collect();
    let head = ["ID", "CHAPTER", "KIND", "IN DOC", "IN TEXT", "TITLE"].map(String::from);
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

/// Title line, the drawing, what the counts count.
fn drawing(entry: &Value, ascii: bool) -> String {
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
