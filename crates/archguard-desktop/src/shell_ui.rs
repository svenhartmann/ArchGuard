//! The app shell in the window: start-up from the settings and the last
//! session, the menu commands (open, open recent, close, close all, quit,
//! theme, language) and the "Save changes?" prompt of [`shell`] before a
//! document closes or the app quits, also for the window's close button.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use slint::{CloseRequestResponse, ComponentHandle, SharedString};

use crate::documents::{self, with_store};
use crate::documents_ui::{self, index, with_ui};
use crate::settings::{self, Theme};
use crate::settings_file;
use crate::shell::{Answer, Flow, Next, Request};
use crate::ui::AppWindow;

thread_local! {
    /// The request waiting for answers while the prompt is shown.
    static FLOW: RefCell<Option<Flow>> = const { RefCell::new(None) };
}

/// Load the settings, apply them and open the start documents: `args`, or
/// without them the last session.
pub fn start(ui: &AppWindow, args: Vec<PathBuf>) {
    let notice = settings_file::init(settings_file::default_path());
    apply_settings(ui);
    let (roots, active) = settings_file::get(|s| {
        settings::startup_roots(args, s, std::env::current_dir().ok(), Path::exists)
    });
    for root in roots {
        documents_ui::open(ui, root);
    }
    documents_ui::activate(ui, active);
    documents_ui::push_recent(ui);
    if let Some(notice) = notice {
        show_notice(ui, 1, &notice);
    }
}

/// Theme, language and window size. The environment
/// (`ARCHGUARD_DESKTOP_THEME`, `ARCHGUARD_DESKTOP_LANG`) wins.
fn apply_settings(ui: &AppWindow) {
    let (theme, language, window) = settings_file::get(|s| (s.theme, s.language.clone(), s.window));
    match std::env::var("ARCHGUARD_DESKTOP_THEME") {
        Ok(theme) => ui.invoke_set_dark(theme != "light"),
        Err(_) if theme != Theme::System => ui.invoke_set_dark(theme == Theme::Dark),
        Err(_) => {}
    }
    select_language(ui, &language);
    if let Some((w, h)) = window {
        ui.window()
            .set_size(slint::LogicalSize::new(w as f32, h as f32));
    }
}

/// Register the menu commands, the prompt and the window's close button.
pub fn wire(ui: &AppWindow) {
    ui.on_close_doc(with_ui(ui, |ui, i: i32| {
        let id = with_store(|s| s.docs().get(index(i)).map(|d| d.id));
        if let Some(id) = id {
            request(ui, Request::Close(id));
        }
    }));
    let close_active = with_ui(ui, |ui, ()| {
        if let Some(id) = documents::active_id() {
            request(ui, Request::Close(id));
        }
    });
    ui.on_close_active(move || close_active(()));
    let close_all = with_ui(ui, |ui, ()| request(ui, Request::CloseAll));
    ui.on_close_all(move || close_all(()));
    let quit = with_ui(ui, |ui, ()| request(ui, Request::Quit));
    ui.on_quit_app(move || quit(()));
    ui.on_prompt_answered(with_ui(ui, |ui, a: i32| answer(ui, a)));
    ui.on_open_recent(with_ui(ui, |ui, i: i32| open_recent(ui, index(i))));
    let clear = with_ui(ui, |ui, ()| {
        settings_file::update(|s| s.recent.clear());
        documents_ui::push_recent(ui);
    });
    ui.on_clear_recent(move || clear(()));
    ui.on_choose_theme(|dark| {
        settings_file::update(|s| s.theme = if dark { Theme::Dark } else { Theme::Light });
    });
    ui.on_choose_language(with_ui(ui, |ui, lang: SharedString| {
        settings_file::update(|s| s.language = lang.to_string());
        select_language(ui, &lang);
        // Texts built in Rust follow on the next push.
        documents_ui::refresh(ui);
    }));
    let w = ui.as_weak();
    ui.window().on_close_requested(move || {
        if let Some(ui) = w.upgrade() {
            request(&ui, Request::Quit);
        }
        // Quitting ends the event loop; a prompt keeps the window.
        CloseRequestResponse::KeepWindowShown
    });
    route_app_quit();
    crate::settings_view_ui::wire(ui);
}

fn open_recent(ui: &AppWindow, i: usize) {
    let Some(root) = settings_file::get(|s| s.recent.get(i).cloned()) else {
        return;
    };
    if root.is_dir() {
        documents_ui::open(ui, root);
        return;
    }
    // Moved or deleted: drop it from the list.
    settings_file::update(|s| s.recent.retain(|p| *p != root));
    documents_ui::push_recent(ui);
}

/// Start `request`, asking about unsaved changes first. Ignored while a
/// prompt is open.
fn request(ui: &AppWindow, request: Request) {
    if FLOW.with_borrow(Option::is_some) {
        return;
    }
    let docs: Vec<_> = with_store(|s| s.docs().iter().map(|d| (d.id, d.is_dirty())).collect());
    let flow = Flow::new(request, &docs);
    let next = flow.next();
    FLOW.set(Some(flow));
    step(ui, next);
}

fn answer(ui: &AppWindow, a: i32) {
    let answer = match a {
        0 => Answer::Save,
        1 => Answer::DontSave,
        _ => Answer::Cancel,
    };
    // Not borrowed while the save hook runs.
    let Some(mut flow) = FLOW.take() else {
        return;
    };
    let next = flow.answer(answer, documents::save);
    FLOW.set(Some(flow));
    step(ui, next);
}

fn step(ui: &AppWindow, next: Next) {
    let ask = match next {
        Next::Ask(id) => id,
        Next::Run(request) => {
            finish(ui);
            return run(ui, request);
        }
        Next::Stop(error) => {
            finish(ui);
            if let Some(e) = error {
                show_notice(ui, 2, &e);
            }
            return;
        }
    };
    // Show the document asked about, as other apps do.
    if let Some(i) = with_store(|s| s.index_of(ask)) {
        documents_ui::activate(ui, i);
    }
    let (name, what) = with_store(|s| {
        s.get(ask)
            .map(|d| (d.name.clone(), d.unsaved()))
            .unwrap_or_default()
    });
    ui.set_prompt_project(name.into());
    ui.set_prompt_what(what.into());
    ui.set_prompt_open(true);
}

fn finish(ui: &AppWindow) {
    FLOW.set(None);
    ui.set_prompt_open(false);
}

fn run(ui: &AppWindow, request: Request) {
    match request {
        Request::Close(id) => {
            if let Some(i) = with_store(|s| s.index_of(id)) {
                documents_ui::close(ui, i);
            }
        }
        Request::CloseAll => {
            while with_store(|s| !s.docs().is_empty()) {
                documents_ui::close(ui, 0);
            }
        }
        Request::Quit => {
            remember_window(ui);
            let _ = slint::quit_event_loop();
        }
    }
}

/// Store the active view and the window size before quitting.
fn remember_window(ui: &AppWindow) {
    documents_ui::save_view(ui);
    let window = ui.window();
    let size = window.size().to_logical(window.scale_factor());
    let size = (size.width.round() as u32, size.height.round() as u32);
    settings_file::update(|s| s.window = Some(size));
}

fn show_notice(ui: &AppWindow, kind: i32, text: &str) {
    ui.set_notice_kind(kind);
    ui.set_notice(text.into());
}

/// German or English: the environment override, the setting, then the
/// system language. Call after creating the window.
fn select_language(ui: &AppWindow, setting: &str) {
    let lang = pick_language(
        std::env::var("ARCHGUARD_DESKTOP_LANG").ok(),
        setting,
        system_language,
    );
    let _ = slint::select_bundled_translation(lang);
    crate::format::set_locale(ui, crate::format::Locale::from_code(lang));
}

fn pick_language(
    env: Option<String>,
    setting: &str,
    system: impl FnOnce() -> String,
) -> &'static str {
    let lang = env
        .filter(|v| !v.is_empty())
        .or_else(|| Some(setting.to_string()).filter(|v| !v.is_empty()))
        .unwrap_or_else(system);
    if lang.to_lowercase().starts_with("de") {
        "de"
    } else {
        "en"
    }
}

fn system_language() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
        .or_else(macos_language)
        .unwrap_or_default()
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

/// Slint 1.18 always adds a macOS app menu whose "Quit" (⌘Q) terminates
/// the process without a close request, so unsaved changes would be lost
/// silently. Point it at the window's close button instead, which asks.
/// Slint rebuilds that menu when the menu bar changes, so check again
/// periodically.
#[cfg(target_os = "macos")]
fn route_app_quit() {
    thread_local! {
        static TIMER: slint::Timer = slint::Timer::default();
    }
    TIMER.with(|t| {
        t.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(300),
            patch_app_quit,
        );
    });
}

#[cfg(not(target_os = "macos"))]
fn route_app_quit() {}

#[cfg(target_os = "macos")]
fn patch_app_quit() {
    use objc2::{MainThreadMarker, sel};
    use objc2_app_kit::NSApplication;
    if crate::context::headless_run() {
        return;
    }
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    let Some(app_menu) = app
        .mainMenu()
        .and_then(|m| m.itemAtIndex(0))
        .and_then(|i| i.submenu())
    else {
        return;
    };
    for i in 0..app_menu.numberOfItems() {
        let Some(item) = app_menu.itemAtIndex(i) else {
            continue;
        };
        if item.action() == Some(sel!(terminate:)) {
            // SAFETY: on the main thread; `performClose:` is a standard
            // NSWindow action, sent to the key window (target nil).
            unsafe {
                item.setTarget(None);
                item.setAction(Some(sel!(performClose:)));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_environment_wins_then_the_setting_then_the_system() {
        let system = || "de_DE.UTF-8".to_string();
        assert_eq!(pick_language(Some("en".into()), "de", system), "en");
        assert_eq!(pick_language(None, "en", system), "en");
        assert_eq!(pick_language(Some(String::new()), "", system), "de");
        assert_eq!(pick_language(None, "", || "fr_FR".into()), "en");
    }
}
