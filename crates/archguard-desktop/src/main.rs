//! `archguard-desktop`: native viewer with the web viewer's layout.
//!
//! Usage: `archguard-desktop [PROJECT_DIR …]`. Each directory opens as a
//! document tab; its analysis runs on a worker thread with the same cached
//! pipeline as the CLI. Without arguments the last session reopens (see
//! `settings.rs`), on the first start the current directory.

mod architecture_ui;
mod calls_ui;
mod context;
mod documents;
mod documents_ui;
mod graph_ui;
#[cfg(feature = "snapshot")]
mod headless;
mod matrix;
mod matrix_ui;
mod network;
mod quality_ui;
mod security_ui;
mod services_ui;
mod settings;
mod settings_file;
mod shell;
mod shell_ui;
#[cfg(feature = "snapshot")]
mod snapshot;
mod start_ui;
mod state;
mod tab_support;
mod tree;
mod tree_order;
mod ui;
mod web;

use std::path::PathBuf;

use ui::*;

fn main() -> Result<(), slint::PlatformError> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    // Snapshot runs render off-screen: no window, no Dock icon.
    #[cfg(feature = "snapshot")]
    headless::install()?;
    let ui = AppWindow::new()?;
    if !context::headless_run() {
        set_dock_icon();
    }
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
/// `ARCHGUARD_DESKTOP_LOG_ICON=1` (and snapshot mode) logs the icon at
/// each step.
#[cfg(target_os = "macos")]
fn set_dock_icon() {
    set_icon_now("before run");
    slint::invoke_from_event_loop(|| set_icon_now("event loop")).ok();
    if log_icon() {
        slint::Timer::single_shot(std::time::Duration::from_secs(2), || {
            eprintln!("dock icon after 2 s: {}", icon_report());
        });
    }
}

#[cfg(target_os = "macos")]
fn log_icon() -> bool {
    ["ARCHGUARD_DESKTOP_LOG_ICON", "ARCHGUARD_DESKTOP_SNAPSHOT"]
        .iter()
        .any(|k| std::env::var_os(k).is_some())
}

#[cfg(target_os = "macos")]
fn set_icon_now(step: &str) {
    use objc2::{AllocAnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    if log_icon() {
        eprintln!("dock icon {step}, before setting: {}", icon_report());
    }
    let data = NSData::with_bytes(include_bytes!("../assets/icon.png"));
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    // SAFETY: called on the main thread (checked by `mtm`) with a valid image.
    unsafe { app.setApplicationIconImage(Some(&image)) };
    if log_icon() {
        eprintln!("dock icon {step}, after setting: {}", icon_report());
    }
}

/// The current application icon's largest representation in pixels. The
/// default icon of an unbundled binary reads 256 px; after setting
/// `assets/icon.png` (1024 px) AppKit reports a 2048 px rendering.
#[cfg(target_os = "macos")]
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
