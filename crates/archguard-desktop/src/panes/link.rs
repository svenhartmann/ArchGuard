//! Linked selection between two panes: a node selected in one view is
//! revealed in the view instance beside it, if that view can reveal nodes.
//!
//! Revealing selects the node in the target instance, whose callbacks
//! would link back. [`follow`] runs one link at a time; a link started
//! while one runs is dropped, so a selection never bounces between the
//! panes.

use std::cell::Cell;

thread_local! {
    static LINKING: Cell<bool> = const { Cell::new(false) };
}

/// Views that can reveal a tree node (an index into the document's
/// package tree, shared by Graph and Matrix). A File view stays on its
/// node.
pub fn reveals(kind: &str) -> bool {
    matches!(kind, "graph" | "dsm")
}

/// Releases the guard also when `f` panics.
struct Running;

impl Drop for Running {
    fn drop(&mut self) {
        LINKING.set(false);
    }
}

/// Run `f` with the linked tab `target` (its kind and id, from
/// `Layout::linked`) unless there is none, it cannot reveal, or a link is
/// running. Returns whether `f` ran.
pub fn follow(target: Option<(&str, u32)>, f: impl FnOnce(&str, u32)) -> bool {
    let Some((kind, id)) = target.filter(|(kind, _)| reveals(kind)) else {
        return false;
    };
    guarded(|| f(kind, id))
}

/// Run `f` as the one running link (also "Follow left", `follow.rs`)
/// unless a link is running. Returns whether `f` ran.
pub fn guarded(f: impl FnOnce()) -> bool {
    if LINKING.replace(true) {
        return false;
    }
    let _running = Running;
    f();
    true
}

/// Whether a link runs: views changed now are changed by it, not by the
/// user.
pub fn running() -> bool {
    LINKING.get()
}
