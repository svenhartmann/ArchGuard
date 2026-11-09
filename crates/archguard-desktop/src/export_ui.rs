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
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use slint::{ComponentHandle, SharedString};

use crate::context::Analysis;
use crate::documents::with_store;
use crate::export::{self, SecurityFilters, Snapshot, Theme, View};
use crate::matrix_ui::MatrixState;
use crate::panes::Tab;
use crate::tab_support::{self, index, with_ui};
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
    bytes: Bytes,
    notice: i32,
}

/// Work that makes an export's bytes on a worker thread.
type Job = Box<dyn FnOnce() -> Result<Vec<u8>, Failure> + Send>;

/// An export's bytes: made on the UI thread from the view's state, or a
/// [`Job`] for a worker (arc42, which runs the engine's analysis).
enum Bytes {
    Now(Vec<u8>),
    Later(Job),
}

/// Set while an arc42 export runs on its worker thread.
static ARC42_BUSY: AtomicBool = AtomicBool::new(false);

/// The running arc42 export; dropping it frees the slot.
struct Busy;

impl Busy {
    /// `None` while another arc42 export runs.
    fn begin() -> Option<Busy> {
        (!ARC42_BUSY.swap(true, Ordering::AcqRel)).then_some(Busy)
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        ARC42_BUSY.store(false, Ordering::Release);
    }
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
/// save dialog says. Returns the written path; `None` also while an arc42
/// export still runs on its worker thread (snapshot runs make it in place).
pub fn run(ui: &AppWindow, format: &str, path: Option<PathBuf>) -> Option<PathBuf> {
    let choice = Choice::parse(format)?;
    let layout = panes_ui::layout();
    let tab = layout.view(layout.focus)?.clone();
    // One arc42 export at a time; a request meanwhile does nothing.
    let busy = match choice {
        Choice::Arc42 => Some(Busy::begin()?),
        _ => None,
    };
    let prepared = match prepare(ui, &tab, layout.focus, choice) {
        Ok(p) => p,
        Err(f) => {
            fail(ui, f);
            return None;
        }
    };
    let path = path.or_else(|| ask(ui, &prepared.name))?;
    let job = match prepared.bytes {
        Bytes::Now(bytes) => return save(ui, path, prepared.notice, prepared.name, Ok(bytes)),
        Bytes::Later(job) if tab_support::snapshot_mode() => {
            let bytes = tab_support::catch(job).unwrap_or_else(|e| Err(Failure::Error(e)));
            return save(ui, path, prepared.notice, prepared.name, bytes);
        }
        Bytes::Later(job) => job,
    };
    // The previous export's notice would read as this one's result.
    ui.set_notice(SharedString::new());
    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let bytes = tab_support::catch(job).unwrap_or_else(|e| Err(Failure::Error(e)));
        let _ = slint::invoke_from_event_loop(move || {
            drop(busy);
            if let Some(ui) = weak.upgrade() {
                save(&ui, path, prepared.notice, prepared.name, bytes);
            }
        });
    });
    None
}

/// Write the made bytes to `path` and show the result.
fn save(
    ui: &AppWindow,
    path: PathBuf,
    kind: i32,
    name: String,
    bytes: Result<Vec<u8>, Failure>,
) -> Option<PathBuf> {
    let bytes = match bytes {
        Ok(b) => b,
        Err(f) => {
            fail(ui, f);
            return None;
        }
    };
    match export::write(&path, &bytes) {
        Ok(()) => {
            let file = path.file_name().map(|n| n.to_string_lossy().into_owned());
            notice(ui, kind, &file.unwrap_or(name));
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
        Choice::Markdown => (Bytes::Now(data(ui, tab)?.1.markdown().into_bytes()), DONE),
        Choice::Json => (Bytes::Now(data(ui, tab)?.1.json().into_bytes()), DONE),
        Choice::Svg => (Bytes::Now(svg(ui, tab)?.into_bytes()), DONE),
        Choice::Arc42 => (Bytes::Later(arc42_job(active_analysis()?)), DONE_ARC42),
        Choice::Png => (Bytes::Now(image(ui, pane)?), DONE_IMAGE),
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

fn active_analysis() -> Result<Arc<Analysis>, Failure> {
    with_store(|s| s.active()?.analysis.clone()).ok_or(Failure::Loading)
}

/// The arc42 document of `analysis`, made by a worker: the engine's arc42
/// analysis runs compiler-backed calls, services and coverage.
fn arc42_job(analysis: Arc<Analysis>) -> Job {
    Box::new(move || {
        export::arc42(&analysis.root, &analysis.run, None)
            .map(String::into_bytes)
            .map_err(Failure::Error)
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::Analysis;
    use std::path::Path;
    use std::sync::Arc;

    /// The arc42 export runs the engine's full arc42 analysis; it is a job
    /// a worker thread runs, not work on the UI thread.
    #[test]
    fn the_arc42_export_runs_on_a_worker_thread() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/ts-cyclic");
        let loaded = archguard_engine::load_config(None, &root).expect("config");
        let run =
            archguard_engine::analyze_run_quiet(&root, &loaded, archguard_engine::Level::Module)
                .expect("analysis");
        let analysis = Arc::new(Analysis {
            root,
            run,
            sources: Default::default(),
        });
        let job = arc42_job(analysis);
        let caller = std::thread::current().id();
        let (bytes, worker) = std::thread::spawn(move || (job(), std::thread::current().id()))
            .join()
            .expect("worker");
        assert_ne!(worker, caller);
        let text = String::from_utf8(bytes.ok().expect("arc42")).unwrap();
        assert!(text.contains("arc42"), "{text}");
    }

    /// A second arc42 export while one runs starts no second analysis.
    #[test]
    fn one_arc42_export_runs_at_a_time() {
        let first = Busy::begin().expect("free");
        assert!(Busy::begin().is_none(), "second export while one runs");
        drop(first);
        assert!(Busy::begin().is_some(), "free again after the first");
    }
}
