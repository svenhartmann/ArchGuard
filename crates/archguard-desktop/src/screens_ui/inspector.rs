//! The Screens tab's details panel: the selected screen (access, result
//! for the simulated user, actions and what the backend says to them) or
//! a list of the left panel.

use archguard_engine::core::{
    AccessState, Action, DiagnosticSeverity, EndpointAccess, Origin, Outcome, Screen, UiFlow,
    UiFlowFindingKind, evaluate_condition, join_actions_to_endpoints,
};

use super::state::{List, ScreensState};
use super::words;
use crate::tab_support::{Texts, model};
use crate::ui::{FlowAction, FlowInspector, FlowRow};

fn at(file: &str, line: u32) -> String {
    format!("{file}:{line}")
}

pub fn build(s: &ScreensState, flow: &UiFlow, texts: &Texts) -> FlowInspector {
    if let Some(list) = s.list {
        return list_panel(s, flow, list, texts);
    }
    let Some(screen) = s.selected.and_then(|i| flow.screens.get(i)) else {
        return FlowInspector::default();
    };
    let outcome = screen
        .access
        .as_ref()
        .map_or(Outcome::Allowed, |c| evaluate_condition(c, &s.assignment()));
    FlowInspector {
        mode: 1,
        title: screen.name.as_str().into(),
        route: screen.route.clone().unwrap_or_default().into(),
        location: at(&screen.file, screen.line).into(),
        file: screen.file.as_str().into(),
        access: words::access(texts, screen.access.as_ref()).into(),
        result: words::outcome(texts, outcome).into(),
        result_tone: match outcome {
            Outcome::Allowed => 0,
            Outcome::Denied => 1,
            Outcome::Unknown => 2,
        },
        actions: model(actions(s, flow, screen, texts)),
        ..Default::default()
    }
}

/// Whether `action` starts on `screen`, itself or as a layout of its app.
fn on(action: &Action, screen: &Screen) -> bool {
    match &action.screen {
        Origin::Screen(id) => *id == screen.id,
        Origin::AnyScreen { application } => {
            *application == screen.application.clone().unwrap_or_default()
        }
    }
}

fn actions(s: &ScreensState, flow: &UiFlow, screen: &Screen, texts: &Texts) -> Vec<FlowAction> {
    let joined = join_actions_to_endpoints(flow);
    (flow.actions.iter().filter(|a| on(a, screen)))
        .map(|a| {
            let hits: &[&EndpointAccess] = (joined.iter())
                .find(|(j, _)| std::ptr::eq(*j, a))
                .map_or(&[], |(_, h)| h.as_slice());
            let (agreement, agree) = agreement(s, a, hits, texts);
            FlowAction {
                trigger: a.trigger.clone().unwrap_or_default().into(),
                call: words::call(a).into(),
                backend: backend(hits, texts).into(),
                agreement: agreement.into(),
                agree,
                location: at(&a.file, a.line).into(),
                file: a.file.as_str().into(),
            }
        })
        .collect()
}

fn backend(hits: &[&EndpointAccess], texts: &Texts) -> String {
    if hits.is_empty() {
        return texts.t("no scanned endpoint matches").into();
    }
    let rows: Vec<String> = (hits.iter())
        .map(|e| {
            let access = words::endpoint_access(texts, &e.access);
            format!("{} {}: {access}", e.method, e.path)
        })
        .collect();
    rows.join("\n")
}

/// The mismatch finding of this action, else agreement.
fn agreement(
    s: &ScreensState,
    a: &Action,
    hits: &[&EndpointAccess],
    texts: &Texts,
) -> (String, bool) {
    let finding = s.findings.iter().find(|f| {
        f.action
            .as_ref()
            .is_some_and(|r| r.file == a.file && r.line == a.line)
            && f.kind != UiFlowFindingKind::NoStartScreen
    });
    match finding {
        Some(f) => (f.message.clone(), false),
        None if hits.is_empty() => (
            texts
                .t("The backend of this call is not in the scan.")
                .into(),
            false,
        ),
        None => (texts.t("Frontend and backend agree.").into(), true),
    }
}

fn list_panel(s: &ScreensState, flow: &UiFlow, list: List, texts: &Texts) -> FlowInspector {
    let row = |text: String, file: &str, line: u32| FlowRow {
        text: text.into(),
        location: at(file, line).into(),
        file: file.into(),
    };
    let findings = |kind| {
        (s.findings.iter().filter(|f| f.kind == kind))
            .map(|f| {
                let (file, line) = f
                    .action
                    .as_ref()
                    .map_or(("", 0), |a| (a.file.as_str(), a.line));
                row(f.message.clone(), file, line)
            })
            .collect::<Vec<_>>()
    };
    let (title, note, rows) = match list {
        List::Gaps => (
            "Gaps",
            "Navigation targets, routes, roles and calls the rules could not read literally. Each one may hide a path.",
            (flow.diagnostics.iter())
                .filter(|d| d.severity == DiagnosticSeverity::Gap)
                .map(|d| row(d.message.clone(), &d.file, d.line))
                .collect(),
        ),
        List::ShownButRefused => (
            "Shown but refused",
            "A simulated user reaches the action, but every matching endpoint refuses it.",
            findings(UiFlowFindingKind::ShownButRefused),
        ),
        List::HiddenButUnprotected => (
            "Hidden but unprotected",
            "The frontend guards the action, but a matching endpoint is open or declares no guard.",
            findings(UiFlowFindingKind::HiddenButUnprotected),
        ),
        List::Undeclared => (
            "Endpoints without a declared guard",
            "No guard is declared; the code may still check by hand. Undeclared is never shown as allowed.",
            (flow.endpoints.iter())
                .filter(|e| e.access == AccessState::Undeclared)
                .map(|e| row(format!("{} {}", e.method, e.path), &e.file, e.line))
                .collect(),
        ),
    };
    FlowInspector {
        mode: 2,
        title: texts.t(title).into(),
        list_note: texts.t(note).into(),
        rows: model(rows),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::{Call, Condition, Screen, ScreenKind};
    use slint::Model;

    fn flow() -> UiFlow {
        let screen = Screen {
            id: "orders".into(),
            name: "Orders".into(),
            kind: ScreenKind::Page,
            route: Some("/orders".into()),
            application: Some("Web".into()),
            file: "Orders.razor".into(),
            line: 1,
            access: Some(Condition::Authenticated),
            source: "t".into(),
        };
        let action = |path: &str, line| Action {
            screen: Origin::AnyScreen {
                application: "Web".into(),
            },
            trigger: Some("Load".into()),
            call: Call::Http {
                method: "GET".into(),
                path: path.into(),
            },
            file: "Orders.razor".into(),
            line,
            condition: None,
            source: "t".into(),
        };
        let endpoint = EndpointAccess {
            method: "GET".into(),
            path: "/api/orders".into(),
            access: AccessState::Protected(Condition::Authenticated),
            application: None,
            file: "Api.cs".into(),
            line: 9,
            source: "t".into(),
        };
        UiFlow {
            screens: vec![screen],
            actions: vec![action("/api/orders", 5), action("/api/missing", 6)],
            endpoints: vec![endpoint],
            ..UiFlow::default()
        }
    }

    #[test]
    fn a_selected_screen_shows_access_actions_and_agreement() {
        let mut s = super::super::state::build_from(Ok(flow()));
        s.select_screen(0);
        let f = s.flow().unwrap().clone();
        let i = build(&s, &f, &Texts::default());
        assert_eq!(
            (i.mode, i.access.as_str(), i.result.as_str()),
            (1, "signed in", "denied")
        );
        let a: Vec<FlowAction> = i.actions.iter().collect();
        assert_eq!(
            a.len(),
            2,
            "layout actions count for every screen of the app"
        );
        assert_eq!(a[0].backend, "GET /api/orders: signed in");
        assert_eq!(
            (a[0].agreement.as_str(), a[0].agree),
            ("Frontend and backend agree.", true)
        );
        assert_eq!(a[1].backend, "no scanned endpoint matches");
        assert!(!a[1].agree);
    }

    #[test]
    fn lists_open_in_the_details_panel() {
        let mut s = super::super::state::build_from(Ok(flow()));
        s.open_list(3);
        let f = s.flow().unwrap().clone();
        let i = build(&s, &f, &Texts::default());
        assert_eq!(i.mode, 2);
        assert_eq!(i.rows.row_count(), 0, "the endpoint is protected");
        s.open_list(0);
        assert_eq!(build(&s, &f, &Texts::german()).title, "Lücken");
    }
}
