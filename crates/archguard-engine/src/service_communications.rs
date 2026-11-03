//! Bounded, non-executing reader for literal Aspire/.NET communication patterns.
//! These are static candidates, not traces. Dynamic expressions stay unknown.
use super::support::{diagnostic, relative};
use archguard_core::{
    Service, ServiceDependency, ServiceDependencyKind as Kind, ServiceEvidence, ServiceTopology,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

#[derive(Clone)]
pub(super) struct Token {
    pub(super) text: String,
    pub(super) line: u32,
    pub(super) string: bool,
}
pub(super) fn lex(s: &str) -> Vec<Token> {
    let b = s.as_bytes();
    let (mut i, mut line) = (0, 1);
    let mut out = Vec::new();
    while i < b.len() {
        if b[i] == b'\n' {
            line += 1;
            i += 1;
            continue;
        }
        if b[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b[i..].starts_with(b"//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b[i..].starts_with(b"/*") {
            i += 2;
            while i < b.len() && !b[i..].starts_with(b"*/") {
                if b[i] == b'\n' {
                    line += 1;
                }
                i += 1;
            }
            i = (i + 2).min(b.len());
            continue;
        }
        let start = i;
        let at = line;
        if b[i] == b'"' || b[i] == b'\'' {
            let quote = b[i];
            i += 1;
            let start = i;
            while i < b.len() {
                if b[i] == b'\\' {
                    i = (i + 2).min(b.len());
                    continue;
                }
                if b[i] == quote {
                    break;
                }
                if b[i] == b'\n' {
                    line += 1;
                }
                i += 1;
            }
            let text = String::from_utf8_lossy(&b[start..i]).into_owned();
            i = (i + 1).min(b.len());
            // Interpolated/escaped strings are deliberately not literal endpoints.
            out.push(Token {
                text,
                line: at,
                string: quote == b'"' && !(start >= 2 && matches!(b[start - 2], b'$' | b'@')),
            });
            continue;
        }
        if b[i].is_ascii_alphabetic() || matches!(b[i], b'_' | b'@') {
            i += 1;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || matches!(b[i], b'_' | b'@')) {
                i += 1;
            }
        } else {
            i += 1;
        }
        out.push(Token {
            text: String::from_utf8_lossy(&b[start..i]).into_owned(),
            line: at,
            string: false,
        });
    }
    out
}
pub(super) fn is(t: &Token, s: &str) -> bool {
    !t.string && t.text == s
}
fn evidence(file: &str, t: &Token, description: String) -> ServiceEvidence {
    ServiceEvidence {
        file: file.into(),
        line: Some(t.line),
        pointer: String::new(),
        description,
    }
}
type Edges = BTreeMap<(String, String, Kind), BTreeSet<ServiceEvidence>>;
fn edge(
    edges: &mut Edges,
    from: &str,
    to: &str,
    kind: Kind,
    proof: impl IntoIterator<Item = ServiceEvidence>,
) {
    if from != to {
        edges
            .entry((from.into(), to.into(), kind))
            .or_default()
            .extend(proof);
    }
}
pub(super) fn arguments(tokens: &[Token], at: usize) -> Option<&[Token]> {
    let open = (at + 1..tokens.len()).find(|&i| is(&tokens[i], "("))?;
    let mut depth = 1;
    for i in open + 1..tokens.len() {
        if is(&tokens[i], "(") {
            depth += 1;
        }
        if is(&tokens[i], ")") {
            depth -= 1;
            if depth == 0 {
                return Some(&tokens[open + 1..i]);
            }
        }
    }
    None
}
// Keep local publication candidates inside their method. Conditional assignments
// retain alternatives; an unconditional reassignment invalidates the old value.
pub(super) fn scopes(tokens: &[Token]) -> Vec<(usize, bool)> {
    let mut result = Vec::new();
    let mut blocks = vec![(0, false)];
    let mut parens = Vec::new();
    let mut closed = None;
    for (i, t) in tokens.iter().enumerate() {
        if is(t, "(") {
            parens.push(i);
        }
        if is(t, ")") {
            closed = parens.pop();
        }
        if is(t, "{") {
            let parent = *blocks.last().unwrap();
            let before = i.checked_sub(1).and_then(|i| tokens.get(i));
            let keyword = closed
                .and_then(|at| at.checked_sub(1))
                .and_then(|at| tokens.get(at))
                .map(|t| t.text.as_str());
            let conditional = before.is_some_and(|t| is(t, "else"))
                || before.is_some_and(|t| is(t, ")"))
                    && matches!(
                        keyword,
                        Some("if" | "switch" | "for" | "foreach" | "while" | "catch")
                    );
            let method = before.is_some_and(|t| is(t, ")"))
                && !conditional
                && !matches!(keyword, Some("using" | "lock"));
            blocks.push(if method {
                (i, false)
            } else {
                (parent.0, parent.1 || conditional)
            });
        }
        result.push(*blocks.last().unwrap());
        if is(t, "}") && blocks.len() > 1 {
            blocks.pop();
        }
    }
    result
}
struct Source {
    file: String,
    tokens: Vec<Token>,
}
fn collect(
    root: &Path,
    dir: &Path,
    depth: usize,
    out: &mut Vec<Source>,
    report: &mut ServiceTopology,
    budget: &mut (usize, usize),
) {
    if depth > 30 || out.len() >= 2_000 || budget.0 >= 50_000 || budget.1 >= 32_000_000 {
        diagnostic(
            report,
            &relative(root, dir),
            None,
            "C# source discovery limit reached.",
        );
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().take(50_001).collect();
    entries.sort_by_key(|e| e.path());
    for entry in entries {
        budget.0 += 1;
        if budget.0 > 50_000 || budget.1 >= 32_000_000 {
            diagnostic(
                report,
                "",
                None,
                "C# communication discovery budget reached; remaining sources omitted.",
            );
            break;
        }
        let path = entry.path();
        let Ok(ty) = entry.file_type() else {
            continue;
        };
        if ty.is_symlink() {
            continue;
        }
        if ty.is_dir() {
            if !super::excluded(&entry.file_name().to_string_lossy())
                && !matches!(
                    entry.file_name().to_str(),
                    Some("bin" | "obj" | "node_modules")
                )
            {
                collect(root, &path, depth + 1, out, report, budget);
            }
            continue;
        }
        if path.extension().is_some_and(|x| x == "cs") {
            if entry.metadata().map_or(true, |m| m.len() > 1_048_576) {
                diagnostic(
                    report,
                    &relative(root, &path),
                    None,
                    "C# file exceeds static communication scan limit.",
                );
                continue;
            }
            if let Ok(s) = fs::read_to_string(&path) {
                budget.1 += s.len();
                out.push(Source {
                    file: relative(root, &path),
                    tokens: lex(&s),
                });
            }
        }
    }
}
#[derive(Clone)]
struct Deployment {
    id: String,
    project: String,
    host: String,
    name: String,
    refs: Vec<String>,
    waits: Vec<String>,
    proof: ServiceEvidence,
}
pub(super) fn augment(root: &Path, report: &mut ServiceTopology) {
    if !report.projects.iter().any(|p| p.ecosystem == "dotnet") {
        return;
    }
    let mut files = Vec::new();
    collect(root, root, 0, &mut files, report, &mut (0, 0));
    let mut deployments = Vec::new();
    let mut variables = BTreeMap::new();
    let mut brokers = BTreeMap::new();
    for src in &files {
        if !src
            .tokens
            .iter()
            .any(|t| is(t, "AddDotnetProject") || is(t, "AddProject"))
        {
            continue;
        }
        for stmt in src.tokens.split(|t| is(t, ";")) {
            let binding = stmt
                .windows(3)
                .find(|w| is(&w[0], "var") && is(&w[2], "="))
                .map(|w| w[1].text.clone());
            if let Some(at) = stmt.iter().position(|t| is(t, "AddRabbitMQ"))
                && let (Some(var), Some(name)) = (
                    binding.clone(),
                    arguments(stmt, at).and_then(|a| a.iter().find(|t| t.string)),
                )
            {
                brokers.insert((src.file.clone(), var), name.text.to_lowercase());
            }
            // Explicit Aspire infrastructure registration, scoped to this AppHost.
            if let Some(at) = stmt.iter().position(|t| {
                [
                    "AddRedis",
                    "AddRabbitMQ",
                    "AddSqlServer",
                    "AddPostgres",
                    "AddMySql",
                    "AddMongoDB",
                    "AddKafka",
                    "AddDatabase",
                    "AddHealthChecksUI",
                ]
                .iter()
                .any(|name| is(t, name))
            }) {
                if let Some(name) = arguments(stmt, at)
                    .and_then(|a| a.first())
                    .filter(|t| t.string)
                {
                    let id = format!("aspire:{}::{}", src.file, name.text);
                    if let Some(var) = binding.clone() {
                        variables.insert((src.file.clone(), var), id.clone());
                    }
                    let mut refs = Vec::new();
                    let mut waits = Vec::new();
                    if is(&stmt[at], "AddDatabase") && at >= 2 {
                        refs.push(stmt[at - 2].text.clone());
                    }
                    for (i, t) in stmt.iter().enumerate() {
                        if (is(t, "WithReference")
                            || is(t, "WaitFor")
                            || is(t, "WaitForCompletion"))
                            && let Some(arg) = arguments(stmt, i)
                                .and_then(|a| a.first())
                                .filter(|t| !t.string)
                        {
                            if is(t, "WithReference") {
                                refs.push(arg.text.clone());
                            } else {
                                waits.push(arg.text.clone());
                            }
                        }
                    }
                    deployments.push(Deployment {id,project:String::new(),host:src.file.clone(),name:name.text.clone(),refs,waits,proof:evidence(&src.file,&stmt[at],"Explicit Aspire infrastructure registration; no local source project inferred.".into())});
                }
                continue;
            }
            let Some(at) = stmt
                .iter()
                .position(|t| is(t, "AddDotnetProject") || is(t, "AddProject"))
            else {
                continue;
            };
            let Some(args) = arguments(stmt, at) else {
                continue;
            };
            let literals: Vec<_> = args.iter().filter(|t| t.string).collect();
            let Some(name) = literals.first() else {
                continue;
            };
            let candidates: Vec<_> = if is(&stmt[at], "AddDotnetProject") {
                let Some(path) = literals.get(1) else {
                    continue;
                };
                let base = root.join(&src.file);
                let target = base.parent().unwrap().join(path.text.replace('\\', "/"));
                let Ok(target) = fs::canonicalize(target) else {
                    continue;
                };
                if !target.starts_with(root) {
                    continue;
                }
                report
                    .projects
                    .iter()
                    .filter(|p| root.join(&p.path) == target || root.join(&p.manifest) == target)
                    .collect()
            } else {
                let Some(end) = stmt[at..].iter().position(|t| is(t, ">")) else {
                    continue;
                };
                let Some(typename) = stmt.get(at + end - 1) else {
                    continue;
                };
                report
                    .projects
                    .iter()
                    .filter(|p| p.name.replace(['.', '-'], "_") == typename.text)
                    .collect()
            };
            if candidates.len() != 1 {
                diagnostic(
                    report,
                    &src.file,
                    None,
                    "Aspire project registration is ambiguous or unsupported; no service mapping inferred.",
                );
                continue;
            }
            let project = candidates[0].clone();
            let id = format!("aspire:{}::{}", src.file, name.text);
            if let Some(var) = binding {
                variables.insert((src.file.clone(), var), id.clone());
            }
            let mut refs = Vec::new();
            let mut waits = Vec::new();
            for (i, t) in stmt.iter().enumerate() {
                if (is(t, "WithReference") || is(t, "WaitFor") || is(t, "WaitForCompletion"))
                    && let Some(arg) = arguments(stmt, i).and_then(|a| a.first())
                    && !arg.string
                {
                    if is(t, "WithReference") {
                        refs.push(arg.text.clone());
                    } else {
                        waits.push(arg.text.clone());
                    }
                }
            }
            let proof=evidence(&src.file,&stmt[at],"Literal Aspire registration maps a deployment to its project; configuration is not executed.".into());
            deployments.push(Deployment {
                id,
                project: project.id.clone(),
                host: src.file.clone(),
                name: name.text.clone(),
                refs,
                waits,
                proof,
            });
        }
    }
    if deployments.is_empty() {
        return;
    }
    let deployed: BTreeSet<_> = deployments.iter().map(|d| d.project.clone()).collect();
    report.services.retain(|s| {
        !s.compose_file.is_empty() || !s.project.as_ref().is_some_and(|p| deployed.contains(p))
    });
    for d in &deployments {
        report.services.push(Service {
            id: d.id.clone(),
            name: d.name.clone(),
            compose_file: String::new(),
            build_context: report
                .projects
                .iter()
                .find(|p| p.id == d.project)
                .map(|p| p.path.clone()),
            networks: vec![],
            profiles: vec![],
            project: (!d.project.is_empty()).then(|| d.project.clone()),
        });
    }
    let mut edges = Edges::new();
    for d in &deployments {
        for (names, kind) in [
            (&d.refs, Kind::ConfiguredReference),
            (&d.waits, Kind::Startup),
        ] {
            for var in names {
                if let Some(to) = variables.get(&(d.host.clone(), var.clone())) {
                    edge(&mut edges, &d.id, to, kind, [d.proof.clone()]);
                }
            }
        }
    }
    let mut publishers: Vec<(String, String, String, ServiceEvidence)> = Vec::new();
    let mut subscribers = Vec::new();
    for d in &deployments {
        let Some(project) = report.projects.iter().find(|p| p.id == d.project) else {
            continue;
        };
        let sources: Vec<_> = files
            .iter()
            .filter(|s| {
                s.file.starts_with(&(project.path.clone() + "/"))
                    && !report.projects.iter().any(|p| {
                        p.path != project.path
                            && p.path.starts_with(&(project.path.clone() + "/"))
                            && s.file.starts_with(&(p.path.clone() + "/"))
                    })
            })
            .collect();
        let bus_names: BTreeSet<_> = sources
            .iter()
            .flat_map(|s| {
                s.tokens
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| is(t, "AddRabbitMqEventBus"))
                    .filter_map(|(i, _)| {
                        arguments(&s.tokens, i)
                            .and_then(|a| a.first())
                            .filter(|t| t.string)
                            .map(|t| t.text.to_lowercase())
                    })
            })
            .collect();
        let shared: BTreeSet<_> = d
            .refs
            .iter()
            .filter_map(|r| brokers.get(&(d.host.clone(), r.clone())))
            .filter(|n| bus_names.contains(*n))
            .cloned()
            .collect();
        for src in sources {
            let mut resolved = src.tokens.clone();
            let mut uri_constants = BTreeMap::new();
            for stmt in src.tokens.split(|t| is(t, ";")) {
                for w in stmt.windows(7) {
                    if is(&w[0], "var")
                        && is(&w[2], "=")
                        && is(&w[3], "new")
                        && is(&w[4], "Uri")
                        && is(&w[5], "(")
                        && w[6].string
                    {
                        let writes = src
                            .tokens
                            .windows(2)
                            .filter(|a| a[0].text == w[1].text && is(&a[1], "="))
                            .count();
                        if writes == 1 {
                            uri_constants.insert(w[1].text.clone(), w[6].clone());
                        }
                    }
                }
            }
            for i in 2..resolved.len() {
                if is(&resolved[i - 1], "=")
                    && (is(&resolved[i - 2], "BaseAddress") || is(&resolved[i - 2], "Address"))
                    && let Some(value) = uri_constants.get(&resolved[i].text)
                {
                    resolved[i] = value.clone();
                }
            }
            let ts = &resolved;
            for (at, t) in ts.iter().enumerate() {
                if (is(t, "AddHttpClient") || is(t, "AddGrpcClient"))
                    && let Some(args) = arguments(ts, at)
                {
                    for (endpoint_at, endpoint) in args.iter().enumerate().filter(|(_, t)| t.string)
                    {
                        let destination = args[..endpoint_at]
                            .iter()
                            .rposition(|t| is(t, "="))
                            .filter(|&i| {
                                i > 0
                                    && ["BaseAddress", "Address"]
                                        .iter()
                                        .any(|name| is(&args[i - 1], name))
                            });
                        if destination.is_none() {
                            continue;
                        }

                        let Some((scheme, rest)) = endpoint.text.split_once("://") else {
                            continue;
                        };
                        if !["http", "https", "https+http"].contains(&scheme) {
                            continue;
                        }
                        let hostname = rest.split(['/', ':', '?', '#']).next().unwrap_or_default();
                        if hostname.contains('@') {
                            continue;
                        }
                        let targets: Vec<_> = deployments
                            .iter()
                            .filter(|target| {
                                target.host == d.host && target.name.eq_ignore_ascii_case(hostname)
                            })
                            .collect();
                        if targets.len() == 1 {
                            edge(&mut edges,&d.id,&targets[0].id,if is(t,"AddGrpcClient"){Kind::GrpcCall}else{Kind::HttpCall},[evidence(&src.file,t,"Literal typed-client destination mapped to an Aspire service. Static client configuration; execution is not proven. URL values omitted.".into()),targets[0].proof.clone()]);
                        }
                    }
                }
                if is(t, "AddSubscription")
                    && ts.get(at + 1).is_some_and(|t| is(t, "<"))
                    && let Some(event) = ts.get(at + 2)
                {
                    for bus in &shared {
                        subscribers.push((
                            d.id.clone(),
                            format!("{}::{bus}", d.host),
                            event.text.clone(),
                            evidence(
                                &src.file,
                                t,
                                format!(
                                    "Subscription to {} on shared RabbitMQ resource {bus}.",
                                    event.text
                                ),
                            ),
                        ));
                    }
                }
            }
            // Track event construction assigned to a local that reaches a publish/outbox call.
            // No edge is inferred from a matching class declaration alone.
            let mut assigned: BTreeMap<(usize, String), BTreeSet<String>> = BTreeMap::new();
            let scope = scopes(ts);
            let mut offset = 0;
            for stmt in ts.split(|t| is(t, ";")) {
                let constructions: BTreeSet<_> = stmt
                    .windows(3)
                    .filter(|w| {
                        is(&w[0], "new")
                            && w[1].text.ends_with("IntegrationEvent")
                            && is(&w[2], "(")
                    })
                    .map(|w| w[1].text.clone())
                    .collect();
                if let Some(eq) = stmt.iter().position(|t| is(t, "="))
                    && eq > 0
                {
                    let (method, conditional) = scope[offset + eq];
                    let key = (method, stmt[eq - 1].text.clone());
                    if conditional {
                        assigned
                            .entry(key)
                            .or_default()
                            .extend(constructions.clone());
                    } else {
                        assigned.insert(key, constructions.clone());
                    }
                }
                for (i, t) in stmt.iter().enumerate().filter(|(_, t)| {
                    [
                        "PublishAsync",
                        "PublishThroughEventBusAsync",
                        "AddAndSaveEventAsync",
                    ]
                    .iter()
                    .any(|n| is(t, n))
                }) {
                    let Some(args) = arguments(stmt, i) else {
                        continue;
                    };
                    let mut events = BTreeSet::new();
                    if args.first().is_some_and(|t| is(t, "new")) {
                        events.extend(constructions.clone());
                    }
                    if let Some(arg) = args.first()
                        && let Some(values) = assigned.get(&(scope[offset + i].0, arg.text.clone()))
                    {
                        events.extend(values.clone());
                    }
                    for event in events {
                        for bus in &shared {
                            publishers.push((d.id.clone(),format!("{}::{bus}",d.host),event.clone(),evidence(&src.file,t,format!("Constructed {event} reaches {} on shared RabbitMQ resource {bus}. Inferred event-name route; static candidate, not a runtime trace.",t.text))));
                        }
                    }
                }
                offset += stmt.len() + 1;
            }
        }
    }
    for (from, bus, event, proof) in &publishers {
        for (to, target_bus, target_event, subscription) in &subscribers {
            if bus == target_bus && event == target_event {
                edge(
                    &mut edges,
                    from,
                    to,
                    Kind::Event,
                    [proof.clone(), subscription.clone()],
                );
            }
        }
    }
    report
        .dependencies
        .extend(
            edges
                .into_iter()
                .map(|((from, to, kind), evidence)| ServiceDependency {
                    from,
                    to,
                    kind,
                    evidence: evidence.into_iter().collect(),
                }),
        );
    report.services.sort_by(|a, b| a.id.cmp(&b.id));
    report.services.dedup_by(|a, b| a.id == b.id);
    diagnostic(
        report,
        "",
        None,
        "Aspire/.NET communication coverage is partial: literal project registrations, typed HTTP/gRPC clients and conventional RabbitMQ publisher/subscription candidates only. Branches, dynamic routing, custom brokers and runtime execution are not evaluated. Event cycles may describe intended workflows, not defects.",
    );
}
