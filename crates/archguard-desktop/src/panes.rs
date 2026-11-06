//! Panes of a document (ADR 0002 decision 6): one or two panes side by
//! side, each with view tabs. Independent of Slint; `panes_ui.rs` shows
//! the layout and `pane_store.rs` stores it per project.
//!
//! A view (Start, Graph, Matrix, …) is open at most once per document, in
//! one tab of one pane. Its state stays one struct per view kind and
//! document (`documents.rs`): the Slint views read their data from
//! window-wide properties and globals, so two visible instances of the
//! same view would show, and fight over, one state. Opening a view that is
//! already open selects its tab instead; "Split right" with such a view
//! moves its tab to the right pane.

pub mod link;

/// Every view, in the order of the view bar.
pub const VIEWS: [&str; 8] = [
    "start",
    "architecture",
    "graph",
    "dsm",
    "calls",
    "services",
    "quality",
    "security",
];

/// The order in which a pane that needs a view takes one not open yet.
const FREE_ORDER: [&str; 8] = [
    "dsm",
    "graph",
    "start",
    "architecture",
    "quality",
    "calls",
    "services",
    "security",
];

/// The left pane's share of the width, at least and at most.
pub const RATIO_MIN: f32 = 0.2;
pub const RATIO_MAX: f32 = 0.8;

#[derive(Debug, Clone, PartialEq)]
pub struct Pane {
    /// View ids from [`VIEWS`], in tab order; never empty in a layout.
    pub tabs: Vec<String>,
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
}

impl Default for Layout {
    fn default() -> Self {
        Layout::single("graph")
    }
}

pub fn known(view: &str) -> bool {
    VIEWS.contains(&view)
}

impl Layout {
    /// One pane showing `view` (the Graph for an unknown id).
    pub fn single(view: &str) -> Self {
        let view = if known(view) { view } else { "graph" };
        Layout {
            panes: vec![Pane {
                tabs: vec![view.to_string()],
                active: 0,
            }],
            focus: 0,
            ratio: 0.5,
        }
    }

    pub fn is_split(&self) -> bool {
        self.panes.len() > 1
    }

    /// The view shown in `pane`.
    pub fn view(&self, pane: usize) -> Option<&str> {
        let p = self.panes.get(pane)?;
        p.tabs.get(p.active).map(String::as_str)
    }

    /// The view shown in the focused pane.
    pub fn focused(&self) -> &str {
        self.view(self.focus).unwrap_or("graph")
    }

    /// Pane and tab index of `view`, if it is open.
    pub fn find(&self, view: &str) -> Option<(usize, usize)> {
        self.panes
            .iter()
            .enumerate()
            .find_map(|(p, pane)| Some((p, pane.tabs.iter().position(|t| t == view)?)))
    }

    /// The view shown beside `view` when it is visible in a split layout:
    /// where a selection in `view` is revealed.
    pub fn linked(&self, view: &str) -> Option<&str> {
        let (p, _) = self.find(view).filter(|_| self.is_split())?;
        (self.view(p) == Some(view))
            .then(|| self.view(1 - p))
            .flatten()
    }

    /// The view bar: select `view`'s tab, or show it in the focused tab.
    pub fn navigate(&mut self, view: &str) {
        if let Some((p, t)) = self.find(view) {
            return self.select_tab(p, t);
        }
        if known(view) {
            let pane = &mut self.panes[self.focus];
            pane.tabs[pane.active] = view.to_string();
        }
    }

    /// "Open in new tab": a tab after the focused pane's active one.
    pub fn open_tab(&mut self, view: &str) {
        if let Some((p, t)) = self.find(view) {
            return self.select_tab(p, t);
        }
        if known(view) {
            self.insert(self.focus, view);
        }
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

    /// "Split right": show `view` (else the focused tab of a pane with
    /// several, else a view not open yet) in a right pane, and focus it.
    pub fn split_right(&mut self, view: Option<&str>) {
        if self.is_split() {
            if let Some(v) = view.filter(|v| known(v)) {
                self.remove(v);
                self.insert(1, v);
                self.fill();
            }
            self.focus = 1;
            return;
        }
        let first = &self.panes[0];
        let v = match view.filter(|v| known(v)) {
            Some(v) => v.to_string(),
            None if first.tabs.len() > 1 => first.tabs[first.active].clone(),
            None => self.free(),
        };
        self.remove(&v);
        self.panes.push(Pane {
            tabs: vec![v],
            active: 0,
        });
        self.fill();
        self.focus = 1;
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
        }
    }

    pub fn set_ratio(&mut self, ratio: f32) {
        self.ratio = if ratio.is_finite() {
            ratio.clamp(RATIO_MIN, RATIO_MAX)
        } else {
            0.5
        };
    }

    /// A layout read from the settings with unknown or repeated views,
    /// empty or extra panes and out-of-range indices dropped.
    pub fn repaired(self) -> Layout {
        let mut seen: Vec<String> = Vec::new();
        let mut panes = Vec::new();
        for mut p in self.panes {
            p.tabs.retain(|t| {
                let keep = known(t) && !seen.contains(t);
                if keep {
                    seen.push(t.clone());
                }
                keep
            });
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
            focus: self.focus.min(panes.len() - 1),
            panes,
            ratio: 0.5,
        };
        l.set_ratio(self.ratio);
        l
    }

    /// Insert `view` after the active tab of `pane` and select it.
    fn insert(&mut self, pane: usize, view: &str) {
        let p = &mut self.panes[pane];
        let at = if p.tabs.is_empty() { 0 } else { p.active + 1 };
        p.tabs.insert(at, view.to_string());
        p.active = at;
        self.focus = pane;
    }

    /// Remove `view`'s tab, leaving its pane possibly empty for [`fill`].
    fn remove(&mut self, view: &str) {
        if let Some((p, t)) = self.find(view) {
            let pane = &mut self.panes[p];
            pane.tabs.remove(t);
            if pane.active > t || pane.active == pane.tabs.len() {
                pane.active = pane.active.saturating_sub(1);
            }
        }
    }

    /// Give every empty pane a view not open yet.
    fn fill(&mut self) {
        for p in 0..self.panes.len() {
            if self.panes[p].tabs.is_empty() {
                let v = self.free();
                self.panes[p] = Pane {
                    tabs: vec![v],
                    active: 0,
                };
            }
        }
    }

    fn free(&self) -> String {
        FREE_ORDER
            .iter()
            .find(|v| self.find(v).is_none())
            .unwrap_or(&"graph")
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tabs(l: &Layout) -> Vec<Vec<&str>> {
        l.panes
            .iter()
            .map(|p| p.tabs.iter().map(String::as_str).collect())
            .collect()
    }

    fn visible(l: &Layout) -> Vec<&str> {
        (0..l.panes.len()).filter_map(|p| l.view(p)).collect()
    }

    #[test]
    fn a_new_layout_is_one_pane_with_one_view() {
        let l = Layout::single("dsm");
        assert_eq!(tabs(&l), [["dsm"]]);
        assert_eq!(l.focused(), "dsm");
        assert!(!l.is_split());
        assert_eq!(Layout::default().focused(), "graph");
    }

    #[test]
    fn the_view_bar_replaces_the_focused_view_or_selects_its_tab() {
        let mut l = Layout::single("graph");
        l.navigate("quality");
        assert_eq!(tabs(&l), [["quality"]], "replaces in place");
        l.open_tab("graph");
        assert_eq!(tabs(&l), [["quality", "graph"]]);
        assert_eq!(l.focused(), "graph");
        l.navigate("quality");
        assert_eq!(tabs(&l), [["quality", "graph"]], "an open view is selected");
        assert_eq!(l.focused(), "quality");
    }

    #[test]
    fn opening_a_tab_adds_it_after_the_active_one_once() {
        let mut l = Layout::single("graph");
        l.open_tab("dsm");
        l.select_tab(0, 0);
        l.open_tab("calls");
        assert_eq!(tabs(&l), [["graph", "calls", "dsm"]]);
        l.open_tab("dsm");
        assert_eq!(tabs(&l), [["graph", "calls", "dsm"]], "no second tab");
        assert_eq!(l.focused(), "dsm");
        l.open_tab("nonsense");
        assert_eq!(l.panes[0].tabs.len(), 3, "unknown views are ignored");
    }

    #[test]
    fn closing_tabs_keeps_a_neighbour_and_never_the_last_view() {
        let mut l = Layout::single("graph");
        l.open_tab("dsm");
        l.open_tab("calls");
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
        l.split_right(None);
        assert_eq!(tabs(&l), [["graph"], ["dsm"]]);
        assert_eq!(l.focus, 1);
        assert!(l.is_split());
        // Splitting again only focuses the right pane.
        l.focus(0);
        l.split_right(None);
        assert_eq!(tabs(&l), [["graph"], ["dsm"]]);
        assert_eq!(l.focus, 1);
    }

    #[test]
    fn split_right_moves_the_active_tab_of_a_pane_with_several() {
        let mut l = Layout::single("graph");
        l.open_tab("architecture");
        l.split_right(None);
        assert_eq!(tabs(&l), [["graph"], ["architecture"]]);
        assert_eq!(visible(&l), ["graph", "architecture"]);
    }

    #[test]
    fn a_view_is_open_once_per_document_and_moves_between_panes() {
        let mut l = Layout::single("graph");
        // Opening the only view to the side leaves another one behind.
        l.split_right(Some("graph"));
        assert_eq!(tabs(&l), [["dsm"], ["graph"]]);
        l.focus(0);
        l.split_right(Some("dsm"));
        assert_eq!(tabs(&l), [vec!["start"], vec!["graph", "dsm"]]);
        l.focus(0);
        l.navigate("graph");
        assert_eq!(l.focus, 1, "the view bar goes to the pane showing it");
        assert_eq!(l.focused(), "graph");
        let all: Vec<&str> = tabs(&l).concat();
        let mut unique = all.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(all.len(), unique.len(), "{all:?}");
    }

    #[test]
    fn closing_a_pane_or_its_last_tab_returns_to_one_pane() {
        let mut l = Layout::single("graph");
        l.split_right(Some("quality"));
        l.close_tab(1, 0);
        assert_eq!(tabs(&l), [["graph"]]);
        assert_eq!(l.focus, 0);
        l.split_right(Some("quality"));
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
    fn the_linked_view_is_the_one_visible_in_the_other_pane() {
        let mut l = Layout::single("graph");
        assert_eq!(l.linked("graph"), None, "one pane links nothing");
        l.split_right(Some("dsm"));
        assert_eq!(l.linked("graph"), Some("dsm"));
        assert_eq!(l.linked("dsm"), Some("graph"));
        l.open_tab("calls");
        assert_eq!(l.linked("graph"), Some("calls"));
        assert_eq!(l.linked("dsm"), None, "a hidden tab links nothing");
    }

    #[test]
    fn repair_drops_unknown_and_duplicate_views_and_bad_indices() {
        let broken = Layout {
            panes: vec![
                Pane {
                    tabs: vec!["graph".into(), "bogus".into(), "dsm".into()],
                    active: 9,
                },
                Pane {
                    tabs: vec!["dsm".into()],
                    active: 0,
                },
                Pane {
                    tabs: vec!["calls".into()],
                    active: 0,
                },
            ],
            focus: 7,
            ratio: f32::INFINITY,
        };
        let l = broken.repaired();
        assert_eq!(tabs(&l), [vec!["graph", "dsm"], vec!["calls"]]);
        assert_eq!(l.panes[0].active, 1);
        assert_eq!(l.focus, 1);
        assert_eq!(l.ratio, 0.5);
        let empty = Layout {
            panes: Vec::new(),
            focus: 0,
            ratio: 0.5,
        };
        assert_eq!(empty.repaired(), Layout::default());
    }

    #[test]
    fn a_link_runs_once_and_never_back() {
        let mut l = Layout::single("graph");
        l.split_right(Some("dsm"));
        let mut seen = Vec::new();
        let ran = link::follow(l.linked("graph"), |target| {
            seen.push(target.to_string());
            // Revealing in the target selects there, which links again.
            let back = link::follow(l.linked(target), |t| seen.push(t.to_string()));
            assert!(!back, "no link back while one runs");
        });
        assert!(ran);
        assert_eq!(seen, ["dsm"]);
        assert!(
            link::follow(l.linked("dsm"), |_| ()),
            "the guard is released"
        );
        assert!(!link::follow(None, |_| ()));
        assert!(!link::follow(Some("calls"), |_| ()), "Calls cannot reveal");
    }
}
