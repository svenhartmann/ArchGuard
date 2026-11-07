//! The tab's actions: the web's click and change handlers on [`State`]
//! (viewer.js, viewer_calls_tree.js, viewer_call_rankings_ui.js,
//! viewer_call_cycles_ui.js). Call [`State::refresh`] afterwards.

use super::model::{self, Element, ElementKind};
use super::rankings::Sort;
use super::state::{Mode, PAGE, State, TREE_PAGE};
use super::tree::{self, Kind};

pub fn reset_pages(s: &mut State) {
    s.limit = PAGE;
    s.site_limit = PAGE;
}

pub fn set_language(s: &mut State, language: Option<String>) {
    s.filter.language = language;
    s.relation = None;
    s.element = None;
    reset_pages(s);
}

pub fn set_package(s: &mut State, package: Option<String>) {
    s.filter.package = package;
    s.relation = None;
    s.element = None;
    reset_pages(s);
}

pub fn set_query(s: &mut State, q: String) {
    s.filter.query = q;
    reset_pages(s);
}

pub fn set_resolution(s: &mut State, r: model::Resolution) {
    s.filter.resolution = r;
    s.relation = None;
    reset_pages(s);
}

pub fn toggle_internal(s: &mut State) {
    s.filter.internal = !s.filter.internal;
    reset_pages(s);
}

pub fn set_mode(s: &mut State, mode: Mode) {
    if (s.mode == Mode::Cycles) != (mode == Mode::Cycles) {
        s.element = None;
        s.relation = None;
        s.site_limit = PAGE;
    }
    s.mode = mode;
}

pub fn select_element(s: &mut State, kind: ElementKind, id: &str) {
    s.element = Some(Element {
        kind,
        id: id.to_string(),
    });
    reset_pages(s);
}

pub fn select_relation(s: &mut State, key: String) {
    s.relation = Some(key);
    s.site_limit = PAGE;
}

/// A tree label: select a type or method, open a leaf's connection,
/// else toggle the node.
pub fn tree_select(s: &mut State, node: usize) {
    let Some(n) = s.view.tree.nodes.get(node) else {
        return;
    };
    match n.kind {
        Kind::Class => select_element(s, ElementKind::Class, &n.id.clone()),
        Kind::Method => select_element(s, ElementKind::Method, &n.id.clone()),
        Kind::Leaf => {
            let key = n.relation.clone().unwrap_or_default();
            s.element = None;
            select_relation(s, key);
        }
        _ => tree_toggle(s, node),
    }
}

pub fn tree_toggle(s: &mut State, node: usize) {
    let Some(n) = s.view.tree.nodes.get(node) else {
        return;
    };
    if !s.expanded.remove(&n.key) {
        s.expanded.insert(n.key.clone());
    }
}

pub fn tree_expand(s: &mut State, expand: bool) {
    s.expanded.clear();
    if expand {
        s.expanded
            .extend(s.view.tree.has_children().map(|n| n.key.clone()));
    }
    s.tree_limit = TREE_PAGE;
}

pub fn open_in_tree(s: &mut State, kind: ElementKind, id: &str) {
    let k = match kind {
        ElementKind::Class => tree::key("class", id),
        ElementKind::Method => tree::key("method", id),
    };
    s.view.tree.open_path(&k, &mut s.expanded);
}

/// `openClassMetricCalls` (the World's "Calls"): the connections of class
/// `id` in its language and package, unfiltered, with the class as the
/// element and its tree path open. `false` for a class without calls
/// declaration (the state stays as it was).
pub fn focus_class(s: &mut State, id: &str) -> bool {
    let Some(cls) = s.idx.class(Some(id)) else {
        return false;
    };
    let language = Some(s.idx.language(cls)).filter(|l| !l.is_empty());
    s.filter = model::Filter {
        language,
        package: Some(cls.package.clone()),
        query: String::new(),
        resolution: model::Resolution::All,
        internal: true,
    };
    s.relation = None;
    set_mode(s, Mode::Connections);
    // The tree of the new scope holds the class.
    s.refresh();
    open_in_tree(s, ElementKind::Class, id);
    select_element(s, ElementKind::Class, id);
    true
}

/// `selectCallRanking`: the first connection of the element in the
/// ranked direction, the element filter and its tree path.
pub fn rank_select(s: &mut State, i: usize) {
    let Some(row) = s.view.rank_rows.get(i) else {
        return;
    };
    let id = row.id.clone();
    let kind = if s.rank_method {
        ElementKind::Method
    } else {
        ElementKind::Class
    };
    let (incoming, outgoing) = (s.rank_sort == Sort::Incoming, s.rank_sort == Sort::Outgoing);
    let a = &s.idx.a;
    let hit = |i: &usize| {
        let c = &a.calls[*i];
        let (from, to) = match kind {
            ElementKind::Class => (Some(c.from_class.as_str()), c.to_class.as_deref()),
            ElementKind::Method => (c.from_method.as_deref(), c.to_method.as_deref()),
        };
        (!outgoing && to == Some(id.as_str())) || (!incoming && from == Some(id.as_str()))
    };
    s.relation = model::relations(&s.idx, &s.filter, &s.labels)
        .into_iter()
        .find(|r| r.calls.iter().any(hit))
        .map(|r| r.key);
    open_in_tree(s, kind, &id);
    select_element(s, kind, &id);
}

pub fn set_cycle_kind(s: &mut State, kind: ElementKind) {
    s.cycle_kind = kind;
    s.element = None;
    s.relation = None;
}

pub fn cycle_select(s: &mut State, i: usize) {
    let groups = s.cycles.groups(s.cycle_kind);
    if let Some(&g) = s.view.groups.get(i) {
        s.cycle_selected = Some(groups[g].id.clone());
        s.element = None;
        s.relation = None;
    }
}

pub fn cycles_show_all(s: &mut State) {
    s.filter.language = None;
    s.filter.package = None;
    s.filter.query.clear();
    s.element = None;
    s.relation = None;
}

/// A cycle connection (path step or internal connection).
pub fn cycle_edge(s: &mut State, from: &str, to: &str) {
    let key = s
        .view
        .cycle_relations
        .iter()
        .find(|r| r.cycle.as_ref().is_some_and(|(f, t)| f == from && t == to))
        .map(|r| r.key.clone());
    if let Some(key) = key {
        s.element = None;
        select_relation(s, key);
    }
}

#[cfg(test)]
mod tests {
    use super::super::model::fixture;
    use super::*;

    #[test]
    fn an_element_filters_the_connections() {
        let mut s = State::new(fixture::analysis());
        select_element(&mut s, ElementKind::Class, "blue.Util");
        s.refresh();
        assert_eq!(s.view.relations.len(), 1);
        assert_eq!(s.view.selected.as_ref().unwrap().from, "blue.Util");
    }

    #[test]
    fn rankings_select_the_element_and_its_connection() {
        let mut s = State::new(fixture::analysis());
        set_mode(&mut s, Mode::Rankings);
        s.refresh();
        let ids: Vec<&str> = s.view.rank_rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["red.Red", "blue.Blue", "green.Leaf"]);
        rank_select(&mut s, 0);
        s.refresh();
        assert_eq!(
            s.element,
            Some(Element {
                kind: ElementKind::Class,
                id: "red.Red".into()
            })
        );
        assert_eq!(
            s.view.selected.as_ref().unwrap().key,
            r#"["blue.Blue","red.Red","project"]"#
        );
        s.rank_method = true;
        s.rank_sort = Sort::Cyclomatic;
        s.refresh();
        assert_eq!(s.view.rank_rows.len(), 2);
        s.rank_method = false;
        s.refresh();
        assert_eq!(s.rank_sort, Sort::Incoming);
    }

    #[test]
    fn cycles_mode_lists_groups_and_their_connections() {
        let mut s = State::new(fixture::analysis());
        set_mode(&mut s, Mode::Cycles);
        s.refresh();
        assert_eq!(s.view.groups.len(), 1);
        assert_eq!(s.view.relations.len(), 2);
        cycle_edge(&mut s, "red.Red", "blue.Blue");
        s.refresh();
        assert_eq!(s.view.selected.as_ref().unwrap().from, "red.Red");
        assert!(s.is_cycle_member(Kind::Class, "blue.Blue"));
        set_cycle_kind(&mut s, ElementKind::Method);
        s.refresh();
        assert_eq!(s.view.relations.len(), 3);
    }

    /// The World's "Calls" (`openClassMetricCalls`): the class's language
    /// and package, all its connections, the class as the element and its
    /// tree path open.
    #[test]
    fn focus_class_opens_its_scope_and_selects_it() {
        let mut s = State::new(fixture::analysis());
        set_mode(&mut s, Mode::Rankings);
        set_package(&mut s, Some("green".into()));
        set_query(&mut s, "Leaf".into());
        s.filter.internal = false;
        s.refresh();
        assert!(focus_class(&mut s, "red.Red"));
        s.refresh();
        assert_eq!(s.mode, Mode::Connections);
        assert_eq!(s.filter.language.as_deref(), Some("java"));
        assert_eq!(s.filter.package.as_deref(), Some("red"));
        assert_eq!(s.filter.query, "");
        assert_eq!(s.filter.resolution, model::Resolution::All);
        assert!(s.filter.internal);
        assert_eq!(
            s.element,
            Some(Element {
                kind: ElementKind::Class,
                id: "red.Red".into()
            })
        );
        let row = s.view.visible.iter().find(|&&n| {
            let n = &s.view.tree.nodes[n];
            n.kind == Kind::Class && n.id == "red.Red"
        });
        assert!(row.is_some(), "the class is a visible tree row");
        assert!(!focus_class(&mut s, "no.Such"), "unknown classes are ignored");
        assert_eq!(s.filter.package.as_deref(), Some("red"));
    }

    #[test]
    fn expand_and_collapse_all() {
        let mut s = State::new(fixture::analysis());
        tree_expand(&mut s, false);
        s.refresh();
        assert_eq!(s.view.visible.len(), 1);
        tree_expand(&mut s, true);
        s.refresh();
        assert_eq!(s.view.visible.len(), s.view.tree.nodes.len());
    }
}
