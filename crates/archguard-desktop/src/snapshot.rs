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
    // Comma-separated tab indexes, closed in order.
    let close = std::env::var("ARCHGUARD_DESKTOP_CLOSE").unwrap_or_default();
    for i in close.split(',').filter_map(|i| i.trim().parse().ok()) {
        ui.invoke_close_doc(i);
    }
    apply_view_hooks(ui);
    let weak = ui.as_weak();
    slint::Timer::single_shot(std::time::Duration::from_millis(900), move || {
        let Some(ui) = weak.upgrade() else { return };
        write_png(&ui, &path);
        let _ = slint::quit_event_loop();
    });
}

fn apply_view_hooks(ui: &AppWindow) {
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
