//! Small, bounded Go value-flow reader. Client construction alone is not a call.
use super::{
    service_communications::{Token, lex},
    service_deployments::{self as deployment, Deployment},
};
use archguard_core::{ServiceDependencyKind as Kind, ServiceTopology};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
#[derive(Clone)]
struct Function {
    name: String,
    params: Vec<String>,
    body: Vec<Token>,
    receiver: Option<(String, String)>,
}
#[derive(Clone)]
enum Helper {
    Env(usize, usize),
    Dial(usize, Option<usize>),
}
#[derive(Clone)]
struct Value {
    hosts: BTreeSet<String>,
    grpc: bool,
    connection: bool,
}
fn text(t: &Token, s: &str) -> bool {
    !t.string && t.text == s
}
fn close(ts: &[Token], at: usize, left: &str, right: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, t) in ts.iter().enumerate().skip(at) {
        if text(t, left) {
            depth += 1;
        }
        if text(t, right) {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}
fn args(ts: &[Token], start: usize, end: usize) -> Vec<&[Token]> {
    let mut out = vec![];
    let mut begin = start;
    let mut depth = 0;
    for i in start..end {
        if ts[i].string {
            continue;
        }
        match ts[i].text.as_str() {
            "(" | "[" | "{" => depth += 1,
            ")" | "]" | "}" => depth -= 1,
            "," if depth == 0 => {
                out.push(&ts[begin..i]);
                begin = i + 1
            }
            _ => {}
        }
    }
    if begin < end {
        out.push(&ts[begin..end]);
    }
    out
}
// The shared lexer uses C-like quoted strings. Mask Go raw strings before
// lexing so examples embedded in backticks cannot become executable evidence.
fn go_tokens(source: &str) -> Vec<Token> {
    let mut bytes = source.as_bytes().to_vec();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"//") {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
        } else if bytes[i..].starts_with(b"/*") {
            i += 2;
            while i < bytes.len() && !bytes[i..].starts_with(b"*/") {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
        } else if matches!(bytes[i], b'"' | b'\'') {
            let quote = bytes[i];
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(bytes.len());
                    continue;
                }
                if bytes[i] == quote {
                    i += 1;
                    break;
                }
                i += 1;
            }
        } else if bytes[i] == b'`' {
            bytes[i] = b'"';
            i += 1;
            while i < bytes.len() && bytes[i] != b'`' {
                if bytes[i] != b'\n' {
                    bytes[i] = b' ';
                }
                i += 1;
            }
            if i < bytes.len() {
                bytes[i] = b'"';
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    lex(&String::from_utf8_lossy(&bytes))
}
fn functions(ts: &[Token]) -> Vec<Function> {
    let mut out = vec![];
    let mut at = 0;
    while at < ts.len() {
        if !text(&ts[at], "func") {
            at += 1;
            continue;
        }
        let mut n = at + 1;
        let mut receiver = None;
        if ts.get(n).is_some_and(|t| text(t, "(")) {
            let Some(end) = close(ts, n, "(", ")") else {
                break;
            };
            if end > n + 2 {
                receiver = Some((ts[n + 1].text.clone(), ts[end - 1].text.clone()));
            }
            n = end + 1;
        }
        let Some(name) = ts.get(n) else { break };
        if !ts.get(n + 1).is_some_and(|t| text(t, "(")) {
            at += 1;
            continue;
        }
        let Some(end) = close(ts, n + 1, "(", ")") else {
            break;
        };
        let params = args(ts, n + 2, end)
            .iter()
            .filter_map(|a| a.first().map(|t| t.text.clone()))
            .collect();
        let Some(start) = ts
            .iter()
            .enumerate()
            .skip(end + 1)
            .find(|(_, t)| text(t, "{"))
            .map(|(i, _)| i)
        else {
            break;
        };
        let Some(stop) = close(ts, start, "{", "}") else {
            break;
        };
        out.push(Function {
            name: name.text.clone(),
            params,
            body: ts[start + 1..stop].to_vec(),
            receiver,
        });
        at = stop + 1;
    }
    out
}
// Assignment targets in the supported simple helper grammar (including Go multi-assign).
fn assignment_targets(ts: &[Token], eq: usize) -> Vec<(String, bool)> {
    if eq == 0
        || !text(&ts[eq], "=")
        || ts.get(eq + 1).is_some_and(|t| text(t, "="))
        || text(&ts[eq - 1], "=")
    {
        return vec![];
    }
    let mut end = eq;
    if text(&ts[end - 1], ":") {
        end -= 1;
    }
    let mut out = Vec::new();
    while end > 0 {
        let name = &ts[end - 1];
        if name.string
            || !name
                .text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        {
            break;
        }
        end -= 1;
        let pointer = end > 0 && text(&ts[end - 1], "*");
        if pointer {
            end -= 1;
        }
        out.push((name.text.clone(), pointer));
        if end == 0 || !text(&ts[end - 1], ",") {
            break;
        }
        end -= 1;
    }
    out.reverse();
    out
}
fn writes(ts: &[Token], name: &str) -> usize {
    // Passing a tracked binding by address can mutate it outside this grammar.
    let escapes = ts
        .windows(2)
        .filter(|w| text(&w[0], "&") && w[1].text == name)
        .count();
    escapes
        + ts.iter()
            .enumerate()
            .filter(|(i, t)| {
                text(t, "=") && assignment_targets(ts, *i).iter().any(|(n, _)| n == name)
            })
            .count()
}
fn call_assignment(ts: &[Token], name_at: usize) -> Option<(String, bool)> {
    // package.Function(...), immediately following =; no nested or unrelated call.
    let eq = name_at.checked_sub(3)?;
    if !text(&ts[eq], "=") {
        return None;
    }
    assignment_targets(ts, eq).first().cloned()
}
fn helpers(fs: &[Function]) -> BTreeMap<String, Helper> {
    let mut out = BTreeMap::new();
    for f in fs {
        if f.receiver.is_some() || fs.iter().filter(|other| other.name == f.name).count() != 1 {
            continue;
        }
        let ts = &f.body;
        for i in 2..ts.len() {
            if !ts.get(i + 1).is_some_and(|t| text(t, "(")) {
                continue;
            }
            let Some(end) = close(ts, i + 1, "(", ")") else {
                continue;
            };
            let arguments = args(ts, i + 2, end);
            if text(&ts[i], "Getenv")
                && text(&ts[i - 2], "os")
                && arguments.len() == 1
                && arguments[0].len() == 1
            {
                let Some(key) = f.params.iter().position(|p| p == &arguments[0][0].text) else {
                    continue;
                };
                if writes(ts, &f.params[key]) != 0 {
                    continue;
                }
                let Some((local, pointer)) = call_assignment(ts, i) else {
                    continue;
                };
                if writes(ts, &local) != 1 {
                    continue;
                }
                if ts.get(end + 1).is_some_and(|next| {
                    next.line == ts[end].line && ![";", "}"].iter().any(|s| text(next, s))
                }) {
                    continue;
                }
                if pointer {
                    if ts.iter().filter(|t| !t.string && t.text == local).count() == 1
                        && let Some(target) = f.params.iter().position(|p| p == &local)
                    {
                        out.insert(f.name.clone(), Helper::Env(target, key));
                    }
                } else {
                    for (at, w) in ts.windows(4).enumerate().skip(end + 1) {
                        if text(&w[0], "*")
                            && text(&w[2], "=")
                            && w[3].text == local
                            && writes(ts, &w[1].text) == 1
                            && ts
                                .iter()
                                .filter(|t| !t.string && t.text == w[1].text)
                                .count()
                                == 1
                            && ts.get(at + 4).is_none_or(|next| {
                                next.line > w[3].line || text(next, ";") || text(next, "}")
                            })
                            && let Some(target) = f.params.iter().position(|p| p == &w[1].text)
                        {
                            out.insert(f.name.clone(), Helper::Env(target, key));
                        }
                    }
                }
            }
            if ["NewClient", "Dial", "DialContext"]
                .iter()
                .any(|n| text(&ts[i], n))
                && text(&ts[i - 2], "grpc")
            {
                let index = usize::from(text(&ts[i], "DialContext"));
                let Some(arg) = arguments.get(index).filter(|arg| arg.len() == 1) else {
                    continue;
                };
                let Some(param) = f.params.iter().position(|p| p == &arg[0].text) else {
                    continue;
                };
                if writes(ts, &f.params[param]) != 0 {
                    continue;
                }
                let Some((local, pointer)) = call_assignment(ts, i) else {
                    continue;
                };
                if writes(ts, &local) != 1 {
                    continue;
                }
                if pointer {
                    if ts.iter().filter(|t| !t.string && t.text == local).count() == 1
                        && let Some(target) = f.params.iter().position(|p| p == &local)
                    {
                        out.insert(f.name.clone(), Helper::Dial(param, Some(target)));
                    }
                } else {
                    let returns: Vec<_> = ts
                        .iter()
                        .enumerate()
                        .filter(|(_, t)| text(t, "return"))
                        .collect();
                    if !returns.is_empty()
                        && returns.iter().all(|(at, _)| {
                            ts.get(at + 1).is_some_and(|t| t.text == local)
                                && ts.get(at + 2).is_none_or(|t| {
                                    t.line > ts[at + 1].line || text(t, ";") || text(t, "}")
                                })
                        })
                    {
                        out.insert(f.name.clone(), Helper::Dial(param, None));
                    }
                }
            }
        }
    }
    out
}
fn key(ts: &[Token], types: &BTreeMap<String, String>) -> Option<String> {
    let ts = if ts.first().is_some_and(|t| text(t, "&")) {
        &ts[1..]
    } else {
        ts
    };
    if ts.len() == 1 && !ts[0].string {
        return Some(ts[0].text.clone());
    }
    if ts.len() == 3 && text(&ts[1], ".") {
        return Some(format!(
            "{}.{}",
            types.get(&ts[0].text).unwrap_or(&ts[0].text),
            ts[2].text
        ));
    }
    None
}
fn value(
    ts: &[Token],
    state: &BTreeMap<String, Value>,
    types: &BTreeMap<String, String>,
    env: &BTreeMap<String, String>,
    hs: &BTreeMap<String, Helper>,
) -> Option<Value> {
    if let Some(k) = key(ts, types) {
        return state.get(&k).cloned();
    }
    if ts.len() == 1 && ts[0].string {
        return deployment::host(&ts[0].text).map(host_value);
    }
    if ts.len() == 6 && text(&ts[0], "os") && text(&ts[2], "Getenv") && ts[4].string {
        return env
            .get(&ts[4].text)
            .and_then(|s| deployment::host(s))
            .map(host_value);
    }
    let open = ts.iter().position(|t| text(t, "("))?;
    let end = close(ts, open, "(", ")")?;
    let name = &ts[open.checked_sub(1)?].text;
    let a = args(ts, open + 1, end);
    if open == 1
        && let Some(Helper::Dial(index, None)) = hs.get(name)
    {
        let mut v = value(a.get(*index)?, state, types, env, hs)?;
        v.connection = true;
        return Some(v);
    }
    if open >= 3
        && text(&ts[open - 3], "grpc")
        && matches!(name.as_str(), "NewClient" | "Dial" | "DialContext")
    {
        let mut v = value(
            a.get(usize::from(name == "DialContext"))?,
            state,
            types,
            env,
            hs,
        )?;
        v.connection = true;
        return Some(v);
    }
    if open >= 3 && name.starts_with("New") && name.ends_with("Client") && a.len() == 1 {
        let mut v = value(a[0], state, types, env, hs)?;
        if !v.connection {
            return None;
        }
        v.grpc = true;
        return Some(v);
    }
    None
}
fn emit(
    report: &mut ServiceTopology,
    d: &Deployment,
    all: &[Deployment],
    file: &str,
    line: u32,
    v: &Value,
    kind: Kind,
) {
    for host in &v.hosts {
        let targets: Vec<_> = all
            .iter()
            .filter(|t| t.scope == d.scope && t.aliases.contains(host))
            .collect();
        if targets.len() == 1 {
            let mut p = deployment::proof(
                file,
                "go-client",
                "Static client call linked through bounded local assignments, verified helper bodies and a declared deployment endpoint; execution and branch reachability are not proven.",
            );
            p.line = Some(line);
            deployment::add(
                report,
                &d.id,
                &targets[0].id,
                kind,
                vec![
                    p,
                    deployment::proof(
                        &d.file,
                        "/environment",
                        "Endpoint binding from this deployment; values omitted.",
                    ),
                ],
            );
        }
    }
}
fn condition_body(ts: &[Token], start: usize) -> Option<usize> {
    let mut depth = 0;
    for (i, t) in ts.iter().enumerate().skip(start) {
        if text(t, "(") || text(t, "[") {
            depth += 1;
        }
        if text(t, ")") || text(t, "]") {
            depth -= 1;
        }
        if depth == 0 && text(t, "{") {
            return Some(i);
        }
    }
    None
}
fn request_is_sent(ts: &[Token], call: usize) -> bool {
    let Some(eq) = (0..call)
        .rev()
        .find(|&j| text(&ts[j], "=") && ts[j].line == ts[call].line)
    else {
        return false;
    };
    let mut left = if eq > 0 && text(&ts[eq - 1], ":") {
        eq - 1
    } else {
        eq
    };
    if left == 0 {
        return false;
    }
    left -= 1;
    if left >= 2 && text(&ts[left - 1], ",") {
        left -= 2;
    }
    let variable = &ts[left].text;
    for i in call + 1..ts.len() {
        if text(&ts[i], "=") && assignment_targets(ts, i).iter().any(|(n, _)| n == variable) {
            return false;
        }
        if text(&ts[i], "&") && ts.get(i + 1).is_some_and(|t| t.text == *variable) {
            return false;
        }
        if text(&ts[i], "Do")
            && ts.get(i + 1).is_some_and(|t| text(t, "("))
            && ts.get(i + 2).is_some_and(|t| t.text == *variable)
            && ts.get(i + 3).is_some_and(|t| text(t, ")"))
        {
            return true;
        }
    }
    false
}
type Package = Vec<(String, Vec<Function>)>;
pub(super) fn augment(root: &Path, report: &mut ServiceTopology, deployments: &[Deployment]) {
    for d in deployments {
        let Some(path) = go_project(report, d) else {
            continue;
        };
        for sources in go_packages(root, &path).values() {
            scan_package(report, d, deployments, sources);
        }
    }
}
fn go_project(report: &ServiceTopology, d: &Deployment) -> Option<String> {
    report
        .services
        .iter()
        .find(|s| s.id == d.id)
        .and_then(|s| s.project.as_ref())
        .and_then(|id| report.projects.iter().find(|p| p.id == *id))
        .filter(|p| p.ecosystem == "go")
        .map(|p| p.path.clone())
}
// Parsed Go sources below the project, grouped by directory (one Go package each).
fn go_packages(root: &Path, project: &str) -> BTreeMap<String, Package> {
    let mut packages = BTreeMap::<String, Package>::new();
    let mut budget = 0;
    for e in ignore::WalkBuilder::new(root.join(project))
        .follow_links(false)
        .max_depth(Some(12))
        .build()
        .take(2000)
        .flatten()
    {
        let name = e.path().to_string_lossy();
        if e.file_type().is_some_and(|t| t.is_file())
            && e.path().extension().and_then(|e| e.to_str()) == Some("go")
            && !name.ends_with("_test.go")
            && !name.ends_with(".pb.go")
            && let Some(s) = deployment::read(root, e.path())
        {
            budget += s.len();
            if budget > 8_000_000 {
                break;
            }
            let file = super::support::relative(root, e.path());
            let directory = Path::new(&file)
                .parent()
                .unwrap_or(Path::new(""))
                .to_string_lossy()
                .into_owned();
            let parsed = functions(&go_tokens(&s));
            packages.entry(directory).or_default().push((file, parsed));
        }
    }
    packages
}
fn scan_package(
    report: &mut ServiceTopology,
    d: &Deployment,
    deployments: &[Deployment],
    sources: &Package,
) {
    let all_functions: Vec<_> = sources
        .iter()
        .flat_map(|(_, fs)| fs.iter().cloned())
        .collect();
    let hs = helpers(&all_functions);
    let mut fields = BTreeMap::new();
    // First establish initialization fields, then inspect method call sites using receiver types.
    for pass in 0..2 {
        let callers = sources
            .iter()
            .flat_map(|(file, fs)| fs.iter().map(move |f| (file, f)))
            .filter(|(_, f)| !hs.contains_key(&f.name));
        for (file, f) in callers {
            let flow = Flow::run(f, &hs, &fields, &d.env, pass == 1);
            for (line, v, kind) in &flow.calls {
                emit(report, d, deployments, file, *line, v, *kind);
            }
            if pass == 0 {
                fields.extend(flow.state.into_iter().filter(|(k, _)| k.contains('.')));
            }
        }
    }
}
fn host_value(host: String) -> Value {
    Value {
        hosts: BTreeSet::from([host]),
        grpc: false,
        connection: false,
    }
}
// Bounded value flow through one function body; `calls` holds the client calls found.
struct Flow<'a> {
    ts: &'a [Token],
    hs: BTreeMap<String, Helper>,
    env: &'a BTreeMap<String, String>,
    state: BTreeMap<String, Value>,
    types: BTreeMap<String, String>,
    joins: Vec<(usize, BTreeMap<String, Value>)>,
    calls: Vec<(u32, Value, Kind)>,
}
impl<'a> Flow<'a> {
    fn run(
        f: &'a Function,
        hs: &BTreeMap<String, Helper>,
        fields: &BTreeMap<String, Value>,
        env: &'a BTreeMap<String, String>,
        find_calls: bool,
    ) -> Self {
        let mut flow = Flow {
            ts: &f.body,
            hs: hs
                .iter()
                .filter(|(name, _)| writes(&f.body, name) == 0)
                .map(|(name, helper)| (name.clone(), helper.clone()))
                .collect(),
            env,
            state: fields.clone(),
            types: f.receiver.iter().cloned().collect(),
            joins: Vec::new(),
            calls: Vec::new(),
        };
        for i in 0..flow.ts.len() {
            flow.merge_join(i);
            flow.open_join(i);
            flow.assign(i);
            flow.call(i, find_calls);
        }
        flow
    }
    fn value_of(&self, ts: &[Token]) -> Option<Value> {
        value(ts, &self.state, &self.types, self.env, &self.hs)
    }
    // Leaving an `if` without `else`: keep the hosts from both branches as static alternatives.
    fn merge_join(&mut self, i: usize) {
        if !self.joins.last().is_some_and(|(end, _)| *end == i) {
            return;
        }
        let (_, before) = self.joins.pop().unwrap();
        let after = std::mem::replace(&mut self.state, before);
        for (key, v) in &mut self.state {
            if let Some(next) = after
                .get(key)
                .filter(|n| n.grpc == v.grpc && n.connection == v.connection)
            {
                v.hosts.extend(next.hosts.iter().cloned());
            }
        }
    }
    fn open_join(&mut self, i: usize) {
        let ts = self.ts;
        if text(&ts[i], "if")
            && let Some(open) = condition_body(ts, i + 1)
            && let Some(end) = close(ts, open, "{", "}")
            && !ts.get(end + 1).is_some_and(|t| text(t, "else"))
        {
            self.joins.push((end, self.state.clone()));
        }
    }
    fn assign(&mut self, i: usize) {
        let ts = self.ts;
        if !text(&ts[i], "=")
            || i == 0
            || ts.get(i + 1).is_some_and(|t| text(t, "="))
            || text(&ts[i - 1], "=")
        {
            return;
        }
        let Some(k) = key(Self::assigned(ts, i), &self.types) else {
            return;
        };
        let rhs = &ts[i + 1..Self::statement_end(ts, i)];
        if rhs.len() >= 4 && text(&rhs[0], "new") && text(&rhs[1], "(") {
            self.types.insert(k.clone(), rhs[2].text.clone());
        }
        if let Some(v) = self.value_of(rhs) {
            self.state.insert(k, v);
        } else {
            self.state.remove(&k);
        }
    }
    // Left side of the assignment at `eq`: a name or `a.b`.
    fn assigned(ts: &[Token], eq: usize) -> &[Token] {
        let left_end = if text(&ts[eq - 1], ":") { eq - 1 } else { eq };
        let mut begin = left_end.saturating_sub(1);
        if begin >= 2 && text(&ts[begin - 1], ".") {
            begin -= 2;
        }
        // Go multi-assignment: first return value is the connection, not err.
        if begin >= 2 && text(&ts[begin - 1], ",") {
            begin -= 2;
        }
        let left_stop = ts[begin..left_end]
            .iter()
            .position(|t| text(t, ","))
            .map_or(left_end, |n| begin + n);
        &ts[begin..left_stop]
    }
    fn statement_end(ts: &[Token], eq: usize) -> usize {
        let mut end = eq + 1;
        let mut depth = 0;
        while end < ts.len() {
            let t = &ts[end];
            if depth == 0 && (text(t, ";") || text(t, "}") || t.line > ts[eq + 1].line) {
                break;
            }
            if text(t, "(") || text(t, "{") {
                depth += 1;
            }
            if text(t, ")") || text(t, "}") {
                depth -= 1;
            }
            end += 1;
        }
        end
    }
    fn call(&mut self, i: usize, find_calls: bool) {
        let ts = self.ts;
        if !ts.get(i + 1).is_some_and(|t| text(t, "(")) {
            return;
        }
        let Some(end) = close(ts, i + 1, "(", ")") else {
            return;
        };
        let a = args(ts, i + 2, end);
        self.apply_helper(i, &a);
        if find_calls {
            self.grpc_call(i);
            self.http_call(i, &a);
        }
    }
    fn apply_helper(&mut self, i: usize, a: &[&[Token]]) {
        let ts = self.ts;
        let helper = self
            .hs
            .get(&ts[i].text)
            .filter(|_| i == 0 || !text(&ts[i - 1], "."));
        match helper.cloned() {
            Some(Helper::Env(p, k)) => {
                if let (Some(target), Some(name)) = (
                    a.get(p).and_then(|a| key(a, &self.types)),
                    a.get(k).filter(|a| a.len() == 1 && a[0].string),
                ) && let Some(host) = self
                    .env
                    .get(&name[0].text)
                    .and_then(|s| deployment::host(s))
                {
                    self.state.insert(target, host_value(host));
                }
            }
            Some(Helper::Dial(p, Some(target))) => {
                if let (Some(k), Some(mut v)) = (
                    a.get(target).and_then(|a| key(a, &self.types)),
                    a.get(p).and_then(|a| self.value_of(a)),
                ) {
                    v.connection = true;
                    self.state.insert(k, v);
                }
            }
            _ => {}
        }
    }
    // Generated client invocation, either inline or via a tracked field/local.
    fn grpc_call(&mut self, i: usize) {
        let ts = self.ts;
        if i < 2 || !text(&ts[i - 1], ".") {
            return;
        }
        let receiver_end = i - 1;
        let begin = Self::receiver_start(ts, receiver_end);
        if let Some(v) = self.value_of(&ts[begin..receiver_end]).filter(|v| v.grpc) {
            self.calls.push((ts[i].line, v, Kind::GrpcCall));
        }
    }
    fn receiver_start(ts: &[Token], receiver_end: usize) -> usize {
        let mut begin = receiver_end - 1;
        if text(&ts[begin], ")") {
            let mut depth = 1;
            while begin > 0 {
                begin -= 1;
                if text(&ts[begin], ")") {
                    depth += 1;
                }
                if text(&ts[begin], "(") {
                    depth -= 1;
                    if depth == 0 {
                        return begin.saturating_sub(3);
                    }
                }
            }
        } else if begin >= 2 && text(&ts[begin - 1], ".") {
            begin -= 2;
        }
        begin
    }
    // A request constructor needs a corresponding Do in this function; construction alone is insufficient.
    fn http_call(&mut self, i: usize, a: &[&[Token]]) {
        let ts = self.ts;
        if !matches!(ts[i].text.as_str(), "NewRequest" | "NewRequestWithContext")
            || i < 2
            || !text(&ts[i - 2], "http")
            || !request_is_sent(ts, i)
        {
            return;
        }
        let index = if text(&ts[i], "NewRequest") { 1 } else { 2 };
        let Some(url) = a.get(index) else {
            return;
        };
        let stop = url.iter().position(|t| text(t, "+")).unwrap_or(url.len());
        if let Some(v) = self.value_of(&url[..stop]) {
            self.calls.push((ts[i].line, v, Kind::HttpCall));
        }
    }
}
