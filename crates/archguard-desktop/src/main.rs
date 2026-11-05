//! `archguard-desktop`: native viewer with the web viewer's layout.
//!
//! Usage: `archguard-desktop [PROJECT_DIR …]` (default: the current
//! directory). Each directory opens as a document tab; its analysis runs
//! on a worker thread with the same cached pipeline as the CLI.

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
    let mut roots: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if roots.is_empty() {
        roots.push(std::env::current_dir().expect("current directory is readable"));
    }

    let ui = AppWindow::new()?;
    set_dock_icon();
    // Slint selects bundled translations only after the first component.
    select_language();
    if let Ok(theme) = std::env::var("ARCHGUARD_DESKTOP_THEME") {
        ui.invoke_set_dark(theme != "light");
    }
    documents_ui::wire(&ui);
    for root in roots {
        documents_ui::open(&ui, root);
    }
    documents_ui::activate(&ui, 0);

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
