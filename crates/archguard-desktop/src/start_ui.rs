//! Start tab: the web viewer's "Your architecture" world view
//! (`viewer_world.js`, `viewer_world_model.js`). Packages are plates of
//! surfaces, one per class or file, drawn in an isometric projection that
//! is computed here ([`scene`]); Slint draws the resulting polygons.
//!
//! All tab state is one [`StartState`], built by [`Inputs::build`] and shown by
//! [`push`] as one `StartModel` value.

mod arrows;
mod details;
mod evidence;
mod follow_place;
mod layout;
mod modes;
mod overlay;
mod pins;
mod scene;
mod services;
mod services_view;
mod services_wire;
mod stage;
mod state;
mod view;

use std::path::PathBuf;

use archguard_engine::core::{Config, Finding, Graph};
use slint::ComponentHandle;

use crate::context::Loaded;
use crate::deferred::{Coalesce, FRAME, TYPING};
use crate::tab_support::{self, Actions, Calls, Shared, TabState, Topology, index};
use crate::ui::{AppWindow, StartData, StartModel};
use evidence::Evidence;
use follow_place::navigation;
use scene::Camera;
use services::ServicesWorld;
pub use state::{Lens, StartState};
use state::{Mode, from_evidence};

pub use view::push;

/// What the tab's build needs, copied from an analysis.
struct Inputs {
    calls: Calls,
    root: PathBuf,
    config: Config,
    graph: Graph,
    findings: Vec<Finding>,
    /// The document's topology, read once (the Services tab shares it).
    topology: Shared<Topology>,
}

impl Inputs {
    fn of(data: &Loaded) -> Self {
        Inputs {
            calls: Calls::of(data),
            root: data.root.to_path_buf(),
            config: data.run.config.clone(),
            graph: data.run.graph.clone(),
            findings: data.run.report.findings.clone(),
            topology: Shared::<Topology>::active(),
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
            topology,
        } = self;
        let calls = calls.get();
        let coverage = archguard_engine::read_test_coverage(&root, &config, &graph);
        let world = archguard_engine::architecture_world(&graph, &calls, &coverage);
        let services = ServicesWorld::new(tab_support::topology(&topology, &root));
        let mut state = from_evidence(Evidence::new(graph, findings, &calls, world), services);
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
    let back = navigation(ui, a.on(|s, _, ()| s.back()));
    d.on_go_back(move || back(()));
    d.on_crumb(navigation(
        ui,
        a.on(|s, ui, i: i32| {
            // With a services World, its crumb leads the path.
            let lead = i32::from(s.services.is_some());
            if i < lead {
                s.set_mode(Mode::Services);
                return;
            }
            let crumbs = s.crumbs(&ui.get_project());
            if let Some((_, scope)) = index(i - lead).and_then(|i| crumbs.get(i)) {
                s.navigate(scope.clone());
            }
        }),
    ));
    d.on_open_area(navigation(
        ui,
        a.on(|s, _, i: i32| s.pin(index(i).unwrap_or(0))),
    ));
    d.on_resized(
        a.on2(|s, _, w: f32, h: f32| s.canvas = (f64::from(w).max(1.0), f64::from(h).max(1.0))),
    );
    // A drag moves the camera at most once per frame, by the summed steps.
    let pan = a.on(|s, _, (dx, dy): (f32, f32)| {
        s.cam.x += f64::from(dx);
        s.cam.y += f64::from(dy);
    });
    let pan = Coalesce::throttle(FRAME, |a, b| (a.0 + b.0, a.1 + b.1), pan);
    d.on_pan(move |dx, dy| pan.call((dx, dy)));
    let wheel = a.on(|s, _, (x, y, delta): (f32, f32, f32)| {
        let delta = f64::from(delta).clamp(-180.0, 180.0);
        s.wheel(f64::from(x), f64::from(y), (delta * 0.005).exp())
    });
    let wheel = navigation(ui, wheel);
    d.on_wheel(move |x, y, delta| wheel((x, y, delta)));
    d.on_zoom_by(a.on(|s, _, f: f32| s.cam = scene::zoom(s.cam, f64::from(f), (s.cam.x, s.cam.y))));
    let rotate = a.on(|s, _, r: f32| s.cam = scene::rotate(s.cam, f64::from(r)));
    d.on_rotate_by(Coalesce::throttle(FRAME, |a, b| a + b, rotate).callback());
    d.on_orient(a.on0(|s, _| s.cam.rotation = 0.0));
    d.on_fit(a.on0(|s, _| {
        s.cam = Camera {
            rotation: s.cam.rotation,
            ..Camera::default()
        }
    }));
    let reset = navigation(
        ui,
        a.on(|s, _, ()| match s.mode {
            Mode::Services => s.reset_services(),
            Mode::Architecture => s.reset(),
        }),
    );
    d.on_reset(move || reset(()));
    wire_selection(ui, &d);
    services_wire::wire(ui, &d);
}

fn wire_selection(ui: &AppWindow, d: &StartData<'_>) {
    let a = Actions::<StartState>::new(ui);
    let search = a.on(|s, _, q: slint::SharedString| s.set_search(q.into()));
    let search = Coalesce::debounce(TYPING, search);
    d.on_search(search.callback());
    let accept = navigation(ui, a.on(|s, _, ()| s.accept_search()));
    // Enter selects the first match of what is typed, not of the last pause.
    d.on_accept_search(move || {
        search.flush();
        accept(());
    });
    let click = navigation(
        ui,
        a.on(|s, _, (x, y): (f32, f32)| s.click_at(f64::from(x), f64::from(y))),
    );
    d.on_click(move |x, y| click((x, y)));
    let double = navigation(
        ui,
        a.on(|s, _, (x, y): (f32, f32)| s.double_at(f64::from(x), f64::from(y))),
    );
    d.on_double_click(move |x, y| double((x, y)));
    // Entry 0 is "Select area…" ("Open project…" on the services).
    d.on_pick_area(navigation(ui, a.on(|s, _, i: i32| s.pick(i))));
    d.on_toggle_details(a.on0(|s, _| s.details_open = !s.details_open && s.has_selection()));
    d.on_pick_entity(navigation(
        ui,
        a.on(|s, _, e: i32| {
            if let Some(e) = index(e) {
                s.pick_entity(e);
            }
        }),
    ));
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
        crate::panes_ui::reveal(ui, n);
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
    // The architecture hooks show the architecture World.
    let architecture = [
        "ARCHGUARD_DESKTOP_WORLD",
        "ARCHGUARD_DESKTOP_PICK",
        "ARCHGUARD_DESKTOP_LENS",
    ];
    if architecture.iter().any(|k| var(k).is_some())
        || var("ARCHGUARD_DESKTOP_START_MODE").as_deref() == Some("architecture")
    {
        s.set_mode(Mode::Architecture);
    }
    services_wire::snapshot_overrides(s);
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
        s.select(state::Selection {
            area,
            entity: None,
            members: None,
        });
        s.details_open = true;
    }
}

#[cfg(not(feature = "snapshot"))]
fn snapshot_overrides(_: &mut StartState) {}

#[cfg(feature = "snapshot")]
impl StartState {
    /// The index of the shown area named `name` (snapshot hooks).
    pub fn area_named(&self, name: &str) -> Option<usize> {
        self.scope.areas.iter().position(|a| a.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use state::Mode;
    use std::path::Path;

    /// The Start state of the project at `root`.
    fn fixture_state_at(root: &Path) -> StartState {
        let root = root.canonicalize().expect("project");
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

    /// The Start state of a fixture project, and its top-level packages
    /// as the analysis reports them.
    fn fixture_state(name: &str) -> (StartState, Vec<String>) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(name)
            .canonicalize()
            .expect("fixture");
        let loaded = archguard_engine::load_config(None, &root).expect("config");
        let run =
            archguard_engine::analyze_run_quiet(&root, &loaded, archguard_engine::Level::Module)
                .expect("analysis");
        let packages = run
            .graph
            .nodes
            .iter()
            .filter(|n| n.parent.is_none() && n.kind == archguard_engine::core::NodeKind::Module)
            .map(|n| n.name.clone())
            .collect();
        let sources = Default::default();
        let state = Inputs::of(&Loaded {
            root: &root,
            run: &run,
            sources: &sources,
        })
        .build();
        (state, packages)
    }

    /// A project with several top-level packages: each is a package plate
    /// of the overview, every world entity lands on one surface, and
    /// opening a package and going back restores the overview.
    #[test]
    fn build_shows_the_packages_and_navigates() {
        let (mut s, mut packages) = fixture_state("py-layers");
        let mut names: Vec<_> = s.scope.areas.iter().map(|a| a.name.clone()).collect();
        assert_eq!(s.home, None, "{names:?}");
        names.sort();
        packages.sort();
        assert!(packages.len() > 1, "{packages:?}");
        assert_eq!(names, packages);
        assert!(s.scope.areas.iter().all(|a| a.module));
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

        let domain = s
            .scope
            .areas
            .iter()
            .position(|a| a.name == "domain")
            .expect("package domain");
        s.open_area(domain);
        assert_eq!(s.within.as_deref(), Some("domain"));
        assert_eq!(
            s.crumbs("repo"),
            vec![
                ("repo".to_string(), None),
                ("domain".to_string(), Some("domain".to_string()))
            ]
        );
        assert!(!s.scope.areas.is_empty());
        assert!(s.scope.areas.iter().all(|a| a.name.starts_with("domain/")));
        s.back();
        assert_eq!(s.within, None);
        assert!(s.history.is_empty());
    }

    /// Lenses change values and heights, not the entities; a KPI selects
    /// the Explore lens; Reset restores the defaults.
    #[test]
    fn lenses_and_search() {
        let (mut s, _) = fixture_state("py-layers");
        let count = s.layout.cells.len();
        s.set_kpi(1);
        assert_eq!(s.lens, Lens::Explore);
        assert!(s.kpi_active());
        assert_eq!(s.layout.cells.len(), count);
        s.set_search(" Ledger".into());
        assert_eq!(s.query, "ledger");
        s.accept_search();
        let sel = s.selection.clone().expect("a match");
        assert_eq!(s.scope.areas[sel.area].name, "domain", "domain/ledger.py");
        // As the web: details open for a single entity or a file area, not
        // for a batched surface inside a package.
        let module = s.scope.areas[sel.area].module;
        assert_eq!(s.details_open, sel.entity.is_some() || !module);
        s.reset();
        assert_eq!((s.lens, s.kpi, s.query.as_str()), (Lens::Problems, 0, ""));
        assert_eq!(s.search, "");
        assert_eq!(s.selection, None);
    }

    /// Two npm projects side by side.
    fn two_projects() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temp dir");
        for name in ["api", "web"] {
            let p = dir.path().join(name);
            std::fs::create_dir_all(p.join("src")).expect("dir");
            let manifest =
                format!(r#"{{"name": "{name}", "version": "1.0.0", "main": "src/index.js"}}"#);
            std::fs::write(p.join("package.json"), manifest).expect("manifest");
            std::fs::write(p.join("src/index.js"), "export const x = 1;\n").expect("source");
        }
        dir
    }

    /// A repository opens on its services World; zooming into a project
    /// shows its packages, and Back returns to the services.
    #[test]
    fn a_repository_opens_on_its_services() {
        let dir = two_projects();
        let mut s = fixture_state_at(dir.path());
        assert_eq!(s.mode, Mode::Services);
        let w = s.services.as_ref().expect("services World");
        let api = w
            .inventory
            .iter()
            .position(|i| i.path == "api")
            .expect("the api project is an island");
        let area = w.area(api).expect("shown");
        assert_eq!(s.layout.plates.len(), w.shown.len());
        s.pin(area);
        assert_eq!(s.services.as_ref().unwrap().selected, Some(api));
        assert!(
            s.has_selection() && !s.details_open,
            "a click selects, as the web"
        );
        let units = crate::documents::build_units(s.services.as_ref().unwrap().topology());
        assert_eq!(
            s.project_unit().map(|i| units[i].path.as_str()),
            Some("api")
        );
        s.enter_area(area);
        assert_eq!(s.mode, Mode::Architecture);
        assert_eq!(s.within.as_deref(), Some("api"));
        assert!(!s.scope.areas.is_empty());
        assert!(s.scope.areas.iter().all(|a| a.name.starts_with("api/")));
        s.back();
        assert_eq!(s.mode, Mode::Services);
        assert_eq!(
            s.layout.plates.len(),
            s.services.as_ref().unwrap().shown.len()
        );
        s.set_mode(Mode::Architecture);
        assert_eq!((s.mode, s.entered), (Mode::Architecture, false));
    }
}
