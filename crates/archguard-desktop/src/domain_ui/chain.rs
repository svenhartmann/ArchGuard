//! The Domain tab's chain view (ADR 0022 decision 4): for the selected
//! state, the chains that lead into it ("who can cause this"), and every
//! chain touching the selected entity. Each chain reads screen action →
//! endpoint → use cases → transitions → events; the endpoint is coloured by
//! what it says to the simulated user of the Screens tab's role list.

use archguard_engine::core::domain_model::DomainModel;
use archguard_engine::core::{Assignment, Origin, Outcome, UiFlow, evaluate_access};
use archguard_engine::domain_chain::{Chain, ChainStart, chains, triggers_of_state};

use crate::screens_ui::words;
use crate::tab_support::{Texts, model};
use crate::ui::{DomChain, DomStep};

/// The chains into `state` of `entity` (id).
pub fn triggers(
    flow: &UiFlow,
    d: &DomainModel,
    entity: &str,
    state: &str,
    a: &Assignment,
    texts: &Texts,
) -> Vec<DomChain> {
    (triggers_of_state(flow, d, entity, state).iter())
        .map(|c| chain(flow, c, a, texts))
        .collect()
}

/// Every chain with a transition of `entity` (id).
pub fn of_entity(
    flow: &UiFlow,
    d: &DomainModel,
    entity: &str,
    a: &Assignment,
    texts: &Texts,
) -> Vec<DomChain> {
    (chains(flow, d).iter())
        .filter(|c| c.transitions.iter().any(|t| t.entity == entity))
        .map(|c| chain(flow, c, a, texts))
        .collect()
}

fn step(kind: &str, text: String, file: &str, line: u32, tone: i32) -> DomStep {
    DomStep {
        kind: kind.into(),
        text: text.into(),
        location: if file.is_empty() {
            Default::default()
        } else {
            format!("{file}:{line}").into()
        },
        file: file.into(),
        tone,
    }
}

fn chain(flow: &UiFlow, c: &Chain, a: &Assignment, texts: &Texts) -> DomChain {
    let mut steps = Vec::new();
    if let Some(act) = c.action() {
        let screen = match &act.screen {
            Origin::Screen(id) => (flow.screens.iter())
                .find(|s| s.id == *id)
                .map_or(id.clone(), |s| s.name.clone()),
            Origin::AnyScreen { application } => {
                texts.f("every screen of {name}", &[("name", application.clone())])
            }
        };
        let what = act.trigger.clone().unwrap_or_else(|| words::call(act));
        let text = format!("{screen} · {what}");
        steps.push(step(
            texts.t("Screen action"),
            text,
            &act.file,
            act.line,
            -1,
        ));
    }
    match (c.endpoint(), &c.start) {
        (Some(e), _) => {
            let outcome = evaluate_access(&e.access, a);
            let text = format!(
                "{} {} · {} · {}",
                e.method,
                e.path,
                words::endpoint_access(texts, &e.access),
                words::outcome(texts, outcome)
            );
            let tone = match outcome {
                Outcome::Allowed => 0,
                Outcome::Denied => 1,
                Outcome::Unknown => 2,
            };
            steps.push(step(texts.t("Endpoint"), text, &e.file, e.line, tone));
        }
        (None, ChainStart::Message { message_handler: m }) => {
            let text = format!("{} · {} · {}", m.message, m.id, texts.t("not a user path"));
            steps.push(step(texts.t("Message handler"), text, &m.file, m.line, -1));
        }
        (None, ChainStart::Endpoint { .. }) => {}
    }
    for u in &c.use_cases {
        steps.push(step(
            texts.t("Use case"),
            u.name.clone(),
            &u.file,
            u.line,
            -1,
        ));
    }
    for t in &c.transitions {
        let x = t.transition;
        let from = if x.from.is_empty() {
            "*".to_string()
        } else {
            x.from.join(", ")
        };
        let text = format!("{}.{}: {from} → {} ({})", t.entity, t.field, x.to, x.method);
        steps.push(step(texts.t("Transition"), text, &x.file, x.line, -1));
    }
    for ev in &c.events {
        let handlers = if ev.handlers.is_empty() {
            texts.t("none").to_string()
        } else {
            ev.handlers.join(", ")
        };
        let text = format!("{} → {handlers}", ev.event);
        steps.push(step(texts.t("Event"), text, "", 0, -1));
    }
    DomChain {
        steps: model(steps),
    }
}
