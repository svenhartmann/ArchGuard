//! "Follow left" in the window ([`crate::follow`]): the chain
//! toggle on the right pane's strip, and the views' navigation reports.
//!
//! A view reports what the user did with [`navigated`]; a view that can
//! follow registers how with [`on_follow`]. This module depends on no
//! view.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use slint::ComponentHandle;

use crate::documents::{self, DocId, with_store};
use crate::follow::{self, Nav, Place, Route};
use crate::panes::link;
use crate::panes_ui;
use crate::tab_support::{self, with_ui};
use crate::ui::AppWindow;

/// How a view instance follows: `(ui, tab id, navigation)`.
type FollowFn = Rc<dyn Fn(&AppWindow, u32, &Nav)>;

/// What is done with a view's place once it has one.
type Then = Box<dyn FnOnce(&AppWindow, Place)>;

/// Hands where a view instance stands (`tab id`) to `then`: now, or,
/// while the view builds, once it is built.
type PlaceFn = Rc<dyn Fn(&AppWindow, u32, Then)>;

/// The packages open in a tree view instance (`tab id`), by name.
type OpenFn = Rc<dyn Fn(u32) -> Option<BTreeSet<String>>>;

/// A pane's shown tab: its kind and id.
type Side = (String, u32);

thread_local! {
    static FOLLOWS: RefCell<Vec<(&'static str, FollowFn)>> = const { RefCell::new(Vec::new()) };
    static PLACES: RefCell<Vec<(&'static str, PlaceFn)>> = const { RefCell::new(Vec::new()) };
    static OPENS: RefCell<Vec<(&'static str, OpenFn)>> = const { RefCell::new(Vec::new()) };
}

/// Register the toggle once at start-up; a new pair (a view switched in
/// a pane, Follow switched on) brings the right pane to the left place,
/// and so does a pair restored with the project, once its analysis is
/// shown.
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
    panes_ui::on_synced(|ui| {
        if first_shown() {
            resync(ui);
        }
    });
}

/// Marks a document whose views were shown with its analysis.
struct Shown;

/// Whether the active document shows its analysis for the first time
/// (marking it).
fn first_shown() -> bool {
    let doc = with_store(|s| s.active().filter(|d| d.analysis.is_some()).map(|d| d.id));
    doc.is_some_and(first_time)
}

/// Whether `doc` is shown for the first time (marking it).
fn first_time(doc: DocId) -> bool {
    documents::with_state(doc, |_: &mut Shown| ()).is_none() && {
        documents::set_state(doc, Shown);
        true
    }
}

/// Instances of `kind` follow the left pane with `f` (at start-up).
pub fn on_follow(kind: &'static str, f: impl Fn(&AppWindow, u32, &Nav) + 'static) {
    FOLLOWS.with_borrow_mut(|r| r.push((kind, Rc::new(f))));
}

/// Instance `id` of `kind` stands at `f(id)` (`None` while it builds),
/// for a pane that starts following it (at start-up).
pub fn on_place(kind: &'static str, f: impl Fn(u32) -> Option<Place> + 'static) {
    let f = move |ui: &AppWindow, id: u32, then: Then| {
        if let Some(place) = f(id) {
            then(ui, place);
        }
    };
    PLACES.with_borrow_mut(|r| r.push((kind, Rc::new(f))));
}

/// Instance `id` of tree view `kind` has packages `f(id)` open, which a
/// following tree view takes with the place (at start-up).
pub fn on_open(kind: &'static str, f: impl Fn(u32) -> Option<BTreeSet<String>> + 'static) {
    OPENS.with_borrow_mut(|r| r.push((kind, Rc::new(f))));
}

/// [`on_place`] for a view open once (`kind`), whose state is the active
/// document's `S`; while `S` builds, its place is handed on once it
/// arrives.
pub fn on_state_place<S: 'static>(kind: &'static str, place: fn(&S) -> Place) {
    let f =
        move |ui: &AppWindow, _: u32, then: Then| match documents::with_active_state(|s: &mut S| {
            place(s)
        }) {
            Some(p) => then(ui, p),
            None => {
                let w = ui.as_weak();
                tab_support::run_or_queue(move |s: &mut S| {
                    if let Some(ui) = w.upgrade() {
                        then(&ui, place(s));
                    }
                });
            }
        };
    PLACES.with_borrow_mut(|r| r.push((kind, Rc::new(f))));
}

/// Whether views of `kind` report their place for a new pair.
#[cfg(test)]
pub fn leads(kind: &str) -> bool {
    PLACES.with(|r| registered(r, kind)).is_some()
}

fn registered<F: Clone>(r: &RefCell<Vec<(&'static str, F)>>, kind: &str) -> Option<F> {
    r.borrow()
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, f)| f.clone())
}

/// The right pane takes the left pane's current place, once the left
/// view has one.
fn resync(ui: &AppWindow) {
    let layout = panes_ui::layout();
    let (Some(left), Some(right), Some(doc)) =
        (layout.view(0), layout.view(1), documents::active_id())
    else {
        return;
    };
    let Some(place) = PLACES.with(|r| registered(r, &left.kind)) else {
        return;
    };
    let sides: (Side, Side) = ((left.kind.clone(), left.id), (right.kind.clone(), right.id));
    place(
        ui,
        left.id,
        Box::new(move |ui, p| replay(ui, doc, &sides, &p)),
    );
}

/// The right tab takes the left tab's `place` if the two still follow in
/// document `doc`: its scope, its open packages (tree views), then its
/// selection; inside the link guard so its own reports keep Follow on.
fn replay(ui: &AppWindow, doc: DocId, (left, right): &(Side, Side), place: &Place) {
    let same = documents::active_id() == Some(doc)
        && follow::following(&panes_ui::layout()) == Some((left.1, right.1));
    let Some(f) = FOLLOWS.with(|r| registered(r, &right.0)).filter(|_| same) else {
        return;
    };
    let (scope, rest): (Vec<Nav>, Vec<Nav>) = follow::replay(&left.0, &right.0, place)
        .into_iter()
        .partition(|n| matches!(n, Nav::Scope(_)));
    link::guarded(|| {
        scope.iter().for_each(|nav| f(ui, right.1, nav));
        // Read after the scope: entering a package opens packages.
        if let Some(nav) = open_nav(left, right) {
            f(ui, right.1, &nav);
        }
        rest.iter().for_each(|nav| f(ui, right.1, nav));
    });
}

/// What opens and closes packages in the right tree view as in the left.
fn open_nav(left: &Side, right: &Side) -> Option<Nav> {
    let open = |(kind, id): &Side| OPENS.with(|r| registered(r, kind)).and_then(|f| f(*id));
    follow::expand(&left.0, &right.0, &open(left)?, &open(right)?)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// A document takes its restored place once, when it first shows its
    /// analysis; another document takes its own.
    #[test]
    fn a_document_replays_once_when_its_analysis_shows() {
        let (a, b) = with_store(|s| {
            *s = documents::Store::default();
            let (a, _) = s.open(Path::new("/a"));
            let (b, _) = s.open(Path::new("/b"));
            (a, b)
        });
        assert!(!first_shown(), "no analysis yet");
        assert!(first_time(b));
        assert!(!first_time(b), "once");
        assert!(first_time(a), "per document");
        assert!(!first_time(a));
    }
}
