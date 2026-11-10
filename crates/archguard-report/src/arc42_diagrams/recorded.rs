//! Chapter 6: the project's own recorded runtime (ADR 0004). One sequence
//! per use case (entry span name), drawn from the request whose duration
//! is closest to the use case's median: participants are the modules of
//! the method spans, messages the calls in measured order.

use archguard_core::{
    Diagram, DiagramElement, DiagramKind, DiagramRelation, DocLang, ElementKind, RelationKind,
    RuntimeEndpoint, RuntimeProfile, RuntimeRequest, RuntimeSpan,
};

/// Use cases with the most total time first; more do not help a reader.
const MAX_DIAGRAMS: usize = 4;
const MAX_MESSAGES: usize = 30;
/// Fewer calls tell no more than the use case table.
const MIN_MESSAGES: usize = 3;
/// Call depth below the request that is still drawn.
const MAX_DEPTH: usize = 4;
/// Calls shorter than this share of the request are left out, errors never.
const MIN_SHARE: f64 = 0.01;

pub fn diagrams(l: DocLang, p: &RuntimeProfile) -> Vec<Diagram> {
    let mut cases: Vec<&RuntimeEndpoint> = p.endpoints.iter().collect();
    cases.sort_by(|a, b| total(b).total_cmp(&total(a)));
    cases
        .into_iter()
        .filter_map(|e| Some(diagram(l, e, typical(p, e)?, p)))
        .filter(|d| d.relations.len() >= MIN_MESSAGES)
        .take(MAX_DIAGRAMS)
        .collect()
}

fn total(e: &RuntimeEndpoint) -> f64 {
    e.requests as f64 * e.duration_ms.mean
}

/// The kept request of `e` with method spans closest to its median duration.
fn typical<'a>(p: &'a RuntimeProfile, e: &RuntimeEndpoint) -> Option<&'a RuntimeRequest> {
    p.requests
        .iter()
        .filter(|r| r.service == e.service && r.name == e.name)
        .filter(|r| r.spans.iter().any(|s| s.function.is_some()))
        .min_by(|a, b| {
            let off = |r: &RuntimeRequest| (r.duration_ms - e.duration_ms.p50).abs();
            off(a).total_cmp(&off(b))
        })
}

/// `crate.module.function` -> `crate.module`; a database call -> the database.
fn participant(l: DocLang, s: &RuntimeSpan) -> Option<String> {
    if s.db.is_some() {
        return Some(l.pick("Database", "Datenbank").into());
    }
    let f = s.function.as_deref()?;
    Some(f.rsplit_once('.').map_or(f, |(m, _)| m).to_string())
}

/// Namespace and name: `archguard_engine.pipeline` -> (`archguard_engine`,
/// `pipeline`), a Java `com.shop.OrderService` -> (`com.shop`, `OrderService`).
fn split(module: &str) -> (Option<&str>, &str) {
    module
        .rsplit_once('.')
        .map_or((None, module), |(space, name)| (Some(space), name))
}

fn method(s: &RuntimeSpan) -> &str {
    s.function
        .as_deref()
        .and_then(|f| f.rsplit('.').next())
        .unwrap_or(&s.name)
}

struct Message {
    relation: DiagramRelation,
    method: String,
    ms: f64,
    times: u32,
}

/// The drawn calls of `r` and their participants (the request first).
/// Returns whether calls were cut by `MAX_MESSAGES` or the probe's limit.
fn messages(l: DocLang, r: &RuntimeRequest) -> (Vec<String>, Vec<Message>, bool) {
    let mut names = vec![r.service.clone()];
    let mut out: Vec<Message> = Vec::new();
    // (span depth, message index, participant index) of the open calls.
    let mut open: Vec<(u32, usize, usize)> = Vec::new();
    for s in &r.spans {
        while open.last().is_some_and(|o| o.0 >= s.depth) {
            open.pop();
        }
        let kept = s.error || s.duration_ms >= r.duration_ms * MIN_SHARE;
        let Some(to) = participant(l, s).filter(|_| kept && open.len() < MAX_DEPTH) else {
            continue;
        };
        let to = names.iter().position(|n| *n == to).unwrap_or_else(|| {
            names.push(to);
            names.len() - 1
        });
        let caller = open.last().map(|o| o.1);
        let last = out.len().checked_sub(1);
        if let Some(m) = last.map(|i| &mut out[i]).filter(|m| {
            m.relation.caller == caller
                && m.relation.to == format!("e{to}")
                && m.method == method(s)
        }) {
            // A repeated call in a row: one message, counted.
            m.times += 1;
            m.ms += s.duration_ms;
            m.relation.error |= s.error;
            open.push((s.depth, last.unwrap_or(0), to));
            continue;
        }
        if out.len() == MAX_MESSAGES {
            return (names, out, true);
        }
        let from = open.last().map_or(0, |o| o.2);
        out.push(Message {
            relation: DiagramRelation {
                from: format!("e{from}"),
                to: format!("e{to}"),
                kind: RelationKind::Call,
                count: 1,
                cycle: false,
                label: None,
                error: s.error,
                caller,
                reply: false,
            },
            method: method(s).to_string(),
            ms: s.duration_ms,
            times: 1,
        });
        open.push((s.depth, out.len() - 1, to));
    }
    (names, out, r.spans_truncated)
}

fn diagram(l: DocLang, e: &RuntimeEndpoint, r: &RuntimeRequest, p: &RuntimeProfile) -> Diagram {
    let (names, messages, cut) = messages(l, r);
    let relations = messages
        .into_iter()
        .enumerate()
        .map(|(n, m)| {
            let times = if m.times > 1 {
                format!(" ×{}", m.times)
            } else {
                String::new()
            };
            DiagramRelation {
                label: Some(format!("{}. {}{times} · {:.1} ms", n + 1, m.method, m.ms)),
                ..m.relation
            }
        })
        .collect();
    let elements = names
        .iter()
        .enumerate()
        .map(|(i, n)| DiagramElement {
            id: format!("e{i}"),
            name: if i == 0 { n } else { split(n).1 }.to_string(),
            kind: match i {
                0 => ElementKind::Person,
                _ if n == l.pick("Database", "Datenbank") => ElementKind::Database,
                _ => ElementKind::Package,
            },
            technology: None,
            detail: None,
            group: split(n).0.filter(|_| i > 0).map(str::to_string),
            cycle: false,
        })
        .collect();
    let note = l.pick(
        "Source: recorded OTLP traces ({}). The request closest to the median duration ({} ms) of {} recorded ones: trace {}, {} ms. Measured order; calls under 1 % of the request time and deeper than {} levels are left out, repeated calls in a row are counted.",
        "Quelle: aufgezeichnete OTLP-Traces ({}). Die Anfrage, die dem Median ({} ms) von {} aufgezeichneten am nächsten liegt: Trace {}, {} ms. Gemessene Reihenfolge; Aufrufe unter 1 % der Anfragezeit und tiefer als {} Ebenen fehlen, wiederholte Aufrufe in Folge sind gezählt.",
    );
    let trace = &r.trace_id[..r.trace_id.len().min(8)];
    let mut note = crate::arc42_text::fill(
        note,
        &[
            &p.inputs.join(", "),
            &format!("{:.1}", e.duration_ms.p50),
            &e.requests,
            &trace,
            &format!("{:.1}", r.duration_ms),
            &MAX_DEPTH,
        ],
    );
    if cut {
        note.push_str(l.pick(
            " Further calls are cut off.",
            " Weitere Aufrufe sind abgeschnitten.",
        ));
    }
    Diagram {
        id: format!("runtime-recorded-{trace}"),
        chapter: 6,
        title: format!(
            "{} · {}",
            l.pick("Recorded run", "Aufgezeichneter Ablauf"),
            e.name
        ),
        kind: DiagramKind::Sequence,
        note,
        unit: l.pick("calls", "Aufrufe").into(),
        elements,
        relations,
        shown: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(depth: u32, function: &str, ms: f64) -> serde_json::Value {
        serde_json::json!({"depth":depth,"offset_ms":0.0,"duration_ms":ms,"kind":"internal",
            "name":function,"error":false,"db":null,"function":function,"file":null,"line":null})
    }

    fn request(ms: f64, spans: Vec<serde_json::Value>) -> serde_json::Value {
        serde_json::json!({"service":"app","name":"analyze","trace_id":"0123456789abcdef",
            "span_id":"s","start_unix_ns":0,"duration_ms":ms,"error":false,"db_calls":0,
            "db_time_ms":0.0,"spans":spans,"spans_truncated":false,
            "evidence":{"file":"","line":0,"trace_id":"","span_id":""}})
    }

    fn profile() -> RuntimeProfile {
        let stats = serde_json::json!({"mean":100.0,"p50":100.0,"p95":300.0,"max":300.0});
        let zero = serde_json::json!({"mean":0.0,"p50":0.0,"p95":0.0,"max":0.0});
        serde_json::from_value(serde_json::json!({
            "schema_version":1,"inputs":["t.jsonl"],"spans":0,"traces":0,"window_unix_ns":null,
            "sampling":[],"statements":[],"requests_total":2,"findings":[],"diagnostics":[],
            "complete":true,"note":"",
            "endpoints":[{"service":"app","name":"analyze","requests":2,"errors":0,
                "duration_ms":stats,"db_calls":zero,"db_time_ms":0.0,
                "example":{"file":"","line":0,"trace_id":"","span_id":""}}],
            "requests":[
                request(300.0, vec![span(1, "eng.pipeline.slow", 290.0)]),
                request(100.0, vec![
                    span(1, "eng.pipeline.run", 90.0),
                    span(2, "eng.cache.load", 20.0),
                    span(2, "ana.scc.find", 5.0),
                    span(2, "ana.scc.find", 5.0),
                    span(2, "ana.scc.tiny", 0.5),
                    span(3, "ana.scc.inner", 0.4),
                    span(2, "eng.pipeline.save", 10.0),
                    span(1, "eng.cache.load", 3.0),
                ]),
            ],
        }))
        .unwrap()
    }

    #[test]
    fn the_median_request_becomes_a_sequence_of_module_calls() {
        let d = &diagrams(DocLang::En, &profile())[0];
        let names: Vec<&str> = d.elements.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["app", "pipeline", "cache", "scc"]);
        let spaces: Vec<_> = d.elements.iter().map(|e| e.group.as_deref()).collect();
        assert_eq!(spaces, [None, Some("eng"), Some("eng"), Some("ana")]);
        let calls: Vec<(&str, &str, Option<usize>, &str)> = d
            .relations
            .iter()
            .map(|r| {
                (
                    r.from.as_str(),
                    r.to.as_str(),
                    r.caller,
                    r.label.as_deref().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            calls,
            [
                ("e0", "e1", None, "1. run · 90.0 ms"),
                ("e1", "e2", Some(0), "2. load · 20.0 ms"),
                ("e1", "e3", Some(0), "3. find ×2 · 10.0 ms"),
                ("e1", "e1", Some(0), "4. save · 10.0 ms"),
                ("e0", "e2", None, "5. load · 3.0 ms"),
            ],
            "the 300 ms outlier is not typical; calls under 1 % are left out"
        );
        assert!(
            d.note.contains("median duration (100.0 ms) of 2"),
            "{}",
            d.note
        );
    }

    #[test]
    fn the_markdown_sequence_draws_each_return_to_its_caller() {
        let text = super::super::render::mermaid(&diagrams(DocLang::En, &profile())[0]);
        let lines: Vec<&str> = text.lines().skip(6).map(str::trim).collect();
        assert_eq!(
            lines,
            [
                "e0->>+e1: 1. run · 90.0 ms",
                "e1->>+e2: 2. load · 20.0 ms",
                "e2-->>-e1: return",
                "e1->>+e3: 3. find ×2 · 10.0 ms",
                "e3-->>-e1: return",
                "e1->>e1: 4. save · 10.0 ms",
                "e1-->>-e0: return",
                "e0->>+e2: 5. load · 3.0 ms",
                "e2-->>-e0: return",
            ],
            "{text}"
        );
    }
}
