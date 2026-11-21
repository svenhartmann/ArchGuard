//! Vue and Nuxt extractor for ui-flow/v1 (ADR 0021, phase 6). A text-level scanner
//! without a template or TypeScript parser, built like the React extractor; what it
//! cannot read is a diagnostic, and compiler-backed resolution is ADR 0021 phase 7.
//!
//! Applications are directories whose `package.json` names `vue-router` or `nuxt`; a
//! file belongs to the deepest one holding it. Scanned: `.vue`, `.ts`, `.js`, `.tsx`,
//! `.jsx`. Skipped: `node_modules`, `dist`, `build`, `.nuxt`, `.output`, `cypress`,
//! `e2e`, `coverage`, `.storybook`, the engine's other excluded directories, test files
//! and `*.d.ts`. Templates are read only in `.vue` files, from the first `<template` to
//! the last `</template>`.
//!
//! Screens:
//! - Vue Router: the `routes` of `createRouter({ … })` or `new VueRouter({ … })`: an
//!   array literal, an array variable of the same file, or an array imported from one
//!   relative module (`const routes = […]` exported by name, `export default […]`,
//!   `export default routes`). A route object gives `path`, `name`, `component` and
//!   `children`; a child path joins the parent's, `''` is the parent's own path. The
//!   component is an imported name or `() => import('./X.vue')`: the import name or file
//!   stem is the screen name, and the imported file owns the screen. Imports resolve
//!   when relative or `@/` (the Vite scaffold alias of `src/`, only when the file
//!   exists). A route whose children hold a `''` path is a layout (no screen of its
//!   own); a route with only `redirect` or only `children` is no screen.
//! - Nuxt: `pages/**/*.vue` and `app/pages/**/*.vue` (Nuxt 4). `index` is the
//!   directory, `[id]` and `[[id]]` become `:id`, `[...slug]` becomes `*`, a segment
//!   with a parameter inside text becomes `:param`, groups `(x)` are dropped. A `x.vue`
//!   beside `x/index.vue` is a layout (no screen of its own). The screen name is the
//!   file path; the route name for `{ name }` navigation is Nuxt's (`products-id`).
//!
//! Access, as screen conditions; children inherit their parent's, several are `all`:
//! - Route `meta` and Nuxt `definePageMeta({ … })`: a key of [`AUTH_META`] set to
//!   `true` is `authenticated`; a key of [`ROLE_META`] (string or string array) is
//!   `role` (several: `any`). Both lists are constants: a project with other keys adds
//!   them there.
//! - Nuxt `definePageMeta({ middleware: 'auth' })` or `['auth', 'admin']`: `custom(name)`.
//! - Not evaluated, each a note: `beforeEach` guards, a route's `beforeEnter`, Nuxt
//!   global middleware (`middleware/*.global.*`).
//!
//! Backend endpoints (Nuxt server): `server/api/**` (path `/api/…`) and
//! `server/routes/**` (path `/…`), one per file, segments as for pages; the method
//! comes from a `.get.ts`, `.post.ts`, … suffix, else `ANY`. Access is `undeclared`:
//! Nuxt server handlers declare none.
//!
//! Transitions: `<router-link>`, `<RouterLink>`, `<NuxtLink>`, `<nuxt-link>` with
//! `to="…"` or `:to="…"` / `v-bind:to="…"`; `push(…)` and `replace(…)` on a
//! `useRouter()` result, on `$router` (`this.$router`, templates), on a router a
//! `createRouter` call in the file is bound to or default-imported from a relative
//! module that calls `createRouter`; destructured `push`/`replace` of `useRouter()`;
//! Nuxt `navigateTo(…)`. The target is a literal, a template literal (holes become
//! `:param`), `{ path: '…' }`, or `{ name: '…' }`, which resolves by route name. Paths
//! resolve with `best_route` within the application; external URLs and `#anchors` are
//! no transition. Relative targets depend on the current URL and stay unresolved.
//!
//! Actions: `fetch(url, { method })` (default `GET`), Nuxt `$fetch`, `useFetch` and
//! `useLazyFetch` with the same arguments, `axios.get/post/put/patch/delete/head/
//! options(url)` on an `axios` default or namespace import, and the same methods on an
//! instance `axios.create({ baseURL })` declared in the file or imported from a
//! relative module; a literal `baseURL` path is prefixed. No other receiver counts.
//!
//! Triggers: an element with `@click` or `@submit` (also `v-on:`, with modifiers) that
//! names or calls a handler (`@click="save"`, `@click="save(id)"`): the navigations
//! and calls in that handler's body in the same file, and inline in the attribute, get
//! the element's `aria-label`, `data-testid` or text. A link's trigger is its own.
//!
//! Origin: links and calls in a file that owns a screen (a Nuxt page, a route
//! component) start at those screens; any other file (layouts, `App.vue`, shared
//! components, composables) acts on every screen of the application (`any_screen`).
//!
//! Not recognized; each case is a diagnostic and makes the flow incomplete:
//! non-literal paths, components, meta values, targets, URLs, methods and `baseURL`s,
//! route entries that are no object literal (spreads), named views (`components`).
//! Not followed: handlers in other files or called from a handler, calls through
//! helper functions or services, `<a href>`, Nuxt `router.options.ts`, links to
//! catch-all routes.
use crate::extract_ctx::{
    ExtractContext, dir_name as app_name, finish, gap as diag, read_source as read, walk,
    with_root_config,
};
use crate::lang_ts::*;
use crate::lang_ts_decl::*;
use crate::lang_ts_http::*;
use crate::services::support::test_path;
use archguard_core::{
    ANY_METHOD, AccessState, Action, Call, Condition, DiagnosticSeverity, EndpointAccess, Origin,
    Screen, ScreenKind, Target, Transition, UiFlow, UiFlowDiagnostic, best_route,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const SOURCE: &str = "vue";
const MAX_FILES: usize = 5_000;
/// Nesting bound for parsed literals and route trees.
const MAX_DEPTH: usize = 32;
const ROUTER_DEPS: [&str; 2] = ["vue-router", "nuxt"];
const SKIPPED_DIRS: [&str; 6] = [
    ".nuxt",
    ".output",
    "cypress",
    "e2e",
    ".storybook",
    "coverage",
];
const EXTENSIONS: [&str; 5] = [".vue", ".ts", ".js", ".tsx", ".jsx"];
const ROUTER_CALLS: [&str; 2] = ["createRouter(", "new VueRouter("];
/// Meta keys that require a signed-in user when `true`.
const AUTH_META: [&str; 1] = ["requiresAuth"];
/// Meta keys that list the roles of which one is required.
const ROLE_META: [&str; 1] = ["roles"];
const LINK_TAGS: [&str; 4] = ["router-link", "RouterLink", "NuxtLink", "nuxt-link"];
const TRIGGER_EVENTS: [&str; 2] = ["click", "submit"];
const FETCHES: [&str; 4] = ["fetch", "$fetch", "useFetch", "useLazyFetch"];
/// Screens, transitions, HTTP actions and Nuxt server endpoints of every Vue
/// application below `root`. Read errors and everything not literal become diagnostics.
pub fn extract(root: &Path) -> UiFlow {
    with_root_config(root, extract_with)
}

/// [`extract`] for one run.
pub fn extract_with(ctx: &ExtractContext) -> UiFlow {
    let mut flow = UiFlow::default();
    match walk(ctx.root) {
        Ok(walk) => {
            for app in applications(ctx.root, walk.files, &mut flow.diagnostics) {
                app.extract(&mut flow);
            }
        }
        Err(e) => diag(
            &mut flow.diagnostics,
            "",
            0,
            format!("Discovery exclusions not read; no Vue application analyzed: {e}"),
        ),
    }
    finish(&mut flow);
    flow
}

fn note(out: &mut Vec<UiFlowDiagnostic>, file: &str, line: u32, message: &str) {
    out.push(UiFlowDiagnostic {
        file: file.into(),
        line,
        message: message.into(),
        severity: DiagnosticSeverity::Note,
    });
}

fn is_source(rel: &str, name: &str) -> bool {
    EXTENSIONS.iter().any(|e| name.ends_with(e)) && !name.ends_with(".d.ts") && !test_path(rel)
}

struct App {
    name: String,
    /// The application directory with a trailing `/`, `""` at the root.
    prefix: String,
    nuxt: bool,
    /// Sources by path relative to the analysis root.
    files: BTreeMap<String, String>,
}

/// A screen with what navigation needs to find it.
struct Route {
    screen: Screen,
    /// The file of the component that renders the screen.
    owner: Option<String>,
    /// The route name for `{ name: '…' }` targets.
    name: Option<String>,
}

fn applications(root: &Path, files: Vec<String>, out: &mut Vec<UiFlowDiagnostic>) -> Vec<App> {
    let mut apps: BTreeMap<String, App> = BTreeMap::new();
    let mut sources = Vec::new();
    for rel in files {
        if rel.split('/').any(|p| SKIPPED_DIRS.contains(&p)) {
            continue;
        }
        let (dir, name) = rel.rsplit_once('/').unwrap_or(("", rel.as_str()));
        if name == "package.json" {
            if let Some(app) = manifest_app(root, dir, &rel) {
                apps.insert(dir.to_string(), app);
            }
        } else if is_source(&rel, name) {
            sources.push(rel);
        }
    }
    read_sources(root, &mut apps, sources, out);
    apps.into_values().collect()
}

/// The application a `package.json` declares, when it names `vue-router` or `nuxt`.
fn manifest_app(root: &Path, dir: &str, rel: &str) -> Option<App> {
    let text = read(root, rel).ok()?;
    if !ROUTER_DEPS.iter().any(|d| has_dep(&text, d)) {
        return None;
    }
    Some(App {
        name: app_name(root, dir),
        prefix: if dir.is_empty() {
            String::new()
        } else {
            format!("{dir}/")
        },
        nuxt: has_dep(&text, "nuxt"),
        files: BTreeMap::new(),
    })
}

/// Reads each source into the deepest application holding it.
fn read_sources(
    root: &Path,
    apps: &mut BTreeMap<String, App>,
    sources: Vec<String>,
    out: &mut Vec<UiFlowDiagnostic>,
) {
    let mut capped = BTreeSet::new();
    for rel in sources {
        let Some(dir) = apps
            .keys()
            .filter(|d| d.is_empty() || rel.starts_with(&format!("{d}/")))
            .max_by_key(|d| d.len())
            .cloned()
        else {
            continue;
        };
        let app = apps.get_mut(&dir).expect("key from the map");
        if app.files.len() >= MAX_FILES {
            if capped.insert(dir) {
                let message = format!(
                    "Vue scan of {} stopped after {MAX_FILES} source files; later files are unknown.",
                    app.name
                );
                diag(out, &rel, 0, message);
            }
            continue;
        }
        match read(root, &rel) {
            Ok(text) => {
                app.files.insert(rel, text);
            }
            Err(message) => diag(out, &rel, 0, message),
        }
    }
}

impl App {
    fn extract(&self, flow: &mut UiFlow) {
        let mut routes: Vec<Route> = Vec::new();
        let out = &mut flow.diagnostics;
        if self.nuxt {
            nuxt::pages(self, &mut routes, out);
            nuxt::endpoints(self, &mut flow.endpoints);
            nuxt::global_middleware(self, out);
        }
        for (file, text) in &self.files {
            router::routes(self, file, text, &mut routes, out);
        }
        let clients = Clients::new(&self.files);
        for (file, text) in &self.files {
            if self.nuxt && nuxt::is_server(self, file) {
                continue;
            }
            let mut froms: Vec<Origin> = routes
                .iter()
                .filter(|r| r.owner.as_deref() == Some(file))
                .map(|r| Origin::Screen(r.screen.id.clone()))
                .collect();
            if froms.is_empty() {
                let application = self.name.clone();
                froms.push(Origin::AnyScreen { application });
            }
            let vue = file.ends_with(".vue");
            let spans = if vue { trigger::spans(text) } else { vec![] };
            let at = Site {
                file,
                text,
                froms: &froms,
                spans: &spans,
            };
            nav::transitions(self, &routes, &at, vue, flow);
            http::actions(self, &clients, &at, flow);
        }
        flow.screens.extend(routes.into_iter().map(|r| r.screen));
    }

    fn screen(&self, route: String, name: String, file: &str, line: u32) -> Screen {
        Screen {
            weak: false,
            id: format!("{}:{route}", self.name),
            name,
            kind: ScreenKind::Page,
            route: Some(route),
            application: Some(self.name.clone()),
            file: file.into(),
            line,
            access: archguard_core::AccessState::Undeclared,
            source: SOURCE.into(),
        }
    }

    /// A relative or `@/` import resolved to a scanned file and its text.
    fn module(&self, from: &str, spec: &str) -> Option<(&str, &str)> {
        let base = match spec.strip_prefix("@/") {
            Some(rest) => format!("{}src/{rest}", self.prefix),
            None if spec.starts_with('.') => join(dir(from), spec),
            None => return None,
        };
        let mut candidates = vec![base.clone()];
        for ext in EXTENSIONS {
            candidates.push(format!("{base}{ext}"));
            candidates.push(format!("{base}/index{ext}"));
        }
        candidates.into_iter().find_map(|p| {
            self.files
                .get_key_value(&p)
                .map(|(k, v)| (k.as_str(), v.as_str()))
        })
    }
}

/// The file being scanned, the screens it acts on and its trigger spans.
struct Site<'a> {
    file: &'a str,
    text: &'a str,
    froms: &'a [Origin],
    spans: &'a [trigger::Span],
}

impl Site<'_> {
    /// Triggers of the code at `at`; `[None]` when no trigger span holds it.
    fn triggers(&self, at: usize) -> Vec<Option<String>> {
        let mut labels: Vec<Option<String>> = self
            .spans
            .iter()
            .filter(|s| (s.start..s.end).contains(&at))
            .map(|s| Some(s.label.clone()))
            .collect();
        labels.dedup();
        if labels.is_empty() {
            labels.push(None);
        }
        labels
    }
}

/// `None`, the one condition, or `all` of several.
fn all(mut conditions: Vec<Condition>) -> Option<Condition> {
    match conditions.len() {
        0 => None,
        1 => conditions.pop(),
        _ => Some(Condition::All(conditions)),
    }
}

/// A string literal or an array of string literals.
fn strings(value: &Value) -> Option<Vec<String>> {
    match value {
        Value::Str(s) => Some(vec![s.clone()]),
        Value::Arr(items) => items
            .iter()
            .map(|i| match &i.value {
                Value::Str(s) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        _ => None,
    }
}

/// Conditions of a route `meta` or `definePageMeta` object; `middleware` counts only
/// for Nuxt. A non-literal value is `unknown` and a diagnostic.
fn meta_access(
    text: &str,
    fields: &[(String, Item)],
    nuxt: bool,
    file: &str,
    out: &mut Vec<UiFlowDiagnostic>,
) -> Vec<Condition> {
    let mut found = Vec::new();
    for (key, v) in fields {
        let known = if AUTH_META.contains(&key.as_str()) {
            match v.source(text).as_str() {
                "true" => Some(vec![Condition::Authenticated]),
                "false" => Some(vec![]),
                _ => None,
            }
        } else if ROLE_META.contains(&key.as_str()) {
            strings(&v.value).map(|roles| {
                let mut roles: Vec<Condition> = roles.into_iter().map(Condition::Role).collect();
                match roles.len() {
                    0 | 1 => roles,
                    _ => vec![Condition::Any(std::mem::take(&mut roles))],
                }
            })
        } else if nuxt && key == "middleware" {
            strings(&v.value).map(|names| names.into_iter().map(Condition::Custom).collect())
        } else {
            continue;
        };
        match known {
            Some(conditions) => found.extend(conditions),
            None => {
                let source = v.source(text);
                let message = format!(
                    "Access meta {key} is not a literal; the screen's access is unknown: {source}"
                );
                diag(out, file, line_of(text, v.at), message);
                found.push(Condition::Unknown(source));
            }
        }
    }
    found
}

/// A template start tag: its attributes, whether it closes itself, the offset after `>`.
struct Tag {
    attrs: Vec<Attr>,
    self_closing: bool,
    end: usize,
}

struct Attr {
    name: String,
    value: AttrValue,
}

enum AttrValue {
    /// A quoted value and the offset of its content.
    Str(String, usize),
    Bare,
}

/// The start tag whose attributes begin at `start` (after the tag name).
fn tag(text: &str, start: usize) -> Tag {
    let b = text.as_bytes();
    let mut i = start;
    let mut attrs = Vec::new();
    loop {
        i = skip_ws(b, i);
        let (self_closing, end) = match b.get(i) {
            None => (false, b.len()),
            Some(b'>') => (false, i + 1),
            Some(b'/') if b.get(i + 1) == Some(&b'>') => (true, i + 2),
            _ => (false, 0),
        };
        if end > 0 {
            return Tag {
                attrs,
                self_closing,
                end,
            };
        }
        let len = text[i..]
            .find(|c: char| !(is_ident_char(c) || "-:@.#".contains(c)))
            .unwrap_or(text.len() - i);
        if len == 0 {
            i += 1;
            continue;
        }
        let name = text[i..i + len].to_string();
        i = skip_ws(b, i + len);
        let value =
            if b.get(i) == Some(&b'=') && matches!(b.get(skip_ws(b, i + 1)), Some(b'"' | b'\'')) {
                let mut p = Parser {
                    s: text,
                    i: skip_ws(b, i + 1),
                };
                let at = p.i + 1;
                let value = AttrValue::Str(p.string(), at);
                i = p.i;
                value
            } else {
                AttrValue::Bare
            };
        attrs.push(Attr { name, value });
    }
}

/// Offsets and names of the start tags in the `<template>` part of a `.vue` file.
fn template_tags(text: &str) -> Vec<(usize, &str)> {
    let (Some(start), Some(end)) = (text.find("<template"), text.rfind("</template>")) else {
        return vec![];
    };
    text[start..end]
        .match_indices('<')
        .filter_map(|(i, _)| {
            let rest = &text[start + i + 1..];
            if !rest.starts_with(|c: char| c.is_ascii_alphabetic()) {
                return None;
            }
            let len = rest
                .find(|c: char| !(is_ident_char(c) || c == '-' || c == '.'))
                .unwrap_or(rest.len());
            Some((start + i, &rest[..len]))
        })
        .collect()
}

/// `aria-label`, `data-testid` or the text of the element.
fn label(text: &str, tag: &Tag) -> Option<String> {
    for key in ["aria-label", "data-testid"] {
        if let Some(Attr {
            value: AttrValue::Str(s, _),
            ..
        }) = tag.attrs.iter().find(|a| a.name == key)
            && !s.trim().is_empty()
        {
            return Some(s.trim().to_string());
        }
    }
    if tag.self_closing {
        return None;
    }
    let inner = text.get(tag.end..)?;
    let inner = &inner[..inner.find("</").unwrap_or(inner.len())];
    let mut label = String::new();
    // Nested tags and `{{ expression }}` text are no label.
    let (mut in_tag, mut braces) = (false, 0usize);
    for c in inner.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            '{' => braces += 1,
            '}' => braces = braces.saturating_sub(1),
            c if !in_tag && braces == 0 => label.push(c),
            _ => {}
        }
    }
    let label = label.split_whitespace().collect::<Vec<_>>().join(" ");
    (!label.is_empty()).then_some(label)
}

/// Nuxt file routing, page meta and server endpoints.
mod nuxt {
    use super::*;

    /// Path below the application's `pages/` (or Nuxt 4 `app/pages/`), without `.vue`.
    fn page_path<'f>(app: &App, file: &'f str) -> Option<&'f str> {
        let inner = file.strip_prefix(app.prefix.as_str())?;
        let inner = inner.strip_prefix("app/").unwrap_or(inner);
        inner.strip_prefix("pages/")?.strip_suffix(".vue")
    }

    /// Server code: no screen's links or calls.
    pub(super) fn is_server(app: &App, file: &str) -> bool {
        file.strip_prefix(app.prefix.as_str())
            .is_some_and(|i| i.starts_with("server/"))
    }

    /// Route pattern and Nuxt route name of file path segments.
    fn route_of(segments: &[&str]) -> (String, String) {
        let mut segments = segments.to_vec();
        if segments.last() == Some(&"index") {
            segments.pop();
        }
        let (mut route, mut name) = (Vec::new(), Vec::new());
        for s in segments
            .into_iter()
            .filter(|s| !(s.starts_with('(') && s.ends_with(')')))
        {
            let inner = s.trim_start_matches('[').trim_end_matches(']');
            if let Some(rest) = inner.strip_prefix("...") {
                route.push("*".to_string());
                name.push(rest.to_string());
            } else if s.starts_with('[') && s.ends_with(']') {
                route.push(format!(":{inner}"));
                name.push(inner.to_string());
            } else if s.contains('[') {
                route.push(PARAM.to_string());
                name.push(s.replace(['[', ']'], ""));
            } else {
                route.push(s.to_string());
                name.push(s.to_string());
            }
        }
        let name = if name.is_empty() {
            "index".to_string()
        } else {
            name.join("-")
        };
        (format!("/{}", route.join("/")), name)
    }

    pub(super) fn pages(app: &App, routes: &mut Vec<Route>, out: &mut Vec<UiFlowDiagnostic>) {
        for (file, text) in &app.files {
            let Some(path) = page_path(app, file) else {
                continue;
            };
            let layout = format!("{}/index.vue", file.trim_end_matches(".vue"));
            if app.files.contains_key(&layout) {
                continue;
            }
            let (route, name) = route_of(&path.split('/').collect::<Vec<_>>());
            let (line, access) = page_meta(file, text, out);
            let mut screen = app.screen(route, file.clone(), file, line);
            screen.access = archguard_core::AccessState::from_parts(access, false);
            routes.push(Route {
                screen,
                owner: Some(file.clone()),
                name: Some(name),
            });
        }
    }

    /// Line and access of `definePageMeta({ … })`; line 1 and no access without one.
    fn page_meta(
        file: &str,
        text: &str,
        out: &mut Vec<UiFlowDiagnostic>,
    ) -> (u32, Option<Condition>) {
        let b = text.as_bytes();
        for pos in words(text, "definePageMeta") {
            let open = skip_ws(b, pos + "definePageMeta".len());
            if b.get(open) != Some(&b'(') {
                continue;
            }
            let line = line_of(text, pos);
            let arg = Parser {
                s: text,
                i: open + 1,
            }
            .value(0);
            let conditions = match &arg.value {
                Value::Obj(fields) => meta_access(text, fields, true, file, out),
                _ => {
                    let source = arg.source(text);
                    let message = format!(
                        "definePageMeta argument is not an object literal; the page's access is unknown: {source}"
                    );
                    diag(out, file, line, message);
                    vec![Condition::Unknown(source)]
                }
            };
            return (line, all(conditions));
        }
        (1, None)
    }

    pub(super) fn endpoints(app: &App, endpoints: &mut Vec<EndpointAccess>) {
        for (file, text) in &app.files {
            let Some(inner) = file.strip_prefix(app.prefix.as_str()) else {
                continue;
            };
            let (base, rest) = match (
                inner.strip_prefix("server/api/"),
                inner.strip_prefix("server/routes/"),
            ) {
                (Some(rest), _) => ("api/", rest),
                (_, Some(rest)) => ("", rest),
                _ => continue,
            };
            let stem = rest.rsplit_once('.').map_or(rest, |(s, _)| s);
            let (stem, method) = match stem.rsplit_once('.') {
                Some((s, m)) if HTTP_METHODS.contains(&m) => (s, m.to_ascii_uppercase()),
                _ => (stem, ANY_METHOD.to_string()),
            };
            let full = format!("{base}{stem}");
            let (path, _) = route_of(&full.split('/').collect::<Vec<_>>());
            let line = text.find("export default").map_or(1, |p| line_of(text, p));
            endpoints.push(EndpointAccess {
                method,
                path,
                access: AccessState::Undeclared,
                application: Some(app.name.clone()),
                file: file.clone(),
                line,
                source: SOURCE.into(),
                handler: None,
                sends: Vec::new(),
            });
        }
    }

    pub(super) fn global_middleware(app: &App, out: &mut Vec<UiFlowDiagnostic>) {
        for file in app.files.keys() {
            let inner = file.strip_prefix(app.prefix.as_str()).unwrap_or(file);
            let inner = inner.strip_prefix("app/").unwrap_or(inner);
            if inner.starts_with("middleware/") && inner.contains(".global.") {
                let message = "Global route middleware is not evaluated; what it checks on every page is unknown.";
                note(out, file, 1, message);
            }
        }
    }
}

/// Vue Router route arrays.
mod router {
    use super::*;

    struct Ctx<'a> {
        app: &'a App,
        file: &'a str,
        text: &'a str,
        routes: &'a mut Vec<Route>,
        out: &'a mut Vec<UiFlowDiagnostic>,
    }

    impl Ctx<'_> {
        fn diag(&mut self, at: usize, message: String) {
            diag(self.out, self.file, line_of(self.text, at), message);
        }
    }

    /// `(offset, offset after the call head)` of each router construction.
    pub(super) fn calls(text: &str) -> Vec<(usize, usize)> {
        let b = text.as_bytes();
        ROUTER_CALLS
            .iter()
            .flat_map(|call| {
                text.match_indices(call)
                    .filter(|(i, _)| *i == 0 || !is_ident_char(b[i - 1] as char))
                    .map(|(i, _)| (i, i + call.len()))
            })
            .collect()
    }

    pub(super) fn routes(
        app: &App,
        file: &str,
        text: &str,
        routes: &mut Vec<Route>,
        out: &mut Vec<UiFlowDiagnostic>,
    ) {
        for (pos, open) in calls(text) {
            let options = Parser { s: text, i: open }.value(0);
            let Value::Obj(fields) = &options.value else {
                let message = format!(
                    "Router options are not an object literal; the routes are unknown: {}",
                    options.source(text)
                );
                diag(out, file, line_of(text, pos), message);
                continue;
            };
            let Some((_, item)) = fields.iter().find(|(k, _)| k == "routes") else {
                continue;
            };
            let found = match &item.value {
                Value::Arr(_) => Some((file, text, item.at)),
                Value::Raw(name) if is_name(name) => array_of(app, file, text, name),
                _ => None,
            };
            let Some((file, text, at)) = found else {
                let message = format!(
                    "Route array {} is no array literal, array variable of this file or array of the relative module it is imported from; the routes are unknown.",
                    item.source(text)
                );
                diag(out, file, line_of(text, item.at), message);
                continue;
            };
            let mut ctx = Ctx {
                app,
                file,
                text,
                routes,
                out,
            };
            for it in (Parser { s: text, i: at }).array(0) {
                walk(&mut ctx, &it, "", &[], 0);
            }
        }
        for (pos, _) in text.match_indices(".beforeEach(") {
            let message = "Navigation guard beforeEach is not evaluated; screen access comes from route meta only.";
            note(out, file, line_of(text, pos), message);
        }
    }

    /// The file, text and `[` offset of the route array `name` in `file`.
    fn array_of<'a>(
        app: &'a App,
        file: &'a str,
        text: &'a str,
        name: &str,
    ) -> Option<(&'a str, &'a str, usize)> {
        if let Some(at) = local_array(text, name) {
            return Some((file, text, at));
        }
        let import = imports(text).into_iter().find(|i| i.local == name)?;
        let (module, mtext) = app.module(file, &import.module)?;
        let at = match import.imported.as_deref() {
            Some(exported) => local_array(mtext, exported)?,
            None => {
                let b = mtext.as_bytes();
                let pos = words(mtext, "default")
                    .into_iter()
                    .find(|&p| mtext[..p].trim_end().ends_with("export"))?;
                let j = skip_ws(b, pos + "default".len());
                match b.get(j) {
                    Some(b'[') => j,
                    _ => Some(ident_at(mtext, j))
                        .filter(|n| !n.is_empty())
                        .and_then(|n| local_array(mtext, n))?,
                }
            }
        };
        Some((module, mtext, at))
    }

    /// The screen name and owning file of a route's `component`.
    fn component(ctx: &mut Ctx, v: &Item) -> Option<(String, Option<String>)> {
        let (app, file, text) = (ctx.app, ctx.file, ctx.text);
        let source = v.source(text);
        let owner = |spec: &str| app.module(file, spec).map(|(f, _)| f.to_string());
        if is_name(&source) {
            let spec = imports(text)
                .into_iter()
                .find(|i| i.local == source)
                .map(|i| i.module);
            let owner = spec.and_then(|s| owner(&s));
            return Some((source, owner));
        }
        if let Some(i) = source.find("import(")
            && let Value::Str(spec) = (Parser {
                s: text,
                i: v.at + i + "import(".len(),
            })
            .value(0)
            .value
        {
            let stem = spec.rsplit('/').next().unwrap_or(&spec);
            let stem = stem.split('.').next().unwrap_or(stem).to_string();
            return Some((stem, owner(&spec)));
        }
        let message = format!(
            "Route component is neither an imported name nor () => import('…'); the screen is unknown: {source}"
        );
        ctx.diag(v.at, message);
        None
    }

    fn field<'i>(fields: &'i [(String, Item)], key: &str) -> Option<&'i Item> {
        fields.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// The route's own path joined to its parent's, without outer slashes.
    fn full_path(prefix: &str, path: &str) -> String {
        if path.starts_with('/') {
            return path.trim_matches('/').to_string();
        }
        [prefix, path.trim_matches('/')]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("/")
    }

    /// The inherited conditions plus the route's `meta`; `beforeEnter` is a note.
    fn access(ctx: &mut Ctx, fields: &[(String, Item)], inherited: &[Condition]) -> Vec<Condition> {
        let text = ctx.text;
        let mut conditions = inherited.to_vec();
        match field(fields, "meta") {
            Some(Item {
                value: Value::Obj(meta),
                ..
            }) => conditions.extend(meta_access(text, meta, false, ctx.file, ctx.out)),
            Some(v) => {
                let source = v.source(text);
                let message = format!(
                    "Route meta is not an object literal; the screen's access is unknown: {source}"
                );
                ctx.diag(v.at, message);
                conditions.push(Condition::Unknown(source));
            }
            None => {}
        }
        if let Some(v) = field(fields, "beforeEnter") {
            let message = "Route guard beforeEnter is not evaluated; what it checks is unknown.";
            note(ctx.out, ctx.file, line_of(text, v.at), message);
        }
        conditions
    }

    fn children<'i>(ctx: &mut Ctx, fields: &'i [(String, Item)]) -> Vec<&'i Item> {
        match field(fields, "children") {
            Some(Item {
                value: Value::Arr(items),
                ..
            }) => items.iter().collect(),
            Some(v) => {
                let message = format!(
                    "Child routes are not a literal array; they are unknown: {}",
                    v.source(ctx.text)
                );
                ctx.diag(v.at, message);
                vec![]
            }
            None => vec![],
        }
    }

    /// A child with the path `''` makes its parent a layout.
    fn has_empty_path(child: &Item) -> bool {
        let Value::Obj(fields) = &child.value else {
            return false;
        };
        matches!(field(fields, "path"), Some(Item { value: Value::Str(p), .. }) if p.is_empty())
    }

    /// The screen of a route with a component, unless it is a layout.
    fn push_screen(
        ctx: &mut Ctx,
        item: &Item,
        fields: &[(String, Item)],
        full: &str,
        conditions: &[Condition],
        children: &[&Item],
    ) {
        let text = ctx.text;
        if let Some(v) = field(fields, "components") {
            let message = format!(
                "Named views (components) are not read; the screen is unknown: {}",
                v.source(text)
            );
            ctx.diag(v.at, message);
            return;
        }
        let Some(v) = field(fields, "component") else {
            if field(fields, "redirect").is_none() && children.is_empty() {
                let message = format!(
                    "Route has no component, redirect or children; the screen is unknown: {}",
                    item.source(text)
                );
                ctx.diag(item.at, message);
            }
            return;
        };
        let Some((name, owner)) = component(ctx, v) else {
            return;
        };
        if children.iter().any(|c| has_empty_path(c)) {
            return;
        }
        let mut screen = ctx
            .app
            .screen(format!("/{full}"), name, ctx.file, line_of(text, item.at));
        screen.access = archguard_core::AccessState::from_parts(all(conditions.to_vec()), false);
        let name = match field(fields, "name").map(|n| &n.value) {
            Some(Value::Str(n)) => Some(n.clone()),
            _ => None,
        };
        ctx.routes.push(Route {
            screen,
            owner,
            name,
        });
    }

    fn walk(ctx: &mut Ctx, item: &Item, prefix: &str, inherited: &[Condition], depth: usize) {
        let text = ctx.text;
        if depth >= MAX_DEPTH {
            let message = "Routes nest too deep; deeper routes are unknown.";
            return ctx.diag(item.at, message.into());
        }
        let Value::Obj(fields) = &item.value else {
            let message = format!(
                "Route entry is not an object literal; its routes are unknown: {}",
                item.source(text)
            );
            return ctx.diag(item.at, message);
        };
        let full = match field(fields, "path") {
            Some(Item {
                value: Value::Str(p),
                ..
            }) => full_path(prefix, p),
            other => {
                let source = other.unwrap_or(item).source(text);
                let message = format!(
                    "Route path is not a literal; the route and its children are unknown: {source}"
                );
                return ctx.diag(item.at, message);
            }
        };
        let conditions = access(ctx, fields, inherited);
        let children = children(ctx, fields);
        push_screen(ctx, item, fields, &full, &conditions, &children);
        for child in children {
            walk(ctx, child, &full, &conditions, depth + 1);
        }
    }
}

/// Click and submit handlers: which code runs on which element.
mod trigger {
    use super::*;

    /// Code from `start` to `end` runs when the element labeled `label` fires.
    pub(super) struct Span {
        pub(super) start: usize,
        pub(super) end: usize,
        pub(super) label: String,
    }

    pub(super) fn spans(text: &str) -> Vec<Span> {
        let mut out = Vec::new();
        for (pos, name) in template_tags(text) {
            let tag = tag(text, pos + 1 + name.len());
            for attr in &tag.attrs {
                let event = (attr.name.strip_prefix('@'))
                    .or_else(|| attr.name.strip_prefix("v-on:"))
                    .and_then(|e| e.split('.').next());
                let (Some(event), AttrValue::Str(expr, at)) = (event, &attr.value) else {
                    continue;
                };
                if !TRIGGER_EVENTS.contains(&event) {
                    continue;
                }
                let Some(label) = label(text, &tag) else {
                    continue;
                };
                let call = expr.trim();
                let handler = ident_at(call, 0);
                let after = call[handler.len()..].trim_start();
                let body = (!handler.is_empty() && (after.is_empty() || after.starts_with('(')))
                    .then(|| body(text, handler))
                    .flatten();
                for (start, end) in [Some((*at, at + expr.len())), body].into_iter().flatten() {
                    out.push(Span {
                        start,
                        end,
                        label: label.clone(),
                    });
                }
            }
        }
        out
    }

    /// The body of the function `name` declares: `function name(…) {…}`, a method
    /// `name(…) {…}`, `name = (…) => …` or `name: function (…) {…}`.
    fn body(text: &str, name: &str) -> Option<(usize, usize)> {
        let b = text.as_bytes();
        words(text, name)
            .into_iter()
            .filter(|&p| !text[..p].ends_with('.'))
            .find_map(|pos| {
                let j = skip_ws(b, pos + name.len());
                match b.get(j)? {
                    b'(' => block_at(text, skip_ws(b, close(text, j))),
                    b'=' if b.get(j + 1) != Some(&b'=') => function_value(text, j + 1),
                    b':' => function_value(text, j + 1),
                    _ => None,
                }
            })
    }

    /// The `{…}` at `j`, after an optional `: Type` return annotation.
    fn block_at(text: &str, j: usize) -> Option<(usize, usize)> {
        let j = if text[j..].starts_with(':') {
            j + text[j..].find('{')?
        } else {
            j
        };
        text[j..].starts_with('{').then(|| (j, close(text, j)))
    }

    /// The body of the function expression at `j`: `async`, `function`, parameters, `=>`.
    fn function_value(text: &str, j: usize) -> Option<(usize, usize)> {
        let b = text.as_bytes();
        let mut j = skip_ws(b, j);
        if ident_at(text, j) == "async" {
            j = skip_ws(b, j + "async".len());
        }
        if ident_at(text, j) == "function" {
            j = skip_ws(b, j + "function".len());
            j = skip_ws(b, j + ident_at(text, j).len());
            return (b.get(j) == Some(&b'('))
                .then(|| block_at(text, skip_ws(b, close(text, j))))
                .flatten();
        }
        j = match b.get(j) {
            Some(b'(') => close(text, j),
            _ => match ident_at(text, j) {
                "" => return None,
                param => j + param.len(),
            },
        };
        j = skip_ws(b, j);
        if text[j..].starts_with(':') {
            j += text[j..].find("=>")?;
        }
        j = skip_ws(b, j + text[j..].strip_prefix("=>").map(|_| 2)?);
        if text[j..].starts_with('{') {
            return Some((j, close(text, j)));
        }
        Some((j, text[j..].find('\n').map_or(text.len(), |e| j + e)))
    }
}

/// Links and navigation calls.
mod nav {
    use super::*;

    enum Goal {
        Path(String),
        Name(String),
        Unknown,
    }

    struct Nav {
        at: usize,
        source: String,
        goal: Goal,
        /// A link's own label; calls take theirs from the trigger spans.
        label: Option<String>,
    }

    /// A literal, a template literal, `{ path: '…' }` or `{ name: '…' }`.
    fn goal(value: &Value) -> Goal {
        let Value::Obj(fields) = value else {
            return value.literal().map_or(Goal::Unknown, Goal::Path);
        };
        let get = |key: &str| fields.iter().find(|(k, _)| k == key).map(|(_, v)| &v.value);
        if let Some(path) = get("path").and_then(Value::literal) {
            Goal::Path(path)
        } else if let Some(Value::Str(name)) = get("name") {
            Goal::Name(name.clone())
        } else {
            Goal::Unknown
        }
    }

    fn links(text: &str) -> Vec<Nav> {
        let mut out = Vec::new();
        for (pos, name) in template_tags(text) {
            if !LINK_TAGS.contains(&name) {
                continue;
            }
            let tag = tag(text, pos + 1 + name.len());
            for attr in &tag.attrs {
                let AttrValue::Str(value, _) = &attr.value else {
                    continue;
                };
                let goal = match attr.name.as_str() {
                    "to" => Goal::Path(value.clone()),
                    ":to" | "v-bind:to" => goal(&Parser { s: value, i: 0 }.value(0).value),
                    _ => continue,
                };
                out.push(Nav {
                    at: pos,
                    source: value.trim().to_string(),
                    goal,
                    label: label(text, &tag),
                });
            }
        }
        out
    }

    /// Router objects and navigating functions of a file.
    fn navigators(app: &App, file: &str, text: &str) -> (BTreeSet<String>, BTreeSet<String>) {
        let mut objs: BTreeSet<String> = BTreeSet::from(["$router".to_string()]);
        let mut fns = BTreeSet::new();
        if app.nuxt {
            fns.insert("navigateTo".to_string());
        }
        for binding in bindings(text, "useRouter") {
            match binding {
                Binding::Name(n) => {
                    objs.insert(n);
                }
                Binding::Destructure(pairs) => fns.extend(
                    pairs
                        .into_iter()
                        .filter(|(k, _)| k == "push" || k == "replace")
                        .map(|(_, local)| local),
                ),
            }
        }
        for (pos, _) in router::calls(text) {
            if let Some(lhs) = text[..pos].trim_end().strip_suffix('=')
                && let Some(name) = trailing_ident(lhs.trim_end())
            {
                objs.insert(name);
            }
        }
        for i in imports(text) {
            if i.imported.is_none()
                && app
                    .module(file, &i.module)
                    .is_some_and(|(_, m)| !router::calls(m).is_empty())
            {
                objs.insert(i.local);
            }
        }
        (fns, objs)
    }

    /// `router.push|replace(…)`, `push(…)`, `navigateTo(…)`.
    fn calls(app: &App, file: &str, text: &str) -> Vec<Nav> {
        let (fns, objs) = navigators(app, file, text);
        let b = text.as_bytes();
        let mut opens = Vec::new();
        for f in &fns {
            for pos in words(text, f) {
                let j = skip_ws(b, pos + f.len());
                let declared = text[..pos].trim_end().ends_with("function");
                if b.get(j) == Some(&b'(') && !text[..pos].ends_with('.') && !declared {
                    opens.push((pos, j));
                }
            }
        }
        for o in &objs {
            for pos in words(text, o) {
                let rest = &text[pos + o.len()..];
                let Some(m) = [".push", ".replace"]
                    .into_iter()
                    .find(|m| rest.starts_with(m))
                else {
                    continue;
                };
                let j = skip_ws(b, pos + o.len() + m.len());
                if b.get(j) == Some(&b'(') {
                    opens.push((pos, j));
                }
            }
        }
        opens
            .into_iter()
            .map(|(pos, open)| {
                let item = Parser {
                    s: text,
                    i: open + 1,
                }
                .value(0);
                Nav {
                    at: pos,
                    source: item.source(text),
                    goal: goal(&item.value),
                    label: None,
                }
            })
            .collect()
    }

    fn external(path: &str) -> bool {
        path.is_empty()
            || path.contains("://")
            || ["//", "mailto:", "tel:"]
                .iter()
                .any(|p| path.starts_with(p))
    }

    /// The screen an absolute path resolves to, else why it does not.
    fn by_path(app: &App, routes: &[Route], path: &str) -> Result<Target, String> {
        let candidates = routes
            .iter()
            .filter_map(|r| Some((&r.screen.id, r.screen.route.as_deref()?)));
        match best_route(path, candidates) {
            Ok(id) => Ok(Target::Screen(id.clone())),
            Err(tied) if tied.is_empty() => Err(format!(
                "Navigation target {path} matches no route of application {}.",
                app.name
            )),
            Err(tied) => Err(format!(
                "Navigation target {path} matches several routes equally: {}.",
                tied.iter()
                    .map(|id| id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    /// The screen a route name names, else why it does not.
    fn by_name(app: &App, routes: &[Route], name: &str) -> Result<Target, String> {
        let ids: Vec<&String> = routes
            .iter()
            .filter(|r| r.name.as_deref() == Some(name))
            .map(|r| &r.screen.id)
            .collect();
        match ids.as_slice() {
            [id] => Ok(Target::Screen((*id).clone())),
            [] => Err(format!(
                "Route name {name} names no screen of application {}.",
                app.name
            )),
            _ => Err(format!(
                "Route name {name} names several screens: {}.",
                ids.iter()
                    .map(|id| id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    /// The target screen; `None` for an external URL or an anchor.
    fn resolve(
        app: &App,
        routes: &[Route],
        at: &Site,
        nav: &Nav,
        out: &mut Vec<UiFlowDiagnostic>,
    ) -> Option<Target> {
        let found = match &nav.goal {
            Goal::Path(path) => {
                let path = path.split(['?', '#']).next().unwrap_or("");
                if external(path) {
                    return None;
                }
                if path.starts_with('/') {
                    by_path(app, routes, path)
                } else {
                    Err(format!(
                        "Relative navigation {} depends on the current URL; its target is unknown.",
                        nav.source
                    ))
                }
            }
            Goal::Name(name) => by_name(app, routes, name),
            Goal::Unknown => Err(format!(
                "Navigation target is not a literal: {}",
                nav.source
            )),
        };
        Some(found.unwrap_or_else(|message| {
            diag(out, at.file, line_of(at.text, nav.at), message);
            Target::Unresolved(nav.source.clone())
        }))
    }

    pub(super) fn transitions(
        app: &App,
        routes: &[Route],
        at: &Site,
        vue: bool,
        flow: &mut UiFlow,
    ) {
        let mut navs = if vue { links(at.text) } else { vec![] };
        navs.extend(calls(app, at.file, at.text));
        for nav in &navs {
            let Some(target) = resolve(app, routes, at, nav, &mut flow.diagnostics) else {
                continue;
            };
            let triggers = match &nav.label {
                Some(label) => vec![Some(label.clone())],
                None => at.triggers(nav.at),
            };
            for from in at.froms {
                for trigger in &triggers {
                    flow.transitions.push(Transition {
                        weak: false,
                        from: from.clone(),
                        target: target.clone(),
                        trigger: trigger.clone(),
                        file: at.file.into(),
                        line: line_of(at.text, nav.at),
                        condition: archguard_core::AccessState::Undeclared,
                        source: SOURCE.into(),
                    });
                }
            }
        }
    }
}

/// `fetch`, Nuxt fetch helpers, axios and axios instances.
mod http {
    use super::*;

    /// The call; an unknown one becomes `Call::Unresolved` and a diagnostic.
    fn record(
        found: Result<Call, String>,
        source: String,
        (file, text, pos): (&str, &str, usize),
        out: &mut Vec<UiFlowDiagnostic>,
    ) -> (usize, Call) {
        let call = found.unwrap_or_else(|why| {
            let message = format!("HTTP call {why}; the call is unknown: {source}");
            diag(out, file, line_of(text, pos), message);
            Call::Unresolved(source)
        });
        (pos, call)
    }

    pub(super) fn actions(app: &App, clients: &Clients, at: &Site, flow: &mut UiFlow) {
        let (file, text) = (at.file, at.text);
        let out = &mut flow.diagnostics;
        let mut calls = fetches(file, text, out);
        let module = |spec: &str| app.module(file, spec).map(|(m, _)| m);
        for (recv, base) in clients.receivers(file, text, module) {
            for pos in words(text, &recv) {
                let Some((method, url)) = client_call(text, pos, &recv) else {
                    continue;
                };
                let found = http_call(
                    Ok(method.to_ascii_uppercase()),
                    url.value.literal(),
                    base.as_ref(),
                );
                calls.push(record(found, url.source(text), (file, text, pos), out));
            }
        }
        for (pos, call) in &calls {
            for trigger in at.triggers(*pos) {
                for from in at.froms {
                    flow.actions.push(Action {
                        weak: false,
                        screen: from.clone(),
                        trigger: trigger.clone(),
                        call: call.clone(),
                        file: file.into(),
                        line: line_of(text, *pos),
                        condition: archguard_core::AccessState::Undeclared,
                        source: SOURCE.into(),
                    });
                }
            }
        }
    }

    /// `fetch(url, { method })` and the Nuxt helpers of [`FETCHES`].
    fn fetches(file: &str, text: &str, out: &mut Vec<UiFlowDiagnostic>) -> Vec<(usize, Call)> {
        let b = text.as_bytes();
        let mut calls = Vec::new();
        for name in FETCHES {
            for pos in words(text, name) {
                let open = skip_ws(b, pos + name.len());
                if b.get(open) != Some(&b'(') || !global_call(text, pos) {
                    continue;
                }
                let mut p = Parser {
                    s: text,
                    i: open + 1,
                };
                let url = p.value(0);
                p.ws();
                let init = (p.peek() == b',').then(|| {
                    p.i += 1;
                    p.value(0)
                });
                let found = http_call(fetch_method(text, init.as_ref()), url.value.literal(), None);
                calls.push(record(found, url.source(text), (file, text, pos), out));
            }
        }
        calls
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{
        AccessState, Call, Condition, DiagnosticSeverity, EndpointAccess, Origin, Target,
    };

    fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in files {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        dir
    }

    fn screens(flow: &UiFlow) -> Vec<(String, String, Option<String>, Option<Condition>)> {
        flow.screens
            .iter()
            .map(|s| {
                let route = s.route.clone();
                (
                    s.id.clone(),
                    s.name.clone(),
                    route,
                    s.access.condition().cloned(),
                )
            })
            .collect()
    }

    fn transitions(flow: &UiFlow) -> Vec<(Origin, Target, Option<String>)> {
        let mut got: Vec<_> = flow
            .transitions
            .iter()
            .map(|t| (t.from.clone(), t.target.clone(), t.trigger.clone()))
            .collect();
        got.sort();
        got
    }

    fn actions(flow: &UiFlow) -> Vec<(Origin, Call, Option<String>)> {
        let mut got: Vec<_> = flow
            .actions
            .iter()
            .map(|a| (a.screen.clone(), a.call.clone(), a.trigger.clone()))
            .collect();
        got.sort();
        got
    }

    fn page(id: &str) -> Origin {
        Origin::Screen(id.into())
    }

    fn any(app: &str) -> Origin {
        Origin::AnyScreen {
            application: app.into(),
        }
    }

    fn to(id: &str) -> Target {
        Target::Screen(id.into())
    }

    fn http(method: &str, path: &str) -> Call {
        Call::Http {
            method: method.into(),
            path: path.into(),
        }
    }

    fn label(text: &str) -> Option<String> {
        Some(text.into())
    }

    fn gaps(flow: &UiFlow) -> Vec<(String, u32)> {
        flow.diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Gap)
            .map(|d| (d.file.clone(), d.line))
            .collect()
    }

    fn role(name: &str) -> Condition {
        Condition::Role(name.into())
    }

    const ROUTER_PKG: &str = r#"{"dependencies":{"vue":"^3.5.0","vue-router":"^4.4.0"}}"#;

    const ROUTER: &str = "import { createRouter, createWebHistory } from 'vue-router'
import Home from '../views/Home.vue'
import AccountLayout from '@/layouts/AccountLayout.vue'

const routes: RouteRecordRaw[] = [
  { path: '/', name: 'home', component: Home },
  { path: '/about', name: 'about', component: () => import('../views/About.vue') },
  { path: '/old', redirect: '/about' },
  {
    path: '/account',
    component: AccountLayout,
    meta: { requiresAuth: true },
    children: [
      { path: '', name: 'account', component: () => import('../views/Account.vue') },
      {
        path: 'orders/:id',
        name: 'order',
        component: () => import('../views/Order.vue'),
        meta: { roles: ['clerk', 'admin'] },
      },
    ],
  },
]

const router = createRouter({ history: createWebHistory(), routes })
router.beforeEach((to) => to.meta.requiresAuth && !signedIn() ? '/login' : true)
export default router
";

    const HOME: &str = "<script setup lang=\"ts\">
import { useRouter } from 'vue-router'
const router = useRouter()
const openOrder = (id: number) => {
  router.push({ name: 'order', params: { id } })
}
function goAbout() {
  router.push('/about')
}
</script>

<template>
  <router-link to=\"/about\" data-testid=\"about-link\">About us</router-link>
  <RouterLink :to=\"`/account/orders/${o.id}`\">Order</RouterLink>
  <RouterLink :to=\"{ name: 'account' }\">Account</RouterLink>
  <RouterLink :to=\"target\">Dyn</RouterLink>
  <RouterLink to=\"https://vuejs.org\">Docs</RouterLink>
  <a href=\"/about\">plain</a>
  <button aria-label=\"Open order\" @click=\"openOrder(o.id)\">Open</button>
  <button @click.prevent=\"goAbout\">About</button>
  <button @click=\"$router.push('/missing')\">Missing</button>
</template>
";

    const ABOUT: &str = "<script setup>
import axios from 'axios'
import api from '../api'
async function save() {
  await fetch('/api/orders', { method: 'POST' })
}
const load = async () => {
  await api.get(`/orders/${id}`)
}
axios.delete('/api/orders/1')
fetch(url)
</script>

<template>
  <form @submit.prevent=\"save\" data-testid=\"order-form\"><button>Save</button></form>
  <button v-on:click=\"load\">Reload</button>
</template>
";

    fn router_project() -> tempfile::TempDir {
        project(&[
            ("shop/package.json", ROUTER_PKG),
            ("shop/src/router/index.ts", ROUTER),
            ("shop/src/views/Home.vue", HOME),
            ("shop/src/views/About.vue", ABOUT),
            (
                "shop/src/views/Account.vue",
                "<template><p>Account</p></template>\n",
            ),
            (
                "shop/src/views/Order.vue",
                "<template><p>Order</p></template>\n",
            ),
            (
                "shop/src/layouts/AccountLayout.vue",
                "<template><RouterLink to=\"/account\">Overview</RouterLink><RouterView /></template>\n",
            ),
            (
                "shop/src/App.vue",
                "<template><nav><RouterLink to=\"/\" aria-label=\"Start\">Home</RouterLink></nav></template>\n",
            ),
            (
                "shop/src/api.ts",
                "import axios from 'axios'\nexport default axios.create({ baseURL: '/api' })\n",
            ),
            (
                "shop/tests/unit/router.spec.ts",
                "createRouter({ routes: [{ path: '/hidden', component: Home }] })",
            ),
            ("shop/node_modules/x/package.json", ROUTER_PKG),
            (
                "shop/node_modules/x/router.ts",
                "createRouter({ routes: [{ path: '/vendored', component: Home }] })",
            ),
            (
                "admin/package.json",
                r#"{"dependencies":{"vue-router":"4.4.0"}}"#,
            ),
            (
                "admin/src/router.ts",
                "import { routes } from './routes'\nexport default createRouter({ routes })\n",
            ),
            (
                "admin/src/routes.ts",
                "import Dash from './Dash.vue'\nconst routes = [{ path: '/', component: Dash }]\nexport { routes }\n",
            ),
        ])
    }

    #[test]
    fn vue_router_routes_children_meta_and_imported_arrays() {
        let dir = router_project();
        let flow = extract(dir.path());
        let account = Some(Condition::Authenticated);
        let order = Some(Condition::All(vec![
            Condition::Authenticated,
            Condition::Any(vec![role("clerk"), role("admin")]),
        ]));
        assert_eq!(
            screens(&flow),
            vec![
                ("admin:/".into(), "Dash".into(), Some("/".into()), None),
                ("shop:/".into(), "Home".into(), Some("/".into()), None),
                (
                    "shop:/about".into(),
                    "About".into(),
                    Some("/about".into()),
                    None
                ),
                (
                    "shop:/account".into(),
                    "Account".into(),
                    Some("/account".into()),
                    account
                ),
                (
                    "shop:/account/orders/:id".into(),
                    "Order".into(),
                    Some("/account/orders/:id".into()),
                    order
                ),
            ]
        );
        let home = &flow.screens[1];
        assert_eq!(
            (home.file.as_str(), home.line, home.source.as_str()),
            ("shop/src/router/index.ts", 6, "vue")
        );
        assert_eq!(home.application.as_deref(), Some("shop"));
        assert_eq!(flow.screens[0].file, "admin/src/routes.ts");
        // beforeEach is a note, not a gap.
        let note = flow
            .diagnostics
            .iter()
            .find(|d| d.message.contains("beforeEach"))
            .expect("beforeEach note");
        assert_eq!(
            (note.file.as_str(), note.line, note.severity),
            ("shop/src/router/index.ts", 26, DiagnosticSeverity::Note)
        );
    }

    #[test]
    fn links_pushes_named_routes_and_click_triggers() {
        let dir = router_project();
        let flow = extract(dir.path());
        let home = || page("shop:/");
        let unresolved = |s: &str| Target::Unresolved(s.into());
        assert_eq!(
            transitions(&flow),
            vec![
                (home(), to("shop:/about"), label("About")),
                (home(), to("shop:/about"), label("about-link")),
                (home(), to("shop:/account"), label("Account")),
                (home(), to("shop:/account/orders/:id"), label("Open order")),
                (home(), to("shop:/account/orders/:id"), label("Order")),
                (home(), unresolved("'/missing'"), label("Missing")),
                (home(), unresolved("target"), label("Dyn")),
                (any("shop"), to("shop:/"), label("Start")),
                (any("shop"), to("shop:/account"), label("Overview")),
            ]
        );
        let home_file = "shop/src/views/Home.vue";
        let home_gaps: Vec<_> = gaps(&flow)
            .into_iter()
            .filter(|g| g.0 == home_file)
            .collect();
        assert_eq!(
            home_gaps,
            vec![(home_file.into(), 16), (home_file.into(), 21)],
            "{:#?}",
            flow.diagnostics
        );
        assert!(!flow.complete);
    }

    #[test]
    fn fetch_axios_and_submit_triggers_become_actions() {
        let dir = router_project();
        let flow = extract(dir.path());
        let about = || page("shop:/about");
        assert_eq!(
            actions(&flow),
            vec![
                (about(), http("DELETE", "/api/orders/1"), None),
                (about(), http("GET", "/api/orders/:param"), label("Reload")),
                (about(), http("POST", "/api/orders"), label("order-form")),
                (about(), Call::Unresolved("url".into()), None),
            ]
        );
        let line = flow
            .actions
            .iter()
            .find(|a| a.call == http("POST", "/api/orders"))
            .map(|a| (a.file.as_str(), a.line, a.source.as_str()));
        assert_eq!(line, Some(("shop/src/views/About.vue", 5, "vue")));
        assert!(gaps(&flow).contains(&("shop/src/views/About.vue".into(), 11)));
    }

    #[test]
    fn nuxt_pages_meta_server_routes_and_layouts() {
        let dir = project(&[
            (
                "web/package.json",
                r#"{"devDependencies":{"nuxt":"^3.13.0"}}"#,
            ),
            (
                "web/pages/index.vue",
                "<script setup>\nconst go = () => navigateTo('/cart')\n</script>\n<template>\n  <NuxtLink to=\"/products/42\">Product</NuxtLink>\n  <button @click=\"go\">Cart</button>\n</template>\n",
            ),
            (
                "web/pages/products/[id].vue",
                "<script setup lang=\"ts\">\ndefinePageMeta({ middleware: 'auth' })\nconst { data } = await useFetch(`/api/products/${id}`)\n</script>\n",
            ),
            (
                "web/pages/cart.vue",
                "<script setup>\ndefinePageMeta({\n  middleware: ['auth', 'admin'],\n})\nconst router = useRouter()\nrouter.push({ name: 'products-id', params: { id: 1 } })\nawait $fetch('/api/orders', { method: 'POST' })\n</script>\n",
            ),
            ("web/pages/docs/[...slug].vue", "<template />\n"),
            ("web/pages/(marketing)/about.vue", "<template />\n"),
            (
                "web/pages/account.vue",
                "<template><NuxtPage /></template>\n",
            ),
            ("web/pages/account/index.vue", "<template />\n"),
            (
                "web/pages/account/settings.vue",
                "<script setup>definePageMeta({ requiresAuth: true, roles: 'admin' })</script>\n",
            ),
            (
                "web/layouts/default.vue",
                "<template><NuxtLink to=\"/cart\">Cart</NuxtLink><slot /></template>\n",
            ),
            (
                "web/server/api/products/[id].get.ts",
                "export default defineEventHandler(() => ({}))\n",
            ),
            (
                "web/server/api/orders.post.ts",
                "\nexport default defineEventHandler(() => ({}))\n",
            ),
            (
                "web/server/routes/health.ts",
                "export default defineEventHandler(() => 'ok')\n",
            ),
            ("web/server/utils/db.ts", "export const db = 1\n"),
            (
                "web/middleware/auth.global.ts",
                "export default defineNuxtRouteMiddleware(() => {})\n",
            ),
            ("web/.nuxt/pages/hidden.vue", "<template />\n"),
            ("web/.output/server/api/x.get.ts", "export default 1\n"),
        ]);
        let flow = extract(dir.path());
        let custom = |n: &str| Condition::Custom(n.into());
        let s = |id: &str, file: &str, route: &str, access: Option<Condition>| {
            (
                id.to_string(),
                file.to_string(),
                Some(route.to_string()),
                access,
            )
        };
        assert_eq!(
            screens(&flow),
            vec![
                s("web:/", "web/pages/index.vue", "/", None),
                s(
                    "web:/about",
                    "web/pages/(marketing)/about.vue",
                    "/about",
                    None
                ),
                s(
                    "web:/account",
                    "web/pages/account/index.vue",
                    "/account",
                    None
                ),
                s(
                    "web:/account/settings",
                    "web/pages/account/settings.vue",
                    "/account/settings",
                    Some(Condition::All(vec![
                        Condition::Authenticated,
                        role("admin")
                    ]))
                ),
                s(
                    "web:/cart",
                    "web/pages/cart.vue",
                    "/cart",
                    Some(Condition::All(vec![custom("auth"), custom("admin")]))
                ),
                s(
                    "web:/docs/*",
                    "web/pages/docs/[...slug].vue",
                    "/docs/*",
                    None
                ),
                s(
                    "web:/products/:id",
                    "web/pages/products/[id].vue",
                    "/products/:id",
                    Some(custom("auth"))
                ),
            ]
        );
        let cart = flow.screens.iter().find(|s| s.id == "web:/cart").unwrap();
        assert_eq!(cart.line, 2);
        assert_eq!(
            transitions(&flow),
            vec![
                (page("web:/"), to("web:/cart"), label("Cart")),
                (page("web:/"), to("web:/products/:id"), label("Product")),
                (page("web:/cart"), to("web:/products/:id"), None),
                (any("web"), to("web:/cart"), label("Cart")),
            ]
        );
        assert_eq!(
            actions(&flow),
            vec![
                (page("web:/cart"), http("POST", "/api/orders"), None),
                (
                    page("web:/products/:id"),
                    http("GET", "/api/products/:param"),
                    None
                ),
            ]
        );
        let endpoint = |method: &str, path: &str, file: &str, line: u32| EndpointAccess {
            method: method.into(),
            path: path.into(),
            access: AccessState::Undeclared,
            application: Some("web".into()),
            file: file.into(),
            line,
            source: "vue".into(),
            handler: None,
            sends: vec![],
        };
        let mut got = flow.endpoints.clone();
        got.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(
            got,
            vec![
                endpoint("POST", "/api/orders", "web/server/api/orders.post.ts", 2),
                endpoint(
                    "GET",
                    "/api/products/:id",
                    "web/server/api/products/[id].get.ts",
                    1
                ),
                endpoint("ANY", "/health", "web/server/routes/health.ts", 1),
            ]
        );
        let global = flow
            .diagnostics
            .iter()
            .find(|d| d.file == "web/middleware/auth.global.ts")
            .expect("global middleware note");
        assert_eq!(global.severity, DiagnosticSeverity::Note);
        assert!(flow.complete, "{:#?}", flow.diagnostics);
    }

    #[test]
    fn non_literal_routes_meta_and_targets_are_gaps() {
        let dir = project(&[
            ("app/package.json", ROUTER_PKG),
            (
                "app/src/router.ts",
                "import Home from './Home.vue'
export default createRouter({
  routes: [
    ...adminRoutes,
    { path: dynamicPath, component: Home },
    { path: '/x', component: makeView('x') },
    { path: '/y', component: Home, meta: { roles: ROLES.admin } },
    { path: '/z', components: { default: Home } },
  ],
})
",
            ),
            (
                "app/src/Home.vue",
                "<script setup>\nconst router = useRouter()\nrouter.push('edit')\n</script>\n",
            ),
        ]);
        let flow = extract(dir.path());
        let file = |line: u32| ("app/src/router.ts".to_string(), line);
        assert_eq!(
            gaps(&flow),
            vec![
                ("app/src/Home.vue".into(), 3),
                file(4),
                file(5),
                file(6),
                file(7),
                file(8)
            ],
            "{:#?}",
            flow.diagnostics
        );
        assert_eq!(
            screens(&flow),
            vec![(
                "app:/y".into(),
                "Home".into(),
                Some("/y".into()),
                Some(Condition::Unknown("ROLES.admin".into()))
            )]
        );
        assert!(!flow.complete);
    }

    #[test]
    fn no_application_without_vue_router_or_nuxt() {
        let dir = project(&[
            ("package.json", r#"{"dependencies":{"vue":"^3.5.0"}}"#),
            (
                "src/App.vue",
                "<template><RouterLink to=\"/\">x</RouterLink></template>",
            ),
            ("src/api.ts", "fetch('/api/x')"),
        ]);
        let flow = extract(dir.path());
        assert_eq!(flow, UiFlow::default());
    }
}
