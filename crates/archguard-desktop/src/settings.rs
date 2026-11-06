//! Personal desktop settings (`desktop.toml`, ADR 0002 decision 7).
//! Independent of Slint; `settings_file.rs` keeps the loaded file.
//!
//! ```toml
//! theme = "dark"            # "dark" | "light"; absent: follow the system
//! language = "de"           # "de" | "en"; absent: follow the system
//! recent = ["/work/shop"]   # most recent first, at most 10
//!
//! [window]
//! width = 1480              # logical pixels
//! height = 920
//!
//! [session]                 # the documents open at the last quit
//! documents = ["/work/shop", "/work/api"]
//! active = 1
//!
//! [projects."/work/shop"]   # per project, keyed by its canonical path
//! tab = "graph"
//! display = 0
//! explorer_tab = 0
//! panes = [["graph"], ["dsm"]]  # optional: the pane layout (pane_store.rs)
//! ```
//!
//! The last view of a project lives under `[projects]` only; the session
//! lists paths. Writing keeps unknown keys and comments (`toml_edit`).
//! Values of the wrong type read as absent instead of failing the file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use toml_edit::{Array, DocumentMut, Item, Table, value};

use crate::panes::Layout;

/// How many projects "Open recent" lists.
pub const RECENT_MAX: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    System,
    Dark,
    Light,
}

/// The last view of one project.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProjectView {
    /// The focused pane's view.
    pub tab: String,
    pub display: i32,
    pub explorer_tab: i32,
    /// Panes and view tabs; `None` in files written before panes existed.
    pub layout: Option<Layout>,
}

/// The documents open at the last quit, in tab order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Session {
    pub documents: Vec<PathBuf>,
    pub active: usize,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Settings {
    pub theme: Theme,
    /// Empty: follow the system.
    pub language: String,
    /// Logical width and height.
    pub window: Option<(u32, u32)>,
    pub recent: Vec<PathBuf>,
    /// `None` until a session was recorded once.
    pub session: Option<Session>,
    pub projects: BTreeMap<String, ProjectView>,
}

impl Settings {
    /// Put `root` first in the recent list, without duplicates.
    pub fn add_recent(&mut self, root: &Path) {
        self.recent.retain(|p| p != root);
        self.recent.insert(0, root.to_path_buf());
        self.recent.truncate(RECENT_MAX);
    }

    pub fn project(&self, root: &Path) -> Option<&ProjectView> {
        self.projects.get(&key(root))
    }

    pub fn set_project(&mut self, root: &Path, view: ProjectView) {
        self.projects.insert(key(root), view);
    }
}

/// How "Open recent" shows `root`: its folder name and the folder that
/// holds it, with the home directory as `~`.
pub fn recent_label(root: &Path, home: Option<&Path>) -> (String, String) {
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.display().to_string());
    let parent = root.parent().unwrap_or(root);
    let place = match home.and_then(|h| parent.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Some(rest) => format!("~/{}", rest.display()),
        None => parent.display().to_string(),
    };
    (name, place)
}

fn key(root: &Path) -> String {
    root.to_string_lossy().into_owned()
}

/// The documents to open at start and the tab to activate. Arguments win;
/// without them the last session's projects that still exist; without a
/// session the current directory, unless it is the file system root (an
/// app started from the Finder or a desktop launcher).
pub fn startup_roots(
    args: Vec<PathBuf>,
    settings: &Settings,
    cwd: Option<PathBuf>,
    exists: impl Fn(&Path) -> bool,
) -> (Vec<PathBuf>, usize) {
    if !args.is_empty() {
        return (args, 0);
    }
    let Some(session) = &settings.session else {
        let cwd = cwd.filter(|d| d.parent().is_some());
        return (cwd.into_iter().collect(), 0);
    };
    let mut active = 0;
    let mut roots = Vec::new();
    for (i, root) in session.documents.iter().enumerate() {
        if !exists(root) {
            continue;
        }
        if i <= session.active {
            active = roots.len();
        }
        roots.push(root.clone());
    }
    (roots, active)
}

/// Read `text`; an error names the first problem.
pub fn parse(text: &str) -> Result<(DocumentMut, Settings), String> {
    let doc: DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| e.to_string())?;
    let settings = read(&doc);
    Ok((doc, settings))
}

fn read(doc: &DocumentMut) -> Settings {
    let theme = match doc.get("theme").and_then(Item::as_str) {
        Some("dark") => Theme::Dark,
        Some("light") => Theme::Light,
        _ => Theme::System,
    };
    let window = doc.get("window").and_then(|w| {
        let side = |k: &str| u32::try_from(w.get(k)?.as_integer()?).ok();
        Some((side("width")?, side("height")?))
    });
    Settings {
        theme,
        language: doc
            .get("language")
            .and_then(Item::as_str)
            .unwrap_or("")
            .to_string(),
        window,
        recent: paths(doc.get("recent")),
        session: doc.get("session").map(|s| Session {
            documents: paths(s.get("documents")),
            active: s
                .get("active")
                .and_then(Item::as_integer)
                .and_then(|a| usize::try_from(a).ok())
                .unwrap_or(0),
        }),
        projects: read_projects(doc.get("projects")),
    }
}

fn paths(item: Option<&Item>) -> Vec<PathBuf> {
    let Some(array) = item.and_then(Item::as_array) else {
        return Vec::new();
    };
    array
        .iter()
        .filter_map(|v| v.as_str())
        .map(PathBuf::from)
        .collect()
}

fn read_projects(item: Option<&Item>) -> BTreeMap<String, ProjectView> {
    let Some(table) = item.and_then(Item::as_table_like) else {
        return BTreeMap::new();
    };
    let int = |t: &dyn toml_edit::TableLike, k: &str| {
        t.get(k)
            .and_then(Item::as_integer)
            .and_then(|v| i32::try_from(v).ok())
            .unwrap_or(0)
    };
    table
        .iter()
        .filter_map(|(path, item)| {
            let t = item.as_table_like()?;
            let view = ProjectView {
                tab: t
                    .get("tab")
                    .and_then(Item::as_str)
                    .unwrap_or("")
                    .to_string(),
                display: int(t, "display"),
                explorer_tab: int(t, "explorer_tab"),
                layout: crate::pane_store::read(t),
            };
            Some((path.to_string(), view))
        })
        .collect()
}

/// Write `settings` into `doc` and return the file text. Keys and
/// comments ArchGuard does not know stay as they were.
pub fn render(doc: &mut DocumentMut, settings: &Settings) -> String {
    match settings.theme {
        Theme::System => remove(doc.as_table_mut(), "theme"),
        Theme::Dark => set(doc.as_table_mut(), "theme", value("dark")),
        Theme::Light => set(doc.as_table_mut(), "theme", value("light")),
    }
    if settings.language.is_empty() {
        remove(doc.as_table_mut(), "language");
    } else {
        set(
            doc.as_table_mut(),
            "language",
            value(settings.language.as_str()),
        );
    }
    set(
        doc.as_table_mut(),
        "recent",
        value(path_array(&settings.recent)),
    );
    if let Some((w, h)) = settings.window {
        let window = table(doc, "window");
        set(window, "width", value(i64::from(w)));
        set(window, "height", value(i64::from(h)));
    }
    if let Some(session) = &settings.session {
        let t = table(doc, "session");
        set(t, "documents", value(path_array(&session.documents)));
        set(t, "active", value(session.active as i64));
    }
    render_projects(doc, &settings.projects);
    doc.to_string()
}

fn render_projects(doc: &mut DocumentMut, projects: &BTreeMap<String, ProjectView>) {
    if projects.is_empty() {
        return;
    }
    let all = table(doc, "projects");
    all.set_implicit(true);
    for (path, view) in projects {
        let entry = all.entry(path).or_insert_with(|| Item::Table(Table::new()));
        if !entry.is_table() {
            *entry = Item::Table(Table::new());
        }
        let t = entry.as_table_mut().expect("just made a table");
        set(t, "tab", value(view.tab.as_str()));
        set(t, "display", value(i64::from(view.display)));
        set(t, "explorer_tab", value(i64::from(view.explorer_tab)));
        for (k, item) in view.layout.iter().flat_map(crate::pane_store::items) {
            set(t, k, item);
        }
    }
}

/// The table `name` at the top level; a value of another type there is
/// replaced.
fn table<'a>(doc: &'a mut DocumentMut, name: &str) -> &'a mut Table {
    let item = doc.entry(name).or_insert_with(|| Item::Table(Table::new()));
    if !item.is_table() {
        *item = Item::Table(Table::new());
    }
    item.as_table_mut().expect("just made a table")
}

/// Replace a value but keep the comments around it.
fn set(t: &mut Table, k: &str, mut new: Item) {
    let Some(old) = t.get_mut(k) else {
        t.insert(k, new);
        return;
    };
    if let (Some(o), Some(v)) = (old.as_value(), new.as_value_mut()) {
        *v.decor_mut() = o.decor().clone();
    }
    // Replacing the item in place keeps the key and the comment above it.
    *old = new;
}

fn remove(t: &mut Table, k: &str) {
    t.remove(k);
}

fn path_array(paths: &[PathBuf]) -> Array {
    paths
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Settings {
        let mut s = Settings {
            theme: Theme::Light,
            language: "de".into(),
            window: Some((1200, 800)),
            session: Some(Session {
                documents: vec!["/w/shop".into(), "/w/api".into()],
                active: 1,
            }),
            ..Settings::default()
        };
        s.add_recent(Path::new("/w/api"));
        s.add_recent(Path::new("/w/shop"));
        s.set_project(
            Path::new("/w/shop"),
            ProjectView {
                tab: "dsm".into(),
                display: 1,
                explorer_tab: 2,
                layout: Some(split_layout()),
            },
        );
        s
    }

    fn split_layout() -> Layout {
        let mut l = Layout::single("graph");
        l.open_tab("quality", None);
        l.split_right(Some("dsm"), None);
        l.set_ratio(0.62);
        l.focus(1);
        l
    }

    #[test]
    fn the_pane_layout_is_stored_per_project_and_keeps_comments() {
        let text = "[projects.\"/w/shop\"]\ntab = \"graph\"\n# my split\npanes = [[\"graph\"]]\nnote = \"mine\"\n";
        let (mut doc, mut s) = parse(text).expect("valid");
        assert_eq!(
            s.projects["/w/shop"].layout,
            Some(Layout::single("graph")),
            "read"
        );
        s.projects.get_mut("/w/shop").expect("project").layout = Some(split_layout());
        let out = render(&mut doc, &s);
        for kept in ["# my split", "note = \"mine\""] {
            assert!(out.contains(kept), "{kept} lost:\n{out}");
        }
        assert!(
            out.contains("panes = [[\"graph\", \"quality\"], [\"dsm\"]]"),
            "{out}"
        );
        let (_, back) = parse(&out).expect("own output parses");
        assert_eq!(back.projects["/w/shop"].layout, Some(split_layout()));
    }

    /// Two Graphs on different packages, one at another display, and a
    /// File view are stored with their nodes and read back as they were.
    #[test]
    fn view_instances_are_stored_with_their_nodes() {
        let mut l = Layout::single("graph");
        let left = l.open_tab("graph", Some("crates/a")).expect("tab");
        l.tab_mut(left).expect("tab").display = 1;
        l.split_right(Some("file"), Some("crates/b/src/lib.rs"));
        let mut s = Settings::default();
        s.set_project(
            Path::new("/w/shop"),
            ProjectView {
                tab: "file".into(),
                layout: Some(l.clone()),
                ..ProjectView::default()
            },
        );
        let text = render(&mut DocumentMut::new(), &s);
        assert!(
            text.contains(
                "panes = [[\"graph\", { view = \"graph\", node = \"crates/a\", display = 1 }], \
                 [{ view = \"file\", node = \"crates/b/src/lib.rs\" }]]"
            ),
            "{text}"
        );
        let (_, back) = parse(&text).expect("own output parses");
        assert_eq!(back.projects["/w/shop"].layout, Some(l));
    }

    #[test]
    fn a_project_without_panes_has_no_layout() {
        let (_, s) = parse("[projects.\"/w/shop\"]\ntab = \"dsm\"\n").expect("valid");
        assert_eq!(s.projects["/w/shop"].layout, None);
    }

    #[test]
    fn an_empty_file_is_the_default() {
        let (_, s) = parse("").expect("empty toml");
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn rendered_settings_read_back_equal() {
        let s = sample();
        let mut doc = DocumentMut::new();
        let text = render(&mut doc, &s);
        let (_, back) = parse(&text).expect("own output parses");
        assert_eq!(back, s, "{text}");
        assert!(text.contains("[projects.\"/w/shop\"]"), "{text}");
    }

    #[test]
    fn writing_keeps_unknown_keys_and_comments() {
        let text = "# my settings\ntheme = \"dark\" # chosen in the menu\nfuture = 1\n\n[window]\nwidth = 10\nheight = 10\nmaximized = true\n\n[projects.\"/w/shop\"]\ntab = \"start\"\npinned = [\"a\"]\n";
        let (mut doc, mut s) = parse(text).expect("valid");
        assert_eq!(s.theme, Theme::Dark);
        s.theme = Theme::Light;
        s.window = Some((1300, 900));
        s.projects.get_mut("/w/shop").expect("project").tab = "graph".into();
        let out = render(&mut doc, &s);
        for kept in [
            "# my settings",
            "# chosen in the menu",
            "future = 1",
            "maximized = true",
            "pinned = [\"a\"]",
        ] {
            assert!(out.contains(kept), "{kept} lost:\n{out}");
        }
        assert!(out.contains("theme = \"light\""), "{out}");
        assert!(out.contains("width = 1300"), "{out}");
        assert!(out.contains("tab = \"graph\""), "{out}");
    }

    #[test]
    fn system_theme_and_language_remove_their_keys() {
        let (mut doc, mut s) = parse("theme = \"dark\"\nlanguage = \"de\"\n").expect("valid");
        s.theme = Theme::System;
        s.language.clear();
        let out = render(&mut doc, &s);
        assert!(!out.contains("theme") && !out.contains("language"), "{out}");
    }

    #[test]
    fn values_of_the_wrong_type_read_as_absent() {
        let text = "theme = 3\nrecent = \"no list\"\n[window]\nwidth = -5\nheight = 7\n[session]\ndocuments = [\"/a\", 4]\nactive = \"x\"\n[projects]\n\"/a\" = 1\n\"/b\" = { tab = 2, display = 1 }\n";
        let (mut doc, s) = parse(text).expect("valid toml");
        assert_eq!(s.theme, Theme::System);
        assert!(s.recent.is_empty());
        assert_eq!(s.window, None);
        assert_eq!(
            s.session,
            Some(Session {
                documents: vec!["/a".into()],
                active: 0
            })
        );
        assert_eq!(s.projects.len(), 1);
        assert_eq!(s.projects["/b"].display, 1);
        // Writing over wrong types does not panic and repairs them.
        let mut s = s;
        s.set_project(Path::new("/a"), ProjectView::default());
        let (_, back) = parse(&render(&mut doc, &s)).expect("repaired");
        assert_eq!(back.projects.len(), 2);
    }

    #[test]
    fn a_syntax_error_is_reported() {
        assert!(parse("theme = \n[[").is_err());
    }

    #[test]
    fn recent_is_most_recent_first_unique_and_capped() {
        let mut s = Settings::default();
        for i in 0..12 {
            s.add_recent(Path::new(&format!("/p{i}")));
        }
        s.add_recent(Path::new("/p5"));
        assert_eq!(s.recent.len(), RECENT_MAX);
        assert_eq!(s.recent[0], Path::new("/p5"));
        assert_eq!(s.recent[1], Path::new("/p11"));
        assert_eq!(
            s.recent.iter().filter(|p| *p == Path::new("/p5")).count(),
            1
        );
        assert!(!s.recent.contains(&PathBuf::from("/p0")));
    }

    #[test]
    fn recent_labels_shorten_the_home_directory() {
        let home = Some(Path::new("/Users/me"));
        let label = |p: &str| recent_label(Path::new(p), home);
        assert_eq!(label("/Users/me/w/shop"), ("shop".into(), "~/w".into()));
        assert_eq!(label("/Users/me/shop"), ("shop".into(), "~".into()));
        assert_eq!(label("/srv/api"), ("api".into(), "/srv".into()));
        assert_eq!(recent_label(Path::new("/srv/api"), None).1, "/srv");
    }

    #[test]
    fn arguments_win_over_the_session() {
        let s = sample();
        let (roots, active) = startup_roots(vec!["/x".into()], &s, None, |_| true);
        assert_eq!((roots, active), (vec![PathBuf::from("/x")], 0));
    }

    #[test]
    fn the_session_reopens_existing_projects_and_its_active_one() {
        let mut s = sample();
        s.session = Some(Session {
            documents: vec!["/gone".into(), "/a".into(), "/b".into(), "/c".into()],
            active: 2,
        });
        let (roots, active) = startup_roots(Vec::new(), &s, None, |p| p != Path::new("/gone"));
        assert_eq!(roots, [Path::new("/a"), Path::new("/b"), Path::new("/c")]);
        assert_eq!(active, 1, "/b stays active");
        // The active one is gone: its left neighbour becomes active.
        let (_, active) = startup_roots(Vec::new(), &s, None, |p| p != Path::new("/b"));
        assert_eq!(active, 1, "/a");
    }

    #[test]
    fn an_empty_session_opens_nothing() {
        let s = Settings {
            session: Some(Session::default()),
            ..Settings::default()
        };
        let (roots, _) = startup_roots(Vec::new(), &s, Some("/w".into()), |_| true);
        assert!(roots.is_empty());
    }

    #[test]
    fn without_a_session_the_current_directory_opens_unless_it_is_the_root() {
        let s = Settings::default();
        let (roots, _) = startup_roots(Vec::new(), &s, Some("/w/shop".into()), |_| true);
        assert_eq!(roots, [Path::new("/w/shop")]);
        let (roots, _) = startup_roots(Vec::new(), &s, Some("/".into()), |_| true);
        assert!(roots.is_empty());
    }
}
