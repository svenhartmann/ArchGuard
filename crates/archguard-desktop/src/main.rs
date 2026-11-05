//! `archguard-desktop`: native viewer with the web viewer's layout.
//!
//! Usage: `archguard-desktop [PROJECT_DIR]` (default: the current
//! directory). The analysis runs on a worker thread with the same
//! cached pipeline as the CLI; the window fills once it is done.

mod network;
mod state;
mod tree;
mod tree_order;
mod web;

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use archguard_engine::core::{ReviewPriority, ReviewSources};
use archguard_engine::{
    AnalysisRun, Level, analyze_run_quiet, collect_review_sources, load_config,
};
use slint::{ModelRc, SharedString, VecModel};

use state::GraphState;
use tree::{EdgeMode, Sort, Tree};

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
                Ok((run, sources)) => show(&ui, &run, sources),
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

/// A callback that runs `f` on the graph state and refreshes the window.
fn graph_action<A: 'static>(
    ui: &AppWindow,
    f: impl Fn(&mut GraphState, A) + 'static,
) -> impl Fn(A) + 'static {
    let w = ui.as_weak();
    move |a| {
        if let Some(ui) = w.upgrade() {
            update(&ui, |g| f(g, a));
        }
    }
}

fn node_of_row(g: &GraphState, r: i32) -> Option<usize> {
    usize::try_from(r)
        .ok()
        .and_then(|r| g.view.rows.get(r))
        .map(|r| r.node)
}

fn wire(ui: &AppWindow, root: &Path) {
    ui.on_select_row(graph_action(ui, |g, r: i32| {
        g.select_row(usize::try_from(r).ok())
    }));
    ui.on_toggle_row(graph_action(ui, |g, r: i32| {
        g.toggle_row(r.max(0) as usize)
    }));
    let expand = graph_action(ui, |g, ()| g.expand_all());
    ui.on_expand_all(move || expand(()));
    let collapse = graph_action(ui, |g, ()| g.collapse_all());
    ui.on_collapse_all(move || collapse(()));
    let reset = graph_action(ui, |g, ()| g.reset());
    ui.on_reset_tree(move || reset(()));
    ui.on_reveal(graph_action(ui, |g, n: i32| g.reveal(n.max(0) as usize)));
    ui.on_open_node(graph_action(ui, |g, n: i32| g.open(n.max(0) as usize)));
    ui.on_open_row(graph_action(ui, |g, r: i32| {
        if let Some(node) = node_of_row(g, r) {
            g.open(node);
        }
    }));
    ui.on_set_edge_mode(graph_action(ui, |g, m: i32| g.mode = edge_mode(m)));
    ui.on_set_sort(graph_action(ui, |g, by_deps: bool| {
        g.set_sort(if by_deps {
            Sort::Connections
        } else {
            Sort::Alphabetical
        });
    }));
    wire_search(ui);
    let w = ui.as_weak();
    let root = root.to_path_buf();
    ui.on_open_web(move |tab| open_web(&w, &root, tab.as_str()));
}

fn wire_search(ui: &AppWindow) {
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

fn show(ui: &AppWindow, run: &AnalysisRun, sources: ReviewSources) {
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
    ui.set_rows(tree_rows(g, &covered));
    ui.set_links(vec_model(v.draw(selected, g.mode).into_iter().map(|d| {
        TreeLink {
            path: d.path.into(),
            kind: d.kind as i32,
            from: d.from as i32,
            to: d.to as i32,
            right: d.right,
        }
    })));
    ui.set_deps(vec_model(v.shown(selected, g.mode).into_iter().map(
        |(i, lane)| {
            let l = &v.links[i];
            let kind = v.link_kind(i, lane, selected, g.mode) as i32;
            let name = |row: usize| g.tree.nodes[v.rows[row].node].name.clone();
            DepCard {
                from: name(l.from).into(),
                to: name(l.to).into(),
                weight: l.weight.min(i32::MAX as u64) as i32,
                kind,
                refs: l
                    .refs
                    .iter()
                    .map(|r| format!("{}:{}  {}", r.file, r.line, r.raw.trim()))
                    .collect::<Vec<_>>()
                    .join("\n")
                    .into(),
                ref_count: l.refs.len() as i32,
            }
        },
    )));
    push_network(ui, g, selected);
    ui.set_selected_row(selected.map_or(-1, |s| s as i32));
    ui.set_edge_mode(mode_index(g.mode));
    ui.set_sort_by_deps(g.sort == Sort::Connections);
    ui.set_dep_count(v.links.len() as i32);
    ui.set_cycle_groups(v.cycle_groups as i32);
    ui.set_acd(format!("{:.2}", v.acd(&g.tree)).into());
    let (cin, cout, cboth, call) = g.counts();
    ui.set_count_in(cin as i32);
    ui.set_count_out(cout as i32);
    ui.set_count_both(cboth as i32);
    ui.set_count_all(call as i32);
    ui.set_scope_nodes(v.scope_rows().len() as i32);
    push_selection(ui, g);
}

/// The tree rows for the window; a compressed chain reads as its path.
fn tree_rows(g: &GraphState, covered: &BTreeSet<usize>) -> ModelRc<TreeRow> {
    let v = &g.view;
    vec_model(v.rows.iter().enumerate().map(|(i, r)| {
        let n = &g.tree.nodes[r.node];
        TreeRow {
            row: i as i32,
            node: r.node as i32,
            label: r
                .chain
                .iter()
                .map(|&c| g.tree.nodes[c].label.as_str())
                .collect::<Vec<_>>()
                .join("/")
                .into(),
            depth: r.depth as i32,
            expandable: r.expandable,
            expanded: r.expanded,
            file: n.file,
            in_count: v.incoming[i] as i32,
            out_count: v.outgoing[i] as i32,
            in_cycle: v.in_cycle[i],
            covered: covered.contains(&i),
            priority: priority_index(g.priorities[i]),
        }
    }))
}

/// The network display shows the rows that are not expanded: exactly the
/// rows that carry links.
fn push_network(ui: &AppWindow, g: &GraphState, selected: Option<usize>) {
    let v = &g.view;
    let leaves: Vec<usize> = (0..v.rows.len()).filter(|&r| !v.rows[r].expanded).collect();
    let mut slot = vec![usize::MAX; v.rows.len()];
    for (i, &r) in leaves.iter().enumerate() {
        slot[r] = i;
    }
    let net = network::place(leaves.len());
    ui.set_net_cards(vec_model(leaves.iter().zip(&net.cards).map(|(&r, c)| {
        let node = v.rows[r].node;
        let (files, packages) = g.tree.counts(node);
        NetCard {
            row: r as i32,
            label: g.tree.nodes[node].label.clone().into(),
            file: g.tree.nodes[node].file,
            files: files as i32,
            packages: packages as i32,
            priority: priority_index(g.priorities[r]),
            in_cycle: v.in_cycle[r],
            x: c.x,
            y: c.y,
        }
    })));
    ui.set_net_arrows(vec_model(v.shown(selected, g.mode).into_iter().filter_map(
        |(i, lane)| {
            let l = &v.links[i];
            let (a, b) = (slot[l.from], slot[l.to]);
            (a != usize::MAX && b != usize::MAX).then(|| {
                let arrow = network::arrow(&net.cards[a], &net.cards[b], l.weight);
                NetArrow {
                    path: arrow.path.into(),
                    width: arrow.width,
                    kind: v.link_kind(i, lane, selected, g.mode) as i32,
                }
            })
        },
    )));
    ui.set_net_w(net.width);
    ui.set_net_h(net.height);
}

fn priority_index(p: Option<ReviewPriority>) -> i32 {
    match p {
        Some(ReviewPriority::High) => 1,
        Some(ReviewPriority::Medium) => 2,
        _ => 0,
    }
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
    /// (`ARCHGUARD_DESKTOP_REVEAL=<name>`) or open a package as a
    /// breadcrumb does (`ARCHGUARD_DESKTOP_OPEN=<name>`), switch tab
    /// (`ARCHGUARD_DESKTOP_TAB`) or display (`ARCHGUARD_DESKTOP_DISPLAY`,
    /// 0 tree, 1 network, 2 dependencies), render, write the PNG, quit.
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
        if let Ok(name) = std::env::var("ARCHGUARD_DESKTOP_OPEN") {
            let node = super::APP.with_borrow(|app| {
                app.as_ref()
                    .and_then(|g| g.tree.nodes.iter().position(|n| n.name == name))
            });
            if let Some(n) = node {
                ui.invoke_open_node(n as i32);
            }
        }
        if let Some(h) = std::env::var("ARCHGUARD_DESKTOP_HOVER")
            .ok()
            .and_then(|h| h.parse().ok())
        {
            ui.set_tree_hovered(h);
        }
        if let Some(d) = std::env::var("ARCHGUARD_DESKTOP_DISPLAY")
            .ok()
            .and_then(|d| d.parse().ok())
        {
            ui.set_display(d);
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
