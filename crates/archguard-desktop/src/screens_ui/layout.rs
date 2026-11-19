//! A layered left-to-right diagram per application: start screens (route
//! `/`) and the "every screen" source of layouts and menus in the first
//! column, each screen one column right of the nearest screen leading to
//! it, unresolved targets one column right of their source. Applications
//! are stacked as bands.

use std::collections::{BTreeMap, HashMap};

use archguard_engine::core::{Origin, Target, UiFlow, normalize_path_pattern};

use crate::diagram;

pub const CARD_W: f32 = 230.0;
pub const CARD_H: f32 = 66.0;
const GAP_X: f32 = 90.0;
const GAP_Y: f32 = 18.0;
const HEAD: f32 = 34.0;
// Room left of the first column for arrows within a column.
const PAD: f32 = 80.0;
const BAND_GAP: f32 = 22.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Node {
    /// Index into the flow's screens.
    Screen(usize),
    /// Layouts and menus of the band's application.
    Every,
    /// Index into the flow's transitions (the first with this source and text).
    Unresolved(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub node: Node,
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Arrow {
    pub path: String,
    pub unresolved: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Band {
    pub application: String,
    pub y: f32,
    pub h: f32,
}

#[derive(Debug, Default)]
pub struct Graph {
    pub cards: Vec<Card>,
    pub arrows: Vec<Arrow>,
    pub bands: Vec<Band>,
    pub width: f32,
    pub height: f32,
}

/// One application's nodes and edges.
struct App {
    nodes: Vec<Node>,
    edges: Vec<(Node, Node)>,
    starts: Vec<Node>,
}

fn app_of(flow: &UiFlow, i: usize) -> &str {
    flow.screens[i].application.as_deref().unwrap_or("")
}

/// The applications by name, each with its nodes and deduplicated edges.
fn applications(flow: &UiFlow) -> BTreeMap<&str, App> {
    let mut apps: BTreeMap<&str, App> = BTreeMap::new();
    let ids: HashMap<&str, usize> = (flow.screens.iter().enumerate())
        .map(|(i, s)| (s.id.as_str(), i))
        .collect();
    let mut order: Vec<usize> = (0..flow.screens.len()).collect();
    order.sort_by(|&a, &b| flow.screens[a].name.cmp(&flow.screens[b].name));
    for i in order {
        let app = apps.entry(app_of(flow, i)).or_insert_with(|| App {
            nodes: vec![],
            edges: vec![],
            starts: vec![],
        });
        app.nodes.push(Node::Screen(i));
        let route = flow.screens[i].route.as_deref().map(normalize_path_pattern);
        if route.as_deref() == Some("/") {
            app.starts.push(Node::Screen(i));
        }
    }
    let mut texts: HashMap<(Node, &str), usize> = HashMap::new();
    for (k, t) in flow.transitions.iter().enumerate() {
        let Some((from, name)) = source(flow, &ids, &t.from) else {
            continue;
        };
        let Some(app) = apps.get_mut(name) else {
            continue;
        };
        let to = match &t.target {
            Target::Screen(id) => match ids.get(id.as_str()) {
                Some(&j) if app_of(flow, j) == name => Node::Screen(j),
                _ => continue,
            },
            Target::Unresolved(text) => Node::Unresolved(*texts.entry((from, text)).or_insert(k)),
        };
        app.add(from, to);
    }
    apps
}

/// The node and application a transition starts at.
fn source<'a>(
    flow: &'a UiFlow,
    ids: &HashMap<&str, usize>,
    from: &'a Origin,
) -> Option<(Node, &'a str)> {
    match from {
        Origin::Screen(id) => ids
            .get(id.as_str())
            .map(|&i| (Node::Screen(i), app_of(flow, i))),
        Origin::AnyScreen { application } => Some((Node::Every, application.as_str())),
    }
}

impl App {
    /// An edge once, with its pseudo nodes.
    fn add(&mut self, from: Node, to: Node) {
        for n in [from, to] {
            if matches!(n, Node::Every | Node::Unresolved(_)) && !self.nodes.contains(&n) {
                self.nodes.push(n);
            }
        }
        if from != to && !self.edges.contains(&(from, to)) {
            self.edges.push((from, to));
        }
    }
}

/// Column of every node: breadth first from the start screens and the
/// "every screen" source; unreached screens after the last column.
fn columns(app: &App) -> HashMap<Node, usize> {
    let mut col: HashMap<Node, usize> = HashMap::new();
    let mut queue: Vec<Node> = app.starts.clone();
    if app.nodes.contains(&Node::Every) {
        queue.push(Node::Every);
    }
    for n in &queue {
        col.insert(*n, 0);
    }
    let mut at = 0;
    while at < queue.len() {
        let n = queue[at];
        at += 1;
        for (_, to) in app.edges.iter().filter(|(f, _)| *f == n) {
            if !col.contains_key(to) {
                col.insert(*to, col[&n] + 1);
                queue.push(*to);
            }
        }
    }
    let last = col.values().max().map_or(0, |m| m + 1);
    for n in app.nodes.iter().filter(|n| matches!(n, Node::Screen(_))) {
        col.entry(*n).or_insert(last);
    }
    for (from, to) in &app.edges {
        if matches!(to, Node::Unresolved(_)) && !col.contains_key(to) {
            col.insert(*to, col[from] + 1);
        }
    }
    col
}

/// The diagram of all applications.
pub fn layout(flow: &UiFlow) -> Graph {
    let mut g = Graph::default();
    let mut top = BAND_GAP;
    for (name, app) in applications(flow) {
        let col = columns(&app);
        let mut rows: HashMap<usize, usize> = HashMap::new();
        let mut at: HashMap<Node, (f32, f32)> = HashMap::new();
        let ordered = app
            .starts
            .iter()
            .chain(app.nodes.iter().filter(|n| !app.starts.contains(n)));
        for n in ordered {
            let c = col[n];
            let r = rows.entry(c).or_insert(0);
            let (x, y) = (
                PAD + c as f32 * (CARD_W + GAP_X),
                top + HEAD + *r as f32 * (CARD_H + GAP_Y),
            );
            *r += 1;
            at.insert(*n, (x, y));
            g.cards.push(Card { node: *n, x, y });
            g.width = g.width.max(x + CARD_W + PAD);
        }
        for (from, to) in &app.edges {
            g.arrows
                .push(arrow(at[from], at[to], matches!(to, Node::Unresolved(_))));
        }
        let most = rows.values().copied().max().unwrap_or(0) as f32;
        let h = HEAD + most * (CARD_H + GAP_Y) + 26.0;
        g.bands.push(Band {
            application: name.to_string(),
            y: top,
            h,
        });
        top += h + BAND_GAP;
    }
    g.height = top;
    g
}

/// A curve between the facing sides of two cards; within a column, along
/// their left sides. Unresolved targets (always a column right): dashed.
fn arrow(a: (f32, f32), b: (f32, f32), unresolved: bool) -> Arrow {
    let path = match diagram::ends(a, b, CARD_W, CARD_H) {
        (from, to) if unresolved => diagram::line(from, to, true).0,
        (from, to) => diagram::curve(from, to).0,
    };
    Arrow { path, unresolved }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::{Screen, ScreenKind, Transition};

    fn screen(id: &str, app: &str, route: &str) -> Screen {
        Screen {
            id: id.into(),
            name: id.into(),
            kind: ScreenKind::Page,
            route: Some(route.into()),
            application: Some(app.into()),
            file: format!("{id}.razor"),
            line: 1,
            access: None,
            source: "test".into(),
        }
    }

    fn go(from: Origin, target: Target) -> Transition {
        Transition {
            from,
            target,
            trigger: None,
            file: "x.razor".into(),
            line: 3,
            condition: None,
            source: "test".into(),
        }
    }

    fn at(g: &Graph, n: Node) -> (f32, f32) {
        let c = g.cards.iter().find(|c| c.node == n).expect("card");
        (c.x, c.y)
    }

    fn shop() -> UiFlow {
        let s = |id: &str| Origin::Screen(id.into());
        let t = |id: &str| Target::Screen(id.into());
        UiFlow {
            screens: vec![
                screen("home", "Web", "/"),
                screen("cart", "Web", "/cart"),
                screen("pay", "Web", "/pay"),
                screen("lost", "Web", "/lost"),
                screen("admin", "Admin", "/"),
            ],
            transitions: vec![
                go(s("home"), t("cart")),
                go(s("home"), t("cart")),
                go(s("cart"), t("pay")),
                go(s("pay"), t("home")),
                go(s("cart"), Target::Unresolved("@Url(x)".into())),
                go(
                    Origin::AnyScreen {
                        application: "Web".into(),
                    },
                    t("cart"),
                ),
                go(s("home"), t("admin")),
            ],
            ..UiFlow::default()
        }
    }

    #[test]
    fn screens_go_one_column_right_of_their_nearest_source() {
        let g = layout(&shop());
        assert_eq!(
            g.bands
                .iter()
                .map(|b| b.application.as_str())
                .collect::<Vec<_>>(),
            ["Admin", "Web"]
        );
        let web = g.bands[1].y + HEAD;
        let col = |n| ((at(&g, n).0 - PAD) / (CARD_W + GAP_X)).round() as usize;
        assert_eq!(at(&g, Node::Screen(0)), (PAD, web), "start screen first");
        assert_eq!(col(Node::Every), 0, "every screen beside the start");
        assert_eq!(col(Node::Screen(1)), 1);
        assert_eq!(col(Node::Screen(2)), 2);
        assert_eq!(col(Node::Unresolved(4)), 2, "right of its source");
        assert_eq!(col(Node::Screen(3)), 3, "unreached after the last column");
        assert_eq!(col(Node::Screen(4)), 0, "own band, own start");
    }

    #[test]
    fn edges_are_deduplicated_and_stay_in_their_application() {
        let g = layout(&shop());
        // home→cart once, cart→pay, pay→home, cart→unresolved, every→cart;
        // home→admin crosses applications and is left out.
        assert_eq!(g.arrows.len(), 5);
        assert_eq!(g.arrows.iter().filter(|a| a.unresolved).count(), 1);
        assert!(g.height > g.bands[1].y + g.bands[1].h);
        assert_eq!(g.width, PAD + 3.0 * (CARD_W + GAP_X) + CARD_W + PAD);
    }
}
