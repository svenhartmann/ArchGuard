//! Optional, local OTLP/JSON evidence. No collector, application or network is started.
//! A service edge requires matched CLIENT/SERVER spans or explicitly correlated messaging spans. Resource
//! coexistence, shared destinations and orphan spans never establish an edge.
use archguard_core::{
    Service, ServiceDependency, ServiceDependencyKind as Kind, ServiceDiagnostic, ServiceEvidence,
    ServiceTopology,
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Read},
    path::Path,
};

const INPUT: &str = ".archguard/otel-traces.json";
const MAX_BYTES: u64 = 16 * 1024 * 1024;
const MAX_SPANS: usize = 100_000;
const MAX_LINKS_PER_SPAN: usize = 128;
const MAX_LINKS: usize = 1_000_000;
fn diagnostic(report: &mut ServiceTopology, message: impl Into<String>) {
    report.complete = false;
    report.diagnostics.push(ServiceDiagnostic {
        file: INPUT.into(),
        service: None,
        message: message.into(),
    });
}
#[cfg(unix)]
fn open_input(root: &Path) -> io::Result<fs::File> {
    use rustix::fs::{Mode, OFlags, open, openat};
    let root = open(
        root,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW,
        Mode::empty(),
    )?;
    let directory = openat(
        &root,
        ".archguard",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW,
        Mode::empty(),
    )?;
    Ok(openat(
        &directory,
        "otel-traces.json",
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    )?
    .into())
}
#[cfg(not(unix))]
fn open_input(root: &Path) -> io::Result<fs::File> {
    // Reject reparse/symlink entries before opening the fixed local path.
    for path in [root.join(".archguard"), root.join(INPUT)] {
        if fs::symlink_metadata(&path)?.file_type().is_symlink() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "symlink trace input",
            ));
        }
    }
    let file = fs::File::open(root.join(INPUT))?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a regular file",
        ));
    }
    Ok(file)
}
fn attribute<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    let entries = value.get("attributes")?.as_array()?;
    let mut matches = entries
        .iter()
        .filter(|item| item.get("key").and_then(Value::as_str) == Some(key));
    let result = matches.next()?.get("value")?;
    if matches.next().is_some() {
        return None;
    }
    Some(result)
}
fn string_attribute<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    attribute(value, key)?.get("stringValue")?.as_str()
}
fn identifier(value: &Value, length: usize) -> Option<String> {
    let s = value.as_str()?;
    (s.len() == length && s.bytes().all(|b| b.is_ascii_hexdigit()) && s.bytes().any(|b| b != b'0'))
        .then(|| s.to_ascii_lowercase())
}
fn safe_service_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-/".contains(&b))
}
fn nanos(value: &Value) -> Option<u64> {
    value.as_str().and_then(|s| {
        (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse::<u64>().ok())
            .flatten()
    })
}
fn span_kind(value: &Value) -> Option<u8> {
    match (value.as_u64(), value.as_str()) {
        (Some(2), _) | (_, Some("SPAN_KIND_SERVER")) => Some(2),
        (Some(3), _) | (_, Some("SPAN_KIND_CLIENT")) => Some(3),
        (Some(4), _) | (_, Some("SPAN_KIND_PRODUCER")) => Some(4),
        (Some(5), _) | (_, Some("SPAN_KIND_CONSUMER")) => Some(5),
        _ => None,
    }
}
fn protocol(span: &Value) -> Option<Kind> {
    if string_attribute(span, "rpc.system") == Some("grpc") {
        Some(Kind::GrpcCall)
    } else if string_attribute(span, "http.request.method").is_some()
        || string_attribute(span, "http.method").is_some()
    {
        Some(Kind::HttpCall)
    } else {
        None
    }
}
// https://opentelemetry.io/docs/specs/semconv/messaging/messaging-spans/
// Links are the default correlation mechanism; single-message consumers may
// parent directly to the producer context. Destination equality alone is not proof.
#[derive(Clone, PartialEq, Eq)]
struct Messaging {
    system: String,
    destination: String,
    broker: Option<String>,
    port: Option<String>,
    operation: Option<String>,
}
fn messaging(span: &Value) -> Option<Messaging> {
    let bounded = |key| {
        string_attribute(span, key)
            .filter(|v| !v.is_empty() && v.len() <= 512)
            .map(str::to_owned)
    };
    let system = bounded("messaging.system")?;
    let destination =
        bounded("messaging.destination.name").or_else(|| bounded("messaging.destination"))?;
    let port = attribute(span, "server.port")
        .and_then(|v| v.get("intValue"))
        .and_then(|v| {
            v.as_str()
                .map(str::to_owned)
                .or_else(|| v.as_u64().map(|v| v.to_string()))
        });
    Some(Messaging {
        system,
        destination,
        broker: bounded("server.address"),
        port,
        operation: bounded("messaging.operation.type").or_else(|| bounded("messaging.operation")),
    })
}
fn compatible(a: &Messaging, b: &Messaging) -> bool {
    a.system == b.system
        && a.destination == b.destination
        && !matches!((&a.broker,&b.broker),(Some(a),Some(b)) if a != b)
        && !matches!((&a.port,&b.port),(Some(a),Some(b)) if a != b)
}
#[derive(Clone, PartialEq, Eq)]
struct Span {
    trace: String,
    id: String,
    parent: Option<String>,
    service: String,
    namespace: String,
    kind: u8,
    protocol: Option<Kind>,
    messaging: Option<Messaging>,
    links: BTreeSet<(String, String)>,
    start: u64,
    end: u64,
    error: bool,
}
struct Entry {
    span: Span,
    pointer: String,
}
#[derive(Default)]
struct Observation {
    calls: u64,
    errors: u64,
    duration: u128,
    max_duration: u64,
    start: u64,
    end: u64,
    pointer: String,
}
fn service_id(report: &mut ServiceTopology, name: &str) -> Option<String> {
    let matches: BTreeSet<String> = report
        .services
        .iter()
        .filter(|s| {
            s.name == name
                || s.project.as_ref().is_some_and(|id| {
                    report
                        .projects
                        .iter()
                        .any(|p| &p.id == id && p.name == name)
                })
        })
        .map(|s| s.id.clone())
        .collect();
    if matches.len() == 1 {
        return matches.first().cloned();
    }
    if matches.len() > 1 {
        return None;
    }
    let id = format!("trace:{name}");
    if report.services.iter().any(|s| s.id == id) {
        return None;
    }
    report.services.push(Service {
        id: id.clone(),
        name: name.into(),
        compose_file: String::new(),
        build_context: None,
        networks: vec![],
        profiles: vec![],
        project: None,
    });
    Some(id)
}
pub(super) fn augment(root: &Path, report: &mut ServiceTopology) {
    let file = match open_input(root) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return,
        Err(_) => {
            diagnostic(
                report,
                "Trace import refused: input must be a readable regular local file without symlinks.",
            );
            return;
        }
    };
    if file
        .metadata()
        .map_or(true, |m| !m.is_file() || m.len() > MAX_BYTES)
    {
        diagnostic(
            report,
            "Trace import refused: expected a regular OTLP JSON file no larger than 16 MiB.",
        );
        return;
    }
    let mut bytes = Vec::new();
    if file.take(MAX_BYTES + 1).read_to_end(&mut bytes).is_err() || bytes.len() as u64 > MAX_BYTES {
        diagnostic(report, "Trace import failed or exceeded its 16 MiB limit.");
        return;
    }
    let root: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => {
            diagnostic(
                report,
                "Invalid OTLP JSON trace document; no trace evidence imported.",
            );
            return;
        }
    };
    let Some(resources) = root.get("resourceSpans").and_then(Value::as_array) else {
        diagnostic(
            report,
            "Expected OTLP JSON resourceSpans with scopeSpans/spans; other trace export formats are not supported.",
        );
        return;
    };
    let mut entries: BTreeMap<(String, String), Entry> = BTreeMap::new();
    let mut conflicts = BTreeSet::new();
    let mut reasons: BTreeMap<&str, usize> = BTreeMap::new();
    let mut total = 0usize;
    let mut total_links = 0usize;
    let mut namespaces: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (ri, resource) in resources.iter().enumerate() {
        let Some(name) = resource
            .get("resource")
            .and_then(|r| string_attribute(r, "service.name"))
            .filter(|name| safe_service_name(name))
        else {
            *reasons
                .entry("resources without a safe, unique service.name")
                .or_default() += 1;
            continue;
        };
        let namespace = resource
            .get("resource")
            .and_then(|r| string_attribute(r, "service.namespace"))
            .unwrap_or("");
        if !namespace.is_empty() && !safe_service_name(namespace) {
            *reasons
                .entry("resources with unsupported service.namespace")
                .or_default() += 1;
            continue;
        }
        namespaces
            .entry(name.into())
            .or_default()
            .insert(namespace.into());
        let Some(scopes) = resource.get("scopeSpans").and_then(Value::as_array) else {
            *reasons.entry("resources without scopeSpans").or_default() += 1;
            continue;
        };
        for (si, scope) in scopes.iter().enumerate() {
            let Some(spans) = scope.get("spans").and_then(Value::as_array) else {
                *reasons.entry("scopes without spans").or_default() += 1;
                continue;
            };
            for (pi, value) in spans.iter().enumerate() {
                total += 1;
                if total > MAX_SPANS {
                    diagnostic(
                        report,
                        "Trace span limit exceeded (100000); import rejected rather than using a truncated graph.",
                    );
                    return;
                }
                let Some(kind) = value.get("kind").and_then(span_kind) else {
                    *reasons.entry("unsupported span kinds").or_default() += 1;
                    continue;
                };
                let Some(trace) = value.get("traceId").and_then(|v| identifier(v, 32)) else {
                    *reasons
                        .entry("spans with invalid IDs or timestamps")
                        .or_default() += 1;
                    continue;
                };
                let Some(id) = value.get("spanId").and_then(|v| identifier(v, 16)) else {
                    *reasons
                        .entry("spans with invalid IDs or timestamps")
                        .or_default() += 1;
                    continue;
                };
                let parent = match value.get("parentSpanId").and_then(Value::as_str) {
                    None | Some("") => None,
                    Some(_) => match value.get("parentSpanId").and_then(|v| identifier(v, 16)) {
                        Some(id) => Some(id),
                        None => {
                            *reasons
                                .entry("spans with invalid IDs or timestamps")
                                .or_default() += 1;
                            continue;
                        }
                    },
                };
                let (Some(start), Some(end)) = (
                    value.get("startTimeUnixNano").and_then(nanos),
                    value.get("endTimeUnixNano").and_then(nanos),
                ) else {
                    *reasons
                        .entry("spans with invalid IDs or timestamps")
                        .or_default() += 1;
                    continue;
                };
                if end < start || start == 0 {
                    *reasons
                        .entry("spans with invalid IDs or timestamps")
                        .or_default() += 1;
                    continue;
                }
                let mut links = BTreeSet::new();
                let mut invalid_links = false;
                if let Some(raw) = value.get("links") {
                    if let Some(raw) = raw.as_array() {
                        total_links += raw.len();
                        if raw.len() > MAX_LINKS_PER_SPAN || total_links > MAX_LINKS {
                            diagnostic(
                                report,
                                "Trace link limit exceeded (128 per span, 1000000 total); import rejected rather than truncating correlations.",
                            );
                            return;
                        }
                        for link in raw {
                            match (
                                link.get("traceId").and_then(|v| identifier(v, 32)),
                                link.get("spanId").and_then(|v| identifier(v, 16)),
                            ) {
                                (Some(trace), Some(id)) => {
                                    links.insert((trace, id));
                                }
                                _ => {
                                    invalid_links = true;
                                }
                            }
                        }
                    } else {
                        invalid_links = true;
                    }
                }
                if invalid_links {
                    *reasons
                        .entry("spans with malformed links rejected")
                        .or_default() += 1;
                    continue;
                }
                let status = value.get("status").and_then(|v| v.get("code"));
                let error = status.is_some_and(|v| {
                    v.as_u64() == Some(2) || v.as_str() == Some("STATUS_CODE_ERROR")
                });
                let span = Span {
                    trace: trace.clone(),
                    id: id.clone(),
                    parent,
                    service: name.into(),
                    namespace: namespace.into(),
                    kind,
                    protocol: protocol(value),
                    messaging: messaging(value),
                    links,
                    start,
                    end,
                    error,
                };
                let key = (trace, id);
                if conflicts.contains(&key) {
                    continue;
                }
                if let Some(existing) = entries.get(&key) {
                    if existing.span != span {
                        entries.remove(&key);
                        conflicts.insert(key);
                        *reasons
                            .entry("conflicting duplicate span identities")
                            .or_default() += 1;
                    } else {
                        *reasons.entry("duplicate spans deduplicated").or_default() += 1;
                    }
                    continue;
                }
                entries.insert(
                    key,
                    Entry {
                        span,
                        pointer: format!("/resourceSpans/{ri}/scopeSpans/{si}/spans/{pi}"),
                    },
                );
            }
        }
    }
    let mut mappings = BTreeMap::new();
    for (name, spaces) in namespaces {
        if spaces.len() != 1 {
            *reasons
                .entry("service names ambiguous across namespaces")
                .or_default() += 1;
            continue;
        }
        if let Some(id) = service_id(report, &name) {
            mappings.insert(name, id);
        } else {
            *reasons
                .entry("service names ambiguous in the repository")
                .or_default() += 1;
        }
    }
    let mut children: BTreeMap<(String, String), Vec<&Entry>> = BTreeMap::new();
    for entry in entries.values().filter(|e| e.span.kind == 2) {
        if let Some(parent) = &entry.span.parent {
            children
                .entry((entry.span.trace.clone(), parent.clone()))
                .or_default()
                .push(entry);
        } else {
            *reasons.entry("orphan SERVER spans").or_default() += 1;
        }
    }
    let mut observed: BTreeMap<(String, String, Kind), Observation> = BTreeMap::new();
    let mut used_servers = BTreeSet::new();
    for (key, client) in entries
        .iter()
        .filter(|(_, e)| e.span.kind == 3 && e.span.messaging.is_none())
    {
        let Some(servers) = children.get(key) else {
            *reasons.entry("orphan CLIENT spans").or_default() += 1;
            continue;
        };
        if servers.len() != 1 {
            *reasons
                .entry("CLIENT spans with ambiguous multiple SERVER children")
                .or_default() += 1;
            continue;
        }
        let server = servers[0];
        used_servers.insert((server.span.trace.clone(), server.span.id.clone()));
        let (Some(from), Some(to)) = (
            mappings.get(&client.span.service),
            mappings.get(&server.span.service),
        ) else {
            continue;
        };
        if from == to {
            continue;
        }
        let Some(kind) = client.span.protocol.or(server.span.protocol) else {
            *reasons
                .entry("matched pairs without HTTP/gRPC protocol attributes")
                .or_default() += 1;
            continue;
        };
        let sample = observed
            .entry((from.clone(), to.clone(), kind))
            .or_default();
        let duration = client.span.end - client.span.start;
        sample.calls += 1;
        sample.errors += u64::from(client.span.error || server.span.error);
        sample.duration += u128::from(duration);
        sample.max_duration = sample.max_duration.max(duration);
        sample.start = if sample.start == 0 {
            client.span.start
        } else {
            sample.start.min(client.span.start)
        };
        sample.end = sample.end.max(client.span.end);
        if sample.pointer.is_empty() {
            sample.pointer = client.pointer.clone();
        }
    }
    for consumer in entries.values().filter(|e| {
        e.span.kind == 5
            || (e.span.kind == 3
                && e.span
                    .messaging
                    .as_ref()
                    .is_some_and(|m| m.operation.as_deref() == Some("receive")))
    }) {
        let Some(message) = &consumer.span.messaging else {
            *reasons
                .entry("messaging spans without system/destination metadata")
                .or_default() += 1;
            continue;
        };
        if message
            .operation
            .as_deref()
            .is_some_and(|op| !matches!(op, "receive" | "process"))
        {
            *reasons
                .entry("unsupported messaging consumer operations")
                .or_default() += 1;
            continue;
        }
        let mut contexts = consumer.span.links.clone();
        if let Some(parent) = &consumer.span.parent {
            contexts.insert((consumer.span.trace.clone(), parent.clone()));
        }
        let mut paired = false;
        for context in contexts {
            let Some(producer) = entries.get(&context).filter(|p| p.span.kind == 4) else {
                continue;
            };
            let Some(sent) = &producer.span.messaging else {
                continue;
            };
            if !compatible(sent, message)
                || sent
                    .operation
                    .as_deref()
                    .is_some_and(|op| !matches!(op, "send" | "publish" | "create"))
            {
                *reasons
                    .entry("messaging correlations with incompatible metadata")
                    .or_default() += 1;
                continue;
            }
            let (Some(from), Some(to)) = (
                mappings.get(&producer.span.service),
                mappings.get(&consumer.span.service),
            ) else {
                continue;
            };
            paired = true;
            if from == to {
                continue;
            }
            let sample = observed
                .entry((from.clone(), to.clone(), Kind::Event))
                .or_default();
            let duration = consumer.span.end - consumer.span.start;
            sample.calls += 1;
            sample.errors += u64::from(producer.span.error || consumer.span.error);
            sample.duration += u128::from(duration);
            sample.max_duration = sample.max_duration.max(duration);
            let start = producer.span.start.min(consumer.span.start);
            sample.start = if sample.start == 0 {
                start
            } else {
                sample.start.min(start)
            };
            sample.end = sample.end.max(producer.span.end.max(consumer.span.end));
            if sample.pointer.is_empty() {
                sample.pointer = consumer.pointer.clone();
            }
        }
        if !paired {
            *reasons
                .entry("messaging consumers without a compatible correlated producer")
                .or_default() += 1;
        }
    }
    let orphan_servers = entries
        .values()
        .filter(|e| {
            e.span.kind == 2
                && e.span.parent.is_some()
                && !used_servers.contains(&(e.span.trace.clone(), e.span.id.clone()))
        })
        .count();
    if orphan_servers > 0 {
        *reasons.entry("unpaired SERVER spans").or_default() += orphan_servers;
    }
    for ((from, to, kind), sample) in observed {
        let description = if kind == Kind::Event {
            format!(
                "Observed OTLP producer/consumer correlations via parent context or span links: correlations={}; errors={} (recorded ERROR status on either span); mean_consumer_duration_ms={:.3}; max_consumer_duration_ms={:.3}; window_unix_ns={}..{}. Counts are matched span pairs, not unique messages or end-to-end delivery latency. Shared topics without correlated contexts do not establish an edge; this sample does not prove complete traffic or an execution loop.",
                sample.calls,
                sample.errors,
                sample.duration as f64 / sample.calls as f64 / 1_000_000.0,
                sample.max_duration as f64 / 1_000_000.0,
                sample.start,
                sample.end
            )
        } else {
            format!(
                "Observed matched OTLP CLIENT/SERVER pairs: calls={}; errors={} (recorded ERROR status on either span); mean_client_duration_ms={:.3}; max_client_duration_ms={:.3}; window_unix_ns={}..{}. Counts describe this imported sample only, not total traffic or an infinite execution loop.",
                sample.calls,
                sample.errors,
                sample.duration as f64 / sample.calls as f64 / 1_000_000.0,
                sample.max_duration as f64 / 1_000_000.0,
                sample.start,
                sample.end
            )
        };
        let proof = ServiceEvidence {
            file: INPUT.into(),
            pointer: sample.pointer,
            line: None,
            description,
        };
        if let Some(existing) = report
            .dependencies
            .iter_mut()
            .find(|e| e.from == from && e.to == to && e.kind == kind)
        {
            existing.evidence.push(proof);
            existing.evidence.sort();
            existing.evidence.dedup();
        } else {
            report.dependencies.push(ServiceDependency {
                from,
                to,
                kind,
                evidence: vec![proof],
            });
        }
    }
    for (reason, count) in reasons {
        diagnostic(report, format!("Trace import: {count} {reason}."));
    }
    diagnostic(
        report,
        "Trace coverage is partial: only matched HTTP/gRPC CLIENT/SERVER pairs and explicitly correlated messaging producer/consumer spans in this local export are observed. Sampling, missing instrumentation and absent spans hide connections; database, unsupported messaging operations and destination-only edges are not imported. Cycles aggregate across the export window and do not prove a loop within one request.",
    );
    report.services.sort_by(|a, b| a.id.cmp(&b.id));
    report
        .dependencies
        .sort_by(|a, b| (&a.from, &a.to, a.kind).cmp(&(&b.from, &b.to, b.kind)));
}
#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::Service;
    use serde_json::{Value, json};
    use std::fs;
    fn report() -> ServiceTopology {
        ServiceTopology {
            schema_version: 1,
            root: String::new(),
            monorepo: false,
            monorepo_reason: String::new(),
            compose_files: vec![],
            services: vec![],
            projects: vec![],
            dependencies: vec![],
            cycles: vec![],
            diagnostics: vec![],
            complete: true,
        }
    }
    fn span(id: &str, parent: &str, kind: u64) -> Value {
        json!({"traceId":"11111111111111111111111111111111", "spanId":id, "parentSpanId":parent, "kind":kind,"startTimeUnixNano":"1700000000000000001", "endTimeUnixNano":"1700000000010000001", "attributes":[{"key":"http.request.method","value":{"stringValue":"GET"}},{"key":"url.full","value":{"stringValue":"https://secret:password@host/private?token=secret"}}],"status":{"code":0}})
    }
    fn resource(name: &str, spans: Vec<Value>) -> Value {
        json!({"resource":{"attributes":[{"key":"service.name","value":{"stringValue":name}}]},"scopeSpans":[{"spans":spans}]})
    }
    fn run(resources: Vec<Value>, report: &mut ServiceTopology) {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".archguard")).unwrap();
        fs::write(
            dir.path().join(".archguard/otel-traces.json"),
            json!({"resourceSpans":resources}).to_string(),
        )
        .unwrap();
        augment(dir.path(), report);
    }
    #[test]
    fn joins_client_server_and_preserves_observed_counts_without_secrets() {
        let mut r = report();
        r.services.push(Service {
            id: "known:orders".into(),
            name: "orders".into(),
            compose_file: String::new(),
            build_context: None,
            networks: vec![],
            profiles: vec![],
            project: Some("project:orders".into()),
        });
        let client = span("2222222222222222", "", 3);
        let mut server = span("3333333333333333", "2222222222222222", 2);
        server["status"]["code"] = json!(2);
        run(
            vec![
                resource("web", vec![client.clone(), client]),
                resource("orders", vec![server]),
            ],
            &mut r,
        );
        assert_eq!(r.dependencies.len(), 1);
        assert_eq!(r.dependencies[0].to, "known:orders");
        let text = &r.dependencies[0].evidence[0].description;
        assert!(text.contains("calls=1"), "{text}");
        assert!(text.contains("errors=1"));
        assert!(text.contains("mean_client_duration_ms=10.000"));
        assert!(!serde_json::to_string(&r).unwrap().contains("secret"));
        assert!(!r.complete);
    }
    #[test]
    fn orphan_spans_and_coexistence_do_not_invent_edges() {
        let mut r = report();
        run(
            vec![
                resource("web", vec![span("2222222222222222", "", 3)]),
                resource(
                    "orders",
                    vec![span("3333333333333333", "9999999999999999", 2)],
                ),
            ],
            &mut r,
        );
        assert!(r.dependencies.is_empty());
        assert!(!r.diagnostics.is_empty());
    }
    #[test]
    fn conflicting_duplicates_and_invalid_ids_are_not_observations() {
        let mut r = report();
        let client = span("2222222222222222", "", 3);
        let mut conflict = client.clone();
        conflict["endTimeUnixNano"] = json!("1700000000050000001");
        run(
            vec![
                resource("web", vec![client, conflict, span("bad", "", 3)]),
                resource(
                    "orders",
                    vec![span("3333333333333333", "2222222222222222", 2)],
                ),
            ],
            &mut r,
        );
        assert!(r.dependencies.is_empty());
    }
    #[test]
    fn absent_file_does_not_change_static_report() {
        let dir = tempfile::tempdir().unwrap();
        let mut r = report();
        let before = r.clone();
        augment(dir.path(), &mut r);
        assert_eq!(r, before);
    }
    #[cfg(unix)]
    #[test]
    fn refuses_symlink_directory_and_file() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        fs::write(external.path().join("otel-traces.json"), "{}").unwrap();
        symlink(external.path(), dir.path().join(".archguard")).unwrap();
        let mut r = report();
        augment(dir.path(), &mut r);
        assert!(r.dependencies.is_empty());
        assert!(!r.diagnostics.is_empty());
        fs::remove_file(dir.path().join(".archguard")).unwrap();
        fs::create_dir(dir.path().join(".archguard")).unwrap();
        symlink(
            external.path().join("otel-traces.json"),
            dir.path().join(".archguard/otel-traces.json"),
        )
        .unwrap();
        let mut r = report();
        augment(dir.path(), &mut r);
        assert!(!r.diagnostics.is_empty());
    }

    #[test]
    fn matches_grpc_enum_strings_and_does_not_cross_trace_boundaries() {
        let mut r = report();
        let mut client = span("2222222222222222", "", 3);
        client["kind"] = json!("SPAN_KIND_CLIENT");
        client["attributes"] = json!([{"key":"rpc.system","value":{"stringValue":"grpc"}}]);
        let mut server = span("3333333333333333", "2222222222222222", 2);
        server["kind"] = json!("SPAN_KIND_SERVER");
        let mut unrelated = server.clone();
        unrelated["traceId"] = json!("44444444444444444444444444444444");
        run(
            vec![
                resource("web", vec![client]),
                resource("orders", vec![server, unrelated]),
            ],
            &mut r,
        );
        assert_eq!(r.dependencies.len(), 1);
        assert_eq!(r.dependencies[0].kind, Kind::GrpcCall);
        assert!(
            r.dependencies[0].evidence[0]
                .description
                .contains("calls=1")
        );
    }
    #[test]
    fn ambiguous_repository_name_does_not_choose_a_deployment() {
        let mut r = report();
        for id in ["one", "two"] {
            r.services.push(Service {
                id: id.into(),
                name: "orders".into(),
                compose_file: String::new(),
                build_context: None,
                networks: vec![],
                profiles: vec![],
                project: None,
            });
        }
        run(
            vec![
                resource("web", vec![span("2222222222222222", "", 3)]),
                resource(
                    "orders",
                    vec![span("3333333333333333", "2222222222222222", 2)],
                ),
            ],
            &mut r,
        );
        assert!(r.dependencies.is_empty());
        assert!(
            r.diagnostics
                .iter()
                .any(|d| d.message.contains("ambiguous in the repository"))
        );
    }
    #[test]
    fn timestamps_must_be_exact_unsigned_nanoseconds() {
        let mut r = report();
        let mut client = span("2222222222222222", "", 3);
        client["endTimeUnixNano"] = json!("1700000000000000000");
        run(
            vec![
                resource("web", vec![client]),
                resource(
                    "orders",
                    vec![span("3333333333333333", "2222222222222222", 2)],
                ),
            ],
            &mut r,
        );
        assert!(r.dependencies.is_empty());
    }
    #[test]
    fn malformed_and_oversize_files_are_rejected_without_partial_evidence() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".archguard")).unwrap();
        let p = dir.path().join(INPUT);
        fs::write(&p, "{not valid}").unwrap();
        let mut r = report();
        augment(dir.path(), &mut r);
        assert!(r.dependencies.is_empty());
        assert!(!r.complete);
        let file = fs::File::create(&p).unwrap();
        file.set_len(MAX_BYTES + 1).unwrap();
        let mut r = report();
        augment(dir.path(), &mut r);
        assert!(r.dependencies.is_empty());
        assert!(r.diagnostics.iter().any(|d| d.message.contains("16 MiB")));
    }
    fn message(id: &str, parent: &str, kind: u64) -> Value {
        let mut v = span(id, parent, kind);
        v["attributes"] = json!([
            {"key":"messaging.system","value":{"stringValue":"kafka"}},
            {"key":"messaging.destination.name","value":{"stringValue":"private-topic-do-not-export"}},
            {"key":"server.address","value":{"stringValue":"internal-broker"}}
        ]);
        v
    }
    #[test]
    fn messaging_parent_and_links_correlate_without_exporting_topic_or_ids() {
        let producer = message("2222222222222222", "", 4);
        let mut consumer = message("3333333333333333", "2222222222222222", 5);
        consumer["links"] =
            json!([{"traceId":"11111111111111111111111111111111","spanId":"2222222222222222"}]);
        let mut second = message("4444444444444444", "", 5);
        second["traceId"] = json!("55555555555555555555555555555555");
        second["links"] = consumer["links"].clone();
        let mut r = report();
        run(
            vec![
                resource("publisher", vec![producer]),
                resource("consumer", vec![consumer, second]),
            ],
            &mut r,
        );
        assert_eq!(r.dependencies.len(), 1);
        assert_eq!(r.dependencies[0].kind, Kind::Event);
        assert!(
            r.dependencies[0].evidence[0]
                .description
                .contains("correlations=2")
        );
        let output = serde_json::to_string(&r).unwrap();
        assert!(!output.contains("private-topic"));
        assert!(!output.contains("2222222222222222"));
    }
    #[test]
    fn messaging_topic_equality_does_not_replace_correlation() {
        let mut r = report();
        run(
            vec![
                resource("publisher", vec![message("2222222222222222", "", 4)]),
                resource("consumer", vec![message("3333333333333333", "", 5)]),
            ],
            &mut r,
        );
        assert!(r.dependencies.is_empty());
    }
    #[test]
    fn messaging_mismatch_and_malformed_links_are_not_observations() {
        for key in [
            "messaging.system",
            "messaging.destination.name",
            "server.address",
        ] {
            let producer = message("2222222222222222", "", 4);
            let mut consumer = message("3333333333333333", "2222222222222222", 5);
            for attr in consumer["attributes"].as_array_mut().unwrap() {
                if attr["key"] == key {
                    attr["value"]["stringValue"] = json!("different");
                }
            }
            let mut r = report();
            run(
                vec![
                    resource("publisher", vec![producer]),
                    resource("consumer", vec![consumer]),
                ],
                &mut r,
            );
            assert!(r.dependencies.is_empty(), "{key}");
        }
        let producer = message("2222222222222222", "", 4);
        let mut consumer = message("3333333333333333", "", 5);
        consumer["links"] = json!([{"traceId":"11111111111111111111111111111111","spanId":"bad"}]);
        let mut r = report();
        run(
            vec![
                resource("publisher", vec![producer]),
                resource("consumer", vec![consumer]),
            ],
            &mut r,
        );
        assert!(r.dependencies.is_empty());
    }
    #[test]
    fn messaging_batch_receive_client_links_and_limits() {
        let producer = message("2222222222222222", "", 4);
        let mut consumer = message("3333333333333333", "", 3);
        consumer["attributes"]
            .as_array_mut()
            .unwrap()
            .push(json!({"key":"messaging.operation.type","value":{"stringValue":"receive"}}));
        let link =
            json!({"traceId":"11111111111111111111111111111111","spanId":"2222222222222222"});
        consumer["links"] = json!([link.clone()]);
        let mut r = report();
        run(
            vec![
                resource("publisher", vec![producer.clone()]),
                resource("consumer", vec![consumer.clone()]),
            ],
            &mut r,
        );
        assert_eq!(r.dependencies.len(), 1);
        assert_eq!(r.dependencies[0].kind, Kind::Event);
        consumer["links"] = json!(vec![link; MAX_LINKS_PER_SPAN + 1]);
        let mut r = report();
        run(
            vec![
                resource("publisher", vec![producer]),
                resource("consumer", vec![consumer]),
            ],
            &mut r,
        );
        assert!(r.dependencies.is_empty());
        assert!(
            r.diagnostics
                .iter()
                .any(|d| d.message.contains("link limit"))
        );
    }
    #[test]
    fn messaging_conflicting_producer_identity_is_rejected() {
        let producer = message("2222222222222222", "", 4);
        let mut conflicting = producer.clone();
        conflicting["attributes"][0]["value"]["stringValue"] = json!("rabbitmq");
        let consumer = message("3333333333333333", "2222222222222222", 5);
        let mut r = report();
        run(
            vec![
                resource("publisher", vec![producer, conflicting]),
                resource("consumer", vec![consumer]),
            ],
            &mut r,
        );
        assert!(r.dependencies.is_empty());
    }
}
