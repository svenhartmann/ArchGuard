//! Bounded, non-executing reader for literal Aspire/.NET communication patterns.
//! These are static candidates, not traces. Dynamic expressions stay unknown.
use super::support::{diagnostic, relative};
use archguard_core::{
    Service, ServiceDependency, ServiceDependencyKind as Kind, ServiceEvidence, ServiceProject,
    ServiceTopology,
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
            if !super::support::excluded(&entry.file_name().to_string_lossy())
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
    /// Resource this one was created from, such as the server of a database.
    parent: Option<String>,
    refs: Vec<String>,
    waits: Vec<String>,
    proof: ServiceEvidence,
}
const PROJECT_CALLS: [&str; 2] = ["AddDotnetProject", "AddProject"];
/// Value-only resources: configuration, not services.
const VALUE_CALLS: [&str; 2] = ["AddParameter", "AddParameterFromConfiguration"];
const DEPENDENCY_CALLS: [&str; 3] = ["WithReference", "WaitFor", "WaitForCompletion"];
const CLIENT_CALLS: [&str; 2] = ["AddHttpClient", "AddGrpcClient"];
const ADDRESS_NAMES: [&str; 2] = ["BaseAddress", "Address"];
const PUBLISH_CALLS: [&str; 3] = [
    "PublishAsync",
    "PublishThroughEventBusAsync",
    "AddAndSaveEventAsync",
];
/// AppHost file and resource variable that name a registration.
type Binding = (String, String);
/// Deployment id, shared bus, event name and the evidence for one side of an event route.
type Message = (String, String, String, ServiceEvidence);
/// Event types last assigned to a local, keyed by method scope and local name.
type Assignments = BTreeMap<(usize, String), BTreeSet<String>>;
fn is_any(t: &Token, names: &[&str]) -> bool {
    names.iter().any(|name| is(t, name))
}
fn binding(stmt: &[Token]) -> Option<String> {
    stmt.windows(3)
        .find(|w| is(&w[0], "var") && is(&w[2], "="))
        .map(|w| w[1].text.clone())
}
/// Variables passed to `WithReference` (first) and `WaitFor*` (second).
fn dependencies(stmt: &[Token]) -> (Vec<String>, Vec<String>) {
    let mut refs = Vec::new();
    let mut waits = Vec::new();
    for (i, t) in stmt.iter().enumerate() {
        if !is_any(t, &DEPENDENCY_CALLS) {
            continue;
        }
        let Some(arg) = arguments(stmt, i)
            .and_then(|a| a.first())
            .filter(|t| !t.string)
        else {
            continue;
        };
        if is(t, "WithReference") {
            refs.push(arg.text.clone());
        } else {
            waits.push(arg.text.clone());
        }
    }
    (refs, waits)
}
fn bus_names(sources: &[&Source]) -> BTreeSet<String> {
    sources
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
        .collect()
}
/// Locals declared once as `var x = new Uri("...")`, mapped to their literal.
fn uri_constants(tokens: &[Token]) -> BTreeMap<String, Token> {
    let mut constants = BTreeMap::new();
    for stmt in tokens.split(|t| is(t, ";")) {
        for w in stmt.windows(7).filter(|w| {
            is(&w[0], "var")
                && is(&w[2], "=")
                && is(&w[3], "new")
                && is(&w[4], "Uri")
                && is(&w[5], "(")
                && w[6].string
        }) {
            let writes = tokens
                .windows(2)
                .filter(|a| a[0].text == w[1].text && is(&a[1], "="))
                .count();
            if writes == 1 {
                constants.insert(w[1].text.clone(), w[6].clone());
            }
        }
    }
    constants
}
/// Tokens with constant URIs substituted where they are assigned to a client address.
fn resolve_addresses(tokens: &[Token]) -> Vec<Token> {
    let constants = uri_constants(tokens);
    let mut resolved = tokens.to_vec();
    for i in 2..resolved.len() {
        if is(&resolved[i - 1], "=")
            && is_any(&resolved[i - 2], &ADDRESS_NAMES)
            && let Some(value) = constants.get(&resolved[i].text)
        {
            resolved[i] = value.clone();
        }
    }
    resolved
}
/// Whether a literal after `before` is assigned to a client address.
fn is_destination(before: &[Token]) -> bool {
    before
        .iter()
        .rposition(|t| is(t, "="))
        .is_some_and(|i| i > 0 && is_any(&before[i - 1], &ADDRESS_NAMES))
}
/// Host of an http(s) URL without user information.
fn hostname(url: &str) -> Option<&str> {
    let (scheme, rest) = url.split_once("://")?;
    if !["http", "https", "https+http"].contains(&scheme) {
        return None;
    }
    let hostname = rest.split(['/', ':', '?', '#']).next().unwrap_or_default();
    (!hostname.contains('@')).then_some(hostname)
}
fn constructions(stmt: &[Token]) -> BTreeSet<String> {
    stmt.windows(3)
        .filter(|w| is(&w[0], "new") && w[1].text.ends_with("IntegrationEvent") && is(&w[2], "("))
        .map(|w| w[1].text.clone())
        .collect()
}
fn assign(
    assigned: &mut Assignments,
    key: (usize, String),
    events: &BTreeSet<String>,
    conditional: bool,
) {
    if conditional {
        assigned.entry(key).or_default().extend(events.clone());
    } else {
        assigned.insert(key, events.clone());
    }
}
/// Events that reach a publish call: constructed inline or held by its first argument.
fn published(
    args: &[Token],
    constructed: &BTreeSet<String>,
    assigned: &Assignments,
    method: usize,
) -> BTreeSet<String> {
    let mut events = BTreeSet::new();
    if args.first().is_some_and(|t| is(t, "new")) {
        events.extend(constructed.clone());
    }
    if let Some(arg) = args.first()
        && let Some(values) = assigned.get(&(method, arg.text.clone()))
    {
        events.extend(values.clone());
    }
    events
}
/// How one resource call is bound and linked within its statement.
#[derive(Default)]
struct Registration {
    binding: Option<String>,
    parent: Option<String>,
    refs: Vec<String>,
    waits: Vec<String>,
}
/// AppHost registrations read from literal Aspire calls.
struct Aspire<'a> {
    root: &'a Path,
    report: &'a mut ServiceTopology,
    deployments: Vec<Deployment>,
    variables: BTreeMap<Binding, String>,
    brokers: BTreeMap<Binding, String>,
    /// String constants of the AppHost file being read.
    names: BTreeMap<String, String>,
}
impl Aspire<'_> {
    /// Locals and constants assigned a string literal exactly once, such as a resource name.
    fn string_constants(tokens: &[Token]) -> BTreeMap<String, String> {
        let writes = |name: &str| {
            tokens
                .windows(2)
                .filter(|a| a[0].text == name && is(&a[1], "="))
                .count()
        };
        tokens
            .split(|t| is(t, ";"))
            .filter_map(|stmt| match stmt {
                [.., name, eq, value] if is(eq, "=") && value.string && writes(&name.text) == 1 => {
                    Some((name.text.clone(), value.text.clone()))
                }
                _ => None,
            })
            .collect()
    }
    /// Top-level `Add…` calls that create Aspire resources: on the builder, on a
    /// resource variable or chained on such a call, outside lambdas and arguments.
    fn resource_calls(stmt: &[Token]) -> Vec<usize> {
        let mut depth = 0usize;
        let mut calls = Vec::new();
        for (i, t) in stmt.iter().enumerate() {
            if is(t, "(") {
                depth += 1;
            } else if is(t, ")") {
                depth = depth.saturating_sub(1);
            } else if depth == 0 && Self::is_resource_call(stmt, i) {
                calls.push(i);
            }
        }
        calls
    }
    /// `receiver.AddName(` or `receiver.AddName<` where the receiver starts a chain.
    fn is_resource_call(stmt: &[Token], i: usize) -> bool {
        let t = &stmt[i];
        let named = !t.string
            && t.text.len() > 3
            && t.text.starts_with("Add")
            && t.text.as_bytes()[3].is_ascii_uppercase()
            && !is_any(t, &VALUE_CALLS);
        let called = stmt.get(i + 1).is_some_and(|n| is(n, "(") || is(n, "<"));
        let receiver = i >= 2
            && is(&stmt[i - 1], ".")
            && (is(&stmt[i - 2], ")") || i < 3 || !is(&stmt[i - 3], "."));
        named && called && receiver
    }
    fn read(&mut self, files: &[Source]) {
        for src in files {
            let apphost = src
                .tokens
                .iter()
                .any(|t| is_any(t, &PROJECT_CALLS) || is(t, "DistributedApplication"));
            if apphost {
                self.names = Self::string_constants(&src.tokens);
                for stmt in src.tokens.split(|t| is(t, ";")) {
                    self.statement(src, stmt);
                }
            }
        }
    }
    fn statement(&mut self, src: &Source, stmt: &[Token]) {
        let binding = binding(stmt);
        if let Some(at) = stmt.iter().position(|t| is(t, "AddRabbitMQ"))
            && let (Some(var), Some(name)) = (
                binding.clone(),
                arguments(stmt, at).and_then(|a| a.iter().find(|t| t.string)),
            )
        {
            self.brokers
                .insert((src.file.clone(), var), name.text.to_lowercase());
        }
        let calls = Self::resource_calls(stmt);
        let Some(&last) = calls.last() else {
            self.configure(src, stmt);
            return;
        };
        let mut parent = calls
            .first()
            .and_then(|&at| self.variable(src, &stmt[at - 2]));
        for &at in &calls {
            let mut registration = Registration {
                parent,
                ..Default::default()
            };
            if at == last {
                registration.binding = binding.clone();
                (registration.refs, registration.waits) = dependencies(stmt);
            }
            parent = self.resource(src, stmt, at, registration);
        }
    }
    /// The deployment id an AppHost variable holds, if any.
    fn variable(&self, src: &Source, t: &Token) -> Option<String> {
        self.variables
            .get(&(src.file.clone(), t.text.clone()))
            .cloned()
    }
    /// `resource.WithReference(...)` as a statement of its own.
    fn configure(&mut self, src: &Source, stmt: &[Token]) {
        if !stmt.get(1).is_some_and(|t| is(t, ".")) {
            return;
        }
        let Some(id) = stmt.first().and_then(|t| self.variable(src, t)) else {
            return;
        };
        let (refs, waits) = dependencies(stmt);
        if let Some(d) = self.deployments.iter_mut().find(|d| d.id == id) {
            d.refs.extend(refs);
            d.waits.extend(waits);
        }
    }
    /// One literal resource registration, mapped to a local project where its
    /// generic type or path argument names exactly one. Returns its id.
    fn resource(
        &mut self,
        src: &Source,
        stmt: &[Token],
        at: usize,
        registration: Registration,
    ) -> Option<String> {
        let Registration {
            binding,
            parent,
            refs,
            waits,
        } = registration;
        let call = &stmt[at];
        let args = arguments(stmt, at)?;
        let constant = match args {
            [t] if !t.string => self.names.get(&t.text).map(|text| Token {
                text: text.clone(),
                line: t.line,
                string: true,
            }),
            _ => None,
        };
        let Some(name) = args.first().filter(|t| t.string).or(constant.as_ref()) else {
            // Unbound calls without a literal name are usually helper extensions.
            if binding.is_some() {
                let message = format!(
                    "Aspire {} resource name is not a string literal; resource not registered.",
                    call.text
                );
                diagnostic(self.report, &src.file, None, &message);
            }
            return None;
        };
        let candidates = if is(&stmt[at + 1], "<") {
            self.projects_of_type(&stmt[at..])
        } else {
            args.iter()
                .filter(|t| t.string)
                .nth(1)
                .and_then(|path| self.projects_at(&src.file, &path.text))
        }
        .unwrap_or_default();
        let project_call = is_any(call, &PROJECT_CALLS);
        if candidates.len() > 1 || project_call && candidates.is_empty() {
            diagnostic(
                self.report,
                &src.file,
                None,
                "Aspire project registration is ambiguous or unsupported; no service mapping inferred.",
            );
            if project_call {
                return None;
            }
        }
        let project = if candidates.len() == 1 {
            candidates[0].clone()
        } else {
            String::new()
        };
        let description = if project.is_empty() {
            format!(
                "Explicit Aspire resource registration ({}); no local source project inferred.",
                call.text
            )
        } else {
            "Literal Aspire registration maps a deployment to its project; configuration is not executed.".into()
        };
        let id = self.bind(src, name, binding);
        self.deployments.push(Deployment {
            id: id.clone(),
            project,
            host: src.file.clone(),
            name: name.text.clone(),
            parent,
            refs,
            waits,
            proof: evidence(&src.file, call, description),
        });
        Some(id)
    }
    /// Projects whose path or manifest is the `AddDotnetProject` path relative to the AppHost.
    fn projects_at(&self, file: &str, path: &str) -> Option<Vec<String>> {
        let base = self.root.join(file);
        let target = base.parent().unwrap().join(path.replace('\\', "/"));
        let target = fs::canonicalize(target).ok()?;
        if !target.starts_with(self.root) {
            return None;
        }
        let at_target = |p: &&ServiceProject| {
            self.root.join(&p.path) == target || self.root.join(&p.manifest) == target
        };
        Some(
            self.report
                .projects
                .iter()
                .filter(at_target)
                .map(|p| p.id.clone())
                .collect(),
        )
    }
    /// Projects whose name matches the generic argument of `AddProject<...>`.
    fn projects_of_type(&self, call: &[Token]) -> Option<Vec<String>> {
        let end = call.iter().position(|t| is(t, ">"))?;
        let typename = call.get(end - 1)?;
        Some(
            self.report
                .projects
                .iter()
                .filter(|p| p.name.replace(['.', '-'], "_") == typename.text)
                .map(|p| p.id.clone())
                .collect(),
        )
    }
    /// The deployment id for `name`, remembered under its AppHost variable.
    fn bind(&mut self, src: &Source, name: &Token, binding: Option<String>) -> String {
        let id = format!("aspire:{}::{}", src.file, name.text);
        if let Some(var) = binding {
            self.variables.insert((src.file.clone(), var), id.clone());
        }
        id
    }
    /// Aspire deployments replace the plain project services they deploy.
    fn replace_services(&mut self) {
        let deployed: BTreeSet<_> = self.deployments.iter().map(|d| d.project.clone()).collect();
        self.report.services.retain(|s| {
            !s.compose_file.is_empty() || !s.project.as_ref().is_some_and(|p| deployed.contains(p))
        });
        for d in &self.deployments {
            let build_context = self
                .report
                .projects
                .iter()
                .find(|p| p.id == d.project)
                .map(|p| p.path.clone());
            self.report.services.push(Service {
                id: d.id.clone(),
                name: d.name.clone(),
                compose_file: String::new(),
                build_context,
                networks: vec![],
                profiles: vec![],
                project: (!d.project.is_empty()).then(|| d.project.clone()),
            });
        }
    }
    fn configured_edges(&self) -> Edges {
        let mut edges = Edges::new();
        for d in &self.deployments {
            if let Some(parent) = &d.parent {
                edge(
                    &mut edges,
                    &d.id,
                    parent,
                    Kind::ConfiguredReference,
                    [d.proof.clone()],
                );
            }
            for (names, kind) in [
                (&d.refs, Kind::ConfiguredReference),
                (&d.waits, Kind::Startup),
            ] {
                for var in names {
                    if let Some(to) = self.variables.get(&(d.host.clone(), var.clone())) {
                        edge(&mut edges, &d.id, to, kind, [d.proof.clone()]);
                    }
                }
            }
        }
        edges
    }
    fn finish(self, edges: Edges) {
        let report = self.report;
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
            "Aspire/.NET communication coverage is partial: literal resource registrations (projects, containers, executables, integrations, external services; parameters excluded), typed HTTP/gRPC clients and conventional RabbitMQ publisher/subscription candidates only. Branches, dynamic routing, custom brokers and runtime execution are not evaluated. Event cycles may describe intended workflows, not defects.",
        );
    }
}
/// One deployment's source file and the RabbitMQ resources it shares with its AppHost.
struct Scan<'s> {
    deployment: &'s Deployment,
    file: &'s str,
    shared: &'s BTreeSet<String>,
}
/// Client calls and event routes found in the sources of deployed projects.
struct Flows<'a> {
    deployments: &'a [Deployment],
    projects: &'a [ServiceProject],
    brokers: &'a BTreeMap<Binding, String>,
    edges: Edges,
    publishers: Vec<Message>,
    subscribers: Vec<Message>,
}
impl Flows<'_> {
    fn deployment(&mut self, files: &[Source], d: &Deployment) {
        let Some(project) = self.projects.iter().find(|p| p.id == d.project) else {
            return;
        };
        let sources = self.sources(files, project);
        let bus_names = bus_names(&sources);
        let shared: BTreeSet<_> = d
            .refs
            .iter()
            .filter_map(|r| self.brokers.get(&(d.host.clone(), r.clone())))
            .filter(|n| bus_names.contains(*n))
            .cloned()
            .collect();
        for src in sources {
            let scan = Scan {
                deployment: d,
                file: &src.file,
                shared: &shared,
            };
            let tokens = resolve_addresses(&src.tokens);
            for at in 0..tokens.len() {
                self.client(&scan, &tokens, at);
                self.subscription(&scan, &tokens, at);
            }
            self.publications(&scan, &tokens);
        }
    }
    /// Sources inside `project` that do not belong to a project nested within it.
    fn sources<'f>(&self, files: &'f [Source], project: &ServiceProject) -> Vec<&'f Source> {
        let prefix = project.path.clone() + "/";
        let nested = |file: &str| {
            self.projects.iter().any(|p| {
                p.path != project.path
                    && p.path.starts_with(&prefix)
                    && file.starts_with(&(p.path.clone() + "/"))
            })
        };
        files
            .iter()
            .filter(|s| s.file.starts_with(&prefix) && !nested(&s.file))
            .collect()
    }
    fn client(&mut self, scan: &Scan, ts: &[Token], at: usize) {
        let t = &ts[at];
        if !is_any(t, &CLIENT_CALLS) {
            return;
        }
        let Some(args) = arguments(ts, at) else {
            return;
        };
        let d = scan.deployment;
        let kind = if is(t, "AddGrpcClient") {
            Kind::GrpcCall
        } else {
            Kind::HttpCall
        };
        for (endpoint_at, endpoint) in args.iter().enumerate().filter(|(_, t)| t.string) {
            if !is_destination(&args[..endpoint_at]) {
                continue;
            }
            let Some(hostname) = hostname(&endpoint.text) else {
                continue;
            };
            let targets: Vec<_> = self
                .deployments
                .iter()
                .filter(|target| {
                    target.host == d.host && target.name.eq_ignore_ascii_case(hostname)
                })
                .collect();
            if targets.len() == 1 {
                let proof = evidence(
                    scan.file,
                    t,
                    "Literal typed-client destination mapped to an Aspire service. Static client configuration; execution is not proven. URL values omitted."
                        .into(),
                );
                let target = targets[0];
                edge(
                    &mut self.edges,
                    &d.id,
                    &target.id,
                    kind,
                    [proof, target.proof.clone()],
                );
            }
        }
    }
    fn subscription(&mut self, scan: &Scan, ts: &[Token], at: usize) {
        let t = &ts[at];
        if !(is(t, "AddSubscription") && ts.get(at + 1).is_some_and(|t| is(t, "<"))) {
            return;
        }
        let Some(event) = ts.get(at + 2) else {
            return;
        };
        let d = scan.deployment;
        for bus in scan.shared {
            self.subscribers.push((
                d.id.clone(),
                format!("{}::{bus}", d.host),
                event.text.clone(),
                evidence(
                    scan.file,
                    t,
                    format!(
                        "Subscription to {} on shared RabbitMQ resource {bus}.",
                        event.text
                    ),
                ),
            ));
        }
    }
    // Track event construction assigned to a local that reaches a publish/outbox call.
    // No edge is inferred from a matching class declaration alone.
    fn publications(&mut self, scan: &Scan, ts: &[Token]) {
        let mut assigned = Assignments::new();
        let scope = scopes(ts);
        let mut offset = 0;
        for stmt in ts.split(|t| is(t, ";")) {
            let constructed = constructions(stmt);
            if let Some(eq) = stmt.iter().position(|t| is(t, "="))
                && eq > 0
            {
                let (method, conditional) = scope[offset + eq];
                let key = (method, stmt[eq - 1].text.clone());
                assign(&mut assigned, key, &constructed, conditional);
            }
            for (i, t) in stmt
                .iter()
                .enumerate()
                .filter(|(_, t)| is_any(t, &PUBLISH_CALLS))
            {
                if let Some(args) = arguments(stmt, i) {
                    let events = published(args, &constructed, &assigned, scope[offset + i].0);
                    self.publish(scan, t, events);
                }
            }
            offset += stmt.len() + 1;
        }
    }
    fn publish(&mut self, scan: &Scan, t: &Token, events: BTreeSet<String>) {
        let d = scan.deployment;
        for event in events {
            for bus in scan.shared {
                let description = format!(
                    "Constructed {event} reaches {} on shared RabbitMQ resource {bus}. Inferred event-name route; static candidate, not a runtime trace.",
                    t.text
                );
                self.publishers.push((
                    d.id.clone(),
                    format!("{}::{bus}", d.host),
                    event.clone(),
                    evidence(scan.file, t, description),
                ));
            }
        }
    }
    fn link_events(&mut self) {
        for (from, bus, event, proof) in &self.publishers {
            for (to, target_bus, target_event, subscription) in &self.subscribers {
                if bus == target_bus && event == target_event {
                    edge(
                        &mut self.edges,
                        from,
                        to,
                        Kind::Event,
                        [proof.clone(), subscription.clone()],
                    );
                }
            }
        }
    }
}
pub(super) fn augment(root: &Path, report: &mut ServiceTopology) {
    if !report.projects.iter().any(|p| p.ecosystem == "dotnet") {
        return;
    }
    let mut files = Vec::new();
    collect(root, root, 0, &mut files, report, &mut (0, 0));
    let mut aspire = Aspire {
        root,
        report,
        deployments: Vec::new(),
        variables: BTreeMap::new(),
        brokers: BTreeMap::new(),
        names: BTreeMap::new(),
    };
    aspire.read(&files);
    if aspire.deployments.is_empty() {
        return;
    }
    aspire.replace_services();
    let mut flows = Flows {
        deployments: &aspire.deployments,
        projects: &aspire.report.projects,
        brokers: &aspire.brokers,
        edges: aspire.configured_edges(),
        publishers: Vec::new(),
        subscribers: Vec::new(),
    };
    for d in &aspire.deployments {
        flows.deployment(&files, d);
    }
    flows.link_events();
    let edges = flows.edges;
    aspire.finish(edges);
}
