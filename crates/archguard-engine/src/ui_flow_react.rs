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
    /// Sources by path relative to the analysis root.
    files: BTreeMap<String, String>,
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
        let sdk = sdk::index(self);
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

    /// A relative import resolved to a scanned file.
    fn module(&self, from: &str, spec: &str) -> Option<&str> {
        if !spec.starts_with('.') {
            return None;
        }
        let base = join(dir(from), spec);
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
        let mut roots = jsx(text);
        roots.extend(objects(&mut ctx));
        for root in &roots {
            walk(&mut ctx, root, "", &[], 0);
        }
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

    fn jsx(text: &str) -> Vec<Node> {
        let mut events: Vec<(usize, bool)> = text
            .match_indices("<Route")
            .filter(|(i, _)| {
                !text[i + "<Route".len()..].starts_with(|c: char| is_ident_char(c) || c == '.')
            })
            .map(|(i, _)| (i, true))
            .chain(text.match_indices("</Route>").map(|(i, _)| (i, false)))
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

    /// What the module's `beforeLoad` requires: each `if (c)` whose branch calls
    /// `redirect(` needs `not c`. A redirect without `if` is a forwarding page.
    fn guards(m: &Module, access: &Access, out: &mut Vec<UiFlowDiagnostic>) -> Vec<Condition> {
        let (text, b) = (m.text, m.text.as_bytes());
        let Some(start) = words(text, "beforeLoad").first().copied() else {
            return Vec::new();
        };
        let Some(arrow) = text[start..].find("=>").map(|i| start + i + 2) else {
            return Vec::new();
        };
        let open = skip_ws(b, arrow);
        if b.get(open) != Some(&b'{') {
            return Vec::new();
        }
        let end = block_end(text, open, b'{', b'}');
        let mut found = Vec::new();
        for pos in words(&text[..end], "if").into_iter().filter(|&p| p > open) {
            let paren = skip_ws(b, pos + 2);
            if b.get(paren) != Some(&b'(') {
                continue;
            }
            let cond_end = block_end(text, paren, b'(', b')');
            let cond = &text[paren + 1..cond_end.saturating_sub(1).max(paren + 1)];
            let branch = skip_ws(b, cond_end);
            let branch_end = if b.get(branch) == Some(&b'{') {
                block_end(text, branch, b'{', b'}')
            } else {
                text[branch..].find([';', '\n']).map_or(end, |i| branch + i)
            };
            if !text[branch..branch_end.min(text.len())].contains("redirect(") {
                continue;
            }
            match check(cond, access) {
                Ok(c) => found.extend(negate(c)),
                Err(t) => {
                    let message = format!(
                        "Check {t} is not a name or listed in [views.access]; the condition is unknown."
                    );
                    diag(out, m.file, line_of(text, pos), message);
                    found.push(Condition::Unknown(t));
                }
            }
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
            m.guards = guards(m, access, out);
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

    /// `method: Method.DELETE`, `method: 'delete'` or a `.delete<…>(` call.
    fn method(body: &str) -> Option<String> {
        if let Some(item) = field(body, "method") {
            return match item.value {
                Value::Str(m) => Some(m.to_ascii_uppercase()),
                Value::Raw(r) => r.rsplit('.').next().map(str::to_ascii_uppercase),
                _ => None,
            };
        }
        HTTP_METHODS
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
                let call = match (url.value.literal(), method(body)) {
                    (Some(url), Some(method)) => Ok(Call::Http {
                        method,
                        path: format!("{base}{}", url_path(&holes(&url))),
                    }),
                    (None, _) => Err(format!("{key} has a url that is not a literal")),
                    (_, None) => Err(format!("{key} has a method that is not a literal")),
                };
                out.insert(key.clone(), call);
            }
        }
        out
    }

    /// `(offset, source, call)` of each call of an indexed function in `text`: a
    /// named import from a `client`/`sdk` module, or `Class.method` of an imported class.
    pub(super) fn calls(index: &Index, text: &str) -> Vec<(usize, String, Result<Call, String>)> {
        let b = text.as_bytes();
        let mut out = Vec::new();
        if index.is_empty() {
            return out;
        }
        for i in imports(text) {
            let Some(name) = i.imported.as_deref().filter(|n| *n != "*") else {
                continue;
            };
            if !(i.module.contains("client") || i.module.contains("sdk")) {
                continue;
            }
            for pos in words(text, &i.local) {
                let after = pos + i.local.len();
                let (key, end) = if text[after..].starts_with('.') {
                    let member = ident_at(text, after + 1);
                    (format!("{name}.{member}"), after + 1 + member.len())
                } else if text[..pos].ends_with('.') {
                    continue;
                } else {
                    (name.to_string(), after)
                };
                if b.get(skip_ws(b, end)) == Some(&b'(')
                    && let Some(call) = index.get(&key)
                {
                    out.push((pos, key, call.clone()));
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
        (clients, sdk): (&Clients, &sdk::Index),
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
        for (pos, key, found) in sdk::calls(sdk, text) {
            let at = (file, line_of(text, pos));
            calls.push(record(found, key, at, out));
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
        for p in ["fe/src/client/sdk.gen.ts", "fe/src/client/core/OpenAPI.ts"] {
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
            files,
        }
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
}
