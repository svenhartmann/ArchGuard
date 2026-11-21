//! The tab's choices: application, unknown conditions, selection history.

use archguard_engine::core::{
    Condition, Origin, Screen, ScreenKind, ScreenReach, Target, Transition, UiFlow, screen_reaches,
};

use super::state::{ScreensState, Unknown, build_from};

fn screen(id: &str, app: &str, route: &str, access: Option<Condition>) -> Screen {
    Screen {
        weak: false,
        id: id.into(),
        name: id.into(),
        kind: ScreenKind::Page,
        route: Some(route.into()),
        application: Some(app.into()),
        file: format!("{id}.ts"),
        line: 1,
        access: archguard_engine::core::AccessState::from_parts(access, true),
        source: "test".into(),
    }
}

fn go(from: &str, to: &str) -> Transition {
    Transition {
        weak: false,
        from: Origin::Screen(from.into()),
        target: Target::Screen(to.into()),
        trigger: None,
        file: "x.ts".into(),
        line: 1,
        condition: archguard_engine::core::AccessState::Open,
        source: "test".into(),
    }
}

/// `web` starts at home and reaches two screens; `admin` has a start but
/// reaches one; `tools` has no start. One guard is a custom atom, one an
/// unreadable expression.
fn flow() -> UiFlow {
    UiFlow {
        screens: vec![
            screen("aaa", "tools", "/aaa", None),
            screen("panel", "admin", "/", None),
            screen("home", "web", "/", None),
            screen(
                "edit",
                "web",
                "/edit",
                Some(Condition::Custom("canEdit".into())),
            ),
            screen(
                "odd",
                "web",
                "/odd",
                Some(Condition::Unknown("inject(X).ok()".into())),
            ),
        ],
        transitions: vec![go("home", "edit"), go("home", "odd")],
        ..UiFlow::default()
    }
}

fn reach(s: &ScreensState, id: &str) -> ScreenReach {
    screen_reaches(&s.sim, &s.assignment())[id]
}

#[test]
fn the_app_with_a_start_and_most_reached_screens_comes_first() {
    let s = build_from(Ok(flow()));
    assert_eq!(s.app, "web");
    assert_eq!(s.selected, Some(2), "its start screen is selected");
    let mut f = flow();
    f.screens
        .retain(|s| s.application.as_deref() == Some("tools"));
    let s = build_from(Ok(f));
    assert_eq!(
        (s.app.as_str(), s.selected),
        ("tools", Some(0)),
        "no start: first by name"
    );
}

#[test]
fn unknown_conditions_stay_open_or_all_hold_or_none_hold() {
    let mut s = build_from(Ok(flow()));
    assert_eq!(s.atoms.len(), 2, "the unreadable guard is a switch too");
    assert_eq!(reach(&s, "edit"), ScreenReach::Possible);
    assert_eq!(reach(&s, "odd"), ScreenReach::Possible);
    s.set_unknown(1);
    assert_eq!(s.unknown, Unknown::AllHold);
    assert_eq!(reach(&s, "edit"), ScreenReach::Sure);
    assert_eq!(reach(&s, "odd"), ScreenReach::Sure);
    s.set_unknown(2);
    assert_eq!(reach(&s, "edit"), ScreenReach::Denied);
    assert_eq!(reach(&s, "odd"), ScreenReach::Denied);
    let i = s.atoms.iter().position(|a| a.name == "canEdit").unwrap();
    s.set_switch(i, 1);
    assert_eq!(reach(&s, "edit"), ScreenReach::Sure, "an own switch wins");
    assert!(
        matches!(
            &s.flow().unwrap().screens[4].access,
            archguard_engine::core::AccessState::Protected(Condition::Unknown(_))
        ),
        "the shown flow keeps the unreadable text"
    );
}

#[test]
fn selection_switches_the_app_and_back_returns() {
    let mut s = build_from(Ok(flow()));
    s.select_screen(3);
    s.select_screen(1);
    assert_eq!(s.app, "admin", "a screen of another app shows that app");
    s.go_back();
    assert_eq!((s.selected, s.app.as_str()), (Some(3), "web"));
    s.go_back();
    assert_eq!(s.selected, Some(2));
    assert!(s.back.is_empty());
    s.select_app(1);
    assert_eq!((s.app.as_str(), s.selected), ("admin", Some(1)));
}
