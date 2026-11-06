//! State of view instances: the views a document can show several times
//! (Graph, Matrix, File; `panes::multi`) keep one state per view tab,
//! keyed by the tab's id, in the active document's [`Instances`].
//!
//! [`sync`] makes a kind's instances those of the layout (building new
//! ones, dropping closed ones) and shows each visible instance in its
//! pane: an [`Instance`] writes its data into the row of its pane in the
//! view's per-pane model ([`set_row`]), which the pane's Slint component
//! reads. Callbacks change one instance with [`update`].
//!
//! The document's analysis and package tree stay one per document
//! (`tab_support::tree`); instances share them.

use std::collections::BTreeMap;

use slint::{Model, VecModel};

use crate::documents;
use crate::panes::{Layout, Tab};
use crate::tree::Tree;
use crate::ui::AppWindow;

/// The instances of one view kind in a document, by tab id.
pub struct Instances<T>(BTreeMap<u32, T>);

impl<T> Default for Instances<T> {
    fn default() -> Self {
        Instances(BTreeMap::new())
    }
}

/// A view instance's state.
pub trait Instance: Sized + 'static {
    /// Show the state in pane `pane`; `id` is its tab's id.
    fn show(&mut self, ui: &AppWindow, pane: usize, id: u32);
}

/// Run `f` on instance `id` of the active document's `T`.
pub fn with<T: 'static, R>(id: u32, f: impl FnOnce(&mut T) -> R) -> Option<R> {
    documents::with_active_state(|m: &mut Instances<T>| m.0.get_mut(&id).map(f)).flatten()
}

/// Run `f` on every instance of the active document's `T`.
pub fn each<T: 'static>(mut f: impl FnMut(&mut T)) {
    documents::with_active_state(|m: &mut Instances<T>| m.0.values_mut().for_each(&mut f));
}

/// Run `f` on instance `id` and show it again in the pane `layout` shows
/// it in, if any. Returns what `f` returned.
pub fn update<T: Instance, R>(
    ui: &AppWindow,
    layout: &Layout,
    id: u32,
    f: impl FnOnce(&mut T) -> R,
) -> Option<R> {
    let pane = pane_of(layout, id);
    with(id, |s: &mut T| {
        let r = f(s);
        if let Some(p) = pane {
            crate::timing::measure(std::any::type_name::<T>(), || s.show(ui, p, id));
        }
        r
    })
}

/// The pane that shows tab `id`.
pub fn pane_of(layout: &Layout, id: u32) -> Option<usize> {
    (0..layout.panes.len()).find(|&p| layout.view(p).is_some_and(|t| t.id == id))
}

/// The tree node named `name`.
pub fn node_named(tree: &Tree, name: &str) -> Option<usize> {
    tree.nodes.iter().position(|n| n.name == name)
}

/// Make the active document's instances of `kind` the tabs of `layout`
/// and show the visible ones; see [`reconcile`]. Returns the new tabs
/// whose node `tree` does not have.
pub fn sync<T: Instance>(
    ui: &AppWindow,
    layout: &Layout,
    kind: &str,
    tree: &Tree,
    build: impl FnMut(&Tab, Option<usize>) -> Option<T>,
) -> Vec<u32> {
    let gone = reconcile(layout, kind, tree, build);
    documents::with_active_state(|m: &mut Instances<T>| {
        for p in 0..layout.panes.len() {
            let Some(t) = layout.view(p).filter(|t| t.kind == kind) else {
                continue;
            };
            if let Some(s) = m.0.get_mut(&t.id) {
                crate::timing::measure(std::any::type_name::<T>(), || s.show(ui, p, t.id));
            }
        }
    });
    gone
}

/// Drop the instances of closed tabs of `kind` and build those of new
/// ones with `build` (given the tab's node in `tree`; `None` without one
/// or when `tree` lacks it; `build` may refuse). Returns the new tabs
/// whose node `tree` lacks.
fn reconcile<T: 'static>(
    layout: &Layout,
    kind: &str,
    tree: &Tree,
    mut build: impl FnMut(&Tab, Option<usize>) -> Option<T>,
) -> Vec<u32> {
    let tabs: Vec<&Tab> = layout
        .panes
        .iter()
        .flat_map(|p| &p.tabs)
        .filter(|t| t.kind == kind)
        .collect();
    let mut gone = Vec::new();
    documents::with_active_state_or(Instances::<T>::default, |m| {
        m.0.retain(|id, _| tabs.iter().any(|t| t.id == *id));
        let new: Vec<&&Tab> = tabs.iter().filter(|t| !m.0.contains_key(&t.id)).collect();
        for t in new {
            let node = t.node.as_deref().and_then(|n| node_named(tree, n));
            if t.node.is_some() && node.is_none() {
                gone.push(t.id);
            }
            if let Some(s) = build(t, node) {
                m.0.insert(t.id, s);
            }
        }
    });
    gone
}

/// Put `row` at index `pane` of a per-pane model, adding empty rows
/// before it as needed.
pub fn set_row<T: Clone + Default + 'static>(model: &VecModel<T>, pane: usize, row: T) {
    while model.row_count() <= pane {
        model.push(T::default());
    }
    model.set_row_data(pane, row);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panes::FILE;
    use std::path::Path;

    /// A view's state: the node its tab was opened on.
    struct Demo(Option<usize>);

    fn open_document() {
        documents::with_store(|s| {
            *s = documents::Store::default();
            s.open(Path::new("/i"));
        });
    }

    fn ids() -> Vec<u32> {
        documents::with_active_state(|m: &mut Instances<Demo>| m.0.keys().copied().collect())
            .unwrap_or_default()
    }

    /// Each Graph tab has its own state, built once; closing a tab drops
    /// its state.
    #[test]
    fn instances_follow_the_tabs_of_the_layout() {
        open_document();
        let tree = crate::matrix::tests::tree();
        let lib = node_named(&tree, "lib").expect("node");
        let mut l = Layout::single("graph");
        let first = l.view(0).expect("tab").id;
        let second = l.open_tab("graph", Some("lib")).expect("tab");
        let mut built = Vec::new();
        let gone = reconcile(&l, "graph", &tree, |t, node| {
            built.push((t.id, node));
            Some(Demo(node))
        });
        assert!(gone.is_empty());
        assert_eq!(built, [(first, None), (second, Some(lib))]);
        assert_eq!(with(second, |d: &mut Demo| d.0), Some(Some(lib)));
        assert_eq!(with(first, |d: &mut Demo| d.0), Some(None), "own state");
        l.close_tab(0, 0);
        let again = reconcile(&l, "graph", &tree, |_, _| -> Option<Demo> {
            panic!("nothing new to build")
        });
        assert!(again.is_empty());
        assert_eq!(ids(), [second], "the closed tab's state is gone");
    }

    /// A tab of a node the analysis no longer has is reported, so the
    /// layout can drop it; it gets no state.
    #[test]
    fn a_tab_of_a_missing_node_is_reported() {
        open_document();
        let tree = crate::matrix::tests::tree();
        let mut l = Layout::single("graph");
        let gone = l.split_right(Some(FILE), Some("no/such.rs")).expect("tab");
        let kept = l.open_tab(FILE, Some("lib")).expect("tab");
        let missing = reconcile(&l, FILE, &tree, |_, node| node.map(|n| Demo(Some(n))));
        assert_eq!(missing, [gone]);
        assert_eq!(ids(), [kept]);
    }
}
