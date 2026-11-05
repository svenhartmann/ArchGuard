//! Start tab: the web viewer's "Your architecture" world view
//! (`viewer_world.js`, `viewer_world_model.js`). Packages are plates of
//! surfaces, one per class or file, drawn in an isometric projection that
//! is computed here ([`scene`]); Slint draws the resulting polygons.
//!
//! All tab state is one [`StartState`], built by [`Inputs::build`] and shown by
//! [`push`] as one `StartModel` value.

mod details;
mod evidence;
mod layout;
mod pins;
mod scene;
mod state;
mod view;

use std::path::PathBuf;

use archguard_engine::core::{Config, Finding, Graph};
use slint::ComponentHandle;

use crate::context::Loaded;
use crate::graph_ui::tab_support::{self, Actions, Calls, TabState, index};
use crate::ui::{AppWindow, StartData, StartModel};
use evidence::Evidence;
use scene::Camera;
use state::from_evidence;
pub use state::{Lens, Selection, StartState};

pub use view::push;

/// What the tab's build needs, copied from an analysis.
struct Inputs {
    calls: Calls,
    root: PathBuf,
    config: Config,
    graph: Graph,
    findings: Vec<Finding>,
}

impl Inputs {
    fn of(data: &Loaded) -> Self {
        Inputs {
            calls: Calls::of(data),
            root: data.root.to_path_buf(),
            config: data.run.config.clone(),
            graph: data.run.graph.clone(),
            findings: data.run.report.findings.clone(),
        }
    }

    /// The tab state: the same engine calls as the CLI's report
    /// (`review_cmd.rs`). Slow; runs on a worker thread.
    fn build(self) -> StartState {
        let Inputs {
            calls,
            root,
            config,
            graph,
            findings,
        } = self;
        let calls = calls.get();
        let coverage = archguard_engine::read_test_coverage(&root, &config, &graph);
        let world = archguard_engine::architecture_world(&graph, &calls, &coverage);
        let mut state = from_evidence(Evidence::new(graph, findings, &calls, world));
        snapshot_overrides(&mut state);
        state
    }
}

/// Register the tab's callbacks once at start-up.
pub fn wire(ui: &AppWindow) {
    let d = ui.global::<StartData>();
    let a = Actions::<StartState>::new(ui);
    d.on_set_lens(a.on(|s, _, l: i32| {
        s.set_lens(match l {
            1 => Lens::Explore,
            2 => Lens::Coverage,
            _ => Lens::Problems,
        })
    }));
    d.on_set_kpi(a.on(|s, _, k: i32| s.set_kpi(index(k).unwrap_or(0))));
    d.on_go_back(a.on0(|s, _| s.back()));
    d.on_crumb(a.on(|s, ui, i: i32| {
        let crumbs = s.crumbs(&ui.get_project());
        if let Some((_, scope)) = index(i).and_then(|i| crumbs.get(i)) {
            s.navigate(scope.clone());
        }
    }));
    d.on_open_area(a.on(|s, _, i: i32| s.open_area(index(i).unwrap_or(0))));
    d.on_resized(
        a.on2(|s, _, w: f32, h: f32| s.canvas = (f64::from(w).max(1.0), f64::from(h).max(1.0))),
    );
    d.on_pan(a.on2(|s, _, dx: f32, dy: f32| {
        s.cam.x += f64::from(dx);
        s.cam.y += f64::from(dy);
    }));
    let wheel = a.on(|s, _, (x, y, delta): (f32, f32, f32)| {
        let delta = f64::from(delta).clamp(-180.0, 180.0);
        s.wheel(f64::from(x), f64::from(y), (delta * 0.005).exp())
    });
    d.on_wheel(move |x, y, delta| wheel((x, y, delta)));
    d.on_zoom_by(a.on(|s, _, f: f32| s.cam = scene::zoom(s.cam, f64::from(f), (s.cam.x, s.cam.y))));
    d.on_rotate_by(a.on(|s, _, r: f32| s.cam = scene::rotate(s.cam, f64::from(r))));
    d.on_orient(a.on0(|s, _| s.cam.rotation = 0.0));
    d.on_fit(a.on0(|s, _| {
        s.cam = Camera {
            rotation: s.cam.rotation,
            ..Camera::default()
        }
    }));
    d.on_reset(a.on0(|s, _| s.reset()));
    wire_selection(ui, &d);
}

fn wire_selection(ui: &AppWindow, d: &StartData<'_>) {
    let a = Actions::<StartState>::new(ui);
    d.on_search(a.on(|s, _, q: slint::SharedString| s.query = q.trim().to_lowercase()));
    d.on_accept_search(a.on0(|s, _| s.accept_search()));
    d.on_click(a.on2(|s, _, x: f32, y: f32| {
        if let Some(c) = s.cell_at(f64::from(x), f64::from(y)) {
            s.select_cell(c);
        }
    }));
    d.on_double_click(a.on2(|s, _, x: f32, y: f32| {
        if let Some(c) = s.cell_at(f64::from(x), f64::from(y)) {
            s.open_area(s.layout.cells[c].area);
        }
    }));
    d.on_pick_area(a.on(|s, _, i: i32| {
        // Entry 0 is "Select area…".
        s.selection = index(i - 1)
            .filter(|&a| a < s.scope.areas.len())
            .map(|area| Selection {
                area,
                entity: None,
                members: None,
            });
    }));
    d.on_toggle_details(a.on0(|s, _| s.details_open = !s.details_open && s.selection.is_some()));
    d.on_pick_entity(a.on(|s, _, e: i32| {
        if let (Some(sel), Some(e)) = (&mut s.selection, index(e)) {
            sel.entity = Some(e);
            s.limit = 20;
        }
    }));
    d.on_more(a.on0(|s, _| s.limit += 20));
    d.on_source_graph(a.on0(source_graph));
}

/// "Source graph": reveal the selection in the Graph tab.
fn source_graph(s: &mut StartState, ui: &AppWindow) {
    let Some(sel) = &s.selection else { return };
    let name = sel
        .entity
        .map(|e| s.ev.entity(e).node.clone())
        .unwrap_or_else(|| s.scope.areas[sel.area].name.clone());
    // Graph-tab nodes are the graph's nodes.
    if let Some(n) = s.ev.graph.nodes.iter().position(|n| n.name == name) {
        tab_support::reveal(ui, n);
    }
}

impl TabState for StartState {
    fn push(&mut self, ui: &AppWindow) {
        push(ui, self);
    }

    /// Keeps the canvas size the view reported.
    fn pending(ui: &AppWindow, error: Option<&str>) {
        let d = ui.global::<StartData>();
        let old = d.get_model();
        d.set_model(StartModel {
            canvas_w: old.canvas_w,
            canvas_h: old.canvas_h,
            error: error.unwrap_or_default().into(),
            ..Default::default()
        });
    }

    /// The view may have reported its size before the state existed.
    fn arrive(&mut self, ui: &AppWindow) {
        let m = ui.global::<StartData>().get_model();
        if m.canvas_w > 1.0 && m.canvas_h > 1.0 {
            self.canvas = (f64::from(m.canvas_w), f64::from(m.canvas_h));
        }
    }
}

/// Show the active document's tab. Its world is built once per document,
/// on a worker thread.
pub fn show(ui: &AppWindow, data: &Loaded) {
    tab_support::show_async(ui, || {
        let inputs = Inputs::of(data);
        move || inputs.build()
    });
}

/// `ARCHGUARD_DESKTOP_WORLD=<package>`, `ARCHGUARD_DESKTOP_LENS=<0..2>`,
/// `ARCHGUARD_DESKTOP_KPI=<0..6>`, `ARCHGUARD_DESKTOP_ROTATE=<degrees>` and
/// `ARCHGUARD_DESKTOP_PICK=<area name>` set the view for snapshot runs.
#[cfg(feature = "snapshot")]
fn snapshot_overrides(s: &mut StartState) {
    let var = |k: &str| std::env::var(k).ok();
    if let Some(w) = var("ARCHGUARD_DESKTOP_WORLD") {
        s.navigate(Some(w));
    }
    if let Some(k) = var("ARCHGUARD_DESKTOP_KPI").and_then(|k| k.parse().ok()) {
        s.set_kpi(k);
    }
    match var("ARCHGUARD_DESKTOP_LENS").as_deref() {
        Some("1") => s.set_lens(Lens::Explore),
        Some("2") => s.set_lens(Lens::Coverage),
        _ => {}
    }
    if let Some(r) = var("ARCHGUARD_DESKTOP_ROTATE").and_then(|r| r.parse::<f64>().ok()) {
        s.cam = scene::rotate(s.cam, r.to_radians());
    }
    if let Some(name) = var("ARCHGUARD_DESKTOP_PICK")
        && let Some(area) = s.scope.areas.iter().position(|a| a.name == name)
    {
        s.select(Selection {
            area,
            entity: None,
            members: None,
        });
        s.details_open = true;
    }
}

#[cfg(not(feature = "snapshot"))]
fn snapshot_overrides(_: &mut StartState) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn repo_state() -> StartState {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("repository root");
        let loaded = archguard_engine::load_config(None, &root).expect("config");
        let run =
            archguard_engine::analyze_run_quiet(&root, &loaded, archguard_engine::Level::Module)
                .expect("analysis");
        let sources = Default::default();
        Inputs::of(&Loaded {
            root: &root,
            run: &run,
            sources: &sources,
        })
        .build()
    }

    /// This repository (source root `crates`): every crate is a
    /// package plate, every world entity lands on one surface, and opening
    /// a crate and going back restores the overview.
    #[test]
    fn build_shows_the_crates_and_navigates() {
        let mut s = repo_state();
        let names: Vec<_> = s.scope.areas.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(s.home, None, "{names:?}");
        assert_eq!(names.len(), 9);
        assert!(
            s.scope
                .areas
                .iter()
                .any(|a| a.name == "archguard-engine" && a.module)
        );
        let entities: usize = s.layout.cells.iter().map(|c| c.entities.len()).sum();
        let in_scope: usize = s.scope.areas.iter().map(|a| a.entities.len()).sum();
        assert_eq!(entities, in_scope);
        // One surface per entity up to 750; above that the web batches
        // ceil(total / 750) entities per surface, per area.
        let batch = in_scope.div_ceil(750).max(1);
        let expected: usize = s
            .scope
            .areas
            .iter()
            .map(|a| a.entities.len().div_ceil(batch))
            .sum();
        assert_eq!(s.layout.cells.len(), expected, "batch {batch}");
        assert_eq!(s.crumbs("repo"), vec![("repo".to_string(), None)]);

        let engine = s
            .scope
            .areas
            .iter()
            .position(|a| a.name == "archguard-engine")
            .unwrap();
        s.open_area(engine);
        assert_eq!(s.within.as_deref(), Some("archguard-engine"));
        assert_eq!(
            s.crumbs("repo"),
            vec![
                ("repo".to_string(), None),
                (
                    "archguard-engine".to_string(),
                    Some("archguard-engine".to_string())
                )
            ]
        );
        assert!(
            s.scope
                .areas
                .iter()
                .all(|a| a.name.starts_with("archguard-engine/"))
        );
        s.back();
        assert_eq!(s.within, None);
        assert!(s.history.is_empty());
    }

    /// Lenses change values and heights, not the entities; a KPI selects
    /// the Explore lens; Reset restores the defaults.
    #[test]
    fn lenses_and_search() {
        let mut s = repo_state();
        let count = s.layout.cells.len();
        s.set_kpi(1);
        assert_eq!(s.lens, Lens::Explore);
        assert!(s.kpi_active());
        assert_eq!(s.layout.cells.len(), count);
        s.query = "start_ui".into();
        s.accept_search();
        let sel = s.selection.clone().expect("a match");
        assert!(s.scope.areas[sel.area].name.ends_with("archguard-desktop"));
        // As the web: details open for a single entity or a file area, not
        // for a batched surface inside a package.
        let module = s.scope.areas[sel.area].module;
        assert_eq!(s.details_open, sel.entity.is_some() || !module);
        s.reset();
        assert_eq!((s.lens, s.kpi, s.query.as_str()), (Lens::Problems, 0, ""));
        assert_eq!(s.selection, None);
    }
}
