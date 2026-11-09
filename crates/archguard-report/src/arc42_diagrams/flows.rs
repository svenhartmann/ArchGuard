//! Chapter 6 for service systems (ADR 0006): one runtime scenario per
//! entry route (a route no other service calls), as in arc42's example
//! runtime views. The user calls the entry service; each route's calls to
//! other services follow in source order, into the routes they reach.
//! Below: the errors those routes raise and the timeouts, retries and
//! circuit breakers found for the services involved. Static: no measured
//! order or duration.

use archguard_core::{
    Diagram, DiagramElement, DiagramKind, DiagramRelation, DocLang, ElementKind, RelationKind,
    RouteStep, ServiceRoute, ServiceTopology,
};

use crate::arc42_text::fill;
use std::collections::BTreeSet;

const MAX_FLOWS: usize = 6;
const MAX_DEPTH: usize = 6;
const MAX_MESSAGES: usize = 40;

pub fn diagrams(l: DocLang, t: &ServiceTopology) -> Vec<Diagram> {
    let mut flows: Vec<(usize, Diagram)> = entries(t)
        .map(|entry| Flow::new(l, t).build(entry))
        .filter(|(services, _)| *services > 1)
        .collect();
    flows.sort_by_key(|(services, _)| std::cmp::Reverse(*services));
    flows.into_iter().take(MAX_FLOWS).map(|(_, d)| d).collect()
}

/// Routes no other service calls: where a scenario starts.
pub(super) fn entries(t: &ServiceTopology) -> impl Iterator<Item = &ServiceRoute> {
    let called: BTreeSet<(&str, &str, &str)> = t
        .routes
        .iter()
        .flat_map(|r| {
            r.steps
                .iter()
                .map(|s| (s.to.as_str(), s.method.as_str(), s.path.as_str()))
        })
        .collect();
    t.routes.iter().filter(move |r| {
        !called.contains(&(r.service.as_str(), r.method.as_str(), r.path.as_str()))
    })
}

/// The route `step` reaches, if its service declares it.
pub(super) fn target<'a>(t: &'a ServiceTopology, step: &RouteStep) -> Option<&'a ServiceRoute> {
    t.routes
        .iter()
        .find(|r| r.service == step.to && r.method == step.method && r.path == step.path)
}

/// One message of a scenario, between participant indices.
pub(super) struct Message {
    pub from: usize,
    pub to: usize,
    pub text: String,
    pub error: bool,
    pub event: bool,
    /// The message (index) this one is sent within.
    pub caller: Option<usize>,
    /// The explicit answer to `caller`.
    pub reply: bool,
}

impl Message {
    /// A call or publication along `step`.
    pub(super) fn step(from: usize, to: usize, step: &RouteStep, caller: Option<usize>) -> Self {
        let together = if step.parallel && !event(&step.method) {
            " (parallel)"
        } else {
            ""
        };
        Message {
            from,
            to,
            text: format!("{}{together}", trigger(&step.method, &step.path)),
            error: false,
            event: event(&step.method),
            caller,
            reply: false,
        }
    }
}

/// Consumers are routes with method `EVENT` and the topic as path.
pub(super) fn event(method: &str) -> bool {
    method == "EVENT"
}

/// `POST /checkout`, or `Event order.placed (async)` for a topic.
pub(super) fn trigger(method: &str, path: &str) -> String {
    if event(method) {
        format!("Event {path} (async)")
    } else {
        format!("{method} {path}")
    }
}

/// Who starts `entry`: the user, or for a consumer whose publisher is
/// not in the source, an external sender.
pub(super) fn starter(entry: &ServiceRoute) -> &'static str {
    if event(&entry.method) {
        "external"
    } else {
        "user"
    }
}

/// The display name of a participant id: the user, an external sender,
/// a topic without consumer in the source, or a service.
pub(super) fn name(l: DocLang, t: &ServiceTopology, id: &str) -> String {
    match id {
        "user" => return l.pick("User", "Nutzer").into(),
        "external" => return l.pick("External sender", "Externer Absender").into(),
        _ => {}
    }
    if let Some(topic) = id.strip_prefix("topic:") {
        return format!("Topic {topic}");
    }
    t.services
        .iter()
        .find(|s| s.id == id)
        .map_or_else(|| id.to_string(), |s| s.name.clone())
}

/// Timeouts, retries, circuit breakers and fallbacks of the `involved`
/// services, as one sentence part.
pub(super) fn handling(l: DocLang, t: &ServiceTopology, involved: &BTreeSet<&str>) -> String {
    let mut found: Vec<String> = Vec::new();
    for r in t
        .resilience
        .iter()
        .filter(|r| involved.contains(r.service.as_str()))
    {
        let text = format!("{} {} ({})", kind(&r.kind), r.detail, r.file);
        if !found.contains(&text) {
            found.push(text);
        }
    }
    if found.is_empty() {
        l.pick(
            "no timeouts, retries or circuit breakers found",
            "keine Timeouts, Retries oder Circuit Breaker gefunden",
        )
        .into()
    } else {
        found.join("; ")
    }
}

/// Participants as diagram elements: the person first, then services.
pub(super) fn elements(l: DocLang, t: &ServiceTopology, order: &[String]) -> Vec<DiagramElement> {
    order
        .iter()
        .enumerate()
        .map(|(i, id)| DiagramElement {
            id: format!("e{i}"),
            name: name(l, t, id),
            kind: match id.as_str() {
                "user" => ElementKind::Person,
                "external" => ElementKind::External,
                _ if id.starts_with("topic:") => ElementKind::External,
                _ => ElementKind::Container,
            },
            technology: None,
            detail: None,
            group: None,
            cycle: false,
        })
        .collect()
}

/// Numbered relations; publications are events.
pub(super) fn relations(messages: &[Message]) -> Vec<DiagramRelation> {
    messages
        .iter()
        .enumerate()
        .map(|(n, m)| DiagramRelation {
            from: format!("e{}", m.from),
            to: format!("e{}", m.to),
            kind: if m.event {
                RelationKind::Event
            } else {
                RelationKind::Http
            },
            count: 1,
            cycle: false,
            label: Some(format!("{}. {}", n + 1, m.text)),
            error: m.error,
            caller: m.caller,
            reply: m.reply,
        })
        .collect()
}

struct Flow<'a> {
    l: DocLang,
    t: &'a ServiceTopology,
    /// Participants: "user", then service ids in first-use order.
    order: Vec<String>,
    messages: Vec<Message>,
    routes: Vec<&'a ServiceRoute>,
}

impl<'a> Flow<'a> {
    fn new(l: DocLang, t: &'a ServiceTopology) -> Self {
        Flow {
            l,
            t,
            order: Vec::new(),
            messages: Vec::new(),
            routes: Vec::new(),
        }
    }

    fn at(&mut self, id: &str) -> usize {
        match self.order.iter().position(|x| x == id) {
            Some(i) => i,
            None => {
                self.order.push(id.into());
                self.order.len() - 1
            }
        }
    }

    /// The scenario of `entry` and how many services it involves.
    fn build(mut self, entry: &'a ServiceRoute) -> (usize, Diagram) {
        self.at(starter(entry));
        let to = self.at(&entry.service);
        self.messages.push(Message {
            from: 0,
            to,
            text: trigger(&entry.method, &entry.path),
            error: false,
            event: event(&entry.method),
            caller: None,
            reply: false,
        });
        let mut seen = BTreeSet::new();
        self.visit(entry, 0, &mut seen, 0);
        let services = self.order.len() - 1;
        (services, self.diagram(entry))
    }

    fn visit(
        &mut self,
        route: &'a ServiceRoute,
        depth: usize,
        seen: &mut BTreeSet<(String, String, String)>,
        // The message that entered `route`.
        caller: usize,
    ) {
        self.routes.push(route);
        let from = self.at(&route.service);
        for step in &route.steps {
            if self.messages.len() >= MAX_MESSAGES {
                return;
            }
            let to = self.at(&step.to);
            self.messages
                .push(Message::step(from, to, step, Some(caller)));
            let sent = self.messages.len() - 1;
            let key = (step.to.clone(), step.method.clone(), step.path.clone());
            if depth + 1 < MAX_DEPTH
                && seen.insert(key)
                && let Some(next) = target(self.t, step)
            {
                self.visit(next, depth + 1, seen, sent);
            }
        }
    }

    fn name(&self, id: &str) -> String {
        name(self.l, self.t, id)
    }

    fn diagram(&self, entry: &ServiceRoute) -> Diagram {
        let elements = elements(self.l, self.t, &self.order);
        let relations = relations(&self.messages);
        let entry_service = self.name(&entry.service);
        Diagram {
            id: format!(
                "runtime-flow-{}",
                slug(&format!("{entry_service} {} {}", entry.method, entry.path))
            ),
            chapter: 6,
            title: format!(
                "{} · {} · {entry_service}",
                self.l.pick("Flow", "Ablauf"),
                trigger(&entry.method, &entry.path).trim_end_matches(" (async)")
            ),
            kind: DiagramKind::Sequence,
            note: self.note(entry),
            unit: self.l.pick("calls", "Aufrufe").into(),
            elements,
            relations,
            shown: true,
        }
    }

    /// Trigger, errors raised on the way and the error handling found.
    fn note(&self, entry: &ServiceRoute) -> String {
        let mut failures: Vec<String> = Vec::new();
        for r in &self.routes {
            for f in &r.failures {
                let (open, close) = self.l.pick(("“", "”"), ("„", "“"));
                let text = format!(
                    "{} {} {open}{}{close}",
                    self.name(&r.service),
                    f.status,
                    f.message
                );
                if !failures.contains(&text) {
                    failures.push(text);
                }
            }
        }
        let involved: BTreeSet<&str> = self.order.iter().skip(1).map(String::as_str).collect();
        let l = self.l;
        let text = l.pick(
            "Trigger: {} to {} ({}:{}). Steps from the services' source in source order, derived statically: no measured order or duration, branches not told apart. Aborts on the way: {}. Error handling: {}.{}",
            "Auslöser: {} an {} ({}:{}). Schritte aus dem Quelltext der Dienste in Quelltext-Reihenfolge, statisch abgeleitet: keine gemessene Reihenfolge oder Dauer, Verzweigungen nicht unterschieden. Abbrüche unterwegs: {}. Fehlerbehandlung: {}.{}",
        );
        let aborts = if failures.is_empty() {
            l.pick("none found", "keine gefunden").into()
        } else {
            failures.join("; ")
        };
        let events = if self.messages.iter().any(|m| m.event) {
            l.pick(
                " Events run asynchronously: the sender does not wait for the receivers, errors there do not reach it.",
                " Events laufen asynchron: der Absender wartet nicht auf die Empfänger, Fehler dort erreichen ihn nicht.",
            )
        } else {
            ""
        };
        fill(
            text,
            &[
                &trigger(&entry.method, &entry.path),
                &self.name(&entry.service),
                &entry.file,
                &entry.line,
                &aborts,
                &handling(l, self.t, &involved),
                &events,
            ],
        )
    }
}

pub(crate) fn kind(k: &str) -> &str {
    match k {
        "timeout" => "Timeout",
        "retry" => "Retry",
        "circuit_breaker" => "Circuit Breaker",
        "fallback" => "Fallback",
        other => other,
    }
}

pub(super) fn slug(text: &str) -> String {
    let s: String = text
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    s.split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{RouteFailure, ServiceResilience};

    fn route(
        service: &str,
        method: &str,
        path: &str,
        steps: &[(&str, &str, &str)],
    ) -> ServiceRoute {
        ServiceRoute {
            service: service.into(),
            method: method.into(),
            path: path.into(),
            file: format!("{service}.js"),
            line: 3,
            steps: steps
                .iter()
                .map(|(to, m, p)| RouteStep {
                    to: (*to).into(),
                    method: (*m).into(),
                    path: (*p).into(),
                    line: 4,
                    parallel: false,
                })
                .collect(),
            failures: vec![],
            catches: false,
        }
    }

    #[test]
    fn an_entry_route_becomes_a_flow_through_the_routes_it_reaches() {
        let mut t = ServiceTopology::default();
        for name in ["gateway", "orders", "payments"] {
            t.services.push(archguard_core::Service {
                id: format!("c::{name}"),
                name: name.into(),
                compose_file: "c".into(),
                build_context: None,
                networks: vec![],
                profiles: vec![],
                project: None,
            });
        }
        t.routes = vec![
            route(
                "c::gateway",
                "POST",
                "/checkout",
                &[("c::orders", "POST", "/checkout")],
            ),
            route(
                "c::orders",
                "POST",
                "/checkout",
                &[("c::payments", "POST", "/authorize")],
            ),
            route("c::payments", "POST", "/authorize", &[]),
        ];
        t.routes[2].failures.push(RouteFailure {
            status: 409,
            message: "Rejected".into(),
            line: 9,
        });
        t.resilience.push(ServiceResilience {
            service: "c::orders".into(),
            kind: "timeout".into(),
            detail: "3000 ms".into(),
            file: "http.js".into(),
            line: 2,
        });
        let all = diagrams(DocLang::De, &t);
        assert_eq!(all.len(), 1, "only the gateway route is an entry");
        let d = &all[0];
        assert_eq!(d.title, "Ablauf · POST /checkout · gateway");
        let names: Vec<&str> = d.elements.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Nutzer", "gateway", "orders", "payments"]);
        let labels: Vec<&str> = d
            .relations
            .iter()
            .filter_map(|r| r.label.as_deref())
            .collect();
        assert_eq!(
            labels,
            [
                "1. POST /checkout",
                "2. POST /checkout",
                "3. POST /authorize"
            ]
        );
        assert!(d.note.contains("payments 409 „Rejected“"), "{}", d.note);
        assert!(d.note.contains("Timeout 3000 ms (http.js)"), "{}", d.note);
    }
}
