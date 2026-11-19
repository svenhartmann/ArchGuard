//! Blazor extractor for ui-flow/v1 (ADR 0021): a line and tag level scanner
//! over `.razor` files and their `.razor.cs` partials, no Razor parser.
//!
//! Recognized:
//! - `@page "/route"`: one `page` screen per component; several `@page` lines
//!   keep the first as `route`, all of them resolve navigation targets.
//! - `[Authorize]`, `[Authorize(Roles = "A,B")]`, `[Authorize(Policy = "P")]`,
//!   `[Authorize("P")]` (via `@attribute` or on the code-behind class) and
//!   `[AllowAnonymous]`, which drops every condition.
//! - `<a href>` and `<NavLink href>` with a relative or rooted path; Razor
//!   expressions inside a segment (`item/@item.Id`) become parameter segments.
//!   Links with a scheme (`https:`, `mailto:`) or a fragment only are ignored.
//! - `.NavigateTo("x")` and `.NavigateTo($"x/{id}")`, holes becoming parameters.
//! - `<AuthorizeView>` (with `Roles=`/`Policy=`) around a link; inside
//!   `<NotAuthorized>` the condition is negated.
//! - Direct calls on an HTTP client (`Http.GetFromJsonAsync("api/x")`,
//!   `PostAsJsonAsync`, `PutAsync`, `DeleteAsync`, `PatchAsync`, ...) with a
//!   literal or interpolated path.
//!
//! Not recognized: routes from constants or `[Route]`, targets built by helper
//! methods (they stay unresolved), calls through injected services, buttons
//! whose handler navigates (no trigger), commented-out C# code, components of
//! referenced Razor class libraries (targets resolve within one application).
use crate::project_files::read_repo_file;
use crate::services::support::{Discovery, relative, test_path};
use archguard_core::{
    Action, Call, Condition, Screen, ScreenKind, ServiceTopology, Target, Transition, UiFlow,
    UiFlowDiagnostic, path_patterns_match,
};
use std::collections::BTreeSet;
use std::path::Path;
use text::{
    Blocks, attr, balanced, blank, calls, guards, ident_start, line, links, literal, path_of,
    shape, split_args, tag_end,
};

const SOURCE: &str = "blazor";
const MAX_BYTES: u64 = 1 << 20;
const MAX_FILES: usize = 5_000;
const VERBS: [&str; 5] = ["Get", "Post", "Put", "Delete", "Patch"];

/// One `.razor` file with its optional code-behind.
struct Component {
    /// Screen id, or `application:*` for a component without `@page`.
    id: String,
    application: Option<String>,
    /// (root-relative file, text with comments blanked); the `.razor` first.
    texts: Vec<(String, String)>,
    routes: Vec<String>,
}

/// `.razor` files (sorted), directories holding a `.csproj`, `.razor.cs` files.
type Files = (Vec<String>, BTreeSet<String>, BTreeSet<String>);

/// Screens, transitions and actions of every Blazor component below `root`.
///
/// Components without `@page` (layouts, menus) have no screen of their own:
/// their transitions and actions use the pseudo id `application:*`, which
/// means "from every screen of that application".
pub fn extract(root: &Path) -> UiFlow {
    let mut flow = UiFlow::default();
    let Some(files) = discover(root, &mut flow) else {
        return flow;
    };
    let components = load(root, files, &mut flow);
    for c in &components {
        navigation(&mut flow, c, &components);
    }
    // A guard around several links reports its unknown condition once.
    flow.diagnostics.sort();
    flow.diagnostics.dedup();
    flow
}

fn discover(root: &Path, flow: &mut UiFlow) -> Option<Files> {
    let mut scratch = ServiceTopology::default();
    let discovery = match Discovery::new(root, &mut scratch) {
        Ok(discovery) => discovery,
        Err(error) => {
            report(flow, "", 0, format!("Blazor files not scanned: {error}"));
            return None;
        }
    };
    flow.diagnostics
        .extend(scratch.diagnostics.into_iter().map(|d| UiFlowDiagnostic {
            file: d.file,
            line: 0,
            message: d.message,
        }));
    let (mut razor, mut projects, mut code_behind) = Files::default();
    for entry in discovery.walk(root).flatten() {
        let rel = relative(root, entry.path());
        if !entry.file_type().is_some_and(|t| t.is_file()) || test_path(&rel) {
            continue;
        }
        if rel.ends_with(".csproj") {
            projects.insert(rel.rsplit_once('/').map_or("", |(dir, _)| dir).to_string());
        } else if rel.ends_with(".razor") {
            razor.push(rel);
        } else if rel.ends_with(".razor.cs") {
            code_behind.insert(rel);
        }
    }
    razor.sort();
    if razor.len() > MAX_FILES {
        let message = format!("More than {MAX_FILES} .razor files; the rest was not read.");
        report(flow, "", 0, message);
        razor.truncate(MAX_FILES);
    }
    Some((razor, projects, code_behind))
}

fn load(root: &Path, (razor, projects, code_behind): Files, flow: &mut UiFlow) -> Vec<Component> {
    let mut components = Vec::new();
    for rel in razor {
        let Some(text) = read(root, &rel, flow) else {
            continue;
        };
        let mut texts = vec![(rel.clone(), text)];
        let cs = format!("{rel}.cs");
        if code_behind.contains(&cs)
            && let Some(text) = read(root, &cs, flow)
        {
            texts.push((cs, text));
        }
        components.push(component(root, &rel, &projects, texts, flow));
    }
    components
}

/// Links in the markup, `NavigateTo` and HTTP calls in markup and code-behind.
fn navigation(flow: &mut UiFlow, c: &Component, components: &[Component]) {
    for (index, (file, text)) in c.texts.iter().enumerate() {
        let views = guards(text);
        let markup = if index == 0 { links(text) } else { vec![] };
        let navigations = calls(text, ".NavigateTo(")
            .into_iter()
            .filter_map(|(at, arg)| match path_of(&arg) {
                Some(path) if text::external(&path) => None,
                path => Some((at, arg, path, None)),
            });
        for (at, source, path, trigger) in markup.into_iter().chain(navigations) {
            let target = match path {
                Some(path) => resolve(&path, c, components, &source),
                None => Target::Unresolved(source),
            };
            let condition = guard(text, &views, at, file, flow);
            push(
                flow,
                Transition {
                    from: c.id.clone(),
                    target,
                    trigger,
                    file: file.clone(),
                    line: line(text, at),
                    condition,
                    source: SOURCE.into(),
                },
            );
        }
        http(flow, c, file, text, &views);
    }
}

fn read(root: &Path, rel: &str, flow: &mut UiFlow) -> Option<String> {
    match read_repo_file(root, Path::new(rel), MAX_BYTES) {
        Ok(bytes) => {
            let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
            let text = String::from_utf8_lossy(bytes).into_owned();
            Some(blank(&blank(&text, "@*", "*@"), "<!--", "-->"))
        }
        Err(_) => {
            let message = "File not read: larger than 1 MiB, not a regular file or not readable; its screens and navigation are unknown.";
            report(flow, rel, 0, message.into());
            None
        }
    }
}

/// Diagnostic that also marks the flow incomplete.
fn report(flow: &mut UiFlow, file: &str, line: u32, message: String) {
    flow.complete = false;
    flow.diagnostics.push(UiFlowDiagnostic {
        file: file.into(),
        line,
        message,
    });
}

fn component(
    root: &Path,
    rel: &str,
    projects: &BTreeSet<String>,
    texts: Vec<(String, String)>,
    flow: &mut UiFlow,
) -> Component {
    let (application, inner) = owner(root, rel, projects);
    let (file, text) = &texts[0];
    let (routes, first) = routes(file, text, flow);
    let local = if first > 0 { inner } else { "*" };
    let id = match &application {
        Some(app) => format!("{app}:{local}"),
        None => local.to_string(),
    };
    if first > 0 {
        let name = inner.rsplit('/').next().unwrap_or(inner);
        let access = access(&texts, flow);
        flow.screens.push(Screen {
            id: id.clone(),
            name: name.trim_end_matches(".razor").into(),
            kind: ScreenKind::Page,
            route: routes.first().cloned(),
            application: application.clone(),
            file: file.clone(),
            line: first,
            access,
            source: SOURCE.into(),
        });
    }
    Component {
        id,
        application,
        texts,
        routes,
    }
}

/// The application (nearest ancestor directory with a `.csproj`) and the
/// component path relative to it.
fn owner<'a>(root: &Path, rel: &'a str, projects: &BTreeSet<String>) -> (Option<String>, &'a str) {
    let mut dir = rel;
    while let Some((parent, _)) = dir.rsplit_once('/') {
        dir = parent;
        if projects.contains(dir) {
            let name = dir.rsplit('/').next().unwrap_or(dir);
            return (Some(name.into()), &rel[dir.len() + 1..]);
        }
    }
    let name = root.file_name().map(|n| n.to_string_lossy().into_owned());
    (name.filter(|_| projects.contains("")), rel)
}

/// Literal `@page` routes and the line of the first `@page` (0 without one).
fn routes(file: &str, text: &str, flow: &mut UiFlow) -> (Vec<String>, u32) {
    let mut routes = Vec::new();
    let mut first = 0;
    for (n, l) in text.lines().enumerate() {
        let Some(rest) = l
            .trim()
            .strip_prefix("@page")
            .filter(|r| r.starts_with(char::is_whitespace))
        else {
            continue;
        };
        let line = n as u32 + 1;
        if first == 0 {
            first = line;
        }
        match literal(rest.trim()) {
            Some(route) => routes.push(route),
            None => {
                let message = format!(
                    "@page route {} is not a literal; it is no route of the screen.",
                    rest.trim()
                );
                report(flow, file, line, message);
            }
        }
    }
    (routes, first)
}

/// Page access from `[Authorize]`/`[AllowAnonymous]` in the markup or code-behind.
fn access(texts: &[(String, String)], flow: &mut UiFlow) -> Option<Condition> {
    let mut found = Vec::new();
    for (file, text) in texts {
        for (n, l) in text.lines().enumerate() {
            let l = l.trim();
            let attribute = l.strip_prefix("@attribute").map(str::trim).unwrap_or(l);
            if !attribute.starts_with('[') {
                continue;
            }
            if attribute.contains("AllowAnonymous") {
                return None;
            }
            let Some(at) = attribute.find("Authorize") else {
                continue;
            };
            let rest = &attribute[at + "Authorize".len()..];
            if rest.starts_with(|c: char| c.is_alphanumeric()) {
                continue;
            }
            let args = rest
                .strip_prefix('(')
                .map(|r| balanced(r, ')'))
                .unwrap_or("");
            found.push(authorize(args, file, n as u32 + 1, flow));
        }
    }
    all(found)
}

/// Condition of one `[Authorize(...)]` argument list.
fn authorize(args: &str, file: &str, line: u32, flow: &mut UiFlow) -> Condition {
    let mut parts = Vec::new();
    for part in split_args(args) {
        let (key, value) = match part.split_once('=') {
            Some((k, v)) if !k.contains('"') => (k.trim(), v.trim()),
            _ => ("Policy", part.as_str()),
        };
        match (key, literal(value)) {
            ("AuthenticationSchemes", _) => {}
            ("Roles", Some(roles)) => parts.push(roles_condition(&roles)),
            ("Policy", Some(policy)) => parts.push(Condition::Policy(policy)),
            _ => parts.push(unknown(&part, file, line, flow)),
        }
    }
    all(parts).unwrap_or(Condition::Authenticated)
}

fn unknown(text: &str, file: &str, line: u32, flow: &mut UiFlow) -> Condition {
    let message = format!("Authorization {text} is not a literal; the condition is unknown.");
    report(flow, file, line, message);
    Condition::Unknown(text.into())
}

fn roles_condition(roles: &str) -> Condition {
    let mut roles: Vec<Condition> = roles
        .split(',')
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(|r| Condition::Role(r.into()))
        .collect();
    if roles.len() == 1 {
        roles.remove(0)
    } else {
        Condition::Any(roles)
    }
}

fn all(mut items: Vec<Condition>) -> Option<Condition> {
    match items.len() {
        0 => None,
        1 => items.pop(),
        _ => Some(Condition::All(items)),
    }
}

fn push(flow: &mut UiFlow, transition: Transition) {
    if let Target::Unresolved(text) = &transition.target {
        let message = format!("Navigation target {text} is not a route of this application.");
        report(flow, &transition.file, transition.line, message);
    }
    flow.transitions.push(transition);
}

/// The first screen of the same application with a matching route.
fn resolve(path: &str, from: &Component, components: &[Component], text: &str) -> Target {
    components
        .iter()
        .filter(|c| c.application == from.application)
        .find(|c| c.routes.iter().any(|r| path_patterns_match(r, path)))
        .map_or_else(
            || Target::Unresolved(text.into()),
            |c| Target::Screen(c.id.clone()),
        )
}

/// Actions for direct HTTP client calls with a literal path.
fn http(flow: &mut UiFlow, c: &Component, file: &str, text: &str, views: &Blocks) {
    for (at, _) in text.match_indices("Async") {
        let before = &text[..at];
        let start = ident_start(before);
        let Some(verb) = VERBS.iter().find(|v| before[start..].starts_with(*v)) else {
            continue;
        };
        let Some(object) = before[..start].strip_suffix('.') else {
            continue;
        };
        let receiver = &object[ident_start(object)..];
        if !receiver.to_ascii_lowercase().contains("http") {
            continue;
        }
        let mut rest = &text[at + "Async".len()..];
        if rest.starts_with('<') {
            rest = rest
                .get(balanced(&rest[1..], '>').len() + 2..)
                .unwrap_or("");
        }
        let Some(args) = rest.strip_prefix('(') else {
            continue;
        };
        let arg = split_args(balanced(args, ')'))
            .into_iter()
            .next()
            .unwrap_or_default();
        let line = line(text, at);
        let Some(path) = path_of(&arg) else {
            let message = format!("HTTP call path {arg} is not a literal; no action recorded.");
            flow.diagnostics.push(UiFlowDiagnostic {
                file: file.into(),
                line,
                message,
            });
            continue;
        };
        let path = match path.split_once("://") {
            Some((_, host_path)) => shape(host_path.split_once('/').map_or("", |(_, p)| p)),
            None => path,
        };
        let condition = guard(text, views, at, file, flow);
        flow.actions.push(Action {
            screen: c.id.clone(),
            trigger: None,
            call: Call::Http {
                method: verb.to_uppercase(),
                path,
            },
            file: file.into(),
            line,
            condition,
            source: SOURCE.into(),
        });
    }
}

/// Condition of the `<AuthorizeView>` blocks around `at`; inside a block's
/// `<NotAuthorized>` its condition is negated.
fn guard(
    text: &str,
    (views, denied): &Blocks,
    at: usize,
    file: &str,
    flow: &mut UiFlow,
) -> Option<Condition> {
    let around: Vec<&(usize, usize)> = views.iter().filter(|(s, e)| *s < at && at < *e).collect();
    let mut found = Vec::new();
    for &&(start, end) in &around {
        let condition = view_condition(text, start, file, flow);
        // A <NotAuthorized> belongs to this view when no inner view around `at` opens first.
        let negated = denied.iter().any(|&(ds, de)| {
            ds > start
                && de < end
                && ds < at
                && at < de
                && !around.iter().any(|&&(s, _)| s > start && s < ds)
        });
        found.push(if negated {
            Condition::Not(Box::new(condition))
        } else {
            condition
        });
    }
    all(found)
}

/// Condition of the `<AuthorizeView>` tag at `start`.
fn view_condition(text: &str, start: usize, file: &str, flow: &mut UiFlow) -> Condition {
    let attrs = &text[start..];
    let attrs = &attrs[..tag_end(attrs).unwrap_or(0)];
    let line = line(text, start);
    let mut parts = Vec::new();
    for name in ["Roles", "Policy"] {
        let Some(value) = attr(attrs, name) else {
            continue;
        };
        parts.push(match (value.starts_with('@'), name) {
            (true, _) => unknown(&format!("{name}=\"{value}\""), file, line, flow),
            (false, "Roles") => roles_condition(value),
            (false, _) => Condition::Policy(value.into()),
        });
    }
    all(parts).unwrap_or(Condition::Authenticated)
}

/// Markup and C# text scanning without knowledge of the model.
mod text {
    /// `<AuthorizeView>` and `<NotAuthorized>` blocks of one text.
    pub(super) type Blocks = (Vec<(usize, usize)>, Vec<(usize, usize)>);

    /// One navigation found in text: (offset, source text, route shape or
    /// `None` when not literal, trigger label).
    pub(super) type Found = (usize, String, Option<String>, Option<String>);

    pub(super) fn guards(text: &str) -> Blocks {
        (blocks(text, "AuthorizeView"), blocks(text, "NotAuthorized"))
    }

    /// (open, close) offsets of every non-self-closing `<name>` ... `</name>` block.
    fn blocks(text: &str, name: &str) -> Vec<(usize, usize)> {
        let open = format!("<{name}");
        let close = format!("</{name}>");
        let mut events: Vec<(usize, bool)> = text
            .match_indices(&open)
            .filter(|(at, _)| {
                let after = &text[at + open.len()..];
                after.starts_with(|c: char| c.is_whitespace() || c == '>')
                    && !after[..tag_end(after).unwrap_or(0)]
                        .trim_end()
                        .ends_with('/')
            })
            .map(|(at, _)| (at, true))
            .chain(text.match_indices(&close).map(|(at, _)| (at, false)))
            .collect();
        events.sort();
        let mut stack = Vec::new();
        let mut found = Vec::new();
        for (at, opening) in events {
            if opening {
                stack.push(at);
            } else if let Some(start) = stack.pop() {
                found.push((start, at));
            }
        }
        found
    }

    /// `<a>` and `<NavLink>` tags with an internal `href`, in text order.
    pub(super) fn links(text: &str) -> Vec<Found> {
        let mut found: Vec<Found> = ["<a", "<NavLink"]
            .iter()
            .flat_map(|tag| {
                text.match_indices(tag)
                    .filter_map(|(at, _)| link(&text[at + tag.len()..], &tag[1..]).map(|l| (at, l)))
            })
            .map(|(at, (href, path, trigger))| (at, href, path, trigger))
            .collect();
        found.sort_by_key(|(at, ..)| *at);
        found
    }

    /// The link after the tag name `tag`: (href, route shape, trigger).
    fn link(after: &str, tag: &str) -> Option<(String, Option<String>, Option<String>)> {
        if !after.starts_with(|c: char| c.is_whitespace() || c == '>') {
            return None;
        }
        let end = tag_end(after)?;
        let attrs = &after[..end];
        let href = attr(attrs, "href")?.trim();
        if href.starts_with('#') || external(href) {
            return None;
        }
        let path = (!href.starts_with('@')).then(|| shape(href));
        let inner = if attrs.trim_end().ends_with('/') {
            ""
        } else {
            let body = &after[end + 1..];
            &body[..body.find(&format!("</{tag}>")).unwrap_or(0)]
        };
        // A Razor expression is no label.
        let readable = |l: &String| !l.is_empty() && !l.starts_with('@');
        let trigger = Some(words(&strip_tags(inner)))
            .filter(readable)
            .or_else(|| attr(attrs, "aria-label").map(words).filter(readable));
        Some((href.into(), path, trigger))
    }

    /// A URL with a scheme (`https:`, `mailto:`) or protocol-relative.
    pub(super) fn external(href: &str) -> bool {
        let head = href.split(['/', '?', '#']).next().unwrap_or("");
        head.contains(':') || href.starts_with("//")
    }

    /// Offset of the `>` closing a tag, skipping quoted attribute values.
    pub(super) fn tag_end(s: &str) -> Option<usize> {
        let mut quote = None;
        for (i, c) in s.char_indices() {
            match (quote, c) {
                (None, '"' | '\'') => quote = Some(c),
                (Some(q), _) if q == c => quote = None,
                (None, '>') => return Some(i),
                _ => {}
            }
        }
        None
    }

    pub(super) fn attr<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
        for (at, _) in attrs.match_indices(name) {
            if !attrs[..at].ends_with(char::is_whitespace) {
                continue;
            }
            let rest = attrs[at + name.len()..].trim_start();
            let Some(rest) = rest.strip_prefix('=') else {
                continue;
            };
            let rest = rest.trim_start();
            return Some(match rest.chars().next() {
                Some(q @ ('"' | '\'')) => rest[1..].split(q).next().unwrap_or(""),
                _ => rest.split(char::is_whitespace).next().unwrap_or(""),
            });
        }
        None
    }

    fn strip_tags(s: &str) -> String {
        let mut out = String::new();
        let mut depth = 0;
        for c in s.chars() {
            match c {
                '<' => depth += 1,
                '>' if depth > 0 => depth -= 1,
                _ if depth == 0 => out.push(c),
                _ => {}
            }
        }
        out
    }

    fn words(s: &str) -> String {
        s.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// Route shape of a literal path: query and fragment dropped, leading `/`,
    /// segments with a Razor expression or interpolation hole become `{p}`.
    pub(super) fn shape(path: &str) -> String {
        let path = path.split(['?', '#']).next().unwrap_or("");
        let segments: Vec<&str> = path
            .split('/')
            .filter(|s| !s.is_empty())
            .map(|s| if s.contains(['@', '{']) { "{p}" } else { s })
            .collect();
        format!("/{}", segments.join("/"))
    }

    /// Path of a C# string argument: `"x"` or `$"x/{id}"`; `None` otherwise.
    /// Absolute URLs stay as written so [`external`] can tell them.
    pub(super) fn path_of(arg: &str) -> Option<String> {
        let path = match arg.strip_prefix('$') {
            Some(body) => holes(&literal(body)?),
            None => literal(arg)?,
        };
        Some(if external(&path) { path } else { shape(&path) })
    }

    /// Interpolation holes replaced by `{p}`; holes may hold `/` or `?`.
    fn holes(body: &str) -> String {
        let mut out = String::new();
        let mut depth = 0;
        for c in body.chars() {
            match c {
                '{' if depth == 0 => {
                    out.push_str("{p}");
                    depth = 1;
                }
                '{' => depth += 1,
                '}' if depth > 0 => depth -= 1,
                _ if depth == 0 => out.push(c),
                _ => {}
            }
        }
        out
    }

    /// Content of a plain `"..."` literal spanning the whole input.
    pub(super) fn literal(s: &str) -> Option<String> {
        let body = s.strip_prefix('"')?.strip_suffix('"')?;
        (!body.contains('"')).then(|| body.into())
    }

    /// Every `needle(` call: (offset, first argument text).
    pub(super) fn calls(text: &str, needle: &str) -> Vec<(usize, String)> {
        text.match_indices(needle)
            .map(|(at, _)| {
                let args = balanced(&text[at + needle.len()..], ')');
                (at, split_args(args).into_iter().next().unwrap_or_default())
            })
            .collect()
    }

    /// Prefix of `s` up to the unquoted, unnested `close`.
    pub(super) fn balanced(s: &str, close: char) -> &str {
        let mut depth = 0;
        let mut quote = false;
        let mut prev = ' ';
        for (i, c) in s.char_indices() {
            match c {
                '"' if prev != '\\' => quote = !quote,
                _ if quote => {}
                '(' | '[' | '{' => depth += 1,
                c2 if c2 == close && depth == 0 => return &s[..i],
                ')' | ']' | '}' => depth -= 1,
                _ => {}
            }
            prev = c;
        }
        s
    }

    /// Top-level comma separated arguments, trimmed.
    pub(super) fn split_args(args: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut rest = args;
        while !rest.trim().is_empty() {
            let part = balanced(rest, ',');
            out.push(part.trim().to_string());
            rest = rest.get(part.len() + 1..).unwrap_or("");
        }
        out
    }

    /// Offset where the identifier ending `s` starts.
    pub(super) fn ident_start(s: &str) -> usize {
        s.char_indices()
            .rev()
            .take_while(|(_, c)| c.is_alphanumeric() || *c == '_')
            .last()
            .map_or(s.len(), |(i, _)| i)
    }

    pub(super) fn line(text: &str, at: usize) -> u32 {
        text[..at].matches('\n').count() as u32 + 1
    }

    /// Replaces `open`...`close` regions with spaces, keeping line breaks.
    pub(super) fn blank(text: &str, open: &str, close: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        while let Some(start) = rest.find(open) {
            out.push_str(&rest[..start]);
            let end = rest[start..]
                .find(close)
                .map_or(rest.len(), |e| start + e + close.len());
            out.extend(
                rest[start..end]
                    .chars()
                    .map(|c| if c == '\n' { '\n' } else { ' ' }),
            );
            rest = &rest[end..];
        }
        out.push_str(rest);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{Call, Condition, Screen, Target, Transition};

    const HOME: &str = r##"@page "/"
@page "/home"
<h1>Home</h1>
<NavLink href="cart">Your <b>cart</b></NavLink>
<a href="https://example.com/x">External</a>
<a href="mailto:a@b.c">Mail</a>
<a href="#top">Top</a>
<a href="item/@item.Id">Item</a>
@* <a href="ghost">commented out</a> *@
"##;

    const ITEM: &str = r##"@page "/item/{id:int}"
@attribute [Authorize(Roles = "Admin, Buyer")]
@inject NavigationManager Nav
@inject HttpClient Http
<button @onclick="Go">Go</button>
@code {
    [Parameter] public int Id { get; set; }
    async Task Go() {
        var items = await Http.GetFromJsonAsync<Item[]>("api/items?page=1");
        await Http.PostAsJsonAsync($"api/items/{Id}", items);
        Nav.NavigateTo($"item/{Id + 1}");
        NavigationManager.NavigateTo("/cart");
        Nav.NavigateTo(returnUrl);
    }
}
"##;

    // Starts with a UTF-8 byte order mark, as Visual Studio writes it.
    const CART: &str = concat!(
        "\u{feff}",
        r##"@page "/cart"
@attribute [Authorize]
<a href="nowhere">Lost</a>
<a href="/" aria-label="@Label">@Count</a>
"##
    );

    const CART_CS: &str = r##"public partial class CartPage {
    void Done() => Nav.NavigateTo("/", forceLoad: true);
}
"##;

    const ADMIN: &str = "@page \"/admin\"\n@attribute [Authorize(Policy = \"AdminOnly\")]\n";
    const LOGIN: &str = "@page \"/login\"\n@attribute [AllowAnonymous]\n";
    const WEIRD: &str = "@page \"/weird\"\n@attribute [Authorize(Roles = RoleNames.Admin)]\n";

    const MENU: &str = r##"<AuthorizeView>
    <Authorized>
        <a href="cart">My cart</a>
    </Authorized>
    <NotAuthorized>
        <NavLink href="login" aria-label="Sign in"><img src="x.svg" /></NavLink>
    </NotAuthorized>
</AuthorizeView>
<AuthorizeView Roles="Admin">
    <a href="admin">Admin</a>
</AuthorizeView>
<a href="@LoginUrl()">Dynamic</a>
"##;

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let files: &[(&str, &str)] = &[
            ("Shop/Shop.csproj", "<Project />"),
            ("Shop/Pages/Home.razor", HOME),
            ("Shop/Pages/Item.razor", ITEM),
            ("Shop/Pages/Cart.razor", CART),
            ("Shop/Pages/Cart.razor.cs", CART_CS),
            ("Shop/Pages/Admin.razor", ADMIN),
            ("Shop/Pages/Login.razor", LOGIN),
            ("Shop/Pages/Weird.razor", WEIRD),
            ("Shop/Layout/UserMenu.razor", MENU),
            ("Shop/bin/Debug/Old.razor", "@page \"/old\"\n"),
            ("Shop/obj/Gen.razor", "@page \"/gen\"\n"),
            ("Shop.Tests/Shop.Tests.csproj", "<Project />"),
            ("Shop.Tests/Fake.razor", "@page \"/fake\"\n"),
            ("Other/Other.csproj", "<Project />"),
            ("Other/Pages/Cart.razor", "@page \"/cart\"\n"),
        ];
        for (path, text) in files {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        dir
    }

    fn screen<'a>(flow: &'a UiFlow, id: &str) -> &'a Screen {
        flow.screens
            .iter()
            .find(|s| s.id == id)
            .unwrap_or_else(|| panic!("no screen {id}: {:#?}", flow.screens))
    }

    fn from<'a>(flow: &'a UiFlow, id: &str) -> Vec<&'a Transition> {
        flow.transitions.iter().filter(|t| t.from == id).collect()
    }

    fn to(id: &str) -> Target {
        Target::Screen(id.into())
    }

    #[test]
    fn pages_become_screens_with_application_and_first_route() {
        let flow = extract(fixture().path());
        let home = screen(&flow, "Shop:Pages/Home.razor");
        assert_eq!(home.name, "Home");
        assert_eq!(home.route.as_deref(), Some("/"));
        assert_eq!(home.application.as_deref(), Some("Shop"));
        assert_eq!(home.file, "Shop/Pages/Home.razor");
        assert_eq!(home.line, 1);
        assert_eq!(home.source, "blazor");
        let item = screen(&flow, "Shop:Pages/Item.razor");
        assert_eq!(item.route.as_deref(), Some("/item/{id:int}"));
        assert_eq!(
            screen(&flow, "Other:Pages/Cart.razor")
                .application
                .as_deref(),
            Some("Other")
        );
        // bin, obj, test projects and non-page components are no screens.
        let mut ids: Vec<&str> = flow.screens.iter().map(|s| s.id.as_str()).collect();
        ids.sort();
        assert_eq!(
            ids,
            [
                "Other:Pages/Cart.razor",
                "Shop:Pages/Admin.razor",
                "Shop:Pages/Cart.razor",
                "Shop:Pages/Home.razor",
                "Shop:Pages/Item.razor",
                "Shop:Pages/Login.razor",
                "Shop:Pages/Weird.razor",
            ]
        );
    }

    #[test]
    fn authorize_forms_become_access_conditions() {
        let flow = extract(fixture().path());
        let access = |id: &str| screen(&flow, id).access.clone();
        assert_eq!(
            access("Shop:Pages/Cart.razor"),
            Some(Condition::Authenticated)
        );
        assert_eq!(
            access("Shop:Pages/Item.razor"),
            Some(Condition::Any(vec![
                Condition::Role("Admin".into()),
                Condition::Role("Buyer".into())
            ]))
        );
        assert_eq!(
            access("Shop:Pages/Admin.razor"),
            Some(Condition::Policy("AdminOnly".into()))
        );
        assert_eq!(access("Shop:Pages/Login.razor"), None);
        assert_eq!(access("Shop:Pages/Home.razor"), None);
        assert_eq!(
            access("Shop:Pages/Weird.razor"),
            Some(Condition::Unknown("Roles = RoleNames.Admin".into()))
        );
        assert!(
            flow.diagnostics
                .iter()
                .any(|d| d.file == "Shop/Pages/Weird.razor" && d.line == 2)
        );
        assert!(!flow.complete);
    }

    #[test]
    fn links_resolve_within_the_application_and_external_ones_are_ignored() {
        let flow = extract(fixture().path());
        let home = from(&flow, "Shop:Pages/Home.razor");
        let targets: Vec<(&Target, Option<&str>)> = home
            .iter()
            .map(|t| (&t.target, t.trigger.as_deref()))
            .collect();
        assert_eq!(
            targets,
            [
                (&to("Shop:Pages/Cart.razor"), Some("Your cart")),
                (&to("Shop:Pages/Item.razor"), Some("Item")),
            ]
        );
        assert_eq!(home[0].line, 4);
        assert_eq!(home[0].source, "blazor");
        assert_eq!(home[0].condition, None);
    }

    #[test]
    fn navigate_to_resolves_literals_and_interpolations() {
        let flow = extract(fixture().path());
        let item = from(&flow, "Shop:Pages/Item.razor");
        let targets: Vec<&Target> = item.iter().map(|t| &t.target).collect();
        assert_eq!(
            targets,
            [
                &to("Shop:Pages/Item.razor"),
                &to("Shop:Pages/Cart.razor"),
                &Target::Unresolved("returnUrl".into()),
            ]
        );
        assert!(
            flow.diagnostics
                .iter()
                .any(|d| d.file == "Shop/Pages/Item.razor" && d.line == 13)
        );
        // A code-behind partial belongs to its component.
        let cart = from(&flow, "Shop:Pages/Cart.razor");
        let done = cart
            .iter()
            .find(|t| t.file == "Shop/Pages/Cart.razor.cs")
            .unwrap();
        assert_eq!(done.target, to("Shop:Pages/Home.razor"));
        assert_eq!(done.line, 2);
    }

    #[test]
    fn unresolved_link_target_is_reported() {
        let flow = extract(fixture().path());
        let lost = from(&flow, "Shop:Pages/Cart.razor")
            .into_iter()
            .find(|t| t.file == "Shop/Pages/Cart.razor")
            .unwrap();
        assert_eq!(lost.target, Target::Unresolved("nowhere".into()));
        assert_eq!(lost.trigger.as_deref(), Some("Lost"));
        // Razor expressions are no labels.
        let home = flow
            .transitions
            .iter()
            .find(|t| t.file == "Shop/Pages/Cart.razor" && t.line == 4)
            .unwrap();
        assert_eq!(
            (&home.target, &home.trigger),
            (&to("Shop:Pages/Home.razor"), &None)
        );
        assert!(
            flow.diagnostics
                .iter()
                .any(|d| d.file == "Shop/Pages/Cart.razor" && d.line == 3)
        );
        assert!(!flow.complete);
    }

    #[test]
    fn layout_links_start_from_every_screen_with_authorize_view_conditions() {
        let flow = extract(fixture().path());
        let menu = from(&flow, "Shop:*");
        let found: Vec<(&Target, Option<&Condition>, Option<&str>)> = menu
            .iter()
            .map(|t| (&t.target, t.condition.as_ref(), t.trigger.as_deref()))
            .collect();
        assert_eq!(
            found,
            [
                (
                    &to("Shop:Pages/Cart.razor"),
                    Some(&Condition::Authenticated),
                    Some("My cart")
                ),
                (
                    &to("Shop:Pages/Login.razor"),
                    Some(&Condition::Not(Box::new(Condition::Authenticated))),
                    Some("Sign in")
                ),
                (
                    &to("Shop:Pages/Admin.razor"),
                    Some(&Condition::Role("Admin".into())),
                    Some("Admin")
                ),
                (
                    &Target::Unresolved("@LoginUrl()".into()),
                    None,
                    Some("Dynamic")
                ),
            ]
        );
        assert!(menu.iter().all(|t| t.file == "Shop/Layout/UserMenu.razor"));
        // The other application's /cart is never a target here.
        assert!(
            flow.transitions
                .iter()
                .all(|t| t.target != to("Other:Pages/Cart.razor"))
        );
    }

    #[test]
    fn direct_http_calls_with_literal_paths_become_actions() {
        let flow = extract(fixture().path());
        let calls: Vec<(&str, &Call, u32)> = flow
            .actions
            .iter()
            .map(|a| (a.screen.as_str(), &a.call, a.line))
            .collect();
        let http = |method: &str, path: &str| Call::Http {
            method: method.into(),
            path: path.into(),
        };
        assert_eq!(
            calls,
            [
                ("Shop:Pages/Item.razor", &http("GET", "/api/items"), 9),
                ("Shop:Pages/Item.razor", &http("POST", "/api/items/{p}"), 10),
            ]
        );
        assert!(flow.actions.iter().all(|a| a.source == "blazor"));
    }

    #[test]
    fn complete_when_everything_is_literal() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("App.csproj"), "").unwrap();
        std::fs::write(
            dir.path().join("A.razor"),
            "@page \"/a\"\n<a href=\"bee\">B</a>\n",
        )
        .unwrap();
        // A second @page route resolves too.
        std::fs::write(dir.path().join("B.razor"), "@page \"/b\"\n@page \"/bee\"\n").unwrap();
        let flow = extract(dir.path());
        let app = dir
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(flow.complete, "{:#?}", flow.diagnostics);
        assert_eq!(flow.transitions[0].target, to(&format!("{app}:B.razor")));
    }
}
