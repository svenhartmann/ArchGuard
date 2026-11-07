//! "Follow left" in the window ([`crate::follow`]): the chain
//! toggle on the right pane's strip, and the views' navigation reports.
//!
//! A view reports what the user did with [`navigated`]; a view that can
//! follow registers how with [`on_follow`]. This module depends on no
//! view.

use std::cell::RefCell;
use std::rc::Rc;

use slint::ComponentHandle;

use crate::documents;
use crate::follow::{self, Nav, Place, Route};
use crate::panes::link;
use crate::panes_ui;
use crate::tab_support::with_ui;
use crate::ui::AppWindow;

/// How a view instance follows: `(ui, tab id, navigation)`.
type FollowFn = Rc<dyn Fn(&AppWindow, u32, &Nav)>;

/// Where a view instance stands (`tab id`); `None` while it builds.
type PlaceFn = Rc<dyn Fn(u32) -> Option<Place>>;

thread_local! {
    static FOLLOWS: RefCell<Vec<(&'static str, FollowFn)>> = const { RefCell::new(Vec::new()) };
    static PLACES: RefCell<Vec<(&'static str, PlaceFn)>> = const { RefCell::new(Vec::new()) };
}

/// Register the toggle once at start-up; a new pair (a view switched in
/// a pane, Follow switched on) brings the right pane to the left place.
pub fn wire(ui: &AppWindow) {
    let toggle = with_ui(ui, |ui, ()| {
        panes_ui::change(ui, |l| {
            l.follow_left = !l.follow_left && l.is_split() && !follow::blocked(l);
        });
    });
    ui.on_toggle_follow(move || toggle(()));
    panes_ui::on_change(|ui, before, after| {
        if follow::resync(before, after) {
            resync(ui);
        }
    });
}

/// Instances of `kind` follow the left pane with `f` (at start-up).
pub fn on_follow(kind: &'static str, f: impl Fn(&AppWindow, u32, &Nav) + 'static) {
    FOLLOWS.with_borrow_mut(|r| r.push((kind, Rc::new(f))));
}

/// Instance `id` of `kind` stands at `f(id)`, for a pane that starts
/// following it (at start-up).
pub fn on_place(kind: &'static str, f: impl Fn(u32) -> Option<Place> + 'static) {
    PLACES.with_borrow_mut(|r| r.push((kind, Rc::new(f))));
}

/// [`on_place`] for a view open once (`kind`), whose state is the active
/// document's `S`.
pub fn on_state_place<S: 'static>(kind: &'static str, place: fn(&S) -> Place) {
    on_place(kind, move |_| {
        documents::with_active_state(|s: &mut S| place(s))
    });
}

fn registered<F: Clone>(r: &RefCell<Vec<(&'static str, F)>>, kind: &str) -> Option<F> {
    r.borrow()
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, f)| f.clone())
}

/// The right pane takes the left pane's current place: its scope and its
/// selection, inside the link guard so its own reports keep Follow on.
fn resync(ui: &AppWindow) {
    let layout = panes_ui::layout();
    let (Some(left), Some(right)) = (layout.view(0), layout.view(1)) else {
        return;
    };
    let place = PLACES
        .with(|r| registered(r, &left.kind))
        .and_then(|f| f(left.id));
    let follow = FOLLOWS.with(|r| registered(r, &right.kind));
    let (Some(place), Some(f)) = (place, follow) else {
        return;
    };
    link::guarded(|| {
        for nav in follow::replay(&left.kind, &right.kind, &place) {
            f(ui, right.id, &nav);
        }
    });
}

/// Tab `from` navigated (`navs`, by the user unless a link runs): the
/// right pane follows it, or Follow switches off when `from` is the
/// right pane.
pub fn navigated(ui: &AppWindow, from: u32, navs: Vec<Nav>) {
    if navs.is_empty() || link::running() {
        return;
    }
    let layout = panes_ui::layout();
    let (kind, id) = match follow::route(&layout, from) {
        Route::Ignore => return,
        Route::Off => return panes_ui::change(ui, |l| l.follow_left = false),
        Route::Follow(kind, id) => (kind, id),
    };
    let left = layout.view(0).map(|t| t.kind.clone()).unwrap_or_default();
    let Some(f) = FOLLOWS.with(|r| registered(r, &kind)) else {
        return;
    };
    link::guarded(|| {
        for nav in navs.iter().filter_map(|n| follow::map(&left, &kind, n)) {
            f(ui, id, &nav);
        }
    });
}

/// Callback `f` of a view open once (`kind`: Start, Architecture, whose
/// state is the active document's `S`), reporting what it changed of
/// the view's [`Place`] as navigations.
pub fn reporting<S: 'static, A: 'static>(
    ui: &AppWindow,
    kind: &'static str,
    place: fn(&S) -> Place,
    f: impl Fn(A) + 'static,
) -> impl Fn(A) + 'static {
    let w = ui.as_weak();
    move |a| {
        let before = documents::with_active_state(|s: &mut S| place(s));
        f(a);
        let after = documents::with_active_state(|s: &mut S| place(s));
        let (Some(ui), Some(before), Some(after)) = (w.upgrade(), before, after) else {
            return;
        };
        if let Some(id) = shown_id(kind) {
            navigated(&ui, id, follow::place_navs(&before, &after));
        }
    }
}

/// The id of the shown tab of a view open once (Start, Architecture),
/// which reports its navigations under it.
fn shown_id(kind: &str) -> Option<u32> {
    let l = panes_ui::layout();
    (0..l.panes.len())
        .filter_map(|p| l.view(p))
        .find(|t| t.kind == kind)
        .map(|t| t.id)
}
