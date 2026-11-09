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

use super::flows::{elements, entries, handling, name, relations, slug, target};

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
        if chain.len() > 1 {
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

struct Scenario<'a> {
    t: &'a ServiceTopology,
    order: Vec<String>,
    messages: Vec<(usize, usize, String, bool)>,
}

impl<'a> Scenario<'a> {
    fn new(t: &'a ServiceTopology) -> Self {
        Scenario {
            t,
            order: vec!["user".into()],
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
        self.messages
            .push((a, b, format!("{} {}", step.method, step.path), false));
    }

    /// Down to the failing route, then the error back up.
    fn build(mut self, chain: &[&ServiceRoute], hops: &[&RouteStep], f: &RouteFailure) -> Diagram {
        let entry = chain[0];
        let first = self.at(&entry.service);
        self.messages
            .push((0, first, format!("{} {}", entry.method, entry.path), false));
        for (i, route) in chain.iter().enumerate() {
            let until = hops.get(i).map_or(f.line, |h| h.line);
            for step in route.steps.iter().filter(|s| s.line < until) {
                self.call(&route.service, &step.to, step);
            }
            if let Some(hop) = hops.get(i) {
                self.call(&route.service, &hop.to, hop);
            }
        }
        let caught = self.back(chain, f);
        self.diagram(chain, f, caught)
    }

    /// The red way back; the route that catches the error, if any.
    fn back<'r>(
        &mut self,
        chain: &[&'r ServiceRoute],
        f: &RouteFailure,
    ) -> Option<&'r ServiceRoute> {
        let last = chain.len() - 1;
        let text = format!("{} {}", f.status, f.message);
        for j in (0..last).rev() {
            let (from, to) = (self.at(&chain[j + 1].service), self.at(&chain[j].service));
            let label = if j + 1 == last {
                text.clone()
            } else {
                format!("{} weitergereicht", f.status)
            };
            self.messages.push((from, to, label, true));
            if chain[j].catches {
                return Some(chain[j]);
            }
        }
        let from = self.at(&chain[0].service);
        self.messages
            .push((from, 0, format!("{} weitergereicht", f.status), true));
        None
    }

    fn diagram(
        &self,
        chain: &[&ServiceRoute],
        f: &RouteFailure,
        caught: Option<&ServiceRoute>,
    ) -> Diagram {
        let (entry, failing) = (chain[0], chain[chain.len() - 1]);
        let service = name(self.t, &failing.service);
        let involved: BTreeSet<&str> = self.order.iter().skip(1).map(String::as_str).collect();
        let end = match caught {
            Some(r) => format!(
                "{} fängt den Fehler ab (catch/except im Handler {} {}); was der Nutzer dann erhält, zeigt der Code nicht eindeutig",
                name(self.t, &r.service),
                r.method,
                r.path
            ),
            None => "kein Handler auf dem Weg fängt ihn ab, der Nutzer erhält den Fehler".into(),
        };
        Diagram {
            id: format!(
                "runtime-failure-{}",
                slug(&format!("{service} {} {}", f.status, f.message))
            ),
            chapter: 6,
            title: format!(
                "Fehlerfall · {service} {} „{}“ · {} {}",
                f.status, f.message, entry.method, entry.path
            ),
            kind: DiagramKind::Sequence,
            note: format!(
                "Auslöser: {} {} an {}. Abbruch: {service} antwortet {} „{}“ ({}:{}). Vorher abgeschlossene Aufrufe stehen ohne ihre Folgeaufrufe. Rückweg rot: ein Handler ohne catch/except reicht den Fehler weiter; {end}. Statisch aus dem Quelltext abgeleitet, nicht gemessen. Fehlerbehandlung: {}.",
                entry.method,
                entry.path,
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
}
