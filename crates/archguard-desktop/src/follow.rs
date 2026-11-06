//! "Follow left": in a split layout the right pane follows the left
//! pane's navigation where the two views share a context, mapped by tree
//! node name (the views' common id): a package entered or left, packages
//! expanded or collapsed, a node selected.
//!
//! One direction only: the left pane leads. A navigation the right pane
//! makes itself switches Follow off ([`route`]); the follow runs inside
//! the link guard ([`link::guarded`]), so the right pane's own
//! reports while it follows are not taken for the user's.

use std::collections::BTreeSet;

use crate::panes::{self, Layout, link};
use crate::tree::Tree;

/// A navigation in a view, by tree node name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Nav {
    /// A package was entered (the World's plate, the Graph's opened
    /// package, the Matrix scope) or left for the project (`None`).
    Scope(Option<String>),
    /// Packages opened and closed in the tree.
    Expand {
        open: Vec<String>,
        close: Vec<String>,
    },
    /// A node was selected.
    Select(String),
}

/// Whether a right pane showing `right` can follow a left pane showing
/// `left`: World → Graph, Graph → World, Graph → Graph, Graph → Matrix, Graph or Matrix
/// → File view, Architecture → Graph.
pub fn pair(left: &str, right: &str) -> bool {
    matches!(
        (left, right),
        ("start", "graph")
            | ("graph", "start")
            | ("graph", "graph")
            | ("graph", "dsm")
            | ("graph", "file")
            | ("dsm", "file")
            | ("architecture", "graph")
    )
}

/// What the right view does for `nav` of the left. A selection between
/// Graph and Matrix is left to the linked selection, which reveals it
/// already; a File view follows selections only.
pub fn map(left: &str, right: &str, nav: &Nav) -> Option<Nav> {
    if !pair(left, right) {
        return None;
    }
    let linked = link::reveals(left) && link::reveals(right);
    match nav {
        Nav::Select(_) if linked => None,
        Nav::Select(_) => Some(nav.clone()),
        _ if right == panes::FILE => None,
        Nav::Expand { .. } if right == "start" => None,
        Nav::Expand { .. } if !matches!(left, "graph" | "dsm") => None,
        _ => Some(nav.clone()),
    }
}

/// What a navigation reported by tab `from` means for Follow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// Not followed: Follow is off, the layout not split, `from` not
    /// visible, or the pair has no common context.
    Ignore,
    /// The right pane navigated itself: switch Follow off.
    Off,
    /// The right pane's tab follows: its kind and id.
    Follow(String, u32),
}

pub fn route(l: &Layout, from: u32) -> Route {
    let (Some(left), Some(right)) = (l.view(0), l.view(1)) else {
        return Route::Ignore;
    };
    if !l.follow_left || !l.is_split() {
        return Route::Ignore;
    }
    if right.id == from {
        return Route::Off;
    }
    if left.id != from || !pair(&left.kind, &right.kind) {
        return Route::Ignore;
    }
    Route::Follow(right.kind.clone(), right.id)
}

/// Where a tree view (Graph, Matrix) stands, by tree node index.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Marks {
    pub scope: Option<usize>,
    pub open: BTreeSet<usize>,
    pub selected: Option<usize>,
}

/// The navigations from `before` to `after` in `tree`: scope, then
/// opened and closed packages, then the selection.
pub fn navs(tree: &Tree, before: &Marks, after: &Marks) -> Vec<Nav> {
    let name = |n: usize| tree.nodes.get(n).map(|t| t.name.clone());
    let names = |a: &BTreeSet<usize>, b: &BTreeSet<usize>| -> Vec<String> {
        a.difference(b).filter_map(|&n| name(n)).collect()
    };
    let mut out = Vec::new();
    if before.scope != after.scope {
        out.push(Nav::Scope(after.scope.and_then(name)));
    }
    let (open, close) = (
        names(&after.open, &before.open),
        names(&before.open, &after.open),
    );
    if !open.is_empty() || !close.is_empty() {
        out.push(Nav::Expand { open, close });
    }
    if before.selected != after.selected
        && let Some(n) = after.selected.and_then(name)
    {
        out.push(Nav::Select(n));
    }
    out
}

/// Where a view open once stands (the World, Architecture's building
/// blocks), by node name: its package scope and its selection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Place {
    pub scope: Option<String>,
    pub selected: Option<String>,
}

/// The navigations from `before` to `after`: a new scope, then a new
/// selection.
pub fn place_navs(before: &Place, after: &Place) -> Vec<Nav> {
    let mut out = Vec::new();
    if before.scope != after.scope {
        out.push(Nav::Scope(after.scope.clone()));
    }
    if before.selected != after.selected
        && let Some(n) = &after.selected
    {
        out.push(Nav::Select(n.clone()));
    }
    out
}

/// Why the right pane cannot follow, if it cannot: the layout is not
/// split or the pair has no common context.
pub fn blocked(l: &Layout) -> bool {
    match (l.view(0), l.view(1)) {
        (Some(left), Some(right)) => !pair(&left.kind, &right.kind),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn split(left: &str, right: &str) -> (Layout, u32, u32) {
        let mut l = Layout::single(left);
        let a = l.view(0).expect("tab").id;
        let node = (right == panes::FILE).then_some("a.rs");
        let b = l.split_right(Some(right), node).expect("tab");
        l.follow_left = true;
        (l, a, b)
    }

    fn scope(n: &str) -> Nav {
        Nav::Scope(Some(n.into()))
    }

    /// World → Graph, Graph → Graph, Graph → Matrix, Graph/Matrix → File
    /// and Architecture → Graph follow; Security beside the World does not.
    #[test]
    fn the_supported_pairs_follow_by_node() {
        let expand = Nav::Expand {
            open: vec!["src".into()],
            close: vec![],
        };
        let select = Nav::Select("src/a.ts".into());
        assert_eq!(map("start", "graph", &scope("src")), Some(scope("src")));
        assert_eq!(
            map("start", "graph", &Nav::Scope(None)),
            Some(Nav::Scope(None))
        );
        assert_eq!(map("start", "graph", &select), Some(select.clone()));
        assert_eq!(map("graph", "graph", &expand), Some(expand.clone()));
        assert_eq!(map("graph", "dsm", &scope("src")), Some(scope("src")));
        assert_eq!(map("graph", "dsm", &expand), Some(expand.clone()));
        assert_eq!(map("graph", "dsm", &select), None, "the link reveals it");
        assert_eq!(map("graph", "file", &select), Some(select.clone()));
        assert_eq!(map("dsm", "file", &select), Some(select.clone()));
        assert_eq!(map("graph", "file", &expand), None);
        assert_eq!(
            map("architecture", "graph", &scope("src")),
            Some(scope("src"))
        );
        assert_eq!(map("architecture", "graph", &select), Some(select.clone()));
        assert_eq!(map("start", "security", &scope("src")), None);
        // Graph → World: packages entered and selections; the World has
        // no tree to expand.
        assert_eq!(map("graph", "start", &scope("src")), Some(scope("src")));
        assert_eq!(map("graph", "start", &select), Some(select.clone()));
        assert_eq!(map("graph", "start", &expand), None);
        assert_eq!(
            map("dsm", "graph", &scope("src")),
            None,
            "left leads only from the list"
        );
    }

    /// The left pane leads; a navigation in the right pane switches
    /// Follow off; nothing follows when it is off or the pair cannot.
    #[test]
    fn the_left_leads_and_the_right_switches_follow_off() {
        let (l, left, right) = split("graph", "dsm");
        assert_eq!(route(&l, left), Route::Follow("dsm".into(), right));
        assert_eq!(route(&l, right), Route::Off);
        assert_eq!(route(&l, 999), Route::Ignore);
        let off = Layout {
            follow_left: false,
            ..l.clone()
        };
        assert_eq!(route(&off, left), Route::Ignore);
        let (sec, left, _) = split("start", "security");
        assert_eq!(route(&sec, left), Route::Ignore);
        assert!(blocked(&sec));
        assert!(!blocked(&l));
        assert!(blocked(&Layout::single("graph")), "one pane");
    }

    /// A view's change becomes navigations by node name: entering a
    /// package, opening and closing packages, selecting a node.
    #[test]
    fn a_change_in_a_tree_view_becomes_navigations() {
        let tree = crate::matrix::tests::tree();
        let n = |name: &str| tree.nodes.iter().position(|t| t.name == name).expect(name);
        let before = Marks::default();
        let after = Marks {
            scope: Some(n("lib")),
            open: [n("lib"), n("lib/src")].into(),
            selected: Some(n("lib/src/a.rs")),
        };
        assert_eq!(
            navs(&tree, &before, &after),
            [
                Nav::Scope(Some("lib".into())),
                Nav::Expand {
                    open: vec!["lib".into(), "lib/src".into()],
                    close: vec![]
                },
                Nav::Select("lib/src/a.rs".into()),
            ]
        );
        assert_eq!(
            navs(&tree, &after, &before),
            [
                Nav::Scope(None),
                Nav::Expand {
                    open: vec![],
                    close: vec!["lib".into(), "lib/src".into()]
                },
            ],
            "a cleared selection is no navigation"
        );
        assert!(navs(&tree, &after, &after).is_empty());
    }

    /// The World (or Architecture) entering a package and selecting an
    /// area become a scope and a selection; leaving the package for the
    /// overview is the project scope.
    #[test]
    fn a_place_change_becomes_navigations() {
        let overview = Place::default();
        let inside = Place {
            scope: Some("domain".into()),
            selected: None,
        };
        let picked = Place {
            selected: Some("domain/ledger.py".into()),
            ..inside.clone()
        };
        assert_eq!(place_navs(&overview, &inside), [scope("domain")]);
        assert_eq!(
            place_navs(&inside, &picked),
            [Nav::Select("domain/ledger.py".into())]
        );
        assert_eq!(place_navs(&picked, &overview), [Nav::Scope(None)]);
        assert!(place_navs(&picked, &picked).is_empty());
    }

    /// Follow belongs to a split layout: closing the right pane drops it.
    #[test]
    fn follow_ends_with_the_split() {
        let (mut l, _, _) = split("graph", "graph");
        l.close_pane(1);
        assert!(!l.follow_left);
    }
}
