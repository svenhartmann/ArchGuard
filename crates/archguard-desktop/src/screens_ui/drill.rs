//! The drill-down of one action (ADR 0021 with ADR 0022 decision 4): the
//! action, the endpoints its call matches with their answer for the
//! simulated user, the endpoint's handler method, and from
//! `domain_chain::chains` its use cases, state transitions, domain events
//! and the integration events that continue in other services.

use archguard_engine::core::domain_model::DomainModel;
use archguard_engine::core::{
    Assignment, EndpointAccess, Outcome, UiFlow, evaluate_access, join_actions_to_endpoints,
};
use archguard_engine::domain_chain::{Chain, ChainStart, chains};

use super::words;
use crate::tab_support::Texts;
use crate::ui::DrillStep;

/// One step; `depth` indents it under the step it follows from.
fn step(kind: &str, text: String, file: &str, line: u32, depth: i32) -> DrillStep {
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
        tone: -1,
        depth,
        calls: Default::default(),
    }
}

/// The steps from action `a` (index into `flow.actions`) to the services.
pub fn steps(
    flow: &UiFlow,
    model: Option<&DomainModel>,
    a: usize,
    who: &Assignment,
    texts: &Texts,
) -> Vec<DrillStep> {
    let Some(action) = flow.actions.get(a) else {
        return vec![];
    };
    let call = words::call(action);
    let what = match &action.trigger {
        Some(t) => format!("{t} · {call}"),
        None => call.clone(),
    };
    let mut out = vec![step(texts.t("Action"), what, &action.file, action.line, 0)];
    let joined = join_actions_to_endpoints(flow);
    let hits = (joined.iter())
        .find(|(j, _)| std::ptr::eq(*j, action))
        .map_or(&[][..], |(_, h)| h.as_slice());
    if hits.is_empty() {
        let none = texts.f(
            "No backend endpoint found for {call}",
            &[("call", call.clone())],
        );
        let mut s = step(texts.t("Endpoint"), none, "", 0, 1);
        s.tone = 1;
        out.push(s);
        return out;
    }
    let all = model.map(|m| chains(flow, m)).unwrap_or_default();
    let empty = DomainModel::default();
    let d = model.unwrap_or(&empty);
    for e in hits {
        out.push(endpoint(e, who, texts));
        if let Some(h) = &e.handler {
            // The declaration the extractor read, else the endpoint's place.
            let (file, line) = flow
                .handler_site(e)
                .map_or((e.file.as_str(), e.line), |x| (x.file.as_str(), x.line));
            let mut s = step(texts.t("Handler"), h.clone(), file, line, 2);
            s.calls = h.as_str().into();
            out.push(s);
        }
        let chain = (all.iter()).find(|c| {
            c.endpoint().is_some_and(|x| std::ptr::eq(x, *e))
                && c.action().is_some_and(|x| std::ptr::eq(x, action))
        });
        match chain {
            Some(c) if !c.use_cases.is_empty() => domain(&mut out, d, c, 2, texts),
            _ => out.push(step(
                texts.t("Use case"),
                texts.t("No use case read for this endpoint.").into(),
                "",
                0,
                2,
            )),
        }
    }
    out
}

fn endpoint(e: &EndpointAccess, who: &Assignment, texts: &Texts) -> DrillStep {
    let outcome = evaluate_access(&e.access, who.in_app(e.application.as_deref()));
    let text = format!(
        "{} {} · {} · {}",
        e.method,
        e.path,
        words::endpoint_access(texts, &e.access),
        words::outcome(texts, outcome)
    );
    let mut s = step(texts.t("Endpoint"), text, &e.file, e.line, 1);
    s.tone = match outcome {
        Outcome::Allowed => 0,
        Outcome::Denied => 1,
        Outcome::Unknown => 2,
    };
    s
}

/// A chain's use cases, transitions, events and its continuations in
/// other services, at `depth`.
fn domain(out: &mut Vec<DrillStep>, d: &DomainModel, c: &Chain, depth: i32, texts: &Texts) {
    for u in &c.use_cases {
        let mut s = step(texts.t("Use case"), u.name.clone(), &u.file, u.line, depth);
        s.calls = u.name.as_str().into();
        out.push(s);
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
            depth + 1,
        ));
    }
    for ev in &c.events {
        let handlers = if ev.handlers.is_empty() {
            texts.t("none").to_string()
        } else {
            ev.handlers.join(", ")
        };
        let text = format!("{} → {handlers}", ev.event);
        out.push(step(texts.t("Domain event"), text, "", 0, depth + 1));
        for h in d.event_handler_sites(ev.event) {
            let mut s = step(
                texts.t("Event handler"),
                h.handler.clone(),
                &h.file,
                h.line,
                depth + 2,
            );
            s.calls = h.handler.as_str().into();
            out.push(s);
        }
    }
    for n in &c.continues {
        let ChainStart::Message { message_handler: m } = &n.start else {
            continue;
        };
        let (file, line, by) = n.via.map_or(("", 0, String::new()), |p| {
            let by = texts.f(
                "published by {by} in {service}",
                &[("by", p.by.clone()), ("service", p.service.clone())],
            );
            (p.file.as_str(), p.line, by)
        });
        let text = format!("{} · {by}", m.message);
        out.push(step(
            texts.t("Integration event"),
            text,
            file,
            line,
            depth + 1,
        ));
        let service = if m.service.is_empty() {
            String::new()
        } else {
            format!(" · {}", m.service)
        };
        let text = format!("{}{service}", m.id);
        let mut s = step(texts.t("Message handler"), text, &m.file, m.line, depth + 2);
        s.calls = m.id.as_str().into();
        out.push(s);
        domain(out, d, n, depth + 3, texts);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_engine::core::domain_model::{
        IntegrationEvent, MessageHandler, Publisher, UseCase, UseCaseKind,
    };
    use archguard_engine::core::{AccessState, Action, Call, Condition, Origin};

    fn use_case(name: &str, request: &str, file: &str) -> UseCase {
        UseCase {
            id: name.into(),
            name: name.into(),
            kind: UseCaseKind::Command,
            request: Some(request.into()),
            calls: vec![],
            file: file.into(),
            line: 10,
            source: "t".into(),
        }
    }

    /// eShop's "Place order": POST /api/orders → CreateOrderCommandHandler,
    /// which publishes an event the Payment service handles.
    fn shop() -> (UiFlow, DomainModel) {
        let action = |path: &str| Action {
            weak: false,
            screen: Origin::Screen("checkout".into()),
            trigger: Some("Place order".into()),
            call: Call::Http {
                method: "POST".into(),
                path: path.into(),
            },
            file: "Checkout.razor".into(),
            line: 114,
            condition: archguard_engine::core::AccessState::Undeclared,
            source: "t".into(),
        };
        let flow = UiFlow {
            actions: vec![action("/api/orders"), action("/api/x")],
            endpoints: vec![EndpointAccess {
                method: "POST".into(),
                path: "/api/orders".into(),
                access: AccessState::Protected(Condition::Authenticated),
                application: None,
                file: "OrdersApi.cs".into(),
                line: 20,
                source: "t".into(),
                handler: Some("OrdersApi.CreateOrderAsync".into()),
                sends: vec!["CreateOrderCommand".into()],
            }],
            handler_sites: vec![archguard_engine::core::HandlerSite {
                endpoint_file: "OrdersApi.cs".into(),
                endpoint_line: 20,
                handler: "OrdersApi.CreateOrderAsync".into(),
                file: "OrdersApi.cs".into(),
                line: 3,
            }],
            ..UiFlow::default()
        };
        let model = DomainModel {
            use_cases: vec![
                use_case(
                    "CreateOrderCommandHandler",
                    "CreateOrderCommand",
                    "Create.cs",
                ),
                use_case("PayHandler", "PayCommand", "Pay.cs"),
            ],
            message_handlers: vec![MessageHandler {
                id: "OrderStartedHandler".into(),
                message: "OrderStarted".into(),
                sends: vec!["PayCommand".into()],
                file: "Payment/OrderStartedHandler.cs".into(),
                line: 7,
                source: "t".into(),
                service: "Payment".into(),
            }],
            integration_events: vec![IntegrationEvent {
                type_name: "OrderStarted".into(),
                published_by: vec![Publisher {
                    by: "CreateOrderCommandHandler.Handle".into(),
                    service: "Ordering".into(),
                    file: "Create.cs".into(),
                    line: 30,
                }],
                handled_by: vec!["OrderStartedHandler".into()],
                source: "t".into(),
            }],
            ..DomainModel::default()
        };
        (flow, model)
    }

    fn rows(steps: &[DrillStep]) -> Vec<(i32, String, String, String)> {
        (steps.iter())
            .map(|s| {
                let (d, k, t, l) = (s.depth, &s.kind, &s.text, &s.location);
                (d, k.to_string(), t.to_string(), l.to_string())
            })
            .collect()
    }

    #[test]
    fn an_action_leads_to_its_endpoint_handler_use_case_and_other_services() {
        let (flow, model) = shop();
        let who = Assignment::default();
        let s = steps(&flow, Some(&model), 0, &who, &Texts::default());
        let r = |d: i32, k: &str, t: &str, l: &str| (d, k.into(), t.into(), l.into());
        assert_eq!(
            rows(&s),
            [
                r(
                    0,
                    "Action",
                    "Place order · POST /api/orders",
                    "Checkout.razor:114"
                ),
                r(
                    1,
                    "Endpoint",
                    "POST /api/orders · signed in · denied",
                    "OrdersApi.cs:20"
                ),
                r(2, "Handler", "OrdersApi.CreateOrderAsync", "OrdersApi.cs:3"),
                r(2, "Use case", "CreateOrderCommandHandler", "Create.cs:10"),
                r(
                    3,
                    "Integration event",
                    "OrderStarted · published by CreateOrderCommandHandler.Handle in Ordering",
                    "Create.cs:30"
                ),
                r(
                    4,
                    "Message handler",
                    "OrderStartedHandler · Payment",
                    "Payment/OrderStartedHandler.cs:7"
                ),
                r(5, "Use case", "PayHandler", "Pay.cs:10"),
            ]
        );
        assert_eq!(s[1].tone, 1, "anonymous is refused");
        assert_eq!(s[2].calls, "OrdersApi.CreateOrderAsync", "Show calls");
        let mut signed_in = who.clone();
        signed_in.authenticated = true;
        let s = steps(&flow, Some(&model), 0, &signed_in, &Texts::default());
        assert_eq!(
            (s[1].tone, s[1].text.as_str()),
            (0, "POST /api/orders · signed in · allowed")
        );
    }

    #[test]
    fn an_unmatched_call_says_so_plainly() {
        let (flow, model) = shop();
        let s = steps(
            &flow,
            Some(&model),
            1,
            &Assignment::default(),
            &Texts::default(),
        );
        assert_eq!(s.len(), 2);
        assert_eq!(s[1].text, "No backend endpoint found for POST /api/x");
        let de = steps(&flow, None, 1, &Assignment::default(), &Texts::german());
        assert_eq!(de[1].text, "Kein Backend-Endpunkt gefunden für POST /api/x");
    }
}
