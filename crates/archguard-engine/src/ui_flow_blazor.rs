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
//!   literal or interpolated path. The receiver counts when it is injected as
//!   `HttpClient`; a receiver without a visible declaration (a field of the
//!   `@code` block) counts when its name contains `http`.
//! - Calls through injected services, see [`services`]: `@inject T Name`,
//!   `[Inject] T Name { get; set; }`, then `Name.Method(...)` is followed into
//!   the C# class `T` down to `HttpClient` calls.
//!
//! - A component without `@page` (layout, menu) acts on every screen of its
//!   application: its transitions and actions start at `any_screen`.
//! - A literal target matches parameter routes (`item/42` -> `/item/{id:int}`);
//!   an exact literal route wins, and a tie of equally good routes stays
//!   unresolved. An HTTP call with a computed path is a `Call::Unresolved` action.
//!
//! Not recognized: routes from constants or `[Route]`, targets built by helper
//! methods (they stay unresolved), buttons whose handler navigates or calls
//! (no trigger), commented-out C# code in `.razor` files, `@inject` in
//! `_Imports.razor`, components of referenced Razor class libraries (targets
//! resolve within one application).
use crate::project_files::read_repo_file;
use crate::services::support::{Discovery, relative, test_path};
use archguard_core::{
    Action, Call, Condition, DiagnosticSeverity, Origin, Screen, ScreenKind, ServiceTopology,
    Target, Transition, UiFlow, UiFlowDiagnostic, best_route,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use text::{
    Blocks, attr, balanced, blank, calls, guards, line, links, literal, path_of, split_args,
    tag_end,
};

const SOURCE: &str = "blazor";
const MAX_BYTES: u64 = 1 << 20;
const MAX_FILES: usize = 5_000;
const VERBS: [&str; 5] = ["Get", "Post", "Put", "Delete", "Patch"];

/// One `.razor` file with its optional code-behind.
struct Component {
    /// The component's screen, or every screen of its application without `@page`.
    origin: Origin,
    application: Option<String>,
    /// Directory of the owning `.csproj` (`""` for the root or none).
    project: String,
    /// (root-relative file, text with comments blanked); the `.razor` first.
    texts: Vec<(String, String)>,
    routes: Vec<String>,
    /// Injected member name to its declared type.
    injected: BTreeMap<String, String>,
}

/// Project directory to its `.csproj` file.
type Projects = BTreeMap<String, String>;

#[derive(Default)]
struct Files {
    /// `.razor` files, sorted.
    razor: Vec<String>,
    projects: Projects,
    code_behind: BTreeSet<String>,
    /// Other `.cs` files: the service classes.
    cs: Vec<String>,
}

/// Screens, transitions and actions of every Blazor component below `root`.
///
/// Components without `@page` (layouts, menus) have no screen of their own:
/// their transitions and actions start at `any_screen` of their application.
pub fn extract(root: &Path) -> UiFlow {
    let mut flow = UiFlow::default();
    let Some(files) = discover(root, &mut flow) else {
        return flow;
    };
    let components = load(root, &files, &mut flow);
    let index = services::Index::build(root, &files, &components, &mut flow);
    let mut grpc = BTreeSet::new();
    for c in &components {
        navigation(&mut flow, c, &components);
        services::actions(&index, c, &mut flow, &mut grpc);
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
            severity: DiagnosticSeverity::Note,
        }));
    let mut files = Files::default();
    for entry in discovery.walk(root).flatten() {
        let rel = relative(root, entry.path());
        if !entry.file_type().is_some_and(|t| t.is_file()) || test_path(&rel) {
            continue;
        }
        if rel.ends_with(".csproj") {
            let dir = rel.rsplit_once('/').map_or("", |(dir, _)| dir).to_string();
            files.projects.entry(dir).or_insert(rel);
        } else if rel.ends_with(".razor") {
            files.razor.push(rel);
        } else if rel.ends_with(".razor.cs") {
            files.code_behind.insert(rel);
        } else if rel.ends_with(".cs") {
            files.cs.push(rel);
        }
    }
    files.razor.sort();
    files.cs.sort();
    if files.razor.len() > MAX_FILES {
        let message = format!("More than {MAX_FILES} .razor files; the rest was not read.");
        report(flow, "", 0, message);
        files.razor.truncate(MAX_FILES);
    }
    Some(files)
}

fn load(root: &Path, files: &Files, flow: &mut UiFlow) -> Vec<Component> {
    let razor = |text: String| blank(&blank(&text, "@*", "*@"), "<!--", "-->");
    let mut components = Vec::new();
    for rel in &files.razor {
        let Some(text) = read(root, rel, flow) else {
            continue;
        };
        let mut texts = vec![(rel.clone(), razor(text))];
        let cs = format!("{rel}.cs");
        if files.code_behind.contains(&cs)
            && let Some(text) = read(root, &cs, flow)
        {
            texts.push((cs, razor(text)));
        }
        components.push(component(root, rel, &files.projects, texts, flow));
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
            let line = line(text, at);
            let target = match resolve(path.as_deref(), c, components, &source) {
                Ok(id) => Target::Screen(id),
                Err(message) => {
                    report(flow, file, line, message);
                    Target::Unresolved(source)
                }
            };
            let condition = guard(text, &views, at, file, flow);
            flow.transitions.push(Transition {
                from: c.origin.clone(),
                target,
                trigger,
                file: file.clone(),
                line,
                condition,
                source: SOURCE.into(),
            });
        }
        http(flow, c, file, text, &views);
    }
}

fn read(root: &Path, rel: &str, flow: &mut UiFlow) -> Option<String> {
    match read_repo_file(root, Path::new(rel), MAX_BYTES) {
        Ok(bytes) => {
            let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
            Some(String::from_utf8_lossy(bytes).into_owned())
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
        severity: DiagnosticSeverity::Gap,
    });
}

fn component(
    root: &Path,
    rel: &str,
    projects: &Projects,
    texts: Vec<(String, String)>,
    flow: &mut UiFlow,
) -> Component {
    let (application, inner) = owner(root, rel, projects);
    let injected = services::injections(&texts);
    let (file, text) = &texts[0];
    let (routes, first) = routes(file, text, flow);
    let id = match &application {
        Some(app) => format!("{app}:{inner}"),
        None => inner.to_string(),
    };
    let origin = if first > 0 {
        Origin::Screen(id.clone())
    } else {
        Origin::AnyScreen {
            application: application.clone().unwrap_or_default(),
        }
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
        origin,
        application,
        project: project_of(rel, projects).unwrap_or_default().into(),
        texts,
        routes,
        injected,
    }
}

/// The application (nearest ancestor directory with a `.csproj`) and the
/// component path relative to it.
fn owner<'a>(root: &Path, rel: &'a str, projects: &Projects) -> (Option<String>, &'a str) {
    match project_of(rel, projects) {
        Some("") => {
            let name = root.file_name().map(|n| n.to_string_lossy().into_owned());
            (name, rel)
        }
        Some(dir) => (
            Some(dir.rsplit('/').next().unwrap_or(dir).into()),
            &rel[dir.len() + 1..],
        ),
        None => (None, rel),
    }
}

/// Directory of the nearest ancestor `.csproj` of `rel` (`""` for the root).
fn project_of<'a>(rel: &str, projects: &'a Projects) -> Option<&'a str> {
    let mut dir = rel;
    while let Some((parent, _)) = dir.rsplit_once('/') {
        dir = parent;
        if let Some((found, _)) = projects.get_key_value(dir) {
            return Some(found);
        }
    }
    projects.get_key_value("").map(|(found, _)| found.as_str())
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

/// The screen of the same application whose route matches best, or the
/// diagnostic why there is none.
fn resolve(
    path: Option<&str>,
    from: &Component,
    components: &[Component],
    text: &str,
) -> Result<String, String> {
    let unresolved = || format!("Navigation target {text} is not a route of this application.");
    let candidates = components
        .iter()
        .filter(|c| c.application == from.application)
        .filter_map(|c| match &c.origin {
            Origin::Screen(id) => Some((id, &c.routes)),
            Origin::AnyScreen { .. } => None,
        })
        .flat_map(|(id, routes)| routes.iter().map(move |r| (id, r.as_str())));
    match best_route(path.ok_or_else(unresolved)?, candidates) {
        Ok(id) => Ok(id.clone()),
        Err(tied) if tied.is_empty() => Err(unresolved()),
        Err(tied) => Err(format!(
            "Navigation target {text} matches several routes equally: {}.",
            tied.iter()
                .map(|id| id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

/// Actions for direct HTTP client calls with a literal path.
fn http(flow: &mut UiFlow, c: &Component, file: &str, text: &str, views: &Blocks) {
    for (at, receiver, verb, arg) in csharp::http_calls(text) {
        // shortcut: without a visible declaration the name decides, until `@code` fields are read.
        let client = match c.injected.get(receiver) {
            Some(ty) => services::is_http_client(ty),
            None => receiver.to_ascii_lowercase().contains("http"),
        };
        if !client {
            continue;
        }
        let line = line(text, at);
        let call = match path_of(&arg) {
            Some(path) => Call::Http {
                method: verb.to_uppercase(),
                path: csharp::endpoint(&path),
            },
            None => {
                let message =
                    format!("HTTP call path {arg} is not a literal; its endpoint is unknown.");
                report(flow, file, line, message);
                Call::Unresolved(arg)
            }
        };
        let condition = guard(text, views, at, file, flow);
        flow.actions.push(Action {
            screen: c.origin.clone(),
            trigger: None,
            call,
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

/// C# reading without a parser: comments blanked, declarations found on a
/// mask whose string and char literal contents are blanked too.
mod csharp {
    use super::VERBS;
    use super::text::{balanced, ident_start, literal, shape, split_args};
    use std::collections::BTreeMap;

    pub(super) struct Method {
        pub(super) name: String,
        /// Parameter name to declared type.
        pub(super) params: BTreeMap<String, String>,
        /// Byte range of the block contents or the `=>` expression.
        pub(super) body: (usize, usize),
    }

    pub(super) struct Class {
        pub(super) name: String,
        pub(super) namespace: String,
        pub(super) interface: bool,
        /// Base types: last segment, generics dropped.
        pub(super) bases: Vec<String>,
        /// Index of the source file.
        pub(super) source: usize,
        /// Field, property and (primary) constructor parameter names to types.
        pub(super) members: BTreeMap<String, String>,
        /// `const`/`readonly` string fields with a literal initializer.
        pub(super) strings: BTreeMap<String, String>,
        pub(super) methods: Vec<Method>,
    }

    pub(super) enum Part {
        Text(String),
        Hole(String),
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Kind {
        Code,
        Comment,
        Literal,
    }

    /// (code with comments blanked, mask with literal contents blanked too);
    /// both keep the byte offsets and line breaks of `text`.
    pub(super) fn clean(text: &str) -> (String, String) {
        let kinds = kinds(text.as_bytes());
        let (mut code, mut mask) = (String::new(), String::new());
        for (i, c) in text.char_indices() {
            let pad = |out: &mut String| match c {
                '\n' => out.push('\n'),
                _ => out.extend(std::iter::repeat_n(' ', c.len_utf8())),
            };
            match kinds[i] {
                Kind::Comment => pad(&mut code),
                _ => code.push(c),
            }
            match kinds[i] {
                Kind::Code => mask.push(c),
                _ => pad(&mut mask),
            }
        }
        (code, mask)
    }

    fn kinds(b: &[u8]) -> Vec<Kind> {
        let mut kinds = vec![Kind::Code; b.len()];
        let mut i = 0;
        while i < b.len() {
            // (kind, first byte, end byte, next position)
            let (kind, from, to, next) = match (b[i], b.get(i + 1)) {
                (b'/', Some(b'/')) => {
                    let end = find(b, i, b"\n");
                    (Kind::Comment, i, end, end)
                }
                (b'/', Some(b'*')) => {
                    let end = (find(b, i + 2, b"*/") + 2).min(b.len());
                    (Kind::Comment, i, end, end)
                }
                (b'"', _) => {
                    let end = string_end(b, i);
                    (Kind::Literal, i + 1, end, end + 1)
                }
                (b'\'', _) => match char_end(b, i) {
                    Some(end) => (Kind::Literal, i + 1, end, end + 1),
                    None => (Kind::Code, i, i, i + 1),
                },
                _ => (Kind::Code, i, i, i + 1),
            };
            kinds[from..to].fill(kind);
            i = next;
        }
        kinds
    }

    /// Offset of `needle` at or after `from`, or the end.
    fn find(b: &[u8], from: usize, needle: &[u8]) -> usize {
        b.get(from..)
            .and_then(|rest| rest.windows(needle.len()).position(|w| w == needle))
            .map_or(b.len(), |at| from + at)
    }

    /// Offset of the quote closing the string opened at `i` (verbatim `@"`
    /// doubles quotes; a plain string ends at a line break at the latest).
    fn string_end(b: &[u8], i: usize) -> usize {
        let verbatim = b[..i].ends_with(b"@") || b[..i].ends_with(b"@$");
        let mut j = i + 1;
        while j < b.len() {
            match b[j] {
                b'\\' if !verbatim => j += 1,
                b'"' if verbatim && b.get(j + 1) == Some(&b'"') => j += 1,
                b'"' | b'\n' if b[j] == b'"' || !verbatim => return j,
                _ => {}
            }
            j += 1;
        }
        b.len()
    }

    /// Offset of the quote closing a char literal at `i`, if it is one.
    fn char_end(b: &[u8], i: usize) -> Option<usize> {
        if b.get(i + 1) == Some(&b'\\') {
            return (i + 3..(i + 10).min(b.len())).find(|&j| b[j] == b'\'');
        }
        (b.get(i + 2) == Some(&b'\'')).then_some(i + 2)
    }

    /// Every `class` and `interface` with a body in one cleaned file.
    pub(super) fn classes(source: usize, code: &str, mask: &str) -> Vec<Class> {
        let mut found = Vec::new();
        for keyword in ["class", "interface"] {
            for (at, _) in mask.match_indices(keyword) {
                found.extend(declaration(source, (code, mask), at, keyword));
            }
        }
        found
    }

    fn declaration(
        source: usize,
        (code, mask): (&str, &str),
        at: usize,
        keyword: &str,
    ) -> Option<Class> {
        let word = |c: char| c.is_alphanumeric() || c == '_';
        let rest = &mask[at + keyword.len()..];
        if mask[..at].ends_with(|c: char| word(c) || c == '.')
            || !rest.starts_with(char::is_whitespace)
        {
            return None;
        }
        let rest = rest.trim_start();
        let name_at = mask.len() - rest.len();
        let name_end = name_at + rest.find(|c: char| !word(c)).unwrap_or(rest.len());
        let open = name_end + mask[name_end..].find(['{', ';'])?;
        if name_end == name_at || mask.as_bytes()[open] == b';' {
            return None;
        }
        let end = open + 1 + balanced(&mask[open + 1..], '}').len();
        let (members, bases) = header(&code[name_end..open]);
        let mut class = Class {
            name: mask[name_at..name_end].into(),
            namespace: namespace(code, mask, at),
            interface: keyword == "interface",
            bases,
            source,
            members,
            strings: BTreeMap::new(),
            methods: Vec::new(),
        };
        for segment in segments(mask, open + 1, end) {
            member(&mut class, (code, mask), segment);
        }
        Some(class)
    }

    /// Primary constructor parameters and base types after the class name.
    fn header(header: &str) -> (BTreeMap<String, String>, Vec<String>) {
        let mut rest = header.trim_start();
        if rest.starts_with('<') {
            rest = skip_generics(rest).trim_start();
        }
        let mut primary = BTreeMap::new();
        if let Some(inner) = rest.strip_prefix('(') {
            let list = balanced(inner, ')');
            primary = params(list);
            rest = inner.get(list.len() + 1..).unwrap_or("");
        }
        let bases = rest
            .trim_start()
            .strip_prefix(':')
            .map_or_else(Vec::new, |b| {
                let b = &b[..b.find("where").unwrap_or(b.len())];
                split_args(b)
                    .iter()
                    .filter_map(|t| t.split(['<', '(']).next()?.rsplit('.').next())
                    .map(|t| t.trim().to_string())
                    .collect()
            });
        (primary, bases)
    }

    /// `Type name` pairs of a parameter list; attributes and defaults dropped.
    fn params(list: &str) -> BTreeMap<String, String> {
        split_args(list)
            .iter()
            .filter_map(|p| {
                let p = p.split('=').next()?.trim();
                let p = match p.strip_prefix('[') {
                    Some(rest) => rest.get(balanced(rest, ']').len() + 1..)?,
                    None => p,
                };
                match p.split_whitespace().collect::<Vec<_>>()[..] {
                    [.., ty, name] => Some((name.to_string(), ty.to_string())),
                    _ => None,
                }
            })
            .collect()
    }

    fn namespace(code: &str, mask: &str, at: usize) -> String {
        mask[..at]
            .rfind("namespace ")
            .map_or_else(String::new, |n| {
                let rest = code[n + "namespace ".len()..].trim_start();
                let end = rest.find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.'));
                rest[..end.unwrap_or(rest.len())].into()
            })
    }

    /// Member declarations at the top level of a class body: ranges ending
    /// before a `;` or after a closing `}`.
    fn segments(mask: &str, start: usize, end: usize) -> Vec<(usize, usize)> {
        let (mut found, mut depth, mut from) = (Vec::new(), 0, start);
        for (i, b) in mask.bytes().enumerate().take(end).skip(start) {
            match b {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' => depth -= 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        found.push((from, i + 1));
                        from = i + 1;
                    }
                }
                b';' if depth == 0 => {
                    found.push((from, i));
                    from = i + 1;
                }
                _ => {}
            }
        }
        found
    }

    /// Records one field, property, constructor or method declaration.
    fn member(class: &mut Class, (code, mask): (&str, &str), (start, end): (usize, usize)) {
        let lead = |s: usize| s + mask[s..end].len() - mask[s..end].trim_start().len();
        let mut s = lead(start);
        while mask[s..end].starts_with('[') {
            s = lead((s + balanced(&mask[s + 1..end], ']').len() + 2).min(end));
        }
        let decl = &mask[s..end];
        let stop = decl.find(['(', '=', '{', ';']).unwrap_or(decl.len());
        let words: Vec<&str> = decl[..stop].split_whitespace().collect();
        let nested = [
            "class",
            "interface",
            "record",
            "struct",
            "enum",
            "delegate",
            "event",
        ];
        if words.iter().any(|w| nested.contains(w)) {
            return;
        }
        if decl[stop..].starts_with('(') {
            return method(class, (code, mask), s + stop, &words);
        }
        let [.., ty, name] = words[..] else {
            return;
        };
        class.members.insert(name.into(), ty.into());
        let constant = words.contains(&"const") || words.contains(&"readonly");
        let init = code[s + stop..end].strip_prefix('=').map(str::trim);
        if let Some(value) = init
            .filter(|_| constant && ty == "string")
            .and_then(literal)
        {
            class.strings.insert(name.into(), value);
        }
    }

    fn method(class: &mut Class, (code, mask): (&str, &str), open: usize, words: &[&str]) {
        let Some(name) = words.last().and_then(|w| w.split('<').next()) else {
            return;
        };
        let list = balanced(&code[open + 1..], ')');
        let params = params(list);
        let after = (open + list.len() + 2).min(mask.len());
        let Some(at) = mask[after..].find(['{', ';', '=']) else {
            return;
        };
        let start = after + at;
        let body = match &mask[start..start + 1] {
            "{" => (
                start + 1,
                start + 1 + balanced(&mask[start + 1..], '}').len(),
            ),
            "=" if mask[start..].starts_with("=>") => (
                start + 2,
                start + 2 + balanced(&mask[start + 2..], ';').len(),
            ),
            _ => return,
        };
        if name == class.name {
            class.members.extend(params.clone());
        }
        class.methods.push(Method {
            name: name.into(),
            params,
            body,
        });
    }

    /// Parts of a whole `$"..."` argument; `None` for anything else.
    pub(super) fn interpolation(arg: &str) -> Option<Vec<Part>> {
        let body = arg.strip_prefix("$\"")?;
        let (mut parts, mut text, mut i) = (Vec::new(), String::new(), 0);
        while let Some(c) = body.get(i..).and_then(|r| r.chars().next()) {
            let next = body[i + c.len_utf8()..].chars().next();
            match (c, next) {
                ('{', Some('{')) | ('}', Some('}')) => i += 1,
                ('\\', Some(n)) => {
                    text.push(n);
                    i += n.len_utf8() + 1;
                    continue;
                }
                ('"', _) => {
                    parts.push(Part::Text(text));
                    return body[i + 1..].trim().is_empty().then_some(parts);
                }
                ('{', _) => {
                    let hole = balanced(&body[i + 1..], '}');
                    parts.push(Part::Text(std::mem::take(&mut text)));
                    parts.push(Part::Hole(hole.trim().into()));
                    i += hole.len() + 2;
                    continue;
                }
                _ => {}
            }
            text.push(c);
            i += c.len_utf8();
        }
        None
    }

    /// Every call `name(` with its receiver (`None` for an unqualified call,
    /// the identifier for `receiver.name(` / `receiver?.name(`) and its
    /// argument count. Calls on a chain (`a.b.name(`) or an expression
    /// (`f().name(`) are skipped.
    pub(super) fn invocations(text: &str) -> Vec<(usize, Option<&str>, &str, usize)> {
        let mut found = Vec::new();
        for (open, _) in text.match_indices('(') {
            let before = unangle(&text[..open]);
            let name = &before[ident_start(before)..];
            if !name.starts_with(|c: char| c.is_alphabetic() || c == '_') {
                continue;
            }
            let at = before.len() - name.len();
            let receiver = match before[..at].strip_suffix('.') {
                None => None,
                Some(object) => {
                    let object = object.strip_suffix('?').unwrap_or(object);
                    let start = ident_start(object);
                    if start == object.len() || object[..start].ends_with('.') {
                        continue;
                    }
                    Some(&object[start..])
                }
            };
            let arity = split_args(balanced(&text[open + 1..], ')')).len();
            found.push((at, receiver, name, arity));
        }
        found
    }

    /// `before` without trailing generic arguments (`Get<List<T>>`).
    fn unangle(before: &str) -> &str {
        let mut depth = 0;
        for (i, c) in before.char_indices().rev() {
            match c {
                '>' => depth += 1,
                '<' if depth > 0 => depth -= 1,
                _ if depth == 0 => return before,
                _ => {}
            }
            if depth == 0 {
                return &before[..i];
            }
        }
        before
    }

    /// The rest of `s` after the generic arguments it starts with.
    fn skip_generics(s: &str) -> &str {
        let mut depth = 0;
        for (i, c) in s.char_indices() {
            match c {
                '<' => depth += 1,
                '>' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                return &s[i + 1..];
            }
        }
        ""
    }

    /// `receiver.{Get,Post,Put,Delete,Patch}...Async(arg, ...)` calls:
    /// (offset, receiver, verb, first argument).
    pub(super) fn http_calls(text: &str) -> Vec<(usize, &str, &'static str, String)> {
        let mut found = Vec::new();
        for (at, _) in text.match_indices("Async") {
            let before = &text[..at];
            let start = ident_start(before);
            let Some(verb) = VERBS.iter().find(|v| before[start..].starts_with(*v)) else {
                continue;
            };
            let Some(object) = before[..start].strip_suffix('.') else {
                continue;
            };
            let mut rest = &text[at + "Async".len()..];
            if rest.starts_with('<') {
                rest = skip_generics(rest);
            }
            let Some(args) = rest.strip_prefix('(') else {
                continue;
            };
            let arg = split_args(balanced(args, ')')).into_iter().next();
            found.push((
                at,
                &object[ident_start(object)..],
                *verb,
                arg.unwrap_or_default(),
            ));
        }
        found
    }

    /// Route shape of a request path; an absolute URL keeps only its path.
    pub(super) fn endpoint(path: &str) -> String {
        match path.split_once("://") {
            Some((_, host)) => shape(host.split_once('/').map_or("", |(_, p)| p)),
            None => shape(path),
        }
    }
}

/// Calls through injected services, followed by fixed rules:
///
/// - Injection: `@inject T Name` and `[Inject] T Name { get; set; }` in a
///   component; in a C# class its fields, properties, constructor and C# 12
///   primary-constructor parameters with their declared types (a field
///   assigned from a constructor parameter has its own declared type).
/// - The class `T` is looked up among the `.cs` files of the component's
///   project and the projects it references (`<ProjectReference>`,
///   transitively); a qualified `T` must match the namespace. Several classes
///   of that name are a gap and are not followed; partial classes are not
///   merged. An interface is followed only when exactly one class of that
///   scope lists it as a base; otherwise a gap. A type without a class there
///   (framework, library) is skipped silently.
/// - `Name.Method(...)` follows the overloads with as many parameters as the
///   call has arguments; when none has (defaults, `params`), every overload
///   of that name, and the action gets the union of their calls. From there
///   calls on typed members and unqualified calls of the same class are
///   followed too, up to three method bodies deep (a gap beyond); a visited
///   set stops cycles.
/// - HTTP: `client.{Get,Post,Put,Delete,Patch}...Async(url)` and
///   `client.SendAsync(new HttpRequestMessage(HttpMethod.X, url))` (also via a
///   local), where `client` is declared `HttpClient`; the name does not count.
/// - `url`: a literal; `$"..."` whose holes are `const`/`readonly` string
///   fields with a literal initializer (substituted) or parameters and locals
///   (parameter segments), except that a leading hole is the base address and
///   must be such a field or a local resolving by these rules; a local
///   assigned once from such an expression; a
///   field alone. The query string is dropped. Anything else (a helper
///   method, `string.Join`, `.TrimEnd`) is `Call::Unresolved` with a gap at the
///   service line.
/// - gRPC clients (`Outer.OuterClient`, also through a `using` alias) are no
///   HTTP: one note per client type, no action.
/// - The action is placed at the call site in the component, with its screen
///   and `<AuthorizeView>` condition; trigger is none.
mod services {
    use super::csharp::{self, Class, Method, Part};
    use super::text::{
        attr, balanced, calls, guards, ident_start, line, literal, split_args, tag_end,
    };
    use super::{Component, Files, MAX_FILES, SOURCE, guard, project_of, read, report};
    use archguard_core::{Action, Call, DiagnosticSeverity, UiFlow, UiFlowDiagnostic};
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::{Component as Step, Path, PathBuf};

    /// Method bodies followed from one component call site.
    pub(super) const MAX_DEPTH: usize = 3;

    struct Source {
        file: String,
        code: String,
        aliases: BTreeMap<String, String>,
    }

    /// Classes of every project a component can reach; each file parsed once.
    #[derive(Default)]
    pub(super) struct Index {
        sources: Vec<Source>,
        classes: Vec<Class>,
        projects: Vec<String>,
        by_name: BTreeMap<String, Vec<usize>>,
        /// Project directory to the project directories it references.
        refs: BTreeMap<String, BTreeSet<String>>,
    }

    enum Resolved {
        Http,
        Grpc(String),
        Class(usize),
        Gap(String),
        Unknown,
    }

    impl Index {
        pub(super) fn build(
            root: &Path,
            files: &Files,
            components: &[Component],
            flow: &mut UiFlow,
        ) -> Index {
            let mut index = Index::default();
            let base = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
            let mut queue: Vec<String> = components.iter().map(|c| c.project.clone()).collect();
            while let Some(project) = queue.pop() {
                if let std::collections::btree_map::Entry::Vacant(slot) = index.refs.entry(project)
                {
                    let refs = references(&base, slot.key(), files, flow);
                    queue.extend(refs.iter().cloned());
                    slot.insert(refs);
                }
            }
            let project = |rel: &str| {
                project_of(rel, &files.projects)
                    .unwrap_or_default()
                    .to_string()
            };
            let wanted: Vec<&String> = files
                .cs
                .iter()
                .filter(|rel| index.refs.contains_key(&project(rel)))
                .collect();
            for (n, rel) in wanted.into_iter().enumerate() {
                if n == MAX_FILES {
                    let message = format!(
                        "More than {MAX_FILES} .cs files; the rest was not read and its services are unknown."
                    );
                    report(flow, "", 0, message);
                    break;
                }
                if let Some(text) = read(root, rel, flow) {
                    index.add(rel, project(rel), &text);
                }
            }
            index
        }

        fn add(&mut self, rel: &str, project: String, text: &str) {
            let (code, mask) = csharp::clean(text);
            let source = self.sources.len();
            for class in csharp::classes(source, &code, &mask) {
                let at = self.classes.len();
                self.by_name.entry(class.name.clone()).or_default().push(at);
                self.classes.push(class);
                self.projects.push(project.clone());
            }
            self.sources.push(Source {
                file: rel.into(),
                aliases: aliases(&code),
                code,
            });
        }

        /// `project` and every project it references, transitively.
        fn scope(&self, project: &str) -> BTreeSet<String> {
            let mut seen = BTreeSet::new();
            let mut queue = vec![project.to_string()];
            while let Some(p) = queue.pop() {
                if let Some(refs) = self.refs.get(&p)
                    && seen.insert(p)
                {
                    queue.extend(refs.iter().cloned());
                }
            }
            seen
        }

        fn resolve(
            &self,
            ty: &str,
            aliases: &BTreeMap<String, String>,
            scope: &BTreeSet<String>,
        ) -> Resolved {
            let ty = ty.trim_end_matches('?');
            let ty = aliases.get(ty).map_or(ty, String::as_str);
            let segments: Vec<&str> = ty.split('.').collect();
            let (namespace, name) = match ty.rsplit_once('.') {
                Some((namespace, name)) => (Some(namespace), name),
                None => (None, ty),
            };
            if ty.contains('<') {
                return Resolved::Unknown;
            }
            if name == "HttpClient" {
                return Resolved::Http;
            }
            let found: Vec<usize> = self
                .candidates(name, scope)
                .filter(|&i| namespace.is_none_or(|ns| self.classes[i].namespace == ns))
                .collect();
            match found[..] {
                [] if matches!(segments[..], [.., outer, client] if client == format!("{outer}Client")) => {
                    Resolved::Grpc(ty.into())
                }
                [] => Resolved::Unknown,
                [one] if self.classes[one].interface => self.implementation(name, scope),
                [one] => Resolved::Class(one),
                _ => Resolved::Gap(format!(
                    "Service type {ty} is declared {} times ({}); calls through it are not followed.",
                    found.len(),
                    self.files(&found)
                )),
            }
        }

        fn candidates(&self, name: &str, scope: &BTreeSet<String>) -> impl Iterator<Item = usize> {
            let found = self.by_name.get(name).map_or(&[][..], Vec::as_slice);
            found
                .iter()
                .copied()
                .filter(move |&i| scope.contains(&self.projects[i]))
        }

        fn implementation(&self, interface: &str, scope: &BTreeSet<String>) -> Resolved {
            let found: Vec<usize> = (0..self.classes.len())
                .filter(|&i| scope.contains(&self.projects[i]))
                .filter(|&i| {
                    !self.classes[i].interface
                        && self.classes[i].bases.iter().any(|b| b == interface)
                })
                .collect();
            match found[..] {
                [one] => Resolved::Class(one),
                _ => Resolved::Gap(format!(
                    "Interface {interface} has {} implementations ({}); calls through it are not followed.",
                    found.len(),
                    self.files(&found)
                )),
            }
        }

        fn files(&self, classes: &[usize]) -> String {
            let files: BTreeSet<&str> = classes
                .iter()
                .map(|&i| self.sources[self.classes[i].source].file.as_str())
                .collect();
            files.into_iter().collect::<Vec<_>>().join(", ")
        }
    }

    /// Project directories referenced by the `.csproj` of `project`.
    fn references(
        base: &Path,
        project: &str,
        files: &Files,
        flow: &mut UiFlow,
    ) -> BTreeSet<String> {
        let Some(text) = files
            .projects
            .get(project)
            .and_then(|p| read(base, p, flow))
        else {
            return BTreeSet::new();
        };
        text.match_indices("<ProjectReference")
            .filter_map(|(at, _)| {
                let tag = &text[at..];
                let include = attr(&tag[..tag_end(tag)?], "Include")?;
                let path = normalize(&base.join(project).join(include.replace('\\', "/")));
                let dir = path.parent()?.strip_prefix(base).ok()?;
                let dir = dir.to_string_lossy().replace('\\', "/");
                files.projects.contains_key(&dir).then_some(dir)
            })
            .collect()
    }

    /// `..` and `.` removed without touching the file system.
    fn normalize(path: &Path) -> PathBuf {
        let mut out = PathBuf::new();
        for step in path.components() {
            match step {
                Step::ParentDir => {
                    out.pop();
                }
                Step::CurDir => {}
                other => out.push(other),
            }
        }
        out
    }

    /// `using Alias = Full.Name;` lines of one file.
    fn aliases(code: &str) -> BTreeMap<String, String> {
        code.lines()
            .filter_map(|l| {
                let rest = l.trim().strip_prefix("using ")?.strip_suffix(';')?;
                let (alias, target) = rest.split_once('=')?;
                Some((alias.trim().to_string(), target.trim().to_string()))
            })
            .collect()
    }

    /// `@inject T Name` lines and `[Inject] T Name { get; set; }` properties.
    pub(super) fn injections(texts: &[(String, String)]) -> BTreeMap<String, String> {
        let mut found = BTreeMap::new();
        for l in texts.first().map_or("", |(_, t)| t.as_str()).lines() {
            if let Some(rest) = l.trim().strip_prefix("@inject ")
                && let [ty @ .., name] = &rest.split_whitespace().collect::<Vec<_>>()[..]
                && !ty.is_empty()
            {
                found.insert(name.to_string(), ty.join(" "));
            }
        }
        for (_, text) in texts {
            for (at, _) in text.match_indices("[Inject]") {
                let rest = &text[at + "[Inject]".len()..];
                let decl = &rest[..rest.find(['{', ';', '=']).unwrap_or(0)];
                if let [.., ty, name] = decl.split_whitespace().collect::<Vec<_>>()[..] {
                    found.insert(name.to_string(), ty.to_string());
                }
            }
        }
        found
    }

    pub(super) fn is_http_client(ty: &str) -> bool {
        ty.trim_end_matches('?').rsplit('.').next() == Some("HttpClient")
    }

    /// Actions for the calls a component makes through injected services.
    pub(super) fn actions(
        index: &Index,
        c: &Component,
        flow: &mut UiFlow,
        grpc: &mut BTreeSet<String>,
    ) {
        let scope = index.scope(&c.project);
        let none = BTreeMap::new();
        for (file, text) in &c.texts {
            let views = guards(text);
            for (at, receiver, method, arity) in csharp::invocations(text) {
                let Some(ty) = receiver.and_then(|r| c.injected.get(r)) else {
                    continue;
                };
                let line = line(text, at);
                let mut walk = Walk {
                    index,
                    scope: &scope,
                    flow: &mut *flow,
                    grpc: &mut *grpc,
                    visited: BTreeSet::new(),
                    calls: BTreeSet::new(),
                };
                walk.call(ty, &none, (method, arity), (file, line), 1);
                for call in walk.calls {
                    let condition = guard(text, &views, at, file, flow);
                    flow.actions.push(Action {
                        screen: c.origin.clone(),
                        trigger: None,
                        call,
                        file: file.clone(),
                        line,
                        condition,
                        source: SOURCE.into(),
                    });
                }
            }
        }
    }

    /// One component call site followed through the service classes.
    struct Walk<'a> {
        index: &'a Index,
        scope: &'a BTreeSet<String>,
        flow: &'a mut UiFlow,
        grpc: &'a mut BTreeSet<String>,
        /// (class, method, argument count) already followed.
        visited: BTreeSet<(usize, String, usize)>,
        calls: BTreeSet<Call>,
    }

    impl Walk<'_> {
        /// Follows `method` (name, argument count) on a member of type `ty`.
        fn call(
            &mut self,
            ty: &str,
            aliases: &BTreeMap<String, String>,
            method: (&str, usize),
            site: (&str, u32),
            depth: usize,
        ) {
            match self.index.resolve(ty, aliases, self.scope) {
                Resolved::Class(class) => self.follow(class, method, site, depth),
                Resolved::Grpc(client) if self.grpc.insert(client.clone()) => {
                    self.flow.diagnostics.push(UiFlowDiagnostic {
                        file: site.0.into(),
                        line: site.1,
                        message: format!(
                            "{client} is a gRPC client; its calls are not HTTP and give no action."
                        ),
                        severity: DiagnosticSeverity::Note,
                    });
                }
                Resolved::Gap(message) => report(self.flow, site.0, site.1, message),
                _ => {}
            }
        }

        fn follow(
            &mut self,
            class: usize,
            (method, arity): (&str, usize),
            (file, line): (&str, u32),
            depth: usize,
        ) {
            let index = self.index;
            let c = &index.classes[class];
            if depth > MAX_DEPTH {
                let message = format!(
                    "Call {}.{method} is more than {MAX_DEPTH} service calls deep; it is not followed.",
                    c.name
                );
                return report(self.flow, file, line, message);
            }
            if !self.visited.insert((class, method.into(), arity)) {
                return;
            }
            let named: Vec<&Method> = c.methods.iter().filter(|m| m.name == method).collect();
            if named.is_empty() {
                let message = format!(
                    "{}.{method} has no body in {} (inherited, extension or abstract); it is not followed.",
                    c.name, index.sources[c.source].file
                );
                return report(self.flow, file, line, message);
            }
            let exact: Vec<&Method> = named
                .iter()
                .copied()
                .filter(|m| m.params.len() == arity)
                .collect();
            for m in if exact.is_empty() { named } else { exact } {
                self.body(class, m, depth);
            }
        }

        /// HTTP calls in one method body, then the calls it makes.
        fn body(&mut self, class: usize, m: &Method, depth: usize) {
            let index = self.index;
            let c = &index.classes[class];
            let src = &index.sources[c.source];
            let (start, end) = m.body;
            let body = &src.code[start..end];
            let typed = |name: &str| m.params.get(name).or_else(|| c.members.get(name));
            let client = |name: &str| typed(name).is_some_and(|t| is_http_client(t));
            let site = |at: usize| (src.file.as_str(), line(&src.code, start + at));
            for (at, receiver, verb, arg) in csharp::http_calls(body) {
                if client(receiver) {
                    let found = url(c, m, body, &arg, 2).map(|path| (verb.to_uppercase(), path));
                    self.http(found, site(at));
                }
            }
            for (at, arg) in calls(body, ".SendAsync(") {
                if client(&body[ident_start(&body[..at])..at]) {
                    self.http(request(c, m, body, &arg), site(at));
                }
            }
            for (at, receiver, name, arity) in csharp::invocations(body) {
                let own = c.methods.iter().any(|x| x.name == name);
                let method = (name, arity);
                match (receiver, receiver.and_then(typed)) {
                    (Some("this") | None, _) if own => {
                        self.follow(class, method, site(at), depth + 1)
                    }
                    (Some(_), Some(ty)) => self.call(ty, &src.aliases, method, site(at), depth + 1),
                    _ => {}
                }
            }
        }

        fn http(&mut self, found: Result<(String, String), String>, (file, line): (&str, u32)) {
            let call = match found {
                Ok((method, path)) => Call::Http {
                    method,
                    path: csharp::endpoint(&path),
                },
                Err(text) => {
                    let message =
                        format!("HTTP call path {text} is not a literal; its endpoint is unknown.");
                    report(self.flow, file, line, message);
                    Call::Unresolved(text)
                }
            };
            self.calls.insert(call);
        }
    }

    /// Method and path of `SendAsync(arg)` when `arg` is (a local holding)
    /// `new HttpRequestMessage(HttpMethod.X, url)`.
    fn request(c: &Class, m: &Method, body: &str, arg: &str) -> Result<(String, String), String> {
        const NEW: &str = "new HttpRequestMessage(";
        let arg = arg.trim();
        let expr = match arg.starts_with(NEW) {
            true => arg,
            false => local(body, arg)
                .filter(|e| e.starts_with(NEW))
                .ok_or_else(|| arg.to_string())?,
        };
        let args = split_args(balanced(&expr[NEW.len()..], ')'));
        let verb = args
            .first()
            .and_then(|a| a.strip_prefix("HttpMethod."))
            .filter(|v| v.chars().all(char::is_alphanumeric));
        match (verb, args.get(1)) {
            (Some(verb), Some(path)) => Ok((verb.to_uppercase(), url(c, m, body, path, 2)?)),
            _ => Err(expr.into()),
        }
    }

    /// The path an argument evaluates to, or the source text that blocks it.
    fn url(c: &Class, m: &Method, body: &str, arg: &str, hops: u8) -> Result<String, String> {
        let arg = arg.trim().trim_end_matches('!').trim_end();
        if let Some(path) = literal(arg) {
            return Ok(path);
        }
        if arg.starts_with("$\"") {
            let parts = csharp::interpolation(arg);
            return parts
                .and_then(|p| holes(c, m, body, p, hops))
                .ok_or_else(|| arg.into());
        }
        match local(body, arg) {
            Some(expr) if hops > 0 => url(c, m, body, expr, hops - 1),
            _ => c.strings.get(arg).cloned().ok_or_else(|| arg.into()),
        }
    }

    /// The path of an interpolated string up to its query string.
    fn holes(c: &Class, m: &Method, body: &str, parts: Vec<Part>, hops: u8) -> Option<String> {
        let mut path = String::new();
        for part in parts {
            match part {
                Part::Text(text) => match text.find(['?', '#']) {
                    Some(query) => return Some(path + &text[..query]),
                    None => path.push_str(&text),
                },
                Part::Hole(hole) => {
                    let hole = hole.split([':', ',']).next().unwrap_or_default().trim();
                    let root = &hole[..ident_len(hole)];
                    match c.strings.get(hole) {
                        Some(value) => path.push_str(value),
                        // A leading hole is the base address, never a parameter segment.
                        None if path.is_empty() => {
                            path.push_str(&url(c, m, body, hole, hops).ok()?)
                        }
                        None if declared(m, body, root) => path.push_str("{p}"),
                        None => return None,
                    }
                }
            }
        }
        Some(path)
    }

    /// Length of the identifier `s` starts with.
    fn ident_len(s: &str) -> usize {
        s.find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .unwrap_or(s.len())
    }

    /// A parameter of `m`, or a local declared with `var` or assigned in `body`.
    fn declared(m: &Method, body: &str, name: &str) -> bool {
        !name.is_empty()
            && (m.params.contains_key(name)
                || local(body, name).is_some()
                || body.match_indices(&format!("var {name}")).any(|(at, _)| {
                    !body[at + 4 + name.len()..]
                        .starts_with(|c: char| c.is_alphanumeric() || c == '_')
                }))
    }

    /// The expression of the only assignment `name = ...;` in `body`.
    fn local<'a>(body: &'a str, name: &str) -> Option<&'a str> {
        if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return None;
        }
        let mut found = None;
        for (at, _) in body.match_indices(name) {
            let before = &body[..at];
            let after = body[at + name.len()..].trim_start();
            if before.ends_with(|c: char| c.is_alphanumeric() || c == '_' || c == '.') {
                continue;
            }
            let Some(rest) = after
                .strip_prefix('=')
                .filter(|r| !r.starts_with(['=', '>']))
            else {
                continue;
            };
            if found.replace(balanced(rest, ';').trim()).is_some() {
                return None;
            }
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{
        Assignment, Call, Condition, Origin, Reach, Screen, Target, Transition, reachable_screens,
    };

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
        await Http.DeleteAsync(ItemUrl(Id));
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

    fn from<'a>(flow: &'a UiFlow, origin: &Origin) -> Vec<&'a Transition> {
        flow.transitions
            .iter()
            .filter(|t| t.from == *origin)
            .collect()
    }

    fn page(id: &str) -> Origin {
        Origin::Screen(id.into())
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
        let home = from(&flow, &page("Shop:Pages/Home.razor"));
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
        let item = from(&flow, &page("Shop:Pages/Item.razor"));
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
        let cart = from(&flow, &page("Shop:Pages/Cart.razor"));
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
        let lost = from(&flow, &page("Shop:Pages/Cart.razor"))
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
        let shop = Origin::AnyScreen {
            application: "Shop".into(),
        };
        let menu = from(&flow, &shop);
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
        // The menu link reaches the admin page from every screen of Shop.
        let admin = Assignment {
            authenticated: true,
            roles: ["Admin".to_string()].into(),
            ..Assignment::default()
        };
        let reached = reachable_screens(&flow, "Shop:Pages/Home.razor", &admin);
        assert_eq!(
            reached.get("Shop:Pages/Admin.razor"),
            Some(&Reach::Possible)
        );
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
        let calls: Vec<(&Origin, &Call, u32)> = flow
            .actions
            .iter()
            .map(|a| (&a.screen, &a.call, a.line))
            .collect();
        let item = page("Shop:Pages/Item.razor");
        let http = |method: &str, path: &str| Call::Http {
            method: method.into(),
            path: path.into(),
        };
        assert_eq!(
            calls,
            [
                (&item, &http("GET", "/api/items"), 9),
                (&item, &http("POST", "/api/items/{p}"), 10),
                (&item, &Call::Unresolved("ItemUrl(Id)".into()), 14),
            ]
        );
        let gap = flow.diagnostics.iter().find(|d| d.line == 14).unwrap();
        assert_eq!(gap.severity, DiagnosticSeverity::Gap);
        assert!(flow.actions.iter().all(|a| a.source == "blazor"));
    }

    #[test]
    fn literal_targets_prefer_exact_routes_and_ties_stay_unresolved() {
        let dir = tempfile::tempdir().unwrap();
        let files = [
            ("App.csproj", ""),
            (
                "Nav.razor",
                "<a href=\"item/new\">N</a>\n<a href=\"item/42\">D</a>\n<a href=\"x/1\">X</a>\n",
            ),
            ("New.razor", "@page \"/item/new\"\n"),
            ("Detail.razor", "@page \"/item/{id:int}\"\n"),
            ("X.razor", "@page \"/x/{id}\"\n"),
            ("One.razor", "@page \"/{name}/1\"\n"),
        ];
        for (name, text) in files {
            std::fs::write(dir.path().join(name), text).unwrap();
        }
        let flow = extract(dir.path());
        let app = dir.path().file_name().unwrap().to_string_lossy();
        let targets: Vec<&Target> = flow.transitions.iter().map(|t| &t.target).collect();
        assert_eq!(
            targets,
            [
                &to(&format!("{app}:New.razor")),
                &to(&format!("{app}:Detail.razor")),
                &Target::Unresolved("x/1".into()),
            ]
        );
        let tie = flow.diagnostics.iter().find(|d| d.line == 3).unwrap();
        assert!(tie.message.contains("several routes"), "{}", tie.message);
        assert!(!flow.complete);
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

#[cfg(test)]
mod service_tests {
    use super::*;

    const WEB: &str = r#"<Project><ItemGroup>
  <ProjectReference Include="..\Components\Components.csproj" />
</ItemGroup></Project>"#;

    const ORDERS: &str = r#"@page "/orders"
@inject OrderingService OrderingService
@inject QueryHttpBuilder httpBuilder
@code {
    protected override async Task OnInitializedAsync()
    {
        orders = await OrderingService.GetOrders();
        await httpBuilder.GetAsync("api/fake");
    }
}
"#;

    const ORDERING: &str = r#"namespace Shop.WebApp.Services;

public class OrderingService(HttpClient httpClient, HttpParams httpParams)
{
    // "/api/old/" was the former base
    private readonly string remoteServiceBaseUrl = "/api/orders/";

    public Task<Order[]> GetOrders()
    {
        httpParams.PostAsync("api/nope");
        return httpClient.GetFromJsonAsync<Order[]>(remoteServiceBaseUrl)!;
    }

    public Task CreateOrder(Info info, Guid id)
    {
        var message = new HttpRequestMessage(HttpMethod.Post, remoteServiceBaseUrl);
        return httpClient.SendAsync(message);
    }
}
"#;

    const CHECKOUT: &str = r#"@page "/checkout"
@inject BasketState Basket
<button @onclick="Place">Place order</button>
@code {
    async Task Place() => await Basket.CheckoutAsync(Info);
}
"#;

    const BASKET: &str = r#"using GrpcBasketClient = Shop.Grpc.Basket.BasketClient;

namespace Shop.WebApp.Services;

public class BasketState(OrderingService orderingService, GrpcBasketClient basketClient)
{
    public async Task CheckoutAsync(Info info)
    {
        await basketClient.DeleteBasketAsync(new());
        await orderingService.CreateOrder(info, Guid.NewGuid());
    }
}
"#;

    const ITEM_CS: &str = r#"public partial class Item
{
    [Inject] public ICatalogService Catalog { get; set; } = default!;

    protected override async Task OnInitializedAsync()
    {
        item = await Catalog.GetItem(Id);
        page = await Catalog.GetPage(0, 10);
        items = await Catalog.GetItems(ids);
        more = await Catalog.GetItems(0, 10);
    }
}
"#;

    const CATALOG: &str = r#"namespace Shop.Components.Services;

public interface ICatalogService
{
    Task<Item> GetItem(int id);
}

public class CatalogService : ICatalogService
{
    private const string BaseUrl = "api/catalog/";
    private readonly HttpClient _http;

    public CatalogService(HttpClient http)
    {
        _http = http;
    }

    public Task<Item> GetItem(int id)
    {
        var uri = $"{BaseUrl}items/{id}?api-version=2.0";
        return _http.GetFromJsonAsync<List<Item>>(uri);
    }

    public Task<Page> GetPage(int index, int size)
    {
        var uri = PageUri(BaseUrl, index, size);
        return _http.GetFromJsonAsync<Page>(uri);
    }

    private static string PageUri(string b, int i, int s) => $"{b}items?pageIndex={i}";

    public Task<List<Item>> GetItems(int[] ids)
    {
        var uri = $"{BaseUrl}items/by?ids={string.Join("&ids=", ids)}";
        return _http.GetFromJsonAsync<List<Item>>(uri);
    }

    public Task<List<Item>> GetItems(int page, int size)
    {
        var uri = PageUri(BaseUrl, page, size);
        return _http.GetFromJsonAsync<List<Item>>($"{uri}&v=2");
    }
}
"#;

    const ADMIN: &str = r#"@page "/admin"
@inject IReportService Reports
@inject Exporter Exporter
@code {
    void Run() { Reports.Build(); Exporter.Export(); }
}
"#;

    const REPORTS: &str = r#"public interface IReportService { void Build(); }
public class PdfReports : IReportService { public void Build() { } }
public class CsvReports : IReportService { public void Build() { } }
"#;

    const PING: &str = r#"public class PingService(PongService pong, HttpClient client)
{
    public void Ping() { pong.Pong(); }
    public void Again() => client.DeleteAsync("api/ping");
}

public class PongService(PingService ping, HttpClient client)
{
    public void Pong()
    {
        ping.Ping();
        client.GetAsync("api/pong");
    }
}
"#;

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let files: &[(&str, &str)] = &[
            ("WebApp/WebApp.csproj", WEB),
            ("WebApp/Pages/Orders.razor", ORDERS),
            ("WebApp/Pages/Checkout.razor", CHECKOUT),
            ("WebApp/Pages/Item.razor", "@page \"/item/{id:int}\"\n"),
            ("WebApp/Pages/Item.razor.cs", ITEM_CS),
            ("WebApp/Pages/Admin.razor", ADMIN),
            (
                "WebApp/Layout/PingMenu.razor",
                "@inject PingService Ping\n<button @onclick=\"() => Ping.Ping()\">Ping</button>\n",
            ),
            ("WebApp/Services/OrderingService.cs", ORDERING),
            ("WebApp/Services/BasketState.cs", BASKET),
            ("WebApp/Services/Reports.cs", REPORTS),
            ("WebApp/Services/Ping.cs", PING),
            (
                "WebApp/Services/ExporterA.cs",
                "namespace A;\npublic class Exporter { public void Export() { } }\n",
            ),
            (
                "WebApp/Services/ExporterB.cs",
                "namespace B;\npublic class Exporter { public void Export() { } }\n",
            ),
            ("Components/Components.csproj", "<Project />"),
            ("Components/Services/CatalogService.cs", CATALOG),
            // Not referenced by WebApp: its classes must not count.
            ("Other/Other.csproj", "<Project />"),
            (
                "Other/Services.cs",
                "public class CatalogService : ICatalogService { }\npublic class OrderingService { }\n",
            ),
        ];
        for (path, text) in files {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        dir
    }

    fn actions(flow: &UiFlow, file: &str) -> Vec<(Call, u32)> {
        flow.actions
            .iter()
            .filter(|a| a.file == file)
            .map(|a| (a.call.clone(), a.line))
            .collect()
    }

    fn http(method: &str, path: &str) -> Call {
        Call::Http {
            method: method.into(),
            path: path.into(),
        }
    }

    fn diagnostic<'a>(flow: &'a UiFlow, file: &str, line: u32) -> &'a UiFlowDiagnostic {
        flow.diagnostics
            .iter()
            .find(|d| d.file == file && d.line == line)
            .unwrap_or_else(|| panic!("no diagnostic {file}:{line}: {:#?}", flow.diagnostics))
    }

    #[test]
    fn service_call_reaches_the_http_client_through_a_base_url_field() {
        let flow = extract(fixture().path());
        // Neither `httpParams` (typed HttpParams) nor `httpBuilder` (typed
        // QueryHttpBuilder) is an HTTP client, whatever their names say.
        assert_eq!(
            actions(&flow, "WebApp/Pages/Orders.razor"),
            [(http("GET", "/api/orders"), 7)]
        );
        let orders = flow
            .actions
            .iter()
            .find(|a| a.file == "WebApp/Pages/Orders.razor")
            .unwrap();
        assert_eq!(
            orders.screen,
            Origin::Screen("WebApp:Pages/Orders.razor".into())
        );
        assert_eq!(orders.source, "blazor");
    }

    #[test]
    fn state_calls_service_through_primary_constructor_and_grpc_is_a_note() {
        let flow = extract(fixture().path());
        assert_eq!(
            actions(&flow, "WebApp/Pages/Checkout.razor"),
            [(http("POST", "/api/orders"), 5)]
        );
        let grpc = diagnostic(&flow, "WebApp/Services/BasketState.cs", 9);
        assert_eq!(grpc.severity, DiagnosticSeverity::Note);
        assert!(
            grpc.message.contains("Shop.Grpc.Basket.BasketClient"),
            "{}",
            grpc.message
        );
        let notes = flow
            .diagnostics
            .iter()
            .filter(|d| d.message.contains("gRPC"));
        assert_eq!(notes.count(), 1);
    }

    #[test]
    fn inject_attribute_and_single_implementation_with_interpolated_and_helper_urls() {
        let flow = extract(fixture().path());
        assert_eq!(
            actions(&flow, "WebApp/Pages/Item.razor.cs"),
            [
                (http("GET", "/api/catalog/items/{p}"), 7),
                (Call::Unresolved("PageUri(BaseUrl, index, size)".into()), 8),
                // The overload with one parameter only; the query hole does not count.
                (http("GET", "/api/catalog/items/by"), 9),
                // A leading hole is the base address: unknown, not a parameter.
                (Call::Unresolved("$\"{uri}&v=2\"".into()), 10),
            ]
        );
        let gap = diagnostic(&flow, "Components/Services/CatalogService.cs", 27);
        assert_eq!(gap.severity, DiagnosticSeverity::Gap);
        assert!(!flow.complete);
    }

    #[test]
    fn ambiguous_services_are_gaps_not_guesses() {
        let flow = extract(fixture().path());
        assert_eq!(actions(&flow, "WebApp/Pages/Admin.razor"), []);
        let gaps: Vec<&str> = flow
            .diagnostics
            .iter()
            .filter(|d| d.file == "WebApp/Pages/Admin.razor" && d.line == 5)
            .map(|d| d.message.as_str())
            .collect();
        assert_eq!(gaps.len(), 2, "{gaps:#?}");
        assert!(
            gaps.iter().any(|m| m.contains("IReportService")),
            "{gaps:#?}"
        );
        assert!(gaps.iter().any(|m| m.contains("Exporter")), "{gaps:#?}");
    }

    #[test]
    fn cycle_between_services_terminates() {
        let flow = extract(fixture().path());
        let menu: Vec<(&Origin, &Call)> = flow
            .actions
            .iter()
            .filter(|a| a.file == "WebApp/Layout/PingMenu.razor")
            .map(|a| (&a.screen, &a.call))
            .collect();
        let any = Origin::AnyScreen {
            application: "WebApp".into(),
        };
        assert_eq!(menu, [(&any, &http("GET", "/api/pong"))]);
    }
}
