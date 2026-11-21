//! The focus view of one screen: where it comes from, where it leads, and
//! with two steps the next ring on both sides; placed as columns
//! (two steps back, comes from, the screen, leads to, two steps ahead).

use archguard_engine::core::{Origin, Target, Transition, UiFlow};

use super::layout::Arrow;
use crate::diagram;

pub const W: f32 = 200.0;
pub const H: f32 = 80.0;
pub const CENTER_W: f32 = 270.0;
pub const CENTER_H: f32 = 200.0;
const GAP_X: f32 = 46.0;
const GAP_Y: f32 = 12.0;
pub const HEAD_H: f32 = 24.0;
const TOP: f32 = HEAD_H + 12.0;
const PAD: f32 = 12.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Screen(usize),
    /// Every screen of an application: its layouts and menus.
    Every(String),
    /// The unresolved targets of the screen: the first transition, the count.
    Unreadable {
        first: usize,
        count: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub node: Node,
    /// Trigger labels, each once, in flow order.
    pub triggers: Vec<String>,
    /// Starts at every screen of the application (layout or menu).
    pub layout: bool,
    /// The ring-one link this ring-two link hangs on.
    pub parent: usize,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Hood {
    pub from: Vec<Link>,
    pub to: Vec<Link>,
    pub from2: Vec<Link>,
    pub to2: Vec<Link>,
}

fn app(flow: &UiFlow, i: usize) -> &str {
    flow.screens[i].application.as_deref().unwrap_or("")
}

fn index(flow: &UiFlow, id: &str) -> Option<usize> {
    flow.screens.iter().position(|s| s.id == id)
}

/// The transition starts at screen `i` itself (false) or at every screen of
/// its application (true); none if neither.
fn leaves(flow: &UiFlow, t: &Transition, i: usize) -> Option<bool> {
    match &t.from {
        Origin::Screen(id) => (*id == flow.screens[i].id).then_some(false),
        Origin::AnyScreen { application } => (application == app(flow, i)).then_some(true),
    }
}

fn add(links: &mut Vec<Link>, node: Node, trigger: Option<&String>, layout: bool, parent: usize) {
    let at = match links.iter().position(|l| l.node == node) {
        Some(at) => at,
        None => {
            links.push(Link {
                node,
                triggers: vec![],
                layout,
                parent,
            });
            links.len() - 1
        }
    };
    let l = &mut links[at];
    if let Some(t) = trigger.filter(|t| !l.triggers.contains(t)) {
        l.triggers.push(t.clone());
    }
}

/// The resolved targets of `i`: its own transitions, with `layout` also
/// those of every screen of its application, and the unresolved ones.
fn targets(flow: &UiFlow, i: usize, layout: bool, parent: usize) -> (Vec<Link>, Option<Node>) {
    let mut links = vec![];
    let (mut first, mut count) = (None, 0);
    // Own links first, then the layout's.
    for pass in [false, true].into_iter().take(1 + usize::from(layout)) {
        for (k, t) in flow.transitions.iter().enumerate() {
            if leaves(flow, t, i) != Some(pass) {
                continue;
            }
            match &t.target {
                Target::Screen(id) => {
                    let Some(j) = index(flow, id).filter(|&j| j != i) else {
                        continue;
                    };
                    // A target of an own link keeps that card.
                    if !(pass && links.iter().any(|l: &Link| l.node == Node::Screen(j))) {
                        add(
                            &mut links,
                            Node::Screen(j),
                            t.trigger.as_ref(),
                            pass,
                            parent,
                        );
                    }
                }
                Target::Unresolved(_) => {
                    first.get_or_insert(k);
                    count += 1;
                }
            }
        }
    }
    (links, first.map(|first| Node::Unreadable { first, count }))
}

/// The sources leading to `i`: screens and "every screen of" sources.
fn sources(flow: &UiFlow, i: usize, parent: usize) -> Vec<Link> {
    let mut links = vec![];
    let id = &flow.screens[i].id;
    for t in &flow.transitions {
        if t.target != Target::Screen(id.clone()) {
            continue;
        }
        let (node, layout) = match &t.from {
            Origin::Screen(from) => match index(flow, from) {
                Some(k) if k != i => (Node::Screen(k), false),
                _ => continue,
            },
            Origin::AnyScreen { application } => (Node::Every(application.clone()), true),
        };
        add(&mut links, node, t.trigger.as_ref(), layout, parent);
    }
    links
}

/// The neighbourhood of screen `i`: one ring, or two with `steps` 2. The
/// second ring follows only screens' own links (not the layout's, which
/// every screen of the application shares) and repeats no card.
pub fn hood(flow: &UiFlow, i: usize, steps: usize) -> Hood {
    let (mut to, unreadable) = targets(flow, i, true, 0);
    let from = sources(flow, i, 0);
    let mut h = Hood {
        from,
        ..Hood::default()
    };
    if steps >= 2 {
        let center = Node::Screen(i);
        h.to2 = ring_two(&to, &center, |j, p| targets(flow, j, false, p).0);
        h.from2 = ring_two(&h.from, &center, |k, p| sources(flow, k, p));
    }
    if let Some(node) = unreadable {
        to.push(Link {
            node,
            triggers: vec![],
            layout: false,
            parent: 0,
        });
    }
    h.to = to;
    h
}

/// The links of each screen of `ring` (hung on its index), each node once
/// and none of `ring` or the centre.
fn ring_two(ring: &[Link], center: &Node, next: impl Fn(usize, usize) -> Vec<Link>) -> Vec<Link> {
    let mut out: Vec<Link> = vec![];
    for (p, l) in ring.iter().enumerate() {
        let Node::Screen(j) = l.node else { continue };
        for link in next(j, p) {
            let seen = link.node == *center
                || ring.iter().any(|x| x.node == link.node)
                || out.iter().any(|x| x.node == link.node);
            if !seen {
                out.push(link);
            }
        }
    }
    out
}

/// What a placed box shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// A column heading: 0 two steps back, 1 comes from, 2 leads to,
    /// 3 two steps ahead; with the column's count.
    Heading(usize, usize),
    Center,
    From(usize),
    To(usize),
    From2(usize),
    To2(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    pub slot: Slot,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

#[derive(Debug, Default)]
pub struct Placement {
    pub boxes: Vec<Placed>,
    pub arrows: Vec<Arrow>,
    pub width: f32,
    pub height: f32,
}

/// Columns left to right; the screen at the top of its column, so long
/// columns scroll without moving it.
pub fn place(h: &Hood) -> Placement {
    let mut p = Placement::default();
    let mut x = PAD;
    fn column(
        p: &mut Placement,
        x: &mut f32,
        heading: usize,
        links: &[Link],
        slot: fn(usize) -> Slot,
    ) -> Vec<(f32, f32)> {
        let x0 = *x;
        if links.is_empty() && matches!(heading, 0 | 3) {
            return Vec::new();
        }
        p.boxes.push(Placed {
            slot: Slot::Heading(heading, links.len()),
            x: x0,
            y: 0.0,
            w: W,
            h: HEAD_H,
        });
        let mut at = Vec::new();
        for (k, _) in links.iter().enumerate() {
            let y = TOP + k as f32 * (H + GAP_Y);
            p.boxes.push(Placed {
                slot: slot(k),
                x: x0,
                y,
                w: W,
                h: H,
            });
            at.push((x0, y));
            p.height = p.height.max(y + H);
        }
        *x += W + GAP_X;
        at
    }
    let back2 = column(&mut p, &mut x, 0, &h.from2, Slot::From2);
    let back = column(&mut p, &mut x, 1, &h.from, Slot::From);
    let center = (x, TOP);
    p.boxes.push(Placed {
        slot: Slot::Center,
        x,
        y: TOP,
        w: CENTER_W,
        h: CENTER_H,
    });
    p.height = p.height.max(TOP + CENTER_H);
    x += CENTER_W + GAP_X;
    let ahead = column(&mut p, &mut x, 2, &h.to, Slot::To);
    let ahead2 = column(&mut p, &mut x, 3, &h.to2, Slot::To2);
    let mid = |(x, y): (f32, f32), w: f32, hh: f32, right: bool| {
        (if right { x + w } else { x }, y + hh / 2.0)
    };
    let mut arrow = |a: (f32, f32), b: (f32, f32), unresolved: bool| {
        let path = if unresolved {
            diagram::line(a, b, true).0
        } else {
            diagram::curve(a, b).0
        };
        p.arrows.push(Arrow { path, unresolved });
    };
    for &b in &back {
        arrow(
            mid(b, W, H, true),
            mid(center, CENTER_W, CENTER_H, false),
            false,
        );
    }
    for (k, l) in h.to.iter().enumerate() {
        let unresolved = matches!(l.node, Node::Unreadable { .. });
        arrow(
            mid(center, CENTER_W, CENTER_H, true),
            mid(ahead[k], W, H, false),
            unresolved,
        );
    }
    for (k, l) in h.from2.iter().enumerate() {
        arrow(
            mid(back2[k], W, H, true),
            mid(back[l.parent], W, H, false),
            false,
        );
    }
    for (k, l) in h.to2.iter().enumerate() {
        arrow(
            mid(ahead[l.parent], W, H, true),
            mid(ahead2[k], W, H, false),
            false,
        );
    }
    p.width = x - GAP_X + PAD;
    p.height += PAD;
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::{Screen, ScreenKind};

    fn screen(id: &str) -> Screen {
        Screen {
            weak: false,
            id: id.into(),
            name: id.into(),
            kind: ScreenKind::Page,
            route: Some(format!("/{id}")),
            application: Some("web".into()),
            file: format!("{id}.ts"),
            line: 1,
            access: archguard_engine::core::AccessState::Undeclared,
            source: "test".into(),
        }
    }

    fn go(from: Origin, to: Target, trigger: &str) -> Transition {
        Transition {
            weak: false,
            from,
            target: to,
            trigger: (!trigger.is_empty()).then(|| trigger.to_string()),
            file: "x.ts".into(),
            line: 1,
            condition: archguard_engine::core::AccessState::Undeclared,
            source: "test".into(),
        }
    }

    /// home → list → item → edit; a menu on every screen leads to help;
    /// list has two unreadable links; other → list.
    fn flow() -> UiFlow {
        let s = |id: &str| Origin::Screen(id.into());
        let t = |id: &str| Target::Screen(id.into());
        let every = Origin::AnyScreen {
            application: "web".into(),
        };
        UiFlow {
            screens: ["home", "list", "item", "edit", "help", "other"]
                .map(screen)
                .to_vec(),
            transitions: vec![
                go(s("home"), t("list"), "Lists"),
                go(s("list"), t("item"), "Open"),
                go(s("list"), t("item"), "Open"),
                go(s("list"), t("item"), "Details"),
                go(s("item"), t("edit"), "Edit"),
                go(every, t("help"), "Help"),
                go(s("list"), Target::Unresolved("url()".into()), ""),
                go(s("list"), Target::Unresolved("other()".into()), ""),
                go(s("other"), t("list"), ""),
                go(s("list"), t("list"), "Reload"),
            ],
            ..UiFlow::default()
        }
    }

    fn nodes(links: &[Link]) -> Vec<Node> {
        links.iter().map(|l| l.node.clone()).collect()
    }

    #[test]
    fn one_step_shows_sources_targets_and_one_unreadable_card() {
        let h = hood(&flow(), 1, 1);
        assert_eq!(nodes(&h.from), [Node::Screen(0), Node::Screen(5)]);
        assert_eq!(h.from[0].triggers, ["Lists"]);
        assert_eq!(
            nodes(&h.to),
            [
                Node::Screen(2),
                Node::Screen(4),
                Node::Unreadable { first: 6, count: 2 }
            ],
            "own links, then the menu's, the unreadable ones last; no self link"
        );
        assert_eq!(h.to[0].triggers, ["Open", "Details"], "each trigger once");
        assert!(!h.to[0].layout && h.to[1].layout);
        assert!(h.from2.is_empty() && h.to2.is_empty());
    }

    #[test]
    fn every_screen_sources_show_as_their_own_card() {
        let h = hood(&flow(), 4, 1);
        assert_eq!(nodes(&h.from), [Node::Every("web".into())]);
        assert!(h.from[0].layout);
        assert_eq!(h.from[0].triggers, ["Help"]);
    }

    #[test]
    fn two_steps_add_the_next_ring_without_repeating_cards() {
        let h = hood(&flow(), 1, 2);
        assert_eq!(
            nodes(&h.to2),
            [Node::Screen(3)],
            "item → edit; help leads nowhere"
        );
        assert_eq!(h.to2[0].parent, 0);
        assert!(h.from2.is_empty(), "home and other have no sources");
        let h = hood(&flow(), 2, 2);
        assert_eq!(nodes(&h.from), [Node::Screen(1)]);
        assert_eq!(
            nodes(&h.from2),
            [Node::Screen(0), Node::Screen(5)],
            "the sources of list"
        );
        assert_eq!(nodes(&h.to2), Vec::<Node>::new(), "edit leads nowhere");
    }

    #[test]
    fn placement_puts_columns_left_to_right() {
        let h = hood(&flow(), 1, 2);
        let p = place(&h);
        let x = |slot| {
            p.boxes
                .iter()
                .find(|b| b.slot == slot)
                .map(|b| b.x)
                .unwrap()
        };
        assert!(x(Slot::From(0)) < x(Slot::Center));
        assert!(x(Slot::Center) < x(Slot::To(0)));
        assert!(x(Slot::To(0)) < x(Slot::To2(0)));
        assert!(
            !p.boxes.iter().any(|b| b.slot == Slot::Heading(0, 0)),
            "empty ring two"
        );
        assert_eq!(p.arrows.len(), 2 + 3 + 1);
        assert_eq!(p.arrows.iter().filter(|a| a.unresolved).count(), 1);
    }
}
