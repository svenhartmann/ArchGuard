//! File → Export and a view tab's "Export": the export core
//! ([`crate::export`]) on the focused pane's view instance, with its scope
//! and filters. The file goes where the save dialog says; the notice bar
//! reports the result in the web viewer's words.
//!
//! The views hand over their data as the export's input structs
//! (`graph_ui::export`, `MatrixState::export_input`, `services_ui::export_data`,
//! …); this module only chooses, renders and writes.

mod choice;
mod png;

use std::path::PathBuf;

use slint::{ComponentHandle, SharedString};

use crate::documents::with_store;
use crate::export::{self, SecurityFilters, Snapshot, Theme, View};
use crate::matrix_ui::MatrixState;
use crate::panes::Tab;
use crate::tab_support::{index, with_ui};
use crate::ui::AppWindow;
use crate::{graph_ui, instances, panes_ui, quality_ui, security_ui, services_ui};
pub use choice::Choice;
use choice::Hint;

/// `notice-kind` values of the export results (`ui/app.slint`).
const DONE: i32 = 3;
const DONE_IMAGE: i32 = 4;
const DONE_ARC42: i32 = 5;
const FAILED: i32 = 6;
const HINT_SVG: i32 = 7;
const HINT_DATA: i32 = 8;
const LOADING: i32 = 9;

/// Why an export was not written.
enum Failure {
    Hint(Hint),
    /// The view's state is still being built.
    Loading,
    Error(String),
}

/// What to write and how the notice reads.
struct Prepared {
    name: String,
    bytes: Vec<u8>,
    notice: i32,
}

/// Register the menu and tab callbacks once at start-up.
pub fn wire(ui: &AppWindow) {
    ui.on_export(with_ui(ui, |ui, format: SharedString| {
        run(ui, &format, None);
    }));
    let tab = with_ui(ui, |ui, (pane, tab, format): (i32, i32, SharedString)| {
        let (Some(p), Some(t)) = (index(pane), index(tab)) else {
            return;
        };
        // Export the instance as it is shown: select its tab first.
        panes_ui::change(ui, |l| l.select_tab(p, t));
        run(ui, &format, None);
    });
    ui.on_export_tab(move |pane, t, format| tab((pane, t, format)));
}

/// Export the focused pane's view as `format`, to `path` or where the
/// save dialog says. Returns the written path.
pub fn run(ui: &AppWindow, format: &str, path: Option<PathBuf>) -> Option<PathBuf> {
    let choice = Choice::parse(format)?;
    let layout = panes_ui::layout();
    let tab = layout.view(layout.focus)?.clone();
    let prepared = match prepare(ui, &tab, layout.focus, choice) {
        Ok(p) => p,
        Err(f) => {
            fail(ui, f);
            return None;
        }
    };
    let path = path.or_else(|| ask(ui, &prepared.name))?;
    match export::write(&path, &prepared.bytes) {
        Ok(()) => {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
            notice(ui, prepared.notice, &name.unwrap_or(prepared.name));
            Some(path)
        }
        Err(e) => {
            fail(ui, Failure::Error(e.to_string()));
            None
        }
    }
}

fn fail(ui: &AppWindow, f: Failure) {
    match f {
        Failure::Hint(Hint::Svg) => notice(ui, HINT_SVG, ""),
        Failure::Hint(Hint::Data) => notice(ui, HINT_DATA, ""),
        Failure::Loading => notice(ui, LOADING, ""),
        Failure::Error(e) => notice(ui, FAILED, &e),
    }
}

fn notice(ui: &AppWindow, kind: i32, text: &str) {
    ui.set_notice_kind(kind);
    // A space keeps the bar open for hints without a text of their own.
    ui.set_notice(if text.is_empty() { " " } else { text }.into());
}

/// The Graph instance's display (0 tree), for the SVG check.
fn display(tab: &Tab) -> i32 {
    if tab.kind == "graph" {
        instances::with(tab.id, |g: &mut graph_ui::Graph| g.g.display).unwrap_or(tab.display)
    } else {
        0
    }
}

fn prepare(ui: &AppWindow, tab: &Tab, pane: usize, choice: Choice) -> Result<Prepared, Failure> {
    choice::check(&tab.kind, display(tab), choice).map_err(Failure::Hint)?;
    let name = choice::file_name(&tab.kind, choice);
    let (bytes, notice) = match choice {
        Choice::Markdown => (data(ui, tab)?.1.markdown().into_bytes(), DONE),
        Choice::Json => (data(ui, tab)?.1.json().into_bytes(), DONE),
        Choice::Svg => (svg(ui, tab)?.into_bytes(), DONE),
        Choice::Arc42 => (arc42()?.into_bytes(), DONE_ARC42),
        Choice::Png => (image(ui, pane)?, DONE_IMAGE),
    };
    Ok(Prepared {
        name,
        bytes,
        notice,
    })
}

/// The export snapshot of the view instance `tab`.
fn data(ui: &AppWindow, tab: &Tab) -> Result<(View, Snapshot), Failure> {
    let project = ui.get_project().to_string();
    let view = choice::view(&tab.kind).ok_or(Failure::Hint(Hint::Data))?;
    let snapshot = match view {
        View::Graph => graph_ui::export(ui, tab.id).map(|(i, _)| export::graph_snapshot(&i)),
        View::Matrix => instances::with(tab.id, |m: &mut MatrixState| m.export_input())
            .map(|i| export::graph_snapshot(&i)),
        View::Services => services_ui::export_data(ui).map(|(s, _)| s),
        View::Security => security_ui::export_state().map(|(report, query, severity)| {
            let filters = SecurityFilters {
                query,
                severity: severity.into(),
            };
            export::security_snapshot(&project, &report, &filters)
        }),
        View::Quality => quality_ui::export(ui),
        View::Architecture => Some(architecture(&project)?),
    };
    snapshot.map(|s| (view, s)).ok_or(Failure::Loading)
}

/// The target architecture assessment, as the web's Architecture export.
fn architecture(project: &str) -> Result<Snapshot, Failure> {
    let analysis = with_store(|s| s.active()?.analysis.clone()).ok_or(Failure::Loading)?;
    let run = &analysis.run;
    let model = archguard_engine::analyze_target_architecture(&run.graph, &run.config)
        .map_err(|e| Failure::Error(e.to_string()))?;
    Ok(export::architecture_snapshot(
        project,
        model.as_ref(),
        false,
        "",
    ))
}

/// The drawn diagram of Graph → Tree or Services.
fn svg(ui: &AppWindow, tab: &Tab) -> Result<String, Failure> {
    let project = ui.get_project();
    let (title, snapshot, diagram) = match tab.kind.as_str() {
        "graph" => {
            let (input, diagram) = graph_ui::export(ui, tab.id).ok_or(Failure::Loading)?;
            let diagram = diagram.ok_or(Failure::Hint(Hint::Svg))?;
            (
                "Import dependencies",
                export::graph_snapshot(&input),
                diagram,
            )
        }
        "services" => {
            let (snapshot, diagram) = services_ui::export_data(ui).ok_or(Failure::Loading)?;
            ("Service configuration", snapshot, diagram)
        }
        _ => return Err(Failure::Hint(Hint::Svg)),
    };
    let diagram = export::Diagram {
        title: format!("{project} · {title}"),
        description: snapshot.description(),
        ..diagram
    };
    let dark = ui.global::<crate::ui::Theme>().get_dark();
    Ok(export::svg(&diagram, Theme::of(dark)))
}

/// The arc42 document of the active document's analysis.
fn arc42() -> Result<String, Failure> {
    let analysis = with_store(|s| s.active()?.analysis.clone()).ok_or(Failure::Loading)?;
    export::arc42(&analysis.root, &analysis.run, None).map_err(Failure::Error)
}

/// Pane `pane` cropped from the rendered window.
fn image(ui: &AppWindow, pane: usize) -> Result<Vec<u8>, Failure> {
    let window = ui.window();
    let shot = window
        .take_snapshot()
        .map_err(|e| Failure::Error(e.to_string()))?;
    let scale = window.scale_factor();
    // The rendered image is the whole window; a menu bar drawn by Slint
    // takes its top rows.
    let width = shot.width() as f32 / scale;
    let height = shot.height() as f32 / scale;
    let menu = (height - ui.get_content_height()).max(0.0);
    let body = png::Body {
        width,
        height,
        top: menu + ui.get_body_top(),
        split: panes_ui::layout().is_split(),
        ratio: ui.get_pane_ratio(),
    };
    let rect = png::physical(
        png::pane_area(body, pane),
        scale,
        shot.width(),
        shot.height(),
    );
    export::png(&shot, rect).map_err(|e| Failure::Error(e.to_string()))
}

/// The save dialog, in the project folder with the default name. Never
/// in headless runs, which pass their path.
fn ask(ui: &AppWindow, name: &str) -> Option<PathBuf> {
    if crate::context::headless_run() {
        return None;
    }
    let mut dialog = rfd::FileDialog::new()
        .set_title(ui.get_export_title().as_str())
        .set_file_name(name);
    if let Some(dir) = with_store(|s| s.active().map(|d| d.root.clone())) {
        dialog = dialog.set_directory(dir);
    }
    dialog.save_file()
}
