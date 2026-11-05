//! The tab state and the web's selection rules (renderCalls,
//! renderCallTree, renderCallRankings, prepareCallCycles and the click
//! handlers in viewer.js and viewer_call_*.js).

use std::collections::{HashMap, HashSet};

use archguard_engine::core::CallAnalysis;

use super::cycles::{self, CycleModel};
use super::model::{self, Element, ElementKind, Filter, Index, Labels, Relation};
use super::rankings::{self, Rankings, Row, Sort};
use super::tree::{self, CallTree, Kind};

pub const PAGE: usize = 100;
pub const TREE_PAGE: usize = 200;
pub const RANK_PAGE: usize = 20;
pub const CYCLE_PAGE: usize = 20;
pub const MEMBER_PAGE: usize = 50;
pub const WITNESS_PAGE: usize = 20;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Connections,
    Rankings,
    Cycles,
}

/// What the window shows, derived by [`State::refresh`].
#[derive(Default)]
pub struct Derived {
    /// Active connections after the element filter (the list).
    pub relations: Vec<Relation>,
    pub selected: Option<Relation>,
    pub tree: CallTree,
    pub visible: Vec<usize>,
    pub shown: HashSet<usize>,
    pub rankings: Rankings,
    pub rank_rows: Vec<Row>,
    pub rank_missing: usize,
    /// Indices of the matching cycle groups and the selected one.
    pub groups: Vec<usize>,
    pub group: Option<usize>,
    pub cycle_relations: Vec<Relation>,
}

/// Paging limits of the cycle detail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CycleLimits {
    pub groups: usize,
    pub members: usize,
    pub edges: usize,
    pub witness: usize,
}

impl Default for CycleLimits {
    fn default() -> Self {
        Self {
            groups: CYCLE_PAGE,
            members: MEMBER_PAGE,
            edges: MEMBER_PAGE,
            witness: WITNESS_PAGE,
        }
    }
}

pub struct State {
    pub idx: Index,
    pub cycles: CycleModel,
    pub labels: Labels,
    pub filter: Filter,
    pub mode: Mode,
    pub relation: Option<String>,
    pub element: Option<Element>,
    pub limit: usize,
    pub site_limit: usize,
    pub expanded: HashSet<String>,
    pub tree_limit: usize,
    tree_relation: Option<String>,
    tree_scope: String,
    pub rank_method: bool,
    pub rank_sort: Sort,
    pub rank_limit: usize,
    rank_order: String,
    pub cycle_kind: ElementKind,
    pub cycle_selected: Option<String>,
    cycle_group: Option<String>,
    cycle_scope: String,
    pub cycle: CycleLimits,
    /// Open state of evidence groups by method-pair key, per relation.
    pub open_groups: HashMap<String, bool>,
    pub open_suggestions: HashSet<usize>,
    evidence_relation: Option<String>,
    pub analysis_open: bool,
    pub rank_help: bool,
    pub cycle_help: bool,
    pub view: Derived,
}

impl State {
    pub fn new(a: CallAnalysis) -> Self {
        let idx = Index::new(a);
        let filter = Filter {
            package: idx.packages(None).into_iter().next(),
            resolution: idx.default_resolution(),
            ..Default::default()
        };
        let mut s = Self {
            cycles: cycles::compute(&idx),
            idx,
            labels: Labels::default(),
            filter,
            mode: Mode::Connections,
            relation: None,
            element: None,
            limit: PAGE,
            site_limit: PAGE,
            expanded: HashSet::new(),
            tree_limit: TREE_PAGE,
            tree_relation: None,
            tree_scope: String::new(),
            rank_method: false,
            rank_sort: Sort::Incoming,
            rank_limit: RANK_PAGE,
            rank_order: String::new(),
            cycle_kind: ElementKind::Class,
            cycle_selected: None,
            cycle_group: None,
            cycle_scope: String::new(),
            cycle: CycleLimits::default(),
            open_groups: HashMap::new(),
            open_suggestions: HashSet::new(),
            evidence_relation: None,
            analysis_open: false,
            rank_help: false,
            cycle_help: false,
            view: Derived::default(),
        };
        s.refresh();
        s
    }

    /// Picked values that left the scope fall back as in the web.
    fn normalize(&mut self) {
        if let Some(l) = &self.filter.language
            && !self.idx.languages().contains(l)
        {
            self.filter.language = None;
        }
        let packages = self.idx.packages(self.filter.language.as_deref());
        if let Some(p) = &self.filter.package
            && !packages.contains(p)
        {
            self.filter.package = packages.into_iter().next();
        }
    }

    fn prepare_cycles(&mut self, d: &mut Derived) {
        let groups = self.cycles.groups(self.cycle_kind);
        d.groups = cycles::matching(groups, &self.idx, &self.filter, &self.labels);
        let scope = format!("{:?}\u{0}{}", self.cycle_kind, self.filter.scope());
        if scope != self.cycle_scope {
            self.cycle_scope = scope;
            self.cycle.groups = CYCLE_PAGE;
        }
        d.group = d
            .groups
            .iter()
            .copied()
            .find(|&g| Some(&groups[g].id) == self.cycle_selected.as_ref())
            .or(d.groups.first().copied());
        let id = d.group.map(|g| groups[g].id.clone());
        self.cycle_selected = id.clone();
        if id != self.cycle_group {
            self.cycle_group = id;
            self.cycle = CycleLimits {
                groups: self.cycle.groups,
                ..Default::default()
            };
            self.element = None;
            self.relation = None;
            self.site_limit = PAGE;
            self.tree_limit = TREE_PAGE;
        }
        d.cycle_relations = d
            .group
            .map(|g| cycles::relations(&groups[g], &self.idx, &self.labels))
            .unwrap_or_default();
    }

    /// The connections ignoring the element filter.
    pub fn base(&self, d: &Derived) -> Vec<Relation> {
        if self.mode == Mode::Cycles {
            d.cycle_relations.clone()
        } else {
            model::relations(&self.idx, &self.filter, &self.labels)
        }
    }

    fn select(&mut self, d: &mut Derived, base: &[Relation]) {
        let cycles = self.mode == Mode::Cycles;
        d.relations = if cycles {
            base.to_vec()
        } else {
            model::focus(base, &self.idx.a, self.element.as_ref())
        };
        let focused: Vec<&Relation> = match (&self.element, cycles) {
            (Some(el), true) => d
                .relations
                .iter()
                .filter(|r| r.calls.iter().any(|&i| el.matches(&self.idx.a.calls[i])))
                .collect(),
            _ => d.relations.iter().collect(),
        };
        let selected = focused
            .iter()
            .find(|r| Some(&r.key) == self.relation.as_ref())
            .or(focused.first())
            .copied()
            .or(d.relations.first())
            .cloned();
        self.relation = selected.as_ref().map(|r| r.key.clone());
        if !cycles && let Some(sel) = &selected {
            let pos = d
                .relations
                .iter()
                .position(|r| r.key == sel.key)
                .unwrap_or(0);
            self.limit = self.limit.max((pos + 1).div_ceil(PAGE) * PAGE);
        }
        d.selected = selected;
    }

    fn element_key(&self) -> String {
        match &self.element {
            Some(e) if e.kind == ElementKind::Class => tree::key("class", &e.id),
            Some(e) => tree::key("method", &e.id),
            None => String::new(),
        }
    }

    fn prepare_tree(&mut self, d: &mut Derived, base: &[Relation]) {
        let scope = self.filter.scope();
        if scope != self.tree_scope {
            self.tree_scope = scope;
            self.tree_limit = TREE_PAGE;
            self.tree_relation = None;
        }
        d.tree = tree::build(base, &self.idx, &self.labels);
        let key = d.selected.as_ref().map(|r| r.key.clone());
        if let Some(sel) = &d.selected
            && self.tree_relation != key
        {
            d.tree
                .open_path(&tree::key("class", &sel.from), &mut self.expanded);
            if let Some(to) = &sel.to {
                d.tree
                    .open_path(&tree::key("class", to), &mut self.expanded);
            }
            if sel.resolution != "project" {
                d.tree
                    .open_path(&tree::key("group", &sel.resolution), &mut self.expanded);
            }
        }
        self.tree_relation = key;
        d.visible = d.tree.visible(&self.expanded);
        let mut reserved = vec![self.element_key()];
        if let Some(sel) = &d.selected {
            reserved.push(tree::key("class", &sel.from));
            reserved.extend(sel.to.as_ref().map(|t| tree::key("class", t)));
            reserved.push(tree::key(&sel.resolution, &sel.key));
        }
        d.shown = d.tree.shown(&d.visible, &reserved, self.tree_limit);
    }

    fn prepare_rankings(&mut self, d: &mut Derived, base: &[Relation]) {
        if !self.rank_method && self.rank_sort.measured() {
            self.rank_sort = Sort::Incoming;
        }
        d.rankings =
            rankings::compute(base.iter().flat_map(|r| r.calls.iter().copied()), &self.idx);
        let order = format!(
            "{}\u{0}{}\u{0}{:?}",
            self.filter.scope(),
            self.rank_method,
            self.rank_sort
        );
        if order != self.rank_order {
            self.rank_order = order;
            self.rank_limit = RANK_PAGE;
        }
        let rows = if self.rank_sort.measured() {
            let (rows, missing) =
                rankings::measurements(&d.rankings.methods, &self.idx, &self.filter);
            d.rank_missing = missing;
            rows
        } else if self.rank_method {
            d.rankings.methods.clone()
        } else {
            d.rankings.classes.clone()
        };
        d.rank_rows = rankings::ordered(rows, self.rank_sort);
    }

    /// Apply the web's rules after any change and derive the view.
    pub fn refresh(&mut self) {
        self.normalize();
        let mut d = Derived::default();
        if self.mode == Mode::Cycles {
            self.prepare_cycles(&mut d);
        }
        let base = self.base(&d);
        self.select(&mut d, &base);
        self.prepare_tree(&mut d, &base);
        if self.mode == Mode::Rankings {
            self.prepare_rankings(&mut d, &base);
        }
        let key = d.selected.as_ref().map(|r| r.key.clone());
        if key != self.evidence_relation {
            self.evidence_relation = key;
            self.open_groups.clear();
            self.open_suggestions.clear();
        }
        self.view = d;
    }

    pub fn group(&self) -> Option<&cycles::Group> {
        self.view
            .group
            .map(|g| &self.cycles.groups(self.cycle_kind)[g])
    }

    pub fn is_cycle_member(&self, kind: Kind, id: &str) -> bool {
        let want = match self.cycle_kind {
            ElementKind::Class => Kind::Class,
            ElementKind::Method => Kind::Method,
        };
        self.mode == Mode::Cycles
            && kind == want
            && self
                .group()
                .is_some_and(|g| g.members.iter().any(|m| m == id))
    }
}

#[cfg(test)]
mod tests {
    use super::super::model::fixture;
    use super::*;

    #[test]
    fn opens_the_first_package_and_its_first_connection() {
        let s = State::new(fixture::analysis());
        assert_eq!(s.filter.package.as_deref(), Some("blue"));
        assert_eq!(s.view.relations.len(), 3);
        let sel = s.view.selected.as_ref().unwrap();
        assert_eq!(
            (sel.from.as_str(), sel.to.as_deref()),
            ("blue.Blue", Some("red.Red"))
        );
        // The path to caller and target is open in the tree.
        let labels: Vec<String> = s
            .view
            .visible
            .iter()
            .map(|&n| s.view.tree.nodes[n].label.clone().unwrap_or_default())
            .collect();
        assert_eq!(
            labels,
            vec![
                "",
                "blue",
                "Blue",
                "name()",
                "Util",
                "green",
                "red",
                "Red",
                "describe()"
            ]
        );
    }
}
