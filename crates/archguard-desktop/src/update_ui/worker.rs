//! The blocking parts of an update, run on worker threads: the check, the
//! download and install, and starting a program when the app quits.

use std::cell::RefCell;

use archguard_update::desktop::{self, How, Installed, Update};
use archguard_update::{http, release};

use super::notice::Launch;

/// The running app's version.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// The latest release's desktop package if newer than [`CURRENT`];
/// `api` as [`release::api_base`].
pub fn check(api: &str) -> Result<Option<Update>, String> {
    desktop::available(api, CURRENT, release::TARGET)
}

/// Download `update` and install it as `how` says. On any error the
/// installed app is untouched (`archguard_update::desktop::install`).
pub fn download_and_install(update: &Update, how: &How) -> Result<Installed, String> {
    let bytes = http::download(&update.url, http::MAX_DOWNLOAD)?;
    desktop::install(&bytes, update, how)
}

thread_local! {
    /// What [`QuitGuard`] starts when the app quits.
    static ON_QUIT: RefCell<Option<Launch>> = const { RefCell::new(None) };
}

/// Start `launch` when the app quits (replaces an earlier one).
pub fn on_quit(launch: Launch) {
    ON_QUIT.set(Some(launch));
}

/// Held by `main` for the event loop's lifetime: when it ends, the
/// program registered with [`on_quit`] starts (the restarted app, the
/// installer). Nothing starts without one.
pub struct QuitGuard;

impl Drop for QuitGuard {
    fn drop(&mut self) {
        let Some(launch) = ON_QUIT.take() else {
            return;
        };
        if let Err(e) = std::process::Command::new(&launch.program)
            .args(&launch.args)
            .spawn()
        {
            eprintln!(
                "archguard-desktop: cannot start {}: {e}",
                launch.program.to_string_lossy()
            );
        }
    }
}

/// The update crate's local stand-in for the GitHub API (no network).
#[cfg(test)]
#[path = "../../../archguard-update/tests/stub/mod.rs"]
mod stub;

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use archguard_update::checksum::sha256_hex;

    use super::stub;
    use super::*;

    const PACKAGE: &[u8] = b"new appimage";

    /// A stub API whose latest release is `tag` with this target's package.
    fn release(tag: &str, digest: &str) -> String {
        let name = desktop::package_name(tag.trim_start_matches('v'), release::TARGET)
            .expect("a desktop target");
        let digest = digest.to_string();
        stub::serve(move |base| {
            HashMap::from([
                (
                    stub::LATEST.to_string(),
                    stub::latest_json(base, tag, &[&name, desktop::SUMS]),
                ),
                (format!("/download/{name}"), PACKAGE.to_vec()),
                (
                    "/download/SHA256SUMS-desktop".to_string(),
                    format!("{digest}  {name}\n").into_bytes(),
                ),
            ])
        })
    }

    #[test]
    fn check_finds_a_newer_release_and_nothing_for_the_running_version() {
        let api = release("v999.0.0", &sha256_hex(PACKAGE));
        let update = check(&api).expect("checked").expect("an update");
        assert_eq!(update.version, "999.0.0");
        let api = release(&format!("v{CURRENT}"), &sha256_hex(PACKAGE));
        assert_eq!(check(&api), Ok(None));
    }

    #[test]
    fn check_without_a_server_is_an_error() {
        let api = stub::serve(|_| HashMap::new());
        assert!(check(&api).is_err());
        assert!(check("http://127.0.0.1:1").is_err());
    }

    #[test]
    fn install_replaces_an_appimage_saves_an_msi_or_falls_back_to_the_page() {
        let api = release("v999.0.0", &sha256_hex(PACKAGE));
        let update = check(&api).unwrap().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let image = dir.path().join("ArchGuard.AppImage");
        std::fs::write(&image, b"old").unwrap();
        let how = How::AppImage(Some(image.clone()));
        assert_eq!(
            download_and_install(&update, &how),
            Ok(Installed::Replaced {
                path: image.clone()
            })
        );
        assert_eq!(std::fs::read(&image).unwrap(), PACKAGE);
        let how = How::Msi {
            temp_dir: dir.path().to_path_buf(),
        };
        assert!(matches!(
            download_and_install(&update, &how),
            Ok(Installed::RunBeforeQuit { .. })
        ));
        assert_eq!(
            download_and_install(&update, &How::Unsupported),
            Ok(Installed::Manual {
                page: update.page.clone()
            })
        );
    }

    #[test]
    fn a_checksum_mismatch_leaves_the_installed_app_untouched() {
        let api = release("v999.0.0", &"0".repeat(64));
        let update = check(&api).unwrap().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let image = dir.path().join("ArchGuard.AppImage");
        std::fs::write(&image, b"old").unwrap();
        let error = download_and_install(&update, &How::AppImage(Some(image.clone())))
            .expect_err("mismatch");
        assert!(error.contains("checksum mismatch"), "{error}");
        assert_eq!(std::fs::read(&image).unwrap(), b"old");
    }

    #[test]
    fn the_guard_starts_nothing_without_a_registered_program() {
        drop(QuitGuard);
        on_quit(Launch {
            program: "true".into(),
            args: Vec::new(),
        });
        assert!(ON_QUIT.with_borrow(Option::is_some));
        ON_QUIT.take();
        drop(QuitGuard);
    }
}
