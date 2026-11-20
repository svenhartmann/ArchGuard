//! React and Next.js extractor for ui-flow/v1 (ADR 0021, phase 6). A text-level
//! scanner without a TypeScript parser, built like the Angular extractor; what it
//! cannot read is a diagnostic, and compiler-backed resolution is ADR 0021 phase 7.
//!
//! Applications are directories whose `package.json` names `react-router`,
//! `react-router-dom`, `@tanstack/react-router` or `next`; a file belongs to the
//! deepest one holding it. Scanned: `.ts`, `.tsx`, `.js`, `.jsx`. Skipped:
//! `node_modules`, `dist`, `build`, `.next`, `cypress`, `e2e`, `.storybook`, the
//! engine's other excluded directories, test files, `*.stories.*`, `*.cy.*`, `*.d.ts`.
//!
//! Screens:
//! - Next.js `pages/**` and `src/pages/**` (pages router): one page per file;
//!   `index` is the directory, `[id]` becomes `:id`, `[...slug]` and `[[...slug]]`
//!   become `*`, a segment starting with `_` (`_app`, `_document`) is skipped.
//! - Next.js `app/**/page.*` and `src/app/**/page.*` (app router): route groups
//!   `(x)` are dropped; pages below a `@slot`, a private `_folder` or an
//!   intercepting `(.)x` segment are skipped.
//! - The screen name is the default export's name, else the file path.
//! - React Router: `<Route path element Component index>` nested in JSX, and route
//!   objects (`path`, `element`, `Component`, `index`, `children`) passed as a
//!   literal array or a same-file array variable to `createBrowserRouter`,
//!   `createHashRouter`, `createMemoryRouter` or `useRoutes`. Child paths join the
//!   parent's; the screen name is the first capitalized element tag. A route
//!   whose children hold an index route is a layout (no screen of its own);
//!   `<Navigate>` elements are redirects (no screen).
//!
//! - TanStack Router file routes: a module with `createFileRoute('/a/$id')` is a screen
//!   (`$id` is `:id`, `_x` and `(x)` segments drop out) unless it renders `<Outlet`
//!   (layout) or is the root. The screen name is the `component:` identifier.
//!
//! Access:
//! - TanStack `beforeLoad`: `if (c) { throw redirect(…) }` in the module and its
//!   ancestor routes (by route id) requires `not c`; `c` is `custom(name)` unless
//!   `[views.access]` lists it. A redirect without `if` is no access. Only arrow
//!   functions with a block body are read.
//! - A route element wrapped in `RequireAuth`, `ProtectedRoute`, `PrivateRoute` or
//!   `AuthGuard` (also as pathless parent route) is `custom(name)`; children
//!   inherit it, several are `all`.
//! - Next.js `middleware.{ts,js}` (app or `src/`): every literal `matcher` path
//!   (`/x`, `/x/:id`, trailing `/:path*`, `/:path+`, `/:path?`) adds
//!   `custom(middleware)` to the screens and endpoints it covers; without a
//!   matcher it covers all. Regular-expression matchers are diagnostics.
//!
//! Backend endpoints (the Next.js server side): `pages/api/**` is one endpoint per
//! file with method `ANY`; `app/**/route.*` one per exported `GET`, `POST`, …
//! (`export function M(`, `export async function M(`, `export const M =`). Access
//! is `undeclared` unless the handler is wrapped literally as
//! `export default withAuth(…)` / `withApiAuthRequired(…)` (or `export const M =
//! withAuth(…)`), which is `custom(name)`. Checks written inside a handler stay
//! `undeclared` (ADR 0021, consequences).
//!
//! Transitions: `<Link href|to>` and `<NavLink to>` (also `<X.Link>`, a default
//! import of `next/link` and `styled(…)` wrappers of these), `router.push|replace`
//! on `useRouter()` results or the `next/router` default import, destructured
//! `push`/`replace` of `useRouter()`, `useNavigate()` results, and `redirect` /
//! `permanentRedirect` imported from `next/navigation` or React Router. The target
//! is a literal, a template literal (holes become `:param`) or `{ pathname: '…' }`;
//! relative targets join the screen's route. Targets resolve with `best_route`
//! within the application; external URLs and `#anchors` are no transition.
//!
//! Actions: `fetch(url, { method })` (default `GET`), `axios.get/post/put/patch/
//! delete/head/options(url)` on an `axios` default or namespace import, and the
//! same methods on an instance `const api = axios.create({ baseURL })` declared in
//! the file or imported from a relative module (named or default export); a
//! literal `baseURL` path is prefixed. No other receiver counts.
//!
//! Generated hey-api clients (`sdk.gen.ts`, optional `core/OpenAPI.ts` `BASE`): each
//! exported function or static method with a literal `url:` and a method (`method:
//! Method.X`, `'x'` or a `.get<…>(` call) is indexed one hop; `Class.method(` or an
//! imported function from a `client`/`sdk` module is that `Call::Http` (`{id}` holes
//! are `:param`). A non-literal url or method is `Call::Unresolved` and a diagnostic.
//! TanStack links resolve `$id` as `:id`; `navigate({ to })` is read like `{ pathname }`.
//!
//! Origin: links and calls in a file that defines a screen (its Next.js page file,
//! or a file declaring the route's component) start at those screens; any other
//! file (layout, menu, shared component, helper) acts on every screen of the
//! application (`any_screen`).
//!
//! Not recognized; each case is a diagnostic and makes the flow incomplete:
//! non-literal paths, targets, URLs, methods and `baseURL`s, route `lazy`, route
//! elements that are not JSX, TanStack code-based route trees (a note) and
//! `<Link to={Route.x}>` (unresolved). Not followed: descendant `<Routes>` in other components, calls
//! through helper functions or services (`request({ url })`), links to catch-all
//! routes (`*` matches no literal segment), `<a href>`.
use crate::extract_ctx::{
    ExtractContext, dir_name as app_name, finish, gap as diag, read_source as read, walk,
    with_root_config,
};
use crate::lang_ts::*;
use crate::lang_ts_decl::*;
use crate::lang_ts_http::*;
use crate::services::support::test_path;
use archguard_core::{
    ANY_METHOD, AccessMeaning, AccessState, Action, Call, Condition, EndpointAccess, Origin,
    Screen, ScreenKind, Target, Transition, UiFlow, UiFlowDiagnostic, best_route,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const SOURCE: &str = "react";
type Access = BTreeMap<String, AccessMeaning>;
const MAX_FILES: usize = 5_000;
/// Nesting bound for parsed literals and route trees.
const MAX_DEPTH: usize = 32;
const ROUTER_DEPS: [&str; 4] = [
    "react-router",
    "react-router-dom",
    "@tanstack/react-router",
    "next",
];
const SKIPPED_DIRS: [&str; 5] = [".next", "cypress", "e2e", ".storybook", "coverage"];
const EXTENSIONS: [&str; 4] = [".tsx", ".ts", ".jsx", ".js"];
const GUARDS: [&str; 4] = ["RequireAuth", "ProtectedRoute", "PrivateRoute", "AuthGuard"];
const API_GUARDS: [&str; 2] = ["withAuth", "withApiAuthRequired"];
/// Screens, transitions, HTTP actions and Next.js endpoints of every React
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
                app.extract(&ctx.config.views.access, &mut flow);
            }
        }
        Err(e) => diag(
            &mut flow.diagnostics,
            "",
            0,
            format!("Discovery exclusions not read; no React application analyzed: {e}"),
        ),
    }
    finish(&mut flow);
    flow
}

fn is_source(rel: &str, name: &str) -> bool {
    EXTENSIONS.iter().any(|e| name.ends_with(e))
        && !name.ends_with(".d.ts")
        && !name.contains(".stories.")
        && !name.contains(".cy.")
        && !test_path(rel)
}

struct App {
    name: String,
    /// The application directory with a trailing `/`, `""` at the root.
    prefix: String,
    next: bool,
    tanstack: bool,
    /// `paths` of the application's `tsconfig.json`.
    aliases: Vec<Alias>,
    /// Sources by path relative to the analysis root.
    files: BTreeMap<String, String>,
}

/// What the `*` of a tsconfig `paths` pattern matches in `spec`; `""` for an
/// exact pattern; `None` when the pattern does not match.
fn alias_rest<'s>(a: &Alias, spec: &'s str) -> Option<&'s str> {
    match a.pattern.split_once('*') {
        Some((head, tail)) => spec.strip_prefix(head)?.strip_suffix(tail),
        None if a.pattern == spec => Some(""),
        None => None,
    }
}

/// Who owns a screen's links and calls.
enum Owner {
    /// A Next.js page file.
    File(String),
    /// A React Router element component, found by its declaration.
    Component(String),
}

fn applications(root: &Path, files: Vec<String>, out: &mut Vec<UiFlowDiagnostic>) -> Vec<App> {
    let mut apps: BTreeMap<String, App> = BTreeMap::new();
    let mut sources = Vec::new();
    let mut tsconfigs = Vec::new();
    for rel in files {
        if rel.split('/').any(|p| SKIPPED_DIRS.contains(&p)) {
            continue;
        }
        let (dir, name) = rel.rsplit_once('/').unwrap_or(("", rel.as_str()));
        if name == "package.json" {
            if let Some(app) = manifest_app(root, dir, &rel) {
                apps.insert(dir.to_string(), app);
            }
        } else if name == "tsconfig.json" {
            tsconfigs.push(rel);
        } else if is_source(&rel, name) {
            sources.push(rel);
        }
    }
    for rel in tsconfigs {
        let dir = rel.rsplit_once('/').map_or("", |(d, _)| d);
        if let Some(app) = apps.get_mut(dir) {
            app.aliases = tsconfig_aliases(&rel, &|p| read(root, p).ok());
        }
    }
    read_sources(root, &mut apps, sources, out);
    apps.into_values().collect()
}

/// The application a `package.json` declares, when it names a router dependency.
fn manifest_app(root: &Path, dir: &str, rel: &str) -> Option<App> {
    let text = read(root, rel).ok()?;
    let deps: Vec<&str> = ROUTER_DEPS
        .into_iter()
        .filter(|d| has_dep(&text, d))
        .collect();
    if deps.is_empty() {
        return None;
    }
    let prefix = if dir.is_empty() {
        String::new()
    } else {
        format!("{dir}/")
    };
    Some(App {
        name: app_name(root, dir),
        prefix,
        next: deps.contains(&"next"),
        tanstack: deps.contains(&"@tanstack/react-router"),
        aliases: Vec::new(),
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
                    "React scan of {} stopped after {MAX_FILES} source files; later files are unknown.",
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
    fn extract(&self, access: &Access, flow: &mut UiFlow) {
        let mut screens: Vec<(Screen, Owner)> = Vec::new();
        let mut endpoints = Vec::new();
        let out = &mut flow.diagnostics;
        if self.next {
            next::file_routes(self, &mut screens, &mut endpoints);
        }
        for (file, text) in &self.files {
            router::routes(self, file, text, &mut screens, out);
        }
        if self.next {
            next::middleware(self, &mut screens, &mut endpoints, out);
        }
        if self.tanstack && !tanstack::file_routes(self, access, &mut screens, out) {
            let message =
                "No createFileRoute module found; TanStack code-based route trees are not read.";
            diag(out, &format!("{}package.json", self.prefix), 0, message);
        }
        let routes = nav::Routes {
            app: self,
            names: nav::link_names(self),
            screens: &screens,
        };
        let clients = Clients::new(&self.files);
        let sdk = sdk::Sdk::new(self);
        for (file, text) in &self.files {
            if self.next && next::is_backend(self, file) {
                continue;
            }
            let mut froms: Vec<(Origin, Option<String>)> = routes
                .screens
                .iter()
                .filter(|(_, owner)| match owner {
                    Owner::File(f) => f == file,
                    Owner::Component(c) => declares(text, c),
                })
                .map(|(s, _)| (Origin::Screen(s.id.clone()), s.route.clone()))
                .collect();
            if froms.is_empty() {
                let application = self.name.clone();
                froms.push((Origin::AnyScreen { application }, None));
            }
            nav::transitions(&routes, file, text, &froms, flow);
            http::actions(self, file, text, (&clients, &sdk), &froms, flow);
        }
        flow.screens.extend(screens.into_iter().map(|(s, _)| s));
        flow.endpoints.extend(endpoints);
    }

    fn screen(&self, route: String, name: String, file: &str, line: u32) -> Screen {
        Screen {
            id: format!("{}:{route}", self.name),
            name,
            kind: ScreenKind::Page,
            route: Some(route),
            application: Some(self.name.clone()),
            file: file.into(),
            line,
            access: None,
            source: SOURCE.into(),
        }
    }

    /// An import resolved to a scanned file: relative, or through a tsconfig `paths` entry.
    fn resolve(&self, from: &str, spec: &str) -> Option<&str> {
        if spec.starts_with('.') {
            return self.module(from, spec);
        }
        self.aliases.iter().find_map(|a| {
            let rest = alias_rest(a, spec)?;
            a.targets
                .iter()
                .find_map(|t| self.file_at(&t.replace('*', rest)))
        })
    }

    /// The imports of `text` that name a project file (relative, or an alias
    /// pattern, `@/` or `~/`) no scanned file satisfies. A bare package name is
    /// not one.
    fn missing_imports(&self, file: &str, text: &str) -> Vec<Import> {
        imports(text)
            .into_iter()
            .filter(|i| {
                let internal = i.module.starts_with('.')
                    || i.module.starts_with("@/")
                    || i.module.starts_with("~/")
                    || self
                        .aliases
                        .iter()
                        .any(|a| alias_rest(a, &i.module).is_some());
                internal && self.resolve(file, &i.module).is_none()
            })
            .collect()
    }

    /// A relative import resolved to a scanned file.
    fn module(&self, from: &str, spec: &str) -> Option<&str> {
        if !spec.starts_with('.') {
            return None;
        }
        self.file_at(&join(dir(from), spec))
    }

    /// The scanned file a module path names: with an extension, or an index file.
    fn file_at(&self, base: &str) -> Option<&str> {
        let base = base.to_string();
        let mut candidates = vec![base.clone()];
        for ext in EXTENSIONS {
            candidates.push(format!("{base}{ext}"));
            candidates.push(format!("{base}/index{ext}"));
        }
        candidates
            .into_iter()
            .find_map(|p| self.files.get_key_value(&p).map(|(k, _)| k.as_str()))
    }
}

/// A JSX start tag: its attributes, whether it closes itself, the offset after `>`.
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
    Str(String),
    /// `{…}`: the offset of the expression.
    Expr(usize),
    Bare,
}

/// The start tag whose attributes begin at `start` (after the tag name).
fn tag(text: &str, start: usize) -> Tag {
    let b = text.as_bytes();
    let mut i = start;
    let mut attrs = Vec::new();
    loop {
        i = skip_ws(b, i);
        match b.get(i) {
            None => {
                return Tag {
                    attrs,
                    self_closing: false,
                    end: b.len(),
                };
            }
            Some(b'>') => {
                return Tag {
                    attrs,
                    self_closing: false,
                    end: i + 1,
                };
            }
            Some(b'/') if b.get(i + 1) == Some(&b'>') => {
                return Tag {
                    attrs,
                    self_closing: true,
                    end: i + 2,
                };
            }
            Some(b'{') => {
                i = block_end(text, i, b'{', b'}');
                continue;
            }
            _ => {}
        }
        let len = text[i..]
            .find(|c: char| !(is_ident_char(c) || c == '-' || c == ':'))
            .unwrap_or(text.len() - i);
        if len == 0 {
            i += 1;
            continue;
        }
        let name = text[i..i + len].to_string();
        i = skip_ws(b, i + len);
        let value = if b.get(i) == Some(&b'=') {
            i = skip_ws(b, i + 1);
            match b.get(i) {
                Some(b'"' | b'\'') => {
                    let mut p = Parser { s: text, i };
                    let value = AttrValue::Str(p.string());
                    i = p.i;
                    value
                }
                Some(b'{') => {
                    let value = AttrValue::Expr(i + 1);
                    i = block_end(text, i, b'{', b'}');
                    value
                }
                _ => AttrValue::Bare,
            }
        } else {
            AttrValue::Bare
        };
        attrs.push(Attr { name, value });
    }
}

/// Offsets and names of JSX start tags (`<Name`, `<A.B`) in `text`.
fn start_tags(text: &str) -> Vec<(usize, &str)> {
    text.match_indices('<')
        .filter_map(|(i, _)| {
            let rest = &text[i + 1..];
            if !rest.starts_with(|c: char| c.is_ascii_alphabetic()) {
                return None;
            }
            let len = rest
                .find(|c: char| !(is_ident_char(c) || c == '.'))
                .unwrap_or(rest.len());
            Some((i, &rest[..len]))
        })
        .collect()
}

/// Next.js file routing, middleware and API endpoints.
mod next {
    use super::*;

    /// Path below the application's `pages/` or `app/` directory (also under `src/`).
    fn routed<'f>(app: &App, file: &'f str) -> Option<(bool, &'f str)> {
        let inner = file.strip_prefix(app.prefix.as_str())?;
        let inner = inner.strip_prefix("src/").unwrap_or(inner);
        match (inner.strip_prefix("pages/"), inner.strip_prefix("app/")) {
            (Some(rest), _) => Some((true, rest)),
            (_, Some(rest)) => Some((false, rest)),
            _ => None,
        }
    }

    fn stem(path: &str) -> &str {
        EXTENSIONS
            .iter()
            .find_map(|e| path.strip_suffix(e))
            .unwrap_or(path)
    }

    /// API routes, route handlers and middleware: server code, no screen's links or calls.
    pub(super) fn is_backend(app: &App, file: &str) -> bool {
        let inner = file.strip_prefix(app.prefix.as_str()).unwrap_or(file);
        let inner = stem(inner.strip_prefix("src/").unwrap_or(inner));
        inner == "middleware"
            || match routed(app, file) {
                Some((true, rest)) => rest.starts_with("api/"),
                Some((false, rest)) => stem(rest) == "route" || stem(rest).ends_with("/route"),
                None => false,
            }
    }

    /// Route of the directory segments; `None` for skipped segments.
    fn route_of(segments: &[&str]) -> Option<String> {
        let mut out = Vec::new();
        for s in segments {
            if s.starts_with(['_', '@']) || (s.starts_with('(') && !s.ends_with(')')) {
                return None;
            }
            if s.starts_with('(') {
                continue;
            }
            match s.strip_prefix('[').and_then(|x| x.strip_suffix(']')) {
                Some(inner) if inner.trim_start_matches('[').starts_with("...") => {
                    out.push("*".to_string())
                }
                Some(inner) => out.push(format!(":{inner}")),
                None => out.push(s.to_string()),
            }
        }
        Some(format!("/{}", out.join("/")))
    }

    /// Offset after `export default` and the identifier following it.
    fn default_export(text: &str) -> Option<(usize, &str)> {
        let pos = text.find("export default")?;
        let j = skip_ws(text.as_bytes(), pos + "export default".len());
        Some((pos, ident_at(text, j)))
    }

    fn default_name(text: &str) -> Option<String> {
        let (pos, mut word) = default_export(text)?;
        let b = text.as_bytes();
        let mut j = skip_ws(b, pos + "export default".len());
        let mut declared = false;
        while matches!(word, "async" | "function" | "class") {
            declared |= word != "async";
            j = skip_ws(b, j + word.len());
            word = ident_at(text, j);
        }
        let after = text[j + word.len()..].trim_start_matches([' ', '\t']);
        let plain = declared || after.is_empty() || after.starts_with([';', '\n', '\r']);
        (plain && !word.is_empty() && !matches!(word, "function" | "class"))
            .then(|| word.to_string())
    }

    /// `name(` right after `at` with `name` one of [`API_GUARDS`].
    fn api_guard(text: &str, at: usize) -> AccessState {
        let j = skip_ws(text.as_bytes(), at);
        let name = ident_at(text, j);
        if API_GUARDS.contains(&name) && text[j + name.len()..].starts_with('(') {
            AccessState::Protected(Condition::Custom(name.into()))
        } else {
            AccessState::Undeclared
        }
    }

    /// What a routed file is.
    enum Routed {
        Page,
        /// `pages/api/**`: one handler for every method.
        Api,
        /// `app/**/route.*`: one handler per exported method.
        Handler,
    }

    /// The kind and route of a file below `pages/` or `app/`; `None` for others.
    fn route_file(app: &App, file: &str) -> Option<(Routed, String)> {
        let (pages, rest) = routed(app, file)?;
        let mut segments: Vec<&str> = stem(rest).split('/').collect();
        let kind = if pages {
            if segments.last() == Some(&"index") {
                segments.pop();
            }
            if segments.first() == Some(&"api") {
                Routed::Api
            } else {
                Routed::Page
            }
        } else {
            match segments.pop() {
                Some("page") => Routed::Page,
                Some("route") => Routed::Handler,
                _ => return None,
            }
        };
        Some((kind, route_of(&segments)?))
    }

    /// `(method, line, access)` of each handler in an API or route-handler file.
    fn handlers(kind: &Routed, text: &str) -> Vec<(String, u32, AccessState)> {
        if let Routed::Api = kind {
            return vec![match default_export(text) {
                Some((pos, _)) => (
                    ANY_METHOD.into(),
                    line_of(text, pos),
                    api_guard(text, pos + "export default".len()),
                ),
                None => (ANY_METHOD.into(), 1, AccessState::Undeclared),
            }];
        }
        let mut out = Vec::new();
        for method in HTTP_METHODS.map(|m| m.to_ascii_uppercase()) {
            for pat in [
                format!("export async function {method}("),
                format!("export function {method}("),
                format!("export const {method} ="),
            ] {
                let Some(pos) = text.find(&pat) else {
                    continue;
                };
                let access = if pat.ends_with('=') {
                    api_guard(text, pos + pat.len())
                } else {
                    AccessState::Undeclared
                };
                out.push((method.clone(), line_of(text, pos), access));
            }
        }
        out
    }

    pub(super) fn file_routes(
        app: &App,
        screens: &mut Vec<(Screen, Owner)>,
        endpoints: &mut Vec<EndpointAccess>,
    ) {
        for (file, text) in &app.files {
            let Some((kind, route)) = route_file(app, file) else {
                continue;
            };
            if let Routed::Page = kind {
                let name = default_name(text).unwrap_or_else(|| file.clone());
                let line = default_export(text).map_or(1, |(pos, _)| line_of(text, pos));
                let screen = app.screen(route, name, file, line);
                screens.push((screen, Owner::File(file.clone())));
                continue;
            }
            for (method, line, access) in handlers(&kind, text) {
                endpoints.push(EndpointAccess {
                    method,
                    path: route.clone(),
                    access,
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

    /// A matcher path without regular-expression parts.
    fn literal_matcher(m: &str) -> bool {
        let segments: Vec<&str> = m.split('/').filter(|s| !s.is_empty()).collect();
        m.starts_with('/')
            && segments
                .iter()
                .enumerate()
                .all(|(i, s)| match s.strip_prefix(':') {
                    Some(p) => {
                        let name = p.trim_end_matches(['*', '+', '?']);
                        is_name(name) && (name.len() == p.len() || i + 1 == segments.len())
                    }
                    None => s
                        .chars()
                        .all(|c| is_ident_char(c) || matches!(c, '-' | '.')),
                })
    }

    /// Whether the literal matcher `m` covers `route`.
    fn matcher_hits(m: &str, route: &str) -> bool {
        let ms: Vec<&str> = m.split('/').filter(|s| !s.is_empty()).collect();
        let rs: Vec<&str> = route.split('/').filter(|s| !s.is_empty()).collect();
        let (prefix, min_tail) = match ms.split_last() {
            Some((last, rest)) if last.starts_with(':') && last.ends_with(['*', '?']) => {
                (rest, Some(0))
            }
            Some((last, rest)) if last.starts_with(':') && last.ends_with('+') => (rest, Some(1)),
            _ => (&ms[..], None),
        };
        let len_ok = match min_tail {
            None => rs.len() == prefix.len(),
            Some(n) => rs.len() >= prefix.len() + n,
        };
        len_ok
            && prefix
                .iter()
                .zip(&rs)
                .all(|(p, r)| p.starts_with(':') || p.eq_ignore_ascii_case(r))
    }

    /// A matcher entry as a literal path: a string or `{ source: '…' }`.
    fn matcher_text(item: &Item) -> Option<String> {
        let value = match &item.value {
            Value::Obj(fields) => &fields.iter().find(|(k, _)| k == "source")?.1.value,
            v => v,
        };
        match value {
            Value::Str(s) if literal_matcher(s) => Some(s.clone()),
            _ => None,
        }
    }

    /// The literal `matcher` paths of `config`; `None` without a matcher, since
    /// Next.js then runs the middleware on every path.
    fn matchers(file: &str, text: &str, out: &mut Vec<UiFlowDiagnostic>) -> Option<Vec<String>> {
        let b = text.as_bytes();
        let colon = words(text, "matcher")
            .into_iter()
            .map(|pos| skip_ws(b, pos + "matcher".len()))
            .find(|&j| b.get(j) == Some(&b':'))?;
        let item = Parser {
            s: text,
            i: colon + 1,
        }
        .value(0);
        let list: Vec<&Item> = match &item.value {
            Value::Arr(items) => items.iter().collect(),
            _ => vec![&item],
        };
        let mut found = Vec::new();
        for it in list {
            match matcher_text(it) {
                Some(m) => found.push(m),
                None => {
                    let message = format!(
                        "Middleware matcher is not a literal path; the screens it guards are unknown: {}",
                        it.source(text)
                    );
                    diag(out, file, line_of(text, it.at), message);
                }
            }
        }
        Some(found)
    }

    pub(super) fn middleware(
        app: &App,
        screens: &mut [(Screen, Owner)],
        endpoints: &mut [EndpointAccess],
        out: &mut Vec<UiFlowDiagnostic>,
    ) {
        let Some((file, text)) = ["middleware", "src/middleware"]
            .iter()
            .flat_map(|n| [".ts", ".js"].map(|e| format!("{}{n}{e}", app.prefix)))
            .find_map(|f| app.files.get_key_value(&f))
        else {
            return;
        };
        let matchers = matchers(file, text, out);
        let hits = |route: &str| {
            matchers
                .as_ref()
                .is_none_or(|ms| ms.iter().any(|m| matcher_hits(m, route)))
        };
        let mw = Condition::Custom("middleware".into());
        for (screen, _) in screens.iter_mut() {
            if screen.route.as_deref().is_some_and(hits) {
                screen.access = Some(match screen.access.take() {
                    None => mw.clone(),
                    Some(c) => Condition::All(vec![c, mw.clone()]),
                });
            }
        }
        for e in endpoints.iter_mut().filter(|e| hits(&e.path)) {
            e.access = AccessState::Protected(match &e.access {
                AccessState::Protected(c) => Condition::All(vec![c.clone(), mw.clone()]),
                _ => mw.clone(),
            });
        }
    }
}

/// React Router route trees from JSX and route objects.
mod router {
    use super::*;

    const ROUTER_CALLS: [&str; 4] = [
        "createBrowserRouter(",
        "createHashRouter(",
        "createMemoryRouter(",
        "useRoutes(",
    ];

    /// One route in either form; `path` is `Err(source)` when not literal.
    #[derive(Default)]
    struct Node {
        at: usize,
        path: Option<Result<String, String>>,
        index: bool,
        element: Option<String>,
        component: Option<String>,
        children: Vec<Node>,
    }

    struct Ctx<'a> {
        app: &'a App,
        file: &'a str,
        text: &'a str,
        screens: &'a mut Vec<(Screen, Owner)>,
        out: &'a mut Vec<UiFlowDiagnostic>,
    }

    impl Ctx<'_> {
        fn diag(&mut self, at: usize, message: String) {
            diag(self.out, self.file, line_of(self.text, at), message);
        }
    }

    pub(super) fn routes(
        app: &App,
        file: &str,
        text: &str,
        screens: &mut Vec<(Screen, Owner)>,
        out: &mut Vec<UiFlowDiagnostic>,
    ) {
        let mut ctx = Ctx {
            app,
            file,
            text,
            screens,
            out,
        };
        let mapped = mapped(text);
        let spans: Vec<(usize, usize)> = mapped.iter().map(|m| m.span).collect();
        let mut roots = jsx(text, &spans);
        roots.extend(objects(&mut ctx));
        for root in &roots {
            walk(&mut ctx, root, "", &[], 0);
        }
        for m in &mapped {
            map_routes(&mut ctx, m);
        }
    }

    /// `NAME.map(cb)` whose callback returns `<Route {...x}>`: where the call
    /// is, its callback's span and the Route tag.
    struct Mapped<'t> {
        at: usize,
        name: Option<String>,
        span: (usize, usize),
        route: (&'t str, Tag),
    }

    fn mapped(text: &str) -> Vec<Mapped<'_>> {
        let mut out = Vec::new();
        for (pos, _) in text.match_indices(".map(") {
            let (open, before) = (pos + 4, &text[..pos]);
            let span = (open, close(text, open));
            let route = route_tags(text, span.0, span.1).into_iter().find_map(|i| {
                let tag = tag(text, i + "<Route".len());
                let head = &text[i..tag.end];
                head.contains("{...").then_some((head, tag))
            });
            let Some(route) = route else {
                continue;
            };
            let name =
                trailing_ident(before).filter(|n| !before[..before.len() - n.len()].ends_with('.'));
            out.push(Mapped {
                at: pos,
                name,
                span,
                route,
            });
        }
        out
    }

    /// Offsets of the `<Route` start tags in `text[from..to]`.
    fn route_tags(text: &str, from: usize, to: usize) -> Vec<usize> {
        text[from..to.min(text.len())]
            .match_indices("<Route")
            .map(|(i, _)| from + i)
            .filter(|i| {
                !text[i + "<Route".len()..].starts_with(|c: char| is_ident_char(c) || c == '.')
            })
            .collect()
    }

    /// The routes of the array literal a `.map` call renders as `<Route {...x}>`:
    /// local, or imported (default or named) from a scanned file. Any other
    /// shape is a gap.
    fn map_routes(ctx: &mut Ctx, m: &Mapped) {
        let (app, text, file) = (ctx.app, ctx.text, ctx.file);
        let (head, tag) = &m.route;
        let plain = tag.attrs.iter().all(|a| match (&a.name[..], &a.value) {
            ("key", _) => true,
            ("element", AttrValue::Expr(at)) => Parser { s: text, i: *at }
                .value(0)
                .source(text)
                .ends_with("element"),
            _ => false,
        });
        let Some(name) = m.name.as_deref().filter(|_| plain) else {
            let message = format!(
                "Routes rendered by .map are not an array literal of route objects; they are unknown: {head}"
            );
            return ctx.diag(m.at, message);
        };
        let gap = |ctx: &mut Ctx, why: String| {
            let message = format!("Route array {name} is {why}; its routes are unknown.");
            ctx.diag(m.at, message);
        };
        let found = match local_array(text, name) {
            Some(at) => Some((file, text, at)),
            None => {
                let Some(import) = imports(text).into_iter().find(|i| i.local == name) else {
                    return gap(ctx, "not declared in this file or imported".into());
                };
                let Some(target) = app.resolve(file, &import.module) else {
                    let why = format!("imported from {}, which resolves to no file", import.module);
                    return gap(ctx, why);
                };
                let (target, other) = app.files.get_key_value(target).expect("scanned file");
                let at = match import.imported.as_deref() {
                    None => default_array(other),
                    Some("*") => None,
                    Some(n) => local_array(other, n),
                };
                at.map(|at| (target.as_str(), other.as_str(), at))
            }
        };
        let Some((file, text, at)) = found else {
            return gap(ctx, "not declared as an array literal".into());
        };
        let mut other = Ctx {
            app,
            file,
            text,
            screens: &mut *ctx.screens,
            out: &mut *ctx.out,
        };
        let items = Parser { s: text, i: at }.array(0);
        let nodes: Vec<Node> = items
            .iter()
            .filter_map(|it| object(&mut other, it, 0))
            .collect();
        for node in &nodes {
            walk(&mut other, node, "", &[], 0);
        }
    }

    /// The offset of the array `export default` names or holds.
    fn default_array(text: &str) -> Option<usize> {
        let b = text.as_bytes();
        let at = skip_ws(b, text.find("export default ")? + "export default ".len());
        if b.get(at) == Some(&b'[') {
            return Some(at);
        }
        local_array(text, ident_at(text, at))
    }

    /// The route a `<Route …>` start tag at `pos` declares, without children.
    fn jsx_node(text: &str, pos: usize, tag: &Tag) -> Node {
        let mut node = Node {
            at: pos,
            ..Node::default()
        };
        let expr = |at: usize| Parser { s: text, i: at }.value(0);
        for attr in &tag.attrs {
            match (attr.name.as_str(), &attr.value) {
                ("path", AttrValue::Str(s)) => node.path = Some(Ok(s.clone())),
                ("path", AttrValue::Expr(at)) => {
                    let item = expr(*at);
                    node.path = Some(match item.value {
                        Value::Str(s) => Ok(s),
                        _ => Err(item.source(text)),
                    });
                }
                ("index", AttrValue::Bare) => node.index = true,
                ("index", AttrValue::Expr(at)) => node.index = expr(*at).source(text) == "true",
                ("element", AttrValue::Expr(at)) => node.element = Some(expr(*at).source(text)),
                ("Component", AttrValue::Expr(at)) => {
                    node.component = Some(expr(*at).source(text)).filter(|n| is_name(n))
                }
                _ => {}
            }
        }
        node
    }

    fn jsx(text: &str, skip: &[(usize, usize)]) -> Vec<Node> {
        let mut events: Vec<(usize, bool)> = text
            .match_indices("<Route")
            .filter(|(i, _)| {
                !text[i + "<Route".len()..].starts_with(|c: char| is_ident_char(c) || c == '.')
            })
            .map(|(i, _)| (i, true))
            .chain(text.match_indices("</Route>").map(|(i, _)| (i, false)))
            .filter(|(i, _)| !skip.iter().any(|(from, to)| (*from..*to).contains(i)))
            .collect();
        events.sort();
        let attach =
            |stack: &mut Vec<Node>, roots: &mut Vec<Node>, node: Node| match stack.last_mut() {
                Some(parent) => parent.children.push(node),
                None => roots.push(node),
            };
        let (mut stack, mut roots) = (Vec::new(), Vec::new());
        for (pos, open) in events {
            if !open {
                if let Some(node) = stack.pop() {
                    attach(&mut stack, &mut roots, node);
                }
                continue;
            }
            let tag = tag(text, pos + "<Route".len());
            let node = jsx_node(text, pos, &tag);
            if tag.self_closing {
                attach(&mut stack, &mut roots, node);
            } else {
                stack.push(node);
            }
        }
        while let Some(node) = stack.pop() {
            attach(&mut stack, &mut roots, node);
        }
        roots
    }

    fn objects(ctx: &mut Ctx) -> Vec<Node> {
        let (text, b) = (ctx.text, ctx.text.as_bytes());
        let mut roots = Vec::new();
        for call in ROUTER_CALLS {
            for (pos, _) in text.match_indices(call) {
                if pos > 0 && is_ident_char(b[pos - 1] as char) {
                    continue;
                }
                let j = skip_ws(b, pos + call.len());
                let at = if b.get(j) == Some(&b'[') {
                    j
                } else {
                    let name = ident_at(text, j);
                    // A call such as createRoutesFromElements(<Route …>) is read as JSX.
                    if name.is_empty() || b.get(skip_ws(b, j + name.len())) == Some(&b'(') {
                        continue;
                    }
                    match local_array(text, name) {
                        Some(at) => at,
                        None => {
                            let message = format!(
                                "Route array {name} is not declared as an array literal in this file; its routes are unknown."
                            );
                            ctx.diag(pos, message);
                            continue;
                        }
                    }
                };
                let items = Parser { s: text, i: at }.array(0);
                roots.extend(items.iter().filter_map(|it| object(ctx, it, 0)));
            }
        }
        roots
    }

    fn object(ctx: &mut Ctx, item: &Item, depth: usize) -> Option<Node> {
        let text = ctx.text;
        let Value::Obj(fields) = &item.value else {
            let message = format!(
                "Route entry is not an object literal; its routes are unknown: {}",
                item.source(text)
            );
            ctx.diag(item.at, message);
            return None;
        };
        let get = |key: &str| fields.iter().find(|(k, _)| k == key).map(|(_, v)| v);
        if let Some(v) = get("lazy") {
            let message = format!(
                "Lazy route is not read; its screen is unknown: {}",
                v.source(text)
            );
            ctx.diag(v.at, message);
        }
        let children = match get("children") {
            Some(Item {
                value: Value::Arr(items),
                ..
            }) if depth < MAX_DEPTH => items
                .iter()
                .filter_map(|it| object(ctx, it, depth + 1))
                .collect(),
            Some(v) => {
                let message = format!(
                    "Child routes are not a literal array; they are unknown: {}",
                    v.source(text)
                );
                ctx.diag(v.at, message);
                vec![]
            }
            None => vec![],
        };
        Some(Node {
            at: item.at,
            path: get("path").map(|v| match &v.value {
                Value::Str(s) => Ok(s.clone()),
                _ => Err(v.source(text)),
            }),
            index: get("index").is_some_and(|v| v.source(text) == "true"),
            element: get("element").map(|v| v.source(text)),
            component: get("Component")
                .map(|v| v.source(text))
                .filter(|n| is_name(n)),
            children,
        })
    }

    /// Guards wrapping the element, its screen component, and whether it redirects.
    fn element_parts(element: &str) -> (Vec<Condition>, Option<String>, bool) {
        let mut guards = Vec::new();
        for (_, name) in start_tags(element) {
            if GUARDS.contains(&name) {
                guards.push(Condition::Custom(name.into()));
            } else if name == "Navigate" {
                return (guards, None, true);
            } else if name.starts_with(|c: char| c.is_ascii_uppercase()) {
                return (guards, Some(name.into()), false);
            }
        }
        (guards, None, false)
    }

    /// The guards around the route's element and its screen component; an
    /// element that is neither is a diagnostic.
    fn component(ctx: &mut Ctx, node: &Node) -> (Vec<Condition>, Option<String>) {
        let (guards, element_component, redirect) = node
            .element
            .as_deref()
            .map(element_parts)
            .unwrap_or_default();
        let component = node.component.clone().or(element_component);
        if let Some(name) = &component {
            let missing = ctx.app.missing_imports(ctx.file, ctx.text);
            if let Some(i) = missing.iter().find(|i| i.local == *name) {
                let message = format!(
                    "Import {} resolves to no file; the file of screen component {name} is unknown.",
                    i.module
                );
                ctx.diag(node.at, message);
            }
        }
        if let Some(element) = &node.element
            && component.is_none()
            && guards.is_empty()
            && !redirect
        {
            let message =
                format!("Route element is not a JSX component; the screen is unknown: {element}");
            ctx.diag(node.at, message);
        }
        (guards, component)
    }

    fn push_screen(ctx: &mut Ctx, node: &Node, full: &str, name: String, guards: &[Condition]) {
        let line = line_of(ctx.text, node.at);
        let mut screen = ctx
            .app
            .screen(format!("/{full}"), name.clone(), ctx.file, line);
        screen.access = match guards {
            [] => None,
            [one] => Some(one.clone()),
            all => Some(Condition::All(all.to_vec())),
        };
        ctx.screens.push((screen, Owner::Component(name)));
    }

    fn walk(ctx: &mut Ctx, node: &Node, prefix: &str, inherited: &[Condition], depth: usize) {
        if depth >= MAX_DEPTH {
            let message = "Routes nest too deep; deeper routes are unknown.";
            return ctx.diag(node.at, message.into());
        }
        let path = match &node.path {
            None => None,
            Some(Ok(p)) => Some(p.as_str()),
            Some(Err(source)) => {
                let message = format!(
                    "Route path is not a literal; the route and its children are unknown: {source}"
                );
                return ctx.diag(node.at, message);
            }
        };
        let full = match path {
            Some(p) if p.starts_with('/') => p.trim_matches('/').to_string(),
            Some(p) => [prefix, p.trim_matches('/')]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("/"),
            None => prefix.to_string(),
        };
        let (mut guards_here, component) = component(ctx, node);
        let mut guards = inherited.to_vec();
        guards.append(&mut guards_here);
        let layout = node.children.iter().any(|c| c.index);
        if let Some(name) = component
            && (path.is_some() || node.index)
            && !layout
        {
            push_screen(ctx, node, &full, name, &guards);
        }
        for child in &node.children {
            walk(ctx, child, &full, &guards, depth + 1);
        }
    }
}

/// Links and navigation calls.
mod nav {
    use super::*;

    /// One navigation; `path` is `None` when not literal.
    struct Nav {
        line: u32,
        source: String,
        path: Option<String>,
        trigger: Option<String>,
    }

    /// A literal, template literal or `{ pathname: '…' }`.
    fn target_of(value: &Value) -> Option<String> {
        match value {
            Value::Obj(fields) => fields
                .iter()
                .find(|(k, _)| k == "pathname" || k == "to")
                .and_then(|(_, v)| v.value.literal()),
            v => v.literal(),
        }
    }

    /// `Link`, `NavLink`, default imports of `next/link` and `styled(…)` wrappers of them.
    pub(super) fn link_names(app: &App) -> BTreeSet<String> {
        let mut names: BTreeSet<String> = ["Link", "NavLink"].map(String::from).into();
        for text in app.files.values() {
            for import in imports(text) {
                let next = import.module == "next/link" && import.imported.is_none();
                let tanstack = import.module == "@tanstack/react-router"
                    && import.imported.as_deref() == Some("Link");
                if next || tanstack {
                    names.insert(import.local);
                }
            }
        }
        // Two passes: a styled wrapper of a styled wrapper.
        for _ in 0..2 {
            let wrappers: Vec<String> = app
                .files
                .values()
                .flat_map(|text| styled_wrappers(text, &names))
                .collect();
            names.extend(wrappers);
        }
        names
    }

    /// Names bound to `styled(L)` with `L` one of `names`.
    fn styled_wrappers(text: &str, names: &BTreeSet<String>) -> Vec<String> {
        words(text, "styled")
            .into_iter()
            .filter(|&pos| {
                let rest = &text[pos + "styled".len()..];
                rest.strip_prefix('(')
                    .is_some_and(|inner| names.contains(ident_at(inner, 0)))
            })
            .filter_map(|pos| {
                let lhs = text[..pos].trim_end().strip_suffix('=')?;
                trailing_ident(lhs.trim_end())
            })
            .collect()
    }

    /// `data-testid`, `id` or the text of the element.
    fn trigger(text: &str, tag: &Tag) -> Option<String> {
        for key in ["data-testid", "id"] {
            if let Some(Attr {
                value: AttrValue::Str(s),
                ..
            }) = tag.attrs.iter().find(|a| a.name == key)
            {
                return Some(s.clone());
            }
        }
        if tag.self_closing {
            return None;
        }
        let inner = text.get(tag.end..)?;
        let inner = &inner[..inner.find("</").unwrap_or(inner.len())];
        let mut label = String::new();
        // Nested tags and `{expression}` text are no label.
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

    fn links(text: &str, names: &BTreeSet<String>) -> Vec<Nav> {
        let mut out = Vec::new();
        for (pos, name) in start_tags(text) {
            if !names.contains(name.rsplit('.').next().unwrap_or(name)) {
                continue;
            }
            let tag = tag(text, pos + 1 + name.len());
            let Some(attr) = tag
                .attrs
                .iter()
                .find(|a| a.name == "href" || a.name == "to")
            else {
                continue;
            };
            let (source, path) = match &attr.value {
                AttrValue::Str(s) => (s.clone(), Some(s.clone())),
                AttrValue::Expr(at) => {
                    let item = Parser { s: text, i: *at }.value(0);
                    (item.source(text), target_of(&item.value))
                }
                AttrValue::Bare => continue,
            };
            out.push(Nav {
                line: line_of(text, pos),
                source,
                path,
                trigger: trigger(text, &tag),
            });
        }
        out
    }

    /// `router.push|replace('…')`, `navigate('…')`, `push('…')`, `redirect('…')`.
    fn calls(text: &str) -> Vec<Nav> {
        let (fns, objs) = navigators(text);
        let b = text.as_bytes();
        let mut opens = Vec::new();
        for f in &fns {
            for pos in words(text, f) {
                let j = skip_ws(b, pos + f.len());
                if b.get(j) == Some(&b'(') && !text[..pos].ends_with('.') {
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
                    line: line_of(text, pos),
                    source: item.source(text),
                    path: target_of(&item.value),
                    trigger: None,
                }
            })
            .collect()
    }

    /// Navigating functions (`navigate`, `push`, `redirect`) and router objects of a file.
    fn navigators(text: &str) -> (BTreeSet<String>, BTreeSet<String>) {
        let (mut fns, mut objs) = (BTreeSet::new(), BTreeSet::new());
        for binding in bindings(text, "useNavigate") {
            if let Binding::Name(n) = binding {
                fns.insert(n);
            }
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
        for i in imports(text) {
            match (i.module.as_str(), i.imported.as_deref()) {
                ("next/router", None) => {
                    objs.insert(i.local);
                }
                (
                    "next/navigation" | "react-router" | "react-router-dom",
                    Some("redirect" | "permanentRedirect"),
                ) => {
                    fns.insert(i.local);
                }
                _ => {}
            }
        }
        (fns, objs)
    }

    /// What navigation resolves against: the application's screens and link components.
    pub(super) struct Routes<'a> {
        pub(super) app: &'a App,
        pub(super) names: BTreeSet<String>,
        pub(super) screens: &'a [(Screen, Owner)],
    }

    fn external(path: &str) -> bool {
        path.is_empty()
            || path.contains("://")
            || ["//", "mailto:", "tel:"]
                .iter()
                .any(|p| path.starts_with(p))
    }

    impl Routes<'_> {
        /// The screen an absolute path resolves to, else why it does not.
        fn screen(&self, path: &str) -> Result<Target, String> {
            let routes = self
                .screens
                .iter()
                .filter_map(|(s, _)| Some((&s.id, s.route.as_deref()?)));
            match best_route(path, routes) {
                Ok(id) => Ok(Target::Screen(id.clone())),
                Err(tied) if tied.is_empty() => Err(format!(
                    "Navigation target {path} matches no route of application {}.",
                    self.app.name
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

        /// The target screen; `None` for an external URL or an anchor.
        fn resolve(
            &self,
            file: &str,
            nav: &Nav,
            base: Option<&str>,
            out: &mut Vec<UiFlowDiagnostic>,
        ) -> Option<Target> {
            let literal = nav.path.as_deref().map(|p| match self.app.tanstack {
                true => tanstack::route_params(p),
                false => p.to_string(),
            });
            let path = literal
                .as_deref()
                .map(|p| p.split(['?', '#']).next().unwrap_or(""));
            if path.is_some_and(external) {
                return None;
            }
            let found = match (path, base) {
                (None, _) => Err(format!(
                    "Navigation target is not a literal: {}",
                    nav.source
                )),
                (Some(p), _) if p.starts_with('/') => self.screen(p),
                (Some(p), Some(base)) => self.screen(&format!("/{}", join(base, p))),
                (Some(_), None) => Err(format!(
                    "Relative navigation {} in a component that is no route target; its base route is unknown.",
                    nav.source
                )),
            };
            Some(found.unwrap_or_else(|message| {
                diag(out, file, nav.line, message);
                Target::Unresolved(nav.source.clone())
            }))
        }
    }

    pub(super) fn transitions(
        routes: &Routes,
        file: &str,
        text: &str,
        froms: &[(Origin, Option<String>)],
        flow: &mut UiFlow,
    ) {
        let mut navs = links(text, &routes.names);
        navs.extend(calls(text));
        for (from, base) in froms {
            for nav in &navs {
                let Some(target) =
                    routes.resolve(file, nav, base.as_deref(), &mut flow.diagnostics)
                else {
                    continue;
                };
                flow.transitions.push(Transition {
                    from: from.clone(),
                    target,
                    trigger: nav.trigger.clone(),
                    file: file.into(),
                    line: nav.line,
                    condition: None,
                    source: SOURCE.into(),
                });
            }
        }
    }
}

/// TanStack Router file routes (`createFileRoute('/a/$id')`) and `beforeLoad` guards.
mod tanstack {
    use super::*;

    /// A route module: its id (`/_layout/items/$id`, `""` for the root), the
    /// `createFileRoute` offset and what its `beforeLoad` requires.
    struct Module<'f> {
        id: String,
        file: &'f str,
        text: &'f str,
        at: usize,
        guards: Vec<Condition>,
    }

    /// `$id` becomes `:id`, a bare `$` the catch-all `*`.
    pub(super) fn route_params(path: &str) -> String {
        let segment = |s: &str| match s.strip_prefix('$') {
            Some("") => "*".to_string(),
            Some(name) => format!(":{name}"),
            None => s.to_string(),
        };
        path.split('/').map(segment).collect::<Vec<_>>().join("/")
    }

    /// The route path of an id: layout (`_x`) and group (`(x)`) segments drop out.
    fn route_of(id: &str) -> String {
        let segments: Vec<String> = id
            .split('/')
            .filter(|s| {
                !s.is_empty() && !s.starts_with('_') && !(s.starts_with('(') && s.ends_with(')'))
            })
            .map(|s| route_params(s.trim_end_matches('_')))
            .collect();
        format!("/{}", segments.join("/"))
    }

    fn modules(app: &App) -> Vec<Module<'_>> {
        let mut out = Vec::new();
        for (file, text) in &app.files {
            let at = words(text, "createFileRoute").into_iter().find_map(|p| {
                let open = skip_ws(text.as_bytes(), p + "createFileRoute".len());
                (text.as_bytes().get(open) == Some(&b'(')).then(|| {
                    (
                        p,
                        Parser {
                            s: text,
                            i: open + 1,
                        }
                        .value(0),
                    )
                })
            });
            let (at, id) = match at {
                Some((p, item)) => match item.value {
                    Value::Str(id) => (p, id),
                    _ => continue,
                },
                None => match words(text, "createRootRoute")
                    .into_iter()
                    .chain(words(text, "createRootRouteWithContext"))
                    .next()
                {
                    Some(p) => (p, String::new()),
                    None => continue,
                },
            };
            out.push(Module {
                id,
                file,
                text,
                at,
                guards: Vec::new(),
            });
        }
        out
    }

    /// The condition that check `text` is true; `Ok(None)` is always true.
    fn check(text: &str, access: &Access) -> Result<Option<Condition>, String> {
        let t = text.trim();
        let t = t
            .strip_prefix('(')
            .and_then(|r| r.strip_suffix(')'))
            .unwrap_or(t)
            .trim();
        if let Some(rest) = t.strip_prefix('!')
            && !rest.starts_with('=')
        {
            return check(rest, access).map(negate);
        }
        let (name, args) = match call_parts(t) {
            Some((f, args)) => (f.to_string(), args),
            None => (t.replace("?.", "."), Vec::new()),
        };
        if !is_name(&name) {
            return Err(t.to_string());
        }
        match access.get(&name).map(|m| m.at_use_site(!args.is_empty())) {
            None => Ok(Some(Condition::Custom(name))),
            Some(AccessMeaning::Open) => Ok(None),
            Some(AccessMeaning::Authenticated) => Ok(Some(Condition::Authenticated)),
            Some(AccessMeaning::Never) => Ok(Some(Condition::Never)),
            Some(AccessMeaning::Role(n)) => Ok(Some(Condition::Role(n.clone()))),
            Some(AccessMeaning::Policy(n)) => Ok(Some(Condition::Policy(n.clone()))),
            // shortcut: `$arg` meanings are not read in beforeLoad checks, add when a project needs them
            Some(_) => Err(t.to_string()),
        }
    }

    fn negate(c: Option<Condition>) -> Option<Condition> {
        match c {
            None => Some(Condition::Never),
            Some(Condition::Never) => None,
            Some(Condition::Not(inner)) => Some(*inner),
            Some(c) => Some(Condition::Not(Box::new(c))),
        }
    }

    /// A readable function: the interior of its block body, or a function name.
    enum Callable {
        Body(usize, usize),
        Reference(String),
        Unreadable,
    }

    /// The interior of the `{ … }` after offset `at` (a `: Type` may come first).
    fn block_after(m: &str, at: usize) -> Callable {
        let b = m.as_bytes();
        let j = skip_ws(b, at);
        let open = match b.get(j) {
            Some(b'{') => j,
            Some(b':') => match m[j..].find('{') {
                Some(i) => j + i,
                None => return Callable::Unreadable,
            },
            _ => return Callable::Unreadable,
        };
        Callable::Body(open + 1, block_end(m, open, b'{', b'}') - 1)
    }

    /// The function expression starting at `at`: `async`, `function`, an arrow with a
    /// block body, or a name.
    fn callable_at(m: &str, at: usize) -> Callable {
        let b = m.as_bytes();
        let mut i = skip_ws(b, at);
        if ident_at(m, i) == "async" {
            i = skip_ws(b, i + "async".len());
        }
        let arrow = |after_params: usize| {
            let j = skip_ws(b, after_params);
            if !m[j..].starts_with("=>") {
                return Callable::Unreadable;
            }
            match b.get(skip_ws(b, j + 2)) {
                Some(b'{') => block_after(m, j + 2),
                _ => Callable::Unreadable,
            }
        };
        if ident_at(m, i) == "function" {
            i = skip_ws(b, i + "function".len());
            i = skip_ws(b, i + ident_at(m, i).len());
            return match b.get(i) {
                Some(b'(') => block_after(m, block_end(m, i, b'(', b')')),
                _ => Callable::Unreadable,
            };
        }
        if b.get(i) == Some(&b'(') {
            return arrow(block_end(m, i, b'(', b')'));
        }
        let name = ident_at(m, i);
        let j = skip_ws(b, i + name.len());
        if name.is_empty() {
            Callable::Unreadable
        } else if m[j..].starts_with("=>") {
            arrow(i + name.len())
        } else if matches!(b.get(j), Some(b',' | b'}' | b'\n') | None) {
            Callable::Reference(name.to_string())
        } else {
            Callable::Unreadable
        }
    }

    /// The body of `function name(…) {…}` or `const name = (…) => {…}` in `m`.
    fn declared_body(m: &str, name: &str) -> Option<(usize, usize)> {
        let b = m.as_bytes();
        for pos in words(m, name) {
            let before = m[..pos].trim_end();
            let j = skip_ws(b, pos + name.len());
            let callable = if before.ends_with("function") && b.get(j) == Some(&b'(') {
                block_after(m, block_end(m, j, b'(', b')'))
            } else if matches!(
                trailing_ident(before).as_deref(),
                Some("const" | "let" | "var")
            ) && b.get(j) == Some(&b'=')
            {
                callable_at(m, j + 1)
            } else {
                continue;
            };
            return match callable {
                Callable::Body(s, e) => Some((s, e)),
                _ => None,
            };
        }
        None
    }

    /// How a module's `beforeLoad` is written; `None` when it has none.
    fn before_load(m: &str) -> Option<(usize, Callable)> {
        let b = m.as_bytes();
        let pos = words(m, "beforeLoad")
            .into_iter()
            .find(|&p| !m[..p].trim_end().ends_with('.'))?;
        let j = skip_ws(b, pos + "beforeLoad".len());
        let callable = match b.get(j) {
            Some(b':') => callable_at(m, j + 1),
            Some(b'(') => block_after(m, block_end(m, j, b'(', b')')),
            _ => Callable::Unreadable,
        };
        Some((pos, callable))
    }

    /// The statements of one `beforeLoad` body, read with fixed rules: `if`/`else`
    /// chains, early `return`, `throw redirect(…)`. A redirect anywhere else is not read.
    struct Walker<'a> {
        m: &'a str,
        file: &'a str,
        access: &'a Access,
        out: &'a mut Vec<UiFlowDiagnostic>,
        /// `not (path conditions)` for each redirect found.
        found: Vec<Condition>,
    }

    /// How control leaves a statement list.
    #[derive(Clone, Copy, PartialEq)]
    enum Ended {
        /// It runs on.
        No,
        /// It throws a redirect under exactly the condition the list started with.
        Redirect,
        /// It returns, throws, or redirects under a narrower condition.
        Stop,
    }

    fn has_redirect(text: &str) -> bool {
        words(text, "redirect")
            .into_iter()
            .any(|p| text[p + "redirect".len()..].trim_start().starts_with('('))
    }

    /// Whether the line break at `k` continues the statement that began at `i`.
    fn continues(m: &str, i: usize, k: usize) -> bool {
        let b = m.as_bytes();
        let last = m[i..k].trim_end().bytes().last();
        let next = b.get(skip_ws(b, k)).copied();
        last.is_some_and(|c| b",(=+-*/&|?:.!<>".contains(&c))
            || next.is_some_and(|c| b".?:&|+*/".contains(&c))
    }

    /// End of the simple statement at `i`: `;`, a line break, or the closing `}`.
    fn simple_end(m: &str, i: usize) -> usize {
        let b = m.as_bytes();
        let (mut depth, mut k) = (0usize, i);
        while k < b.len() {
            match b[k] {
                b'\'' | b'"' | b'`' => {
                    k = Parser { s: m, i: k }.string_end();
                    continue;
                }
                b'(' | b'[' | b'{' => depth += 1,
                b'}' if depth == 0 => return k,
                b')' | b']' | b'}' => depth = depth.saturating_sub(1),
                b';' if depth == 0 => return k + 1,
                b'\n' if depth == 0 && !continues(m, i, k) => return k,
                _ => {}
            }
            k += 1;
        }
        b.len()
    }

    impl Walker<'_> {
        /// End of the statement at `i`: a block, an `if` chain or a simple statement.
        fn stmt_end(&self, i: usize) -> usize {
            let (m, b) = (self.m, self.m.as_bytes());
            if b.get(i) == Some(&b'{') {
                return block_end(m, i, b'{', b'}');
            }
            if ident_at(m, i) == "if" {
                let paren = skip_ws(b, i + 2);
                let after = self.stmt_end(skip_ws(b, block_end(m, paren, b'(', b')')));
                let j = skip_ws(b, after);
                return match ident_at(m, j) {
                    "else" => self.stmt_end(skip_ws(b, j + 4)),
                    _ => after,
                };
            }
            simple_end(m, i)
        }

        /// The condition `text` as a path element; `None` is always true.
        fn cond(&mut self, text: &str, at: usize) -> Option<Condition> {
            match check(text, self.access) {
                Ok(c) => c,
                Err(t) => {
                    let message = format!(
                        "Check {t} is not a name or listed in [views.access]; the condition is unknown."
                    );
                    diag(self.out, self.file, line_of(self.m, at), message);
                    Some(Condition::Unknown(t))
                }
            }
        }

        /// A redirect reached under `path`: the screen requires that `path` does not hold.
        fn reached(&mut self, path: &[Option<Condition>]) {
            let held: Vec<Condition> = path.iter().flatten().cloned().collect();
            if held.is_empty() || held.contains(&Condition::Never) {
                return;
            }
            self.found.push(match held.len() {
                1 => negate(held.into_iter().next()).expect("a single condition has a negation"),
                _ => Condition::Not(Box::new(Condition::All(held))),
            });
        }

        /// The `if` at `i` (ending at `end`): both branches walked, and what follows
        /// it learns from a branch that always leaves.
        fn walk_if(
            &mut self,
            (i, end): (usize, usize),
            path: &mut Vec<Option<Condition>>,
        ) -> Result<Ended, String> {
            let (m, b) = (self.m, self.m.as_bytes());
            let paren = skip_ws(b, i + 2);
            if b.get(paren) != Some(&b'(') {
                return Err(format!("if without a condition: {}", m[i..end].trim()));
            }
            let ce = block_end(m, paren, b'(', b')');
            let c = self.cond(&m[paren + 1..ce - 1], i);
            let then_start = skip_ws(b, ce);
            let then_end = self.stmt_end(then_start).min(end);
            let j = skip_ws(b, then_end);
            path.push(c.clone());
            let then_ends = self.walk(then_start, then_end, path);
            path.pop();
            let else_ends = if ident_at(m, j) == "else" && j < end {
                path.push(negate(c.clone()));
                let r = self.walk(skip_ws(b, j + 4), end, path);
                path.pop();
                r?
            } else {
                Ended::No
            };
            // A branch that redirects under exactly its condition already makes the
            // screen require the opposite: the path needs no more.
            Ok(match (then_ends?, else_ends) {
                (Ended::No, Ended::No)
                | (Ended::Redirect, Ended::No)
                | (Ended::No, Ended::Redirect) => Ended::No,
                (Ended::Stop, Ended::No) => {
                    path.push(negate(c));
                    Ended::No
                }
                (Ended::No, Ended::Stop) => {
                    path.push(c);
                    Ended::No
                }
                _ => Ended::Stop,
            })
        }

        /// A statement that holds `redirect(`: only `throw redirect(…)` is read.
        fn throw_redirect(
            &mut self,
            stmt: &str,
            path: &[Option<Condition>],
        ) -> Result<Ended, String> {
            let rest = stmt.trim_start().strip_prefix("throw").map(str::trim_start);
            if !rest.is_some_and(|r| r.starts_with("redirect")) {
                return Err(format!(
                    "redirect outside `throw redirect(…)`: {}",
                    stmt.trim()
                ));
            }
            self.reached(path);
            Ok(Ended::Redirect)
        }

        /// How control leaves the statement `i..end`.
        fn statement(
            &mut self,
            (i, end): (usize, usize),
            path: &mut Vec<Option<Condition>>,
        ) -> Result<Ended, String> {
            let (m, stmt) = (self.m, &self.m[i..end]);
            Ok(match ident_at(m, i) {
                _ if m.as_bytes()[i] == b'{' => self.walk(i + 1, end - 1, path)?,
                "if" => self.walk_if((i, end), path)?,
                _ if has_redirect(stmt) => self.throw_redirect(stmt, path)?,
                "throw" | "return" => Ended::Stop,
                _ => Ended::No,
            })
        }

        /// Walks the statements in `s..e` and tells how control leaves them.
        fn walk(
            &mut self,
            s: usize,
            e: usize,
            path: &mut Vec<Option<Condition>>,
        ) -> Result<Ended, String> {
            let b = self.m.as_bytes();
            let base = path.len();
            let (mut i, mut ended) = (s, Ended::No);
            loop {
                i = skip_ws(b, i);
                while b.get(i) == Some(&b';') {
                    i = skip_ws(b, i + 1);
                }
                if i >= e {
                    break;
                }
                let end = self.stmt_end(i).min(e);
                let here = self.statement((i, end), path)?;
                if here != Ended::No {
                    // Only a redirect under the walk's own start path is exact.
                    let exact = here == Ended::Redirect && path.len() == base;
                    ended = if exact { here } else { Ended::Stop };
                    break;
                }
                i = end;
            }
            path.truncate(base);
            Ok(ended)
        }
    }

    /// What the module's `beforeLoad` requires: each redirect needs `not (the
    /// conditions of the `if`s it sits under)`. A `beforeLoad` that is present but
    /// not read in full adds an unknown condition and a gap, never none.
    fn guards(
        app: &App,
        m: &Module,
        access: &Access,
        out: &mut Vec<UiFlowDiagnostic>,
    ) -> Vec<Condition> {
        let masked = mask(m.text, true);
        let Some((pos, callable)) = before_load(&masked) else {
            return Vec::new();
        };
        let head = masked[pos..]
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .trim_end_matches(',');
        // The text and body to read: the module's own, or a declared function's.
        let mut other = None;
        let body = match callable {
            Callable::Body(s, e) => Some((s, e)),
            Callable::Reference(name) => declared_body(&masked, &name).or_else(|| {
                let import = imports(m.text).into_iter().find(|i| i.local == name)?;
                let file = app.resolve(m.file, &import.module)?;
                let text = mask(app.files.get(file)?, true);
                let range = declared_body(&text, import.imported.as_deref()?)?;
                other = Some((file.to_string(), text));
                Some(range)
            }),
            Callable::Unreadable => None,
        };
        let (file, text) = match &other {
            Some((f, t)) => (f.as_str(), t.as_str()),
            None => (m.file, masked.as_str()),
        };
        let mut walker = Walker {
            m: text,
            file,
            access,
            out,
            found: Vec::new(),
        };
        let read = match body {
            Some((s, e)) => walker.walk(s, e, &mut Vec::new()).map(|_| ()),
            None => Err(
                "beforeLoad is not a function with a block body or a function declared in the scan"
                    .into(),
            ),
        };
        let mut found = walker.found;
        if let Err(why) = read {
            let message = format!("{why}; the module's access is unknown.");
            diag(out, m.file, line_of(&masked, pos), message);
            found.push(Condition::Unknown(head.to_string()));
        }
        found
    }

    fn all(conditions: Vec<Condition>) -> Option<Condition> {
        let mut kept: Vec<Condition> = Vec::new();
        for c in conditions {
            if !kept.contains(&c) {
                kept.push(c);
            }
        }
        match kept.len() {
            0 => None,
            1 => kept.pop(),
            _ => Some(Condition::All(kept)),
        }
    }

    /// Screens of the route modules; `false` when the app has none.
    pub(super) fn file_routes(
        app: &App,
        access: &Access,
        screens: &mut Vec<(Screen, Owner)>,
        out: &mut Vec<UiFlowDiagnostic>,
    ) -> bool {
        let mut mods = modules(app);
        for m in &mut mods {
            m.guards = guards(app, m, access, out);
        }
        for m in &mods {
            let layout = m.id.is_empty() || m.text.contains("<Outlet");
            if layout {
                continue;
            }
            let own = format!("{}/", m.id);
            let mut parents: Vec<&Module> = mods
                .iter()
                .filter(|p| p.id.is_empty() || own.starts_with(&format!("{}/", p.id)))
                .collect();
            parents.sort_by_key(|p| p.id.len());
            let conditions = parents.iter().flat_map(|p| p.guards.clone()).collect();
            let name = words(m.text, "component")
                .into_iter()
                .find_map(|p| {
                    let colon = skip_ws(m.text.as_bytes(), p + "component".len());
                    (m.text.as_bytes().get(colon) == Some(&b':'))
                        .then(|| ident_at(m.text, skip_ws(m.text.as_bytes(), colon + 1)))
                })
                .filter(|n| !n.is_empty())
                .map_or_else(|| m.file.to_string(), str::to_string);
            let mut screen = app.screen(route_of(&m.id), name, m.file, line_of(m.text, m.at));
            screen.access = all(conditions);
            screens.push((screen, Owner::File(m.file.to_string())));
        }
        !mods.is_empty()
    }
}

/// hey-api (`@hey-api/openapi-ts`) generated clients, read one hop.
mod sdk {
    use super::*;

    /// `Class.method` or `function` to its call, or why it is unknown.
    pub(super) type Index = BTreeMap<String, Result<Call, String>>;

    const VERBS: [&str; 7] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];

    /// The indexed client and the modules that export it: module file to exported
    /// name to the name in the generated file.
    pub(super) struct Sdk {
        index: Index,
        exports: Exports,
    }

    /// `OpenAPI.BASE` (`core/OpenAPI.ts`) as a path prefix, else `""`.
    fn base(app: &App) -> String {
        let found = app
            .files
            .iter()
            .find(|(f, _)| f.ends_with("core/OpenAPI.ts"));
        found
            .and_then(|(_, text)| {
                let colon = words(text, "BASE").into_iter().find_map(|p| {
                    let j = skip_ws(text.as_bytes(), p + 4);
                    (text.as_bytes().get(j) == Some(&b':')).then_some(j + 1)
                })?;
                Parser { s: text, i: colon }.value(0).value.literal()
            })
            .map_or(String::new(), |b| {
                url_path(&b).trim_end_matches('/').to_string()
            })
    }

    /// `(key, offset of the line)` of each exported function or static method.
    fn entries(text: &str) -> Vec<(String, usize)> {
        let (mut class, mut at, mut out) = (String::new(), 0, Vec::new());
        for line in text.split_inclusive('\n') {
            let t = line.trim_start();
            if let Some(rest) = t.strip_prefix("export class ") {
                class = ident_at(rest, 0).to_string();
            } else if let Some(rest) = t
                .strip_prefix("public static ")
                .or_else(|| t.strip_prefix("static "))
            {
                let name = ident_at(rest.strip_prefix("async ").unwrap_or(rest), 0);
                out.push((format!("{class}.{name}"), at));
            } else if let Some(rest) = t
                .strip_prefix("export const ")
                .or_else(|| t.strip_prefix("export function "))
                .or_else(|| t.strip_prefix("export async function "))
            {
                out.push((ident_at(rest, 0).to_string(), at));
            }
            at += line.len();
        }
        out
    }

    /// The value of `key:` in `body`.
    fn field(body: &str, key: &str) -> Option<Item> {
        words(body, key).into_iter().find_map(|p| {
            let j = skip_ws(body.as_bytes(), p + key.len());
            (body.as_bytes().get(j) == Some(&b':')).then(|| Parser { s: body, i: j + 1 }.value(0))
        })
    }

    /// `{id}` holes become `:param`.
    fn holes(url: &str) -> String {
        let mut out = String::new();
        let mut open = false;
        for c in url.chars() {
            match (c, open) {
                ('{', _) => {
                    open = true;
                    out.push_str(PARAM);
                }
                ('}', true) => open = false,
                (_, false) => out.push(c),
                _ => {}
            }
        }
        out
    }

    fn verb(m: &str) -> Result<String, String> {
        let upper = m.to_ascii_uppercase();
        match VERBS.contains(&upper.as_str()) {
            true => Ok(upper),
            false => Err(format!("method {m} is not an HTTP verb")),
        }
    }

    /// The string `member` of the enum body `{ A = 'a', … }` at `open`.
    fn enum_member(text: &str, open: usize, member: &str) -> Option<String> {
        let body = &text[open + 1..block_end(text, open, b'{', b'}') - 1];
        body.split(',').find_map(|part| {
            let (name, value) = part.split_once('=')?;
            let item = Parser { s: value, i: 0 }.value(0);
            match item.value {
                Value::Str(v) if name.trim() == member => Some(v),
                _ => None,
            }
        })
    }

    /// The string `member` of the object literal after `=` at `eq`.
    fn object_member(text: &str, eq: usize, member: &str) -> Option<String> {
        let Value::Obj(fields) = (Parser { s: text, i: eq + 1 }.value(0)).value else {
            return None;
        };
        fields.into_iter().find_map(|(k, v)| match v.value {
            Value::Str(s) if k == member => Some(s),
            _ => None,
        })
    }

    /// The string `member` of `enum obj { … }` or `const obj = { … }` in the scan.
    fn member_value(app: &App, obj: &str, member: &str) -> Option<String> {
        let declared = |text: &str, p: usize| {
            let before = text[..p].trim_end();
            let next = skip_ws(text.as_bytes(), p + obj.len());
            let at = text.as_bytes().get(next).copied();
            match (at, trailing_ident(before).as_deref()) {
                (Some(b'{'), Some("enum")) => enum_member(text, next, member),
                (Some(b'='), Some("const" | "let" | "var")) => object_member(text, next, member),
                _ => None,
            }
        };
        app.files
            .values()
            .find_map(|text| words(text, obj).into_iter().find_map(|p| declared(text, p)))
    }

    /// `method: Method.DELETE` (the enum or const object's value), `method: 'delete'`
    /// or a `.delete<…>(` call; only HTTP verbs count.
    fn method(app: &App, body: &str) -> Result<String, String> {
        let Some(item) = field(body, "method") else {
            return HTTP_METHODS
                .into_iter()
                .filter_map(|m| {
                    let at = [format!(".{m}<"), format!(".{m}(")]
                        .iter()
                        .filter_map(|p| body.find(p.as_str()))
                        .min()?;
                    Some((at, m))
                })
                .min()
                .map(|(_, m)| m.to_ascii_uppercase())
                .ok_or_else(|| "has no literal method".into());
        };
        match item.value {
            Value::Str(m) => verb(&m),
            Value::Raw(r) => {
                let (obj, member) = r.trim().split_once('.').unwrap_or(("", ""));
                match (is_name(obj) && is_name(member))
                    .then(|| member_value(app, obj, member))
                    .flatten()
                {
                    Some(v) => verb(&v),
                    None => Err(format!(
                        "method {} is not a literal or a declared enum value",
                        r.trim()
                    )),
                }
            }
            _ => Err("method is not a literal".into()),
        }
    }

    pub(super) fn index(app: &App) -> Index {
        let (base, mut out) = (base(app), Index::new());
        for (_, text) in app.files.iter().filter(|(f, _)| f.ends_with("sdk.gen.ts")) {
            let entries = entries(text);
            for (i, (key, start)) in entries.iter().enumerate() {
                let end = entries.get(i + 1).map_or(text.len(), |e| e.1);
                let body = &text[*start..end];
                // An export without a `url` field is no operation.
                let Some(url) = field(body, "url") else {
                    continue;
                };
                let call = match (url.value.literal(), method(app, body)) {
                    (Some(url), Ok(method)) => Ok(Call::Http {
                        method,
                        path: format!("{base}{}", url_path(&holes(&url))),
                    }),
                    (None, _) => Err(format!("{key} has a url that is not a literal")),
                    (_, Err(why)) => Err(format!("{key} {why}")),
                };
                out.insert(key.clone(), call);
            }
        }
        out
    }

    /// `(module, names)` of each `export * from` (`None`) and `export { a as b } from`.
    type ReExport = (String, Option<Vec<(String, String)>>);

    fn reexports(text: &str) -> Vec<ReExport> {
        words(text, "export")
            .into_iter()
            .filter(|&p| text[p + 6..].trim_start().starts_with(['*', '{']))
            .filter_map(|p| {
                let (clause, module) = import_at(&text[p + 6..])?;
                if clause == "*" {
                    return Some((module, None));
                }
                let inner = clause.strip_prefix('{')?.strip_suffix('}')?;
                let names = inner
                    .split(',')
                    .map(str::trim)
                    .filter(|n| !n.is_empty() && !n.starts_with("type "))
                    .map(|n| match n.split_once(" as ") {
                        Some((orig, exported)) => (orig.trim().into(), exported.trim().into()),
                        None => (n.into(), n.into()),
                    })
                    .collect();
                Some((module, Some(names)))
            })
            .collect()
    }

    type Exports = BTreeMap<String, BTreeMap<String, String>>;
    type Names = Vec<(String, String)>;
    /// Per re-exporting module: the modules it re-exports from, and which names.
    type Chains<'a> = Vec<(&'a str, Vec<(&'a str, Option<Names>)>)>;

    /// The modules of `app` with `export … from` of a module in the scan.
    fn chains<'a>(app: &'a App, generated: &Exports) -> Chains<'a> {
        let resolved = |f: &'a str, text: &str| -> Vec<(&'a str, Option<Names>)> {
            reexports(text)
                .into_iter()
                .filter_map(|(spec, names)| Some((app.resolve(f, &spec)?, names)))
                .collect()
        };
        (app.files.iter())
            .filter(|(f, _)| !generated.contains_key(*f))
            .map(|(f, text)| (f.as_str(), resolved(f, text)))
            .filter(|(_, items)| !items.is_empty())
            .collect()
    }

    /// Adds to `got` what `from` exports, all of it or the listed names.
    fn carry(
        from: &BTreeMap<String, String>,
        names: &Option<Names>,
        got: &mut BTreeMap<String, String>,
    ) {
        let listed: Vec<(&String, &String)> = match names {
            None => from.iter().collect(),
            Some(names) => (names.iter())
                .filter_map(|(orig, exported)| Some((exported, from.get(orig)?)))
                .collect(),
        };
        for (exported, original) in listed {
            got.entry(exported.clone()).or_insert(original.clone());
        }
    }

    /// One round over the re-exporting modules; whether anything was added.
    fn propagate(exports: &mut Exports, chains: &Chains) -> bool {
        let mut changed = false;
        for (file, items) in chains {
            let mut got = exports.get(*file).cloned().unwrap_or_default();
            for (target, names) in items {
                if let Some(from) = exports.get(*target) {
                    carry(from, names, &mut got);
                }
            }
            if !got.is_empty() && exports.get(*file) != Some(&got) {
                exports.insert(file.to_string(), got);
                changed = true;
            }
        }
        changed
    }

    impl Sdk {
        pub(super) fn new(app: &App) -> Sdk {
            let index = index(app);
            let names: BTreeMap<String, String> = (index.keys())
                .map(|k| k.split('.').next().unwrap_or(k))
                .map(|n| (n.to_string(), n.to_string()))
                .collect();
            let mut exports: Exports = (app.files.keys())
                .filter(|f| f.ends_with("sdk.gen.ts"))
                .map(|f| (f.clone(), names.clone()))
                .collect();
            if !exports.is_empty() {
                // Re-export chains of any length, up to a fixed depth.
                let chains = chains(app, &exports);
                for _ in 0..8 {
                    if !propagate(&mut exports, &chains) {
                        break;
                    }
                }
            }
            Sdk { index, exports }
        }
    }

    /// `(offset, source, call)` of each use of the client in `file`: names imported
    /// from a module that resolves to the generated file or re-exports it. A name
    /// the index lacks is unresolved, never dropped.
    pub(super) fn calls(
        app: &App,
        sdk: &Sdk,
        file: &str,
        text: &str,
    ) -> Vec<(usize, String, Result<Call, String>)> {
        let b = text.as_bytes();
        let mut out = Vec::new();
        if sdk.exports.is_empty() {
            return out;
        }
        for i in imports(text) {
            let Some(names) = app
                .resolve(file, &i.module)
                .and_then(|m| sdk.exports.get(m))
            else {
                continue;
            };
            let first = words(text, &i.local).first().copied().unwrap_or(0);
            let Some(orig) = i.imported.as_deref().filter(|n| *n != "*") else {
                let why = "default and namespace imports of the generated client are not read";
                out.push((first, i.local.clone(), Err(why.into())));
                continue;
            };
            let Some(orig) = names.get(orig) else {
                continue;
            };
            for pos in words(text, &i.local) {
                let after = pos + i.local.len();
                let (key, end) = if text[after..].starts_with('.') {
                    let member = ident_at(text, after + 1);
                    (format!("{orig}.{member}"), after + 1 + member.len())
                } else if text[..pos].ends_with('.') {
                    continue;
                } else {
                    (orig.clone(), after)
                };
                if b.get(skip_ws(b, end)) == Some(&b'(') {
                    let found = sdk.index.get(&key).cloned().unwrap_or_else(|| {
                        Err(format!("{key} is not in the generated client's index"))
                    });
                    out.push((pos, key, found));
                }
            }
        }
        out
    }
}

/// `fetch`, axios and axios instances.
mod http {
    use super::*;

    /// The call; an unknown one becomes `Call::Unresolved` and a diagnostic.
    fn record(
        found: Result<Call, String>,
        source: String,
        (file, line): (&str, u32),
        out: &mut Vec<UiFlowDiagnostic>,
    ) -> (u32, Call) {
        let call = found.unwrap_or_else(|why| {
            let message = format!("HTTP call {why}; the call is unknown: {source}");
            diag(out, file, line, message);
            Call::Unresolved(source)
        });
        (line, call)
    }

    pub(super) fn actions(
        app: &App,
        file: &str,
        text: &str,
        (clients, sdk): (&Clients, &sdk::Sdk),
        froms: &[(Origin, Option<String>)],
        flow: &mut UiFlow,
    ) {
        let out = &mut flow.diagnostics;
        let mut calls = fetches(file, text, out);
        for (recv, base) in clients.receivers(file, text, |spec| app.module(file, spec)) {
            for pos in words(text, &recv) {
                let Some((method, url)) = client_call(text, pos, &recv) else {
                    continue;
                };
                let found = http_call(
                    Ok(method.to_ascii_uppercase()),
                    url.value.literal(),
                    base.as_ref(),
                );
                let at = (file, line_of(text, pos));
                calls.push(record(found, url.source(text), at, out));
            }
        }
        for (pos, key, found) in sdk::calls(app, sdk, file, text) {
            let at = (file, line_of(text, pos));
            calls.push(record(found, key, at, out));
        }
        for i in app.missing_imports(file, text) {
            if let Some(pos) = call_of(text, &i.local) {
                let message = format!(
                    "Import {} resolves to no file; calls through {} are unknown.",
                    i.module, i.local
                );
                diag(out, file, line_of(text, pos), message);
            }
        }
        for (from, _) in froms {
            for (line, call) in &calls {
                flow.actions.push(Action {
                    screen: from.clone(),
                    trigger: None,
                    call: call.clone(),
                    file: file.into(),
                    line: *line,
                    condition: None,
                    source: SOURCE.into(),
                });
            }
        }
    }

    /// The first call through `local`: `local(…)`, `new local(…)` or `local.member(…)`.
    fn call_of(text: &str, local: &str) -> Option<usize> {
        let b = text.as_bytes();
        words(text, local).into_iter().find(|&pos| {
            let after = skip_ws(b, pos + local.len());
            let called = |at: usize| b.get(skip_ws(b, at)) == Some(&b'(');
            !text[..pos].ends_with('.')
                && (called(after)
                    || (b.get(after) == Some(&b'.')
                        && called(after + 1 + ident_at(text, after + 1).len())))
        })
    }

    /// `fetch(url, { method })`, also on `window` or `globalThis`.
    fn fetches(file: &str, text: &str, out: &mut Vec<UiFlowDiagnostic>) -> Vec<(u32, Call)> {
        let b = text.as_bytes();
        let mut calls = Vec::new();
        for pos in words(text, "fetch") {
            let open = skip_ws(b, pos + "fetch".len());
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
            calls.push(record(
                found,
                url.source(text),
                (file, line_of(text, pos)),
                out,
            ));
        }
        calls
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{AccessState, Call, Condition, Origin, Target};

    fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in files {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        dir
    }

    fn screens(flow: &UiFlow) -> Vec<(String, String, Option<Condition>)> {
        flow.screens
            .iter()
            .map(|s| (s.id.clone(), s.name.clone(), s.access.clone()))
            .collect()
    }

    fn screen(id: &str, name: &str, access: Option<&str>) -> (String, String, Option<Condition>) {
        (
            id.into(),
            name.into(),
            access.map(|a| Condition::Custom(a.into())),
        )
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

    fn unresolved(text: &str) -> Target {
        Target::Unresolved(text.into())
    }

    fn has_diag(flow: &UiFlow, file: &str, line: u32) -> bool {
        flow.diagnostics
            .iter()
            .any(|d| d.file == file && d.line == line)
    }

    const NEXT_PKG: &str = r#"{"dependencies":{"next":"15.1.0","react":"19.0.0"}}"#;

    fn pages_project() -> tempfile::TempDir {
        project(&[
            ("shop/package.json", NEXT_PKG),
            (
                "shop/pages/index.tsx",
                "import Link from 'next/link';
import { useRouter } from 'next/router';
export default function Home() {
  const router = useRouter();
  const go = () => router.push('/cart');
  return <main>
    <Link href=\"/products/42\" data-testid=\"product-42\">Product</Link>
    <Link href=\"https://example.com\">External</Link>
  </main>;
}
",
            ),
            (
                "shop/pages/products/[id].tsx",
                "import Link from 'next/link';
const Product = () => {
  const { push } = useRouter();
  return <div>
    <Link href={`/products/${p.id}/reviews`}>Reviews</Link>
    <button onClick={() => push({ pathname: '/cart' })}>Docs</button>
  </div>;
};
export default Product;
",
            ),
            ("shop/pages/cart.tsx", "export default () => <p>Cart</p>;\n"),
            (
                "shop/pages/docs/[...slug].tsx",
                "export default function Docs() { return null; }\n",
            ),
            (
                "shop/pages/_app.tsx",
                "export default function App() { return null; }\n",
            ),
            (
                "shop/pages/api/orders/[id].ts",
                "export default function handler(req, res) { res.json({}); }\n",
            ),
            (
                "shop/pages/api/admin.ts",
                "const handler = (req, res) => res.json({});\nexport default withAuth(handler);\n",
            ),
            (
                "shop/src/components/Nav.tsx",
                "import NextLink from 'next/link';
import styled from 'styled-components';
export const NavLinkStyled = styled(NextLink)`color: red;`;
export function Nav() {
  return <nav>
    <NavLinkStyled href=\"/cart\" id=\"nav-cart\">Cart</NavLinkStyled>
    <S.Link href=\"/\">Home</S.Link>
  </nav>;
}
",
            ),
            (
                "shop/src/components/Thing.stories.tsx",
                "export const A = () => <Link href=\"/cart\">x</Link>; fetch('/api/story');",
            ),
            ("shop/node_modules/lib/package.json", NEXT_PKG),
            (
                "shop/node_modules/lib/pages/hidden.tsx",
                "export default function Hidden() { return null; }",
            ),
            (
                "shop/.next/server/app.js",
                "<Route path=\"/built\" element={<Built />} />",
            ),
        ])
    }

    #[test]
    fn catch_all_pages_take_deep_links_after_exact_and_parameter_pages() {
        let dir = project(&[
            ("shop/package.json", NEXT_PKG),
            (
                "shop/pages/index.tsx",
                "import Link from 'next/link';
export default function Home() {
  return <nav>
    <Link href=\"/docs/intro\">Intro</Link>
    <Link href=\"/docs/guide/setup\">Setup</Link>
    <Link href=\"/docs/faq\">FAQ</Link>
  </nav>;
}
",
            ),
            (
                "shop/pages/docs/[...slug].tsx",
                "export default function Docs() { return null; }\n",
            ),
            (
                "shop/pages/docs/[page].tsx",
                "export default function DocPage() { return null; }\n",
            ),
            (
                "shop/pages/docs/intro.tsx",
                "export default function Intro() { return null; }\n",
            ),
        ]);
        let flow = extract(dir.path());
        let found: Vec<(Target, Option<String>)> = (transitions(&flow).into_iter())
            .map(|(_, target, trigger)| (target, trigger))
            .collect();
        assert_eq!(
            found,
            [
                (to("shop:/docs/*"), Some("Setup".into())),
                (to("shop:/docs/:page"), Some("FAQ".into())),
                (to("shop:/docs/intro"), Some("Intro".into())),
            ]
        );
    }

    #[test]
    fn next_pages_router_screens_and_api_endpoints() {
        let dir = pages_project();
        let flow = extract(dir.path());
        assert_eq!(
            screens(&flow),
            vec![
                screen("shop:/", "Home", None),
                screen("shop:/cart", "shop/pages/cart.tsx", None),
                screen("shop:/docs/*", "Docs", None),
                screen("shop:/products/:id", "Product", None),
            ]
        );
        let home = &flow.screens[0];
        assert_eq!(home.file, "shop/pages/index.tsx");
        assert_eq!(home.route.as_deref(), Some("/"));
        assert_eq!(home.application.as_deref(), Some("shop"));
        assert!(flow.screens.iter().all(|s| s.source == "react"));
        let endpoints: Vec<(String, String, AccessState)> = flow
            .endpoints
            .iter()
            .map(|e| (e.method.clone(), e.path.clone(), e.access.clone()))
            .collect();
        assert_eq!(
            endpoints,
            vec![
                (
                    "ANY".into(),
                    "/api/admin".into(),
                    AccessState::Protected(Condition::Custom("withAuth".into()))
                ),
                (
                    "ANY".into(),
                    "/api/orders/:id".into(),
                    AccessState::Undeclared
                ),
            ]
        );
        assert!(
            flow.endpoints
                .iter()
                .all(|e| e.application.as_deref() == Some("shop") && e.source == "react")
        );
    }

    #[test]
    fn next_links_router_push_and_nav_components() {
        let dir = pages_project();
        let flow = extract(dir.path());
        let mut expected = vec![
            (page("shop:/"), to("shop:/cart"), None),
            (
                page("shop:/"),
                to("shop:/products/:id"),
                Some("product-42".into()),
            ),
            (
                page("shop:/products/:id"),
                unresolved("`/products/${p.id}/reviews`"),
                Some("Reviews".into()),
            ),
            (page("shop:/products/:id"), to("shop:/cart"), None),
            (any("shop"), to("shop:/cart"), Some("nav-cart".into())),
            (any("shop"), to("shop:/"), Some("Home".into())),
        ];
        expected.sort();
        assert_eq!(transitions(&flow), expected);
        assert!(has_diag(&flow, "shop/pages/products/[id].tsx", 5));
        assert!(!flow.complete);
    }

    #[test]
    fn next_app_router_groups_slots_route_handlers_and_middleware() {
        let dir = project(&[
            ("web/package.json", NEXT_PKG),
            (
                "web/app/page.tsx",
                "import { redirect } from 'next/navigation';
export default function Home() { if (x) redirect('/orders/7'); return null; }
",
            ),
            (
                "web/app/(shop)/orders/[id]/page.tsx",
                "export default async function OrderPage() { return null; }\n",
            ),
            (
                "web/app/@modal/login/page.tsx",
                "export default function Login() { return null; }\n",
            ),
            (
                "web/app/_lib/page.tsx",
                "export default function Private() { return null; }\n",
            ),
            (
                "web/app/api/items/route.ts",
                "export async function GET(req: Request) { return Response.json([]); }
export const POST = async (req: Request) => Response.json({});
",
            ),
            (
                "web/middleware.ts",
                "export const config = { matcher: ['/orders/:path*', '/api/items'] };\n",
            ),
        ]);
        let flow = extract(dir.path());
        assert_eq!(
            screens(&flow),
            vec![
                screen("web:/", "Home", None),
                screen("web:/orders/:id", "OrderPage", Some("middleware")),
            ]
        );
        assert_eq!(
            transitions(&flow),
            vec![(page("web:/"), to("web:/orders/:id"), None)]
        );
        let endpoints: Vec<(String, String, AccessState)> = flow
            .endpoints
            .iter()
            .map(|e| (e.method.clone(), e.path.clone(), e.access.clone()))
            .collect();
        let mw = AccessState::Protected(Condition::Custom("middleware".into()));
        assert_eq!(
            endpoints,
            vec![
                ("GET".into(), "/api/items".into(), mw.clone()),
                ("POST".into(), "/api/items".into(), mw),
            ]
        );
        assert_eq!(flow.endpoints[1].line, 2);
        assert!(flow.complete, "{:?}", flow.diagnostics);
    }

    const SPA_APP: &str = "import { Routes, Route, Navigate } from 'react-router-dom';
export default function App() {
  return (
    <Routes>
      <Route path=\"/\" element={<Layout />}>
        <Route index element={<Home />} />
        <Route path=\"orders\" element={<RequireAuth><Orders /></RequireAuth>}>
          <Route path=\":id\" element={<OrderDetail />} />
        </Route>
        <Route element={<ProtectedRoute />}>
          <Route path=\"admin\" element={<Admin />} />
        </Route>
        <Route path=\"*\" element={<Navigate to=\"/\" />} />
      </Route>
    </Routes>
  );
}
";

    fn spa_project() -> tempfile::TempDir {
        project(&[
            (
                "spa/package.json",
                r#"{"dependencies":{"react":"19","react-router-dom":"7"}}"#,
            ),
            ("spa/src/App.tsx", SPA_APP),
            (
                "spa/src/Layout.tsx",
                "export function Layout() {
  return <nav>
    <NavLink to=\"/orders\" data-testid=\"nav-orders\">Orders</NavLink>
    <Link to={someVar}>Somewhere</Link>
    <Outlet />
  </nav>;
}
",
            ),
            (
                "spa/src/api.ts",
                "import axios from 'axios';
export const api = axios.create({ baseURL: '/api/v1' });
export const ext = axios.create({ baseURL: process.env.API_URL });
",
            ),
            (
                "spa/src/Orders.tsx",
                "import { useNavigate } from 'react-router-dom';
import client from 'axios';
import { api, ext } from './api';
export function Orders() {
  const navigate = useNavigate();
  const open = (id) => navigate(`/orders/${id}`);
  const lost = () => navigate('/nowhere');
  fetch('/api/orders', { method: 'post', body });
  fetch(`/api/orders/${id}?full=1`);
  fetch(url);
  client.delete('https://shop.example/api/orders/1');
  api.get('/items');
  ext.get('/x');
  other.get('/y');
  return null;
}
",
            ),
        ])
    }

    #[test]
    fn react_router_jsx_routes_nest_and_guards_wrap() {
        let dir = spa_project();
        let flow = extract(dir.path());
        assert_eq!(
            screens(&flow),
            vec![
                screen("spa:/", "Home", None),
                screen("spa:/admin", "Admin", Some("ProtectedRoute")),
                screen("spa:/orders", "Orders", Some("RequireAuth")),
                screen("spa:/orders/:id", "OrderDetail", Some("RequireAuth")),
            ]
        );
        let orders = &flow.screens[2];
        assert_eq!((orders.file.as_str(), orders.line), ("spa/src/App.tsx", 7));
    }

    #[test]
    fn react_router_navigation_resolves_within_the_app() {
        let dir = spa_project();
        let flow = extract(dir.path());
        let mut expected = vec![
            (any("spa"), to("spa:/orders"), Some("nav-orders".into())),
            (any("spa"), unresolved("someVar"), Some("Somewhere".into())),
            (page("spa:/orders"), to("spa:/orders/:id"), None),
            (page("spa:/orders"), unresolved("'/nowhere'"), None),
        ];
        expected.sort();
        assert_eq!(transitions(&flow), expected);
        assert!(has_diag(&flow, "spa/src/Layout.tsx", 4));
        assert!(has_diag(&flow, "spa/src/Orders.tsx", 7));
    }

    #[test]
    fn fetch_axios_and_api_instances_become_actions() {
        let dir = spa_project();
        let flow = extract(dir.path());
        let http = |method: &str, path: &str| Call::Http {
            method: method.into(),
            path: path.into(),
        };
        let got: Vec<(Origin, Call, u32)> = flow
            .actions
            .iter()
            .map(|a| (a.screen.clone(), a.call.clone(), a.line))
            .collect();
        let orders = page("spa:/orders");
        assert_eq!(
            got,
            vec![
                (orders.clone(), http("DELETE", "/api/orders/1"), 11),
                (orders.clone(), http("GET", "/api/orders/:param"), 9),
                (orders.clone(), http("GET", "/api/v1/items"), 12),
                (orders.clone(), http("POST", "/api/orders"), 8),
                (orders.clone(), Call::Unresolved("'/x'".into()), 13),
                (orders, Call::Unresolved("url".into()), 10),
            ]
        );
        assert!(flow.actions.iter().all(|a| a.file == "spa/src/Orders.tsx"));
        assert!(has_diag(&flow, "spa/src/Orders.tsx", 13));
    }

    #[test]
    fn route_objects_with_children_and_use_routes() {
        let dir = project(&[
            (
                "dash/package.json",
                r#"{"dependencies":{"react-router":"7.1.0"}}"#,
            ),
            (
                "dash/src/router.tsx",
                "export const router = createBrowserRouter([
  { path: '/', Component: Root, children: [
    { index: true, element: <Dashboard /> },
    { path: 'settings', element: <AuthGuard><Settings /></AuthGuard>, children: [
      { path: 'profile', Component: Profile },
    ] },
    { path: dynamicPath, element: <Hidden /> },
  ] },
]);
",
            ),
            (
                "dash/src/Help.tsx",
                "export function HelpRoutes() { return useRoutes([{ path: '/help', element: <Help /> }]); }\n",
            ),
        ]);
        let flow = extract(dir.path());
        assert_eq!(
            screens(&flow),
            vec![
                screen("dash:/", "Dashboard", None),
                screen("dash:/help", "Help", None),
                screen("dash:/settings", "Settings", Some("AuthGuard")),
                screen("dash:/settings/profile", "Profile", Some("AuthGuard")),
            ]
        );
        assert!(has_diag(&flow, "dash/src/router.tsx", 7));
        assert!(!flow.complete);
    }

    const TANSTACK_PKG: &str = r#"{"dependencies":{"@tanstack/react-router":"1.0.0"}}"#;

    fn tanstack_project(config: &str) -> tempfile::TempDir {
        project(&[
            ("archguard.toml", config),
            ("fe/package.json", TANSTACK_PKG),
            (
                "fe/src/routes/__root.tsx",
                "export const Route = createRootRoute({ component: () => <Outlet /> })\n",
            ),
            (
                "fe/src/routes/_layout.tsx",
                "export const Route = createFileRoute(\"/_layout\")({
  component: Layout,
  beforeLoad: async () => {
    if (!isLoggedIn()) {
      throw redirect({ to: \"/login\" })
    }
  },
})
function Layout() { return <main><Link to=\"/items/$id\" params={{ id }}>Item</Link><Outlet /></main> }
",
            ),
            (
                "fe/src/routes/_layout/items.$id.tsx",
                "import { ItemsService } from '@/client'
export const Route = createFileRoute('/_layout/items/$id')({ component: Item })
function Item() {
  const navigate = useNavigate()
  const go = () => navigate({ to: '/login' })
  const load = () => ItemsService.deleteItem({ path: { id } })
  return <Link to={dynamic}>x</Link>
}
",
            ),
            (
                "fe/src/routes/login.tsx",
                "export const Route = createFileRoute('/login')({ component: Login })
function Login() { return <RouterLink to=\"/items/$id\">Items</RouterLink> }
import { Link as RouterLink } from '@tanstack/react-router'
",
            ),
            (
                "fe/src/client/sdk.gen.ts",
                "export class ItemsService {
    public static deleteItem(options) {
        return request({ baseUrl: OpenAPI.BASE, method: Method.DELETE, url: '/api/v1/items/{id}' });
    }
    public static readItems(options) {
        return (options?.client ?? client).get<A, B>({ url: '/api/v1/items/' });
    }
    public static odd(options) {
        return request({ method: Method.GET, url: path });
    }
}
",
            ),
            ("fe/src/client/core/OpenAPI.ts", "export const OpenAPI = { BASE: '/be' };\n"),
            (
                "fe/src/client/types.gen.ts",
                "export enum Method { GET = 'GET', DELETE = 'DELETE' }\n",
            ),
            ("fe/src/client/index.ts", "export * from './sdk.gen';\n"),
            (
                "fe/tsconfig.json",
                "{ \"compilerOptions\": { /* c */ \"paths\": { \"@/*\": [\"./src/*\"] } } }\n",
            ),
        ])
    }

    #[test]
    fn tanstack_file_routes_layouts_and_before_load_guards() {
        let dir = tanstack_project("[views.access]\nisLoggedIn = \"authenticated\"\n");
        let flow = extract(dir.path());
        let got: Vec<_> = flow
            .screens
            .iter()
            .map(|s| (s.id.clone(), s.name.clone(), s.access.clone()))
            .collect();
        assert_eq!(
            got,
            vec![
                (
                    "fe:/items/:id".to_string(),
                    "Item".to_string(),
                    Some(Condition::Authenticated)
                ),
                ("fe:/login".to_string(), "Login".to_string(), None),
            ]
        );
        let custom = tanstack_project("");
        let flow = extract(custom.path());
        assert_eq!(
            flow.screens[0].access,
            Some(Condition::Custom("isLoggedIn".into()))
        );
    }

    #[test]
    fn tanstack_links_and_navigate_resolve_with_params() {
        let dir = tanstack_project("");
        let flow = extract(dir.path());
        let mut expected = vec![
            (any("fe"), to("fe:/items/:id"), Some("Item".into())),
            (page("fe:/items/:id"), to("fe:/login"), None),
            (
                page("fe:/items/:id"),
                unresolved("dynamic"),
                Some("x".into()),
            ),
            (page("fe:/login"), to("fe:/items/:id"), Some("Items".into())),
        ];
        expected.sort();
        assert_eq!(transitions(&flow), expected);
    }

    #[test]
    fn hey_api_sdk_calls_become_http_actions() {
        let dir = tanstack_project("");
        let flow = extract(dir.path());
        let got: Vec<_> = flow
            .actions
            .iter()
            .map(|a| (a.screen.clone(), a.call.clone(), a.line))
            .collect();
        assert_eq!(
            got,
            vec![(
                page("fe:/items/:id"),
                Call::Http {
                    method: "DELETE".into(),
                    path: "/be/api/v1/items/:param".into()
                },
                6
            )]
        );
    }

    #[test]
    fn hey_api_index_reads_verb_calls_and_flags_unreadable_urls() {
        let sdk = sdk::index(&tanstack_app());
        let call = |k: &str| sdk.get(k).cloned();
        assert_eq!(
            call("ItemsService.readItems"),
            Some(Ok(Call::Http {
                method: "GET".into(),
                path: "/be/api/v1/items/".into()
            }))
        );
        assert!(matches!(call("ItemsService.odd"), Some(Err(_))));
    }

    fn tanstack_app() -> App {
        let dir = tanstack_project("");
        let mut files = BTreeMap::new();
        for p in [
            "fe/src/client/sdk.gen.ts",
            "fe/src/client/core/OpenAPI.ts",
            "fe/src/client/types.gen.ts",
        ] {
            files.insert(
                p.to_string(),
                std::fs::read_to_string(dir.path().join(p)).unwrap(),
            );
        }
        App {
            name: "fe".into(),
            prefix: "fe/".into(),
            next: false,
            tanstack: true,
            aliases: Vec::new(),
            files,
        }
    }

    /// An app with a hey-api client, `@/` alias, an `@/api` re-export and one
    /// route module `src/routes/x.tsx` holding `route`.
    fn sdk_app(sdk: &str, route: &str) -> UiFlow {
        let dir = project(&[
            ("fe/package.json", TANSTACK_PKG),
            (
                "fe/tsconfig.json",
                "{ \"compilerOptions\": { \"paths\": { \"@/*\": [\"./src/*\"] } } }",
            ),
            ("fe/src/client/sdk.gen.ts", sdk),
            ("fe/src/client/index.ts", "export * from './sdk.gen';\n"),
            (
                "fe/src/api.ts",
                "export { ItemsService as Items } from './client';\n",
            ),
            (
                "fe/src/lib/client/utils.ts",
                "export function listUsers() { return []; }\n",
            ),
            (
                "fe/src/client/types.gen.ts",
                "export enum Method { Del = 'DEL', Remove = 'DELETE', GET = 'GET' }\n",
            ),
            ("fe/src/routes/x.tsx", route),
        ]);
        extract(dir.path())
    }

    const SDK: &str = "export class ItemsService {
    public static getItems() {
        return (options?.client ?? client).get<A, B>({ url: '/api/items' });
    }
}
export const listUsers = () => client.get<A, B>({ url: '/api/users' });
";

    fn calls(flow: &UiFlow) -> Vec<Call> {
        flow.actions.iter().map(|a| a.call.clone()).collect()
    }

    fn http(method: &str, path: &str) -> Call {
        Call::Http {
            method: method.into(),
            path: path.into(),
        }
    }

    #[test]
    fn sdk_calls_need_an_import_that_resolves_to_the_sdk_module() {
        let route = "import { ItemsService } from '@apollo/client';
import { listUsers } from '../lib/client/utils';
import { Items } from '@/api';
export const Route = createFileRoute('/x')({ component: X })
function X() { ItemsService.getItems(); listUsers(); Items.getItems(); }
";
        let flow = sdk_app(SDK, route);
        assert_eq!(calls(&flow), vec![http("GET", "/api/items")]);
        assert_eq!(flow.actions[0].line, 5);
        assert!(flow.complete, "{:?}", flow.diagnostics);
    }

    #[test]
    fn sdk_members_missing_from_the_index_are_unresolved_gaps() {
        let route = "import { ItemsService } from '@/client';
import { Items } from '@/api';
export const Route = createFileRoute('/x')({ component: X })
function X() { ItemsService.notInIndex(); Items.alsoMissing(); }
";
        let flow = sdk_app(SDK, route);
        assert_eq!(
            calls(&flow),
            vec![
                Call::Unresolved("ItemsService.alsoMissing".into()),
                Call::Unresolved("ItemsService.notInIndex".into()),
            ]
        );
        assert!(has_diag(&flow, "fe/src/routes/x.tsx", 4));
        assert!(!flow.complete);
    }

    #[test]
    fn sdk_methods_resolve_through_the_enum_to_a_verb() {
        let sdk = "export class S {
    public static a() { return request({ method: Method.Remove, url: '/a' }); }
    public static b() { return request({ method: Method.Del, url: '/b' }); }
    public static c() { return request({ method: verb, url: '/c' }); }
    public static d() { return request({ method: Method.Nope, url: '/d' }); }
}
";
        let route = "import { S } from '@/client';
export const Route = createFileRoute('/x')({ component: X })
function X() { S.a(); S.b(); S.c(); S.d(); }
";
        let flow = sdk_app(sdk, route);
        let unresolved = |m: &str| Call::Unresolved(format!("S.{m}"));
        assert_eq!(
            calls(&flow),
            vec![
                http("DELETE", "/a"),
                unresolved("b"),
                unresolved("c"),
                unresolved("d"),
            ]
        );
        assert!(!flow.complete);
    }

    /// Access of the only screen, `/x`, of a TanStack app, with the route module.
    fn route_access(route: &str) -> (Option<Condition>, UiFlow) {
        let dir = project(&[
            ("fe/package.json", TANSTACK_PKG),
            ("fe/src/routes/x.tsx", route),
            (
                "fe/src/lib/guards.ts",
                "export function viaImport() { if (!ready()) throw redirect({ to: '/login' }) }\n",
            ),
        ]);
        let flow = extract(dir.path());
        let access = flow
            .screens
            .iter()
            .find(|s| s.id == "fe:/x")
            .unwrap()
            .access
            .clone();
        (access, flow)
    }

    fn custom(name: &str) -> Condition {
        Condition::Custom(name.into())
    }

    fn unknown(c: &Option<Condition>) -> bool {
        matches!(c, Some(Condition::Unknown(_)))
    }

    #[test]
    fn before_load_forms_are_read_or_unknown() {
        let shorthand = "export const Route = createFileRoute('/x')({
  component: X,
  async beforeLoad({ context }) {
    if (!context.auth.ok) throw redirect({ to: '/login' })
  },
})
";
        assert_eq!(route_access(shorthand).0, Some(custom("context.auth.ok")));
        let local =
            "const guard = async () => { if (!isLoggedIn()) { throw redirect({ to: '/' }) } }
export const Route = createFileRoute('/x')({ component: X, beforeLoad: guard })
";
        assert_eq!(route_access(local).0, Some(custom("isLoggedIn")));
        let imported = "import { viaImport } from '../lib/guards';
export const Route = createFileRoute('/x')({ component: X, beforeLoad: viaImport })
";
        assert_eq!(route_access(imported).0, Some(custom("ready")));
        let opaque = "import { requireAuth } from 'some-lib';
export const Route = createFileRoute('/x')({ component: X, beforeLoad: requireAuth })
";
        let (access, flow) = route_access(opaque);
        assert!(unknown(&access), "{access:?}");
        assert!(!flow.complete);
        let expression = "export const Route = createFileRoute('/x')({ component: X, beforeLoad: () => check() })
";
        assert!(unknown(&route_access(expression).0));
    }

    #[test]
    fn before_load_early_return_and_else_redirects_are_read() {
        let early = "export const Route = createFileRoute('/x')({
  component: X,
  beforeLoad: ({ context }) => {
    if (context.auth.ok) return
    throw redirect({ to: '/login' })
  },
})
";
        assert_eq!(route_access(early).0, Some(custom("context.auth.ok")));
        let other = "export const Route = createFileRoute('/x')({
  component: X,
  beforeLoad: ({ context }) => {
    if (context.auth.ok) { log() } else { throw redirect({ to: '/login' }) }
  },
})
";
        assert_eq!(route_access(other).0, Some(custom("context.auth.ok")));
        let odd = "export const Route = createFileRoute('/x')({
  component: X,
  beforeLoad: () => {
    try { go() } catch { throw redirect({ to: '/login' }) }
  },
})
";
        let (access, flow) = route_access(odd);
        assert!(unknown(&access), "{access:?}");
        assert!(!flow.complete);
    }

    #[test]
    fn before_load_reading_stops_at_its_own_value() {
        let route = "export const Route = createFileRoute('/x')({
  component: X,
  beforeLoad() { return },
  loader: () => { if (!hasData()) throw redirect({ to: '/' }) },
})
";
        let (access, flow) = route_access(route);
        assert_eq!(access, None);
        assert!(flow.complete, "{:?}", flow.diagnostics);
    }

    #[test]
    fn consecutive_before_load_redirects_each_add_their_own_condition() {
        let route = "export const Route = createFileRoute('/x')({
  component: X,
  beforeLoad: async ({ search }) => {
    if (isLoggedIn()) {
      throw redirect({ to: '/' })
    }
    if (!search.token) {
      throw redirect({ to: '/login' })
    }
  },
})
";
        let want = Condition::All(vec![
            Condition::Not(Box::new(custom("isLoggedIn"))),
            custom("search.token"),
        ]);
        assert_eq!(route_access(route).0, Some(want));
    }

    #[test]
    fn nested_before_load_ifs_negate_the_whole_path() {
        let route = "export const Route = createFileRoute('/x')({
  component: X,
  beforeLoad: () => {
    if (isTrial()) {
      if (isExpired()) throw redirect({ to: '/pay' })
    }
  },
})
";
        let both = Condition::All(vec![custom("isTrial"), custom("isExpired")]);
        assert_eq!(route_access(route).0, Some(Condition::Not(Box::new(both))));
    }

    #[test]
    fn no_application_without_a_router_dependency() {
        let dir = project(&[
            ("lib/package.json", r#"{"dependencies":{"react":"19"}}"#),
            ("lib/pages/index.tsx", "export default function A() {}"),
            ("lib/src/A.tsx", "fetch('/api/a');"),
        ]);
        let flow = extract(dir.path());
        assert_eq!(
            flow,
            UiFlow {
                complete: true,
                ..UiFlow::default()
            }
        );
    }

    const ROUTER_PKG: &str = r#"{"dependencies":{"react":"19","react-router-dom":"7"}}"#;
    const APP_ROUTES: &str = "import { Home } from './Home';
const AppRoutes = [
  { index: true, element: <Home /> },
  { path: '/counter', element: <Counter /> },
  { path: '/weather', element: <ProtectedRoute><Weather /></ProtectedRoute> },
];
export default AppRoutes;
";
    const MAPPED: &str = "<Routes>
  {AppRoutes.map((route, index) => {
    const { element, ...rest } = route;
    return <Route key={index} {...rest} element={element} />;
  })}
</Routes>";

    #[test]
    fn route_arrays_mapped_into_routes_are_read_when_imported() {
        let app = format!(
            "import AppRoutes from './AppRoutes';\nexport const App = () => (\n{MAPPED});\n"
        );
        let dir = project(&[
            ("c/package.json", ROUTER_PKG),
            ("c/src/App.jsx", &app),
            ("c/src/AppRoutes.jsx", APP_ROUTES),
        ]);
        let flow = extract(dir.path());
        assert_eq!(
            screens(&flow),
            vec![
                screen("c:/", "Home", None),
                screen("c:/counter", "Counter", None),
                screen("c:/weather", "Weather", Some("ProtectedRoute")),
            ]
        );
        assert_eq!(flow.screens[0].file, "c/src/AppRoutes.jsx");
    }

    #[test]
    fn route_arrays_mapped_into_routes_are_read_when_local() {
        let local = APP_ROUTES
            .replace("import { Home } from './Home';\n", "")
            .replace("export default AppRoutes;\n", "");
        let app = format!("{local}export const App = () => (\n{MAPPED});\n");
        let dir = project(&[("c/package.json", ROUTER_PKG), ("c/src/App.jsx", &app)]);
        let flow = extract(dir.path());
        assert_eq!(flow.screens.len(), 3, "{:?}", flow.diagnostics);
        assert!(flow.diagnostics.is_empty(), "{:?}", flow.diagnostics);
    }

    #[test]
    fn mapped_route_shapes_other_than_the_array_literal_are_gaps() {
        let app = "import { routes } from './elsewhere';
export const App = () => (
<Routes>
  {routes.map((r) => <Route {...r} />)}
  {more.map((r) => <Route path={r.path} {...r} element={<X />} />)}
</Routes>);
";
        let dir = project(&[
            ("c/package.json", ROUTER_PKG),
            ("c/src/App.jsx", app),
            ("c/src/elsewhere.js", "export const routes = load();\n"),
        ]);
        let flow = extract(dir.path());
        assert!(flow.screens.is_empty());
        assert!(has_diag(&flow, "c/src/App.jsx", 4));
        assert!(has_diag(&flow, "c/src/App.jsx", 5));
        assert!(!flow.complete);
    }

    #[test]
    fn calls_through_an_import_that_resolves_to_no_file_are_gaps() {
        let page = "import { Gen } from '../web-api-client.ts';
import { Aliased } from '@/gen/client';
import axios from 'axios';
import { useState } from 'react';
export function Weather() {
  const [s] = useState(0);
  axios;
  return new Gen().load() && Aliased.run();
}
";
        let app = "export const App = () => <Routes><Route path=\"/w\" element={<Weather />} /></Routes>;\n";
        let dir = project(&[
            (
                "c/package.json",
                r#"{"dependencies":{"react-router-dom":"7","axios":"1","react":"19"}}"#,
            ),
            (
                "c/tsconfig.json",
                r#"{"compilerOptions":{"paths":{"@/*":["./src/*"]}}}"#,
            ),
            ("c/src/App.jsx", app),
            ("c/src/components/Weather.jsx", page),
        ]);
        let flow = extract(dir.path());
        let messages: Vec<&str> = flow
            .diagnostics
            .iter()
            .map(|d| d.message.as_str())
            .collect();
        assert_eq!(messages.len(), 2, "{messages:?}");
        assert!(messages[0].contains("../web-api-client.ts"), "{messages:?}");
        assert!(messages[1].contains("@/gen/client"), "{messages:?}");
        assert!(has_diag(&flow, "c/src/components/Weather.jsx", 8));
    }
}
