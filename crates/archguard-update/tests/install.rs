//! `desktop::install` on temp dirs: AppImage replacement, `.app` bundle swap
//! with a fake `hdiutil`, and the Windows installer command. Nothing is
//! launched and no real disk image is mounted.
#![cfg(unix)]

use std::cell::RefCell;
use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use archguard_update::checksum::sha256_hex;
use archguard_update::desktop::{self, How, Installed, Runner, Update};

const NEW: &[u8] = b"new package";
const EXE: &str = "Contents/MacOS/archguard-desktop";

fn update(package: &str, data: &[u8]) -> Update {
    Update {
        version: "0.9.0".into(),
        package: package.into(),
        url: "http://127.0.0.1:9/unused".into(),
        sha256: sha256_hex(data),
        page: "https://example.invalid/v0.9.0".into(),
    }
}

fn manual() -> Installed {
    Installed::Manual {
        page: "https://example.invalid/v0.9.0".into(),
    }
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

/// Directory listing names, to show that no staging files are left behind.
fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn appimage_is_replaced_atomically_and_executable() {
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("ArchGuard-0.2.1.AppImage");
    std::fs::write(&image, b"old").unwrap();
    let how = How::AppImage(Some(image.clone()));
    let result = desktop::install(NEW, &update("p.AppImage", NEW), &how).unwrap();
    assert_eq!(
        result,
        Installed::Replaced {
            path: image.clone()
        }
    );
    assert_eq!(std::fs::read(&image).unwrap(), NEW);
    assert_eq!(mode(&image), 0o755);
    assert_eq!(names(dir.path()), ["ArchGuard-0.2.1.AppImage"]);
}

#[test]
fn appimage_mismatch_or_read_only_folder_keeps_the_old_file() {
    let dir = tempfile::tempdir().unwrap();
    let image = dir.path().join("ArchGuard.AppImage");
    std::fs::write(&image, b"old").unwrap();
    let how = How::AppImage(Some(image.clone()));
    let wrong = update("p.AppImage", b"something else");
    assert!(desktop::install(NEW, &wrong, &how).is_err());
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
    let result = desktop::install(NEW, &update("p.AppImage", NEW), &how);
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(result.unwrap(), manual());
    assert_eq!(std::fs::read(&image).unwrap(), b"old");
    assert_eq!(names(dir.path()), ["ArchGuard.AppImage"]);
}

#[test]
fn not_running_as_appimage_or_unsupported_means_manual_download() {
    let pkg = update("p", NEW);
    assert_eq!(
        desktop::install(NEW, &pkg, &How::AppImage(None)).unwrap(),
        manual()
    );
    assert_eq!(
        desktop::install(NEW, &pkg, &How::Unsupported).unwrap(),
        manual()
    );
}

/// Records every call; `attach` mounts a fake image holding `ArchGuard.app`
/// unless `empty`, and fails when `fail_attach` is set.
#[derive(Clone, Default)]
struct FakeHdiutil {
    calls: Rc<RefCell<Vec<Vec<String>>>>,
    fail_attach: bool,
    empty: bool,
}

impl Runner for FakeHdiutil {
    fn run(&self, program: &str, args: &[OsString]) -> Result<(), String> {
        let mut call = vec![program.to_string()];
        call.extend(args.iter().map(|a| a.to_string_lossy().into_owned()));
        self.calls.borrow_mut().push(call.clone());
        if call[1] != "attach" {
            return Ok(());
        }
        if self.fail_attach {
            return Err("hdiutil: attach failed".into());
        }
        let mount = PathBuf::from(&call[call.len() - 2]);
        if !self.empty {
            fake_app(&mount.join("ArchGuard.app"), b"new binary");
            std::os::unix::fs::symlink("/Applications", mount.join("Applications")).unwrap();
        }
        Ok(())
    }
}

fn fake_app(app: &Path, binary: &[u8]) {
    std::fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
    std::fs::create_dir_all(app.join("Contents/Resources")).unwrap();
    std::fs::write(app.join(EXE), binary).unwrap();
    std::fs::set_permissions(app.join(EXE), std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(app.join("Contents/Info.plist"), b"plist").unwrap();
    std::os::unix::fs::symlink("../Info.plist", app.join("Contents/Resources/link")).unwrap();
}

/// An installed `My.app` in a fresh folder, and the matching `How`.
fn installed_bundle(runner: &FakeHdiutil) -> (tempfile::TempDir, PathBuf, How) {
    let dir = tempfile::tempdir().unwrap();
    let bundle = dir.path().join("My.app");
    fake_app(&bundle, b"old binary");
    let how = How::MacBundle {
        exe: bundle.join(EXE),
        runner: Box::new(runner.clone()),
    };
    (dir, bundle, how)
}

#[test]
fn bundle_is_swapped_for_the_app_from_the_mounted_image() {
    let runner = FakeHdiutil::default();
    let (dir, bundle, how) = installed_bundle(&runner);
    let result = desktop::install(NEW, &update("p.dmg", NEW), &how).unwrap();
    assert_eq!(
        result,
        Installed::Replaced {
            path: bundle.clone()
        }
    );
    assert_eq!(std::fs::read(bundle.join(EXE)).unwrap(), b"new binary");
    assert_eq!(mode(&bundle.join(EXE)), 0o755);
    let link = std::fs::read_link(bundle.join("Contents/Resources/link")).unwrap();
    assert_eq!(link, Path::new("../Info.plist"));
    assert_eq!(
        names(dir.path()),
        ["My.app"],
        "no staging or old bundle left"
    );
    let calls = runner.calls.borrow();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls[0][..5],
        ["hdiutil", "attach", "-nobrowse", "-readonly", "-mountpoint"]
    );
    assert!(calls[0][6].ends_with("ArchGuard.dmg"));
    assert_eq!(calls[1][..2], ["hdiutil", "detach"]);
    assert_eq!(calls[1][2], calls[0][5], "detaches the mount it attached");
}

#[test]
fn bundle_checksum_mismatch_never_mounts_and_keeps_the_app() {
    let runner = FakeHdiutil::default();
    let (dir, bundle, how) = installed_bundle(&runner);
    assert!(desktop::install(NEW, &update("p.dmg", b"other"), &how).is_err());
    assert!(runner.calls.borrow().is_empty());
    assert_eq!(std::fs::read(bundle.join(EXE)).unwrap(), b"old binary");
    assert_eq!(names(dir.path()), ["My.app"]);
}

#[test]
fn failed_attach_or_missing_app_keeps_the_bundle_and_detaches() {
    for runner in [
        FakeHdiutil {
            fail_attach: true,
            ..FakeHdiutil::default()
        },
        FakeHdiutil {
            empty: true,
            ..FakeHdiutil::default()
        },
    ] {
        let (dir, bundle, how) = installed_bundle(&runner);
        assert!(desktop::install(NEW, &update("p.dmg", NEW), &how).is_err());
        assert_eq!(std::fs::read(bundle.join(EXE)).unwrap(), b"old binary");
        assert_eq!(names(dir.path()), ["My.app"]);
        let detached = runner.calls.borrow().iter().any(|c| c[1] == "detach");
        assert_eq!(
            detached, runner.empty,
            "detach exactly when attach succeeded"
        );
    }
}

#[test]
fn not_a_bundle_or_read_only_folder_means_manual_download() {
    let runner = FakeHdiutil::default();
    let loose = How::MacBundle {
        exe: PathBuf::from("/usr/local/bin/archguard-desktop"),
        runner: Box::new(runner.clone()),
    };
    assert_eq!(
        desktop::install(NEW, &update("p.dmg", NEW), &loose).unwrap(),
        manual()
    );
    let (dir, bundle, how) = installed_bundle(&runner);
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o555)).unwrap();
    let result = desktop::install(NEW, &update("p.dmg", NEW), &how);
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(result.unwrap(), manual());
    assert!(runner.calls.borrow().is_empty());
    assert_eq!(std::fs::read(bundle.join(EXE)).unwrap(), b"old binary");
}

#[test]
fn msi_is_saved_and_msiexec_is_returned_not_run() {
    let dir = tempfile::tempdir().unwrap();
    let how = How::Msi {
        temp_dir: dir.path().to_path_buf(),
    };
    let package = "ArchGuard-0.9.0-x86_64-pc-windows-msvc.msi";
    let result = desktop::install(NEW, &update(package, NEW), &how).unwrap();
    let msi = dir.path().join(package);
    let args: Vec<OsString> = vec!["/i".into(), msi.clone().into(), "/passive".into()];
    assert_eq!(
        result,
        Installed::RunBeforeQuit {
            program: "msiexec".into(),
            args
        }
    );
    assert_eq!(std::fs::read(&msi).unwrap(), NEW);
}

#[test]
fn msi_mismatch_saves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let how = How::Msi {
        temp_dir: dir.path().to_path_buf(),
    };
    assert!(desktop::install(NEW, &update("a.msi", b"other"), &how).is_err());
    assert!(names(dir.path()).is_empty());
}

#[test]
fn package_names_match_the_packaging_script() {
    let name = |t| desktop::package_name("1.2.3", t);
    assert_eq!(
        name("aarch64-apple-darwin").unwrap(),
        "ArchGuard-1.2.3-aarch64-apple-darwin.dmg"
    );
    assert_eq!(
        name("x86_64-apple-darwin").unwrap(),
        "ArchGuard-1.2.3-x86_64-apple-darwin.dmg"
    );
    assert_eq!(
        name("x86_64-pc-windows-msvc").unwrap(),
        "ArchGuard-1.2.3-x86_64-pc-windows-msvc.msi"
    );
    assert_eq!(
        name("x86_64-unknown-linux-gnu").unwrap(),
        "ArchGuard-1.2.3-x86_64-unknown-linux-gnu.AppImage"
    );
    assert_eq!(name("aarch64-unknown-linux-gnu"), None);
}
