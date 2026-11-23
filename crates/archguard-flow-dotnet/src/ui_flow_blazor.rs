//! Blazor extractor for ui-flow/v1 (ADR 0021): a line and tag level scanner
//! over `.razor` files and their `.razor.cs` partials, no Razor parser.
//!
//! Recognized:
//! - `@page "/route"`: one `page` screen per component; several `@page` lines
//!   keep the first as `route`, all of them resolve navigation targets.
//! - `[Authorize]`, `[Authorize(Roles = "A,B")]`, `[Authorize(Policy = "P")]`,
//!   `[Authorize("P")]` (via `@attribute`, `_Imports.razor` above the component,
//!   or on the component's own class in the code-behind) and `[AllowAnonymous]`
//!   (as an attribute only), which makes the page `Open`. Attribute names are exact.
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
//! - A component without `@page` acts where it is used, see [`usage`]: on each
//!   page that renders it (with the `<AuthorizeView>` condition around the
//!   tag), on every screen when a layout renders it, else on every screen of
//!   its application with a note.
//! - Triggers of handlers, see [`triggers`]: `@onclick` and the `EditForm`
//!   submit handlers name the actions and navigations their handler makes.
//! - A literal target matches parameter routes (`item/42` -> `/item/{id:int}`);
//!   an exact literal route wins, and a tie of equally good routes stays
//!   unresolved. An HTTP call with a computed path is a `Call::Unresolved` action.
//! - Targets built by query-only URIs, one-level helpers and fields, see [`helpers`].
//!
//! Not recognized: routes from constants or `[Route]`, helpers calling helpers
//! (they stay unresolved), commented-out C# code in `.razor` files, `@inject` in
//! `_Imports.razor`, components of referenced Razor class libraries (targets
//! resolve within one application).
use archguard_core::{
    AccessState, Action, Call, Condition, DiagnosticSeverity, Origin, Screen, ScreenKind,
    Transition, UiFlow, UiFlowDiagnostic,
};
use archguard_flow_clike::decl as backend;
use archguard_flow_clike::source::Source;
use archguard_flow_common::extract_ctx::{
    ExtractContext, ancestors, read_text, walk, with_root_config,
};
use archguard_flow_common::support::test_path;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use text::{
    Blocks, attr, blank, calls, guards, line, links, literal, path_of, split_args, tag_end,
};

const SOURCE: &str = "blazor";
const MAX_BYTES: u64 = 1 << 20;
const MAX_FILES: usize = 5_000;
const VERBS: [&str; 5] = ["Get", "Post", "Put", "Delete", "Patch"];

/// One `.razor` file with its optional code-behind.
struct Component {
    /// The component's screen, or every screen of its application without `@page`.
    origin: Origin,
    /// Where its transitions and actions start, with the condition of the
    /// usage site; set by [`usage::place`].
    contexts: Vec<(Origin, Option<Condition>)>,
    /// `@inherits LayoutComponentBase`.
    layout: bool,
    triggers: triggers::Triggers,
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
/// their transitions and actions start where they are used ([`usage`]).
pub fn extract(root: &Path) -> UiFlow {
    with_root_config(root, extract_with)
}

/// [`extract`] for one run.
pub fn extract_with(ctx: &ExtractContext) -> UiFlow {
    let root = ctx.root;
    let mut flow = UiFlow::default();
    let Some(files) = discover(root, &mut flow) else {
        return flow;
    };
    let mut components = load(root, &files, &mut flow);
    let index = services::Index::build(root, &files, &components, &mut flow);
    let notes = usage::place(&index, &mut components, &mut flow);
    let mut grpc = BTreeSet::new();
    for c in &components {
        navigation(&mut flow, c, &components, &index);
        services::actions(&index, c, &mut flow, &mut grpc);
    }
    usage::note(&mut flow, &components, notes);
    // A guard around several links reports its unknown condition once.
    flow.diagnostics.sort();
    flow.diagnostics.dedup();
    flow
}

fn discover(root: &Path, flow: &mut UiFlow) -> Option<Files> {
    let walk = match walk(root) {
        Ok(walk) => walk,
        Err(error) => {
            report(flow, "", 0, format!("Blazor files not scanned: {error}"));
            return None;
        }
    };
    flow.diagnostics.extend(
        walk.notes
            .into_iter()
            .map(|(file, message)| UiFlowDiagnostic {
                file,
                line: 0,
                message,
                severity: DiagnosticSeverity::Note,
            }),
    );
    let mut files = Files::default();
    for rel in walk.files {
        if test_path(&rel) {
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
    let mut imports = BTreeMap::new();
    for rel in &files.razor {
        if rel.rsplit('/').next() == Some("_Imports.razor")
            && let Some(text) = read(root, rel, flow)
        {
            let dir = rel.rsplit_once('/').map_or("", |(d, _)| d);
            imports.insert(dir.to_string(), (rel.clone(), razor(text)));
        }
    }
    let derived = attributes::derived_attributes(root, files);
    for rel in &files.razor {
        if rel.rsplit('/').next() == Some("_Imports.razor") {
            continue;
        }
        let Some(text) = read(root, rel, flow) else {
            continue;
        };
        let mut above = Vec::new();
        for dir in ancestors(rel) {
            above.extend(imports.get(dir).cloned());
            if files.projects.contains_key(dir) {
                break;
            }
        }
        above.reverse();
        let mut texts = vec![(rel.clone(), razor(text))];
        let cs = format!("{rel}.cs");
        if files.code_behind.contains(&cs)
            && let Some(text) = read(root, &cs, flow)
        {
            texts.push((cs, razor(text)));
        }
        components.push(component(
            root,
            rel,
            &files.projects,
            texts,
            (&above, &derived),
            flow,
        ));
    }
    components
}

/// Links in the markup, `NavigateTo` and HTTP calls in markup and code-behind.
fn navigation(flow: &mut UiFlow, c: &Component, components: &[Component], index: &services::Index) {
    for (n, (file, text)) in c.texts.iter().enumerate() {
        let views = guards(text);
        let markup = if n == 0 { links(text) } else { vec![] };
        let navigations = calls(text, ".NavigateTo(")
            .into_iter()
            .filter_map(|(at, arg)| match path_of(&arg) {
                Some(path) if text::external(&path) => None,
                path => Some((at, arg, path, None)),
            });
        for (at, source, path, trigger) in markup.into_iter().chain(navigations) {
            let line = line(text, at);
            let dests = match path {
                Some(path) => Ok(vec![helpers::Dest::Path(path)]),
                None => helpers::dests(&source, c, components, index),
            };
            let condition = guard(text, &views, at, file, flow);
            let trigger = trigger.or_else(|| c.triggers.at(n, at));
            for (from, condition) in c.placed(condition) {
                let (targets, gaps) = helpers::targets(&dests, &from, components, &source);
                for message in gaps {
                    report(flow, file, line, message);
                }
                flow.transitions
                    .extend(targets.into_iter().map(|target| Transition {
                        weak: false,
                        from: from.clone(),
                        target,
                        trigger: trigger.clone(),
                        file: file.clone(),
                        line,
                        condition: archguard_core::AccessState::from_parts(
                            condition.clone(),
                            false,
                        ),
                        source: SOURCE.into(),
                    }));
            }
        }
        http(flow, c, (n, file), text, &views);
    }
}

fn read(root: &Path, rel: &str, flow: &mut UiFlow) -> Option<String> {
    let text = read_text(root, rel, MAX_BYTES);
    if text.is_err() {
        let message = "File not read: larger than 1 MiB, not a regular file or not readable; its screens and navigation are unknown.";
        report(flow, rel, 0, message.into());
    }
    text.ok()
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
    (imports, derived): (&[(String, String)], &BTreeSet<String>),
    flow: &mut UiFlow,
) -> Component {
    let (application, inner) = owner(root, rel, projects);
    let injected = services::injections(&texts);
    let (file, text) = &texts[0];
    let (routes, first) = routes(file, text, flow);
    let layout = text.lines().any(|l| {
        (l.trim().strip_prefix("@inherits")).is_some_and(|r| r.contains("LayoutComponentBase"))
    });
    let triggers = triggers::of(&texts);
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
        let access = attributes::access(&texts, imports, derived, flow);
        flow.screens.push(Screen {
            weak: false,
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
        contexts: vec![(origin.clone(), None)],
        origin,
        layout,
        triggers,
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
    ancestors(rel).find_map(|dir| projects.get_key_value(dir).map(|(found, _)| found.as_str()))
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
/// Authorization attributes of a component, see [`attributes::access`].
mod attributes {
    use super::*;

    /// `Authorize` and `AllowAnonymous` of the component: `@attribute` lines of
    /// the `_Imports.razor` files above it (outermost first), of the `.razor` and
    /// the attributes of its own class in the code-behind. Attribute names are
    /// exact (backend reader); `AllowAnonymous` at any level opens the page.
    pub(super) fn access(
        texts: &[(String, String)],
        imports: &[(String, String)],
        derived: &BTreeSet<String>,
        flow: &mut UiFlow,
    ) -> AccessState {
        let mut attributes = Vec::new();
        for (file, text) in imports.iter().chain(&texts[..1]) {
            attributes.extend(directives(file, text));
        }
        if let Some((file, text)) = texts.get(1) {
            let name = texts[0].0.rsplit('/').next().unwrap_or("");
            let name = name.strip_suffix(".razor").unwrap_or(name);
            let src = Source::new(file.clone(), text);
            for decl in backend::decls(&src, backend::Dialect::CSharp) {
                if decl.class && decl.name == name {
                    for a in decl.attrs {
                        let line = line_of(text, a.at);
                        attributes.push((file.clone(), line, a));
                    }
                }
            }
        }
        let (mut found, mut open) = (Vec::new(), false);
        for (file, line, a) in attributes {
            match a.name.as_str() {
                "AllowAnonymous" => open = true,
                "Authorize" => found.push(authorize(&a.args, &file, line, flow)),
                name if derived.contains(name) => {
                    let text = format!("{name}({})", a.args);
                    found.push(unknown(text.trim_end_matches("()"), &file, line, flow));
                }
                name if name.contains("Authorize") => flow.diagnostics.push(UiFlowDiagnostic {
                    file,
                    line,
                    message: format!(
                        "Attribute {name} is not a known authorization attribute; ignored."
                    ),
                    severity: DiagnosticSeverity::Note,
                }),
                _ => {}
            }
        }
        if open {
            AccessState::Open
        } else {
            AccessState::from_parts(all(found), false)
        }
    }

    /// 1-based line of byte offset `at`.
    fn line_of(text: &str, at: usize) -> u32 {
        text[..at.min(text.len())].matches('\n').count() as u32 + 1
    }

    /// The attributes of every `@attribute` directive, with file and line.
    fn directives(file: &str, text: &str) -> Vec<(String, u32, backend::Attr)> {
        let mut out = Vec::new();
        for (at, _) in text.match_indices("@attribute") {
            let (before, rest) = (&text[..at], &text[at + "@attribute".len()..]);
            let starts_line = before
                .rsplit('\n')
                .next()
                .is_some_and(|l| l.trim().is_empty());
            if !starts_line || !rest.starts_with(|c: char| c.is_whitespace() || c == '[') {
                continue;
            }
            let base = line_of(text, at);
            for a in backend::directive_attrs(rest) {
                let line = base + rest[..a.at].matches('\n').count() as u32;
                out.push((file.to_string(), line, a));
            }
        }
        out
    }

    /// Simple names of project classes that derive from `AuthorizeAttribute` or
    /// implement `IAuthorizeData`, following project base classes transitively.
    /// A chain that leaves the scanned files through any unresolved base (except
    /// the plain `Attribute`), or exceeds the depth bound, is included too:
    /// unknown, never "not authorize".
    pub(super) fn derived_attributes(root: &Path, files: &Files) -> BTreeSet<String> {
        let mut bases: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for rel in files.cs.iter().chain(&files.code_behind) {
            let Ok(text) = read_text(root, rel, MAX_BYTES) else {
                continue;
            };
            let src = Source::new(rel.clone(), &text);
            for d in backend::decls(&src, backend::Dialect::CSharp) {
                if d.class {
                    let list = bases.entry(backend::simple(&d.name)).or_default();
                    list.extend(d.bases.iter().cloned());
                }
            }
        }
        let mut names = BTreeSet::new();
        let mut walk = Walk::default();
        for name in bases.keys() {
            if walk.derives(name, &bases) {
                names.insert(name.clone());
            }
        }
        names
    }

    const MAX_DEPTH: usize = 16;

    /// The walk over base classes with a memo for interface diamonds.
    #[derive(Default)]
    struct Walk {
        path: Vec<String>,
        memo: BTreeMap<String, bool>,
        /// Cycle or depth cuts so far; a result is memoised only if none happened inside it.
        cuts: usize,
    }

    impl Walk {
        fn derives(&mut self, name: &str, bases: &BTreeMap<String, Vec<String>>) -> bool {
            if let Some(&hit) = self.memo.get(name) {
                return hit;
            }
            if self.path.iter().any(|p| p == name) {
                self.cuts += 1;
                return false;
            }
            if self.path.len() >= MAX_DEPTH {
                self.cuts += 1;
                return true;
            }
            let cuts = self.cuts;
            self.path.push(name.to_string());
            let list = bases.get(name).map(Vec::as_slice).unwrap_or_default();
            let hit = list.iter().any(|b| match b.as_str() {
                "AuthorizeAttribute" | "IAuthorizeData" => true,
                "Attribute" => false,
                _ => match bases.contains_key(&backend::simple(b)) {
                    true => self.derives(&backend::simple(b), bases),
                    // Base outside the scanned files: unresolved, a gap, never "not authorize".
                    false => true,
                },
            });
            self.path.pop();
            if self.cuts == cuts {
                self.memo.insert(name.to_string(), hit);
            }
            hit
        }
    }
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

impl Component {
    /// One (origin, condition) per usage context for a site whose own
    /// condition inside the component is `inner`.
    fn placed(&self, inner: Option<Condition>) -> Vec<(Origin, Option<Condition>)> {
        (self.contexts.iter())
            .map(|(origin, outer)| (origin.clone(), join(outer.clone(), inner.clone())))
            .collect()
    }

    /// File name without `.razor`: the tag name that renders it.
    fn name(&self) -> &str {
        let file = &self.texts[0].0;
        let file = file.rsplit('/').next().unwrap_or(file);
        file.strip_suffix(".razor").unwrap_or(file)
    }
}

/// Both conditions under one flat `All`, outer first.
fn join(outer: Option<Condition>, inner: Option<Condition>) -> Option<Condition> {
    let mut items = Vec::new();
    for c in outer.into_iter().chain(inner) {
        match c {
            Condition::All(nested) => items.extend(nested),
            c => items.push(c),
        }
    }
    all(items)
}

fn all(mut items: Vec<Condition>) -> Option<Condition> {
    match items.len() {
        0 => None,
        1 => items.pop(),
        _ => Some(Condition::All(items)),
    }
}

/// Actions for direct HTTP client calls with a literal path.
fn http(flow: &mut UiFlow, c: &Component, (n, file): (usize, &str), text: &str, views: &Blocks) {
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
        let trigger = c.triggers.at(n, at);
        for (screen, condition) in c.placed(condition) {
            flow.actions.push(Action {
                weak: false,
                screen,
                trigger: trigger.clone(),
                call: call.clone(),
                file: file.into(),
                line,
                condition: archguard_core::AccessState::from_parts(condition, false),
                source: SOURCE.into(),
            });
        }
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
    pub(super) use archguard_flow_common::lang_csharp::{
        balanced, ident_start, literal, split_args,
    };
    pub(super) use archguard_flow_common::lang_markup::{blank, strip_tags, tag_end};
    pub(super) use archguard_flow_common::lang_text::{collapse as words, line};

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

    /// Every `needle(` call: (offset, first argument text).
    pub(super) fn calls(text: &str, needle: &str) -> Vec<(usize, String)> {
        text.match_indices(needle)
            .map(|(at, _)| {
                let args = balanced(&text[at + needle.len()..], ')');
                (at, split_args(args).into_iter().next().unwrap_or_default())
            })
            .collect()
    }
}

/// C# reading without a parser: comments blanked, declarations found on a
/// mask whose string and char literal contents are blanked too.
mod csharp {
    use super::VERBS;
    use super::text::{balanced, ident_start, literal, shape, split_args};
    pub(super) use archguard_flow_common::lang_csharp::clean;
    use std::collections::BTreeMap;

    pub(super) struct Method {
        pub(super) name: String,
        /// Parameter name to declared type.
        pub(super) params: BTreeMap<String, String>,
        /// Byte range of the block contents or the `=>` expression.
        pub(super) body: (usize, usize),
    }

    #[derive(Default)]
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

    /// Methods of a `.cs` file's classes, or of a Razor file's `@code` and
    /// `@functions` blocks; body offsets are into `text`.
    pub(super) fn methods(file: &str, text: &str) -> Vec<Method> {
        if file.ends_with(".cs") {
            let (code, mask) = clean(text);
            return (classes(0, &code, &mask).into_iter())
                .flat_map(|c| c.methods)
                .collect();
        }
        let mut found = Vec::new();
        for keyword in ["@code", "@functions"] {
            for (at, _) in text.match_indices(keyword) {
                // Only the block is C#; the markup before it is not cleaned.
                let (code, mask) = clean(&text[at..]);
                let Some(open) = mask.find('{') else {
                    continue;
                };
                if !mask[keyword.len()..open].trim().is_empty() {
                    continue;
                }
                let end = open + 1 + balanced(&mask[open + 1..], '}').len();
                let mut block = Class::default();
                for segment in segments(&mask, open + 1, end) {
                    member(&mut block, (&code, &mask), segment);
                }
                found.extend(block.methods.into_iter().map(|mut m| {
                    m.body = (m.body.0 + at, m.body.1 + at);
                    m
                }));
            }
        }
        found
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
/// - The action is placed at the call site in the component, with its usage
///   contexts, `<AuthorizeView>` condition and handler trigger.
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
        pub(super) fn scope(&self, project: &str) -> BTreeSet<String> {
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

        /// (file, code, method) of each method `method` of the one class
        /// `class` in `scope`; empty without such a class.
        pub(super) fn methods_of(
            &self,
            class: &str,
            method: &str,
            scope: &BTreeSet<String>,
        ) -> Result<Vec<(&str, &str, &Method)>, String> {
            let found: Vec<usize> = (self.candidates(class, scope))
                .filter(|&i| !self.classes[i].interface)
                .collect();
            match found[..] {
                [] => Ok(vec![]),
                [one] => {
                    let src = &self.sources[self.classes[one].source];
                    Ok((self.classes[one].methods.iter())
                        .filter(|m| m.name == method)
                        .map(|m| (src.file.as_str(), src.code.as_str(), m))
                        .collect())
                }
                _ => Err(format!(
                    "class {class} is declared {} times ({})",
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
        for (n, (file, text)) in c.texts.iter().enumerate() {
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
                let trigger = c.triggers.at(n, at);
                for call in walk.calls {
                    let condition = guard(text, &views, at, file, flow);
                    for (screen, condition) in c.placed(condition) {
                        flow.actions.push(Action {
                            weak: false,
                            screen,
                            trigger: trigger.clone(),
                            call: call.clone(),
                            file: file.clone(),
                            line,
                            condition: archguard_core::AccessState::from_parts(condition, false),
                            source: SOURCE.into(),
                        });
                    }
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

/// Navigation targets that are no literal, by fixed rules:
///
/// - `Nav.GetUriWithQueryParameter(s)(...)` on a `NavigationManager` (injected,
///   a helper parameter, or the name itself) stays on the current screen: the
///   placed page, a gap from `any_screen`.
/// - A helper call `Name(...)` (a method of the component) or `Class.Name(...)`
///   (a static method of the one component or `.cs` class `Class` in the
///   project scope, as for services) is inlined one level: its expression
///   body, or every `return` of its block, must be a literal, an interpolated
///   string (holes become parameter segments) or the rule above; one
///   transition per distinct value. Anything else is a gap naming the helper
///   with file:line. Overloads count together.
/// - A field `name` of the component whose initializer and assignments are all
///   readable by the rules above gives one transition per distinct value. A
///   component parameter (`[Parameter]`, `[SupplyParameterFromQuery]`, ...)
///   is set from outside: a gap.
/// - Literal paths resolve in the application of the screen the component is
///   placed on.
mod helpers {
    use super::Component;
    use super::csharp::{self, Method};
    use super::services::Index;
    use super::text::{balanced, line, path_of};
    use archguard_core::{Origin, Target, best_route};
    use std::collections::BTreeMap;

    #[derive(PartialEq)]
    pub(super) enum Dest {
        /// Route shape of a literal path.
        Path(String),
        /// Only the query string changes.
        Current,
    }

    impl Dest {
        /// The screen this destination means when the link starts at `from`.
        pub(super) fn resolve(
            &self,
            from: &Origin,
            components: &[Component],
            text: &str,
        ) -> Result<String, String> {
            let application = match from {
                Origin::Screen(_) => (components.iter())
                    .find(|c| c.origin == *from)
                    .and_then(|c| c.application.as_deref()),
                Origin::AnyScreen { application } => {
                    Some(application.as_str()).filter(|a| !a.is_empty())
                }
            };
            match (self, from) {
                (Dest::Current, Origin::Screen(id)) => Ok(id.clone()),
                (Dest::Current, Origin::AnyScreen { .. }) => Err(format!(
                    "Navigation target {text} stays on the current screen; from every screen of the application it is no single target."
                )),
                (Dest::Path(path), _) => resolve(path, application, components, text),
            }
        }
    }

    /// The screen of the same application whose route matches best, or the
    /// diagnostic why there is none.
    fn resolve(
        path: &str,
        application: Option<&str>,
        components: &[Component],
        text: &str,
    ) -> Result<String, String> {
        let unresolved = || not_a_route(text);
        let candidates = components
            .iter()
            .filter(|c| c.application.as_deref() == application)
            .filter_map(|c| match &c.origin {
                Origin::Screen(id) => Some((id, &c.routes)),
                Origin::AnyScreen { .. } => None,
            })
            .flat_map(|(id, routes)| routes.iter().map(move |r| (id, r.as_str())));
        match best_route(path, candidates) {
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

    fn not_a_route(text: &str) -> String {
        format!("Navigation target {text} is not a route of this application.")
    }

    /// The deduplicated targets of `dests` for a link starting at `from`, and
    /// the gap messages of those that are not readable.
    pub(super) fn targets(
        dests: &Result<Vec<Dest>, String>,
        from: &Origin,
        components: &[Component],
        source: &str,
    ) -> (Vec<Target>, Vec<String>) {
        let found = match dests {
            Ok(dests) => (dests.iter())
                .map(|d| d.resolve(from, components, source))
                .collect(),
            Err(message) => vec![Err(message.clone())],
        };
        let (mut targets, mut gaps) = (Vec::new(), Vec::new());
        for found in found {
            let target = found.map(Target::Screen).unwrap_or_else(|message| {
                gaps.push(message);
                Target::Unresolved(source.into())
            });
            if !targets.contains(&target) {
                targets.push(target);
            }
        }
        (targets, gaps)
    }

    /// A method whose result a navigation uses.
    struct Helper {
        name: String,
        /// `file:line` of its declaration.
        at: String,
        params: BTreeMap<String, String>,
        /// The expression body, or the expression of every `return`.
        returns: Vec<String>,
        is_static: bool,
    }

    /// Destinations of a navigation source that is no literal: `@Expr` in an
    /// `href`, or a `NavigateTo` argument.
    pub(super) fn dests(
        source: &str,
        c: &Component,
        components: &[Component],
        index: &Index,
    ) -> Result<Vec<Dest>, String> {
        let fail = |why: String| format!("Navigation target {source}: {why}.");
        let expr = unwrap(source);
        if let Some(dest) = leaf(expr, c, &BTreeMap::new()) {
            return Ok(vec![dest]);
        }
        let helpers = match call(expr) {
            Some((qualifier, name)) => helpers(qualifier, name, c, components, index),
            None if is_ident(expr) => return field(expr, source, c, components, index),
            None => Ok(vec![]),
        };
        let helpers = helpers.map_err(fail)?;
        if helpers.is_empty() {
            return Err(not_a_route(source));
        }
        inline(&helpers, c).map_err(fail)
    }

    /// `@Expr` or `@(Expr)` without the Razor wrapping.
    fn unwrap(source: &str) -> &str {
        let expr = source.trim().trim_start_matches('@').trim();
        match expr.strip_prefix('(') {
            Some(inner) if balanced(inner, ')').len() + 2 == expr.len() => {
                inner[..inner.len() - 1].trim()
            }
            _ => expr,
        }
    }

    fn is_ident(s: &str) -> bool {
        s.starts_with(|c: char| c.is_alphabetic() || c == '_')
            && s.chars().all(|c| c.is_alphanumeric() || c == '_')
    }

    /// A literal or interpolated path, or a query-only URI.
    fn leaf(expr: &str, c: &Component, params: &BTreeMap<String, String>) -> Option<Dest> {
        if let Some(path) = path_of(expr) {
            return Some(Dest::Path(path));
        }
        let (receiver, rest) = expr.split_once('.')?;
        let rest = rest.strip_prefix("GetUriWithQueryParameter")?;
        let rest = rest.strip_prefix('s').unwrap_or(rest).strip_prefix('(')?;
        let navigation =
            |ty: &String| ty.trim_end_matches('?').rsplit('.').next() == Some("NavigationManager");
        let known = receiver == "NavigationManager"
            || c.injected.get(receiver).is_some_and(navigation)
            || params.get(receiver).is_some_and(navigation);
        // An `href` value ends at the first inner quote, so the call may be cut.
        let after = rest[balanced(rest, ')').len()..].trim();
        (known && matches!(after, "" | ")")).then_some(Dest::Current)
    }

    /// `[Qualifier.]Name(...)` spanning all of `expr`.
    fn call(expr: &str) -> Option<(Option<&str>, &str)> {
        let open = expr.find('(')?;
        let inner = &expr[open + 1..];
        if balanced(inner, ')').len() + 1 != inner.len() {
            return None;
        }
        let head = expr[..open].trim();
        if !head.split('.').all(is_ident) {
            return None;
        }
        Some(match head.rsplit_once('.') {
            Some((qualifier, name)) => (Some(qualifier), name),
            None => (None, head),
        })
    }

    /// The methods a helper call means; empty when none is known.
    fn helpers(
        qualifier: Option<&str>,
        name: &str,
        c: &Component,
        components: &[Component],
        index: &Index,
    ) -> Result<Vec<Helper>, String> {
        let Some(qualifier) = qualifier else {
            return Ok(own(c, name));
        };
        let first = qualifier.split('.').next().unwrap_or(qualifier);
        if c.injected.contains_key(first) {
            return Ok(vec![]);
        }
        let class = qualifier.rsplit('.').next().unwrap_or(qualifier);
        let scope = index.scope(&c.project);
        let pages: Vec<&Component> = (components.iter())
            .filter(|p| p.name() == class && scope.contains(&p.project))
            .collect();
        let found = match pages[..] {
            [] => (index.methods_of(class, name, &scope)?.into_iter())
                .map(|(file, code, m)| helper(file, code, m))
                .collect(),
            [page] => own(page, name),
            _ => return Err(format!("class {class} matches {} components", pages.len())),
        };
        Ok(found.into_iter().filter(|h| h.is_static).collect())
    }

    /// The component's own methods named `name`.
    fn own(c: &Component, name: &str) -> Vec<Helper> {
        (c.texts.iter())
            .flat_map(|(file, text)| {
                (csharp::methods(file, text).into_iter())
                    .filter(|m| m.name == name)
                    .map(|m| helper(file, text, &m))
            })
            .collect()
    }

    fn helper(file: &str, text: &str, m: &Method) -> Helper {
        let (start, end) = m.body;
        let (code, mask) = csharp::clean(&text[start..end]);
        let word = |c: char| c.is_alphanumeric() || c == '_';
        let returns = match text[..start].ends_with('{') {
            true => (mask.match_indices("return"))
                .filter(|(at, _)| !mask[..*at].ends_with(word) && !mask[at + 6..].starts_with(word))
                .map(|(at, _)| balanced(&code[at + 6..], ';').trim().to_string())
                .collect(),
            false => vec![code.trim().to_string()],
        };
        let declared = text[..start].rfind(m.name.as_str()).unwrap_or(start);
        Helper {
            name: m.name.clone(),
            at: format!("{file}:{}", line(text, declared)),
            params: m.params.clone(),
            returns,
            is_static: statement(&text[..declared])
                .split_whitespace()
                .any(|w| w == "static"),
        }
    }

    /// The tail of `head` after the last `;`, `{` or `}`.
    fn statement(head: &str) -> &str {
        &head[head.rfind([';', '{', '}']).map_or(0, |b| b + 1)..]
    }

    /// Every distinct value the helpers return, or why one is not readable.
    fn inline(helpers: &[Helper], c: &Component) -> Result<Vec<Dest>, String> {
        let mut found = Vec::new();
        for h in helpers {
            if h.returns.is_empty() {
                return Err(format!("helper {} ({}) returns no value", h.name, h.at));
            }
            for value in &h.returns {
                let dest = leaf(value, c, &h.params).ok_or_else(|| {
                    format!(
                        "helper {} ({}) returns {value}, which is no literal; the target is unknown",
                        h.name, h.at
                    )
                })?;
                add(&mut found, dest);
            }
        }
        Ok(found)
    }

    /// Destinations of a field from its initializer and assignments.
    fn field(
        name: &str,
        source: &str,
        c: &Component,
        components: &[Component],
        index: &Index,
    ) -> Result<Vec<Dest>, String> {
        let fail = |why: String| format!("Navigation target {source}: {why}.");
        let mut found = Vec::new();
        for (file, text) in &c.texts {
            for (at, value) in assignments(text, name).map_err(fail)? {
                let site = (file.as_str(), text.as_str(), at);
                for dest in assigned(name, &value, site, c, (components, index)).map_err(fail)? {
                    add(&mut found, dest);
                }
            }
        }
        match found.is_empty() {
            true => Err(not_a_route(source)),
            false => Ok(found),
        }
    }

    /// (offset, value) of the initializer and every assignment of `name`;
    /// `Err` for a component parameter.
    fn assignments(text: &str, name: &str) -> Result<Vec<(usize, String)>, String> {
        let (code, mask) = csharp::clean(text);
        let word = |ch: char| ch.is_alphanumeric() || ch == '_' || ch == '.';
        let mut found = Vec::new();
        for (at, _) in mask.match_indices(name) {
            let after = mask[at + name.len()..].trim_start();
            let assignment =
                after.starts_with('=') && !after.starts_with("==") && !after.starts_with("=>");
            if mask[..at].ends_with(word) || !assignment {
                continue;
            }
            if statement(&mask[..at]).contains("Parameter") {
                return Err(format!("{name} is a component parameter, set from outside"));
            }
            let value = balanced(&code[mask.len() - after.len() + 1..], ';');
            found.push((at, value.trim().to_string()));
        }
        Ok(found)
    }

    /// Destinations of one value assigned to the field `name` at `site`.
    fn assigned(
        name: &str,
        value: &str,
        (file, text, at): (&str, &str, usize),
        c: &Component,
        (components, index): (&[Component], &Index),
    ) -> Result<Vec<Dest>, String> {
        let unreadable = || {
            format!(
                "{name} is assigned {value} ({file}:{}), which is no literal; the target is unknown",
                line(text, at)
            )
        };
        if let Some(dest) = leaf(value, c, &BTreeMap::new()) {
            return Ok(vec![dest]);
        }
        let (qualifier, method) = call(value).ok_or_else(unreadable)?;
        let helpers = helpers(qualifier, method, c, components, index)?;
        if helpers.is_empty() {
            return Err(unreadable());
        }
        inline(&helpers, c)
    }

    fn add(found: &mut Vec<Dest>, dest: Dest) {
        if !found.contains(&dest) {
            found.push(dest);
        }
    }
}

/// Where a component without `@page` acts, by fixed rules:
///
/// - Usage sites are `<Name` tags in the markup of `.razor` files whose
///   project scope (the project and its `<ProjectReference>`s, as for
///   services) holds exactly one component file `Name.razor`. Several such
///   files make the name ambiguous: no usage is followed, a note says so.
/// - A page that renders it: its transitions and actions start at that page,
///   under the `<AuthorizeView>` condition around the tag, joined with the
///   conditions inside the component by a flat `All` (outer first).
/// - A layout (`@inherits LayoutComponentBase`, or named by `@layout`):
///   `any_screen` of the layout's application, under the condition of the tag.
/// - Another component: its own contexts, followed upward up to
///   [`MAX_DEPTH`] components; a usage that returns to a component already on
///   the path is skipped.
/// - No usage found, an ambiguous name or a chain too deep anywhere on the
///   way: `any_screen` of its own application without a condition, as before,
///   and a note on components that emit a transition or action.
mod usage {
    use super::services::Index;
    use super::text::guards;
    use super::{Component, guard, join};
    use archguard_core::{Condition, DiagnosticSeverity, Origin, UiFlow, UiFlowDiagnostic};
    use std::collections::{BTreeMap, BTreeSet};

    /// Components followed upward from one component.
    pub(super) const MAX_DEPTH: usize = 5;

    type Contexts = BTreeSet<(Origin, Option<Condition>)>;

    struct Graph {
        /// Per component: (using component, condition around the tag).
        uses: Vec<Vec<(usize, Option<Condition>)>>,
        ambiguous: BTreeMap<usize, String>,
        layouts: BTreeSet<usize>,
    }

    /// Sets the contexts of every component; returns the notes of those that
    /// fell back to `any_screen`.
    pub(super) fn place(
        index: &Index,
        components: &mut [Component],
        flow: &mut UiFlow,
    ) -> Vec<(usize, String)> {
        let graph = graph(index, components, flow);
        let mut notes = Vec::new();
        for i in 0..components.len() {
            match contexts(&graph, components, i, 0, &mut vec![]) {
                Ok(found) => components[i].contexts = found.into_iter().collect(),
                Err(reason) => notes.push((i, reason)),
            }
        }
        notes
    }

    /// One note per fallen-back component that emitted something.
    pub(super) fn note(flow: &mut UiFlow, components: &[Component], notes: Vec<(usize, String)>) {
        for (i, reason) in notes {
            let c = &components[i];
            let own = |file: &String| c.texts.iter().any(|(f, _)| f == file);
            let emits = (flow.transitions.iter().map(|t| &t.file))
                .chain(flow.actions.iter().map(|a| &a.file))
                .any(own);
            if emits {
                flow.diagnostics.push(UiFlowDiagnostic {
                    file: c.texts[0].0.clone(),
                    line: 1,
                    message: format!(
                        "Component {}: {reason}; its transitions and actions start from every screen of its application.",
                        c.name()
                    ),
                    severity: DiagnosticSeverity::Note,
                });
            }
        }
    }

    impl Graph {
        /// Marks every component a tag `<name>` in `file` could mean.
        fn ambiguous(
            &mut self,
            components: &[Component],
            several: &[usize],
            name: &str,
            file: &str,
        ) {
            let files: Vec<&str> = (several.iter())
                .map(|&i| components[i].texts[0].0.as_str())
                .collect();
            for &i in several {
                self.ambiguous.entry(i).or_insert_with(|| {
                    format!(
                        "<{name}> in {file} matches several components ({})",
                        files.join(", ")
                    )
                });
            }
        }
    }

    fn graph(index: &Index, components: &[Component], flow: &mut UiFlow) -> Graph {
        let mut graph = Graph {
            uses: components.iter().map(|_| vec![]).collect(),
            ambiguous: BTreeMap::new(),
            layouts: (0..components.len())
                .filter(|&i| components[i].layout)
                .collect(),
        };
        let names: BTreeSet<&str> = components.iter().map(Component::name).collect();
        for (by, c) in components.iter().enumerate() {
            let scope = index.scope(&c.project);
            let named = |name: &str| -> Vec<usize> {
                (0..components.len())
                    .filter(|&i| components[i].name() == name)
                    .filter(|&i| scope.contains(&components[i].project))
                    .collect()
            };
            let (file, text) = &c.texts[0];
            let views = guards(text);
            for (at, name) in tags(text).filter(|(_, n)| names.contains(n)) {
                match named(name)[..] {
                    [] => {}
                    [one] => {
                        let condition = guard(text, &views, at, file, flow);
                        graph.uses[one].push((by, condition));
                    }
                    ref several => graph.ambiguous(components, several, name, file),
                }
            }
            for name in layout_names(text) {
                graph.layouts.extend(named(name));
            }
        }
        graph
    }

    fn contexts(
        graph: &Graph,
        components: &[Component],
        i: usize,
        depth: usize,
        path: &mut Vec<usize>,
    ) -> Result<Contexts, String> {
        let c = &components[i];
        if matches!(c.origin, Origin::Screen(_)) || graph.layouts.contains(&i) {
            return Ok([(c.origin.clone(), None)].into());
        }
        if let Some(reason) = graph.ambiguous.get(&i) {
            return Err(reason.clone());
        }
        if depth == MAX_DEPTH {
            return Err(format!(
                "its usage chain is deeper than {MAX_DEPTH} components"
            ));
        }
        path.push(i);
        let mut found = Contexts::new();
        for (by, condition) in &graph.uses[i] {
            if path.contains(by) {
                continue;
            }
            let outer = contexts(graph, components, *by, depth + 1, path).map_err(|reason| {
                format!("used in {}, and there {reason}", components[*by].name())
            });
            let outer = match outer {
                Ok(outer) => outer,
                Err(reason) => {
                    path.pop();
                    return Err(reason);
                }
            };
            for (origin, outer) in outer {
                found.insert((origin, join(outer, condition.clone())));
            }
        }
        path.pop();
        match found.is_empty() {
            true => Err("component usage not found".into()),
            false => Ok(found),
        }
    }

    /// Opening tags with an upper-case name: (offset of `<`, name). A `<`
    /// right after an identifier is a generic (`List<Item>`), no tag.
    fn tags(text: &str) -> impl Iterator<Item = (usize, &str)> {
        text.match_indices('<').filter_map(|(at, _)| {
            if text[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_') {
                return None;
            }
            let after = &text[at + 1..];
            let end = after
                .find(|c: char| !(c.is_alphanumeric() || c == '_'))
                .unwrap_or(after.len());
            let (name, rest) = after.split_at(end);
            (name.starts_with(|c: char| c.is_ascii_uppercase())
                && rest.starts_with(|c: char| c.is_whitespace() || c == '/' || c == '>'))
            .then_some((at, name))
        })
    }

    /// Last segment of every `@layout X.Y.Name` line.
    fn layout_names(text: &str) -> impl Iterator<Item = &str> {
        text.lines().filter_map(|l| {
            let rest = l.trim().strip_prefix("@layout ")?.trim();
            rest.rsplit('.').next()
        })
    }
}

/// The trigger of an action or navigation made by an event handler:
///
/// - Elements: `@onclick` on any tag, `OnValidSubmit` or else `OnSubmit` on
///   `<EditForm>`. The value `Handler`, `@Handler`, `() => Handler(...)`,
///   `@(e => await Handler(...))` names the handler method; a lambda calling
///   something else (`() => Svc.Run()`) triggers the calls inside its text.
/// - The handler, and the methods of the component it calls unqualified (or
///   via `this.`), transitively, carry the trigger. A method claimed by an
///   earlier element in the markup keeps that element's trigger.
/// - Label: `aria-label`, else `data-testid`, else the element's text with
///   tags and Razor expressions (`@x.Y`, `@(...)`) removed; for an `EditForm`
///   instead of its text the label of its first `<button type="submit">`.
///   An element without a label gives no trigger.
mod triggers {
    use super::csharp;
    use super::text::{attr, balanced, strip_tags, tag_end, words};
    use std::collections::BTreeSet;

    /// (text index, start, end, label) ranges of handler bodies and lambdas.
    #[derive(Default)]
    pub(super) struct Triggers(Vec<(usize, usize, usize, String)>);

    impl Triggers {
        /// The label of the first range around offset `at` of text `n`.
        pub(super) fn at(&self, n: usize, at: usize) -> Option<String> {
            (self.0.iter())
                .find(|(t, s, e, _)| *t == n && *s <= at && at < *e)
                .map(|(.., label)| label.clone())
        }
    }

    pub(super) fn of(texts: &[(String, String)]) -> Triggers {
        let methods: Vec<(usize, csharp::Method)> = (texts.iter().enumerate())
            .flat_map(|(n, (file, text))| {
                csharp::methods(file, text).into_iter().map(move |m| (n, m))
            })
            .collect();
        let mut found = Vec::new();
        let mut claimed = BTreeSet::new();
        for (start, end, label, handler) in elements(&texts[0].1) {
            found.push((0, start, end, label.clone()));
            let mut queue: Vec<String> = handler.into_iter().collect();
            while let Some(name) = queue.pop() {
                if !claimed.insert(name.clone()) {
                    continue;
                }
                for (n, m) in methods.iter().filter(|(_, m)| m.name == name) {
                    found.push((*n, m.body.0, m.body.1, label.clone()));
                    queue.extend(local_calls(&texts[*n].1[m.body.0..m.body.1], &methods));
                }
            }
        }
        Triggers(found)
    }

    /// Names of the component's own methods called unqualified or via `this.`.
    fn local_calls(body: &str, methods: &[(usize, csharp::Method)]) -> Vec<String> {
        (csharp::invocations(body).into_iter())
            .filter(|(_, receiver, ..)| matches!(receiver, None | Some("this")))
            .filter(|(_, _, called, _)| methods.iter().any(|(_, m)| m.name == *called))
            .map(|(_, _, called, _)| called.to_string())
            .collect()
    }

    /// Handler elements with a label, in text order: (value start, value end,
    /// label, handler method).
    fn elements(text: &str) -> Vec<(usize, usize, String, Option<String>)> {
        let mut found = Vec::new();
        for (at, _) in text.match_indices('<') {
            let after = &text[at + 1..];
            let name = &after[..after
                .find(|c: char| !c.is_alphanumeric())
                .unwrap_or(after.len())];
            let rest = &after[name.len()..];
            let Some(end) = tag_end(rest).filter(|_| !name.is_empty()) else {
                continue;
            };
            let attrs = &rest[..end];
            let form = name == "EditForm";
            let value = match form {
                true => attr(attrs, "OnValidSubmit").or_else(|| attr(attrs, "OnSubmit")),
                false => attr(attrs, "@onclick"),
            };
            let Some(value) = value else {
                continue;
            };
            let inner = content(&rest[end + 1..], name, attrs);
            let label = label(attrs).or_else(|| match form {
                true => submit(inner),
                false => Some(plain(inner)).filter(|l| !l.is_empty()),
            });
            if let Some(label) = label {
                let start = value.as_ptr() as usize - text.as_ptr() as usize;
                found.push((
                    start,
                    start + value.len(),
                    label,
                    handler(value).map(String::from),
                ));
            }
        }
        found
    }

    /// Text up to `</name>`; empty for a self-closing tag.
    fn content<'a>(body: &'a str, name: &str, attrs: &str) -> &'a str {
        match attrs.trim_end().ends_with('/') {
            true => "",
            false => &body[..body.find(&format!("</{name}>")).unwrap_or(0)],
        }
    }

    fn label(attrs: &str) -> Option<String> {
        ["aria-label", "data-testid"]
            .iter()
            .filter_map(|name| attr(attrs, name).map(words))
            .find(|l| !l.is_empty() && !l.starts_with('@'))
    }

    /// Label of the first `<button type="submit">` in a form body.
    fn submit(form: &str) -> Option<String> {
        form.match_indices("<button").find_map(|(at, _)| {
            let rest = &form[at + "<button".len()..];
            let attrs = &rest[..tag_end(rest)?];
            (attr(attrs, "type") == Some("submit")).then_some(())?;
            let inner = content(&rest[attrs.len() + 1..], "button", attrs);
            label(attrs).or_else(|| Some(plain(inner)).filter(|l| !l.is_empty()))
        })
    }

    /// Literal text: Razor expressions and tags removed, whitespace collapsed.
    fn plain(s: &str) -> String {
        let mut out = String::new();
        let mut rest = s;
        while let Some(at) = rest.find('@') {
            out.push_str(&rest[..at]);
            let after = &rest[at + 1..];
            let skip = match after.strip_prefix('(') {
                Some(inner) => balanced(inner, ')').len() + 2,
                None => {
                    (after.find(|c: char| c.is_whitespace() || c == '<')).unwrap_or(after.len())
                }
            };
            rest = &after[skip.min(after.len())..];
        }
        out.push_str(rest);
        words(&strip_tags(&out))
    }

    /// The method a handler value names, if it names one.
    fn handler(value: &str) -> Option<&str> {
        let mut v = value.trim().trim_start_matches('@').trim();
        if let Some(inner) = v.strip_prefix('(')
            && balanced(inner, ')').len() + 2 == v.len()
        {
            v = inner[..inner.len() - 1].trim();
        }
        let v = v.split_once("=>").map_or(v, |(_, body)| body.trim());
        let v = v.strip_prefix("await ").unwrap_or(v).trim_start();
        let name = &v[..v
            .find(|c: char| !(c.is_alphanumeric() || c == '_'))
            .unwrap_or(v.len())];
        let rest = &v[name.len()..];
        (!name.is_empty() && (rest.trim().is_empty() || rest.starts_with('('))).then_some(name)
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
            archguard_core::AccessState::Protected(Condition::Authenticated)
        );
        assert_eq!(
            access("Shop:Pages/Item.razor"),
            archguard_core::AccessState::Protected(Condition::Any(vec![
                Condition::Role("Admin".into()),
                Condition::Role("Buyer".into())
            ]))
        );
        assert_eq!(
            access("Shop:Pages/Admin.razor"),
            archguard_core::AccessState::Protected(Condition::Policy("AdminOnly".into()))
        );
        assert_eq!(
            access("Shop:Pages/Login.razor"),
            archguard_core::AccessState::Open
        );
        assert_eq!(
            access("Shop:Pages/Home.razor"),
            archguard_core::AccessState::Undeclared
        );
        assert_eq!(
            access("Shop:Pages/Weird.razor"),
            archguard_core::AccessState::Protected(Condition::Unknown(
                "Roles = RoleNames.Admin".into()
            ))
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
        assert_eq!(home[0].condition, archguard_core::AccessState::Undeclared);
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
            .map(|t| (&t.target, t.condition.condition(), t.trigger.as_deref()))
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

    #[test]
    fn onclick_lambda_with_a_service_call_names_the_trigger() {
        let flow = extract(fixture().path());
        let trigger = |file: &str| {
            let action = flow.actions.iter().find(|a| a.file == file).unwrap();
            action.trigger.as_deref()
        };
        assert_eq!(trigger("WebApp/Layout/PingMenu.razor"), Some("Ping"));
        assert_eq!(trigger("WebApp/Pages/Checkout.razor"), Some("Place order"));
    }
}

#[cfg(test)]
mod usage_tests {
    use super::*;
    use archguard_core::{AccessState, EndpointAccess, UiFlowFindingKind, ui_flow_findings};

    const HOME: &str = r#"@page "/"
<AuthorizeView>
    <Authorized>
        <Hooks />
    </Authorized>
</AuthorizeView>
<Shared />
<Card />
"#;

    const OTHER: &str = r#"@page "/other"
@inject HttpClient Http
<Shared Title="x"></Shared>
<button @onclick="Clear" aria-label="Clear cart" data-testid="clear">X</button>
<button @onclick="Reset" data-testid="reset-btn">Reset</button>
@code {
    List<Lonely> generic = new();
    void Clear() => Http.DeleteAsync("api/cart");
    void Reset() => Http.PutAsync("api/reset", null);
}
"#;

    const CHECKOUT: &str = r#"@page "/checkout"
@inject HttpClient Http
@inject NavigationManager Nav
<EditForm Model="@m" OnValidSubmit="@Submit">
    <button class="b" type="submit">Place <i>@n</i> order</button>
</EditForm>
<button @onclick="Submit">Also</button>
@code {
    async Task Submit() { await Save(); Nav.NavigateTo("other"); }
    async Task Save() => await Http.PostAsync("api/orders", null);
}
"#;

    const HOOKS: &str = r#"@inject HttpClient Http
@code {
    protected override async Task OnInitializedAsync() => await Http.GetAsync("api/hooks");
}
"#;

    const SHARED: &str =
        "@inject HttpClient Http\n@code {\n    void Load() => Http.GetAsync(\"api/shared\");\n}\n";

    const LAYOUT: &str = r#"@inherits LayoutComponentBase
<AuthorizeView Roles="Admin"><AdminMenu /></AuthorizeView>
@Body
"#;

    const DEEP: &str =
        "@page \"/deep\"\n<AuthorizeView Roles=\"Admin\"><Panel /></AuthorizeView>\n";
    const PANEL: &str = "<AuthorizeView>\n    <Authorized>\n        <Widget />\n    </Authorized>\n</AuthorizeView>\n";
    const WIDGET: &str = r#"@inject HttpClient Http
<AuthorizeView Policy="P"><a href="other">W</a></AuthorizeView>
<button @onclick="() => Remove(1)">Remove <b>@Name</b> now</button>
@code {
    async Task Remove(int id) { await Http.DeleteAsync("api/widget"); }
}
"#;

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let files: &[(&str, &str)] = &[
            ("App/App.csproj", "<Project />"),
            ("App/Pages/Home.razor", HOME),
            ("App/Pages/Other.razor", OTHER),
            ("App/Pages/Checkout.razor", CHECKOUT),
            ("App/Pages/Deep.razor", DEEP),
            ("App/Parts/Hooks.razor", HOOKS),
            ("App/Parts/Shared.razor", SHARED),
            ("App/Parts/Panel.razor", PANEL),
            ("App/Parts/Widget.razor", WIDGET),
            ("App/Parts/Lonely.razor", "<a href=\"other\">L</a>\n"),
            ("App/Layout/MainLayout.razor", LAYOUT),
            ("App/Layout/AdminMenu.razor", "<a href=\"deep\">Admin</a>\n"),
            ("App/A/Card.razor", "<a href=\"other\">A</a>\n"),
            ("App/B/Card.razor", "<a href=\"other\">B</a>\n"),
        ];
        for (path, text) in files {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        dir
    }

    fn page(name: &str) -> Origin {
        Origin::Screen(format!("App:Pages/{name}.razor"))
    }

    fn any() -> Origin {
        Origin::AnyScreen {
            application: "App".into(),
        }
    }

    fn actions<'a>(flow: &'a UiFlow, file: &str) -> Vec<&'a Action> {
        flow.actions.iter().filter(|a| a.file == file).collect()
    }

    fn transitions<'a>(flow: &'a UiFlow, file: &str) -> Vec<&'a Transition> {
        flow.transitions.iter().filter(|t| t.file == file).collect()
    }

    fn notes<'a>(flow: &'a UiFlow, file: &str) -> Vec<&'a str> {
        (flow.diagnostics.iter())
            .filter(|d| d.file == file && d.severity == DiagnosticSeverity::Note)
            .map(|d| d.message.as_str())
            .collect()
    }

    fn http(method: &str, path: &str) -> Call {
        Call::Http {
            method: method.into(),
            path: path.into(),
        }
    }

    #[test]
    fn component_inside_authorize_view_on_one_page_starts_there_with_the_condition() {
        let mut flow = extract(fixture().path());
        let placed: Vec<(&Origin, Option<&Condition>)> = (actions(&flow, "App/Parts/Hooks.razor"))
            .into_iter()
            .map(|a| (&a.screen, a.condition.condition()))
            .collect();
        assert_eq!(placed, [(&page("Home"), Some(&Condition::Authenticated))]);
        flow.endpoints.push(EndpointAccess {
            method: "GET".into(),
            path: "/api/hooks".into(),
            access: AccessState::Protected(Condition::Authenticated),
            application: None,
            file: "Api/Hooks.cs".into(),
            line: 1,
            source: "test".into(),
            handler: None,
            sends: Vec::new(),
        });
        let refused: Vec<_> = (ui_flow_findings(&flow).into_iter())
            .filter(|f| f.kind == UiFlowFindingKind::ShownButRefused)
            .collect();
        assert!(refused.is_empty(), "{refused:#?}");
    }

    #[test]
    fn component_on_two_pages_acts_on_each() {
        let flow = extract(fixture().path());
        let placed: Vec<(&Origin, Option<&Condition>)> = (actions(&flow, "App/Parts/Shared.razor"))
            .into_iter()
            .map(|a| (&a.screen, a.condition.condition()))
            .collect();
        assert_eq!(placed, [(&page("Home"), None), (&page("Other"), None)]);
    }

    #[test]
    fn component_in_a_layout_acts_on_every_screen_with_the_usage_condition() {
        let flow = extract(fixture().path());
        let menu = transitions(&flow, "App/Layout/AdminMenu.razor");
        let placed: Vec<(&Origin, Option<&Condition>)> = menu
            .iter()
            .map(|t| (&t.from, t.condition.condition()))
            .collect();
        assert_eq!(placed, [(&any(), Some(&Condition::Role("Admin".into())))]);
        assert!(notes(&flow, "App/Layout/MainLayout.razor").is_empty());
        assert!(notes(&flow, "App/Layout/AdminMenu.razor").is_empty());
    }

    #[test]
    fn nested_components_follow_usage_up_to_the_page_and_join_conditions() {
        let flow = extract(fixture().path());
        let widget = transitions(&flow, "App/Parts/Widget.razor");
        let placed: Vec<(&Origin, Option<&Condition>)> = widget
            .iter()
            .map(|t| (&t.from, t.condition.condition()))
            .collect();
        let joined = Condition::All(vec![
            Condition::Role("Admin".into()),
            Condition::Authenticated,
            Condition::Policy("P".into()),
        ]);
        assert_eq!(placed, [(&page("Deep"), Some(&joined))]);
        let remove = actions(&flow, "App/Parts/Widget.razor");
        let placed: Vec<(&Origin, Option<&Condition>, Option<&str>)> = (remove.iter())
            .map(|a| (&a.screen, a.condition.condition(), a.trigger.as_deref()))
            .collect();
        let outer = Condition::All(vec![
            Condition::Role("Admin".into()),
            Condition::Authenticated,
        ]);
        // The lambda calls a local method; tags are stripped, `@Name` is no text.
        assert_eq!(placed, [(&page("Deep"), Some(&outer), Some("Remove now"))]);
    }

    #[test]
    fn unused_component_keeps_any_screen_with_a_note() {
        let flow = extract(fixture().path());
        let lonely = transitions(&flow, "App/Parts/Lonely.razor");
        assert_eq!(lonely.len(), 1);
        assert_eq!(
            (&lonely[0].from, lonely[0].condition.condition()),
            (&any(), None)
        );
        let notes = notes(&flow, "App/Parts/Lonely.razor");
        assert!(
            notes
                .iter()
                .any(|m| m.contains("component usage not found")),
            "{notes:#?}"
        );
    }

    #[test]
    fn ambiguous_component_name_keeps_any_screen_with_a_note() {
        let flow = extract(fixture().path());
        for file in ["App/A/Card.razor", "App/B/Card.razor"] {
            let placed: Vec<&Origin> = (transitions(&flow, file).into_iter())
                .map(|t| &t.from)
                .collect();
            assert_eq!(placed, [&any()], "{file}");
            let notes = notes(&flow, file);
            assert!(notes.iter().any(|m| m.contains("several")), "{notes:#?}");
        }
    }

    #[test]
    fn edit_form_submit_button_names_the_trigger_through_local_calls() {
        let flow = extract(fixture().path());
        let file = "App/Pages/Checkout.razor";
        let found: Vec<(&Call, Option<&str>)> = (actions(&flow, file).into_iter())
            .map(|a| (&a.call, a.trigger.as_deref()))
            .collect();
        // The form comes first; the later button with the same handler loses.
        assert_eq!(found, [(&http("POST", "/api/orders"), Some("Place order"))]);
        let nav: Vec<Option<&str>> = (transitions(&flow, file).into_iter())
            .map(|t| t.trigger.as_deref())
            .collect();
        assert_eq!(nav, [Some("Place order")]);
    }

    #[test]
    fn aria_label_wins_over_test_id_and_text() {
        let flow = extract(fixture().path());
        let found: Vec<(&Call, Option<&str>)> = (actions(&flow, "App/Pages/Other.razor"))
            .into_iter()
            .map(|a| (&a.call, a.trigger.as_deref()))
            .collect();
        assert_eq!(
            found,
            [
                (&http("DELETE", "/api/cart"), Some("Clear cart")),
                (&http("PUT", "/api/reset"), Some("reset-btn")),
            ]
        );
    }
}

#[cfg(test)]
mod helper_tests {
    use super::*;
    use archguard_core::Target;

    const WEB: &str = r#"<Project><ItemGroup>
  <ProjectReference Include="..\Parts\Parts.csproj" />
</ItemGroup></Project>"#;

    const CATALOG: &str = r#"@page "/"
@inject NavigationManager Nav
<CatalogSearch />
<CatalogListItem />
<a href="@ResetFiltersUri()">Clear all filters</a>
<a href="@PageUri(current - 1)">Previous</a>
<NavLink href="@Nav.GetUriWithQueryParameter("page", 2)">2</NavLink>
<a href="@Pick(true)">Pick</a>
<a href="@Opaque()">Opaque</a>
@code {
    private string PageUri(int page) => Nav.GetUriWithQueryParameter("page", page <= 1 ? (int?)null : page);
    private string ResetFiltersUri() => Nav.GetUriWithQueryParameters(new Dictionary<string, object?>()
    {
        { "brand", null },
    });
    private string Pick(bool cart)
    {
        if (cart)
        {
            return "cart";
        }
        return $"item/{Id}";
    }
    private string Opaque() => Compute(Id);
}
"#;

    const ITEM: &str = r#"@page "/item/{itemId:int}"
@inject NavigationManager Nav
<a class="item-back" href="@backHref">Back</a>
<a href="@homeHref">Home</a>
<button @onclick="Add">Add</button>
@code {
    private string backHref = "";
    private string homeHref = "";
    protected override void OnInitialized()
    {
        backHref = ComputeBackHref();
        homeHref = "cart";
    }
    private string ComputeBackHref()
    {
        if (Referer is not null)
        {
            return refUri.PathAndQuery;
        }
        return "";
    }
    private void Add()
    {
        Nav.NavigateTo(Pages.User.LogIn.Url(Nav));
    }
}
"#;

    const LOGIN: &str = r#"@page "/user/login"
@inject NavigationManager Nav
@code {
    public static string Url(NavigationManager nav)
        => $"user/login?returnUrl={Uri.EscapeDataString(nav.ToBaseRelativePath(nav.Uri))}";
}
"#;

    const SEARCH: &str = r#"@inject NavigationManager Nav
<a href="@BrandUri(null)">All</a>
@code {
    private string BrandUri(int? brandId) => Nav.GetUriWithQueryParameters(new Dictionary<string, object?>()
    {
        { "page", null },
        { "brand", brandId },
    });
}
"#;

    const LIST_ITEM: &str = r#"@using Parts.Item
<a class="catalog-product" href="@ItemHelper.Url(Item)">@Item.Name</a>
"#;

    const HELPER: &str = r#"namespace Parts.Item;

public static class ItemHelper
{
    public static string Url(CatalogItem item)
        => $"item/{item.Id}";
}
"#;

    fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let files: &[(&str, &str)] = &[
            ("Web/Web.csproj", WEB),
            ("Web/Pages/Catalog.razor", CATALOG),
            ("Web/Pages/Item.razor", ITEM),
            ("Web/Pages/User/LogIn.razor", LOGIN),
            ("Web/Pages/Cart.razor", "@page \"/cart\"\n"),
            ("Parts/Parts.csproj", "<Project />"),
            ("Parts/Catalog/CatalogSearch.razor", SEARCH),
            ("Parts/Catalog/CatalogListItem.razor", LIST_ITEM),
            ("Parts/Item/ItemHelper.cs", HELPER),
        ];
        for (path, text) in files {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        dir
    }

    /// (from, target) of the transitions written in `file`, in order.
    fn edges(flow: &UiFlow, file: &str) -> Vec<(String, String)> {
        let id = |o: &Origin| match o {
            Origin::Screen(id) => id.clone(),
            Origin::AnyScreen { application } => format!("{application}:*"),
        };
        (flow.transitions.iter())
            .filter(|t| t.file == file)
            .map(|t| {
                let target = match &t.target {
                    Target::Screen(id) => id.clone(),
                    Target::Unresolved(text) => format!("? {text}"),
                };
                (id(&t.from), target)
            })
            .collect()
    }

    fn gaps<'a>(flow: &'a UiFlow, file: &str) -> Vec<(u32, &'a str)> {
        (flow.diagnostics.iter())
            .filter(|d| d.file == file && d.severity == DiagnosticSeverity::Gap)
            .map(|d| (d.line, d.message.as_str()))
            .collect()
    }

    fn pair(from: &str, to: &str) -> (String, String) {
        (from.into(), to.into())
    }

    const HOME: &str = "Web:Pages/Catalog.razor";

    #[test]
    fn query_parameter_uris_target_the_current_screen() {
        let flow = extract(fixture().path());
        let found = edges(&flow, "Web/Pages/Catalog.razor");
        assert_eq!(found[0], pair(HOME, HOME), "{found:#?}");
        assert_eq!(found[1], pair(HOME, HOME), "{found:#?}");
        // An inline call; the attribute value ends at the inner quote.
        assert_eq!(found[2], pair(HOME, HOME), "{found:#?}");
        // A component without @page targets the page it is placed on.
        let search = edges(&flow, "Parts/Catalog/CatalogSearch.razor");
        assert_eq!(search, [pair(HOME, HOME)]);
    }

    #[test]
    fn helpers_inline_one_level_with_one_transition_per_return() {
        let flow = extract(fixture().path());
        let found = edges(&flow, "Web/Pages/Catalog.razor");
        assert_eq!(
            found[3..],
            [
                pair(HOME, "Web:Pages/Cart.razor"),
                pair(HOME, "Web:Pages/Item.razor"),
                pair(HOME, "? @Opaque()"),
            ]
        );
        let gaps = gaps(&flow, "Web/Pages/Catalog.razor");
        assert_eq!(gaps.len(), 1, "{gaps:#?}");
        assert_eq!(gaps[0].0, 9);
        assert!(
            gaps[0].1.contains("Opaque") && gaps[0].1.contains("Web/Pages/Catalog.razor:24"),
            "{}",
            gaps[0].1
        );
    }

    #[test]
    fn static_helper_classes_resolve_in_the_page_that_places_the_component() {
        let flow = extract(fixture().path());
        let found = edges(&flow, "Parts/Catalog/CatalogListItem.razor");
        assert_eq!(found, [pair(HOME, "Web:Pages/Item.razor")]);
    }

    #[test]
    fn static_members_of_page_classes_resolve_to_their_route() {
        let flow = extract(fixture().path());
        let found = edges(&flow, "Web/Pages/Item.razor");
        assert_eq!(
            found.last(),
            Some(&pair("Web:Pages/Item.razor", "Web:Pages/User/LogIn.razor"))
        );
    }

    #[test]
    fn fields_assigned_only_readable_values_give_one_transition_per_value() {
        let flow = extract(fixture().path());
        let item = "Web:Pages/Item.razor";
        let found = edges(&flow, "Web/Pages/Item.razor");
        assert_eq!(
            found[..3],
            [
                pair(item, "? @backHref"),
                pair(item, HOME),
                pair(item, "Web:Pages/Cart.razor"),
            ]
        );
        let gaps = gaps(&flow, "Web/Pages/Item.razor");
        assert_eq!(gaps.len(), 1, "{gaps:#?}");
        assert!(
            gaps[0].1.contains("ComputeBackHref") && gaps[0].1.contains("Web/Pages/Item.razor:14"),
            "{}",
            gaps[0].1
        );
    }
}

#[cfg(test)]
mod access_tests {
    use super::*;
    use archguard_core::AccessState::{self, Open, Protected, Undeclared};

    fn state(files: &[(&str, &str)], id: &str) -> (AccessState, UiFlow) {
        let dir = tempfile::tempdir().unwrap();
        let mut all = vec![("App/App.csproj", "<Project />")];
        all.extend_from_slice(files);
        for (path, text) in all {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let flow = extract(dir.path());
        let access = (flow.screens.iter())
            .find(|s| s.id == id)
            .unwrap_or_else(|| panic!("no screen {id}: {:#?}", flow.screens))
            .access
            .clone();
        (access, flow)
    }

    fn role(r: &str) -> Condition {
        Condition::Role(r.into())
    }

    #[test]
    fn imports_file_applies_to_the_folder() {
        let (a, _) = state(
            &[
                (
                    "App/Pages/Admin/_Imports.razor",
                    "@attribute [Authorize(Roles = \"Admin\")]\n",
                ),
                ("App/Pages/Admin/Users.razor", "@page \"/admin/users\"\n"),
            ],
            "App:Pages/Admin/Users.razor",
        );
        assert_eq!(a, Protected(role("Admin")));
    }

    #[test]
    fn nested_allow_anonymous_in_imports_opens_the_page() {
        let files = [
            ("App/_Imports.razor", "@attribute [Authorize]\n"),
            (
                "App/Account/_Imports.razor",
                "@attribute [AllowAnonymous]\n",
            ),
            ("App/Account/Login.razor", "@page \"/login\"\n"),
            ("App/Home.razor", "@page \"/\"\n"),
        ];
        assert_eq!(state(&files, "App:Account/Login.razor").0, Open);
        assert_eq!(
            state(&files, "App:Home.razor").0,
            Protected(Condition::Authenticated)
        );
    }

    #[test]
    fn imports_and_page_attributes_combine() {
        let (a, _) = state(
            &[
                ("App/_Imports.razor", "@attribute [Authorize]\n"),
                (
                    "App/P.razor",
                    "@page \"/p\"\n@attribute [Authorize(Roles=\"A\")]\n",
                ),
            ],
            "App:P.razor",
        );
        assert_eq!(
            a,
            Protected(Condition::All(vec![Condition::Authenticated, role("A")]))
        );
    }

    #[test]
    fn unknown_attribute_names_are_not_authorization() {
        let page = "@page \"/p\"\n@attribute [MyAuthorize(\"x\")]\n";
        let (a, flow) = state(&[("App/P.razor", page)], "App:P.razor");
        assert_eq!(a, Undeclared);
        assert!(
            flow.diagnostics
                .iter()
                .any(|d| d.message.contains("MyAuthorize"))
        );
        // A project class deriving from AuthorizeAttribute makes it a gap.
        let derived = "public class MyAuthorizeAttribute : AuthorizeAttribute { }";
        let (a, flow) = state(
            &[("App/P.razor", page), ("App/My.cs", derived)],
            "App:P.razor",
        );
        assert_eq!(
            a,
            Protected(Condition::Unknown("MyAuthorize(\"x\")".into()))
        );
        assert!(!flow.complete);
    }

    #[test]
    fn transitive_derived_attributes_are_gaps() {
        let page = "@page \"/p\"\n@attribute [My(\"x\")]\n";
        let gap = Protected(Condition::Unknown("My(\"x\")".into()));
        // Two levels, declared in any order.
        let two = "public class My : BaseAuth { }\npublic class BaseAuth : AuthorizeAttribute { }";
        let (a, flow) = state(&[("App/P.razor", page), ("App/My.cs", two)], "App:P.razor");
        assert_eq!(a, gap);
        assert!(!flow.complete);
        // Interface through a base.
        let iface = "public class My : Base { }\npublic class Base : Attribute, IAuthorizeData { }";
        let (a, _) = state(
            &[("App/P.razor", page), ("App/My.cs", iface)],
            "App:P.razor",
        );
        assert_eq!(a, gap);
        // Intermediate base outside the scanned files: unknown, not "not authorize".
        let open = "public class My : VendorAuthorizeAttribute { }";
        let (a, flow) = state(&[("App/P.razor", page), ("App/My.cs", open)], "App:P.razor");
        assert_eq!(a, gap);
        assert!(!flow.complete);
        // A cycle terminates and is not an authorize attribute.
        let cycle = "public class My : B { }\npublic class B : My { }";
        let (a, _) = state(
            &[("App/P.razor", page), ("App/My.cs", cycle)],
            "App:P.razor",
        );
        assert_eq!(a, Undeclared);
    }

    #[test]
    fn attribute_with_an_unscanned_base_is_a_gap() {
        let page = "@page \"/p\"\n@attribute [My]\n";
        let cs = "public class My : VendorBase { }";
        let (a, flow) = state(&[("App/P.razor", page), ("App/My.cs", cs)], "App:P.razor");
        assert_eq!(a, Protected(Condition::Unknown("My".into())));
        assert!(!flow.complete);
    }

    /// Guard: interface diamonds terminate and give the same answer as chains.
    #[test]
    fn interface_diamonds_terminate_with_the_same_result() {
        let page = "@page \"/p\"\n@attribute [My]\n";
        let mut cs = String::from("public class My : IA0, IB0 { }\n");
        for i in 0..14 {
            let (a, b) = (format!("IA{i}"), format!("IB{i}"));
            let next = if i == 13 {
                "Attribute".to_string()
            } else {
                format!("IA{}, IB{}", i + 1, i + 1)
            };
            cs += &format!(
                "public interface {a} : {next} {{ }}\npublic interface {b} : {next} {{ }}\n"
            );
        }
        let (a, _) = state(&[("App/P.razor", page), ("App/My.cs", &cs)], "App:P.razor");
        assert_eq!(a, Undeclared);
        let cs = cs.replace(": Attribute", ": IAuthorizeData");
        let (a, _) = state(&[("App/P.razor", page), ("App/My.cs", &cs)], "App:P.razor");
        assert_eq!(a, Protected(Condition::Unknown("My".into())));
    }

    #[test]
    fn exact_names_and_qualified_forms() {
        for attr in [
            "[AuthorizeAttribute]",
            "[Microsoft.AspNetCore.Authorization.Authorize]",
        ] {
            let page = format!("@page \"/p\"\n@attribute {attr}\n");
            let (a, _) = state(&[("App/P.razor", &page)], "App:P.razor");
            assert_eq!(a, Protected(Condition::Authenticated), "{attr}");
        }
    }

    #[test]
    fn multi_line_attribute_keeps_the_role() {
        let page = "@page \"/p\"\n@attribute [Authorize(\n    Roles = \"Admin\")]\n";
        assert_eq!(
            state(&[("App/P.razor", page)], "App:P.razor").0,
            Protected(role("Admin"))
        );
    }

    #[test]
    fn allow_anonymous_only_counts_as_an_attribute() {
        let page = "@page \"/p\"\n@attribute [Authorize(Roles = \"Admin\")]\n@code {\n    [Parameter] public bool AllowAnonymous { get; set; }\n}\n";
        assert_eq!(
            state(&[("App/P.razor", page)], "App:P.razor").0,
            Protected(role("Admin"))
        );
    }

    #[test]
    fn several_attributes_combine() {
        let one = "@page \"/p\"\n@attribute [Authorize] [StreamRendering]\n";
        assert_eq!(
            state(&[("App/P.razor", one)], "App:P.razor").0,
            Protected(Condition::Authenticated)
        );
        let two = "@page \"/p\"\n@attribute [Authorize(Roles=\"A\")]\n@attribute [Authorize(Roles=\"B\")]\n";
        assert_eq!(
            state(&[("App/P.razor", two)], "App:P.razor").0,
            Protected(Condition::All(vec![role("A"), role("B")]))
        );
    }

    #[test]
    fn code_behind_counts_only_the_component_class() {
        let cs = "[Authorize(Roles=\"X\")]\npublic class Other { }\n\n[Authorize(Policy = \"P\")]\npublic partial class Orders { }\n";
        let (a, _) = state(
            &[
                ("App/Orders.razor", "@page \"/o\"\n"),
                ("App/Orders.razor.cs", cs),
            ],
            "App:Orders.razor",
        );
        assert_eq!(a, Protected(Condition::Policy("P".into())));
    }
}
