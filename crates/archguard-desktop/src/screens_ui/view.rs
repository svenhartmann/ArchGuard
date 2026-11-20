//! The Screens tab's window model from its state: the top bar, the route
//! tree, the focus view or the whole diagram with each screen's reach for
//! the simulated user, the path from start, the details.

use std::collections::BTreeMap;

use archguard_engine::core::{
    DiagnosticSeverity, RoleKind, ScreenReach, UiFlow, UiFlowFindingKind, screen_reaches,
};

use super::card::card;
use super::layout::{self, Node};
use super::state::{Mode, ScreensState, applications};
use super::{center, inspector, words};
use crate::tab_support::{Texts, model};
use crate::ui::{FlowArrow, FlowBand, FlowRole, FlowRow, FlowSwitch, ScreensModel};

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
    let editor = crate::editor::template();
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
        role: s.role as i32,
        role_names: role_picker(texts, Some(flow), &s.roles),
        policies: model(policy_rows(texts, flow)),
        implied: implied(s, texts).into(),
        editor_preset: crate::editor::preset_of(&editor) as i32,
        editor: editor.into(),
        apps: model(apps(s, flow, texts)),
        app_index: (applications(&s.sim).iter().position(|a| *a == s.app)).map_or(0, |i| i as i32),
        // Fieldless enums in the order of the window's choices.
        unknown: s.unknown as i32,
        mode: s.mode as i32,
        can_back: !s.back.is_empty(),
        selected: s.selected.map_or(-1, |i| i as i32),
        search: s.search.as_str().into(),
        filter: s.filter as i32,
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
        details_open: s.details_open,
        code: s.code.view(),
        ..Default::default()
    };
    let reaches = screen_reaches(&s.sim, &s.assignment());
    if s.mode == Mode::Whole {
        diagram(&mut m, s, &reaches, texts);
    } else {
        center::focus_cards(&mut m, s, &reaches, texts);
    }
    center::tree_rows(&mut m, s, &reaches, texts);
    center::path(&mut m, s, texts);
    m
}

/// The application picker: "frontend · 92 screens", in band order.
fn apps(s: &ScreensState, flow: &UiFlow, texts: &Texts) -> Vec<slint::SharedString> {
    (applications(&s.sim).into_iter())
        .map(|name| {
            let n = (flow.screens.iter())
                .filter(|x| x.application.as_deref().unwrap_or("") == name)
                .count();
            let count = texts.plural(n, "{count} screen", "{count} screens");
            let name = if name.is_empty() {
                texts.t("Screens without an application").to_string()
            } else {
                name
            };
            format!("{name} · {count}").into()
        })
        .collect()
}

/// The role picker of the Screens and Domain top bars: "Role …", then each
/// role with what the catalog says of it.
pub fn role_picker(
    texts: &Texts,
    flow: Option<&UiFlow>,
    roles: &[String],
) -> slint::ModelRc<slint::SharedString> {
    let label = |r: &String| flow.map_or(r.clone(), |f| role_label(texts, f, r));
    model(std::iter::once(texts.t("Role …").into()).chain(roles.iter().map(|r| label(r).into())))
}

/// A role of the picker with what the catalog says of it: "ADMIN · role ·
/// includes USER"; a role only conditions name stays bare.
pub fn role_label(texts: &Texts, flow: &UiFlow, name: &str) -> String {
    let defs: Vec<_> = flow.roles.iter().filter(|r| r.name == name).collect();
    let Some(first) = defs.first() else {
        return name.to_string();
    };
    let kind = texts.t(match first.kind {
        RoleKind::Role => "role",
        RoleKind::Permission => "permission",
        RoleKind::Group => "group",
    });
    let mut includes: Vec<&str> = (defs.iter().flat_map(|d| &d.includes))
        .map(String::as_str)
        .collect();
    includes.sort_unstable();
    includes.dedup();
    if includes.is_empty() {
        return format!("{name} · {kind}");
    }
    let what = texts.f("includes {names}", &[("names", includes.join(", "))]);
    format!("{name} · {kind} · {what}")
}

/// What the chosen user implies by the catalog, in one line: the roles
/// it includes and each defined policy's value. Empty when nothing.
pub fn implied(s: &ScreensState, texts: &Texts) -> String {
    let base = super::state::role_assignment(s.role, &s.roles);
    let a = s.assignment();
    let mut parts = vec![];
    let extra: Vec<&str> = (a.roles.difference(&base.roles))
        .map(String::as_str)
        .collect();
    if !extra.is_empty() {
        parts.push(texts.f("also holds {names}", &[("names", extra.join(", "))]));
    }
    let mut names: Vec<&str> = s.sim.policies.iter().map(|p| p.name.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    for name in names {
        let key = match a.policies.get(name) {
            Some(true) => "policy {name} holds",
            Some(false) => "policy {name} does not hold",
            None => "policy {name} unknown",
        };
        parts.push(texts.f(key, &[("name", name.to_string())]));
    }
    parts.join(" · ")
}

/// The catalog's policies with what they require in words.
fn policy_rows(texts: &Texts, flow: &UiFlow) -> Vec<FlowRow> {
    (flow.policies.iter())
        .map(|p| {
            let needs = words::condition(texts, &p.requires);
            let text = texts.f(
                "{name} requires {needs}",
                &[("name", p.name.clone()), ("needs", needs)],
            );
            inspector::row(text, &p.file, p.line)
        })
        .collect()
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

fn diagram(
    m: &mut ScreensModel,
    s: &ScreensState,
    reaches: &BTreeMap<String, ScreenReach>,
    texts: &Texts,
) {
    let flow = &s.sim;
    let g = layout::layout(flow, &s.collapsed);
    let cards = g.cards.iter().map(|c| {
        let mut card = card(flow, c.node, reaches, texts);
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

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::{Condition, Origin, Screen, Target, Transition};

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
        s.set_mode(2);
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
        let mut s = super::super::state::build_from(Ok(shop()));
        s.set_mode(2);
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
        s.set_mode(2);
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

    /// The picker names each role with its kind and what it includes; the
    /// line under it says what the chosen user implies.
    #[test]
    fn the_role_picker_reads_the_catalog_and_says_what_a_role_implies() {
        use archguard_engine::core::{PolicyDef, RoleDef};
        use slint::Model;
        let def = |name: &str, includes: &[&str]| RoleDef {
            name: name.into(),
            kind: RoleKind::Role,
            includes: includes.iter().map(|s| s.to_string()).collect(),
            file: "SecurityConfiguration.java".into(),
            line: 40,
            source: "t".into(),
        };
        let mut f = shop();
        f.roles = vec![def("ADMIN", &["USER"]), def("USER", &[])];
        f.policies = vec![PolicyDef {
            name: "CanPurge".into(),
            requires: Condition::Role("ADMIN".into()),
            file: "P.cs".into(),
            line: 2,
            source: "t".into(),
        }];
        let mut s = super::super::state::build_from(Ok(f));
        let m = build(&s, &Texts::default());
        let names: Vec<String> = m.role_names.iter().map(|n| n.to_string()).collect();
        assert_eq!(
            names,
            ["Role …", "ADMIN · role · includes USER", "USER · role"]
        );
        assert_eq!(
            m.policies.row_data(0).unwrap().text,
            "CanPurge requires role ADMIN"
        );
        assert_eq!(m.implied, "policy CanPurge does not hold");
        s.select_role(2);
        let m = build(&s, &Texts::default());
        assert_eq!(m.implied, "also holds USER · policy CanPurge holds");
        assert_eq!(
            build(&s, &Texts::german()).implied,
            "hat damit auch USER · Policy CanPurge erfüllt"
        );
    }
}
