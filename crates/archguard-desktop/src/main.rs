//! `archguard-desktop`: native viewer with the web viewer's layout.
//!
//! Usage: `archguard-desktop [PROJECT_DIR]` (default: the current
//! directory). The analysis runs on a worker thread with the same
//! cached pipeline as the CLI; the window fills once it is done.

mod layout;
mod model;
mod state;
mod tree;
mod web;

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use archguard_engine::core::{ReviewPriority, ReviewSources};
use archguard_engine::{
    AnalysisRun, Level, analyze_run_quiet, collect_review_sources, load_config,
};
use slint::{Color, ModelRc, SharedString, VecModel};

use model::{Module, Overview};
use state::GraphState;
use tree::{EdgeMode, Tree};

slint::include_modules!();

thread_local! {
    static APP: RefCell<Option<GraphState>> = const { RefCell::new(None) };
}

/// Run `f` on the graph state and push the Graph tab into the window.
fn update(ui: &AppWindow, f: impl FnOnce(&mut GraphState)) {
    APP.with_borrow_mut(|app| {
        if let Some(g) = app {
            f(g);
            push_graph(ui, g);
        }
    });
}

fn main() -> Result<(), slint::PlatformError> {
    let root = std::env::args_os()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().expect("current directory is readable"));
    let root = root.canonicalize().unwrap_or(root);
    let project = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.display().to_string());

    let ui = AppWindow::new()?;
    // Slint selects bundled translations only after the first component.
    select_language();
    ui.set_project(project.clone().into());
    ui.set_root_path(root.display().to_string().into());
    ui.set_status(root.display().to_string().into());
    if let Ok(theme) = std::env::var("ARCHGUARD_DESKTOP_THEME") {
        ui.invoke_set_dark(theme != "light");
    }
    wire(&ui, &root);

    let weak = ui.as_weak();
    std::thread::spawn(move || {
        let result = load_config(None, &root)
            .and_then(|loaded| analyze_run_quiet(&root, &loaded, Level::Module))
            .map(|run| {
                // Git evidence for review priorities; without it the
                // churn component is zero, as in the CLI.
                let sources = collect_review_sources(
                    &root,
                    &run.config,
                    &run.graph,
                    &run.config.review.git_since,
                )
                .unwrap_or_default();
                (run, sources)
            })
            .map_err(|e| e.to_string());
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            match result {
                Ok((run, sources)) => show(&ui, project, &run, sources),
                Err(e) => {
                    ui.set_loading(false);
                    ui.set_error(e.into());
                }
            }
        });
    });

    let result = ui.run();
    web::shutdown();
    result
}

fn wire(ui: &AppWindow, root: &Path) {
    let w = ui.as_weak();
    ui.on_select_row(move |r| {
        if let Some(ui) = w.upgrade() {
            update(&ui, |g| g.select_row(usize::try_from(r).ok()));
        }
    });
    let w = ui.as_weak();
    ui.on_toggle_row(move |r| {
        if let Some(ui) = w.upgrade() {
            update(&ui, |g| g.toggle_row(r.max(0) as usize));
        }
    });
    let w = ui.as_weak();
    ui.on_expand_all(move || {
        if let Some(ui) = w.upgrade() {
            update(&ui, GraphState::expand_all);
        }
    });
    let w = ui.as_weak();
    ui.on_collapse_all(move || {
        if let Some(ui) = w.upgrade() {
            update(&ui, GraphState::collapse_all);
        }
    });
    let w = ui.as_weak();
    ui.on_reveal(move |n| {
        if let Some(ui) = w.upgrade() {
            update(&ui, |g| g.reveal(n.max(0) as usize));
        }
    });
    let w = ui.as_weak();
    ui.on_set_edge_mode(move |m| {
        if let Some(ui) = w.upgrade() {
            update(&ui, |g| g.mode = edge_mode(m));
        }
    });
    let w = ui.as_weak();
    ui.on_search(move |q| {
        let Some(ui) = w.upgrade() else { return };
        APP.with_borrow(|app| {
            let Some(g) = app else { return };
            ui.set_searching(!q.trim().is_empty());
            ui.set_hits(vec_model(g.search(&q, 300).into_iter().map(|h| Hit {
                node: h.node as i32,
                label: h.label.into(),
                scope: h.scope.into(),
            })));
        });
    });
    let w = ui.as_weak();
    let root = root.to_path_buf();
    ui.on_open_web(move |tab| open_web(&w, &root, tab.as_str()));
}

fn open_web(w: &slint::Weak<AppWindow>, root: &Path, tab: &str) {
    let Some(ui) = w.upgrade() else { return };
    ui.set_web_status(SharedString::from("… archguard serve"));
    let (w, root, tab) = (w.clone(), root.to_path_buf(), tab.to_string());
    std::thread::spawn(move || {
        let msg = match web::open(&root, &tab) {
            Ok(url) => url,
            Err(e) => format!("archguard serve: {e}"),
        };
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(ui) = w.upgrade() {
                ui.set_web_status(msg.into());
            }
        });
    });
}

fn edge_mode(m: i32) -> EdgeMode {
    match m {
        0 => EdgeMode::Incoming,
        1 => EdgeMode::Outgoing,
        2 => EdgeMode::Both,
        4 => EdgeMode::Cycles,
        _ => EdgeMode::All,
    }
}

fn mode_index(m: EdgeMode) -> i32 {
    match m {
        EdgeMode::Incoming => 0,
        EdgeMode::Outgoing => 1,
        EdgeMode::Both => 2,
        EdgeMode::All => 3,
        EdgeMode::Cycles => 4,
    }
}

fn vec_model<T: Clone + 'static>(items: impl IntoIterator<Item = T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items.into_iter().collect::<Vec<_>>()))
}

fn show(ui: &AppWindow, project: String, run: &AnalysisRun, sources: ReviewSources) {
    let graph = GraphState::with_review(
        Tree::from_graph(&run.graph),
        sources,
        run.config.review.entry_points.clone(),
    );
    ui.set_node_total(graph.tree.nodes.len() as i32);
    ui.set_cycles(vec_model(
        graph
            .cycles(&run.report.findings)
            .into_iter()
            .map(|c| CycleItem {
                first_node: c.first_node as i32,
                label: c.label.into(),
                size: c.size as i32,
            }),
    ));
    show_layers(ui, &Overview::from_run(project, run));
    push_graph(ui, &graph);
    APP.set(Some(graph));
    ui.set_loading(false);
    ui.invoke_fit_views();
    #[cfg(feature = "snapshot")]
    snapshot::schedule(ui);
}

fn push_graph(ui: &AppWindow, g: &GraphState) {
    let v = &g.view;
    let selected = g.selected_row();
    let covered = selected.map(|s| v.selection(s)).unwrap_or_default();
    ui.set_rows(vec_model(v.rows.iter().enumerate().map(|(i, r)| {
        let n = &g.tree.nodes[r.node];
        TreeRow {
            row: i as i32,
            node: r.node as i32,
            label: n.label.clone().into(),
            depth: r.depth as i32,
            expandable: r.expandable,
            expanded: r.expanded,
            file: n.file,
            in_count: v.incoming[i] as i32,
            out_count: v.outgoing[i] as i32,
            in_cycle: v.in_cycle[i],
            covered: covered.contains(&i),
            priority: match g.priorities[i] {
                Some(ReviewPriority::High) => 1,
                Some(ReviewPriority::Medium) => 2,
                _ => 0,
            },
        }
    })));
    ui.set_links(vec_model(v.draw(selected, g.mode).into_iter().map(
        |(path, kind)| TreeLink {
            path: path.into(),
            kind: kind as i32,
        },
    )));
    ui.set_selected_row(selected.map_or(-1, |s| s as i32));
    ui.set_edge_mode(mode_index(g.mode));
    ui.set_dep_count(v.links.len() as i32);
    ui.set_cycle_groups(v.cycle_groups as i32);
    ui.set_acd(format!("{:.2}", v.acd(&g.tree)).into());
    let (cin, cout, call) = g.counts();
    ui.set_count_in(cin as i32);
    ui.set_count_out(cout as i32);
    ui.set_count_all(call as i32);
    push_selection(ui, g);
}

fn push_selection(ui: &AppWindow, g: &GraphState) {
    let mut crumbs = vec![Crumb {
        node: -1,
        label: ui.get_project(),
    }];
    crumbs.extend(g.crumbs().into_iter().map(|n| Crumb {
        node: n as i32,
        label: g.tree.nodes[n].label.clone().into(),
    }));
    ui.set_crumbs(vec_model(crumbs));

    let node = g.selected.map(|n| &g.tree.nodes[n]);
    ui.set_reference(node.map(|n| n.name.clone()).unwrap_or_default().into());
    ui.set_selected_label(node.map(|n| n.label.clone()).unwrap_or_default().into());
    let (inc, out) = g.neighbors();
    let neighbors = |list: Vec<state::Neighbor>| {
        vec_model(list.into_iter().map(|n| Neighbor {
            row: n.row as i32,
            label: g.tree.nodes[g.view.rows[n.row].node].label.clone().into(),
            weight: n.weight.min(i32::MAX as u64) as i32,
        }))
    };
    ui.set_incoming(neighbors(inc));
    ui.set_outgoing(neighbors(out));
}

fn show_layers(ui: &AppWindow, o: &Overview) {
    ui.set_modules(vec_model(o.modules.iter().map(module_item)));
    ui.set_edges(vec_model(o.deps.iter().map(|d| EdgeItem {
        from: d.from as i32,
        to: d.to as i32,
        weight: d.weight.min(i32::MAX as u64) as i32,
        path: d.path.clone().into(),
        cyclic: d.cyclic,
    })));
    ui.set_layers_w(o.width);
    ui.set_layers_h(o.height);
}

fn module_item(m: &Module) -> ModuleItem {
    ModuleItem {
        name: m.name.clone().into(),
        label: m.label.clone().into(),
        scope: m.scope.clone().into(),
        fan_in: m.fan_in as i32,
        fan_out: m.fan_out as i32,
        instability: m.instability,
        in_cycle: m.in_cycle,
        x: m.x,
        y: m.y,
        bright: true,
        accent: accent(m),
    }
}

/// Green for stable, amber for unstable modules (the web's m-low/m-medium).
fn accent(m: &Module) -> Color {
    let t = m.instability.clamp(0.0, 1.0);
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    Color::from_rgb_u8(mix(0x16, 0xc5), mix(0x84, 0x8a), mix(0x5b, 0x23))
}

/// German when the system language is German, else English.
/// Call after creating the window.
fn select_language() {
    let lang = ["ARCHGUARD_DESKTOP_LANG", "LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
        .or_else(macos_language)
        .unwrap_or_default();
    if lang.to_lowercase().starts_with("de") {
        let _ = slint::select_bundled_translation("de");
    }
}

fn macos_language() -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let out = std::process::Command::new("defaults")
        .args(["read", "-g", "AppleLanguages"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    Some(text.split('"').nth(1)?.to_string())
}

#[cfg(feature = "snapshot")]
mod snapshot {
    use super::AppWindow;
    use slint::ComponentHandle;

    /// After the layout settled: optionally reveal a node by name
    /// (`ARCHGUARD_DESKTOP_REVEAL=<name>`), switch tab
    /// (`ARCHGUARD_DESKTOP_TAB`), render, write the PNG, quit.
    pub fn schedule(ui: &AppWindow) {
        let Some(path) = std::env::var_os("ARCHGUARD_DESKTOP_SNAPSHOT") else {
            return;
        };
        if let Ok(name) = std::env::var("ARCHGUARD_DESKTOP_REVEAL") {
            let node = super::APP.with_borrow(|app| {
                app.as_ref()
                    .and_then(|g| g.tree.nodes.iter().position(|n| n.name == name))
            });
            if let Some(n) = node {
                ui.invoke_reveal(n as i32);
            }
        }
        if let Ok(tab) = std::env::var("ARCHGUARD_DESKTOP_TAB") {
            ui.set_tab(tab.into());
        }
        let weak = ui.as_weak();
        slint::Timer::single_shot(std::time::Duration::from_millis(900), move || {
            let Some(ui) = weak.upgrade() else { return };
            let shot = ui
                .window()
                .take_snapshot()
                .expect("renderer supports snapshots");
            let file = std::fs::File::create(&path).expect("snapshot file is writable");
            let mut enc = png::Encoder::new(file, shot.width(), shot.height());
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut writer = enc.write_header().expect("png header");
            writer.write_image_data(shot.as_bytes()).expect("png data");
            drop(writer);
            let _ = slint::quit_event_loop();
        });
    }
}
