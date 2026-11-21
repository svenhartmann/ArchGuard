//! The model parts of the focus layout: the route tree rows, the focus
//! cards around the selected screen, its path from a start screen and the
//! "Open in browser" button.

use std::collections::BTreeMap;

use archguard_engine::core::{ScreenReach, UiFlow};
use archguard_engine::ui_flow_replay::{path_to, url_for};

use super::card::card;
use super::focus::{self, Link, Node, Slot};
use super::layout::Node as GraphNode;
use super::state::{Mode, ScreensState};
use super::tree::{self, NO_ROUTE};
use super::{inspector, words};
use crate::diagram::dash_rect;
use crate::tab_support::{Texts, model};
use crate::ui::{FlowArrow, FlowCard, FlowStep, FlowTreeRow, ScreensModel};

type Reaches = BTreeMap<String, ScreenReach>;

/// The route tree of the chosen application with the filter counts.
pub fn tree_rows(m: &mut ScreensModel, s: &ScreensState, reaches: &Reaches, texts: &Texts) {
    let flow = &s.sim;
    let t = tree::build(flow, &s.app, reaches, &s.search, s.filter, &s.folded);
    m.tree = model(t.rows.into_iter().map(|r| {
        let mut row = FlowTreeRow {
            depth: r.depth as i32,
            label: if r.key == NO_ROUTE {
                texts.t("Without a route").into()
            } else {
                r.label.as_str().into()
            },
            key: r.key.as_str().into(),
            group: !r.key.is_empty(),
            open: r.open,
            count: r.count as i32,
            screen: -1,
            state: 4,
            ..Default::default()
        };
        if let Some(i) = r.screen {
            let c = card(flow, GraphNode::Screen(i), reaches, texts);
            row.screen = i as i32;
            row.state = c.state;
            row.note = short_note(c.state, &c.note, texts).into();
            if c.title != r.label {
                row.name = c.title;
            }
            row.selected = s.selected == Some(i);
        }
        row
    }));
    let [all, denied, no_path] = t.counts.map(|n| n as i32);
    (m.count_all, m.count_denied, m.count_no_path) = (all, denied, no_path);
}

/// The tree's status note: the reason only where it is not "reached".
fn short_note(state: i32, note: &str, texts: &Texts) -> String {
    match state {
        2 => note.to_string(),
        3 => texts.t("no known path").into(),
        _ => String::new(),
    }
}

/// The selected screen's focus cards (one or two rings) as the canvas.
pub fn focus_cards(m: &mut ScreensModel, s: &ScreensState, reaches: &Reaches, texts: &Texts) {
    let Some(i) = s.selected else { return };
    let flow = &s.sim;
    let steps = if s.mode == Mode::TwoSteps { 2 } else { 1 };
    let h = focus::hood(flow, i, steps);
    let p = focus::place(&h);
    let cards = p.boxes.iter().map(|b| {
        let mut c = match b.slot {
            Slot::Heading(k, n) => heading(k, n, texts),
            Slot::Center => center(s, i, reaches, texts),
            Slot::From(k) => neighbour(flow, &h.from[k], reaches, texts),
            Slot::To(k) => neighbour(flow, &h.to[k], reaches, texts),
            Slot::From2(k) => neighbour(flow, &h.from2[k], reaches, texts),
            Slot::To2(k) => neighbour(flow, &h.to2[k], reaches, texts),
        };
        (c.x, c.y, c.w, c.h) = (b.x, b.y, b.w, b.h);
        if !c.border.is_empty() {
            c.border = dash_rect(b.w, b.h).into();
        }
        c
    });
    m.cards = model(cards);
    m.arrows = model(p.arrows.into_iter().map(|a| FlowArrow {
        path: a.path.into(),
        unresolved: a.unresolved,
    }));
    (m.width, m.height) = (p.width, p.height);
}

fn heading(k: usize, n: usize, texts: &Texts) -> FlowCard {
    let key = [
        "Two steps back",
        "Comes from",
        "Leads to",
        "Two steps ahead",
    ][k];
    FlowCard {
        kind: 3,
        state: 4,
        screen: -1,
        group: -1,
        title: format!("{} ({n})", texts.t(key)).into(),
        ..Default::default()
    }
}

/// The selected screen: status, route, what it needs and from where.
fn center(s: &ScreensState, i: usize, reaches: &Reaches, texts: &Texts) -> FlowCard {
    let flow = s.flow().unwrap_or(&s.sim);
    let mut c = card(&s.sim, GraphNode::Screen(i), reaches, texts);
    let screen = &flow.screens[i];
    c.selected = true;
    // The detail line says what it needs; the status says it in one word.
    c.note = match c.state {
        2 => texts.t("denied for this user").into(),
        _ => status(c.state, &c.note, texts).into(),
    };
    let needs = match &screen.access {
        archguard_engine::core::AccessState::Protected(a) => texts.f(
            "needs: {name}",
            &[("name", short(&words::condition(texts, a)))],
        ),
        archguard_engine::core::AccessState::Open => texts.t("needs no sign-in or role").into(),
        archguard_engine::core::AccessState::Undeclared => texts.t("access not declared").into(),
    };
    let needs = if screen.weak {
        format!("{needs} ({})", texts.t("weakly protected"))
    } else {
        needs
    };
    let from = inspector::meaning(s, screen, texts).2;
    c.detail = format!("{needs}\n{from}").trim_end().into();
    c
}

/// At most 120 characters; the details panel has the whole text.
fn short(text: &str) -> String {
    match text.char_indices().nth(120) {
        Some((at, _)) => format!("{} …", &text[..at]),
        None => text.to_string(),
    }
}

/// The status of a card in words, "reached" included.
fn status(state: i32, note: &str, texts: &Texts) -> String {
    match state {
        0 => texts.t("reached").into(),
        1 => texts.f("reached {name}", &[("name", note.to_string())]),
        _ => note.to_string(),
    }
}

fn quoted(triggers: &[String]) -> String {
    let q: Vec<String> = triggers.iter().map(|t| format!("“{t}”")).collect();
    q.join(", ")
}

fn neighbour(flow: &UiFlow, l: &Link, reaches: &Reaches, texts: &Texts) -> FlowCard {
    let how = match (l.triggers.is_empty(), l.layout) {
        (true, false) => texts.t("no trigger read").to_string(),
        (true, true) => texts.t("from the layout or menu").to_string(),
        (false, false) => quoted(&l.triggers),
        (false, true) => texts.f(
            "from the layout or menu: {name}",
            &[("name", quoted(&l.triggers))],
        ),
    };
    match &l.node {
        Node::Screen(j) => {
            let mut c = card(flow, GraphNode::Screen(*j), reaches, texts);
            c.note = status(c.state, &c.note, texts).into();
            c.file = how.into();
            c
        }
        Node::Every(app) => FlowCard {
            kind: 1,
            state: 4,
            screen: -1,
            group: -1,
            title: texts
                .f("every screen of {name}", &[("name", app.clone())])
                .into(),
            file: how.into(),
            ..Default::default()
        },
        Node::Unreadable { first, count } => {
            let mut c = card(flow, GraphNode::Unresolved(*first), reaches, texts);
            c.title = (texts.plural(
                *count,
                "{count} target not readable",
                "{count} targets not readable",
            ))
            .into();
            c
        }
    }
}

/// The shortest path from a start screen, by the replay's rule.
pub fn path(m: &mut ScreensModel, s: &ScreensState, texts: &Texts) {
    let Some(i) = s.selected else { return };
    let flow = &s.sim;
    let id = &flow.screens[i].id;
    let step = |j: usize, trigger: Option<&String>| FlowStep {
        label: flow.screens[j].name.as_str().into(),
        trigger: trigger.cloned().unwrap_or_default().into(),
        screen: j as i32,
    };
    let index = |id: &str| flow.screens.iter().position(|x| x.id == id);
    match path_to(flow, id, &s.assignment()) {
        None => {
            m.path_note = texts
                .t("No known path from a start screen for this user.")
                .into()
        }
        Some(steps) if steps.is_empty() => {
            m.path_note = texts.t("This is a start screen.").into();
        }
        Some(steps) => {
            // The path's first screen is the start the first step leaves;
            // a layout step leaves every screen, so it starts at a start screen.
            let first = match &steps[0].from {
                archguard_engine::core::Origin::Screen(from) => index(from),
                archguard_engine::core::Origin::AnyScreen { .. } => start_of(s, steps[0]),
            };
            let mut chips: Vec<FlowStep> = first.map(|j| step(j, None)).into_iter().collect();
            for t in &steps {
                if let archguard_engine::core::Target::Screen(to) = &t.target
                    && let Some(j) = index(to)
                {
                    chips.push(step(j, t.trigger.as_ref()));
                }
            }
            m.path = model(chips);
        }
    }
    match s.config.as_ref() {
        None => {
            m.browser_tip = texts
                .t("The project configuration could not be read.")
                .into()
        }
        Some(config) => match url_for(flow, id, config, &BTreeMap::new()) {
            Ok(url) => {
                m.browser_ok = true;
                m.browser_tip = url.into();
            }
            // The engine's hint names the CLI flag; the window has none.
            Err(e) => m.browser_tip = e.split(": pass --param").next().unwrap_or(&e).into(),
        },
    }
    m.browser_note = s.browser_note.as_str().into();
}

/// A start screen of the step's application.
fn start_of(s: &ScreensState, t: &archguard_engine::core::Transition) -> Option<usize> {
    let archguard_engine::core::Origin::AnyScreen { application } = &t.from else {
        return None;
    };
    let starts = archguard_engine::core::start_screen_ids(&s.sim);
    (s.sim.screens.iter()).position(|x| {
        starts.contains(&x.id.as_str()) && x.application.as_deref().unwrap_or("") == application
    })
}
