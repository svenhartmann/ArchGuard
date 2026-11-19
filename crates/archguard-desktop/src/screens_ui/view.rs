//! The Screens tab's window model from its state: the left panel, the
//! diagram with each screen's reach for the simulated user, the details.

use std::collections::BTreeMap;

use archguard_engine::core::{
    DiagnosticSeverity, Screen, ScreenReach, UiFlow, UiFlowFindingKind, screen_reaches,
};

use super::layout::{self, CARD_H, CARD_W, Node};
use super::state::ScreensState;
use super::{inspector, words};
use crate::diagram::dash_rect;
use crate::tab_support::{Texts, model};
use crate::ui::{FlowArrow, FlowBand, FlowCard, FlowRole, FlowSwitch, ScreensModel};

pub fn build(s: &ScreensState, texts: &Texts) -> ScreensModel {
    let flow = match &*s.flow {
        Ok(f) => f,
        Err(e) => {
            return ScreensModel {
                state: 2,
                error: e.into(),
                ..Default::default()
            };
        }
    };
    let sum = &s.summary;
    let kind = |k| s.findings.iter().filter(|f| f.kind == k).count() as i32;
    let mut m = ScreensModel {
        state: 1,
        screens: sum.screens as i32,
        resolved: sum.transitions_resolved as i32,
        navigations: (sum.transitions_resolved + sum.transitions_unresolved) as i32,
        actions: sum.actions as i32,
        actions_unresolved: sum.actions_unresolved as i32,
        protected: sum.endpoints_by_access.protected as i32,
        open: sum.endpoints_by_access.open as i32,
        undeclared: sum.endpoints_by_access.undeclared as i32,
        complete: sum.complete,
        gaps: (flow.diagnostics.iter())
            .filter(|d| d.severity == DiagnosticSeverity::Gap)
            .count() as i32,
        refused: kind(UiFlowFindingKind::ShownButRefused),
        unprotected: kind(UiFlowFindingKind::HiddenButUnprotected),
        roles: model(role_rows(texts, &s.roles, s.role)),
        switches: model(s.atoms.iter().zip(&s.switches).map(|(a, v)| FlowSwitch {
            label: a.name.as_str().into(),
            policy: a.policy,
            value: match v {
                None => 0,
                Some(true) => 1,
                Some(false) => 2,
            },
        })),
        inspector: inspector::build(s, flow, texts),
        ..Default::default()
    };
    diagram(&mut m, s, flow, texts);
    m
}

/// The simulated users: anonymous, signed in without a role, each role.
pub fn role_rows(texts: &Texts, roles: &[String], chosen: usize) -> Vec<FlowRole> {
    let fixed = [texts.t("Anonymous"), texts.t("Signed in without a role")];
    let named = (roles.iter()).map(|r| texts.f("Role {name}", &[("name", r.clone())]));
    (fixed.into_iter().map(String::from).chain(named))
        .enumerate()
        .map(|(i, label)| FlowRole {
            label: label.into(),
            chosen: i == chosen,
        })
        .collect()
}

fn diagram(m: &mut ScreensModel, s: &ScreensState, flow: &UiFlow, texts: &Texts) {
    let g = layout::layout(flow, &s.collapsed);
    let reaches = screen_reaches(flow, &s.assignment());
    let cards = g.cards.iter().map(|c| {
        let mut card = card(flow, c.node, &reaches, texts);
        card.x = c.x;
        card.y = c.y;
        card.selected = matches!(c.node, Node::Screen(i) if Some(i) == s.selected);
        card
    });
    m.cards = model(cards);
    m.arrows = model(g.arrows.into_iter().map(|a| FlowArrow {
        path: a.path.into(),
        unresolved: a.unresolved,
    }));
    m.bands = model(g.bands.into_iter().map(|b| {
        let name = if b.application.is_empty() {
            texts.t("Screens without an application").to_string()
        } else {
            b.application
        };
        let count = texts.plural(b.screens, "{count} screen", "{count} screens");
        FlowBand {
            title: format!("{name} · {count}").into(),
            y: b.y,
            h: b.h,
            collapsed: b.collapsed,
        }
    }));
    m.width = g.width;
    m.height = g.height;
}

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
    let (state, note) = match reaches.get(&screen.id) {
        Some(ScreenReach::Sure) => (0, String::new()),
        Some(ScreenReach::Possible) => (1, texts.t("only if an open condition holds").into()),
        Some(ScreenReach::Denied) => {
            let needs = words::access(texts, screen.access.as_ref());
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

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::{Condition, Origin, Target, Transition};

    fn screen(id: &str, route: &str, access: Option<Condition>) -> Screen {
        Screen {
            id: id.into(),
            name: id.into(),
            kind: archguard_engine::core::ScreenKind::Page,
            route: Some(route.into()),
            application: Some("Web".into()),
            file: format!("{id}.razor"),
            line: 1,
            access,
            source: "test".into(),
        }
    }

    fn shop() -> UiFlow {
        let go = |to: Target| Transition {
            from: Origin::Screen("home".into()),
            target: to,
            trigger: None,
            file: "home.razor".into(),
            line: 7,
            condition: None,
            source: "test".into(),
        };
        UiFlow {
            screens: vec![
                screen("home", "/", None),
                screen("checkout", "/checkout", Some(Condition::Authenticated)),
                screen("lost", "/lost", None),
            ],
            transitions: vec![
                go(Target::Screen("checkout".into())),
                go(Target::Unresolved("@Url(x)".into())),
            ],
            ..UiFlow::default()
        }
    }

    fn states(m: &ScreensModel) -> Vec<(String, i32, String)> {
        use slint::Model;
        (m.cards.iter())
            .map(|c| (c.title.to_string(), c.state, c.note.to_string()))
            .collect()
    }

    #[test]
    fn cards_show_the_reach_of_the_simulated_user() {
        let mut s = super::super::state::build_from(Ok(shop()));
        let m = build(&s, &Texts::default());
        assert_eq!(
            states(&m),
            [
                ("home".into(), 0, String::new()),
                ("checkout".into(), 2, "needs: signed in".into()),
                (
                    "lost".into(),
                    3,
                    "no known path, 1 link in this app unresolved".into()
                ),
                ("1 target not readable".into(), 4, String::new()),
            ]
        );
        s.select_role(1);
        let m = build(&s, &Texts::default());
        assert_eq!(states(&m)[1].1, 0, "signed in reaches the checkout");
        assert_eq!((m.resolved, m.navigations), (1, 2));
    }

    #[test]
    fn unresolved_and_unreached_cards_are_dashed() {
        let s = super::super::state::build_from(Ok(shop()));
        let m = build(&s, &Texts::default());
        use slint::Model;
        let dashed: Vec<bool> = m.cards.iter().map(|c| !c.border.is_empty()).collect();
        assert_eq!(dashed, [false, false, true, true]);
        let unresolved = m.cards.row_data(3).unwrap();
        assert_eq!(unresolved.screen, -1);
        assert_eq!(unresolved.group, 1, "opens the list of its source");
    }

    #[test]
    fn unreadable_targets_list_with_file_and_line_and_bands_collapse() {
        use slint::Model;
        let mut s = super::super::state::build_from(Ok(shop()));
        s.open_group(1);
        let m = build(&s, &Texts::default());
        assert_eq!(m.inspector.title, "Targets not readable");
        let row = m.inspector.rows.row_data(0).unwrap();
        assert_eq!(
            (row.text.as_str(), row.location.as_str()),
            ("@Url(x)", "home.razor:7")
        );
        assert_eq!(m.bands.row_data(0).unwrap().title, "Web · 3 screens");
        s.toggle_band(0);
        let m = build(&s, &Texts::default());
        assert!(m.bands.row_data(0).unwrap().collapsed);
        assert_eq!(m.cards.row_count(), 0, "a collapsed band hides its cards");
        s.toggle_band(0);
        assert_eq!(build(&s, &Texts::default()).cards.row_count(), 4);
    }

    #[test]
    fn an_unreadable_flow_shows_its_error() {
        let s = super::super::state::build_from(Err("bad import".into()));
        let m = build(&s, &Texts::default());
        assert_eq!((m.state, m.error.as_str()), (2, "bad import"));
    }
}
