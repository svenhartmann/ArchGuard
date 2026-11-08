//! The update notice as a state machine, independent of Slint: what the
//! banner shows ([`Phase`]) and what the app does next ([`Effect`]).
//!
//! A check finds an update (`Available`) → Install downloads it
//! (`Downloading`) → the installation was replaced (`Restart`), an
//! installer runs when the app quits (`Installer`), the release page is
//! the way (`Manual`) or it failed (`InstallFailed`). Checks started at
//! start-up stay silent unless they find an update.

use std::ffi::OsString;
use std::path::PathBuf;

use archguard_update::desktop::{Installed, Update};

/// Who started a check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    Startup,
    /// Help → Check for updates…
    Manual,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum Phase {
    #[default]
    Hidden,
    /// A manual check is running.
    Checking,
    /// A manual check found nothing newer than the running version.
    UpToDate {
        current: String,
    },
    CheckFailed {
        error: String,
    },
    Available(Update),
    Downloading(Update),
    /// The installed app was replaced; a restart runs the new version.
    Restart {
        update: Update,
        path: PathBuf,
    },
    /// The installer runs when the app quits.
    Installer(Update),
    /// This installation cannot update itself; the release page is open.
    Manual(Update),
    InstallFailed {
        update: Update,
        error: String,
    },
}

/// A program to start when the app quits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    pub program: OsString,
    pub args: Vec<OsString>,
}

/// What the app does after a transition.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    None,
    /// Download and install on a worker thread.
    Download(Update),
    OpenPage(String),
    /// Start this when the app quits.
    OnQuit(Launch),
    /// Quit now (asking about unsaved changes first), then start this.
    Quit(Option<Launch>),
}

impl Phase {
    /// The banner's kind for `UpdateNotice.kind` (ui/update_notice.slint).
    pub fn kind(&self) -> i32 {
        match self {
            Phase::Hidden => 0,
            Phase::Checking => 1,
            Phase::UpToDate { .. } => 2,
            Phase::CheckFailed { .. } => 3,
            Phase::Available(_) => 4,
            Phase::Downloading(_) => 5,
            Phase::Restart { .. } => 6,
            Phase::Installer(_) => 7,
            Phase::Manual(_) => 8,
            Phase::InstallFailed { .. } => 9,
        }
    }

    /// The update the banner is about, if any.
    pub fn update(&self) -> Option<&Update> {
        match self {
            Phase::Available(u)
            | Phase::Downloading(u)
            | Phase::Installer(u)
            | Phase::Manual(u)
            | Phase::Restart { update: u, .. }
            | Phase::InstallFailed { update: u, .. } => Some(u),
            _ => None,
        }
    }

    /// Whether a new check may replace this phase: not while a check or a
    /// download runs, nor after an install.
    pub fn may_check(&self) -> bool {
        !matches!(
            self,
            Phase::Checking | Phase::Downloading(_) | Phase::Restart { .. } | Phase::Installer(_)
        )
    }

    /// A manual check starts.
    pub fn checking(self) -> Phase {
        if self.may_check() {
            Phase::Checking
        } else {
            self
        }
    }

    /// A check by `origin` ended with `result`; `current` is the running version.
    pub fn checked(
        self,
        origin: Origin,
        current: &str,
        result: Result<Option<Update>, String>,
    ) -> Phase {
        if !self.may_check() && self != Phase::Checking {
            return self;
        }
        match (result, origin) {
            (Ok(Some(update)), _) => Phase::Available(update),
            (Ok(None), Origin::Manual) => Phase::UpToDate {
                current: current.to_string(),
            },
            (Err(error), Origin::Manual) => Phase::CheckFailed { error },
            // Silent at start-up: keep what is shown.
            (_, Origin::Startup) => self,
        }
    }

    /// Install (Available, InstallFailed: try again), Restart now or Quit
    /// and install.
    pub fn primary(self) -> (Phase, Effect) {
        match self {
            Phase::Available(u) | Phase::InstallFailed { update: u, .. } => {
                (Phase::Downloading(u.clone()), Effect::Download(u))
            }
            Phase::Restart { ref path, .. } => {
                let launch = restart(path, cfg!(target_os = "macos"));
                (self, Effect::Quit(Some(launch)))
            }
            Phase::Installer(_) => (self, Effect::Quit(None)),
            other => (other, Effect::None),
        }
    }

    /// The download and install of `Downloading` ended.
    pub fn installed(self, result: Result<Installed, String>) -> (Phase, Effect) {
        let Phase::Downloading(update) = self else {
            return (self, Effect::None);
        };
        match result {
            Ok(Installed::Replaced { path }) => (Phase::Restart { update, path }, Effect::None),
            Ok(Installed::RunBeforeQuit { program, args }) => (
                Phase::Installer(update),
                Effect::OnQuit(Launch { program, args }),
            ),
            Ok(Installed::Manual { page }) => (Phase::Manual(update), Effect::OpenPage(page)),
            Err(error) => (Phase::InstallFailed { update, error }, Effect::None),
        }
    }

    /// Later or close: hide the banner, except while downloading. A
    /// registered installer still runs when the app quits.
    pub fn later(self) -> Phase {
        match self {
            Phase::Downloading(_) => self,
            _ => Phase::Hidden,
        }
    }
}

/// Start the replaced app at `path`: a bundle with `open -n` on macOS (a
/// new instance even while this one still quits), else the executable.
pub fn restart(path: &std::path::Path, macos: bool) -> Launch {
    if macos {
        Launch {
            program: "open".into(),
            args: vec!["-n".into(), path.as_os_str().to_owned()],
        }
    } else {
        Launch {
            program: path.as_os_str().to_owned(),
            args: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn update() -> Update {
        Update {
            version: "9.9.9".into(),
            package: "ArchGuard-9.9.9-x86_64-unknown-linux-gnu.AppImage".into(),
            url: "http://127.0.0.1/p".into(),
            sha256: "00".into(),
            page: "https://example.invalid/v9.9.9".into(),
        }
    }

    fn downloading() -> Phase {
        let (phase, effect) = Phase::Available(update()).primary();
        assert_eq!(effect, Effect::Download(update()));
        assert_eq!(phase, Phase::Downloading(update()));
        phase
    }

    #[test]
    fn a_found_update_shows_for_either_origin() {
        for origin in [Origin::Startup, Origin::Manual] {
            let p = Phase::Hidden.checked(origin, "0.2.0", Ok(Some(update())));
            assert_eq!(p, Phase::Available(update()));
            assert_eq!(p.kind(), 4);
        }
    }

    #[test]
    fn start_up_is_silent_without_update_and_on_errors() {
        let p = Phase::Hidden.checked(Origin::Startup, "0.2.0", Ok(None));
        assert_eq!(p, Phase::Hidden);
        let p = Phase::Hidden.checked(Origin::Startup, "0.2.0", Err("offline".into()));
        assert_eq!(p, Phase::Hidden);
    }

    #[test]
    fn a_manual_check_reports_up_to_date_and_errors() {
        let p = Phase::Hidden.checking();
        assert_eq!(p, Phase::Checking);
        let up = p.clone().checked(Origin::Manual, "0.2.0", Ok(None));
        assert_eq!(
            up,
            Phase::UpToDate {
                current: "0.2.0".into()
            }
        );
        let failed = p.checked(Origin::Manual, "0.2.0", Err("offline".into()));
        assert_eq!(
            failed,
            Phase::CheckFailed {
                error: "offline".into()
            }
        );
        assert_eq!(failed.kind(), 3);
    }

    #[test]
    fn replaced_offers_a_restart_that_quits() {
        let path = PathBuf::from("/Applications/ArchGuard.app");
        let (p, effect) = downloading().installed(Ok(Installed::Replaced { path: path.clone() }));
        assert_eq!(
            p,
            Phase::Restart {
                update: update(),
                path: path.clone()
            }
        );
        assert_eq!(effect, Effect::None);
        let (p, effect) = p.primary();
        assert_eq!(p.kind(), 6);
        let launch = restart(&path, cfg!(target_os = "macos"));
        assert_eq!(effect, Effect::Quit(Some(launch)));
    }

    #[test]
    fn run_before_quit_registers_the_installer_for_the_quit() {
        let (p, effect) = downloading().installed(Ok(Installed::RunBeforeQuit {
            program: "msiexec".into(),
            args: vec!["/i".into(), "x.msi".into()],
        }));
        assert_eq!(p, Phase::Installer(update()));
        let launch = Launch {
            program: "msiexec".into(),
            args: vec!["/i".into(), "x.msi".into()],
        };
        assert_eq!(effect, Effect::OnQuit(launch));
        assert_eq!(p.clone().later(), Phase::Hidden);
        assert_eq!(p.primary().1, Effect::Quit(None));
    }

    #[test]
    fn manual_opens_the_release_page_and_errors_allow_a_retry() {
        let page = update().page;
        let (p, effect) = downloading().installed(Ok(Installed::Manual { page: page.clone() }));
        assert_eq!((p.kind(), effect), (8, Effect::OpenPage(page)));
        let (p, effect) = downloading().installed(Err("checksum mismatch".into()));
        assert_eq!(
            p,
            Phase::InstallFailed {
                update: update(),
                error: "checksum mismatch".into()
            }
        );
        assert_eq!(effect, Effect::None);
        assert_eq!(p.primary().1, Effect::Download(update()));
    }

    #[test]
    fn checks_do_not_replace_a_download_or_an_install() {
        for p in [downloading(), Phase::Installer(update())] {
            assert_eq!(p.clone().checking(), p);
            assert_eq!(p.clone().checked(Origin::Manual, "0.2.0", Ok(None)), p);
        }
        // An install result without a download is ignored.
        let (p, _) = Phase::Hidden.installed(Err("late".into()));
        assert_eq!(p, Phase::Hidden);
        assert_eq!(Phase::Available(update()).later(), Phase::Hidden);
        assert_eq!(downloading().later(), downloading());
    }

    #[test]
    fn restart_opens_a_new_bundle_instance_on_macos_and_runs_the_file_elsewhere() {
        let mac = restart(std::path::Path::new("/A/ArchGuard.app"), true);
        assert_eq!(mac.program, "open");
        assert_eq!(mac.args, ["-n", "/A/ArchGuard.app"]);
        let linux = restart(std::path::Path::new("/h/ArchGuard.AppImage"), false);
        assert_eq!(linux.program, "/h/ArchGuard.AppImage");
        assert!(linux.args.is_empty());
    }
}
