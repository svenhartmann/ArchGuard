//! Where `desktop.toml` lives, reading it without losing data, and the
//! settings of the running app.
//!
//! `ARCHGUARD_DESKTOP_CONFIG_DIR` overrides the directory (tests,
//! snapshots). In snapshot mode without it nothing is read or written,
//! so a snapshot never touches the user's settings.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use toml_edit::DocumentMut;

use crate::settings::{self, Settings};

pub const FILE: &str = "desktop.toml";

/// The settings directory for `os` (`std::env::consts::OS`):
/// macOS `~/Library/Application Support/ArchGuard`, Windows
/// `%APPDATA%\ArchGuard`, else `$XDG_CONFIG_HOME/archguard` (default
/// `~/.config/archguard`).
pub fn config_dir(os: &str, env: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let var = |k: &str| env(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    if let Some(dir) = var("ARCHGUARD_DESKTOP_CONFIG_DIR") {
        return Some(dir);
    }
    match os {
        "macos" => Some(var("HOME")?.join("Library/Application Support/ArchGuard")),
        "windows" => Some(var("APPDATA")?.join("ArchGuard")),
        _ => {
            // The XDG spec ignores relative values.
            let xdg = var("XDG_CONFIG_HOME").filter(|d| d.is_absolute());
            Some(
                xdg.or_else(|| Some(var("HOME")?.join(".config")))?
                    .join("archguard"),
            )
        }
    }
}

/// The file the running app uses, `None` when it must not persist.
pub fn default_path() -> Option<PathBuf> {
    let env = |k: &str| std::env::var(k).ok();
    let overridden = env("ARCHGUARD_DESKTOP_CONFIG_DIR").is_some_and(|d| !d.is_empty());
    // Only snapshot builds render PNGs; others ignore the variable.
    let snapshot = cfg!(feature = "snapshot") && env("ARCHGUARD_DESKTOP_SNAPSHOT").is_some();
    if snapshot && !overridden {
        return None;
    }
    Some(config_dir(std::env::consts::OS, env)?.join(FILE))
}

/// A read settings file.
pub struct Loaded {
    pub doc: DocumentMut,
    pub settings: Settings,
    /// Set when the file could not be read: what happened and where the
    /// copy of the old file is.
    pub notice: Option<String>,
}

/// Read `path`. A missing file gives the defaults. A file that does not
/// parse is copied to `desktop.toml.bak` first, so writing the defaults
/// later loses nothing.
pub fn load(path: &Path) -> Loaded {
    let fresh = |notice| Loaded {
        doc: DocumentMut::new(),
        settings: Settings::default(),
        notice,
    };
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return fresh(None),
        Err(e) => return fresh(Some(backup(path, &e.to_string()))),
    };
    match settings::parse(&text) {
        Ok((doc, settings)) => Loaded {
            doc,
            settings,
            notice: None,
        },
        Err(e) => fresh(Some(backup(path, &e))),
    }
}

fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".bak");
    path.with_file_name(name)
}

fn backup(path: &Path, error: &str) -> String {
    let bak = backup_path(path);
    let first_line = error.lines().next().unwrap_or(error).trim();
    match std::fs::copy(path, &bak) {
        Ok(_) => format!("{}: {first_line} → {}", path.display(), bak.display()),
        Err(e) => format!("{}: {first_line} ({e})", path.display()),
    }
}

/// Write `settings` into `doc` and replace `path` atomically.
pub fn save(path: &Path, doc: &mut DocumentMut, settings: &Settings) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = settings::render(doc, settings);
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

struct Current {
    path: Option<PathBuf>,
    doc: DocumentMut,
    settings: Settings,
}

thread_local! {
    static CURRENT: RefCell<Current> = RefCell::new(Current {
        path: None,
        doc: DocumentMut::new(),
        settings: Settings::default(),
    });
}

/// Load the app's settings from `path` (`None`: keep them in memory only).
/// Returns the notice of [`Loaded`].
pub fn init(path: Option<PathBuf>) -> Option<String> {
    let loaded = path.as_deref().map(load);
    CURRENT.with_borrow_mut(|c| {
        c.path = path;
        let loaded = loaded?;
        c.doc = loaded.doc;
        c.settings = loaded.settings;
        loaded.notice
    })
}

/// Read the current settings.
pub fn get<R>(f: impl FnOnce(&Settings) -> R) -> R {
    CURRENT.with_borrow(|c| f(&c.settings))
}

/// Change the settings in memory; [`write`] stores them.
pub fn edit(f: impl FnOnce(&mut Settings)) {
    CURRENT.with_borrow_mut(|c| f(&mut c.settings));
}

/// Store the current settings; errors go to stderr, the app keeps running.
pub fn write() {
    CURRENT.with_borrow_mut(|c| {
        let Some(path) = c.path.clone() else {
            return;
        };
        if let Err(e) = save(&path, &mut c.doc, &c.settings) {
            eprintln!("archguard-desktop: cannot write {}: {e}", path.display());
        }
    });
}

/// [`edit`], then [`write`].
pub fn update(f: impl FnOnce(&mut Settings)) {
    edit(f);
    write();
}

/// Read keys that [`Settings`] does not model, e.g. `[updates]`
/// (`update_ui.rs`), from the loaded file.
pub fn get_doc<R>(f: impl FnOnce(&DocumentMut) -> R) -> R {
    CURRENT.with_borrow(|c| f(&c.doc))
}

/// Change such keys in the loaded file, then [`write`]; writing the
/// settings later keeps them.
pub fn update_doc(f: impl FnOnce(&mut DocumentMut)) {
    CURRENT.with_borrow_mut(|c| f(&mut c.doc));
    write();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Theme;

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        move |k| {
            pairs
                .iter()
                .find(|(n, _)| *n == k)
                .map(|(_, v)| v.to_string())
        }
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "archguard-desktop-test-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir
    }

    #[test]
    fn platform_config_dirs() {
        let e = env(&[
            ("HOME", "/Users/me"),
            ("APPDATA", r"C:\Users\me\AppData\Roaming"),
            ("XDG_CONFIG_HOME", "/home/me/.cfg"),
        ]);
        assert_eq!(
            config_dir("macos", &e),
            Some(PathBuf::from(
                "/Users/me/Library/Application Support/ArchGuard"
            ))
        );
        assert_eq!(
            config_dir("windows", &e),
            Some(PathBuf::from(r"C:\Users\me\AppData\Roaming").join("ArchGuard"))
        );
        assert_eq!(
            config_dir("linux", &e),
            Some(PathBuf::from("/home/me/.cfg/archguard"))
        );
    }

    #[test]
    fn linux_falls_back_to_dot_config_and_ignores_relative_xdg() {
        let e = env(&[("HOME", "/home/me"), ("XDG_CONFIG_HOME", "rel")]);
        assert_eq!(
            config_dir("linux", e),
            Some(PathBuf::from("/home/me/.config/archguard"))
        );
        assert_eq!(config_dir("linux", env(&[])), None);
        assert_eq!(config_dir("windows", env(&[("HOME", "/h")])), None);
    }

    #[test]
    fn the_override_wins_on_every_platform() {
        let e = env(&[("HOME", "/h"), ("ARCHGUARD_DESKTOP_CONFIG_DIR", "/tmp/ag")]);
        for os in ["macos", "windows", "linux"] {
            assert_eq!(config_dir(os, &e), Some(PathBuf::from("/tmp/ag")));
        }
    }

    #[test]
    fn a_missing_file_loads_the_defaults_without_notice() {
        let dir = temp("missing");
        let l = load(&dir.join(FILE));
        assert_eq!(l.settings, Settings::default());
        assert!(l.notice.is_none());
    }

    #[test]
    fn save_creates_the_directory_and_load_reads_it_back() {
        let dir = temp("roundtrip").join("nested");
        let path = dir.join(FILE);
        let mut s = Settings {
            theme: Theme::Dark,
            ..Settings::default()
        };
        s.add_recent(Path::new("/w/shop"));
        save(&path, &mut DocumentMut::new(), &s).expect("written");
        assert_eq!(load(&path).settings, s);
        assert!(!dir.join("desktop.toml.tmp").exists());
    }

    #[test]
    fn a_broken_file_is_kept_as_bak_and_reported() {
        let dir = temp("broken");
        let path = dir.join(FILE);
        let broken = "theme = \"dark\"\n[[[ not toml\n";
        std::fs::write(&path, broken).expect("write");
        let l = load(&path);
        assert_eq!(l.settings, Settings::default());
        let notice = l.notice.expect("a notice");
        assert!(notice.contains("desktop.toml.bak"), "{notice}");
        assert_eq!(
            std::fs::read_to_string(dir.join("desktop.toml.bak")).expect("backup"),
            broken
        );
        // Saving afterwards replaces the broken file, the copy stays.
        save(&path, &mut DocumentMut::new(), &l.settings).expect("written");
        assert!(dir.join("desktop.toml.bak").exists());
        assert!(load(&path).notice.is_none());
    }

    #[test]
    fn the_running_settings_write_only_with_a_path() {
        let dir = temp("current");
        let path = dir.join(FILE);
        std::fs::write(&path, "# mine\nlanguage = \"de\"\n").expect("write");
        assert!(init(Some(path.clone())).is_none());
        assert_eq!(get(|s| s.language.clone()), "de");
        update(|s| s.add_recent(Path::new("/w/api")));
        let text = std::fs::read_to_string(&path).expect("written");
        assert!(text.starts_with("# mine"), "{text}");
        assert!(text.contains("/w/api"), "{text}");
        // In memory only: nothing is written.
        init(None);
        update(|s| s.language = "en".into());
        assert!(
            !std::fs::read_to_string(&path)
                .expect("kept")
                .contains("language = \"en\"")
        );
    }
}
