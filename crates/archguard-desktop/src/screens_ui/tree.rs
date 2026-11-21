//! The route tree of one application: screens grouped by route segment
//! (`/app` › `:appName` › `content`), a screen whose route has child routes
//! (a layout route) is the group row itself, screens without a route are
//! listed by name under one group. Search and the status filter keep the
//! groups above every match open.

use std::collections::{BTreeMap, BTreeSet};

use archguard_engine::core::{Screen, ScreenReach, UiFlow};

/// The status filter chips: all, denied, no known path.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    All,
    Denied,
    NoPath,
}

/// The group key of the screens without a route.
pub const NO_ROUTE: &str = "\u{0}no-route";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub depth: usize,
    /// The route segment, or the screen name for a screen without a route.
    pub label: String,
    /// Index into the flow's screens; none for a pure group.
    pub screen: Option<usize>,
    /// Group key (route prefix); empty for a leaf.
    pub key: String,
    pub open: bool,
    /// Screens below a group row.
    pub count: usize,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Tree {
    pub rows: Vec<Row>,
    /// Screens matching the search: all, denied, no known path.
    pub counts: [usize; 3],
}

#[derive(Default)]
struct Node {
    screens: Vec<usize>,
    children: BTreeMap<String, Node>,
}

/// Denied, no known path (also no start screen), or reached.
pub fn class(reach: Option<&ScreenReach>) -> Filter {
    match reach {
        Some(ScreenReach::Denied) => Filter::Denied,
        Some(ScreenReach::Sure | ScreenReach::Possible | ScreenReach::Undeclared) => Filter::All,
        _ => Filter::NoPath,
    }
}

pub fn build(
    flow: &UiFlow,
    app: &str,
    reaches: &BTreeMap<String, ScreenReach>,
    search: &str,
    filter: Filter,
    folded: &BTreeSet<String>,
) -> Tree {
    let needle = search.trim().to_lowercase();
    let mut tree = Tree::default();
    let mut root = Node::default();
    let mut loose = Node::default();
    let found = |s: &Screen| {
        let hay = format!("{}\n{}", s.route.as_deref().unwrap_or(""), s.name);
        s.application.as_deref().unwrap_or("") == app && hay.to_lowercase().contains(&needle)
    };
    for (i, s) in flow.screens.iter().enumerate().filter(|(_, s)| found(s)) {
        let c = class(reaches.get(&s.id));
        tree.counts[0] += 1;
        tree.counts[1] += usize::from(c == Filter::Denied);
        tree.counts[2] += usize::from(c == Filter::NoPath);
        if filter != Filter::All && c != filter {
            continue;
        }
        match &s.route {
            None => loose.screens.push(i),
            Some(route) => root.at(route).screens.push(i),
        }
    }
    // Searching or filtering opens every group, so each match is in view.
    let all_open = !needle.is_empty() || filter != Filter::All;
    let open = |key: &str| all_open || !folded.contains(key);
    let by_name =
        |v: &mut Vec<usize>| v.sort_by(|&a, &b| flow.screens[a].name.cmp(&flow.screens[b].name));
    if !root.screens.is_empty() {
        // Routed `/`: the root entries come first.
        by_name(&mut root.screens);
        for &i in &root.screens {
            tree.rows.push(leaf(0, "/".into(), i));
        }
    }
    walk(&root, "", 0, &open, &mut tree.rows, flow);
    if !loose.screens.is_empty() {
        by_name(&mut loose.screens);
        let is_open = open(NO_ROUTE);
        tree.rows.push(Row {
            depth: 0,
            label: String::new(),
            screen: None,
            key: NO_ROUTE.into(),
            open: is_open,
            count: loose.screens.len(),
        });
        if is_open {
            for &i in &loose.screens {
                tree.rows.push(leaf(1, flow.screens[i].name.clone(), i));
            }
        }
    }
    tree
}

impl Node {
    /// The node of a route, created with its parents.
    fn at(&mut self, route: &str) -> &mut Node {
        let mut node = self;
        for seg in route.split('/').filter(|x| !x.is_empty()) {
            node = node.children.entry(seg.to_string()).or_default();
        }
        node
    }
}

fn leaf(depth: usize, label: String, screen: usize) -> Row {
    Row {
        depth,
        label,
        screen: Some(screen),
        key: String::new(),
        open: false,
        count: 0,
    }
}

fn count(n: &Node) -> usize {
    n.screens.len() + n.children.values().map(count).sum::<usize>()
}

fn walk(
    node: &Node,
    prefix: &str,
    depth: usize,
    open: &dyn Fn(&str) -> bool,
    rows: &mut Vec<Row>,
    flow: &UiFlow,
) {
    for (seg, child) in &node.children {
        let key = format!("{prefix}/{seg}");
        let mut screens = child.screens.clone();
        screens.sort_by(|&a, &b| flow.screens[a].name.cmp(&flow.screens[b].name));
        if child.children.is_empty() {
            for i in screens {
                rows.push(leaf(depth, seg.clone(), i));
            }
            continue;
        }
        // A layout route: the group row carries its screen; more screens of
        // the same route follow as entries inside the group.
        let is_open = open(&key);
        rows.push(Row {
            depth,
            label: seg.clone(),
            screen: screens.first().copied(),
            key: key.clone(),
            open: is_open,
            count: count(child),
        });
        if is_open {
            for &i in screens.iter().skip(1) {
                rows.push(leaf(depth + 1, flow.screens[i].name.clone(), i));
            }
            walk(child, &key, depth + 1, open, rows, flow);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::ScreenKind;

    fn screen(id: &str, app: &str, route: Option<&str>) -> Screen {
        Screen {
            weak: false,
            id: id.into(),
            name: id.into(),
            kind: ScreenKind::Page,
            route: route.map(String::from),
            application: Some(app.into()),
            file: format!("{id}.ts"),
            line: 1,
            access: archguard_engine::core::AccessState::Undeclared,
            source: "test".into(),
        }
    }

    fn flow() -> UiFlow {
        UiFlow {
            screens: vec![
                screen("Home", "web", Some("/")),
                screen("AppArea", "web", Some("/app")),
                screen("Apps", "web", Some("/app/:appName")),
                screen("Content", "web", Some("/app/:appName/content")),
                screen("Schemas", "web", Some("/app/:appName/schemas")),
                screen("Login", "web", Some("/login")),
                screen("Settings", "web", None),
                screen("Other", "admin", Some("/x")),
            ],
            ..UiFlow::default()
        }
    }

    fn reaches(f: &UiFlow) -> BTreeMap<String, ScreenReach> {
        let mut r: BTreeMap<String, ScreenReach> = (f.screens.iter())
            .map(|s| (s.id.clone(), ScreenReach::Sure))
            .collect();
        r.insert("Schemas".into(), ScreenReach::Denied);
        r.insert(
            "Login".into(),
            ScreenReach::NoKnownPath {
                unresolved_transitions_in_application: 0,
            },
        );
        r.insert("Settings".into(), ScreenReach::NoStartScreen);
        r
    }

    fn shape(t: &Tree, f: &UiFlow) -> Vec<String> {
        (t.rows.iter())
            .map(|r| {
                let name = r.screen.map_or("-", |i| f.screens[i].name.as_str());
                let mark = if r.key.is_empty() {
                    ""
                } else if r.open {
                    "▾"
                } else {
                    "▸"
                };
                format!("{}{mark}{} [{name}]", "  ".repeat(r.depth), r.label)
            })
            .collect()
    }

    #[test]
    fn routes_group_by_segment_and_layout_routes_are_groups() {
        let f = flow();
        let t = build(&f, "web", &reaches(&f), "", Filter::All, &BTreeSet::new());
        assert_eq!(
            shape(&t, &f),
            [
                "/ [Home]",
                "▾app [AppArea]",
                "  ▾:appName [Apps]",
                "    content [Content]",
                "    schemas [Schemas]",
                "login [Login]",
                "▾ [-]",
                "  Settings [Settings]",
            ]
        );
        assert_eq!(t.rows[1].count, 4, "the group counts its own screen too");
        assert_eq!(t.rows[1].key, "/app");
        assert_eq!(t.rows[6].key, NO_ROUTE, "screens without a route by name");
        assert_eq!(t.counts, [7, 1, 2], "other applications stay out");
    }

    #[test]
    fn folded_groups_hide_their_entries() {
        let f = flow();
        let folded = BTreeSet::from(["/app/:appName".to_string()]);
        let t = build(&f, "web", &reaches(&f), "", Filter::All, &folded);
        let s = shape(&t, &f);
        assert_eq!(s[2], "  ▸:appName [Apps]");
        assert_eq!(s[3], "login [Login]");
    }

    #[test]
    fn filters_count_and_keep_the_groups_above_matches_open() {
        let f = flow();
        let folded = BTreeSet::from(["/app".to_string()]);
        let r = reaches(&f);
        let t = build(&f, "web", &r, "", Filter::Denied, &folded);
        assert_eq!(
            shape(&t, &f),
            ["▾app [-]", "  ▾:appName [-]", "    schemas [Schemas]"],
            "a group without a match carries no screen"
        );
        let t = build(&f, "web", &r, "", Filter::NoPath, &folded);
        assert_eq!(
            shape(&t, &f),
            ["login [Login]", "▾ [-]", "  Settings [Settings]"]
        );
        let t = build(&f, "web", &r, "CONT", Filter::All, &folded);
        assert_eq!(
            shape(&t, &f),
            ["▾app [-]", "  ▾:appName [-]", "    content [Content]"],
            "search by route, case-insensitive, opens folded groups"
        );
        assert_eq!(t.counts, [1, 0, 0], "counts follow the search");
        let t = build(&f, "web", &r, "settings", Filter::All, &folded);
        assert_eq!(t.counts, [1, 0, 1], "search by name");
    }

    #[test]
    fn possible_counts_as_reached_and_no_reach_as_no_path() {
        assert_eq!(class(Some(&ScreenReach::Denied)), Filter::Denied);
        assert_eq!(class(Some(&ScreenReach::Possible)), Filter::All);
        assert_eq!(class(None), Filter::NoPath);
    }
}
