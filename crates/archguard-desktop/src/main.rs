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
mod tree;
mod tree_order;
mod ui;
mod web;

use std::path::PathBuf;

use ui::*;

fn main() -> Result<(), slint::PlatformError> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let ui = AppWindow::new()?;
    set_dock_icon();
    documents_ui::wire(&ui);
    shell_ui::wire(&ui);
    shell_ui::start(&ui, args);

    let result = ui.run();
    web::shutdown();
    result
}

/// macOS ignores window icons; without an app bundle the Dock shows the
/// application icon set here.
#[cfg(target_os = "macos")]
fn set_dock_icon() {
    use objc2::{AllocAnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let data = NSData::with_bytes(include_bytes!("../assets/icon.png"));
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    // SAFETY: called on the main thread (checked by `mtm`) with a valid image.
    unsafe { app.setApplicationIconImage(Some(&image)) };
}

#[cfg(not(target_os = "macos"))]
fn set_dock_icon() {}
