//! arc42 chapter 8, error handling between services (ADR 0006): the error
//! responses each route raises, whether its handler catches errors, and
//! the timeouts, retries and circuit breakers found. Static, from the
//! source; the error scenarios in chapter 6 draw the same findings.

use std::fmt::Write;

use archguard_core::ServiceTopology;

const LIMIT: usize = 40;

/// Closes chapter 8: the section and the question for what the source
/// cannot show.
pub(crate) fn add(
    markdown: &mut String,
    questions: &mut Vec<String>,
    t: &ServiceTopology,
    cell: fn(&str) -> String,
) {
    markdown.push_str(&section(t, cell));
    questions.push("Sicherheits-, Fehlerbehandlungs-, Persistenz-, Transaktions- und Observability-Konzepte anhand zusätzlicher Belege beschreiben.".into());
}

/// The section, or nothing when the source shows no routes.
pub(crate) fn section(t: &ServiceTopology, cell: fn(&str) -> String) -> String {
    if t.routes.is_empty() {
        return String::new();
    }
    let name = |id: &str| {
        t.services
            .iter()
            .find(|s| s.id == id)
            .map_or(id, |s| s.name.as_str())
            .to_string()
    };
    let mut out = String::from(
        "\n### Fehlerbehandlung zwischen Diensten\n\nQuelle: Routen der Dienste im Quelltext, statisch erfasst. „Fängt ab“ heißt: der Handler enthält catch, except, rescue, recover() oder err != nil; ohne das reicht er Fehler seiner Aufrufe weiter.\n\n| Dienst | Route | Antwort | Meldung | Fängt ab | Quelle |\n| --- | --- | --- | --- | --- | --- |\n",
    );
    let rows: Vec<String> = t
        .routes
        .iter()
        .flat_map(|r| {
            r.failures.iter().map(move |f| {
                format!(
                    "| {} | {} {} | {} | {} | {} | {}:{} |",
                    cell(&name(&r.service)),
                    r.method,
                    cell(&r.path),
                    f.status,
                    cell(&f.message),
                    if r.catches { "ja" } else { "nein" },
                    cell(&r.file),
                    f.line
                )
            })
        })
        .collect();
    if rows.is_empty() {
        out.push_str("| – | – | – | Keine Fehlerantworten im Quelltext gefunden. | – | – |\n");
    }
    for row in rows.iter().take(LIMIT) {
        let _ = writeln!(out, "{row}");
    }
    if rows.len() > LIMIT {
        let _ = writeln!(
            out,
            "\n{} weitere Fehlerantworten nicht gezeigt.",
            rows.len() - LIMIT
        );
    }
    out.push_str("\nTimeouts, Retries, Circuit Breaker und Fallbacks:\n\n");
    if t.resilience.is_empty() {
        out.push_str("Keine gefunden.\n");
    }
    // One shared client serves many services: one line per place.
    let mut places: Vec<(&archguard_core::ServiceResilience, Vec<String>)> = Vec::new();
    for r in &t.resilience {
        let same = |p: &&mut (&archguard_core::ServiceResilience, Vec<String>)| {
            (&p.0.kind, &p.0.detail, &p.0.file, p.0.line) == (&r.kind, &r.detail, &r.file, r.line)
        };
        match places.iter_mut().find(same) {
            Some((_, services)) => services.push(name(&r.service)),
            None => places.push((r, vec![name(&r.service)])),
        }
    }
    for (r, services) in places.iter().take(LIMIT) {
        let _ = writeln!(
            out,
            "- {} {} ({}:{}): {}",
            crate::arc42_diagrams::resilience_kind(&r.kind),
            cell(&r.detail),
            cell(&r.file),
            r.line,
            cell(&services.join(", "))
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{RouteFailure, ServiceResilience, ServiceRoute};

    #[test]
    fn failures_and_resilience_become_a_table_and_a_list() {
        let mut t = ServiceTopology::default();
        assert_eq!(section(&t, |s| s.to_string()), "", "no routes, no section");
        t.routes.push(ServiceRoute {
            service: "orders".into(),
            method: "POST".into(),
            path: "/checkout".into(),
            file: "orders.js".into(),
            line: 3,
            steps: vec![],
            failures: vec![RouteFailure {
                status: 409,
                message: "a | b".into(),
                line: 7,
            }],
            catches: false,
        });
        t.resilience.push(ServiceResilience {
            service: "orders".into(),
            kind: "circuit_breaker".into(),
            detail: "CircuitBreaker".into(),
            file: "http.js".into(),
            line: 2,
        });
        let mut shared = t.resilience[0].clone();
        shared.service = "payments".into();
        t.resilience.push(shared);
        let text = section(&t, |s| s.replace('|', "\\|"));
        assert!(
            text.contains("| orders | POST /checkout | 409 | a \\| b | nein | orders.js:7 |"),
            "{text}"
        );
        assert!(
            text.contains("- Circuit Breaker CircuitBreaker (http.js:2): orders, payments"),
            "{text}"
        );
    }
}
