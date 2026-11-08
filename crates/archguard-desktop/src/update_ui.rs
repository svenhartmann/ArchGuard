//! Self-update of the desktop app: Help → Check for updates…, the check at
//! start-up (at most once a day, `[updates] check = false` turns it off,
//! [`schedule`]) and the notice below the header (ui/update_notice.slint)
//! whose states [`notice`] defines. Checks, downloads and installs run on
//! worker threads ([`worker`]) with `archguard-update`; the window never
//! waits for them.

mod notice;
mod schedule;
mod worker;

use std::cell::RefCell;

use archguard_update::desktop::How;
use archguard_update::release;
use slint::ComponentHandle;

use crate::settings_file;
use crate::ui::{AppWindow, UpdateNotice};
use notice::{Effect, Origin, Phase};
pub use worker::QuitGuard;

thread_local! {
    static PHASE: RefCell<Phase> = const { RefCell::new(Phase::Hidden) };
}

/// Register the notice's callbacks and start the start-up check if due.
/// Call after the settings are loaded; keep the guard until the event
/// loop ended, it starts a restarted app or an installer.
pub fn wire(ui: &AppWindow) -> QuitGuard {
    let n = ui.global::<UpdateNotice>();
    let w = ui.as_weak();
    n.on_check(move || with(&w, |ui| check(ui, Origin::Manual)));
    let w = ui.as_weak();
    n.on_primary(move || with(&w, |ui| step(ui, PHASE.take().primary())));
    n.on_notes(|| {
        if let Some(page) = PHASE.with_borrow(|p| p.update().map(|u| u.page.clone())) {
            open_page(page);
        }
    });
    let w = ui.as_weak();
    n.on_later(move || with(&w, |ui| set(ui, PHASE.take().later())));
    #[cfg(feature = "snapshot")]
    snapshot_hook(ui);
    if !crate::context::headless_run()
        && settings_file::get_doc(|d| schedule::due(d, schedule::now()))
    {
        settings_file::update_doc(|d| schedule::record(d, schedule::now()));
        check(ui, Origin::Startup);
    }
    QuitGuard
}

fn with(w: &slint::Weak<AppWindow>, f: impl FnOnce(&AppWindow)) {
    if let Some(ui) = w.upgrade() {
        f(&ui);
    }
}

/// Look for an update on a worker thread; a manual check shows "Checking".
fn check(ui: &AppWindow, origin: Origin) {
    if !PHASE.with_borrow(Phase::may_check) {
        return;
    }
    if origin == Origin::Manual {
        set(ui, PHASE.take().checking());
    }
    let w = ui.as_weak();
    std::thread::spawn(move || {
        let result = worker::check(&release::api_base());
        let _ = slint::invoke_from_event_loop(move || {
            with(&w, |ui| {
                let phase = PHASE.take().checked(origin, worker::CURRENT, result);
                set(ui, phase);
            });
        });
    });
}

/// Show a transition's phase and do what it asks for.
fn step(ui: &AppWindow, (phase, effect): (Phase, Effect)) {
    set(ui, phase);
    match effect {
        Effect::None => {}
        Effect::Download(update) => {
            let w = ui.as_weak();
            std::thread::spawn(move || {
                let result = worker::download_and_install(&update, &How::detect());
                let _ = slint::invoke_from_event_loop(move || {
                    with(&w, |ui| step(ui, PHASE.take().installed(result)));
                });
            });
        }
        Effect::OpenPage(page) => open_page(page),
        Effect::OnQuit(launch) => worker::on_quit(launch),
        Effect::Quit(launch) => {
            launch.into_iter().for_each(worker::on_quit);
            // The shell's quit: asks about unsaved changes first.
            ui.invoke_quit_app();
        }
    }
}

fn set(ui: &AppWindow, phase: Phase) {
    let n = ui.global::<UpdateNotice>();
    n.set_kind(phase.kind());
    let version = match &phase {
        Phase::UpToDate { current } => current.clone(),
        p => p.update().map(|u| u.version.clone()).unwrap_or_default(),
    };
    n.set_version(version.into());
    let error = match &phase {
        Phase::CheckFailed { error } | Phase::InstallFailed { error, .. } => error.as_str(),
        _ => "",
    };
    n.set_error(error.into());
    PHASE.set(phase);
}

/// Open a release page (https only) in the browser, off the UI thread.
fn open_page(page: String) {
    if !page.starts_with("https://") {
        return;
    }
    std::thread::spawn(move || {
        if let Err(e) = crate::web::open_in_browser(&page) {
            eprintln!("archguard-desktop: cannot open {page}: {e}");
        }
    });
}

/// `ARCHGUARD_DESKTOP_UPDATE=<kind>` (0 to 9, `Phase::kind`): show the
/// notice for a made-up version 9.9.9 without any network.
#[cfg(feature = "snapshot")]
fn snapshot_hook(ui: &AppWindow) {
    let Some(kind) = std::env::var("ARCHGUARD_DESKTOP_UPDATE")
        .ok()
        .and_then(|k| k.parse::<i32>().ok())
    else {
        return;
    };
    let update = archguard_update::desktop::Update {
        version: "9.9.9".into(),
        package: String::new(),
        url: String::new(),
        sha256: String::new(),
        page: String::new(),
    };
    let error = "package checksum mismatch".to_string();
    let phase = [
        Phase::Hidden,
        Phase::Checking,
        Phase::UpToDate {
            current: worker::CURRENT.into(),
        },
        Phase::CheckFailed {
            error: error.clone(),
        },
        Phase::Available(update.clone()),
        Phase::Downloading(update.clone()),
        Phase::Restart {
            update: update.clone(),
            path: "/Applications/ArchGuard.app".into(),
        },
        Phase::Installer(update.clone()),
        Phase::Manual(update.clone()),
        Phase::InstallFailed { update, error },
    ]
    .into_iter()
    .find(|p| p.kind() == kind);
    if let Some(phase) = phase {
        set(ui, phase);
    }
}
