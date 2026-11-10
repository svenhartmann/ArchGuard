//! Start tab: the web viewer's "Your architecture" world view
//! (`viewer_world.js`, `viewer_world_model.js`). Packages are plates of
//! surfaces, one per class or file, drawn in an isometric projection that
//! is computed here ([`scene`]); Slint draws the resulting polygons.
//!
//! All tab state is one [`StartState`], built by [`Inputs::build`] and shown by
//! [`push`] as one `StartModel` value.

mod actions;
mod arrows;
mod camera_wire;
mod class_detail;
mod connections;
mod coverage_detail;
mod details;
mod evidence;
mod follow_place;
mod hover;
mod keys;
mod kpis;
mod kpis_profile;
mod kpis_zones;
mod layout;
mod lines;
mod memo;
mod modes;
mod motion;
#[cfg(test)]
mod narrow_check;
mod overlay;
mod pins;
mod scene;
mod services;
mod services_view;
mod services_wire;
mod source_deps;
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
use services::ServicesWorld;
pub use state::{Lens, StartState};
use state::{Mode, from_evidence};

#[cfg(feature = "snapshot")]
pub use actions::snapshot_hook;
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
    /// The World view kept with the pane layout.
    memo: memo::Memo,
    /// What the A/I scatter's performance report needs.
    zones: kpis_zones::Inputs,
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
            memo: memo::kept(),
            zones: kpis_zones::Inputs::of(data),
        }
    }

    /// The tab state: the same engine calls as the CLI's report
    /// (`review_cmd.rs`). Slow; runs on a worker thread.
    fn build(self) -> StartState {
        let _span = archguard_engine::self_trace::tracing::info_span!("build").entered();
        let Inputs {
            calls,
            root,
            config,
            graph,
            findings,
            topology,
            memo,
            zones,
        } = self;
        let calls = calls.get();
        let coverage = archguard_engine::read_test_coverage(&root, &config, &graph);
        let world = archguard_engine::architecture_world(&graph, &calls, &coverage);
        let review = archguard_engine::test_coverage_review(&graph, &coverage);
        let snapshots = archguard_engine::read_snapshots(&root);
        let mut glance = kpis::Glance::new(graph.review_kpis.clone(), &findings, &snapshots);
        glance.abstractions = kpis_zones::apply(glance.kpis.as_mut(), zones.measure(&calls));
        let services = ServicesWorld::new(tab_support::topology(&topology, &root));
        let mut state = from_evidence(Evidence::new(graph, findings, &calls, world), services);
        state.glance = glance;
        state.files = coverage_detail::Files::new(&coverage, &review);
        state.root = root.display().to_string();
        state.restore(&memo);
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
    // A drag or the wheel stops a glide and moves at once.
    let pan = a.on(|s, _, (dx, dy): (f32, f32)| {
        camera_wire::stop();
        s.cam.x += f64::from(dx);
        s.cam.y += f64::from(dy);
    });
    let pan = Coalesce::throttle(FRAME, |a, b| (a.0 + b.0, a.1 + b.1), pan);
    d.on_pan(move |dx, dy| pan.call((dx, dy)));
    let wheel = a.on(|s, _, (x, y, delta): (f32, f32, f32)| {
        camera_wire::stop();
        let delta = f64::from(delta).clamp(-180.0, 180.0);
        s.wheel(f64::from(x), f64::from(y), (delta * 0.005).exp())
    });
    let wheel = navigation(ui, wheel);
    d.on_wheel(move |x, y, delta| wheel((x, y, delta)));
    let rotate = a.on(|s, _, r: f32| {
        camera_wire::stop();
        s.cam = scene::rotate(s.cam, f64::from(r));
    });
    d.on_rotate_by(Coalesce::throttle(FRAME, |a, b| a + b, rotate).callback());
    camera_wire::wire(ui, &d);
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
    follow_place::wire();
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
    d.on_toggle_entry(a.on(|s, _, i: i32| {
        if let Some(i) = index(i) {
            s.toggle_entry(i);
        }
    }));
    d.on_toggle_section(a.on2(|s, _, key: slint::SharedString, open: bool| {
        s.sections.insert(key.into(), !open);
    }));
    d.on_pick_neighbor(navigation(
        ui,
        a.on(|s, _, area: i32| {
            if let Some(area) = index(area) {
                s.select_neighbor(area);
            }
        }),
    ));
    wire_hover(ui, d);
    d.on_detail_action(tab_support::with_ui(ui, detail_action));
    d.on_open_source(tab_support::with_ui(ui, |ui, name: slint::SharedString| {
        let node = crate::documents::with_active_state(|s: &mut StartState| {
            s.ev.graph
                .nodes
                .iter()
                .position(|n| n.name == name.as_str())
        });
        if let Some(Some(n)) = node {
            crate::panes_ui::reveal(ui, n);
        }
    }));
}

/// Cycles, Matrix, Calls, Class metrics or Quality scope for the
/// selection; the Start state is released before the other view runs.
fn detail_action(ui: &AppWindow, which: i32) {
    let target = crate::documents::with_active_state(|s: &mut StartState| {
        let t = s.action_target()?;
        let node = s.ev.graph.nodes.iter().position(|n| n.name == t.node);
        Some((t, node))
    })
    .flatten();
    if let Some((t, node)) = target {
        actions::run(ui, t, node, which);
    }
}

/// The surface tooltip follows the pointer at most once per frame; it
/// changes no state, so nothing else is pushed.
fn wire_hover(ui: &AppWindow, d: &StartData<'_>) {
    let at = tab_support::with_ui(ui, |ui, p: Option<(f32, f32)>| {
        let p = p.map(|(x, y)| (f64::from(x), f64::from(y)));
        crate::documents::with_active_state(|s: &mut StartState| hover::show(ui, s, p));
    });
    let at = Coalesce::throttle(FRAME, |_, b| b, at);
    let moved = at.clone();
    d.on_hover_at(move |x, y| moved.call(Some((x, y))));
    d.on_hover_end(move || at.call(None));
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
        memo::keep(ui, self);
    }

    /// As the web: without a selection the details close, so the next
    /// selection does not show them unasked.
    fn changed(&mut self) {
        self.details_open &= self.has_selection();
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
/// `ARCHGUARD_DESKTOP_PICK=<area name>` set the view for snapshot runs;
/// `ARCHGUARD_DESKTOP_START_FIND` selects a search match and
/// `ARCHGUARD_DESKTOP_START_GLANCE` shows the KPIs below the World.
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
    // `ARCHGUARD_DESKTOP_START_FIND=<term>`: search and select the first
    // match, as Enter in the search field.
    if let Some(term) = var("ARCHGUARD_DESKTOP_START_FIND") {
        s.set_search(term);
        s.accept_search();
    }
    // `ARCHGUARD_DESKTOP_START_GLANCE=1`: scroll to the KPIs; `=profile`
    // also opens the architecture profile.
    if let Some(g) = var("ARCHGUARD_DESKTOP_START_GLANCE") {
        s.glance.reveal = true;
        s.glance.profile = g == "profile";
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
        assert!(!s.details_open, "Enter in the search selects only");
        s.reset();
        assert_eq!((s.lens, s.kpi, s.query.as_str()), (Lens::Problems, 0, ""));
        assert_eq!(s.search, "");
        assert_eq!(s.selection, None);
    }

    /// The tooltip names the surface under the pointer: a single entity by
    /// its class or file, a group by its size, with the lens value; off
    /// the surfaces there is none.
    #[test]
    fn hover_names_the_surface_under_the_pointer() {
        let (mut s, _) = fixture_state("py-layers");
        let v = s.view().expect("view");
        let at = |c: &layout::Cell| v.screen(c.x as f64 + 0.5, c.y as f64 + 0.5, c.z);
        let (i, cell) = s
            .layout
            .cells
            .iter()
            .enumerate()
            .find(|(_, c)| c.entities.len() == 1)
            .expect("a single surface");
        let (x, y) = at(cell);
        let h = s.hover_at(x, y).expect("a surface");
        assert_eq!(h.cell, i);
        let e = s.ev.entity(cell.entities[0]);
        assert_eq!(h.title, e.class_id.clone().unwrap_or(e.node.clone()));
        assert_eq!((h.grouped, h.kpi, h.value), (0, 0, cell.value));
        assert_eq!(s.hover_at(-50.0, -50.0), None);
        s.set_kpi(2);
        assert_eq!(s.hover_at(x, y).expect("a surface").kpi, 2);
    }

    /// A connection in the details opens and closes; its neighbour is
    /// selected with the details kept open.
    #[test]
    fn a_connection_selects_its_neighbour() {
        let (mut s, _) = fixture_state("py-layers");
        let c = s.scope.connections.first().cloned().expect("a connection");
        s.pick(c.from as i32 + 1);
        s.details_open = true;
        assert!(!s.connection_open(0));
        s.toggle_connection(0);
        assert!(s.connection_open(0));
        s.select_neighbor(c.to);
        let sel = s.selection.clone().expect("the neighbour");
        assert_eq!((sel.area, sel.entity), (c.to, None));
        assert!(s.details_open, "the details stay open");
        assert!(s.connection_open(0), "open entries stay open");
        s.toggle_connection(0);
        assert!(!s.connection_open(0));
    }

    /// Selecting (a click, the search, a neighbour, "Follow left") only
    /// selects; the details open on request (Details, Enter, a file's
    /// double click), then stay open and follow the selection until
    /// closed, which keeps the selection. No selection, no details.
    #[test]
    fn the_details_open_on_request() {
        use crate::follow::Nav;
        let (mut s, _) = fixture_state("py-layers");
        let cell = (0..s.layout.cells.len())
            .find(|&c| s.layout.cells[c].entities.len() == 1)
            .expect("a single surface");
        s.select_cell(cell);
        assert!(s.has_selection() && !s.details_open, "a click selects");
        s.follow(&Nav::Select("domain".into()));
        assert!(s.has_selection() && !s.details_open, "a follow selects");
        s.details_open = true;
        let c = s.scope.connections.first().cloned().expect("a connection");
        s.select_neighbor(c.to);
        s.changed();
        assert!(s.details_open, "open details follow the selection");
        s.details_open = false;
        assert!(s.has_selection(), "closing keeps the selection");
        s.details_open = true;
        s.pick(0);
        s.changed();
        assert!(!s.details_open, "no selection, no details");
        s.navigate(Some("domain".into()));
        let file = s
            .scope
            .areas
            .iter()
            .position(|a| !a.module)
            .expect("a file");
        s.open_area(file);
        assert!(s.details_open, "a file's double click shows its details");
    }

    /// Copy path gives `file:line` of an entity, else the area; AI context
    /// the web's three read-only MCP requests for the selection.
    #[test]
    fn copy_path_and_ai_context() {
        let (mut s, _) = fixture_state("py-layers");
        assert_eq!(
            (s.copy_value(), s.ai_context()),
            (String::new(), String::new())
        );
        let domain = s
            .scope
            .areas
            .iter()
            .position(|a| a.name == "domain")
            .expect("domain");
        s.pick(domain as i32 + 1);
        assert_eq!(s.copy_value(), "domain");
        let t = s.action_target().expect("a target");
        assert_eq!((t.node.as_str(), t.module), ("domain", true));
        let e = s.scope.areas[domain].entities[0];
        s.pick_entity(e);
        let entity = s.ev.entity(e).clone();
        let expected = match (&entity.file, entity.line) {
            (Some(f), Some(l)) => format!("{f}:{l}"),
            (Some(f), None) => f.clone(),
            _ => entity.node.clone(),
        };
        assert_eq!(s.copy_value(), expected);
        let v: serde_json::Value = serde_json::from_str(&s.ai_context()).expect("JSON");
        let tools: Vec<&str> = v["tools"]
            .as_array()
            .expect("tools")
            .iter()
            .map(|t| t["name"].as_str().unwrap_or(""))
            .collect();
        assert_eq!(
            tools,
            [
                "architecture_world",
                "architecture_atlas",
                "architecture_findings"
            ]
        );
        assert_eq!(v["tools"][0]["arguments"]["within"], entity.node.as_str());
        assert_eq!(v["tools"][1]["arguments"]["within"], "domain");
        assert_eq!(v["project"], s.root.as_str());
        assert!(!s.root.is_empty());
    }

    /// The kept view comes back in a new state of the same project; a
    /// package or area that no longer exists is left out.
    #[test]
    fn the_world_view_is_kept_and_restored() {
        let (mut s, _) = fixture_state("py-layers");
        assert_eq!(s.memo(), Vec::new(), "defaults are not kept");
        s.set_kpi(3);
        let domain = s.scope.areas.iter().position(|a| a.name == "domain");
        s.pick(domain.expect("domain") as i32 + 1);
        let memo = s.memo();
        let (mut back, _) = fixture_state("py-layers");
        back.restore(&memo);
        assert_eq!((back.lens, back.kpi), (Lens::Explore, 3));
        let sel = back.selection.clone().expect("the area");
        assert_eq!(back.scope.areas[sel.area].name, "domain");
        assert_eq!(back.memo(), memo);
        let gone: memo::Memo = vec![
            ("within".into(), "no/such/package".into()),
            ("area".into(), "gone".into()),
            ("lens".into(), "2".into()),
        ];
        let (mut other, _) = fixture_state("py-layers");
        other.restore(&gone);
        assert_eq!(
            (other.within.clone(), other.selection.clone()),
            (None, None)
        );
        assert_eq!(other.lens, Lens::Coverage);
    }

    /// The services World keeps its filters, the architecture mode and
    /// the selected island.
    #[test]
    fn the_services_view_is_kept_and_restored() {
        let dir = two_projects();
        let mut s = fixture_state_at(dir.path());
        let api = {
            let w = s.services.as_mut().expect("services");
            w.filter.cycles_only = true;
            w.filter.direction = services::Direction::Outgoing;
            let api = w
                .inventory
                .iter()
                .position(|i| i.path == "api")
                .expect("api");
            w.selected = Some(api);
            api
        };
        let memo = s.memo();
        let mut back = fixture_state_at(dir.path());
        back.restore(&memo);
        let w = back.services.as_ref().expect("services");
        assert!(w.filter.cycles_only && !w.filter.flow);
        assert_eq!(w.filter.direction, services::Direction::Outgoing);
        assert_eq!((w.selected, back.mode), (Some(api), Mode::Services));
        s.set_mode(Mode::Architecture);
        let mut arch = fixture_state_at(dir.path());
        arch.restore(&s.memo());
        assert_eq!(arch.mode, Mode::Architecture);
    }

    /// Beside a Graph, the World enters its package and selects its node:
    /// an area by name, else the entity of a file or class node.
    #[test]
    fn the_world_follows_a_graph() {
        use crate::follow::Nav;
        let (mut s, _) = fixture_state("py-layers");
        s.follow(&Nav::Select("domain".into()));
        let sel = s.selection.clone().expect("the area");
        assert_eq!(s.scope.areas[sel.area].name, "domain");
        s.follow(&Nav::Scope(Some("domain".into())));
        assert_eq!(s.within.as_deref(), Some("domain"));
        let e = s.scope.areas[0].entities[0];
        let node = s.ev.entity(e).node.clone();
        s.follow(&Nav::Select(node.clone()));
        let sel = s.selection.clone().expect("a selection");
        let picked = sel.entity.map(|e| s.ev.entity(e).node.clone());
        let area = s.scope.areas[sel.area].name.clone();
        assert!(picked.as_deref() == Some(&node) || area == node, "{node}");
        s.follow(&Nav::Scope(None));
        assert_eq!(s.within, None);
        s.follow(&Nav::Select("no/such/node".into()));
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

    fn fixture_root(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures")
            .join(name)
    }

    /// The kinds of the detail lines of the current selection.
    fn detail_kinds(s: &StartState) -> Vec<i32> {
        use slint::Model;
        let mut m = StartModel::default();
        details::fill(&mut m, s, crate::format::Locale::En);
        m.detail_lines.iter().map(|l| l.kind).collect()
    }

    /// The glance carries the analysis' review KPIs: the fixture's cycle,
    /// SDP violation and leak are cards that open their graph node.
    #[test]
    fn the_glance_shows_the_review_kpis_of_the_analysis() {
        let s = fixture_state_at(&fixture_root("review-kpis"));
        assert!(s.glance.kpis.is_some());
        let (cards, _) = kpis::cards(&s.glance, crate::format::Locale::En);
        let card = |k: i32| cards.iter().find(|c| c.kind == k).expect("card");
        let cycle = card(kpis::card::PROPAGATION);
        assert!(!cycle.empty && !cycle.node.is_empty(), "{cycle:?}");
        assert_ne!(card(kpis::card::SDP).value, "0");
        assert_ne!(card(kpis::card::LEAKS).value, "0");
    }

    /// A class in Explore shows its rating, other metrics and methods; the
    /// Test gaps lens adds the file's coverage section.
    #[test]
    fn class_and_file_details_go_into_depth() {
        let mut s = fixture_state_at(&fixture_root("java-packages"));
        let e = (0..s.ev.world.entities.len())
            .find(|&e| {
                s.ev.entity(e)
                    .class_id
                    .as_ref()
                    .is_some_and(|id| s.ev.classes.contains_key(id))
            })
            .expect("a measured class");
        let area = s
            .scope
            .areas
            .iter()
            .position(|a| a.entities.contains(&e))
            .expect("its area");
        s.set_kpi(1);
        s.select(state::Selection {
            area,
            entity: Some(e),
            members: None,
        });
        let kinds = detail_kinds(&s);
        for k in [
            lines::kind::KPI_NOTE,
            lines::kind::OTHER_METRICS,
            lines::kind::METHODS,
        ] {
            assert!(kinds.contains(&k), "{k} in {kinds:?}");
        }
        s.set_lens(Lens::Coverage);
        let kinds = detail_kinds(&s);
        assert!(kinds.contains(&lines::kind::COV_HEAD), "{kinds:?}");
        assert!(
            kinds.contains(&lines::kind::COV_NONE),
            "no coverage imported"
        );
    }
}
