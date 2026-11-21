//! One card of the Screens diagrams: a screen with its reach for the
//! simulated user, "every screen" of an application, or the unreadable
//! targets of one source.

use std::collections::BTreeMap;

use archguard_engine::core::{Screen, ScreenReach, UiFlow};

use super::layout::{CARD_H, CARD_W, Node};
use super::words;
use crate::diagram::dash_rect;
use crate::tab_support::Texts;
use crate::ui::FlowCard;

fn at(file: &str, line: u32) -> String {
    format!("{file}:{line}")
}

/// A card without its place.
pub fn card(
    flow: &UiFlow,
    node: Node,
    reaches: &BTreeMap<String, ScreenReach>,
    texts: &Texts,
) -> FlowCard {
    let mut c = FlowCard {
        w: CARD_W,
        h: CARD_H,
        screen: -1,
        group: -1,
        state: 4,
        ..Default::default()
    };
    match node {
        Node::Screen(i) => screen_card(&mut c, &flow.screens[i], reaches, texts, i),
        Node::Every => {
            c.kind = 1;
            c.title = texts.t("Every screen").into();
            c.file = texts.t("layouts and menus").into();
        }
        Node::Unresolved(k) => {
            let n = super::inspector::unreadable(flow, k).count();
            c.kind = 2;
            c.group = k as i32;
            c.title = texts
                .plural(
                    n,
                    "{count} target not readable",
                    "{count} targets not readable",
                )
                .into();
            c.file = texts.t("click for the list with file and line").into();
        }
    }
    if c.kind == 2 || c.state == 3 {
        c.border = dash_rect(CARD_W, CARD_H).into();
    }
    c
}

fn screen_card(
    c: &mut FlowCard,
    screen: &Screen,
    reaches: &BTreeMap<String, ScreenReach>,
    texts: &Texts,
    i: usize,
) {
    c.screen = i as i32;
    c.title = screen.name.as_str().into();
    c.route = screen.route.clone().unwrap_or_default().into();
    c.file = at(&screen.file, screen.line).into();
    c.path = screen.file.as_str().into();
    c.line = screen.line as i32;
    let (state, note) = match reaches.get(&screen.id) {
        Some(ScreenReach::Sure) if screen.weak => (0, texts.t("weakly protected").into()),
        Some(ScreenReach::Sure) => (0, String::new()),
        // never shown as reached: nothing was declared
        Some(ScreenReach::Undeclared) => (3, texts.t("access not declared").into()),
        Some(ScreenReach::Possible) => (1, texts.t("only if an open condition holds").into()),
        Some(ScreenReach::Denied) => {
            let needs = words::access(texts, screen.access.condition());
            (2, texts.f("needs: {name}", &[("name", needs)]))
        }
        Some(ScreenReach::NoKnownPath {
            unresolved_transitions_in_application: n,
        }) => (
            3,
            texts.plural(
                *n,
                "no known path, {count} link in this app unresolved",
                "no known path, {count} links in this app unresolved",
            ),
        ),
        Some(ScreenReach::NoStartScreen) | None => {
            (3, texts.t("no start screen in this app").into())
        }
    };
    c.state = state;
    c.note = note.into();
}
