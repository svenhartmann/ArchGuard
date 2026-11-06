//! `ARCHGUARD_DESKTOP_SNAPSHOT=<file.png>`: render the window once every
//! document's analysis finished, write the PNG and quit.
//!
//! Before rendering, optionally: show the document at a tab index
//! (`ARCHGUARD_DESKTOP_DOC`), close tabs (`ARCHGUARD_DESKTOP_CLOSE=1,0`),
//! reveal a node by name
//! (`ARCHGUARD_DESKTOP_REVEAL=<name>`) or open a package as a breadcrumb
//! does (`ARCHGUARD_DESKTOP_OPEN=<name>`), hover a tree row
//! (`ARCHGUARD_DESKTOP_HOVER`), switch display (`ARCHGUARD_DESKTOP_DISPLAY`,
//! 0 tree, 1 network, 2 dependencies), open a matrix cell
//! (`ARCHGUARD_DESKTOP_MCELL=<name>:<col>`), switch tab
//! (`ARCHGUARD_DESKTOP_TAB`, in the focused pane) or open the Projects
//! menu (`ARCHGUARD_DESKTOP_MENU=projects`). Pane hooks: see
//! [`apply_pane_hooks`].
//!
//! App shell: mark tabs dirty (`ARCHGUARD_DESKTOP_DIRTY=0,1`, before the
//! closes, so closing one shows the save prompt), press the window's close
//! button (`ARCHGUARD_DESKTOP_QUIT=1`), click at logical points
//! (`ARCHGUARD_DESKTOP_CLICK=x:y,rx:y`, `r`: right button; e.g. the menu bar Slint draws with
//! `SLINT_NO_MUDA=1`) and move the mouse to the last point after them
//! (`ARCHGUARD_DESKTOP_HOVER_AT=x:y`, opens a sub-menu). Settings come from
//! `ARCHGUARD_DESKTOP_CONFIG_DIR` only; without it nothing is persisted.
//! Export without the save dialog: [`export_hook`].

use std::cell::Cell;

use slint::ComponentHandle;

use crate::graph_ui::node_named;
use crate::panes_ui;
use crate::ui::AppWindow;

thread_local! {
    static SCHEDULED: Cell<bool> = const { Cell::new(false) };
}

fn env<T: std::str::FromStr>(key: &str) -> Option<T> {
    std::env::var(key).ok().and_then(|v| v.parse().ok())
}

pub fn schedule(ui: &AppWindow) {
    let Some(path) = std::env::var_os("ARCHGUARD_DESKTOP_SNAPSHOT") else {
        return;
    };
    if SCHEDULED.replace(true) {
        return;
    }
    if let Some(doc) = env::<i32>("ARCHGUARD_DESKTOP_DOC") {
        ui.invoke_select_doc(doc);
    }
    mark_dirty(&std::env::var("ARCHGUARD_DESKTOP_DIRTY").unwrap_or_default());
    // Comma-separated tab indexes, closed in order.
    let close = std::env::var("ARCHGUARD_DESKTOP_CLOSE").unwrap_or_default();
    for i in close.split(',').filter_map(|i| i.trim().parse().ok()) {
        ui.invoke_close_doc(i);
    }
    apply_view_hooks(ui);
    apply_shell_hooks(ui);
    export_hook(ui);
    let weak = ui.as_weak();
    // `ARCHGUARD_DESKTOP_HOLD=<seconds>`: keep the prepared view open that
    // long without writing a PNG (for renderer measurements), then quit.
    let hold = env::<u64>("ARCHGUARD_DESKTOP_HOLD");
    let delay = hold.map_or(900, |s| s * 1000);
    slint::Timer::single_shot(std::time::Duration::from_millis(delay), move || {
        let Some(ui) = weak.upgrade() else { return };
        if hold.is_none() {
            write_png(&ui, &path);
        }
        let _ = slint::quit_event_loop();
    });
}

/// `ARCHGUARD_DESKTOP_EXPORT=<format>:<path>[,…]`: export the focused
/// pane's view as File → Export does, but to `path` without the save
/// dialog (`format`: markdown, json, arc42, svg, png). Runs once the views
/// are drawn, before the snapshot, so the PNG shows the notice; logs each
/// result.
fn export_hook(ui: &AppWindow) {
    let Ok(spec) = std::env::var("ARCHGUARD_DESKTOP_EXPORT") else {
        return;
    };
    let weak = ui.as_weak();
    slint::Timer::single_shot(std::time::Duration::from_millis(500), move || {
        let Some(ui) = weak.upgrade() else { return };
        for item in spec.split(',') {
            let Some((format, path)) = item.split_once(':') else {
                continue;
            };
            let written = crate::export_ui::run(&ui, format, Some(path.trim().into()));
            eprintln!(
                "export {format}: {written:?}; notice {}: {}",
                ui.get_notice_kind(),
                ui.get_notice()
            );
        }
    });
}

/// `ARCHGUARD_DESKTOP_MQUERY=<text>`: search the Matrix, then leave the
/// tab and come back, so the snapshot shows the rebuilt view's state.
fn matrix_query_round_trip(ui: &AppWindow) {
    let Ok(q) = std::env::var("ARCHGUARD_DESKTOP_MQUERY") else {
        return;
    };
    ui.invoke_navigate("dsm".into());
    if let Some(id) = shown("dsm") {
        ui.invoke_m_search(id, q.into());
    }
    ui.invoke_navigate("graph".into());
    let weak = ui.as_weak();
    slint::Timer::single_shot(std::time::Duration::from_millis(300), move || {
        if let Some(ui) = weak.upgrade() {
            ui.invoke_navigate("dsm".into());
        }
    });
}

/// The id of a tab of `kind`, as the view bar would show it.
fn shown(kind: &str) -> Option<i32> {
    panes_ui::shown_id(kind).map(|id| id as i32)
}

/// `kind` or `kind:<node name>`.
fn view_spec(v: &str) -> (&str, Option<&str>) {
    match v.trim().split_once(':') {
        Some((kind, node)) => (kind.trim(), Some(node.trim())),
        None => (v.trim(), None),
    }
}

/// Show `spec` in the focused pane: its view, opened on its node (a Graph
/// on a package; a File view in a new tab).
fn show_left(ui: &AppWindow, spec: &str) {
    let (kind, node) = view_spec(spec);
    match node {
        Some(n) if kind == "graph" => {
            ui.invoke_navigate("graph".into());
            if let (Some(id), Some(n)) = (shown("graph"), node_named(n)) {
                ui.invoke_open_node(id, n as i32);
            }
        }
        Some(n) => {
            panes_ui::open_node(ui, kind, n, false);
        }
        None => ui.invoke_navigate(kind.into()),
    }
}

/// Panes, before the reveal hooks so those link across them: open views
/// in new tabs of the focused pane
/// (`ARCHGUARD_DESKTOP_VIEW_TABS=calls,graph:crates/a,file:src/x.rs`),
/// split into two panes (`ARCHGUARD_DESKTOP_SPLIT=graph:<scope>,file:<path>`,
/// left and right view, each `kind` or `kind:<node name>`; two Graphs on
/// different packages: `graph:a,graph:b`), set the divider
/// (`ARCHGUARD_DESKTOP_RATIO=0.6`), close a view tab
/// (`ARCHGUARD_DESKTOP_CLOSE_VIEW=<pane>:<tab>`) or focus a pane
/// (`ARCHGUARD_DESKTOP_FOCUS=0`).
fn apply_pane_hooks(ui: &AppWindow) {
    let tabs = std::env::var("ARCHGUARD_DESKTOP_VIEW_TABS").unwrap_or_default();
    for v in tabs.split(',').filter(|v| !v.trim().is_empty()) {
        match view_spec(v) {
            (kind, Some(node)) => {
                panes_ui::open_node(ui, kind, node, false);
            }
            (kind, None) => ui.invoke_open_view_tab(kind.into()),
        }
    }
    if let Some((left, right)) = env::<String>("ARCHGUARD_DESKTOP_SPLIT")
        .as_deref()
        .and_then(|v| v.split_once(','))
    {
        show_left(ui, left);
        match view_spec(right) {
            (kind, Some(node)) => {
                panes_ui::open_node(ui, kind, node, true);
            }
            (kind, None) => ui.invoke_split_view(kind.into()),
        }
    }
    if let Some(r) = env::<f32>("ARCHGUARD_DESKTOP_RATIO") {
        ui.invoke_set_pane_ratio(r);
    }
    let close = env::<String>("ARCHGUARD_DESKTOP_CLOSE_VIEW").and_then(|v| {
        let (p, t) = v.split_once(':')?;
        Some((p.trim().parse().ok()?, t.trim().parse().ok()?))
    });
    if let Some((p, t)) = close {
        ui.invoke_close_view_tab(p, t);
    }
    if let Some(p) = env::<i32>("ARCHGUARD_DESKTOP_FOCUS") {
        ui.invoke_focus_pane(p);
    }
}

/// Graph hooks act on the Graph the view bar would show, Matrix hooks on
/// that Matrix.
fn apply_view_hooks(ui: &AppWindow) {
    matrix_query_round_trip(ui);
    apply_pane_hooks(ui);
    let graph = shown("graph").unwrap_or(-1);
    if let Some(n) = env::<String>("ARCHGUARD_DESKTOP_REVEAL").and_then(|n| node_named(&n)) {
        ui.invoke_reveal(graph, n as i32);
    }
    if let Some(n) = env::<String>("ARCHGUARD_DESKTOP_OPEN").and_then(|n| node_named(&n)) {
        ui.invoke_open_node(graph, n as i32);
    }
    if let Some(h) = env("ARCHGUARD_DESKTOP_HOVER") {
        ui.set_tree_hovered(h);
    }
    if let Some(d) = env("ARCHGUARD_DESKTOP_DISPLAY") {
        ui.invoke_set_display(graph, d);
    }
    let cell = env::<String>("ARCHGUARD_DESKTOP_MCELL").and_then(|v| {
        let (name, col) = v.rsplit_once(':')?;
        Some((node_named(name)?, col.parse::<i32>().ok()?))
    });
    if let (Some((n, c)), Some(id)) = (cell, shown("dsm")) {
        ui.invoke_m_cell(id, n as i32, c);
    }
    if let Some(tab) = env::<String>("ARCHGUARD_DESKTOP_TAB") {
        ui.invoke_navigate(tab.into());
    }
    if env::<String>("ARCHGUARD_DESKTOP_MENU").as_deref() == Some("projects") {
        ui.set_projects_open(true);
    }
}

fn mark_dirty(tabs: &str) {
    for i in tabs
        .split(',')
        .filter_map(|i| i.trim().parse::<usize>().ok())
    {
        let id = crate::documents::with_store(|s| s.docs().get(i).map(|d| d.id));
        if let Some(id) = id {
            crate::documents::set_save_hook(id, || Ok(()));
            crate::documents::mark_dirty(id, "Layer rules");
            crate::documents::mark_dirty(id, "Thresholds");
        }
    }
}

fn point(v: &str) -> Option<slint::LogicalPosition> {
    let (x, y) = v.split_once(':')?;
    Some(slint::LogicalPosition::new(
        x.trim().parse().ok()?,
        y.trim().parse().ok()?,
    ))
}

fn apply_shell_hooks(ui: &AppWindow) {
    use slint::platform::{PointerEventButton, WindowEvent};
    if env::<String>("ARCHGUARD_DESKTOP_QUIT").as_deref() == Some("1") {
        ui.window().dispatch_event(WindowEvent::CloseRequested);
    }
    let clicks = std::env::var("ARCHGUARD_DESKTOP_CLICK").unwrap_or_default();
    // `r` before a point clicks the right button (context menus).
    for v in clicks.split(',') {
        let (button, v) = match v.trim().strip_prefix('r') {
            Some(v) => (PointerEventButton::Right, v),
            None => (PointerEventButton::Left, v),
        };
        let Some(position) = point(v) else { continue };
        ui.window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        ui.window()
            .dispatch_event(WindowEvent::PointerPressed { position, button });
        ui.window()
            .dispatch_event(WindowEvent::PointerReleased { position, button });
    }
    if let Some(position) = env::<String>("ARCHGUARD_DESKTOP_HOVER_AT").and_then(|v| point(&v)) {
        ui.window()
            .dispatch_event(WindowEvent::PointerMoved { position });
    }
    drag(ui);
}

/// `ARCHGUARD_DESKTOP_DRAG=x:y,x:y`: press at the first point, move to the
/// second and release there (e.g. the pane divider).
fn drag(ui: &AppWindow) {
    use slint::platform::{PointerEventButton, WindowEvent};
    let Ok(v) = std::env::var("ARCHGUARD_DESKTOP_DRAG") else {
        return;
    };
    let Some((from, to)) = v
        .split_once(',')
        .and_then(|(a, b)| Some((point(a)?, point(b)?)))
    else {
        return;
    };
    let button = PointerEventButton::Left;
    let w = ui.window();
    w.dispatch_event(WindowEvent::PointerMoved { position: from });
    w.dispatch_event(WindowEvent::PointerPressed {
        position: from,
        button,
    });
    w.dispatch_event(WindowEvent::PointerMoved { position: to });
    w.dispatch_event(WindowEvent::PointerReleased {
        position: to,
        button,
    });
}

fn write_png(ui: &AppWindow, path: &std::ffi::OsStr) {
    let shot = ui
        .window()
        .take_snapshot()
        .expect("renderer supports snapshots");
    let file = std::fs::File::create(path).expect("snapshot file is writable");
    let mut enc = png::Encoder::new(file, shot.width(), shot.height());
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().expect("png header");
    writer.write_image_data(shot.as_bytes()).expect("png data");
}
