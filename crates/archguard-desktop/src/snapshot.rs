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
//! (`ARCHGUARD_DESKTOP_TAB`) or open the Projects menu
//! (`ARCHGUARD_DESKTOP_MENU=projects`).
//!
//! App shell: mark tabs dirty (`ARCHGUARD_DESKTOP_DIRTY=0,1`, before the
//! closes, so closing one shows the save prompt), press the window's close
//! button (`ARCHGUARD_DESKTOP_QUIT=1`), click at logical points
//! (`ARCHGUARD_DESKTOP_CLICK=x:y,x:y`, e.g. the menu bar Slint draws with
//! `SLINT_NO_MUDA=1`) and move the mouse to the last point after them
//! (`ARCHGUARD_DESKTOP_HOVER_AT=x:y`, opens a sub-menu). Settings come from
//! `ARCHGUARD_DESKTOP_CONFIG_DIR` only; without it nothing is persisted.

use std::cell::Cell;

use slint::ComponentHandle;

use crate::graph_ui::node_named;
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

/// `ARCHGUARD_DESKTOP_MQUERY=<text>`: search the Matrix, then leave the
/// tab and come back, so the snapshot shows the rebuilt view's state.
fn matrix_query_round_trip(ui: &AppWindow) {
    let Ok(q) = std::env::var("ARCHGUARD_DESKTOP_MQUERY") else {
        return;
    };
    ui.set_tab("dsm".into());
    ui.invoke_m_search(q.into());
    ui.set_tab("graph".into());
    let weak = ui.as_weak();
    slint::Timer::single_shot(std::time::Duration::from_millis(300), move || {
        if let Some(ui) = weak.upgrade() {
            ui.set_tab("dsm".into());
        }
    });
}

fn apply_view_hooks(ui: &AppWindow) {
    matrix_query_round_trip(ui);
    if let Some(n) = env::<String>("ARCHGUARD_DESKTOP_REVEAL").and_then(|n| node_named(&n)) {
        ui.invoke_reveal(n as i32);
    }
    if let Some(n) = env::<String>("ARCHGUARD_DESKTOP_OPEN").and_then(|n| node_named(&n)) {
        ui.invoke_open_node(n as i32);
    }
    if let Some(h) = env("ARCHGUARD_DESKTOP_HOVER") {
        ui.set_tree_hovered(h);
    }
    if let Some(d) = env("ARCHGUARD_DESKTOP_DISPLAY") {
        ui.set_display(d);
    }
    let cell = env::<String>("ARCHGUARD_DESKTOP_MCELL").and_then(|v| {
        let (name, col) = v.rsplit_once(':')?;
        Some((node_named(name)?, col.parse::<i32>().ok()?))
    });
    if let Some((n, c)) = cell {
        ui.invoke_m_cell(n as i32, c);
    }
    if let Some(tab) = env::<String>("ARCHGUARD_DESKTOP_TAB") {
        ui.set_tab(tab.into());
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
    for position in clicks.split(',').filter_map(point) {
        let button = PointerEventButton::Left;
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
