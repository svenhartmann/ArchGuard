//! File → Compose projects… (ui/project_set_dialog.slint): the dialog's
//! model, writing the set file and opening it (ADR 0024). The folder checks
//! (empty, exists, is a folder, no duplicate after canonicalisation) are the
//! engine's `resolve_project_dirs`, run live and again by `write_project_set`;
//! the dialog itself only blocks what is not a folder check: name and count,
//! and a repeated path as a hint.

use std::cell::RefCell;
use std::path::{Component, Path, PathBuf};

use archguard_engine::{ProjectSet, read_project_set, resolve_project_dirs, write_project_set};
use slint::{ComponentHandle, SharedString};

use crate::documents::with_store;
use crate::documents_set::{active_set, is_set_file};
use crate::settings_file;
use crate::tab_support::{index, model, with_ui};
use crate::ui::{AppWindow, ProjectSetData};

const SUFFIX: &str = ".archguard-set.toml";
const MAX_NAME_CHARS: usize = 200;
/// A folder further from the set file than this many `..` is written absolute.
const MAX_UPS: usize = 2;

/// Why saving is blocked; the number is what the dialog's texts are keyed by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Block {
    Name = 1,
    TooFew = 2,
    TooMany = 3,
    Duplicate = 4,
}

/// The dialog's state: a name and the folders, the first is the primary project.
#[derive(Debug, Default, Clone)]
pub struct Dialog {
    pub name: String,
    pub folders: Vec<PathBuf>,
}

impl Dialog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_set(name: String, folders: Vec<PathBuf>) -> Self {
        Dialog { name, folders }
    }

    /// Appends `dir`; false when it is listed already.
    pub fn add(&mut self, dir: PathBuf) -> bool {
        let new = !self.folders.contains(&dir);
        if new {
            self.folders.push(dir);
        }
        new
    }

    pub fn remove(&mut self, i: usize) {
        if i < self.folders.len() {
            self.folders.remove(i);
        }
    }

    /// Moves folder `i` one place up; from place 1 it becomes the primary project.
    pub fn up(&mut self, i: usize) {
        if i > 0 && i < self.folders.len() {
            self.folders.swap(i - 1, i);
        }
    }

    /// The engine's folder check (empty, missing, not a folder, duplicate after
    /// canonicalisation), the one place for these rules; the reason names the
    /// position only.
    pub fn folder_error(&self) -> Option<String> {
        resolve_project_dirs(Path::new("/"), &self.folders).err()
    }

    pub fn block(&self) -> Option<Block> {
        let n = &self.name;
        if n.trim().is_empty()
            || n.chars().count() > MAX_NAME_CHARS
            || n.chars().any(char::is_control)
        {
            return Some(Block::Name);
        }
        match self.folders.len() {
            0 | 1 => return Some(Block::TooFew),
            2..=16 => {}
            _ => return Some(Block::TooMany),
        }
        let dup = self
            .folders
            .iter()
            .enumerate()
            .any(|(i, f)| self.folders[..i].contains(f));
        dup.then_some(Block::Duplicate)
    }
}

/// `dir` as seen from `base` (both absolute): relative when the set file's
/// folder is at most [`MAX_UPS`] levels below their common parent, so a set
/// inside a repository stays portable; else absolute, as in the config folder.
pub fn relative(base: &Path, dir: &Path) -> PathBuf {
    let (b, d): (Vec<Component>, Vec<Component>) =
        (base.components().collect(), dir.components().collect());
    let common = b.iter().zip(&d).take_while(|(x, y)| x == y).count();
    let ups = b.len() - common;
    // Only the root in common (or another drive): nothing to be relative to.
    if common < 2 || ups > MAX_UPS {
        return dir.to_path_buf();
    }
    let mut out: PathBuf = std::iter::repeat_n("..", ups).collect();
    out.extend(&d[common..]);
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

/// The last component of `dir`, shown first in a row; the whole path for a root.
pub fn folder_name(dir: &Path) -> String {
    dir.file_name().map_or_else(
        || dir.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// The default file name for a set: its name without characters a file name cannot hold.
pub fn file_name(name: &str) -> String {
    let clean: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    format!("{clean}{SUFFIX}")
}

/// Writes the dialog as a set file at `file`, folders relative where [`relative`] says so.
/// The error is the engine's text: the file's path and the reason, no file contents.
pub fn write(dialog: &Dialog, file: &Path) -> Result<(), String> {
    let base = file.parent().and_then(|p| p.canonicalize().ok());
    let projects = dialog
        .folders
        .iter()
        .map(|f| {
            let f = f.canonicalize().unwrap_or_else(|_| f.clone());
            base.as_deref()
                .map_or_else(|| f.clone(), |b| relative(b, &f))
        })
        .collect();
    let set = ProjectSet {
        name: dialog.name.clone(),
        projects,
    };
    write_project_set(file, &set).map_err(|e| e.to_string())
}

/// Opens a project folder or set file as the active document.
type Opener = fn(&AppWindow, PathBuf);

thread_local! {
    /// Handed in by `wire`: this module cannot import `documents_ui`, which
    /// reaches the snapshot hooks (a file cycle).
    static OPEN: std::cell::Cell<Option<Opener>> = const { std::cell::Cell::new(None) };
    static DIALOG: RefCell<Dialog> = RefCell::new(Dialog::new());
}

fn edit(ui: &AppWindow, f: impl FnOnce(&mut Dialog)) {
    DIALOG.with_borrow_mut(f);
    push(ui, "");
}

/// Show the model in the dialog; `error` is the last save's failure.
fn push(ui: &AppWindow, error: &str) {
    let d = ui.global::<ProjectSetData>();
    let found = DIALOG.with_borrow(Dialog::folder_error);
    let error = if error.is_empty() {
        found.as_deref().unwrap_or("")
    } else {
        error
    };
    DIALOG.with_borrow(|m| {
        d.set_folders(model(
            m.folders
                .iter()
                .map(|p| SharedString::from(p.display().to_string())),
        ));
        d.set_names(model(
            m.folders.iter().map(|p| SharedString::from(folder_name(p))),
        ));
        d.set_reason(m.block().map_or(0, |b| b as i32));
        let last = m.folders.len() as i32 - 1;
        d.set_selected(d.get_selected().min(last));
    });
    d.set_error(error.into());
}

/// `<config_dir>/sets/`, created when missing.
fn sets_dir() -> Option<PathBuf> {
    let dir =
        settings_file::config_dir(std::env::consts::OS, |k| std::env::var(k).ok())?.join("sets");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// Open the dialog, filled with the active document's set if it is one.
fn show(ui: &AppWindow) {
    let set = active_set().and_then(|f| read_project_set(&f).ok());
    DIALOG.set(set.map_or_else(Dialog::new, |s| Dialog::from_set(s.name, s.projects)));
    let d = ui.global::<ProjectSetData>();
    d.set_name(DIALOG.with_borrow(|m| m.name.clone()).into());
    d.set_selected(-1);
    push(ui, "");
    d.set_dialog(true);
}

fn add(ui: &AppWindow) {
    let start = DIALOG
        .with_borrow(|m| {
            m.folders
                .last()
                .and_then(|f| f.parent().map(Path::to_path_buf))
        })
        .or_else(|| with_store(|s| s.active().map(|d| d.root.clone())));
    let mut dialog =
        rfd::FileDialog::new().set_title(ui.global::<ProjectSetData>().get_add_title().as_str());
    if let Some(dir) = start {
        dialog = dialog.set_directory(dir);
    }
    if let Some(dir) = dialog.pick_folder() {
        edit(ui, |m| {
            m.add(dir);
        });
    }
}

fn save(ui: &AppWindow) {
    let d = ui.global::<ProjectSetData>();
    let Some((name, blocked)) = DIALOG.with_borrow(|m| Some((m.name.clone(), m.block()))) else {
        return;
    };
    if blocked.is_some() || DIALOG.with_borrow(|m| m.folder_error().is_some()) {
        return;
    }
    let mut dialog = rfd::FileDialog::new()
        .set_title(d.get_save_title().as_str())
        .add_filter(d.get_filter_name().as_str(), &["toml"])
        .set_file_name(file_name(&name));
    if let Some(dir) = sets_dir() {
        dialog = dialog.set_directory(dir);
    }
    let Some(mut file) = dialog.save_file() else {
        return;
    };
    if !is_set_file(&file) {
        let mut text = file.into_os_string();
        text.push(SUFFIX);
        file = text.into();
    }
    match DIALOG.with_borrow(|m| write(m, &file)) {
        Ok(()) => {
            close(ui);
            open(ui, file);
        }
        Err(e) => push(ui, &e),
    }
}

fn open(ui: &AppWindow, file: PathBuf) {
    if let Some(f) = OPEN.get() {
        f(ui, file);
    }
}

fn close(ui: &AppWindow) {
    ui.global::<ProjectSetData>().set_dialog(false);
}

/// File → Open project set…: pick a set file and open it as a document.
fn open_set(ui: &AppWindow) {
    let d = ui.global::<ProjectSetData>();
    let mut dialog = rfd::FileDialog::new()
        .set_title(d.get_open_title().as_str())
        .add_filter(d.get_filter_name().as_str(), &["toml"]);
    if let Some(dir) = sets_dir() {
        dialog = dialog.set_directory(dir);
    }
    let Some(file) = dialog.pick_file() else {
        return;
    };
    if is_set_file(&file) {
        open(ui, file);
    } else {
        ui.set_notice_kind(2);
        ui.set_notice(d.get_not_a_set());
    }
}

/// A callback without arguments that runs `f` on the window.
fn plain(ui: &AppWindow, f: fn(&AppWindow)) -> impl Fn() + 'static {
    let run = with_ui(ui, move |ui, ()| f(ui));
    move || run(())
}

pub fn wire(ui: &AppWindow, opener: Opener) {
    OPEN.set(Some(opener));
    let d = ui.global::<ProjectSetData>();
    d.on_show(plain(ui, show));
    d.on_open_set(plain(ui, open_set));
    d.on_add(plain(ui, add));
    d.on_save(plain(ui, save));
    d.on_close(plain(ui, close));
    d.on_name_edited(with_ui(ui, |ui, t: SharedString| {
        edit(ui, |m| m.name = t.to_string())
    }));
    d.on_remove(with_ui(ui, |ui, i: i32| {
        if let Some(i) = index(i) {
            edit(ui, |m| m.remove(i));
        }
    }));
    d.on_up(with_ui(ui, |ui, i: i32| {
        if let Some(i) = index(i).filter(|&i| i > 0) {
            edit(ui, |m| m.up(i));
            ui.global::<ProjectSetData>().set_selected(i as i32 - 1);
        }
    }));
}

/// `ARCHGUARD_DESKTOP_SET_DIALOG=<folder1>,<folder2>[,…]`: File → Compose
/// projects… filled with these folders and the name `ARCHGUARD_DESKTOP_SET_NAME`
/// (default `search`). Logs the dialog's state.
#[cfg(feature = "snapshot")]
pub fn snapshot_hook(ui: &AppWindow) {
    let Ok(spec) = std::env::var("ARCHGUARD_DESKTOP_SET_DIALOG") else {
        return;
    };
    show(ui);
    let name = std::env::var("ARCHGUARD_DESKTOP_SET_NAME").unwrap_or_else(|_| "search".into());
    let d = ui.global::<ProjectSetData>();
    d.set_name(name.as_str().into());
    edit(ui, |m| {
        m.name = name;
        for dir in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            m.add(Path::new(dir).canonicalize().unwrap_or_else(|_| dir.into()));
        }
    });
    eprintln!(
        "project set dialog: reason {}, folders {}",
        d.get_reason(),
        DIALOG.with_borrow(|m| m.folders.len())
    );
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use archguard_engine::{ProjectSet, read_project_set, write_project_set};

    use super::{Block, Dialog, file_name, folder_name, relative, write};
    use crate::documents_set::{Target, target};

    fn dir(root: &Path, name: &str) -> PathBuf {
        let d = root.join(name);
        std::fs::create_dir_all(&d).expect("dir");
        d.canonicalize().expect("canonical")
    }

    fn two() -> Dialog {
        let mut d = Dialog::new();
        d.name = "search".into();
        d.add("/p/a".into());
        d.add("/p/b".into());
        d
    }

    #[test]
    fn adding_skips_duplicates_and_keeps_order() {
        let mut d = Dialog::new();
        assert!(d.add("/p/a".into()));
        assert!(d.add("/p/b".into()));
        assert!(!d.add("/p/a".into()));
        assert_eq!(d.folders, [PathBuf::from("/p/a"), PathBuf::from("/p/b")]);
    }

    #[test]
    fn remove_and_up_move_the_primary_project() {
        let mut d = two();
        d.add("/p/c".into());
        d.up(2);
        assert_eq!(d.folders[1], PathBuf::from("/p/c"));
        d.up(1);
        assert_eq!(d.folders[0], PathBuf::from("/p/c"), "up from 1 = primary");
        d.up(0);
        assert_eq!(d.folders[0], PathBuf::from("/p/c"), "the first stays");
        d.remove(0);
        assert_eq!(d.folders, [PathBuf::from("/p/a"), PathBuf::from("/p/b")]);
        d.remove(9);
        assert_eq!(d.folders.len(), 2, "out of range is ignored");
    }

    #[test]
    fn saving_is_blocked_with_a_reason() {
        let mut d = Dialog::new();
        assert_eq!(d.block(), Some(Block::Name));
        d.name = "  ".into();
        assert_eq!(d.block(), Some(Block::Name));
        d.name = "search".into();
        assert_eq!(d.block(), Some(Block::TooFew));
        d.add("/p/a".into());
        assert_eq!(d.block(), Some(Block::TooFew));
        d.add("/p/b".into());
        assert_eq!(d.block(), None);
        d.folders.push("/p/a".into());
        assert_eq!(d.block(), Some(Block::Duplicate));
        d.folders = (0..17).map(|i| PathBuf::from(format!("/p/{i}"))).collect();
        assert_eq!(d.block(), Some(Block::TooMany));
        d.folders.truncate(16);
        assert_eq!(d.block(), None);
        d.name = "x".repeat(201);
        assert_eq!(d.block(), Some(Block::Name));
    }

    #[test]
    fn the_engine_checks_the_folders() {
        let t = tempfile::tempdir().expect("temp");
        let root = t.path().canonicalize().expect("root");
        let mut d = Dialog::new();
        d.add(dir(&root, "a"));
        assert_eq!(d.folder_error(), None);
        d.add(root.join("gone"));
        assert_eq!(
            d.folder_error().as_deref(),
            Some("project 2 does not exist")
        );
        d.folders[1] = root.join("a/../a");
        assert_eq!(
            d.folder_error().as_deref(),
            Some("project 2 is a duplicate")
        );
    }

    #[test]
    fn a_set_prefills_the_dialog() {
        let d = Dialog::from_set("search".into(), vec!["/p/a".into(), "/p/b".into()]);
        assert_eq!((d.name.as_str(), d.folders.len()), ("search", 2));
        assert_eq!(d.block(), None);
    }

    #[test]
    fn paths_are_relative_when_close_and_absolute_when_far() {
        let base = Path::new("/r/repo/sets");
        assert_eq!(
            relative(base, Path::new("/r/repo/infra")),
            Path::new("../infra")
        );
        assert_eq!(relative(base, Path::new("/r/repo/sets/x")), Path::new("x"));
        assert_eq!(
            relative(base, Path::new("/r/repo/a/b/c")),
            Path::new("../a/b/c")
        );
        let far = Path::new("/home/me/.config/archguard/sets");
        assert_eq!(
            relative(far, Path::new("/home/me/p/a")),
            Path::new("/home/me/p/a")
        );
    }

    #[test]
    fn a_row_leads_with_the_folder_name() {
        assert_eq!(folder_name(Path::new("/p/deep/infra")), "infra");
        assert_eq!(folder_name(Path::new("/p/deep/infra/")), "infra");
        assert_eq!(folder_name(Path::new("/")), "/");
    }

    #[test]
    fn the_file_name_is_the_set_name() {
        assert_eq!(file_name("Search"), "Search.archguard-set.toml");
        assert_eq!(file_name("a/b:c"), "a_b_c.archguard-set.toml");
    }

    #[test]
    fn saving_writes_what_write_project_set_writes_with_relative_paths() {
        let t = tempfile::tempdir().expect("temp");
        let root = t.path().canonicalize().expect("root");
        let (a, b) = (dir(&root, "a"), dir(&root, "b"));
        let sets = dir(&root, "sets");
        let mut d = Dialog::new();
        d.name = "search".into();
        d.add(a.clone());
        d.add(b.clone());
        let file = sets.join(file_name(&d.name));
        write(&d, &file).expect("saved");
        let expected = sets.join("expected.archguard-set.toml");
        let set = ProjectSet {
            name: "search".into(),
            projects: vec!["../a".into(), "../b".into()],
        };
        write_project_set(&expected, &set).expect("expected");
        assert_eq!(
            std::fs::read_to_string(&file).expect("file"),
            std::fs::read_to_string(&expected).expect("expected text")
        );
        assert_eq!(read_project_set(&file).expect("read").projects, [a, b]);
    }

    #[test]
    fn far_folders_are_written_absolute() {
        let t = tempfile::tempdir().expect("temp");
        let root = t.path().canonicalize().expect("root");
        let (a, b) = (dir(&root, "a"), dir(&root, "b"));
        let sets = dir(&root, "x/y/z/sets");
        let mut d = Dialog::new();
        d.name = "far".into();
        d.add(a.clone());
        d.add(b);
        let file = sets.join(file_name("far"));
        write(&d, &file).expect("saved");
        let text = std::fs::read_to_string(&file).expect("file");
        assert!(text.contains(a.to_str().expect("utf8")), "{text}");
    }

    #[test]
    fn a_saved_set_opens_as_a_document() {
        let t = tempfile::tempdir().expect("temp");
        let root = t.path().canonicalize().expect("root");
        let (a, b) = (dir(&root, "a"), dir(&root, "b"));
        let mut d = Dialog::new();
        d.name = "search".into();
        d.add(a.clone());
        d.add(b);
        let file = root.join(file_name(&d.name));
        write(&d, &file).expect("saved");
        let Ok(Target::Set { primary, name, .. }) = target(&file) else {
            panic!("a set");
        };
        assert_eq!((primary, name.as_str()), (a, "search"));
    }

    #[test]
    fn save_errors_name_the_problem_without_file_contents() {
        let t = tempfile::tempdir().expect("temp");
        let root = t.path().canonicalize().expect("root");
        let a = dir(&root, "a");
        let mut d = Dialog::new();
        d.name = "search".into();
        d.add(a);
        d.add(root.join("gone"));
        let e = write(&d, &root.join(file_name("search"))).expect_err("missing folder");
        assert!(e.contains("project 2 does not exist"), "{e}");
        assert!(!root.join(file_name("search")).exists());
    }
}
