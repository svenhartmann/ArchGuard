//! View → Fullscreen (#29), the web viewer's fullscreen button: the whole
//! window, not one view. ⌃⌘F on macOS, F11 elsewhere (`ui/app.slint`).
//! Escape is left to the views (the World closes its details with it).

use slint::ComponentHandle;

use crate::ui::AppWindow;

/// A window that can be shown fullscreen.
pub trait Screen {
    fn is_fullscreen(&self) -> bool;
    fn set_fullscreen(&self, on: bool);
}

impl Screen for slint::Window {
    fn is_fullscreen(&self) -> bool {
        slint::Window::is_fullscreen(self)
    }

    fn set_fullscreen(&self, on: bool) {
        slint::Window::set_fullscreen(self, on);
    }
}

/// Switch `w` into or out of fullscreen, from its current state (the user
/// may have left fullscreen through the title bar); returns the new state.
pub fn toggle(w: &impl Screen) -> bool {
    let on = !w.is_fullscreen();
    w.set_fullscreen(on);
    on
}

/// Register View → Fullscreen once at start-up.
pub fn wire(ui: &AppWindow) {
    let weak = ui.as_weak();
    ui.on_toggle_fullscreen(move || {
        if let Some(ui) = weak.upgrade() {
            let on = toggle(ui.window());
            ui.set_fullscreen(on);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[derive(Default)]
    struct Fake {
        on: Cell<bool>,
        calls: Cell<usize>,
    }

    impl Screen for Fake {
        fn is_fullscreen(&self) -> bool {
            self.on.get()
        }

        fn set_fullscreen(&self, on: bool) {
            self.calls.set(self.calls.get() + 1);
            self.on.set(on);
        }
    }

    #[test]
    fn toggle_enters_and_leaves_fullscreen() {
        let w = Fake::default();
        assert!(toggle(&w));
        assert!(w.on.get());
        assert!(!toggle(&w));
        assert!(!w.on.get());
        assert_eq!(w.calls.get(), 2);
    }

    /// Left through the title bar: the next toggle enters again.
    #[test]
    fn toggle_follows_the_window_not_the_last_toggle() {
        let w = Fake::default();
        toggle(&w);
        w.on.set(false);
        assert!(toggle(&w));
    }
}
