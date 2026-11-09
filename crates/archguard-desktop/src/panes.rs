//! Panes of a document (ADR 0002 decision 6): one or two panes side by
//! side, each with view tabs. Independent of Slint; `panes_ui.rs` shows
//! the layout and `pane_store.rs` stores it per project.
//!
//! A view tab is an *instance* of a view kind (Start, Graph, Matrix, …)
//! with an id unique in its layout. Graph, Matrix and File ([`multi`]) can
//! be open several times per document: "Open in new tab" and "Open to the
//! side" always add an instance, and every instance keeps its own state,
//! keyed by its id (`instances::Instances`). The other views read their
//! data from window-wide globals, so they stay open at most once per
//! document: opening one that is open selects its tab, opening it to the
//! side moves its tab.

pub mod link;

/// Every view of the view bar, in its order.
pub const VIEWS: [&str; 9] = [
    "start",
    "architecture",
    "graph",
    "dsm",
    "calls",
    "services",
    "runtime",
    "quality",
    "security",
];

/// The view of one tree node (a file or package), opened from the Graph or
/// the Matrix; not in the view bar.
pub const FILE: &str = "file";

/// The `archguard.toml` editor (View → Settings), open once per
/// document; not in the view bar.
pub const SETTINGS: &str = "settings";

/// The order in which a pane that needs a view takes one not open yet.
const FREE_ORDER: [&str; 9] = [
    "dsm",
    "graph",
    "start",
    "architecture",
    "quality",
    "calls",
    "services",
    "runtime",
    "security",
];

/// The left pane's share of the width, at least and at most.
pub const RATIO_MIN: f32 = 0.2;
pub const RATIO_MAX: f32 = 0.8;

/// One view tab: an instance of a view kind.
#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    /// A view id from [`VIEWS`] or [`FILE`].
    pub kind: String,
    /// Unique in the layout; keys the instance's state.
    pub id: u32,
    /// The tree node the instance is about, by name: the Graph's opened
    /// package, the Matrix scope, the File view's node.
    pub node: Option<String>,
    /// The Graph's display: 0 tree, 1 network, 2 dependencies.
    pub display: i32,
    /// The Start tab's kept World view (`start_ui::memo`), as key/value
    /// pairs; empty for other views.
    pub world: Vec<(String, String)>,
}

impl Tab {
    /// Kind and id, as [`link::follow`] takes a tab.
    pub fn key(&self) -> (&str, u32) {
        (&self.kind, self.id)
    }

    /// The node's last path segment, for the tab title ("Graph · core").
    pub fn label(&self) -> &str {
        let name = self.node.as_deref().unwrap_or("");
        name.trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or(name)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pane {
    /// In tab order; never empty in a layout.
    pub tabs: Vec<Tab>,
    pub active: usize,
}

/// The panes of one document, left to right.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// One or two panes.
    pub panes: Vec<Pane>,
    /// The pane the view bar acts on.
    pub focus: usize,
    /// The left pane's share of the width when split.
    pub ratio: f32,
    /// The id the next new tab gets.
    pub next: u32,
    /// "Follow left": the right pane follows the left pane's navigation
    /// (`crate::follow`); only in a split layout.
    pub follow_left: bool,
}

impl Default for Layout {
    fn default() -> Self {
        Layout::single("graph")
    }
}

/// A view kind this version knows.
pub fn known(kind: &str) -> bool {
    VIEWS.contains(&kind) || kind == FILE || kind == SETTINGS
}

/// Kinds of which a document can show several instances, each with its
/// own state.
pub fn multi(kind: &str) -> bool {
    matches!(kind, "graph" | "dsm" | FILE)
}

fn free_kind(l: &Layout) -> &'static str {
    FREE_ORDER
        .iter()
        .find(|v| l.find(v).is_none())
        .unwrap_or(&"graph")
}

impl Layout {
    /// One pane showing `view` (the Graph for an unknown id or a File
    /// view without node).
    pub fn single(view: &str) -> Self {
        let view = if VIEWS.contains(&view) { view } else { "graph" };
        let mut l = Layout {
            panes: vec![Pane {
                tabs: Vec::new(),
                active: 0,
            }],
            focus: 0,
            ratio: 0.5,
            next: 1,
            follow_left: false,
        };
        let tab = l.new_tab(view, None);
        l.panes[0].tabs.push(tab);
        l
    }

    pub fn is_split(&self) -> bool {
        self.panes.len() > 1
    }

    /// The tab shown in `pane`.
    pub fn view(&self, pane: usize) -> Option<&Tab> {
        let p = self.panes.get(pane)?;
        p.tabs.get(p.active)
    }

    /// The kind shown in the focused pane.
    pub fn focused(&self) -> &str {
        self.view(self.focus).map_or("graph", |t| t.kind.as_str())
    }

    /// Pane and tab index of an instance of `kind`: the one shown in the
    /// focused pane, else one shown in the other pane, else the first.
    pub fn find(&self, kind: &str) -> Option<(usize, usize)> {
        let shown = |p: usize| {
            let pane = self.panes.get(p)?;
            (pane.tabs.get(pane.active)?.kind == kind).then_some((p, pane.active))
        };
        shown(self.focus)
            .or_else(|| shown(1 - self.focus.min(1)).filter(|_| self.is_split()))
            .or_else(|| {
                self.panes
                    .iter()
                    .enumerate()
                    .find_map(|(p, pane)| Some((p, pane.tabs.iter().position(|t| t.kind == kind)?)))
            })
    }

    /// Pane and tab index of the tab with `id`.
    pub fn find_id(&self, id: u32) -> Option<(usize, usize)> {
        self.panes
            .iter()
            .enumerate()
            .find_map(|(p, pane)| Some((p, pane.tabs.iter().position(|t| t.id == id)?)))
    }

    /// The tab with `id`, to change what it is about.
    pub fn tab_mut(&mut self, id: u32) -> Option<&mut Tab> {
        let (p, t) = self.find_id(id)?;
        Some(&mut self.panes[p].tabs[t])
    }

    /// The tab shown beside tab `id` when that one is visible in a split
    /// layout: where a selection in it is revealed.
    pub fn linked(&self, id: u32) -> Option<&Tab> {
        let (p, _) = self.find_id(id).filter(|_| self.is_split())?;
        (self.view(p)?.id == id).then(|| self.view(1 - p)).flatten()
    }

    /// The view bar: select a tab of `kind`, or show a new instance in the
    /// focused tab.
    pub fn navigate(&mut self, kind: &str) {
        if let Some((p, t)) = self.find(kind) {
            return self.select_tab(p, t);
        }
        if VIEWS.contains(&kind) {
            let tab = self.new_tab(kind, None);
            let pane = &mut self.panes[self.focus];
            pane.tabs[pane.active] = tab;
        }
    }

    /// "Open in new tab": a tab after the focused pane's active one; a
    /// view that is open only once is selected instead. Returns the id of
    /// the tab shown.
    pub fn open_tab(&mut self, kind: &str, node: Option<&str>) -> Option<u32> {
        if !multi(kind)
            && let Some((p, t)) = self.find(kind)
        {
            self.select_tab(p, t);
            return Some(self.panes[p].tabs[t].id);
        }
        if !known(kind) || (kind == FILE && node.is_none()) {
            return None;
        }
        let tab = self.new_tab(kind, node);
        let id = tab.id;
        self.insert(self.focus, tab);
        Some(id)
    }

    pub fn select_tab(&mut self, pane: usize, tab: usize) {
        if self.panes.get(pane).is_some_and(|p| tab < p.tabs.len()) {
            self.panes[pane].active = tab;
            self.focus = pane;
        }
    }

    pub fn focus(&mut self, pane: usize) {
        self.focus = pane.min(self.panes.len() - 1);
    }

    /// "Open to the side": show `kind` (a new instance, or the tab of a
    /// view open once) in the right pane; without a kind the focused tab
    /// of a pane with several, else a view not open yet. Focuses the right
    /// pane and returns the id of the tab shown there.
    pub fn split_right(&mut self, kind: Option<&str>, node: Option<&str>) -> Option<u32> {
        let kind = kind.filter(|k| known(k) && (*k != FILE || node.is_some()));
        let tab = match kind {
            Some(k) if multi(k) => Some(self.new_tab(k, node)),
            Some(k) => Some(self.remove(k).unwrap_or_else(|| self.new_tab(k, None))),
            None if self.is_split() => None,
            None if self.panes[0].tabs.len() > 1 => remove_active(self, 0),
            None => Some(self.new_tab(free_kind(self), None)),
        };
        if !self.is_split() {
            self.panes.push(Pane {
                tabs: Vec::new(),
                active: 0,
            });
        }
        if let Some(tab) = tab {
            self.insert(1, tab);
        }
        fill(self);
        self.focus = 1;
        self.view(1).map(|t| t.id)
    }

    /// Close a tab; the last tab of a split pane closes the pane, the last
    /// view of the document stays.
    pub fn close_tab(&mut self, pane: usize, tab: usize) {
        let Some(p) = self.panes.get_mut(pane).filter(|p| tab < p.tabs.len()) else {
            return;
        };
        if p.tabs.len() > 1 {
            p.tabs.remove(tab);
            if tab < p.active || p.active == p.tabs.len() {
                p.active -= 1;
            }
        } else {
            self.close_pane(pane);
        }
    }

    /// Close a pane and its tabs; one pane always stays.
    pub fn close_pane(&mut self, pane: usize) {
        if self.is_split() && pane < self.panes.len() {
            self.panes.remove(pane);
            self.focus = 0;
            self.follow_left = false;
        }
    }

    pub fn set_ratio(&mut self, ratio: f32) {
        self.ratio = if ratio.is_finite() {
            ratio.clamp(RATIO_MIN, RATIO_MAX)
        } else {
            0.5
        };
    }

    /// A tab with a new id.
    fn new_tab(&mut self, kind: &str, node: Option<&str>) -> Tab {
        let id = self.next;
        self.next += 1;
        Tab {
            kind: kind.to_string(),
            id,
            // The Settings tab is titled by its file.
            node: node
                .or((kind == SETTINGS).then_some("archguard.toml"))
                .map(str::to_string),
            display: 0,
            world: Vec::new(),
        }
    }

    /// Insert `tab` after the active tab of `pane` and select it.
    fn insert(&mut self, pane: usize, tab: Tab) {
        let p = &mut self.panes[pane];
        let at = if p.tabs.is_empty() { 0 } else { p.active + 1 };
        p.tabs.insert(at, tab);
        p.active = at;
        self.focus = pane;
    }

    /// Take the tab of `kind` out, leaving its pane possibly empty for
    /// [`fill`].
    fn remove(&mut self, kind: &str) -> Option<Tab> {
        let (p, t) = self.find(kind)?;
        let pane = &mut self.panes[p];
        let tab = pane.tabs.remove(t);
        if pane.active > t || pane.active == pane.tabs.len() {
            pane.active = pane.active.saturating_sub(1);
        }
        Some(tab)
    }
}

/// Take the active tab of pane `pane` out (its pane keeps a tab).
fn remove_active(l: &mut Layout, pane: usize) -> Option<Tab> {
    let p = l.panes.get_mut(pane)?;
    let tab = p.tabs.remove(p.active);
    p.active = p.active.min(p.tabs.len().saturating_sub(1));
    Some(tab)
}

/// Give every empty pane a view not open yet.
fn fill(l: &mut Layout) {
    for p in 0..l.panes.len() {
        if l.panes[p].tabs.is_empty() {
            let tab = l.new_tab(free_kind(l), None);
            l.panes[p] = Pane {
                tabs: vec![tab],
                active: 0,
            };
        }
    }
}

/// A layout read from the settings with unknown views, repeated views
/// that are open once, File views without node, empty or extra panes and
/// out-of-range indices dropped. Tab ids are numbered anew.
pub fn repaired(layout: Layout) -> Layout {
    let mut seen: Vec<String> = Vec::new();
    let mut next = 1;
    let mut panes = Vec::new();
    for mut p in layout.panes {
        p.tabs.retain(|t| {
            let once = !multi(&t.kind);
            let keep = known(&t.kind)
                && !(once && seen.contains(&t.kind))
                && (t.kind != FILE || t.node.is_some());
            if keep && once {
                seen.push(t.kind.clone());
            }
            keep
        });
        for t in &mut p.tabs {
            t.id = next;
            next += 1;
        }
        if !p.tabs.is_empty() {
            p.active = p.active.min(p.tabs.len() - 1);
            panes.push(p);
        }
    }
    panes.truncate(2);
    if panes.is_empty() {
        return Layout::default();
    }
    let mut l = Layout {
        focus: layout.focus.min(panes.len() - 1),
        follow_left: layout.follow_left && panes.len() > 1,
        panes,
        ratio: 0.5,
        next,
    };
    l.set_ratio(layout.ratio);
    l
}

/// Remove tab `id` (its node is gone from the analysis). The last tab of
/// the document becomes a Graph.
pub fn drop_tab(layout: &mut Layout, id: u32) {
    let Some((p, t)) = layout.find_id(id) else {
        return;
    };
    if layout.is_split() || layout.panes[p].tabs.len() > 1 {
        return layout.close_tab(p, t);
    }
    layout.panes[p].tabs[t] = layout.new_tab("graph", None);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tabs(l: &Layout) -> Vec<Vec<&str>> {
        l.panes
            .iter()
            .map(|p| p.tabs.iter().map(|t| t.kind.as_str()).collect())
            .collect()
    }

    fn visible(l: &Layout) -> Vec<&str> {
        (0..l.panes.len())
            .filter_map(|p| l.view(p))
            .map(|t| t.kind.as_str())
            .collect()
    }

    fn ids(l: &Layout) -> Vec<u32> {
        l.panes
            .iter()
            .flat_map(|p| p.tabs.iter().map(|t| t.id))
            .collect()
    }

    fn tab(kind: &str, node: Option<&str>) -> Tab {
        Tab {
            kind: kind.into(),
            id: 0,
            node: node.map(str::to_string),
            display: 0,
            world: Vec::new(),
        }
    }

    #[test]
    fn a_new_layout_is_one_pane_with_one_view() {
        let l = Layout::single("dsm");
        assert_eq!(tabs(&l), [["dsm"]]);
        assert_eq!(l.focused(), "dsm");
        assert!(!l.is_split());
        assert_eq!(Layout::default().focused(), "graph");
        assert_eq!(
            Layout::single(FILE).focused(),
            "graph",
            "a File view needs a node"
        );
    }

    #[test]
    fn the_view_bar_replaces_the_focused_view_or_selects_its_tab() {
        let mut l = Layout::single("graph");
        l.navigate("quality");
        assert_eq!(tabs(&l), [["quality"]], "replaces in place");
        l.open_tab("graph", None);
        assert_eq!(tabs(&l), [["quality", "graph"]]);
        assert_eq!(l.focused(), "graph");
        l.navigate("quality");
        assert_eq!(tabs(&l), [["quality", "graph"]], "an open view is selected");
        assert_eq!(l.focused(), "quality");
    }

    #[test]
    fn views_open_once_are_selected_instead_of_opened_again() {
        let mut l = Layout::single("graph");
        l.open_tab("quality", None);
        l.select_tab(0, 0);
        l.open_tab("calls", None);
        assert_eq!(tabs(&l), [["graph", "calls", "quality"]]);
        l.open_tab("quality", None);
        assert_eq!(tabs(&l), [["graph", "calls", "quality"]], "no second tab");
        assert_eq!(l.focused(), "quality");
        assert_eq!(l.open_tab("nonsense", None), None);
        assert_eq!(l.panes[0].tabs.len(), 3, "unknown views are ignored");
    }

    /// Graph, Matrix and File open a new instance every time, each with an id
    /// of its own.
    #[test]
    fn graph_matrix_and_file_open_as_new_instances() {
        let mut l = Layout::single("graph");
        let a = l.open_tab("graph", Some("app")).expect("a tab");
        let b = l.open_tab("graph", Some("core")).expect("a tab");
        l.open_tab("dsm", None);
        l.open_tab("dsm", None);
        let f = l.open_tab(FILE, Some("core/lib.rs")).expect("a tab");
        assert_eq!(
            tabs(&l),
            [["graph", "graph", "graph", "dsm", "dsm", "file"]]
        );
        let mut all = ids(&l);
        all.dedup();
        assert_eq!(all.len(), 6, "ids are unique: {all:?}");
        assert_ne!(a, b);
        assert_eq!(l.view(0).map(|t| t.id), Some(f), "the new tab is shown");
        let (p, t) = l.find_id(b).expect("open");
        assert_eq!(l.panes[p].tabs[t].label(), "core");
        assert_eq!(l.open_tab(FILE, None), None, "a File view needs a node");
    }

    /// View → Settings: one tab per document, titled by its file; not in
    /// the view bar, kept when a stored layout is restored.
    #[test]
    fn settings_open_once_titled_by_the_file() {
        let mut l = Layout::single("graph");
        let id = l.open_tab(SETTINGS, None).expect("a tab");
        l.select_tab(0, 0);
        assert_eq!(l.open_tab(SETTINGS, None), Some(id), "selected again");
        assert_eq!(tabs(&l), [["graph", SETTINGS]]);
        assert_eq!(l.view(0).map(Tab::label), Some("archguard.toml"));
        assert!(!VIEWS.contains(&SETTINGS));
        assert_eq!(tabs(&repaired(l)), [["graph", SETTINGS]]);
    }

    #[test]
    fn closing_tabs_keeps_a_neighbour_and_never_the_last_view() {
        let mut l = Layout::single("graph");
        l.open_tab("dsm", None);
        l.open_tab("calls", None);
        l.select_tab(0, 1);
        l.close_tab(0, 1);
        assert_eq!(tabs(&l), [["graph", "calls"]]);
        assert_eq!(l.focused(), "calls", "the right neighbour");
        l.close_tab(0, 1);
        assert_eq!(l.focused(), "graph");
        l.close_tab(0, 0);
        assert_eq!(tabs(&l), [["graph"]], "the only view stays");
        l.close_tab(3, 0);
        assert_eq!(tabs(&l), [["graph"]]);
    }

    #[test]
    fn split_right_opens_a_second_pane_with_a_view_not_shown_yet() {
        let mut l = Layout::single("graph");
        l.split_right(None, None);
        assert_eq!(tabs(&l), [["graph"], ["dsm"]]);
        assert_eq!(l.focus, 1);
        assert!(l.is_split());
        // Splitting again only focuses the right pane.
        l.focus(0);
        l.split_right(None, None);
        assert_eq!(tabs(&l), [["graph"], ["dsm"]]);
        assert_eq!(l.focus, 1);
    }

    #[test]
    fn split_right_moves_the_active_tab_of_a_pane_with_several() {
        let mut l = Layout::single("graph");
        l.open_tab("architecture", None);
        l.split_right(None, None);
        assert_eq!(tabs(&l), [["graph"], ["architecture"]]);
        assert_eq!(visible(&l), ["graph", "architecture"]);
    }

    /// "Open to the side" on the only Graph shows a second Graph beside it:
    /// two instances to compare.
    #[test]
    fn a_graph_opened_to_the_side_is_a_second_instance() {
        let mut l = Layout::single("graph");
        let right = l.split_right(Some("graph"), Some("core")).expect("a tab");
        assert_eq!(tabs(&l), [["graph"], ["graph"]]);
        assert_ne!(l.view(0).map(|t| t.id), Some(right));
        assert_eq!(l.view(1).and_then(|t| t.node.as_deref()), Some("core"));
        l.focus(0);
        l.split_right(Some(FILE), Some("core/lib.rs"));
        assert_eq!(tabs(&l), [vec!["graph"], vec!["graph", "file"]]);
        assert_eq!(l.focus, 1);
    }

    #[test]
    fn a_view_open_once_moves_between_panes() {
        let mut l = Layout::single("quality");
        // Opening the only view to the side leaves another one behind.
        l.split_right(Some("quality"), None);
        assert_eq!(tabs(&l), [["dsm"], ["quality"]]);
        l.focus(0);
        l.split_right(Some("start"), None);
        assert_eq!(tabs(&l), [vec!["dsm"], vec!["quality", "start"]]);
        l.focus(0);
        l.navigate("quality");
        assert_eq!(l.focus, 1, "the view bar goes to the pane showing it");
        assert_eq!(l.focused(), "quality");
    }

    #[test]
    fn the_view_bar_prefers_the_instance_in_the_focused_pane() {
        let mut l = Layout::single("graph");
        l.split_right(Some("graph"), None);
        l.open_tab("dsm", None);
        l.focus(0);
        l.navigate("graph");
        assert_eq!(
            (l.focus, l.panes[1].active),
            (0, 1),
            "left Graph, right stays"
        );
        l.focus(1);
        l.navigate("graph");
        assert_eq!(
            (l.focus, l.panes[1].active),
            (0, 1),
            "the Graph shown beside, not the hidden one under dsm"
        );
    }

    #[test]
    fn closing_a_pane_or_its_last_tab_returns_to_one_pane() {
        let mut l = Layout::single("graph");
        l.split_right(Some("quality"), None);
        l.close_tab(1, 0);
        assert_eq!(tabs(&l), [["graph"]]);
        assert_eq!(l.focus, 0);
        l.split_right(Some("quality"), None);
        l.close_pane(0);
        assert_eq!(tabs(&l), [["quality"]]);
        assert_eq!(l.focus, 0);
        l.close_pane(0);
        assert_eq!(tabs(&l), [["quality"]], "one pane stays");
    }

    #[test]
    fn the_ratio_is_kept_within_bounds() {
        let mut l = Layout::single("graph");
        l.set_ratio(0.65);
        assert_eq!(l.ratio, 0.65);
        l.set_ratio(0.01);
        assert_eq!(l.ratio, RATIO_MIN);
        l.set_ratio(3.0);
        assert_eq!(l.ratio, RATIO_MAX);
        l.set_ratio(f32::NAN);
        assert_eq!(l.ratio, 0.5);
    }

    #[test]
    fn the_linked_view_is_the_instance_visible_in_the_other_pane() {
        let mut l = Layout::single("graph");
        let left = l.view(0).expect("tab").id;
        assert_eq!(l.linked(left), None, "one pane links nothing");
        let right = l.split_right(Some("graph"), None).expect("tab");
        assert_eq!(l.linked(left).map(|t| t.id), Some(right));
        assert_eq!(l.linked(right).map(|t| t.id), Some(left));
        let calls = l.open_tab("calls", None).expect("tab");
        assert_eq!(l.linked(left).map(|t| t.id), Some(calls));
        assert_eq!(l.linked(right), None, "a hidden tab links nothing");
    }

    #[test]
    fn repair_drops_unknown_and_duplicate_views_and_bad_indices() {
        let broken = Layout {
            panes: vec![
                Pane {
                    tabs: vec![
                        tab("graph", None),
                        tab("bogus", None),
                        tab("quality", None),
                        tab("graph", Some("core")),
                        tab(FILE, None),
                    ],
                    active: 9,
                },
                Pane {
                    tabs: vec![tab("quality", None)],
                    active: 0,
                },
                Pane {
                    tabs: vec![tab("calls", None)],
                    active: 0,
                },
            ],
            focus: 7,
            ratio: f32::INFINITY,
            next: 0,
            follow_left: false,
        };
        let l = repaired(broken);
        assert_eq!(
            tabs(&l),
            [vec!["graph", "quality", "graph"], vec!["calls"]],
            "a second Graph stays, a second Quality and a File without node go"
        );
        assert_eq!(ids(&l), [1, 2, 3, 4]);
        assert_eq!(l.next, 5);
        assert_eq!(l.panes[0].active, 2);
        assert_eq!(l.focus, 1);
        assert_eq!(l.ratio, 0.5);
        let empty = Layout {
            panes: Vec::new(),
            focus: 0,
            ratio: 0.5,
            next: 1,
            follow_left: false,
        };
        assert_eq!(repaired(empty), Layout::default());
    }

    /// A tab whose node the analysis no longer has is dropped; the document
    /// keeps a view.
    #[test]
    fn a_tab_of_a_missing_node_is_dropped() {
        let mut l = Layout::single("graph");
        let f = l.open_tab(FILE, Some("gone.rs")).expect("tab");
        drop_tab(&mut l, f);
        assert_eq!(tabs(&l), [["graph"]]);
        let mut l = Layout::single("graph");
        let f = l.split_right(Some(FILE), Some("gone.rs")).expect("tab");
        drop_tab(&mut l, f);
        assert_eq!(tabs(&l), [["graph"]], "its pane closes");
        let mut only = repaired(Layout {
            panes: vec![Pane {
                tabs: vec![tab(FILE, Some("gone.rs"))],
                active: 0,
            }],
            focus: 0,
            ratio: 0.5,
            next: 1,
            follow_left: false,
        });
        let id = only.view(0).expect("tab").id;
        drop_tab(&mut only, id);
        assert_eq!(tabs(&only), [["graph"]], "the last view becomes a Graph");
        assert_ne!(only.view(0).map(|t| t.id), Some(id));
        drop_tab(&mut only, 99);
        assert_eq!(tabs(&only), [["graph"]]);
    }

    #[test]
    fn a_link_runs_once_and_never_back() {
        let mut l = Layout::single("graph");
        let graph = l.view(0).expect("tab").id;
        let dsm = l.split_right(Some("dsm"), None).expect("tab");
        let mut seen = Vec::new();
        let linked = |id| l.linked(id).map(Tab::key);
        let ran = link::follow(linked(graph), |_, target| {
            seen.push(target);
            // Revealing in the target selects there, which links again.
            let back = link::follow(linked(target), |_, t| seen.push(t));
            assert!(!back, "no link back while one runs");
            assert!(link::running(), "the target sees the running link");
            assert!(!link::guarded(|| ()), "no follow while a link runs");
        });
        assert!(ran);
        assert_eq!(seen, [dsm]);
        assert!(!link::running());
        assert!(
            link::guarded(|| assert!(link::running())),
            "a follow runs alone"
        );
        assert!(
            link::follow(linked(dsm), |_, _| ()),
            "the guard is released"
        );
        assert!(!link::follow(None, |_, _| ()));
        let calls = tab("calls", None);
        assert!(
            !link::follow(Some(calls.key()), |_, _| ()),
            "Calls cannot reveal"
        );
        let file = tab(FILE, Some("a.rs"));
        assert!(
            !link::follow(Some(file.key()), |_, _| ()),
            "a File view keeps its node"
        );
    }
}
