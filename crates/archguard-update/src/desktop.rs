//! Desktop app updates. [`available`] finds the package of the latest release
//! for a target (`ArchGuard-<version>-<target>.{dmg,msi,AppImage}`, listed in
//! `SHA256SUMS-desktop`, see `scripts/release/package_desktop.py`);
//! [`install`] verifies the downloaded package and installs it the way the
//! running app was installed ([`How`]). A mismatch or any failure leaves the
//! installed app untouched.
use std::ffi::OsString;
use std::path::PathBuf;

use crate::appimage;
use crate::checksum::sha256_hex;
use crate::macos_bundle;
pub use crate::macos_bundle::{Runner, SystemRunner};
use crate::msi;
use crate::release;

/// The checksum file of the desktop packages in a release.
pub const SUMS: &str = "SHA256SUMS-desktop";

/// A newer desktop package than the running app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    pub version: String,
    /// The package file name, e.g. `ArchGuard-0.3.0-aarch64-apple-darwin.dmg`.
    pub package: String,
    /// Download URL of the package; fetch it with [`crate::http::download`]
    /// and [`crate::http::MAX_DOWNLOAD`].
    pub url: String,
    /// The SHA-256 that `SHA256SUMS-desktop` lists for the package.
    pub sha256: String,
    /// The release page, for a manual download.
    pub page: String,
}

/// How the running app was installed, and so how to replace it.
pub enum How {
    /// Linux: `APPIMAGE` (set by the AppImage runtime) names the running file;
    /// `None` means the app does not run as an AppImage.
    AppImage(Option<PathBuf>),
    /// macOS: the running executable, expected inside `X.app/Contents/MacOS/`.
    MacBundle {
        exe: PathBuf,
        runner: Box<dyn Runner>,
    },
    /// Windows: where the `.msi` is saved before `msiexec` runs it.
    Msi { temp_dir: PathBuf },
    /// No way to update this installation in place.
    Unsupported,
}

impl How {
    /// The installation of the running process on this platform.
    pub fn detect() -> Self {
        if cfg!(target_os = "macos") {
            return std::env::current_exe().map_or(How::Unsupported, |exe| How::MacBundle {
                exe,
                runner: Box::new(SystemRunner),
            });
        }
        if cfg!(windows) {
            return How::Msi {
                temp_dir: std::env::temp_dir(),
            };
        }
        if cfg!(target_os = "linux") {
            return How::AppImage(std::env::var_os("APPIMAGE").map(PathBuf::from));
        }
        How::Unsupported
    }
}

/// The result of [`install`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Installed {
    /// The app at `path` was replaced; it runs the new version after a restart.
    Replaced { path: PathBuf },
    /// Spawn this command right before the app quits (Windows `msiexec`).
    RunBeforeQuit {
        program: OsString,
        args: Vec<OsString>,
    },
    /// This installation cannot update itself; download from `page`.
    Manual { page: String },
}

/// The desktop package file name for `target`, or `None` without a package.
pub fn package_name(version: &str, target: &str) -> Option<String> {
    let suffix = match target {
        "aarch64-apple-darwin" | "x86_64-apple-darwin" => ".dmg",
        "x86_64-pc-windows-msvc" => ".msi",
        "x86_64-unknown-linux-gnu" => ".AppImage",
        _ => return None,
    };
    Some(format!("ArchGuard-{version}-{target}{suffix}"))
}

/// The latest release's package for `target` if it is newer than `current`.
/// `api` is the GitHub API base, normally [`release::api_base`].
pub fn available(api: &str, current: &str, target: &str) -> Result<Option<Update>, String> {
    let latest = release::latest(api)?;
    if !latest.is_newer_than(current)? {
        return Ok(None);
    }
    let package = package_name(&latest.version, target)
        .ok_or_else(|| format!("there is no desktop package for {target}"))?;
    Ok(Some(Update {
        url: latest.asset_url(&package)?,
        sha256: latest.expected_sha256(SUMS, &package)?,
        version: latest.version,
        package,
        page: latest.page,
    }))
}

/// Verifies `downloaded` against `update.sha256`, then installs it as `how` says.
pub fn install(downloaded: &[u8], update: &Update, how: &How) -> Result<Installed, String> {
    if sha256_hex(downloaded) != update.sha256 {
        return Err(format!("package checksum mismatch for {}", update.package));
    }
    let manual = || Installed::Manual {
        page: update.page.clone(),
    };
    match how {
        How::AppImage(Some(path)) => install_appimage(path, downloaded, manual),
        How::MacBundle { exe, runner } => match macos_bundle::bundle_of(exe) {
            Some(bundle) if macos_bundle::install(&bundle, downloaded, runner.as_ref())? => {
                Ok(Installed::Replaced { path: bundle })
            }
            _ => Ok(manual()),
        },
        How::Msi { temp_dir } => {
            let path = msi::save(temp_dir, &update.package, downloaded)
                .map_err(|e| format!("cannot save {}: {e}", update.package))?;
            let (program, args) = msi::command(&path);
            Ok(Installed::RunBeforeQuit { program, args })
        }
        How::AppImage(None) | How::Unsupported => Ok(manual()),
    }
}

fn install_appimage(
    path: &std::path::Path,
    image: &[u8],
    manual: impl FnOnce() -> Installed,
) -> Result<Installed, String> {
    match appimage::replace(path, image) {
        Ok(()) => Ok(Installed::Replaced {
            path: path.to_path_buf(),
        }),
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => Ok(manual()),
        Err(e) => Err(format!("cannot replace {}: {e}", path.display())),
    }
}
