//! Chapter 6: one sequence diagram per recorded trace (OTLP), with the
//! matched service-to-service spans in their measured order.

use archguard_core::{
    Diagram, DiagramElement, DiagramKind, DiagramRelation, ElementKind, RelationKind,
    ServiceDependencyKind, ServiceTopology, TraceSequence, TraceStep,
};

pub fn diagrams(t: &ServiceTopology) -> Vec<Diagram> {
    t.traces.iter().map(|s| diagram(t, s)).collect()
}

fn ms(us: u64) -> String {
    format!("{:.1} ms", us as f64 / 1000.0)
}

/// Service ids in first-use order.
fn participants(s: &TraceSequence) -> Vec<&str> {
    let mut ids: Vec<&str> = Vec::new();
    for step in &s.steps {
        for id in [step.from.as_str(), step.to.as_str()] {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }
    ids
}

fn element(t: &ServiceTopology, i: usize, id: &str) -> DiagramElement {
    DiagramElement {
        id: format!("e{i}"),
        name: t
            .services
            .iter()
            .find(|x| x.id == id)
            .map_or(id, |x| x.name.as_str())
            .to_string(),
        kind: ElementKind::Container,
        technology: None,
        detail: None,
        group: None,
        cycle: false,
    }
}

/// Step `n` (0-based) as a message between participant positions.
fn message(n: usize, step: &TraceStep, at: impl Fn(&str) -> usize) -> DiagramRelation {
    let (kind, word) = match step.kind {
        ServiceDependencyKind::GrpcCall => (RelationKind::Grpc, "gRPC"),
        ServiceDependencyKind::Event => (RelationKind::Event, "Event"),
        _ => (RelationKind::Http, "HTTP"),
    };
    let operation = step
        .operation
        .as_deref()
        .map(|o| format!(" {o}"))
        .unwrap_or_default();
    let failed = if step.error { " · Fehler" } else { "" };
    DiagramRelation {
        from: format!("e{}", at(&step.from)),
        to: format!("e{}", at(&step.to)),
        kind,
        count: 1,
        cycle: false,
        label: Some(format!(
            "{}. {word}{operation} · {}{failed}",
            n + 1,
            ms(step.duration_us)
        )),
        error: step.error,
    }
}

fn diagram(t: &ServiceTopology, s: &TraceSequence) -> Diagram {
    let ids = participants(s);
    let at = |id: &str| ids.iter().position(|x| *x == id).unwrap_or(0);
    let relations = s
        .steps
        .iter()
        .enumerate()
        .map(|(n, step)| message(n, step, at))
        .collect();
    let short = &s.trace[..s.trace.len().min(8)];
    let first = s
        .steps
        .first()
        .and_then(|f| f.operation.as_deref())
        .unwrap_or("Anfrage");
    Diagram {
        id: format!("runtime-trace-{short}"),
        chapter: 6,
        title: format!("Sequenz · {first} · Trace {short}"),
        kind: DiagramKind::Sequence,
        note: format!(
            "Quelle: aufgezeichneter OTLP-Trace in .archguard/otel-traces.json, Start {} ns (Unix), Dauer {}. Reihenfolge und Dauer sind gemessen; gezeigt sind nur zugeordnete Client/Server- und Producer/Consumer-Paare zwischen Diensten, keine Datenbank- oder internen Spans.",
            s.start_unix_ns,
            ms(s.duration_us)
        ),
        unit: "Aufrufe".into(),
        elements: ids
            .iter()
            .enumerate()
            .map(|(i, id)| element(t, i, id))
            .collect(),
        relations,
        shown: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trace_becomes_a_sequence_in_step_order_with_named_participants() {
        let step = |from: &str, to: &str, kind, offset_us, error| TraceStep {
            from: from.into(),
            to: to.into(),
            kind,
            offset_us,
            duration_us: 1500,
            error,
            operation: Some("GET /orders/{id}".into()),
        };
        let mut t: ServiceTopology = serde_json::from_value(serde_json::json!({
            "schema_version":1,"root":"shop","monorepo":false,"monorepo_reason":"","compose_files":[],
            "services":[{"id":"s:web","name":"web","compose_file":"","build_context":null,"networks":[],"profiles":[],"project":null}],
            "dependencies":[],"cycles":[],"diagnostics":[],"complete":true
        }))
        .unwrap();
        t.traces.push(TraceSequence {
            trace: "abcdef0123456789abcdef0123456789".into(),
            start_unix_ns: 1,
            duration_us: 4000,
            steps: vec![
                step(
                    "s:web",
                    "s:orders",
                    ServiceDependencyKind::HttpCall,
                    0,
                    false,
                ),
                step(
                    "s:orders",
                    "s:mail",
                    ServiceDependencyKind::Event,
                    900,
                    true,
                ),
            ],
        });
        let d = &diagrams(&t)[0];
        assert_eq!(
            (d.id.as_str(), d.chapter, d.kind),
            ("runtime-trace-abcdef01", 6, DiagramKind::Sequence)
        );
        let names: Vec<&str> = d.elements.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["web", "s:orders", "s:mail"]);
        assert_eq!(
            d.relations[0].label.as_deref(),
            Some("1. HTTP GET /orders/{id} · 1.5 ms")
        );
        assert!(d.relations[1].error && d.relations[1].kind == RelationKind::Event);
        let m = super::super::render::mermaid(d);
        assert!(m.starts_with("sequenceDiagram\n"), "{m}");
        assert!(
            m.contains("  e0->>e1: 1. HTTP GET /orders/{id} · 1.5 ms\n"),
            "{m}"
        );
        assert!(m.contains("  e1-xe2:"), "{m}");
        let p = super::super::render::plantuml(d);
        assert!(p.contains("e1 -[#d33]> e2 : 2. Event"), "{p}");
    }
}
