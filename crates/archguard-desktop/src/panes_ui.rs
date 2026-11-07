//! The active document's panes in the window: view bar, view tabs,
//! "Split right", the divider and linked selection. The model is
//! [`panes`]; each document keeps its [`Layout`] in [`documents`], and
//! every change is stored as the project's layout in `desktop.toml`.
//!
//! The views that can be open several times register a sync with
//! [`on_sync`]; it runs after every layout change and builds, drops and
//! shows their instances (`instances::sync`); then [`on_change`] sees the
//! layout before and after.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{Model, SharedString, VecModel};

use crate::documents::{self, with_store};
use crate::panes::{self, Layout, Tab, link};
use crate::settings_file;
use crate::tab_support::{model, with_ui};
use crate::ui::{AppWindow, PaneItem, TabItem};

fn index(i: i32) -> usize {
    crate::tab_support::index(i).unwrap_or(usize::MAX)
}

/// How a view instance reveals a tree node: `(ui, tab id, node)`;
/// registered by the view's module, so this module depends on none of
/// them.
type Reveal = Rc<dyn Fn(&AppWindow, u32, usize)>;

/// Builds, drops and shows a kind's instances for a layout; returns the
/// tabs whose node the analysis lacks.
type SyncFn = Rc<dyn Fn(&AppWindow, &Layout) -> Vec<u32>>;

/// Called after a layout change with the layout before and after.
type Changed = Rc<dyn Fn(&AppWindow, &Layout, &Layout)>;

thread_local! {
    static REVEALS: RefCell<Vec<(&'static str, Reveal)>> = const { RefCell::new(Vec::new()) };
    static SYNCS: RefCell<Vec<(&'static str, SyncFn)>> = const { RefCell::new(Vec::new()) };
    static CHANGED: RefCell<Option<Changed>> = const { RefCell::new(None) };
    /// The window's pane list. Rows are updated in place, so a pane whose
    /// view stays keeps its Slint view instance (scroll position, zoom).
    static PANES: RefCell<Option<Rc<VecModel<PaneItem>>>> = const { RefCell::new(None) };
}

/// Register the pane callbacks once at start-up.
pub fn wire(ui: &AppWindow) {
    ui.on_navigate(with_ui(ui, |ui, v: SharedString| {
        change(ui, |l| l.navigate(&v));
    }));
    ui.on_open_view_tab(with_ui(ui, |ui, v: SharedString| {
        change(ui, |l| {
            l.open_tab(&v, None);
        });
    }));
    let split = with_ui(ui, |ui, ()| {
        change(ui, |l| {
            l.split_right(None, None);
        })
    });
    ui.on_split_right(move || split(()));
    ui.on_split_view(with_ui(ui, |ui, v: SharedString| {
        change(ui, |l| {
            l.split_right(Some(&v), None);
        });
    }));
    ui.on_close_pane(with_ui(ui, |ui, p: i32| {
        change(ui, |l| l.close_pane(index(p)));
    }));
    let f = with_ui(ui, |ui, (p, t): (i32, i32)| {
        change(ui, |l| l.select_tab(index(p), index(t)));
    });
    ui.on_select_view_tab(move |p, t| f((p, t)));
    let f = with_ui(ui, |ui, (p, t): (i32, i32)| {
        change(ui, |l| l.close_tab(index(p), index(t)));
    });
    ui.on_close_view_tab(move |p, t| f((p, t)));
    ui.on_focus_pane(with_ui(ui, |ui, p: i32| {
        change(ui, |l| l.focus(index(p)));
    }));
    ui.on_set_pane_ratio(with_ui(ui, |ui, r: f32| change(ui, |l| l.set_ratio(r))));
}

/// The active document's layout (a single Graph pane until it has one).
pub fn layout() -> Layout {
    documents::with_active_state_or(Layout::default, |l: &mut Layout| l.clone()).unwrap_or_default()
}

/// Change the active document's layout, show it and store it if it
/// changed.
pub fn change(ui: &AppWindow, f: impl FnOnce(&mut Layout)) {
    let changed = documents::with_active_state_or(Layout::default, |l: &mut Layout| {
        let before = l.clone();
        f(l);
        (*l != before).then(|| (before, l.clone()))
    })
    .flatten();
    push(ui);
    if let Some((before, l)) = changed {
        sync(ui);
        store(&l);
        if let Some(f) = CHANGED.with_borrow(|c| c.clone()) {
            f(ui, &before, &l);
        }
    }
}

/// After a layout change, once the views' instances exist, `f` gets the
/// layout before and after ("Follow left", at start-up).
pub fn on_change(f: impl Fn(&AppWindow, &Layout, &Layout) + 'static) {
    CHANGED.set(Some(Rc::new(f)));
}

/// The project's last view in the settings follows the layout.
fn store(layout: &Layout) {
    let Some(root) = with_store(|s| s.active().map(|d| d.root.clone())) else {
        return;
    };
    settings_file::update(|s| {
        let mut view = s.project(&root).cloned().unwrap_or_default();
        view.tab = layout.focused().to_string();
        view.layout = Some(layout.clone());
        s.set_project(&root, view);
    });
}

fn tab_item(t: &Tab) -> TabItem {
    TabItem {
        kind: t.kind.as_str().into(),
        id: t.id as i32,
        label: t.label().into(),
    }
}

/// Show the active document's layout.
pub fn push(ui: &AppWindow) {
    let l = layout();
    let rows: Vec<PaneItem> = l
        .panes
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let shown = &p.tabs[p.active];
            PaneItem {
                view: shown.kind.as_str().into(),
                id: shown.id as i32,
                tabs: model(p.tabs.iter().map(tab_item)),
                active: p.active as i32,
                focused: i == l.focus,
            }
        })
        .collect();
    let panes = PANES.with_borrow_mut(|m| {
        m.get_or_insert_with(|| Rc::new(VecModel::default()))
            .clone()
    });
    sync_rows(&panes, rows);
    ui.set_panes(panes.into());
    ui.set_tab(l.focused().into());
    ui.set_focused_pane(l.focus as i32);
    ui.set_pane_ratio(l.ratio);
    ui.set_follow(l.follow_left);
    ui.set_follow_blocked(crate::follow::blocked(&l));
}

/// Make `model` hold `rows`, touching only rows that differ.
fn sync_rows(model: &VecModel<PaneItem>, rows: Vec<PaneItem>) {
    while model.row_count() > rows.len() {
        model.remove(model.row_count() - 1);
    }
    for (i, row) in rows.into_iter().enumerate() {
        match model.row_data(i) {
            Some(old) if same(&old, &row) => {}
            Some(_) => model.set_row_data(i, row),
            None => model.push(row),
        }
    }
}

fn same(a: &PaneItem, b: &PaneItem) -> bool {
    a.view == b.view
        && a.id == b.id
        && a.active == b.active
        && a.focused == b.focused
        && a.tabs.iter().eq(b.tabs.iter())
}

/// `kind`'s instances follow the layout through `f` (at start-up, for the
/// kinds of `panes::multi`).
pub fn on_sync(kind: &'static str, f: impl Fn(&AppWindow, &Layout) -> Vec<u32> + 'static) {
    SYNCS.with_borrow_mut(|s| s.push((kind, Rc::new(f))));
}

/// Build, drop and show the active document's view instances. A tab
/// whose node the analysis lacks is dropped (a File view) or forgets its
/// node (Graph, Matrix).
pub fn sync(ui: &AppWindow) {
    let syncs: Vec<(&str, SyncFn)> = SYNCS.with_borrow(|s| s.clone());
    let l = layout();
    let gone: Vec<u32> = syncs.iter().flat_map(|(_, f)| f(ui, &l)).collect();
    if gone.is_empty() {
        return;
    }
    change(ui, |l| {
        for id in gone {
            match l.tab_mut(id) {
                Some(t) if t.kind == crate::panes::FILE => panes::drop_tab(l, id),
                Some(t) => t.node = None,
                None => {}
            }
        }
    });
}

/// Tab `id` is now about `node` (a Graph opened on a package, a Matrix
/// scope) at `display`: its title and stored layout follow.
pub fn describe(ui: &AppWindow, id: u32, node: Option<String>, display: i32) {
    change(ui, |l| {
        if let Some(t) = l.tab_mut(id) {
            t.node = node;
            t.display = display;
        }
    });
}

/// The id of a tab of `kind` (as the view bar would show it).
pub fn shown_id(kind: &str) -> Option<u32> {
    let l = layout();
    l.find(kind).map(|(p, t)| l.panes[p].tabs[t].id)
}

/// Open `kind` ("file" or "graph") on the tree node named `node` in a new
/// tab, or to the side. Returns the new tab's id.
pub fn open_node(ui: &AppWindow, kind: &str, node: &str, side: bool) -> Option<u32> {
    let mut id = None;
    change(ui, |l| {
        id = if side {
            l.split_right(Some(kind), Some(node))
        } else {
            l.open_tab(kind, Some(node))
        };
    });
    id
}

/// The view instance of tab `id` reveals tree nodes with `f` (at
/// start-up, for the views of [`link::reveals`]).
pub fn on_reveal(kind: &'static str, f: impl Fn(&AppWindow, u32, usize) + 'static) {
    REVEALS.with_borrow_mut(|r| r.push((kind, Rc::new(f))));
}

fn reveal_in(ui: &AppWindow, (kind, id): (&str, u32), node: usize) {
    let f = REVEALS.with_borrow(|r| r.iter().find(|(v, _)| *v == kind).map(|(_, f)| f.clone()));
    if let Some(f) = f {
        f(ui, id, node);
    }
}

/// Linked selection: `node` (a tree node) was selected in tab `from`;
/// reveal it in the view beside, once (see [`link`]).
pub fn link_from(ui: &AppWindow, from: u32, node: usize) {
    let l = layout();
    link::follow(l.linked(from).map(Tab::key), |kind, id| {
        reveal_in(ui, (kind, id), node);
    });
}

/// Show `node` (an index into the analysed graph's nodes) in a Graph:
/// the one the view bar would show, else a new one in the focused pane.
pub fn reveal(ui: &AppWindow, node: usize) {
    reveal_in_view(ui, "graph", node);
}

/// Show `node` in a view of `kind` that registered [`on_reveal`] (the
/// Graph, the Matrix), as [`reveal`] does.
pub fn reveal_in_view(ui: &AppWindow, kind: &str, node: usize) {
    change(ui, |l| l.navigate(kind));
    let l = layout();
    if let Some((p, t)) = l.find(kind) {
        reveal_in(ui, l.panes[p].tabs[t].key(), node);
    }
}
