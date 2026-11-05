//! What the tabs share: callbacks on the active document's state, Slint
//! list models, picker indices and snapshot mode.
//!
//! A tab's state type implements [`TabState`]; [`Actions`] turns closures
//! on that state into Slint callbacks.

use std::marker::PhantomData;

use slint::{ComponentHandle, ModelRc, VecModel};

use crate::documents;
use crate::ui::AppWindow;

/// One tab's state, kept per document in [`documents`].
pub trait TabState: Sized + 'static {
    /// Show the state in the window.
    fn push(&mut self, ui: &AppWindow);

    /// After a callback changed the state, before [`TabState::push`].
    fn changed(&mut self) {}
}

/// Run `f` on the active document's `T` and push it.
pub fn update<T: TabState>(ui: &AppWindow, f: impl FnOnce(&mut T, &AppWindow)) {
    documents::with_active_state(|s: &mut T| {
        f(s, ui);
        s.changed();
        s.push(ui);
    });
}

/// Builds Slint callbacks that change the active document's `T`.
pub struct Actions<T> {
    ui: slint::Weak<AppWindow>,
    state: PhantomData<fn() -> T>,
}

impl<T: TabState> Actions<T> {
    pub fn new(ui: &AppWindow) -> Self {
        Actions {
            ui: ui.as_weak(),
            state: PhantomData,
        }
    }

    /// A callback with one argument.
    pub fn on<A: 'static>(
        &self,
        f: impl Fn(&mut T, &AppWindow, A) + 'static,
    ) -> impl Fn(A) + 'static {
        let w = self.ui.clone();
        move |arg| {
            if let Some(ui) = w.upgrade() {
                update(&ui, |s: &mut T, ui| f(s, ui, arg));
            }
        }
    }

    /// A callback without argument.
    pub fn on0(&self, f: impl Fn(&mut T, &AppWindow) + 'static) -> impl Fn() + 'static {
        let g = self.on(move |s, ui, ()| f(s, ui));
        move || g(())
    }

    /// A callback with two arguments.
    pub fn on2<A: 'static, B: 'static>(
        &self,
        f: impl Fn(&mut T, &AppWindow, A, B) + 'static,
    ) -> impl Fn(A, B) + 'static {
        let g = self.on(move |s, ui, (a, b)| f(s, ui, a, b));
        move |a, b| g((a, b))
    }
}

/// A Slint list model of `items`.
pub fn model<T: Clone + 'static>(items: impl IntoIterator<Item = T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items.into_iter().collect::<Vec<_>>()))
}

/// A row or picker index from Slint; `None` for negative values (`-1` is
/// "nothing"). Call sites choose their own fallback.
pub fn index(i: i32) -> Option<usize> {
    usize::try_from(i).ok()
}

/// Whether this run renders a PNG (`ARCHGUARD_DESKTOP_SNAPSHOT`); always
/// false without the `snapshot` feature. Snapshot runs build tab states in
/// place so the picture shows them.
pub fn snapshot_mode() -> bool {
    cfg!(feature = "snapshot") && std::env::var_os("ARCHGUARD_DESKTOP_SNAPSHOT").is_some()
}

/// Show `node` (an index into the analysed graph's nodes) in the Graph tab.
pub fn reveal(ui: &AppWindow, node: usize) {
    ui.set_tab("graph".into());
    ui.invoke_reveal(node as i32);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_indices_are_none() {
        assert_eq!(index(-1), None);
        assert_eq!(index(3), Some(3));
    }
}
