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
//! Access, as screen conditions; the conditions of `beforeEnter` and Nuxt `middleware`
//! of a route and its parents are `all`ed. Route `meta` and `definePageMeta` keys carry
//! no meaning of their own: they count only through a guard that reads them.
//! - `router.beforeEach(…)` / `beforeResolve(…)` (any file of the application) and Nuxt
//!   `middleware/*.global.*`: a function whose statements are `if (<a && b …>) return
//!   <target>` / `next(<target>)` is read. A conjunct is `to.meta.K`, `!to.meta.K`,
//!   `to.matched.some(r => r.meta.K)`, `to.path|name === 'x'` / `!==`, `<literal list>
//!   .includes(to.path|name)` or a check name (`store.user`, `auth.isLoggedIn()`). Routes
//!   the selectors pick get the negated checks as `custom(name)`; a route an opt-out
//!   selector excludes is `open`; one a plain selector does not pick is undeclared. A
//!   guard in any other shape makes every screen `unknown` and is a gap. `to.meta` takes
//!   the nearest record, `matched.some` any record (child overrides parent per key).
//! - `beforeEnter` (name or array of names → `custom`; anything else `unknown`, a gap),
//!   `beforeRouteEnter` in the route component (`unknown`, a gap).
//! - Nuxt `definePageMeta({ middleware: 'auth' })` or `['auth', 'admin']`: `custom(name)`,
//!   also from parent pages (`pages/users.vue` for `pages/users/*.vue`).
//! - `@sidebase/nuxt-auth`, read from `nuxt.config.{ts,js}` (`modules`, `auth:
//!   { globalAppMiddleware }`): the global default is `authenticated`; page `auth: false`
//!   is `open` (it replaces only that default), `true` is `authenticated`, `{
//!   unauthenticatedOnly }` is guest-only. Without the module the key is inert.
//! - A meta flag or list no guard reads is a note. Not evaluated: `addRoute`, Nuxt
//!   `addRouteMiddleware`, `routeRules`, `pages:extend`, layers, Nuxt 2.
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
    path_patterns_match,
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
const SIDEBASE: &str = "@sidebase/nuxt-auth";
/// Meta keys the router itself reads; never reported as unread.
const BUILTIN_META: [&str; 1] = ["alias"];
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
    /// The `meta` of every matched record, root first.
    chain: Vec<Rec>,
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
        }
        for (file, text) in &self.files {
            router::routes(self, file, text, &mut routes, out);
        }
        guard::apply(self, &mut routes, out);
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

/// Whether a binding's initializer awaits or calls anything; `useUserSession()` is the
/// one call read (as `loggedIn`).
fn calls(binding: &str) -> bool {
    let init = binding.split_once('=').map_or("", |(_, r)| r).trim();
    init != "useUserSession()" && (init.contains('(') || !words(init, "await").is_empty())
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

/// A meta value as far as a guard can test it.
#[derive(Clone)]
enum Lit {
    Bool(bool),
    /// A non-empty string or a number.
    Truthy,
    Falsy,
    List,
    Obj(Vec<(String, Lit)>),
    Unknown(String),
}

impl Lit {
    fn of(text: &str, v: &Item) -> Lit {
        match &v.value {
            Value::Str(s) if s.is_empty() => Lit::Falsy,
            Value::Str(_) => Lit::Truthy,
            Value::Arr(_) => Lit::List,
            Value::Obj(f) => Lit::Obj(
                f.iter()
                    .map(|(k, i)| (k.clone(), Lit::of(text, i)))
                    .collect(),
            ),
            Value::Raw(r) => match r.as_str() {
                "true" => Lit::Bool(true),
                "false" => Lit::Bool(false),
                "null" | "undefined" | "0" => Lit::Falsy,
                _ => Lit::Unknown(v.source(text)),
            },
            Value::Tpl(_) => Lit::Unknown(v.source(text)),
        }
    }

    /// `Err` holds the source of a value that cannot be tested.
    fn truthy(&self) -> Result<bool, String> {
        match self {
            Lit::Bool(b) => Ok(*b),
            Lit::Truthy | Lit::List | Lit::Obj(_) => Ok(true),
            Lit::Falsy => Ok(false),
            Lit::Unknown(s) => Err(s.clone()),
        }
    }
}

/// The meta fields of one route record (or Nuxt page), in source order; a spread is the
/// field `""` and hides every key before it.
#[derive(Clone, Default)]
struct Rec {
    fields: Vec<(String, Lit)>,
    file: String,
    line: u32,
}

impl Rec {
    fn get(&self, key: &str) -> Option<&Lit> {
        self.fields
            .iter()
            .rev()
            .find(|(k, _)| k == key || k.is_empty())
            .map(|(_, l)| l)
    }
}

/// The nearest record that sets `key` (`to.meta.key`).
fn nearest<'r>(chain: &'r [Rec], key: &str) -> Option<&'r Lit> {
    chain.iter().rev().find_map(|r| r.get(key))
}

/// Nuxt `middleware` conditions and the meta record of a route `meta` or
/// `definePageMeta` object. A non-literal middleware is `unknown` and a diagnostic.
fn meta_access(
    text: &str,
    fields: &[(String, Item)],
    nuxt: bool,
    file: &str,
    line: u32,
    out: &mut Vec<UiFlowDiagnostic>,
) -> (Vec<Condition>, Rec) {
    let mut found = Vec::new();
    let mut rec = Rec {
        file: file.into(),
        line,
        ..Rec::default()
    };
    for (key, v) in fields {
        if nuxt && (key == "middleware" || key.is_empty()) {
            let names = if key.is_empty() {
                None
            } else {
                strings(&v.value)
            };
            match names {
                Some(names) => found.extend(names.into_iter().map(Condition::Custom)),
                None => {
                    let source = v.source(text);
                    let message = format!(
                        "Page middleware is not a literal; the page's access is unknown: {source}"
                    );
                    diag(out, file, line_of(text, v.at), message);
                    found.push(Condition::Unknown(source));
                }
            }
        }
        if key != "middleware" || !nuxt {
            let lit = if key.is_empty() {
                Lit::Unknown(v.source(text))
            } else {
                Lit::of(text, v)
            };
            rec.fields.push((key.clone(), lit));
        }
    }
    (found, rec)
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

    /// Meta of every page file: line, `middleware` conditions and the meta record.
    type PageMeta = (u32, Vec<Condition>, Rec);

    pub(super) fn pages(app: &App, routes: &mut Vec<Route>, out: &mut Vec<UiFlowDiagnostic>) {
        let metas: BTreeMap<&str, PageMeta> = app
            .files
            .iter()
            .filter(|(f, _)| page_path(app, f).is_some())
            .map(|(f, t)| (f.as_str(), page_meta(f, t, out)))
            .collect();
        for file in app.files.keys() {
            let Some(path) = page_path(app, file) else {
                continue;
            };
            let layout = format!("{}/index.vue", file.trim_end_matches(".vue"));
            if app.files.contains_key(&layout) {
                continue;
            }
            let segments: Vec<&str> = path.split('/').collect();
            let root = &file[..file.len() - path.len() - ".vue".len()];
            // Pages above this one (`users.vue` for `users/[id].vue`) run their middleware too.
            let mut chain = Vec::new();
            let mut conditions = Vec::new();
            for n in 1..segments.len() {
                let parent = format!("{root}{}.vue", segments[..n].join("/"));
                if let Some((_, c, rec)) = metas.get(parent.as_str()) {
                    conditions.extend(c.iter().cloned());
                    chain.push(rec.clone());
                }
            }
            let (line, own, rec) = &metas[file.as_str()];
            conditions.extend(own.iter().cloned());
            chain.push(rec.clone());
            let (route, name) = route_of(&segments);
            let mut screen = app.screen(route, file.clone(), file, *line);
            screen.access = archguard_core::AccessState::from_parts(all(conditions), false);
            routes.push(Route {
                screen,
                owner: Some(file.clone()),
                name: Some(name),
                chain,
            });
        }
    }

    /// Line, `middleware` conditions and meta of `definePageMeta({ … })`; line 1 and
    /// nothing without one.
    fn page_meta(file: &str, text: &str, out: &mut Vec<UiFlowDiagnostic>) -> PageMeta {
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
            let (conditions, rec) = match &arg.value {
                Value::Obj(fields) => meta_access(text, fields, true, file, line, out),
                _ => {
                    let source = arg.source(text);
                    let message = format!(
                        "definePageMeta argument is not an object literal; the page's access is unknown: {source}"
                    );
                    diag(out, file, line, message);
                    let rec = Rec {
                        fields: vec![(String::new(), Lit::Unknown(source.clone()))],
                        file: file.into(),
                        line,
                    };
                    (vec![Condition::Unknown(source)], rec)
                }
            };
            return (line, conditions, rec);
        }
        let rec = Rec {
            file: file.into(),
            line: 1,
            ..Rec::default()
        };
        (1, vec![], rec)
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
                walk(&mut ctx, &it, "", (&[], &[]), 0);
            }
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

    /// The inherited conditions plus the route's `beforeEnter`, and the meta chain.
    fn access(
        ctx: &mut Ctx,
        fields: &[(String, Item)],
        inherited: &[Condition],
        chain: &[Rec],
    ) -> (Vec<Condition>, Vec<Rec>) {
        let text = ctx.text;
        let mut conditions = inherited.to_vec();
        let mut chain = chain.to_vec();
        let mut rec = Rec {
            file: ctx.file.into(),
            line: fields.first().map_or(1, |(_, v)| line_of(text, v.at)),
            ..Rec::default()
        };
        match field(fields, "meta") {
            Some(Item {
                value: Value::Obj(meta),
                at,
                ..
            }) => rec = meta_access(text, meta, false, ctx.file, line_of(text, *at), ctx.out).1,
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
        chain.push(rec);
        if let Some(v) = field(fields, "beforeEnter") {
            let named = |i: &Item| match &i.value {
                Value::Raw(n) if is_name(n) => Some(Condition::Custom(n.clone())),
                _ => None,
            };
            let read = match &v.value {
                Value::Arr(items) => items.iter().map(named).collect(),
                _ => named(v).map(|c| vec![c]),
            };
            match read {
                Some(found) => conditions.extend(found),
                None => {
                    let source = v.source(text);
                    let message = format!(
                        "Route guard beforeEnter is not a guard name or a list of names; what it checks is unknown: {source}"
                    );
                    ctx.diag(v.at, message);
                    conditions.push(Condition::Unknown(source));
                }
            }
        }
        (conditions, chain)
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
        (conditions, chain): (&[Condition], &[Rec]),
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
            chain: chain.to_vec(),
        });
    }

    fn walk(
        ctx: &mut Ctx,
        item: &Item,
        prefix: &str,
        (inherited, chain): (&[Condition], &[Rec]),
        depth: usize,
    ) {
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
        let (conditions, chain) = access(ctx, fields, inherited, chain);
        let children = children(ctx, fields);
        push_screen(ctx, item, fields, &full, (&conditions, &chain), &children);
        for child in children {
            walk(ctx, child, &full, (&conditions, &chain), depth + 1);
        }
    }
}

/// Guards that decide access: `beforeEach`/`beforeResolve`, Nuxt global middleware,
/// `beforeRouteEnter` and the sidebase module.
mod guard {
    use super::shape::{self, Conj};
    use super::sidebase::{Module, Presence};
    use super::*;

    struct Guard {
        file: String,
        line: u32,
        /// `None`: the guard is not in a readable shape.
        rules: Option<Vec<Vec<Conj>>>,
    }

    enum Tri {
        Yes,
        No,
        Unknown(String),
    }

    pub(super) fn apply(app: &App, routes: &mut [Route], out: &mut Vec<UiFlowDiagnostic>) {
        let guards = guards(app, out);
        let module = if app.nuxt {
            Module::read(app, out)
        } else {
            Module::default()
        };
        let (read, all_read) = read_keys(&guards, &module);
        for r in routes.iter_mut() {
            let mut layers = AccessState::Undeclared;
            for g in &guards {
                layers = layers.and(g.state(r, out));
            }
            layers = layers.and(module.state(r, out));
            layers = layers.and(before_route_enter(app, r, out));
            r.screen.access = std::mem::take(&mut r.screen.access).and(layers);
            if all_read {
                for rec in &r.chain {
                    unread(rec, &read, out);
                }
            }
        }
    }

    /// The meta keys some guard (or the auth module) reads, and whether every guard is
    /// readable.
    fn read_keys<'g>(guards: &'g [Guard], module: &Module) -> (BTreeSet<&'g str>, bool) {
        let mut read: BTreeSet<&str> = BTreeSet::new();
        let mut all_read = true;
        for g in guards {
            match &g.rules {
                Some(rules) => read.extend(rules.iter().flatten().filter_map(|c| match c {
                    Conj::Meta { key, .. } | Conj::Matched { key, .. } => Some(key.as_str()),
                    _ => None,
                })),
                None => all_read = false,
            }
        }
        if module.present != Presence::Absent {
            read.insert("auth");
        }
        (read, all_read)
    }

    /// A note for each flag or list of a record that no guard reads.
    fn unread(rec: &Rec, read: &BTreeSet<&str>, out: &mut Vec<UiFlowDiagnostic>) {
        for (key, lit) in &rec.fields {
            let flag = matches!(lit, Lit::Bool(_) | Lit::List);
            if flag && !read.contains(key.as_str()) && !BUILTIN_META.contains(&key.as_str()) {
                let message = format!("Route meta {key} is read by no guard; it grants no access.");
                note(out, &rec.file, rec.line, &message);
            }
        }
    }

    fn before_route_enter(app: &App, r: &Route, out: &mut Vec<UiFlowDiagnostic>) -> AccessState {
        let Some((file, text)) = r.owner.as_ref().and_then(|o| app.files.get_key_value(o)) else {
            return AccessState::Undeclared;
        };
        match words(text, "beforeRouteEnter").first() {
            Some(&at) => {
                let message =
                    "Component guard beforeRouteEnter is not evaluated; what it checks is unknown.";
                diag(out, file, line_of(text, at), message);
                AccessState::Protected(Condition::Unknown("beforeRouteEnter".into()))
            }
            None => AccessState::Undeclared,
        }
    }

    fn guards(app: &App, out: &mut Vec<UiFlowDiagnostic>) -> Vec<Guard> {
        let mut found = Vec::new();
        for (file, text) in &app.files {
            let mut sites: Vec<usize> = [".beforeEach(", ".beforeResolve("]
                .iter()
                .flat_map(|c| text.match_indices(c).map(|(i, _)| i + c.len()))
                .collect();
            if app.nuxt && global_middleware(app, file) {
                sites = words(text, "defineNuxtRouteMiddleware")
                    .into_iter()
                    .filter_map(|p| {
                        let open = skip_ws(text.as_bytes(), p + "defineNuxtRouteMiddleware".len());
                        (text.as_bytes().get(open) == Some(&b'(')).then_some(open + 1)
                    })
                    .collect();
                if sites.is_empty() {
                    sites.push(usize::MAX);
                }
            }
            for at in sites {
                let (line, source) = if at == usize::MAX {
                    (1, String::new())
                } else {
                    let arg = Parser { s: text, i: at }.value(0);
                    (line_of(text, at), arg.source(text))
                };
                let rules =
                    shape::read_fn(&source).and_then(|(tp, body)| shape::rules(body, tp, text));
                if rules.is_none() {
                    let message = "Navigation guard is not in a readable shape; what it checks on every route is unknown.";
                    diag(out, file, line, message);
                }
                found.push(Guard {
                    file: file.clone(),
                    line,
                    rules,
                });
            }
        }
        found
    }

    /// `middleware/*.global.*` of the application (Nuxt 4: below `app/`).
    fn global_middleware(app: &App, file: &str) -> bool {
        let inner = file.strip_prefix(app.prefix.as_str()).unwrap_or(file);
        let inner = inner.strip_prefix("app/").unwrap_or(inner);
        inner.starts_with("middleware/") && inner.contains(".global.")
    }

    impl Guard {
        fn state(&self, r: &Route, out: &mut Vec<UiFlowDiagnostic>) -> AccessState {
            let Some(rules) = &self.rules else {
                let c = Condition::Unknown("unreadable navigation guard".into());
                return AccessState::Protected(c);
            };
            let mut state = AccessState::Undeclared;
            for rule in rules {
                state = state.and(self.rule(rule, r, out));
            }
            state
        }

        fn rule(&self, rule: &[Conj], r: &Route, out: &mut Vec<UiFlowDiagnostic>) -> AccessState {
            let (mut checks, mut opted_out) = (Vec::new(), false);
            let mut unknown = None;
            for c in rule {
                let (tri, neg) = match c {
                    Conj::Check(req) => {
                        checks.push(req.clone());
                        continue;
                    }
                    Conj::Meta { key, neg } => (meta(nearest(&r.chain, key)), *neg),
                    Conj::Matched { key, neg } => (matched(&r.chain, key), *neg),
                    Conj::Route { by_name, lits, neg } => (route(r, *by_name, lits), *neg),
                };
                match (tri, neg) {
                    (Tri::Unknown(why), _) => unknown = Some(why),
                    // `!x` over a true `x`: an opt-out selector excludes the route.
                    (Tri::Yes, true) => opted_out = true,
                    // A plain selector that misses: the guard does not concern the route.
                    (Tri::No, false) => return AccessState::Undeclared,
                    _ => {}
                }
            }
            if let Some(why) = unknown {
                let message = format!(
                    "A guard tests a value that cannot be read; the screen's access is unknown: {why}"
                );
                diag(out, &self.file, self.line, message);
                return AccessState::Protected(Condition::Unknown(why));
            }
            if opted_out {
                return AccessState::Open;
            }
            let condition = match checks.len() {
                0 => {
                    let message = "A guard blocks selected routes without a check; what lets users in is unknown.";
                    diag(out, &self.file, self.line, message);
                    Condition::Unknown("guard without a check".into())
                }
                1 => checks.remove(0),
                _ => Condition::Any(checks),
            };
            AccessState::Protected(condition)
        }
    }

    fn meta(lit: Option<&Lit>) -> Tri {
        lit.map_or(Tri::No, |l| match l.truthy() {
            Ok(true) => Tri::Yes,
            Ok(false) => Tri::No,
            Err(why) => Tri::Unknown(why),
        })
    }

    fn matched(chain: &[Rec], key: &str) -> Tri {
        let mut unknown = None;
        for rec in chain {
            match meta(rec.get(key)) {
                Tri::Yes => return Tri::Yes,
                Tri::Unknown(why) => unknown = Some(why),
                Tri::No => {}
            }
        }
        unknown.map_or(Tri::No, Tri::Unknown)
    }

    fn route(r: &Route, by_name: bool, lits: &[String]) -> Tri {
        let Some(path) = r.screen.route.as_deref() else {
            return Tri::No;
        };
        let mut unknown = None;
        for lit in lits {
            if by_name {
                if r.name.as_deref() == Some(lit) {
                    return Tri::Yes;
                }
            } else if lit == path {
                return Tri::Yes;
            } else if path.contains([':', '*']) && path_patterns_match(path, lit) {
                unknown = Some(format!("{lit} may match {path}"));
            }
        }
        unknown.map_or(Tri::No, Tri::Unknown)
    }
}

/// The text-level shape of a guard function: `if (…) <block>` statements over conjuncts.
mod shape {
    use super::*;

    /// One conjunct of the condition under which a guard blocks.
    pub(super) enum Conj {
        Meta {
            key: String,
            neg: bool,
        },
        Matched {
            key: String,
            neg: bool,
        },
        /// `to.path` or `to.name` among literals.
        Route {
            by_name: bool,
            lits: Vec<String>,
            neg: bool,
        },
        /// What must hold for the guard not to block.
        Check(Condition),
    }

    /// First parameter and the statements of `[async] (to, …) => { … }`, `to => { … }` or
    /// `function (to, …) { … }`.
    pub(super) fn read_fn(src: &str) -> Option<(String, &str)> {
        let b = src.as_bytes();
        let mut j = skip_ws(b, 0);
        if ident_at(src, j) == "async" {
            j = skip_ws(b, j + "async".len());
        }
        let (params, j) = if ident_at(src, j) == "function" {
            function_head(src, j)?
        } else {
            arrow_head(src, j)?
        };
        if b.get(j) != Some(&b'{') {
            return None;
        }
        let e = close(src, j);
        if e != src.trim_end().len() || b.get(e - 1) != Some(&b'}') {
            return None;
        }
        let first = params.split(',').next()?.split(':').next()?.trim();
        // No parameter: the guard cannot test the route, only checks.
        (first.is_empty() || is_ident(first)).then(|| (first.to_string(), &src[j + 1..e - 1]))
    }

    /// Parameters and the offset of the body of `function [name](…) [: T] {` at `j`.
    fn function_head(src: &str, j: usize) -> Option<(&str, usize)> {
        let b = src.as_bytes();
        let j = skip_ws(b, j + "function".len());
        let j = skip_ws(b, j + ident_at(src, j).len());
        let (params, mut j) = paren_params(src, j)?;
        if src[j..].starts_with(':') {
            j += src[j..].find('{')?;
        }
        Some((params, j))
    }

    /// Parameters and the offset of the body of `(…) [: T] =>` or `x =>` at `j`.
    fn arrow_head(src: &str, j: usize) -> Option<(&str, usize)> {
        let b = src.as_bytes();
        let (params, mut j) = if b.get(j) == Some(&b'(') {
            paren_params(src, j)?
        } else {
            let id = ident_at(src, j);
            (id, skip_ws(b, j + id.len()))
        };
        if src[j..].starts_with(':') {
            j += src[j..].find("=>")?;
        }
        let arrow = src[j..].strip_prefix("=>")?;
        Some((params, skip_ws(b, src.len() - arrow.len())))
    }

    /// The text inside the parentheses at `j` and the offset after them.
    fn paren_params(src: &str, j: usize) -> Option<(&str, usize)> {
        if src.as_bytes().get(j) != Some(&b'(') {
            return None;
        }
        let e = close(src, j);
        let params = src.get(j + 1..e.checked_sub(1)?)?;
        Some((params, skip_ws(src.as_bytes(), e)))
    }

    fn skip_noise(s: &str, mut i: usize) -> usize {
        loop {
            let mut p = Parser { s, i };
            p.ws();
            i = p.i;
            if s.as_bytes().get(i) == Some(&b';') {
                i += 1;
            } else {
                return i;
            }
        }
    }

    /// End of the simple statement starting at `from`: `;`, a line break outside brackets,
    /// or a closing bracket.
    fn stmt_end(s: &str, from: usize) -> usize {
        let b = s.as_bytes();
        let (mut i, mut depth) = (from, 0usize);
        while i < b.len() {
            match b[i] {
                b'\'' | b'"' | b'`' => {
                    i = Parser { s, i }.string_end();
                    continue;
                }
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' if depth == 0 => return i,
                b')' | b']' | b'}' => depth -= 1,
                b';' if depth == 0 => return i,
                b'\n' if depth == 0 && !s[from..i].trim().is_empty() => return i,
                _ => {}
            }
            i += 1;
        }
        b.len()
    }

    enum Kind {
        Block,
        Pass,
    }

    /// `return false`, `return '/login'`, `return navigateTo(…)`, `next('/x')` block;
    /// `next()`, `return true` and a bare `return` let the navigation go on.
    fn classify(stmt: &str) -> Option<Kind> {
        let s = stmt.trim().trim_end_matches(';').trim();
        let (returns, e) = match s.strip_prefix("return") {
            Some(r) if !r.starts_with(is_ident_char) => (true, r.trim()),
            _ => (false, s),
        };
        if let Some(args) = e.strip_prefix("next(").and_then(|a| a.strip_suffix(')')) {
            return Some(if args.trim().is_empty() {
                Kind::Pass
            } else {
                Kind::Block
            });
        }
        if returns { returned(e) } else { None }
    }

    /// What `return <e>` does: `true`/nothing continues, a literal, object or navigation
    /// call blocks, anything else is not readable.
    fn returned(e: &str) -> Option<Kind> {
        let literal = matches!(e.chars().next(), Some('\'' | '"' | '`'))
            && Parser { s: e, i: 0 }.string_end() == e.len();
        let blocks = literal
            || (e.starts_with('{') && e.ends_with('}'))
            || e.starts_with("navigateTo(")
            || e.starts_with("abortNavigation(");
        match e {
            "" | "true" | "undefined" => Some(Kind::Pass),
            "false" => Some(Kind::Block),
            _ => blocks.then_some(Kind::Block),
        }
    }

    /// The branch of an `if`/`else` at `i`: its kind and the offset after it. A block holds
    /// exactly one simple statement.
    fn branch(s: &str, i: usize) -> Option<(Kind, usize)> {
        if s.as_bytes().get(i) == Some(&b'{') {
            let end = close(s, i);
            let inner = s.get(i + 1..end.checked_sub(1)?)?;
            let start = skip_noise(inner, 0);
            let stop = stmt_end(inner, start);
            if skip_noise(inner, stop) != inner.len() {
                return None;
            }
            return Some((classify(&inner[start..stop])?, end));
        }
        let end = stmt_end(s, i);
        Some((classify(&s[i..end])?, end))
    }

    /// The blocking rules of a guard body: `if (…) <block>` statements, an optional
    /// `else <pass>`, bindings, `next()` and a final pass. Anything else is not readable.
    pub(super) fn rules(body: &str, tp: String, file_text: &str) -> Option<Vec<Vec<Conj>>> {
        let mut out = Vec::new();
        let mut i = skip_noise(body, 0);
        while i < body.len() {
            if ident_at(body, i) == "if" {
                let (rule, after) = if_rule(body, i, &tp, file_text)?;
                out.push(rule);
                i = after;
            } else if matches!(ident_at(body, i), "const" | "let" | "var") {
                // A pure binding blocks nothing; a call may throw or redirect, so it is unread.
                let end = stmt_end(body, i);
                if calls(&body[i..end]) {
                    return None;
                }
                i = skip_noise(body, end);
            } else {
                let end = stmt_end(body, i);
                if !matches!(classify(&body[i..end])?, Kind::Pass) {
                    return None;
                }
                i = skip_noise(body, end);
                if i < body.len() {
                    return None; // a pass skips what follows
                }
            }
        }
        Some(out)
    }

    /// The conjuncts of the `if (…) <block> [else <pass>]` at `i` and the offset after it.
    fn if_rule(body: &str, i: usize, tp: &str, file_text: &str) -> Option<(Vec<Conj>, usize)> {
        let i = skip_noise(body, i + "if".len());
        if body.as_bytes().get(i) != Some(&b'(') {
            return None;
        }
        let end = close(body, i);
        let cond = body.get(i + 1..end.checked_sub(1)?)?;
        let (kind, after) = branch(body, skip_noise(body, end))?;
        if !matches!(kind, Kind::Block) {
            return None;
        }
        let rule = conjuncts(cond, tp, file_text)?;
        let mut i = skip_noise(body, after);
        if ident_at(body, i) == "else" {
            let (kind, after) = branch(body, skip_noise(body, i + "else".len()))?;
            if !matches!(kind, Kind::Pass) {
                return None;
            }
            i = skip_noise(body, after);
        }
        Some((rule, i))
    }

    /// Splits at top-level `&&`; a top-level `||` is not readable.
    fn conjuncts(cond: &str, tp: &str, file_text: &str) -> Option<Vec<Conj>> {
        let b = cond.as_bytes();
        let (mut i, mut depth, mut start) = (0, 0usize, 0);
        let mut parts = Vec::new();
        while i < b.len() {
            match b[i] {
                b'\'' | b'"' | b'`' => {
                    i = Parser { s: cond, i }.string_end();
                    continue;
                }
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth = depth.saturating_sub(1),
                b'|' if depth == 0 && b.get(i + 1) == Some(&b'|') => return None,
                b'&' if depth == 0 && b.get(i + 1) == Some(&b'&') => {
                    parts.push(&cond[start..i]);
                    start = i + 2;
                    i += 1;
                }
                _ => {}
            }
            i += 1;
        }
        parts.push(&cond[start..]);
        parts
            .into_iter()
            .map(|p| conjunct(p, tp, file_text))
            .collect()
    }

    fn unparen(mut t: &str) -> &str {
        t = t.trim();
        while t.starts_with('(') && close(t, 0) == t.len() {
            t = t[1..t.len() - 1].trim();
        }
        t
    }

    fn conjunct(text: &str, tp: &str, file_text: &str) -> Option<Conj> {
        let t = unparen(text);
        let (neg, t) = match t.strip_prefix('!') {
            Some(r) if !r.starts_with('=') => (true, unparen(r)),
            _ => (false, t),
        };
        meta_conj(t, tp, neg)
            .or_else(|| matched_conj(t, tp, neg))
            .or_else(|| compare_conj(t, tp, neg))
            .or_else(|| includes_conj(t, tp, neg, file_text))
            .or_else(|| check_conj(t, tp, neg, file_text))
    }

    /// `to.meta.key`
    fn meta_conj(t: &str, tp: &str, neg: bool) -> Option<Conj> {
        let key = t.strip_prefix(&format!("{tp}.meta."))?;
        is_ident(key).then(|| Conj::Meta {
            key: key.into(),
            neg,
        })
    }

    /// `to.matched.some(r => r.meta.key)`
    fn matched_conj(t: &str, tp: &str, neg: bool) -> Option<Conj> {
        let rest = t.strip_prefix(&format!("{tp}.matched.some("))?;
        let (param, expr) = rest.strip_suffix(')')?.split_once("=>")?;
        let param = unparen(param);
        let key = expr.trim().strip_prefix(&format!("{param}.meta."))?;
        (is_ident(param) && is_ident(key)).then(|| Conj::Matched {
            key: key.into(),
            neg,
        })
    }

    /// `to.path === '/x'`, `to.name !== 'x'`
    fn compare_conj(t: &str, tp: &str, neg: bool) -> Option<Conj> {
        if neg {
            return None;
        }
        [("path", false), ("name", true)]
            .into_iter()
            .find_map(|(field, by_name)| {
                let rest = t.strip_prefix(&format!("{tp}.{field}"))?.trim_start();
                let (lit, negated) = [("===", false), ("!==", true), ("==", false), ("!=", true)]
                    .into_iter()
                    .find_map(|(op, n)| rest.strip_prefix(op).map(|l| (l.trim(), n)))?;
                let quoted = matches!(lit.chars().next(), Some('\'' | '"'))
                    && Parser { s: lit, i: 0 }.string_end() == lit.len();
                quoted.then(|| Conj::Route {
                    by_name,
                    lits: vec![lit[1..lit.len() - 1].to_string()],
                    neg: negated,
                })
            })
    }

    /// `['/a', '/b'].includes(to.path)` or `LIST.includes(to.name)`
    fn includes_conj(t: &str, tp: &str, neg: bool, file_text: &str) -> Option<Conj> {
        let (list, arg) = t.strip_suffix(')')?.split_once(".includes(")?;
        let by_name = match arg.trim().strip_prefix(&format!("{tp}.")) {
            Some("path") => false,
            Some("name") => true,
            _ => return None,
        };
        let lits = literal_list(list.trim(), file_text)?;
        Some(Conj::Route { by_name, lits, neg })
    }

    /// A check by name: `store.user`, `auth.isLoggedIn()`, `loggedIn.value`.
    fn check_conj(t: &str, tp: &str, neg: bool, file_text: &str) -> Option<Conj> {
        let name = t.strip_suffix("()").unwrap_or(t);
        let name = name.strip_suffix(".value").unwrap_or(name);
        // Anything on the route object that is no selector above is not a check.
        let on_route = !tp.is_empty() && (name == tp || name.starts_with(&format!("{tp}.")));
        if !is_name(name) || on_route {
            return None;
        }
        let uses_session = !words(file_text, "useUserSession").is_empty();
        let held = if name == "loggedIn" && uses_session {
            Condition::Authenticated
        } else {
            Condition::Custom(name.into())
        };
        Some(Conj::Check(if neg {
            held
        } else {
            Condition::Not(Box::new(held))
        }))
    }

    /// Strings of an array literal, or of a `const NAME = […]` of this file.
    fn literal_list(list: &str, file_text: &str) -> Option<Vec<String>> {
        let items = if list.starts_with('[') {
            Parser { s: list, i: 0 }.array(0)
        } else {
            let at = local_array(file_text, list)?;
            Parser {
                s: file_text,
                i: at,
            }
            .array(0)
        };
        items
            .iter()
            .map(|i| match &i.value {
                Value::Str(s) => Some(s.clone()),
                _ => None,
            })
            .collect()
    }
}

/// `@sidebase/nuxt-auth`: module and global default from `nuxt.config`, page `auth` meta.
mod sidebase {
    use super::*;

    #[derive(Default, PartialEq, Eq, Clone, Copy)]
    pub(super) enum Presence {
        #[default]
        Absent,
        Present,
        Unknown,
    }

    #[derive(Default, Clone, Copy)]
    enum Global {
        #[default]
        Off,
        On,
        Unknown,
    }

    #[derive(Default)]
    pub(super) struct Module {
        pub(super) present: Presence,
        global: Global,
    }

    impl Module {
        pub(super) fn read(app: &App, out: &mut Vec<UiFlowDiagnostic>) -> Module {
            let Some((file, text)) = ["nuxt.config.ts", "nuxt.config.js"]
                .iter()
                .find_map(|n| app.files.get_key_value(&format!("{}{n}", app.prefix)))
            else {
                return Module::default();
            };
            let unknown = |out: &mut Vec<UiFlowDiagnostic>, line: u32, why: &str| {
                diag(
                    out,
                    file,
                    line,
                    format!("{why}; whether @sidebase/nuxt-auth protects pages is unknown."),
                );
                Module {
                    present: Presence::Unknown,
                    global: Global::Unknown,
                }
            };
            let b = text.as_bytes();
            let Some(pos) = words(text, "defineNuxtConfig").first().copied() else {
                return Module::default();
            };
            let open = skip_ws(b, pos + "defineNuxtConfig".len());
            let line = line_of(text, pos);
            let Value::Obj(fields) = (Parser {
                s: text,
                i: open + 1,
            })
            .value(0)
            .value
            else {
                return unknown(out, line, "nuxt.config is not an object literal");
            };
            let Some((_, modules)) = fields.iter().find(|(k, _)| k == "modules") else {
                return Module::default();
            };
            let Value::Arr(items) = &modules.value else {
                return unknown(out, line, "modules is not an array literal");
            };
            let name = |i: &Item| match &i.value {
                Value::Str(s) => Some(s.clone()),
                Value::Arr(t) => match t.first().map(|f| &f.value) {
                    Some(Value::Str(s)) => Some(s.clone()),
                    _ => None,
                },
                _ => None,
            };
            let names: Vec<Option<String>> = items.iter().map(name).collect();
            if names.iter().any(|n| n.as_deref() == Some(SIDEBASE)) {
                let global = fields
                    .iter()
                    .find(|(k, _)| k == "auth")
                    .map_or(Global::Off, |(_, v)| global_flag(&v.value));
                if matches!(global, Global::Unknown) {
                    diag(
                        out,
                        file,
                        line,
                        "auth.globalAppMiddleware is not a literal; whether every page requires a session is unknown.",
                    );
                }
                Module {
                    present: Presence::Present,
                    global,
                }
            } else if names.iter().any(Option::is_none) {
                unknown(out, line, "modules holds an entry that is not a literal")
            } else {
                Module::default()
            }
        }

        pub(super) fn state(&self, r: &Route, out: &mut Vec<UiFlowDiagnostic>) -> AccessState {
            let page = nearest(&r.chain, "auth");
            let unknown = |out: &mut Vec<UiFlowDiagnostic>, why: &str| {
                diag(
                    out,
                    &r.screen.file,
                    r.screen.line,
                    format!("Page meta auth is {why}; the page's access is unknown."),
                );
                AccessState::Protected(Condition::Unknown("auth".into()))
            };
            let authenticated = AccessState::Protected(Condition::Authenticated);
            match (self.present, page) {
                (Presence::Absent, _) | (Presence::Unknown, None) => AccessState::Undeclared,
                (Presence::Unknown, Some(_)) => {
                    unknown(out, "set while the auth module is unknown")
                }
                (Presence::Present, None) => match self.global {
                    Global::On => authenticated,
                    Global::Off => AccessState::Undeclared,
                    Global::Unknown => unknown(out, "unset while the global middleware is unknown"),
                },
                (Presence::Present, Some(Lit::Bool(false))) => AccessState::Open,
                (Presence::Present, Some(Lit::Bool(true))) => authenticated,
                (Presence::Present, Some(Lit::Obj(fields))) => {
                    match fields
                        .iter()
                        .rev()
                        .find(|(k, _)| k == "unauthenticatedOnly")
                    {
                        Some((_, Lit::Bool(true))) => AccessState::Protected(Condition::Not(
                            Box::new(Condition::Authenticated),
                        )),
                        Some((_, Lit::Bool(false))) => authenticated,
                        _ => unknown(out, "an object without a literal unauthenticatedOnly"),
                    }
                }
                (Presence::Present, Some(_)) => unknown(out, "not a literal true, false or object"),
            }
        }
    }

    /// `auth: { globalAppMiddleware: true }`, or an object with a literal `isEnabled`.
    fn global_flag(auth: &Value) -> Global {
        let Value::Obj(fields) = auth else {
            return Global::Unknown;
        };
        let Some((_, v)) = fields.iter().find(|(k, _)| k == "globalAppMiddleware") else {
            return Global::Off;
        };
        match &v.value {
            Value::Raw(r) if r == "true" => Global::On,
            Value::Raw(r) if r == "false" => Global::Off,
            Value::Obj(f) => match f
                .iter()
                .find(|(k, _)| k == "isEnabled")
                .map(|(_, i)| &i.value)
            {
                Some(Value::Raw(r)) if r == "true" => Global::On,
                Some(Value::Raw(r)) if r == "false" => Global::Off,
                _ => Global::Unknown,
            },
            _ => Global::Unknown,
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
router.beforeEach((to) => { if (to.meta.requiresAuth && !signedIn()) return '/login' })
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
        // `requiresAuth` counts through the guard; the child inherits it under `to.meta`.
        let account = Some(Condition::Custom("signedIn".into()));
        let order = account.clone();
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
        // `roles` is read by no guard: a note, not a gap or a condition.
        let note = flow
            .diagnostics
            .iter()
            .find(|d| d.message.contains("roles"))
            .expect("unread meta note");
        assert_eq!(
            (note.file.as_str(), note.line, note.severity),
            ("shop/src/router/index.ts", 19, DiagnosticSeverity::Note)
        );
        assert!(
            flow.diagnostics
                .iter()
                .all(|d| !d.message.contains("Navigation guard")),
            "{:#?}",
            flow.diagnostics
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
                    None
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
        // An empty global middleware checks nothing: no note, no gap.
        assert!(
            flow.diagnostics
                .iter()
                .all(|d| d.file != "web/middleware/auth.global.ts"),
            "{:#?}",
            flow.diagnostics
        );
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
                file(8)
            ],
            "{:#?}",
            flow.diagnostics
        );
        assert_eq!(
            screens(&flow),
            // `roles` is read by no guard, so its non-literal value decides nothing.
            vec![("app:/y".into(), "Home".into(), Some("/y".into()), None)]
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

    // --- Wave 2f: access only through a guard that reads it (AVS B1-B12, C1-C13) ---

    fn state(flow: &UiFlow, route: &str) -> AccessState {
        flow.screens
            .iter()
            .find(|s| s.route.as_deref() == Some(route))
            .unwrap_or_else(|| panic!("no screen {route}: {:#?}", flow.screens))
            .access
            .clone()
    }

    fn gap_count(flow: &UiFlow) -> usize {
        gaps(flow).len()
    }

    fn notes(flow: &UiFlow, word: &str) -> usize {
        flow.diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Note && d.message.contains(word))
            .count()
    }

    fn protected(c: Condition) -> AccessState {
        AccessState::Protected(c)
    }

    fn custom(n: &str) -> Condition {
        Condition::Custom(n.into())
    }

    fn is_unknown(a: &AccessState) -> bool {
        matches!(a, AccessState::Protected(Condition::Unknown(_)))
    }

    fn vr(body: &str) -> UiFlow {
        let head = "import A from './A.vue'\nimport B from './B.vue'\nimport Adm from './Adm.vue'\nimport Login from './Login.vue'\nimport X from './X.vue'\nimport P from './P.vue'\n";
        let dir = project(&[
            ("package.json", ROUTER_PKG),
            ("src/router.ts", &format!("{head}{body}\n")),
            ("src/A.vue", "<template />\n"),
            (
                "src/B.vue",
                "<script>\nexport default { beforeRouteEnter(to, from) { return true } }\n</script>\n",
            ),
        ]);
        extract(dir.path())
    }

    fn guarded(routes: &str, guard: &str) -> UiFlow {
        vr(&format!(
            "const router = createRouter({{ routes: [{routes}] }})\nrouter.beforeEach({guard})\nexport default router"
        ))
    }

    #[test]
    fn b1_no_guard_is_undeclared() {
        let flow = vr("export default createRouter({ routes: [{ path: '/a', component: A }] })");
        assert_eq!(state(&flow, "/a"), AccessState::Undeclared);
        assert_eq!(gap_count(&flow), 0);
    }

    #[test]
    fn b2_meta_without_a_guard_is_inert() {
        let flow = vr(
            "export default createRouter({ routes: [{ path: '/a', component: A, meta: { requiresAuth: true } }] })",
        );
        assert_eq!(state(&flow, "/a"), AccessState::Undeclared);
        assert_eq!(notes(&flow, "requiresAuth"), 1, "{:#?}", flow.diagnostics);
    }

    const B3_GUARD: &str =
        "(to) => { if (to.meta.requiresAuth && !auth.isLoggedIn()) return '/login' }";

    #[test]
    fn b3_meta_counts_through_the_guard_that_reads_it() {
        let flow = guarded(
            "{ path: '/a', component: A, meta: { requiresAuth: true } }, { path: '/b', component: X }",
            B3_GUARD,
        );
        assert_eq!(state(&flow, "/a"), protected(custom("auth.isLoggedIn")));
        assert_eq!(state(&flow, "/b"), AccessState::Undeclared);
        assert_eq!(notes(&flow, "requiresAuth"), 0);
        assert_eq!(gap_count(&flow), 0, "{:#?}", flow.diagnostics);
    }

    #[test]
    fn b3_legacy_next_style_reads_too() {
        let flow = guarded(
            "{ path: '/a', component: A, meta: { requiresAuth: true } }",
            "(to, from, next) => { if (to.meta.requiresAuth && !auth.isLoggedIn()) { next('/login') } else { next() } }",
        );
        assert_eq!(state(&flow, "/a"), protected(custom("auth.isLoggedIn")));
        assert_eq!(gap_count(&flow), 0, "{:#?}", flow.diagnostics);
    }

    #[test]
    fn b4_opt_out_guard_protects_routes_without_meta() {
        let flow = guarded(
            "{ path: '/x', component: X }, { path: '/login', component: Login, meta: { public: true } }",
            "(to) => { if (!to.meta.public && !store.user) return '/login' }",
        );
        assert_eq!(state(&flow, "/x"), protected(custom("store.user")));
        assert_eq!(state(&flow, "/login"), AccessState::Open);
    }

    #[test]
    fn b5_literal_allowlist_is_open() {
        let flow = vr("const white = ['/login', '/404']
const router = createRouter({ routes: [{ path: '/login', component: Login }, { path: '/x', component: X }] })
router.beforeEach((to) => { if (!white.includes(to.path) && !token) return '/login' })
export default router");
        assert_eq!(state(&flow, "/login"), AccessState::Open);
        assert_eq!(state(&flow, "/x"), protected(custom("token")));
    }

    const PARENT_CHILD: &str = "{ path: '/p', component: P, meta: { requiresAuth: true }, children: [{ path: 'c', component: X, meta: { requiresAuth: false } }] }";

    #[test]
    fn b6_to_meta_child_overrides_parent() {
        let flow = guarded(
            PARENT_CHILD,
            "(to) => { if (to.meta.requiresAuth && !ok) return '/login' }",
        );
        assert_eq!(state(&flow, "/p"), protected(custom("ok")));
        assert_eq!(state(&flow, "/p/c"), AccessState::Undeclared);
    }

    #[test]
    fn b7_matched_some_sees_every_ancestor() {
        let flow = guarded(
            PARENT_CHILD,
            "(to) => { if (to.matched.some(r => r.meta.requiresAuth) && !ok) return '/login' }",
        );
        assert_eq!(state(&flow, "/p/c"), protected(custom("ok")));
    }

    #[test]
    fn b8_unreadable_guard_makes_every_screen_unknown_with_a_gap() {
        let flow = guarded(
            "{ path: '/a', component: A }, { path: '/x', component: X }",
            "async (to, from, next) => { await store.dispatch('x'); next() }",
        );
        assert!(is_unknown(&state(&flow, "/a")), "{:?}", state(&flow, "/a"));
        assert!(is_unknown(&state(&flow, "/x")));
        assert_eq!(gap_count(&flow), 1, "{:#?}", flow.diagnostics);
        assert!(!flow.complete);
    }

    #[test]
    fn b8_unreadable_guard_never_lets_an_open_marker_win() {
        let flow = vr("const router = createRouter({ routes: [{ path: '/login', component: Login, meta: { public: true } }] })
router.beforeEach((to) => { if (!to.meta.public && !ok) return '/login' })
router.beforeEach(async (to) => { await x() })
export default router");
        assert!(
            is_unknown(&state(&flow, "/login")),
            "{:?}",
            state(&flow, "/login")
        );
    }

    #[test]
    fn guard_reading_a_non_literal_meta_is_unknown_with_a_gap() {
        let flow = guarded(
            "{ path: '/a', component: A, meta: { requiresAuth: flag } }",
            B3_GUARD,
        );
        assert!(is_unknown(&state(&flow, "/a")), "{:?}", state(&flow, "/a"));
        assert!(gap_count(&flow) >= 1);
    }

    #[test]
    fn meta_spread_hides_the_key_so_a_reading_guard_is_unknown() {
        let flow = guarded(
            "{ path: '/a', component: A, meta: { ...base } }",
            "(to) => { if (!to.meta.public && !ok) return '/login' }",
        );
        assert!(is_unknown(&state(&flow, "/a")), "{:?}", state(&flow, "/a"));
    }

    #[test]
    fn b9_before_enter_name_is_custom() {
        let flow = vr(
            "export default createRouter({ routes: [{ path: '/adm', component: Adm, beforeEnter: requireAdmin }] })",
        );
        assert_eq!(state(&flow, "/adm"), protected(custom("requireAdmin")));
        assert_eq!(gap_count(&flow), 0);
    }

    #[test]
    fn b10_before_enter_array_on_parent_reaches_children() {
        let flow = vr(
            "export default createRouter({ routes: [{ path: '/p', component: P, beforeEnter: [a, b], children: [{ path: 'c', component: X }] }] })",
        );
        let both = protected(Condition::All(vec![custom("a"), custom("b")]));
        assert_eq!(state(&flow, "/p"), both);
        assert_eq!(state(&flow, "/p/c"), both);
    }

    #[test]
    fn b11_inline_before_enter_is_unknown_with_a_gap() {
        let flow = vr(
            "export default createRouter({ routes: [{ path: '/a', component: A, beforeEnter: (to) => { if (!ok()) return false } }] })",
        );
        assert!(is_unknown(&state(&flow, "/a")), "{:?}", state(&flow, "/a"));
        assert_eq!(gap_count(&flow), 1, "{:#?}", flow.diagnostics);
    }

    #[test]
    fn b12_before_route_enter_in_the_component_is_unknown_with_a_gap() {
        let flow = vr(
            "export default createRouter({ routes: [{ path: '/b', component: B }, { path: '/a', component: A }] })",
        );
        assert!(is_unknown(&state(&flow, "/b")), "{:?}", state(&flow, "/b"));
        assert_eq!(state(&flow, "/a"), AccessState::Undeclared);
        assert_eq!(gap_count(&flow), 1, "{:#?}", flow.diagnostics);
    }

    #[test]
    fn b13_after_each_is_no_access() {
        let flow = vr(
            "const router = createRouter({ routes: [{ path: '/a', component: A }] })
router.afterEach(() => { track() })
export default router",
        );
        assert_eq!(state(&flow, "/a"), AccessState::Undeclared);
        assert_eq!(gap_count(&flow), 0);
    }

    fn nuxt_app(files: &[(&str, &str)]) -> UiFlow {
        let mut all: Vec<(&str, &str)> =
            vec![("package.json", r#"{"devDependencies":{"nuxt":"^3.13.0"}}"#)];
        all.extend_from_slice(files);
        extract(project(&all).path())
    }

    const PAGE: &str = "<template />\n";

    #[test]
    fn c1_plain_page_is_undeclared() {
        let flow = nuxt_app(&[("pages/a.vue", PAGE)]);
        assert_eq!(state(&flow, "/a"), AccessState::Undeclared);
    }

    #[test]
    fn c2_c3_named_middleware_is_custom_and_all() {
        let flow = nuxt_app(&[
            (
                "pages/a.vue",
                "<script setup>definePageMeta({ middleware: 'auth' })</script>",
            ),
            (
                "pages/b.vue",
                "<script setup>definePageMeta({ middleware: ['auth', 'admin'] })</script>",
            ),
            (
                "middleware/auth.ts",
                "export default defineNuxtRouteMiddleware(() => { check() })",
            ),
        ]);
        assert_eq!(state(&flow, "/a"), protected(custom("auth")));
        assert_eq!(
            state(&flow, "/b"),
            protected(Condition::All(vec![custom("auth"), custom("admin")]))
        );
    }

    #[test]
    fn c4_inline_middleware_is_unknown_with_a_gap() {
        let flow = nuxt_app(&[(
            "pages/a.vue",
            "<script setup>definePageMeta({ middleware: [(to) => { x() }] })</script>",
        )]);
        assert!(is_unknown(&state(&flow, "/a")));
        assert_eq!(gap_count(&flow), 1);
    }

    #[test]
    fn c5_global_middleware_reads_path_exemption() {
        let flow = nuxt_app(&[
            (
                "middleware/01.auth.global.ts",
                "export default defineNuxtRouteMiddleware((to, from) => {\n  if (!loggedIn.value && to.path !== '/login') return navigateTo('/login')\n})\n",
            ),
            ("pages/login.vue", PAGE),
            ("pages/a.vue", PAGE),
        ]);
        assert_eq!(state(&flow, "/login"), AccessState::Open);
        assert_eq!(state(&flow, "/a"), protected(custom("loggedIn")));
        assert_eq!(gap_count(&flow), 0, "{:#?}", flow.diagnostics);
    }

    #[test]
    fn c5_logged_in_from_use_user_session_is_authenticated() {
        let flow = nuxt_app(&[
            (
                "middleware/auth.global.ts",
                "export default defineNuxtRouteMiddleware(() => {\n  const { loggedIn } = useUserSession()\n  if (!loggedIn.value) return navigateTo('/login')\n})\n",
            ),
            ("pages/a.vue", PAGE),
        ]);
        assert_eq!(state(&flow, "/a"), protected(Condition::Authenticated));
    }

    #[test]
    fn c6_unreadable_global_middleware_is_unknown_with_a_gap() {
        let flow = nuxt_app(&[
            (
                "middleware/x.global.ts",
                "export default defineNuxtRouteMiddleware(async (to) => { await load(); })\n",
            ),
            ("pages/a.vue", PAGE),
            ("pages/b.vue", PAGE),
        ]);
        assert!(is_unknown(&state(&flow, "/a")));
        assert!(is_unknown(&state(&flow, "/b")));
        assert_eq!(gap_count(&flow), 1, "{:#?}", flow.diagnostics);
        assert_eq!(notes(&flow, "Global route middleware"), 0);
    }

    #[test]
    fn c7_parent_page_middleware_reaches_children() {
        let flow = nuxt_app(&[
            (
                "pages/users.vue",
                "<script setup>definePageMeta({ middleware: 'auth' })</script><template><NuxtPage /></template>",
            ),
            ("pages/users/index.vue", PAGE),
            ("pages/users/[id].vue", PAGE),
        ]);
        assert_eq!(state(&flow, "/users"), protected(custom("auth")));
        assert_eq!(state(&flow, "/users/:id"), protected(custom("auth")));
    }

    #[test]
    fn c8_page_meta_without_a_reading_middleware_is_inert() {
        let flow = nuxt_app(&[(
            "pages/a.vue",
            "<script setup>definePageMeta({ requiresAuth: true })</script>",
        )]);
        assert_eq!(state(&flow, "/a"), AccessState::Undeclared);
        assert_eq!(notes(&flow, "requiresAuth"), 1, "{:#?}", flow.diagnostics);
    }

    const SIDEBASE: &str = "export default defineNuxtConfig({ modules: ['@sidebase/nuxt-auth'], auth: { globalAppMiddleware: true } })\n";

    #[test]
    fn c9_sidebase_global_middleware_with_page_opt_out() {
        let flow = nuxt_app(&[
            ("nuxt.config.ts", SIDEBASE),
            ("pages/a.vue", PAGE),
            (
                "pages/login.vue",
                "<script setup>definePageMeta({ auth: false })</script>",
            ),
        ]);
        assert_eq!(state(&flow, "/a"), protected(Condition::Authenticated));
        assert_eq!(state(&flow, "/login"), AccessState::Open);
        assert_eq!(gap_count(&flow), 0, "{:#?}", flow.diagnostics);
        assert_eq!(notes(&flow, "auth"), 0);
    }

    #[test]
    fn c10_sidebase_guest_mode() {
        let flow = nuxt_app(&[
            ("nuxt.config.ts", SIDEBASE),
            (
                "pages/register.vue",
                "<script setup>definePageMeta({ auth: { unauthenticatedOnly: true, navigateAuthenticatedTo: '/' } })</script>",
            ),
        ]);
        assert_eq!(
            state(&flow, "/register"),
            protected(Condition::Not(Box::new(Condition::Authenticated)))
        );
    }

    #[test]
    fn c11_sidebase_local_mode() {
        let flow = nuxt_app(&[
            (
                "nuxt.config.ts",
                "export default defineNuxtConfig({ modules: ['@sidebase/nuxt-auth'] })\n",
            ),
            (
                "pages/a.vue",
                "<script setup>definePageMeta({ auth: true })</script>",
            ),
            ("pages/b.vue", PAGE),
        ]);
        assert_eq!(state(&flow, "/a"), protected(Condition::Authenticated));
        assert_eq!(state(&flow, "/b"), AccessState::Undeclared);
    }

    #[test]
    fn c12_auth_key_without_the_module_is_inert() {
        let flow = nuxt_app(&[
            (
                "nuxt.config.ts",
                "export default defineNuxtConfig({ modules: ['@nuxt/ui'] })\n",
            ),
            (
                "pages/a.vue",
                "<script setup>definePageMeta({ auth: false })</script>",
            ),
        ]);
        assert_eq!(state(&flow, "/a"), AccessState::Undeclared);
    }

    #[test]
    fn c13_auth_false_replaces_only_the_module_default() {
        let flow = nuxt_app(&[
            ("nuxt.config.ts", SIDEBASE),
            (
                "pages/adm.vue",
                "<script setup>definePageMeta({ auth: false, middleware: 'admin' })</script>",
            ),
        ]);
        assert_eq!(state(&flow, "/adm"), protected(custom("admin")));
    }

    #[test]
    fn sidebase_unreadable_global_flag_never_shows_open_or_plain() {
        let flow = nuxt_app(&[
            (
                "nuxt.config.ts",
                "export default defineNuxtConfig({ modules: ['@sidebase/nuxt-auth'], auth: { globalAppMiddleware: cfg } })\n",
            ),
            ("pages/a.vue", PAGE),
        ]);
        assert!(is_unknown(&state(&flow, "/a")), "{:?}", state(&flow, "/a"));
        assert!(gap_count(&flow) >= 1);
    }

    #[test]
    fn binding_with_a_call_makes_the_guard_unreadable() {
        let flow = nuxt_app(&[
            (
                "middleware/a.global.ts",
                "export default defineNuxtRouteMiddleware(async (to) => {\n  const u = await check(to)\n  if (!loggedIn.value && to.path !== '/login') return navigateTo('/login')\n})\n",
            ),
            ("pages/login.vue", PAGE),
            ("pages/a.vue", PAGE),
        ]);
        assert!(
            is_unknown(&state(&flow, "/login")),
            "{:?}",
            state(&flow, "/login")
        );
        assert!(is_unknown(&state(&flow, "/a")));
        assert_eq!(gap_count(&flow), 1, "{:#?}", flow.diagnostics);
    }

    #[test]
    fn pure_bindings_stay_readable() {
        let flow = nuxt_app(&[
            (
                "middleware/a.global.ts",
                "export default defineNuxtRouteMiddleware((to) => {\n  const p = to.path\n  const { meta } = to\n  if (!loggedIn.value && to.path !== '/login') return navigateTo('/login')\n})\n",
            ),
            ("pages/login.vue", PAGE),
        ]);
        assert_eq!(state(&flow, "/login"), AccessState::Open);
    }
}
