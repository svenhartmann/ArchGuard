//! Where `desktop.toml` lives, reading it without losing data, and the
//! settings of the running app.
//!
//! `ARCHGUARD_DESKTOP_CONFIG_DIR` overrides the directory (tests,
//! snapshots). In snapshot mode without it nothing is read or written,
//! so a snapshot never touches the user's settings.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use toml_edit::DocumentMut;

use crate::config_editor::write_atomic;
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
    write_atomic(path, settings::render(doc, settings).as_bytes())
}

struct Current {
    path: Option<PathBuf>,
    doc: DocumentMut,
    settings: Settings,
}

/// Shows a failed write in the window ([`on_error`]).
type ErrorHook = Box<dyn Fn(String)>;

thread_local! {
    static ON_ERROR: RefCell<Option<ErrorHook>> = RefCell::new(None);
    static CURRENT: RefCell<Current> = RefCell::new(Current {
        path: None,
        doc: DocumentMut::new(),
        settings: Settings::default(),
    });
}

/// Load the app's settings from `path` (`None`: keep them in memory only).
/// Returns the notice of [`Loaded`].
pub fn init(path: Option<PathBuf>) -> Option<String> {
    // A symlinked file stays a link: writes rename over its target.
    let path = path.map(|p| std::fs::canonicalize(&p).unwrap_or(p));
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

/// Store the current settings. Errors go to stderr and to [`on_error`];
/// the app keeps running.
pub fn write() {
    let error = CURRENT.with_borrow_mut(|c| {
        let path = c.path.clone()?;
        if let Err(e) = merge_others(&path, &c.doc, &mut c.settings) {
            return Some(format!("{}: {e}", path.display()));
        }
        let e = save(&path, &mut c.doc, &c.settings).err()?;
        Some(format!("{}: {e}", path.display()))
    });
    let Some(error) = error else { return };
    eprintln!("archguard-desktop: cannot write {error}");
    // Outside the borrow: the window may read the settings.
    ON_ERROR.with_borrow(|f| f.as_ref().map(|f| f(error)));
}

/// Take over what another app instance changed in `path` since `doc`
/// (this instance's last read or write): per value, per project and per
/// recent entry, what this instance left as in `doc` takes the file's
/// state, so the other instance's changes and removals stick; what this
/// instance changed stays. A file that does not parse is an error: it is
/// not overwritten.
fn merge_others(path: &Path, doc: &DocumentMut, settings: &mut Settings) -> Result<(), String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Ok(());
    };
    let (_, disk) = match settings::parse(&text) {
        Ok(parsed) => parsed,
        // Already copied to `.bak` ([`load`]): replacing it loses nothing.
        Err(_) if std::fs::read_to_string(backup_path(path)).is_ok_and(|b| b == text) => {
            return Ok(());
        }
        Err(e) => return Err(e.lines().next().unwrap_or_default().trim().to_string()),
    };
    let (_, base) = settings::parse(&doc.to_string())?;
    let s = settings;
    s.theme = pick(base.theme, disk.theme, s.theme);
    s.language = pick(
        base.language,
        disk.language,
        std::mem::take(&mut s.language),
    );
    s.window = pick(base.window, disk.window, s.window);
    s.graph = pick(base.graph, disk.graph, s.graph);
    s.session = pick(base.session, disk.session, s.session.take());
    if s.recent == base.recent {
        s.recent = disk.recent;
    } else {
        let mine = std::mem::take(&mut s.recent);
        let keep = |p: &PathBuf| {
            pick(
                base.recent.contains(p),
                disk.recent.contains(p),
                mine.contains(p),
            )
        };
        let new = disk.recent.iter().filter(|p| !mine.contains(p));
        s.recent = mine
            .iter()
            .chain(new)
            .filter(|p| keep(p))
            .cloned()
            .collect();
        s.recent.truncate(settings::RECENT_MAX);
    }
    let keys: std::collections::BTreeSet<String> = [&base.projects, &disk.projects, &s.projects]
        .into_iter()
        .flat_map(|m| m.keys().cloned())
        .collect();
    for key in keys {
        let view = pick(
            base.projects.get(&key),
            disk.projects.get(&key),
            s.projects.get(&key),
        )
        .cloned();
        match view {
            Some(view) => s.projects.insert(key, view),
            None => s.projects.remove(&key),
        };
    }
    Ok(())
}

/// Three-way merge of one value: unchanged here (`mine == base`) takes
/// the file's, else this instance's.
fn pick<T: PartialEq>(base: T, disk: T, mine: T) -> T {
    if mine == base { disk } else { mine }
}

/// Call `f` with the message of every failed [`write`].
pub fn on_error(f: impl Fn(String) + 'static) {
    ON_ERROR.set(Some(Box::new(f)));
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
        assert_eq!(entries(&dir), [FILE], "no temporary file left");
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

    fn entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(dir)
            .expect("dir")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// A save that cannot replace the file leaves no temporary file behind.
    #[test]
    fn a_failed_save_leaves_no_temporary_file() {
        let dir = temp("failed-save");
        let path = dir.join(FILE);
        // A non-empty directory where the file should be: rename fails.
        std::fs::create_dir_all(path.join("in-the-way")).expect("dir");
        assert!(save(&path, &mut DocumentMut::new(), &Settings::default()).is_err());
        assert_eq!(entries(&dir), [FILE]);
    }

    #[cfg(unix)]
    #[test]
    fn save_keeps_the_file_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp("permissions");
        let path = dir.join(FILE);
        std::fs::write(&path, "").expect("write");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("chmod");
        save(&path, &mut DocumentMut::new(), &Settings::default()).expect("written");
        let mode = std::fs::metadata(&path).expect("meta").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    /// A second instance started earlier keeps the recent projects and
    /// project views another instance wrote since; what this instance
    /// removed itself stays removed.
    #[test]
    fn a_write_keeps_what_another_instance_added() {
        let dir = temp("two-instances");
        let path = dir.join(FILE);
        std::fs::write(&path, "# mine\nrecent = [\"/w/old\"]\n").expect("write");
        init(Some(path.clone()));
        // Instance A, started later, opens /w/a.
        let mut a = load(&path);
        a.settings.add_recent(Path::new("/w/a"));
        a.settings
            .set_project(Path::new("/w/a"), Default::default());
        save(&path, &mut a.doc, &a.settings).expect("written");
        // This instance changes the theme and drops /w/old.
        update(|s| {
            s.theme = Theme::Dark;
            s.recent.retain(|p| p != Path::new("/w/old"));
        });
        let back = load(&path).settings;
        assert_eq!(back.theme, Theme::Dark);
        assert_eq!(back.recent, [PathBuf::from("/w/a")]);
        assert!(back.project(Path::new("/w/a")).is_some(), "{back:?}");
        assert!(
            std::fs::read_to_string(&path)
                .expect("read")
                .starts_with("# mine")
        );
        // Clearing the list after the merge sticks.
        update(|s| s.recent.clear());
        assert!(load(&path).settings.recent.is_empty());
        init(None);
    }

    /// What another instance changed or removed survives a write of this
    /// instance that did not touch it.
    #[test]
    fn a_write_keeps_what_another_instance_changed_or_removed() {
        let dir = temp("two-instances-changed");
        let path = dir.join(FILE);
        std::fs::write(
            &path,
            "recent = [\"/w/p\", \"/w/q\"]\n\
             [projects.\"/w/p\"]\ntab = \"graph\"\n\
             [projects.\"/w/q\"]\ntab = \"dsm\"\n",
        )
        .expect("write");
        init(Some(path.clone()));
        // Instance A rearranges /w/p, forgets /w/q, clears the recent list
        // and picks a theme.
        let mut a = load(&path);
        a.settings.projects.get_mut("/w/p").expect("p").tab = "calls".into();
        a.settings.projects.remove("/w/q");
        a.settings.recent.clear();
        a.settings.theme = Theme::Light;
        save(&path, &mut a.doc, &a.settings).expect("written");
        // This instance only records its window size, twice.
        update(|s| s.window = Some((800, 600)));
        update(|s| s.window = Some((900, 700)));
        let back = load(&path).settings;
        assert_eq!(back.window, Some((900, 700)));
        assert_eq!(back.theme, Theme::Light);
        assert!(back.recent.is_empty(), "{back:?}");
        assert_eq!(back.projects["/w/p"].tab, "calls");
        assert!(back.project(Path::new("/w/q")).is_none(), "{back:?}");
        // What this instance changes itself still wins.
        update(|s| s.projects.get_mut("/w/p").expect("p").tab = "dsm".into());
        assert_eq!(load(&path).settings.projects["/w/p"].tab, "dsm");
        init(None);
    }

    /// A file that stops parsing while the app runs is not replaced; the
    /// window reports it instead.
    #[test]
    fn a_file_broken_on_disk_is_kept_and_reported() {
        let dir = temp("broken-on-disk");
        let path = dir.join(FILE);
        std::fs::write(&path, "").expect("write");
        init(Some(path.clone()));
        let broken = "theme = \"dark\"\n[[[ not toml\n";
        std::fs::write(&path, broken).expect("write");
        let seen = std::rc::Rc::new(RefCell::new(Vec::<String>::new()));
        let sink = seen.clone();
        on_error(move |e| sink.borrow_mut().push(e));
        update(|s| s.language = "en".into());
        assert_eq!(std::fs::read_to_string(&path).expect("read"), broken);
        let seen = seen.borrow();
        assert_eq!(seen.len(), 1, "{seen:?}");
        assert!(seen[0].contains(&path.display().to_string()), "{seen:?}");
        init(None);
    }

    /// A file broken at start is saved as `.bak`, so the next write
    /// replaces it instead of failing on it forever.
    #[test]
    fn a_file_broken_at_start_is_replaced_by_the_next_write() {
        let dir = temp("broken-at-start");
        let path = dir.join(FILE);
        let broken = "theme = \"dark\"\n[[[ not toml\n";
        std::fs::write(&path, broken).expect("write");
        assert!(init(Some(path.clone())).is_some(), "reported");
        let seen = std::rc::Rc::new(RefCell::new(Vec::<String>::new()));
        let sink = seen.clone();
        on_error(move |e| sink.borrow_mut().push(e));
        update(|s| s.language = "en".into());
        assert!(seen.borrow().is_empty(), "{:?}", seen.borrow());
        assert_eq!(load(&path).settings.language, "en");
        assert_eq!(
            std::fs::read_to_string(dir.join("desktop.toml.bak")).expect("backup"),
            broken
        );
        init(None);
    }

    /// A symlinked settings file stays a link; the write goes to its target.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_file_stays_a_link() {
        let dir = temp("symlink");
        let real = dir.join("real.toml");
        std::fs::write(&real, "").expect("write");
        let link = dir.join(FILE);
        std::os::unix::fs::symlink(&real, &link).expect("link");
        init(Some(link.clone()));
        update(|s| s.language = "de".into());
        init(None);
        assert!(
            std::fs::symlink_metadata(&link)
                .expect("meta")
                .file_type()
                .is_symlink()
        );
        assert!(
            std::fs::read_to_string(&real)
                .expect("read")
                .contains("language = \"de\"")
        );
    }

    /// The window shows a failed write; stderr alone is invisible when the
    /// app starts from Finder.
    #[test]
    fn a_failed_write_is_reported() {
        let dir = temp("reported");
        let path = dir.join(FILE);
        std::fs::create_dir_all(path.join("in-the-way")).expect("dir");
        init(Some(path.clone()));
        let seen = std::rc::Rc::new(RefCell::new(Vec::<String>::new()));
        let sink = seen.clone();
        on_error(move |e| sink.borrow_mut().push(e));
        update(|s| s.language = "en".into());
        let seen = seen.borrow();
        assert_eq!(seen.len(), 1, "{seen:?}");
        assert!(seen[0].contains(&path.display().to_string()), "{seen:?}");
        init(None);
    }
}
