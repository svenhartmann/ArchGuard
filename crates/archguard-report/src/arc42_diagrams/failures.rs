//! Chapter 6 error scenarios for service systems (ADR 0006), as in
//! arc42's example runtime views: for an error a downstream route raises,
//! the calls from the user down to it, the error response, and the way
//! back in red. A handler without `catch`/`except` passes the error on;
//! one with it stops the red path there. Static: what the source shows,
//! not a measured run.

use archguard_core::{
    Diagram, DiagramKind, RouteFailure, RouteStep, ServiceRoute, ServiceTopology,
};
use std::collections::BTreeSet;

use super::flows::{
    Message, elements, entries, event, handling, name, relations, slug, starter, target, trigger,
};

const MAX_SCENARIOS: usize = 6;
const MAX_DEPTH: usize = 6;

pub fn diagrams(t: &ServiceTopology) -> Vec<Diagram> {
    let mut found = Walk {
        t,
        seen: BTreeSet::new(),
        out: Vec::new(),
    };
    for entry in entries(t) {
        found.walk(&mut vec![entry], &mut Vec::new());
    }
    // Deeper errors first: they cross more services.
    found
        .out
        .sort_by_key(|(depth, _)| std::cmp::Reverse(*depth));
    found
        .out
        .into_iter()
        .take(MAX_SCENARIOS)
        .map(|(_, d)| d)
        .collect()
}

struct Walk<'a> {
    t: &'a ServiceTopology,
    /// Each error once, at the first path that reaches it.
    seen: BTreeSet<(String, u16, String)>,
    out: Vec<(usize, Diagram)>,
}

impl<'a> Walk<'a> {
    /// `chain`: routes from the entry down; `hops[i]`: the step of
    /// `chain[i]` that calls `chain[i + 1]`.
    fn walk(&mut self, chain: &mut Vec<&'a ServiceRoute>, hops: &mut Vec<&'a RouteStep>) {
        let route = chain[chain.len() - 1];
        // An error in a consumer stays there: the publisher does not wait.
        let answered = hops.last().is_some_and(|h| !event(&h.method));
        if answered {
            for f in &route.failures {
                if self
                    .seen
                    .insert((route.service.clone(), f.status, f.message.clone()))
                {
                    let d = Scenario::new(self.t).build(chain, hops, f);
                    self.out.push((chain.len(), d));
                }
            }
        }
        if chain.len() >= MAX_DEPTH {
            return;
        }
        for step in &route.steps {
            if let Some(next) = target(self.t, step)
                && !chain.iter().any(|r| std::ptr::eq(*r, next))
            {
                chain.push(next);
                hops.push(step);
                self.walk(chain, hops);
                chain.pop();
                hops.pop();
            }
        }
    }
}

/// Where the red way back ends.
enum End<'r> {
    User,
    /// A handler with catch/except.
    Caught(&'r ServiceRoute),
    /// A consumer: whoever published the event does not learn of it.
    Async(&'r ServiceRoute),
}

struct Scenario<'a> {
    t: &'a ServiceTopology,
    order: Vec<String>,
    messages: Vec<Message>,
}

impl<'a> Scenario<'a> {
    fn new(t: &'a ServiceTopology) -> Self {
        Scenario {
            t,
            order: Vec::new(),
            messages: Vec::new(),
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

    fn call(&mut self, from: &str, to: &str, step: &RouteStep) {
        let (a, b) = (self.at(from), self.at(to));
        self.messages.push(Message::step(a, b, step));
    }

    fn red(&mut self, from: usize, to: usize, text: String) {
        self.messages.push(Message {
            from,
            to,
            text,
            error: true,
            event: false,
        });
    }

    /// Down to the failing route, then the error back up.
    fn build(mut self, chain: &[&ServiceRoute], hops: &[&RouteStep], f: &RouteFailure) -> Diagram {
        let entry = chain[0];
        self.at(starter(entry));
        let first = self.at(&entry.service);
        self.messages.push(Message {
            from: 0,
            to: first,
            text: trigger(&entry.method, &entry.path),
            error: false,
            event: event(&entry.method),
        });
        for (i, route) in chain.iter().enumerate() {
            let until = hops.get(i).map_or(f.line, |h| h.line);
            for step in route.steps.iter().filter(|s| s.line < until) {
                self.call(&route.service, &step.to, step);
            }
            if let Some(hop) = hops.get(i) {
                self.call(&route.service, &hop.to, hop);
            }
        }
        let end = self.back(chain, hops, f);
        self.diagram(chain, f, end)
    }

    /// The red way back and where it ends.
    fn back<'r>(
        &mut self,
        chain: &[&'r ServiceRoute],
        hops: &[&RouteStep],
        f: &RouteFailure,
    ) -> End<'r> {
        let last = chain.len() - 1;
        for j in (0..last).rev() {
            let (from, to) = (self.at(&chain[j + 1].service), self.at(&chain[j].service));
            let label = if j + 1 == last {
                format!("{} {}", f.status, f.message)
            } else {
                format!("{} weitergereicht", f.status)
            };
            self.red(from, to, label);
            if chain[j].catches {
                return End::Caught(chain[j]);
            }
            if j > 0 && event(&hops[j - 1].method) {
                return End::Async(chain[j]);
            }
        }
        if event(&chain[0].method) {
            return End::Async(chain[0]);
        }
        let from = self.at(&chain[0].service);
        self.red(from, 0, format!("{} weitergereicht", f.status));
        End::User
    }

    fn diagram(&self, chain: &[&ServiceRoute], f: &RouteFailure, end: End<'_>) -> Diagram {
        let (entry, failing) = (chain[0], chain[chain.len() - 1]);
        let service = name(self.t, &failing.service);
        let involved: BTreeSet<&str> = self.order.iter().skip(1).map(String::as_str).collect();
        let end = match end {
            End::Caught(r) => format!(
                "{} fängt den Fehler ab (catch/except im Handler {}); was der Nutzer dann erhält, zeigt der Code nicht eindeutig",
                name(self.t, &r.service),
                trigger(&r.method, &r.path)
            ),
            End::Async(r) => format!(
                "er endet bei {}, das per Event aufgerufen wurde (asynchron): der Absender erfährt davon nichts",
                name(self.t, &r.service)
            ),
            End::User => {
                "kein Handler auf dem Weg fängt ihn ab, der Nutzer erhält den Fehler".into()
            }
        };
        let trigger = trigger(&entry.method, &entry.path);
        Diagram {
            id: format!(
                "runtime-failure-{}",
                slug(&format!("{service} {} {}", f.status, f.message))
            ),
            chapter: 6,
            title: format!(
                "Fehlerfall · {service} {} „{}“ · {}",
                f.status,
                f.message,
                trigger.trim_end_matches(" (async)")
            ),
            kind: DiagramKind::Sequence,
            note: format!(
                "Auslöser: {trigger} an {}. Abbruch: {service} antwortet {} „{}“ ({}:{}). Vorher abgeschlossene Aufrufe stehen ohne ihre Folgeaufrufe. Rückweg rot: ein Handler ohne catch/except reicht den Fehler weiter; {end}. Statisch aus dem Quelltext abgeleitet, nicht gemessen. Fehlerbehandlung: {}.",
                name(self.t, &entry.service),
                f.status,
                f.message,
                failing.file,
                f.line,
                handling(self.t, &involved),
            ),
            unit: "Aufrufe".into(),
            elements: elements(self.t, &self.order),
            relations: relations(&self.messages),
            shown: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::Service;

    fn route(
        service: &str,
        path: &str,
        steps: &[(&str, &str, u32)],
        failure: Option<u32>,
    ) -> ServiceRoute {
        ServiceRoute {
            service: service.into(),
            method: "POST".into(),
            path: path.into(),
            file: format!("{service}.js"),
            line: 1,
            steps: steps
                .iter()
                .map(|(to, p, line)| RouteStep {
                    to: (*to).into(),
                    method: "POST".into(),
                    path: (*p).into(),
                    line: *line,
                    parallel: false,
                })
                .collect(),
            failures: failure
                .map(|line| RouteFailure {
                    status: 409,
                    message: "Rejected".into(),
                    line,
                })
                .into_iter()
                .collect(),
            catches: false,
        }
    }

    fn topology() -> ServiceTopology {
        let mut t = ServiceTopology::default();
        for n in ["gateway", "orders", "payments", "risk"] {
            t.services.push(Service {
                id: n.into(),
                name: n.into(),
                compose_file: "c".into(),
                build_context: None,
                networks: vec![],
                profiles: vec![],
                project: None,
            });
        }
        t.routes = vec![
            route("gateway", "/checkout", &[("orders", "/checkout", 2)], None),
            route(
                "orders",
                "/checkout",
                &[("risk", "/assess", 3), ("payments", "/authorize", 5)],
                None,
            ),
            route("payments", "/authorize", &[], Some(4)),
            route("risk", "/assess", &[], None),
        ];
        t
    }

    fn labels(d: &Diagram) -> Vec<(String, bool)> {
        d.relations
            .iter()
            .map(|r| (r.label.clone().unwrap_or_default(), r.error))
            .collect()
    }

    #[test]
    fn a_downstream_error_goes_back_to_the_user_in_red() {
        let all = diagrams(&topology());
        assert_eq!(all.len(), 1);
        let d = &all[0];
        assert_eq!(
            d.title,
            "Fehlerfall · payments 409 „Rejected“ · POST /checkout"
        );
        let expected = [
            ("1. POST /checkout", false),
            ("2. POST /checkout", false),
            ("3. POST /assess", false),
            ("4. POST /authorize", false),
            ("5. 409 Rejected", true),
            ("6. 409 weitergereicht", true),
            ("7. 409 weitergereicht", true),
        ];
        let expected: Vec<(String, bool)> =
            expected.iter().map(|(l, e)| ((*l).into(), *e)).collect();
        assert_eq!(labels(d), expected);
        assert_eq!(
            d.relations[6].to, "e0",
            "the last red arrow reaches the user"
        );
        assert!(d.note.contains("payments.js:4"), "{}", d.note);
        assert!(
            d.note.contains("der Nutzer erhält den Fehler"),
            "{}",
            d.note
        );
    }

    #[test]
    fn a_catching_handler_stops_the_red_path() {
        let mut t = topology();
        t.routes[1].catches = true;
        let d = &diagrams(&t)[0];
        let red: Vec<(String, bool)> = labels(d).into_iter().filter(|(_, e)| *e).collect();
        assert_eq!(red, [("5. 409 Rejected".to_string(), true)]);
        assert!(d.note.contains("orders fängt den Fehler ab"), "{}", d.note);
    }

    #[test]
    fn errors_of_the_entry_route_itself_get_no_scenario() {
        let mut t = topology();
        t.routes[2].failures.clear();
        t.routes[0].failures.push(RouteFailure {
            status: 400,
            message: "Bad".into(),
            line: 1,
        });
        assert!(diagrams(&t).is_empty());
    }

    /// orders publishes `order.placed` (shipping consumes it) and `audit`
    /// (no consumer); shipping calls carrier, which fails.
    fn evented() -> ServiceTopology {
        let mut t = ServiceTopology::default();
        for n in ["orders", "shipping", "carrier"] {
            t.services.push(Service {
                id: n.into(),
                name: n.into(),
                compose_file: "c".into(),
                build_context: None,
                networks: vec![],
                profiles: vec![],
                project: None,
            });
        }
        let event = |to: &str, topic: &str, line| RouteStep {
            to: to.into(),
            method: "EVENT".into(),
            path: topic.into(),
            line,
            parallel: false,
        };
        let mut orders = route("orders", "/orders", &[], None);
        orders.steps = vec![
            event("shipping", "order.placed", 2),
            event("topic:audit", "audit", 3),
        ];
        let mut shipping = route(
            "shipping",
            "order.placed",
            &[("carrier", "/labels", 2)],
            None,
        );
        shipping.method = "EVENT".into();
        t.routes = vec![orders, shipping, route("carrier", "/labels", &[], Some(3))];
        t
    }

    #[test]
    fn events_are_async_and_errors_stop_at_the_consumer() {
        let t = evented();
        let flows = super::super::flows::diagrams(&t);
        assert_eq!(flows.len(), 1, "the consumer is reached, not an entry");
        let f = &flows[0];
        let names: Vec<&str> = f.elements.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            ["Nutzer", "orders", "shipping", "carrier", "Topic audit"]
        );
        assert_eq!(f.elements[4].kind, archguard_core::ElementKind::External);
        let kinds: Vec<(String, archguard_core::RelationKind)> = f
            .relations
            .iter()
            .map(|r| (r.label.clone().unwrap_or_default(), r.kind))
            .collect();
        use archguard_core::RelationKind::{Event, Http};
        let expected = [
            ("1. POST /orders", Http),
            ("2. Event order.placed (async)", Event),
            ("3. POST /labels", Http),
            ("4. Event audit (async)", Event),
        ];
        let expected: Vec<(String, archguard_core::RelationKind)> =
            expected.iter().map(|(l, k)| ((*l).into(), *k)).collect();
        assert_eq!(kinds, expected);
        assert!(f.note.contains("Events laufen asynchron"), "{}", f.note);

        let d = &diagrams(&t)[0];
        let red: Vec<(String, bool)> = labels(d).into_iter().filter(|(_, e)| *e).collect();
        assert_eq!(
            red,
            [("4. 409 Rejected".to_string(), true)],
            "no red arrow back to orders"
        );
        assert!(d.note.contains("er endet bei shipping"), "{}", d.note);
    }

    #[test]
    fn a_consumer_without_publisher_in_the_source_has_an_external_sender() {
        let mut t = evented();
        t.routes.remove(0);
        let f = &super::super::flows::diagrams(&t)[0];
        assert_eq!(f.title, "Ablauf · Event order.placed · shipping");
        assert_eq!(f.elements[0].name, "Externer Absender");
        let d = &diagrams(&t)[0];
        assert!(d.note.contains("er endet bei shipping"), "{}", d.note);
        assert_eq!(d.relations.last().unwrap().to, "e1", "red ends at shipping");
    }
}
