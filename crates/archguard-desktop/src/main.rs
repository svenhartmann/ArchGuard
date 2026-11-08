//! `archguard-desktop`: native viewer with the web viewer's layout.
//!
//! Usage: `archguard-desktop [PROJECT_DIR …]`. Each directory opens as a
//! document tab; its analysis runs on a worker thread with the same cached
//! pipeline as the CLI. Without arguments the last session reopens (see
//! `settings.rs`), on the first start the current directory.

mod arc42_file;
mod architecture_ui;
#[cfg(feature = "snapshot")]
mod bench;
mod block_names;
mod calls_ui;
mod config_editor;
mod context;
mod deferred;
mod documents;
mod documents_ui;
mod export;
mod export_ui;
#[cfg(test)]
mod fields_check;
mod file_ui;
mod follow;
mod follow_ui;
mod format;
mod fullscreen;
mod graph_ui;
#[cfg(feature = "snapshot")]
mod headless;
mod instances;
mod kpi;
mod matrix;
mod matrix_ui;
mod model_sync;
mod network;
mod node_view;
mod pane_store;
mod panes;
mod panes_ui;
mod platform;
mod quality_ui;
mod rating;
#[cfg(test)]
mod security_check;
mod security_ui;
mod services_ui;
mod settings;
mod settings_file;
mod settings_view_ui;
mod shell;
mod shell_ui;
#[cfg(feature = "snapshot")]
mod snapshot;
mod start_ui;
mod state;
mod status;
mod tab_support;
mod timing;
mod tree;
mod tree_order;
mod ui;
mod view_export;
mod web;

use std::path::PathBuf;

use ui::*;

fn main() -> Result<(), slint::PlatformError> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    // Snapshot runs render off-screen: no window, no Dock icon.
    #[cfg(feature = "snapshot")]
    headless::install()?;
    if !context::headless_run() {
        platform::select_renderer()?;
    }
    let ui = AppWindow::new()?;
    if !context::headless_run() {
        set_dock_icon();
    }
    platform::apply(&ui);
    format::wire(&ui);
    fullscreen::wire(&ui);
    documents_ui::wire(&ui);
    shell_ui::wire(&ui);
    shell_ui::start(&ui, args);

    let result = ui.run();
    web::shutdown();
    result
}

/// macOS ignores window icons; without an app bundle the Dock shows the
/// application icon set here. It is set again once the event loop runs,
/// after AppKit finished launching (measured on macOS 27: the icon set
/// before `run()` was still in place then, so this is a safeguard).
/// Snapshot builds log the icon at each step with
/// `ARCHGUARD_DESKTOP_LOG_ICON=1` (and in snapshot mode).
#[cfg(target_os = "macos")]
fn set_dock_icon() {
    set_icon_now("before run");
    slint::invoke_from_event_loop(|| set_icon_now("event loop")).ok();
    #[cfg(feature = "snapshot")]
    if log_icon() {
        slint::Timer::single_shot(std::time::Duration::from_secs(2), || {
            eprintln!("dock icon after 2 s: {}", icon_report());
        });
    }
}

/// Whether to log the Dock icon (snapshot builds only).
#[cfg(all(target_os = "macos", feature = "snapshot"))]
fn log_icon() -> bool {
    ["ARCHGUARD_DESKTOP_LOG_ICON", "ARCHGUARD_DESKTOP_SNAPSHOT"]
        .iter()
        .any(|k| std::env::var_os(k).is_some())
}

/// Log the Dock icon at `step` (snapshot builds only).
#[cfg(target_os = "macos")]
fn log_step(step: &str) {
    #[cfg(feature = "snapshot")]
    if log_icon() {
        eprintln!("dock icon {step}: {}", icon_report());
    }
    #[cfg(not(feature = "snapshot"))]
    let _ = step;
}

#[cfg(target_os = "macos")]
fn set_icon_now(step: &str) {
    use objc2::{AllocAnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    log_step(&format!("{step}, before setting"));
    let data = NSData::with_bytes(include_bytes!("../assets/icon.png"));
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    // SAFETY: called on the main thread (checked by `mtm`) with a valid image.
    unsafe { app.setApplicationIconImage(Some(&image)) };
    log_step(&format!("{step}, after setting"));
}

/// The current application icon's largest representation in pixels. The
/// default icon of an unbundled binary reads 256 px; after setting
/// `assets/icon.png` (1024 px) AppKit reports a 2048 px rendering.
#[cfg(all(target_os = "macos", feature = "snapshot"))]
fn icon_report() -> String {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSApplication;
    let Some(mtm) = MainThreadMarker::new() else {
        return "not on the main thread".into();
    };
    let Some(icon) = NSApplication::sharedApplication(mtm).applicationIconImage() else {
        return "none".into();
    };
    let reps = icon.representations();
    let largest = reps
        .iter()
        .map(|r| (r.pixelsWide(), r.pixelsHigh()))
        .max()
        .unwrap_or_default();
    format!(
        "{}x{} px, {} representation(s)",
        largest.0,
        largest.1,
        reps.count()
    )
}

#[cfg(not(target_os = "macos"))]
fn set_dock_icon() {}
