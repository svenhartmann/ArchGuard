//! The Domain tab's chain view (ADR 0022 decision 4): for the selected
//! state, the chains that lead into it ("who can cause this"), and every
//! chain touching the selected entity. Each chain reads screen action →
//! endpoint → use cases → transitions → events with the service of each
//! step; a chain continued through an integration event (`continues`,
//! `via`) follows indented. The endpoint and the chain's verdict say what
//! they mean for the simulated user (`domain_chain::rights`); a chain a
//! message starts is no user path.

use archguard_engine::core::domain_model::DomainModel;
use archguard_engine::core::{Assignment, Origin, Outcome, UiFlow, evaluate_access};
use archguard_engine::domain_chain::{
    Chain, ChainStart, Verdict, chains, rights, touches, triggers_of_state,
};

use super::services::Services;
use crate::screens_ui::words;
use crate::tab_support::{Texts, model};
use crate::ui::{DomChain, DrillStep};

/// The chains into `state` of `entity` (id).
pub fn triggers(
    flow: &UiFlow,
    d: &DomainModel,
    sv: &Services,
    (entity, state): (&str, &str),
    a: &Assignment,
    texts: &Texts,
) -> Vec<DomChain> {
    shown(
        flow,
        (d, sv),
        &triggers_of_state(flow, d, entity, state),
        a,
        texts,
    )
}

/// Every chain that changes `entity` (id), nested ones included.
pub fn of_entity(
    flow: &UiFlow,
    d: &DomainModel,
    sv: &Services,
    entity: &str,
    a: &Assignment,
    texts: &Texts,
) -> Vec<DomChain> {
    let found: Vec<Chain> = chains(flow, d)
        .into_iter()
        .filter(|c| touches(c, entity))
        .collect();
    shown(flow, (d, sv), &found, a, texts)
}

/// What every step needs: the flow, the model with its services, the
/// simulated user and the texts.
#[derive(Clone, Copy)]
pub struct Ctx<'a> {
    pub flow: &'a UiFlow,
    pub d: &'a DomainModel,
    pub sv: &'a Services,
    pub a: &'a Assignment,
    pub texts: &'a Texts,
}

/// The model and its services: where each step lies.
type Place<'a> = (&'a DomainModel, &'a Services);

fn shown(
    flow: &UiFlow,
    (d, sv): Place,
    cs: &[Chain],
    a: &Assignment,
    texts: &Texts,
) -> Vec<DomChain> {
    let cx = Ctx {
        flow,
        d,
        sv,
        a,
        texts,
    };
    let r = rights(flow, cs, a);
    (cs.iter().zip(r))
        .map(|(c, right)| {
            let mut steps = Vec::new();
            flatten(cx, c, 0, &mut steps);
            let (verdict, tone) = match right.verdict {
                Verdict::Allowed => (words::outcome(texts, Outcome::Allowed), 0),
                Verdict::Denied => (words::outcome(texts, Outcome::Denied), 1),
                Verdict::Unknown => (words::outcome(texts, Outcome::Unknown), 2),
                Verdict::NotAUserPath => (
                    texts
                        .t("not a user path: a message starts it inside the system")
                        .into(),
                    -1,
                ),
            };
            DomChain {
                verdict: verdict.into(),
                tone,
                steps: model(steps),
            }
        })
        .collect()
}

fn step(kind: &str, text: String, file: &str, line: u32, tone: i32, depth: i32) -> DrillStep {
    DrillStep {
        kind: kind.into(),
        text: text.into(),
        location: if file.is_empty() {
            Default::default()
        } else {
            format!("{file}:{line}").into()
        },
        file: file.into(),
        line: line as i32,
        tone,
        depth,
        calls: Default::default(),
    }
}

/// "text · service", or the text alone when the service is unknown.
fn at(text: String, service: &str) -> String {
    if service.is_empty() {
        text
    } else {
        format!("{text} · {service}")
    }
}

/// The chain's steps at `depth`, then each continued chain one deeper,
/// after the step that publishes its integration event.
pub fn flatten(cx: Ctx, c: &Chain, depth: i32, out: &mut Vec<DrillStep>) {
    start(cx, c, depth, out);
    domain(cx, c, depth, out);
    let mut last = None;
    for n in &c.continues {
        // One "Publishes" step for the handlers of the same publish site.
        if let (Some(p), ChainStart::Message { message_handler: m }) = (n.via, &n.start)
            && last.replace((p, &m.message)) != Some((p, &m.message))
        {
            let text = at(format!("{} · {}", m.message, p.by), &p.service);
            let kind = cx.texts.t("Publishes");
            out.push(step(kind, text, &p.file, p.line, -1, depth + 1));
        }
        flatten(cx, n, depth + 1, out);
    }
}

/// The screen action, if any, and the endpoint or message handler.
fn start(cx: Ctx, c: &Chain, depth: i32, out: &mut Vec<DrillStep>) {
    let texts = cx.texts;
    if let Some(act) = c.action() {
        let screen = match &act.screen {
            Origin::Screen(id) => (cx.flow.screens.iter())
                .find(|s| s.id == *id)
                .map_or(id.clone(), |s| s.name.clone()),
            Origin::AnyScreen { application } => {
                texts.f("every screen of {name}", &[("name", application.clone())])
            }
        };
        let what = act.trigger.clone().unwrap_or_else(|| words::call(act));
        let text = format!("{screen} · {what}");
        out.push(step(
            texts.t("Screen action"),
            text,
            &act.file,
            act.line,
            -1,
            depth,
        ));
    }
    match &c.start {
        ChainStart::Endpoint { endpoint: e, .. } => {
            let outcome = evaluate_access(&e.access, cx.a);
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
            let text = at(text, cx.sv.of(&e.file));
            out.push(step(
                texts.t("Endpoint"),
                text,
                &e.file,
                e.line,
                tone,
                depth,
            ));
        }
        ChainStart::Message { message_handler: m } => {
            let text = at(format!("{} → {}", m.message, m.id), &m.service);
            let kind = texts.t("Message handler");
            out.push(step(kind, text, &m.file, m.line, -1, depth));
        }
    }
}

/// Use cases, transitions, events with their handlers' declarations.
fn domain(cx: Ctx, c: &Chain, depth: i32, out: &mut Vec<DrillStep>) {
    let (texts, sv) = (cx.texts, cx.sv);
    for u in &c.use_cases {
        let text = at(u.name.clone(), sv.of(&u.file));
        out.push(step(texts.t("Use case"), text, &u.file, u.line, -1, depth));
    }
    for t in &c.transitions {
        let x = t.transition;
        let from = if x.from.is_empty() {
            "*".to_string()
        } else {
            x.from.join(", ")
        };
        let text = format!("{}.{}: {from} → {} ({})", t.entity, t.field, x.to, x.method);
        out.push(step(
            texts.t("Transition"),
            text,
            &x.file,
            x.line,
            -1,
            depth,
        ));
    }
    for ev in &c.events {
        let handlers = super::focus::or_none(&ev.handlers, texts);
        let text = format!("{} → {handlers}", ev.event);
        out.push(step(texts.t("Event"), text, "", 0, -1, depth));
        for h in cx.d.event_handler_sites(ev.event) {
            let text = at(h.handler.clone(), sv.of(&h.file));
            let kind = texts.t("Event handler");
            out.push(step(kind, text, &h.file, h.line, -1, depth + 1));
        }
    }
}
