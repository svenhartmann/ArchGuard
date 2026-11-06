//! File view: one tree node (a file or a package) in a tab of its own,
//! with its dependencies and their evidence ([`node_view`]). Opened from
//! the Graph and the Matrix ("Open in new tab", "Open to the side"); one
//! state per tab (`instances`).

use std::rc::Rc;

use slint::{SharedString, VecModel};

use crate::documents::{self, with_store};
use crate::follow::Nav;
use crate::follow_ui;
use crate::instances::{self, Instance};
use crate::node_view::{self, NodeView};
use crate::panes::{FILE, Layout};
use crate::panes_ui;
use crate::tab_support::{self, index, model, with_ui};
use crate::tree::Tree;
use crate::ui::{AppWindow, FileData, NodeDep};

pub struct FileState {
    tree: Rc<Tree>,
    view: NodeView,
}

thread_local! {
    /// Per pane: the data of the File view it shows.
    static PANES: Rc<VecModel<FileData>> = Rc::new(VecModel::default());
}

impl Instance for FileState {
    fn show(&mut self, _ui: &AppWindow, pane: usize, id: u32) {
        let row = data(&self.tree, &self.view, id);
        PANES.with(|m| instances::set_row(m, pane, row));
    }
}

fn deps(tree: &Tree, list: &[node_view::NodeDep]) -> Vec<NodeDep> {
    list.iter()
        .map(|d| NodeDep {
            node: d.node as i32,
            label: tree.nodes[d.node].label.as_str().into(),
            name: tree.nodes[d.node].name.as_str().into(),
            weight: d.weight.min(i32::MAX as u64) as i32,
            evidence: d.evidence.join("\n").into(),
        })
        .collect()
}

fn data(tree: &Tree, v: &NodeView, id: u32) -> FileData {
    let n = &tree.nodes[v.node];
    FileData {
        id: id as i32,
        node: v.node as i32,
        label: n.label.as_str().into(),
        name: n.name.as_str().into(),
        file: n.file,
        files: v.files as i32,
        packages: v.packages as i32,
        outgoing: model(deps(tree, &v.outgoing)),
        incoming: model(deps(tree, &v.incoming)),
    }
}

/// Build, drop and show the active document's File views for `layout`;
/// one whose node is gone gets no state (its tab is dropped).
fn sync(ui: &AppWindow, layout: &Layout) -> Vec<u32> {
    let Some(analysis) = with_store(|s| s.active()?.analysis.clone()) else {
        return Vec::new();
    };
    let tree = tab_support::tree(&analysis.loaded());
    instances::sync(ui, layout, FILE, &tree, |_, node| {
        Some(FileState {
            view: node_view::node_view(&tree, node?)?,
            tree: tree.clone(),
        })
    })
}

/// The name of tree node `node` in the active document.
fn name_of(node: usize) -> Option<String> {
    documents::with_active_state(|t: &mut Rc<Tree>| Some(t.nodes.get(node)?.name.clone())).flatten()
}

/// "Follow left": File view `id` shows the node named `name`; its tab
/// follows.
fn show_node(ui: &AppWindow, id: u32, name: &str) {
    let layout = panes_ui::layout();
    let shown = instances::update(ui, &layout, id, |s: &mut FileState| {
        let node = instances::node_named(&s.tree, name)?;
        s.view = node_view::node_view(&s.tree, node)?;
        Some(())
    })
    .flatten();
    if shown.is_some() {
        panes_ui::describe(ui, id, Some(name.to_string()), 0);
    }
}

pub fn wire(ui: &AppWindow) {
    ui.set_file_panes(PANES.with(|m| m.clone()).into());
    panes_ui::on_sync(FILE, sync);
    // "Open in new tab" / "Open to the side" on a node, in any view.
    let open = with_ui(ui, |ui, (kind, node, side): (SharedString, i32, bool)| {
        if let Some(name) = index(node).and_then(name_of) {
            panes_ui::open_node(ui, &kind, &name, side);
        }
    });
    ui.on_node_tab(move |kind, node, side| open((kind, node, side)));
    // A dependency of the File view is revealed in the pane beside.
    let select = with_ui(ui, |ui, (id, node): (i32, i32)| {
        if let (Ok(id), Some(node)) = (u32::try_from(id), index(node)) {
            // Its own navigation: a following File view stops following.
            let nav = name_of(node).map(Nav::Select);
            follow_ui::navigated(ui, id, nav.into_iter().collect());
            panes_ui::link_from(ui, id, node);
        }
    });
    follow_ui::on_follow(FILE, |ui, id, nav| {
        if let Nav::Select(name) = nav {
            show_node(ui, id, name);
        }
    });
    ui.on_file_select(move |id, node| select((id, node)));
    ui.on_show_in_graph(with_ui(ui, |ui, node: i32| {
        if let Some(node) = index(node) {
            panes_ui::reveal(ui, node);
        }
    }));
}
