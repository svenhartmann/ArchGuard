//! The active document's panes in the window: view bar, view tabs,
//! "Split right", the divider and linked selection. The model is
//! [`panes`]; each document keeps its [`Layout`] in [`documents`], and
//! every change is stored as the project's layout in `desktop.toml`.

use std::cell::RefCell;
use std::rc::Rc;

use slint::{Model, SharedString, VecModel};

use crate::documents::{self, with_store};
use crate::panes::{Layout, link};
use crate::settings_file;
use crate::tab_support::{model, with_ui};
use crate::ui::{AppWindow, PaneItem};

fn index(i: i32) -> usize {
    crate::tab_support::index(i).unwrap_or(usize::MAX)
}

/// How a view reveals a tree node; registered by the view's module, so
/// this module depends on none of them.
type Reveal = Rc<dyn Fn(&AppWindow, usize)>;

thread_local! {
    static REVEALS: RefCell<Vec<(&'static str, Reveal)>> = const { RefCell::new(Vec::new()) };
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
        change(ui, |l| l.open_tab(&v));
    }));
    let split = with_ui(ui, |ui, ()| change(ui, |l| l.split_right(None)));
    ui.on_split_right(move || split(()));
    ui.on_split_view(with_ui(ui, |ui, v: SharedString| {
        change(ui, |l| l.split_right(Some(&v)));
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
        (*l != before).then(|| l.clone())
    })
    .flatten();
    push(ui);
    if let Some(l) = changed {
        store(&l);
    }
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

/// Show the active document's layout.
pub fn push(ui: &AppWindow) {
    let l = layout();
    let rows: Vec<PaneItem> = l
        .panes
        .iter()
        .enumerate()
        .map(|(i, p)| PaneItem {
            view: p.tabs[p.active].as_str().into(),
            tabs: model(p.tabs.iter().map(|t| SharedString::from(t.as_str()))),
            active: p.active as i32,
            focused: i == l.focus,
        })
        .collect();
    let panes = PANES.with_borrow_mut(|m| {
        m.get_or_insert_with(|| Rc::new(VecModel::default()))
            .clone()
    });
    sync(&panes, rows);
    ui.set_panes(panes.into());
    ui.set_tab(l.focused().into());
    ui.set_focused_pane(l.focus as i32);
    ui.set_pane_ratio(l.ratio);
}

/// Make `model` hold `rows`, touching only rows that differ.
fn sync(model: &VecModel<PaneItem>, rows: Vec<PaneItem>) {
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
        && a.active == b.active
        && a.focused == b.focused
        && a.tabs.iter().eq(b.tabs.iter())
}

/// `view` reveals a tree node with `f` (at start-up, for the views of
/// [`link::reveals`]).
pub fn on_reveal(view: &'static str, f: impl Fn(&AppWindow, usize) + 'static) {
    REVEALS.with_borrow_mut(|r| r.push((view, Rc::new(f))));
}

/// Linked selection: `node` (a tree node) was selected in `from`; reveal
/// it in the view beside, once (see [`link`]).
pub fn link_from(ui: &AppWindow, from: &str, node: usize) {
    let l = layout();
    link::follow(l.linked(from), |target| {
        let f =
            REVEALS.with_borrow(|r| r.iter().find(|(v, _)| *v == target).map(|(_, f)| f.clone()));
        if let Some(f) = f {
            f(ui, node);
        }
    });
}

/// Show `node` (an index into the analysed graph's nodes) in the Graph
/// view: in the pane that shows it, else in the focused pane.
pub fn reveal(ui: &AppWindow, node: usize) {
    change(ui, |l| l.navigate("graph"));
    ui.invoke_reveal(node as i32);
}
