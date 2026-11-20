//! The Screens tab's details panel: the selected screen (access, result
//! for the simulated user, actions and what the backend says to them) or
//! a list of the left panel.

use archguard_engine::core::{
    AccessState, Action, Condition, DiagnosticSeverity, EndpointAccess, Origin, Outcome, Screen,
    Target, Transition, UiFlow, UiFlowFinding, UiFlowFindingKind, evaluate_condition,
    join_actions_to_endpoints,
};

use super::state::{List, ScreensState};
use super::{code, drill, notes, words};
use crate::tab_support::{Texts, model};
use crate::ui::{FlowAction, FlowInspector, FlowRow};

fn at(file: &str, line: u32) -> String {
    format!("{file}:{line}")
}

/// A list row without a note.
pub fn row(text: String, file: &str, line: u32) -> FlowRow {
    FlowRow {
        text: text.into(),
        location: at(file, line).into(),
        file: file.into(),
        line: line as i32,
        ..Default::default()
    }
}

/// `row` that takes a note under `id`, with the stored one shown.
fn noted(s: &ScreensState, mut r: FlowRow, id: &str, texts: &Texts) -> FlowRow {
    r.id = id.into();
    if let Some(d) = s.notes.get(id) {
        r.badge = texts.t(notes::badge(d)).into();
        r.note = d.note.as_str().into();
    }
    r
}

pub fn build(s: &ScreensState, flow: &UiFlow, texts: &Texts) -> FlowInspector {
    let mut i = panel(s, flow, texts);
    i.problem = s.problem.as_str().into();
    i
}

fn panel(s: &ScreensState, flow: &UiFlow, texts: &Texts) -> FlowInspector {
    if s.hide_details {
        return FlowInspector::default();
    }
    if let Some((file, line)) = &s.code {
        let (lines, hit) = code::read(&s.root, file, *line).unwrap_or_default();
        return FlowInspector {
            mode: 5,
            title: file.rsplit('/').next().unwrap_or(file).into(),
            location: at(file, *line).into(),
            file: file.as_str().into(),
            line: *line as i32,
            code: model(lines),
            code_hit: hit as i32,
            ..Default::default()
        };
    }
    if let Some((id, subject)) = &s.note_for {
        return FlowInspector {
            mode: 4,
            title: texts.t("Note for the coding agent").into(),
            note_id: id.as_str().into(),
            note_subject: subject.as_str().into(),
            ..Default::default()
        };
    }
    if let Some(a) = s.drill {
        let who = s.assignment();
        let steps = drill::steps(&s.sim, s.domain.as_ref(), &s.root, a, &who, texts);
        let title = (s.sim.actions.get(a)).map_or(String::new(), |x| match &x.trigger {
            Some(t) => format!("{t} · {}", words::call(x)),
            None => words::call(x),
        });
        return FlowInspector {
            mode: 3,
            title: title.into(),
            list_note: texts.t("From the action through the backend to the services. Endpoint results are for the simulated user.").into(),
            steps: model(steps),
            ..Default::default()
        };
    }
    if let Some(list) = s.list {
        return list_panel(s, flow, list, texts);
    }
    let Some(screen) = s.selected.and_then(|i| flow.screens.get(i)) else {
        return FlowInspector::default();
    };
    // The simulated copy: an unreadable guard follows the switches too.
    let guard = s
        .selected
        .and_then(|i| s.sim.screens.get(i)?.access.as_ref());
    let outcome = guard.map_or(Outcome::Allowed, |c| evaluate_condition(c, &s.assignment()));
    let triggered = actions(s, flow, screen, texts);
    let (meaning, meaning_from, _) = meaning(s, screen, texts);
    FlowInspector {
        meaning: meaning.into(),
        meaning_from: meaning_from.into(),
        mode: 1,
        title: screen.name.as_str().into(),
        route: screen.route.clone().unwrap_or_default().into(),
        location: at(&screen.file, screen.line).into(),
        file: screen.file.as_str().into(),
        line: screen.line as i32,
        access: words::access(texts, screen.access.as_ref()).into(),
        result: words::outcome(texts, outcome).into(),
        result_tone: match outcome {
            Outcome::Allowed => 0,
            Outcome::Denied => 1,
            Outcome::Unknown => 2,
        },
        actions: model(triggered.iter().filter(|a| !a.trigger.is_empty()).cloned()),
        untriggered: model(triggered.into_iter().filter(|a| a.trigger.is_empty())),
        ..Default::default()
    }
}

/// Whether `origin` is `screen`, itself or as a layout of its app.
fn on(origin: &Origin, screen: &Screen) -> bool {
    match origin {
        Origin::Screen(id) => *id == screen.id,
        Origin::AnyScreen { application } => {
            *application == screen.application.clone().unwrap_or_default()
        }
    }
}

fn actions(s: &ScreensState, flow: &UiFlow, screen: &Screen, texts: &Texts) -> Vec<FlowAction> {
    let joined = join_actions_to_endpoints(flow);
    (flow.actions.iter().enumerate())
        .filter(|(_, a)| on(&a.screen, screen))
        .map(|(index, a)| {
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
                line: a.line as i32,
                index: index as i32,
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

/// Whether `f` is about action `a`: same place, and the endpoints it names
/// are the action's (several calls can share a line). A finding without
/// endpoints (no backend) needs an action without hits whose call it names.
fn names(f: &UiFlowFinding, a: &Action, hits: &[&EndpointAccess]) -> bool {
    let place = (f.action.as_ref()).is_some_and(|r| r.file == a.file && r.line == a.line);
    let endpoints = if f.endpoints.is_empty() {
        hits.is_empty() && f.message.contains(&words::call(a))
    } else {
        (f.endpoints.iter()).all(|r| {
            (hits.iter()).any(|e| {
                (e.method.as_str(), e.path.as_str(), e.file.as_str(), e.line)
                    == (r.method.as_str(), r.path.as_str(), r.file.as_str(), r.line)
            })
        })
    };
    place && endpoints && f.kind != UiFlowFindingKind::NoStartScreen
}

/// The mismatch finding of this action, else agreement.
fn agreement(
    s: &ScreensState,
    a: &Action,
    hits: &[&EndpointAccess],
    texts: &Texts,
) -> (String, bool) {
    let finding = s.findings.iter().find(|f| names(f, a, hits));
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
    let findings = |kind| {
        (s.findings.iter().enumerate())
            .filter(|(_, f)| f.kind == kind)
            .map(|(i, f)| {
                let (file, line) = f
                    .action
                    .as_ref()
                    .map_or(("", 0), |a| (a.file.as_str(), a.line));
                let r = row(f.message.clone(), file, line);
                match s.ids.finding(i) {
                    Some(id) => noted(s, r, id, texts),
                    None => r,
                }
            })
            .collect::<Vec<_>>()
    };
    let (title, note, rows) = match list {
        List::Gaps => (
            "Gaps",
            "Navigation targets, routes, roles and calls the rules could not read literally. Each one may hide a path.",
            (flow.diagnostics.iter())
                .filter(|d| d.severity == DiagnosticSeverity::Gap)
                .map(|d| {
                    noted(
                        s,
                        row(d.message.clone(), &d.file, d.line),
                        &s.ids.gap(&s.sim, d),
                        texts,
                    )
                })
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
        List::Unreadable(k) => (
            "Targets not readable",
            "Navigation targets of this source the rules could not read literally. Each one may hide a path.",
            target_rows(unreadable(flow, k)),
        ),
        List::UnreadableFrom(i) => (
            "Targets not readable",
            "Navigation targets of this screen and of its layouts and menus the rules could not read literally. Each one may hide a path.",
            target_rows((flow.transitions.iter()).filter(|t| {
                matches!(t.target, Target::Unresolved(_)) && on(&t.from, &flow.screens[i])
            })),
        ),
        List::Mismatches => (
            "Mismatches",
            "Where frontend and backend disagree: an action a simulated user reaches but every matching endpoint refuses, or an action the frontend guards but a matching endpoint leaves open.",
            [
                findings(UiFlowFindingKind::ShownButRefused),
                findings(UiFlowFindingKind::HiddenButUnprotected),
            ]
            .concat(),
        ),
        List::Undeclared => (
            "Endpoints without a declared guard",
            "No guard is declared; the code may still check by hand. Undeclared is never shown as allowed.",
            undeclared(s, flow, texts),
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

/// The endpoints without a declared guard, each taking a note.
fn undeclared(s: &ScreensState, flow: &UiFlow, texts: &Texts) -> Vec<FlowRow> {
    (flow.endpoints.iter().enumerate())
        .filter(|(_, e)| e.access == AccessState::Undeclared)
        .map(|(i, e)| {
            let r = row(format!("{} {}", e.method, e.path), &e.file, e.line);
            match s.ids.endpoint(i) {
                Some(id) => noted(s, r, id, texts),
                None => r,
            }
        })
        .collect()
}

/// The screen's guard in words and where that meaning comes from: code,
/// an entry of `[views.access]` named in the screen's file, or unknown;
/// and the origin short enough for a card.
pub fn meaning(s: &ScreensState, screen: &Screen, texts: &Texts) -> (String, String, String) {
    let Some(c) = &screen.access else {
        return (
            texts
                .t("No guard: everyone who reaches the screen may open it.")
                .into(),
            texts
                .t("No guard was read in the code for this screen.")
                .into(),
            String::new(),
        );
    };
    let mut opaque = vec![];
    let mut todo = vec![c];
    while let Some(c) = todo.pop() {
        match c {
            Condition::All(items) | Condition::Any(items) => todo.extend(items),
            Condition::Not(inner) => todo.push(inner),
            Condition::Custom(name) | Condition::Unknown(name) => opaque.push(name.clone()),
            _ => {}
        }
    }
    let words = words::condition(texts, c);
    if !opaque.is_empty() {
        opaque.reverse();
        let short = texts.plural(
            opaque.len(),
            "{count} guard the rules cannot read: see the details panel",
            "{count} guards the rules cannot read: see the details panel",
        );
        let names = opaque.join(", ");
        return (
            words,
            texts.f(
                "Unknown: the rules cannot tell what {name} checks. Give it a meaning in [views.access] of the project configuration.",
                &[("name", names)],
            ),
            short,
        );
    }
    // shortcut: a listed name anywhere in the screen's file counts, not only at
    // its route; upgrade when the model records where a condition came from.
    let text = std::fs::read_to_string(s.root.join(&screen.file)).unwrap_or_default();
    let listed = (s.config.iter())
        .flat_map(|c| &c.views.access)
        .find(|(key, _)| text.contains(key.as_str()));
    let from = match listed {
        Some((key, m)) => texts.f(
            "From [views.access]: {name}",
            &[("name", format!("{key} = {}", String::from(m.clone())))],
        ),
        None => texts.f(
            "From code: {name}",
            &[(
                "name",
                format!("{} ({})", at(&screen.file, screen.line), screen.source),
            )],
        ),
    };
    (words, from.clone(), from)
}

/// A list row per transition: its target text, file and line.
fn target_rows<'a>(ts: impl Iterator<Item = &'a Transition>) -> Vec<FlowRow> {
    ts.map(|t| {
        let (Target::Unresolved(text) | Target::Screen(text)) = &t.target;
        row(text.clone(), &t.file, t.line)
    })
    .collect()
}

/// The unresolved transitions from the source of transition `k`.
pub fn unreadable(flow: &UiFlow, k: usize) -> impl Iterator<Item = &Transition> {
    let from = flow.transitions.get(k).map(|t| &t.from);
    (flow.transitions.iter())
        .filter(move |t| Some(&t.from) == from && matches!(t.target, Target::Unresolved(_)))
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
            handler: None,
            sends: Vec::new(),
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
    fn a_finding_shows_only_on_the_action_whose_call_it_names() {
        use archguard_engine::core::{UiFlowActionRef, UiFlowEndpointRef, UiFlowFinding};
        let mut f = flow();
        // Two calls on one line, as on Checkout.razor:114.
        f.actions[1].line = 5;
        f.actions[1].call = Call::Http {
            method: "GET".into(),
            path: "/api/catalog".into(),
        };
        f.endpoints.push(EndpointAccess {
            path: "/api/catalog".into(),
            access: AccessState::Open,
            line: 20,
            ..f.endpoints[0].clone()
        });
        let mut s = super::super::state::build_from(Ok(f));
        s.findings = vec![UiFlowFinding {
            severity: archguard_engine::core::Severity::Warning,
            kind: UiFlowFindingKind::HiddenButUnprotected,
            application: None,
            action: Some(UiFlowActionRef {
                file: "Orders.razor".into(),
                line: 5,
                trigger: Some("Load".into()),
            }),
            role: None,
            endpoints: vec![UiFlowEndpointRef {
                method: "GET".into(),
                path: "/api/catalog".into(),
                file: "Api.cs".into(),
                line: 20,
                application: None,
            }],
            message: "catalog unguarded".into(),
        }];
        s.select_screen(0);
        let flow = s.flow().unwrap().clone();
        let a: Vec<FlowAction> = build(&s, &flow, &Texts::default()).actions.iter().collect();
        assert_eq!(a[0].agreement, "Frontend and backend agree.");
        assert_eq!(a[1].agreement, "catalog unguarded");
    }

    #[test]
    fn actions_without_a_trigger_are_listed_apart() {
        let mut f = flow();
        f.actions[1].trigger = None;
        let mut s = super::super::state::build_from(Ok(f));
        s.select_screen(0);
        let flow = s.flow().unwrap().clone();
        let i = build(&s, &flow, &Texts::default());
        assert_eq!((i.actions.row_count(), i.untriggered.row_count()), (1, 1));
        assert_eq!(i.untriggered.row_data(0).unwrap().call, "GET /api/missing");
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

    /// The undeclared list shows a stored note with its badge; an action
    /// opens its drill-down, a file:line the code panel, Back returns.
    #[test]
    fn notes_drill_down_and_code_panel_stack_in_the_details() {
        use archguard_engine::core::{DecisionKind, ReviewDecision};
        let mut f = flow();
        f.endpoints[0].access = AccessState::Undeclared;
        let mut s = super::super::state::build_from(Ok(f));
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join("Orders.razor"), "a\nb\nc\n").expect("file");
        s.root = dir.path().to_path_buf();
        let id = "view-endpoint_access_declared:Api.cs:GET /api/orders";
        s.notes.insert(
            id.into(),
            ReviewDecision {
                decision: DecisionKind::AcceptRisk,
                note: "checked in code".into(),
                ..Default::default()
            },
        );
        s.select_screen(0);
        s.open_list(3);
        let flow = s.flow().unwrap().clone();
        let r = build(&s, &flow, &Texts::default())
            .rows
            .row_data(0)
            .unwrap();
        assert_eq!((r.id.as_str(), r.badge.as_str()), (id, "intentional"));
        assert_eq!(r.note, "checked in code");
        s.open_drill(0);
        let i = build(&s, &flow, &Texts::default());
        assert_eq!((i.mode, i.title.as_str()), (3, "Load · GET /api/orders"));
        assert_eq!(i.steps.row_data(1).unwrap().kind, "Endpoint");
        s.show_code("Orders.razor", 2);
        let i = build(&s, &flow, &Texts::default());
        assert_eq!((i.mode, i.code_hit, i.code.row_count()), (5, 1, 3));
        s.close_details();
        assert_eq!(
            build(&s, &flow, &Texts::default()).mode,
            3,
            "back to the drill-down"
        );
        s.close_details();
        assert_eq!(
            build(&s, &flow, &Texts::default()).mode,
            2,
            "back to the list"
        );
    }
}
