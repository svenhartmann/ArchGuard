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
//!   parent's; the screen name is the first self-closing component tag of the element. A route
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
//! - Wrapper components (`<W><Page/></W>`, also a pathless parent route's element):
//!   every component element around the screen is read by import, never by name.
//!   A project component's body is walked: an early `return` that does not render
//!   `children` / `<Outlet/>` is a deny exit (`<Navigate/>`, `null`), its path
//!   conditions are the requirement (`custom(name)` unless `[views.access]` lists
//!   it); a body without one is transparent. React's `Fragment`, `Suspense`,
//!   `StrictMode`, `Profiler` are transparent; a component from a package, one
//!   declared nowhere, one that never renders its children or renders them under a
//!   condition is `unknown(W)` and a gap. Children inherit, several are `all`.
//!   Fragments around a guard are transparent. A route behind `{c && <Route/>}` or
//!   `{c ? <Route/> : …}` exists under `unknown(c)` and is a gap.
//! - Next.js `proxy.{ts,js}` / `middleware.{ts,js}` (root or `src/`): the matcher
//!   only scopes what the body is read to require. Matchers: literal paths with
//!   `:p*`, `:p+`, `:p?`, the `/((?!a|b).*)` form; `has`/`missing`, other regular
//!   expressions and a bare literal's descendants are `unknown` and a gap; a
//!   non-literal matcher covers all with a gap. The body is walked per route:
//!   `if (…) return NextResponse.redirect(…)` / `Response` with 401|403 /
//!   `redirect()` / `notFound()` / `unauthorized()` / `forbidden()` are deny exits,
//!   `pathname.startsWith('/x')` and `pathname === '/x'` are evaluated on the route;
//!   other checks are `custom(name)`. A body that is not readable is `unknown` on
//!   everything the matcher covers. `clerkMiddleware()` without callback protects
//!   nothing.
//!
//! Backend endpoints (the Next.js server side): `pages/api/**` is one endpoint per
//! file with method `ANY`; `app/**/route.*` one per exported `GET`, `POST`, …
//! (`export function M(`, `export async function M(`, `export const M =`). Access
//! is `undeclared` unless the handler is wrapped as `export default W(…)` /
//! `export const M = W(…)`: `W` is read by import (a project function by the deny
//! exits of the function it returns, `custom(W)`; `withApiAuthRequired` from
//! `@auth0/nextjs-auth0` is `authenticated`), any other `W` is `unknown(W)` and a
//! gap. Checks written inside a handler stay `undeclared` (ADR 0021, consequences).
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

/// A test on a route that may be undecidable.
#[derive(Clone, Copy, PartialEq)]
enum Tri {
    Yes,
    No,
    Maybe,
}
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
// `.mjs`, `.cjs`, `.mts` and `.cts` are not read (node scripts such as epic-stack's
// remix.init would add calls to the UI); for imports they count as existing files.
const EXTENSIONS: [&str; 4] = [".tsx", ".ts", ".jsx", ".js"];
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
    /// Other files below the application (`items.json`), by path; only their
    /// existence is known.
    others: BTreeSet<String>,
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
    let mut others = Vec::new();
    for rel in files {
        if rel.split('/').any(|p| SKIPPED_DIRS.contains(&p)) {
            continue;
        }
        let (dir, name) = rel.rsplit_once('/').unwrap_or(("", rel.as_str()));
        if name == "package.json" {
            if let Some(app) = manifest_app(root, dir, &rel) {
                apps.insert(dir.to_string(), app);
            }
        } else if matches!(
            name,
            "tsconfig.json" | "tsconfig.app.json" | "jsconfig.json"
        ) {
            tsconfigs.push(rel);
        } else if is_source(&rel, name) {
            sources.push(rel);
        } else {
            others.push(rel);
        }
    }
    for app in apps.values_mut() {
        let own = others.iter().filter(|o| o.starts_with(&app.prefix));
        app.others = own.cloned().collect();
    }
    for rel in tsconfigs {
        let dir = rel.rsplit_once('/').map_or("", |(d, _)| d);
        if let Some(app) = apps.get_mut(dir) {
            app.aliases
                .extend(tsconfig_aliases(&rel, &|p| read(root, p).ok()));
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
        others: BTreeSet::new(),
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
            next::file_routes(self, access, &mut screens, &mut endpoints, out);
        }
        for (file, text) in &self.files {
            router::routes(self, access, file, text, &mut screens, out);
        }
        if self.next {
            proxy::middleware(self, access, &mut screens, &mut endpoints, out);
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
    /// pattern, `@/` or `~/`) which exists nowhere in the application. A bare
    /// package name is not one, and neither is what a catch-all `"*"` pattern
    /// matches without a target file: that pattern fits every package.
    fn missing_imports(&self, file: &str, text: &str) -> Vec<Import> {
        imports(text)
            .into_iter()
            .filter(|i| {
                let internal = i.module.starts_with('.')
                    || i.module.starts_with("@/")
                    || i.module.starts_with("~/")
                    || (self.aliases.iter())
                        .any(|a| a.pattern != "*" && alias_rest(a, &i.module).is_some());
                internal
                    && self.resolve(file, &i.module).is_none()
                    && !self.other_file(file, &i.module)
            })
            .collect()
    }

    /// Whether the import names an existing file that is no source (`.json`).
    fn other_file(&self, from: &str, spec: &str) -> bool {
        let paths: Vec<String> = if spec.starts_with('.') {
            vec![join(dir(from), spec)]
        } else {
            (self.aliases.iter())
                .filter_map(|a| Some((a, alias_rest(a, spec)?)))
                .flat_map(|(a, rest)| a.targets.iter().map(move |t| t.replace('*', rest)))
                .collect()
        };
        paths.iter().any(|p| self.others.contains(p))
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

    pub(super) fn stem(path: &str) -> &str {
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
            || inner == "proxy"
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
    pub(super) fn default_export(text: &str) -> Option<(usize, &str)> {
        let pos = text.find("export default")?;
        let j = skip_ws(text.as_bytes(), pos + "export default".len());
        Some((pos, ident_at(text, j)))
    }

    pub(super) fn default_name(text: &str) -> Option<String> {
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

    /// A handler wrapped as `W(…)` right after `at` (`export default W(handler)`,
    /// `export const GET = W(handler)`): `W` is read by import (S1), a project
    /// function by the deny exits of the function it returns; anything else is
    /// unknown. A handler that is no call stays undeclared (checks inside it are
    /// not read).
    fn wrapped(
        ctx: (&App, &Access, &mut Vec<UiFlowDiagnostic>),
        (file, text): (&str, &str),
        at: usize,
    ) -> AccessState {
        let (app, access, out) = ctx;
        let j = skip_ws(text.as_bytes(), at);
        let name = ident_at(text, j);
        let keyword = matches!(name, "async" | "function" | "await" | "new" | "class");
        if name.is_empty() || keyword || !text[j + name.len()..].starts_with('(') {
            return AccessState::Undeclared;
        }
        let own = match read_wrapped(app, access, out, (file, text), name) {
            Ok(state) => state,
            Err(why) => {
                let message = format!(
                    "Handler wrapper {name} is not read ({why}); the handler's access is unknown."
                );
                diag(out, file, line_of(text, j), message);
                AccessState::Protected(Condition::Unknown(name.into()))
            }
        };
        // `W(V(h))`: the wrapped call is read the same way, both must hold.
        let inner = j + name.len() + 1;
        own.and(wrapped((app, access, out), (file, text), inner))
    }

    /// The state the wrapper function `name` gives its handler.
    fn read_wrapped(
        app: &App,
        access: &Access,
        out: &mut Vec<UiFlowDiagnostic>,
        (file, text): (&str, &str),
        name: &str,
    ) -> Result<AccessState, String> {
        let import = imports(text).into_iter().find(|i| i.local == name);
        let found = match &import {
            Some(i)
                if i.module.starts_with("@auth0/nextjs-auth0")
                    && i.imported.as_deref() == Some("withApiAuthRequired") =>
            {
                return Ok(AccessState::Protected(Condition::Authenticated));
            }
            Some(i) => {
                let from = app.resolve(file, &i.module);
                let from = from.ok_or_else(|| {
                    format!("imported from {}, which is no scanned file", i.module)
                })?;
                guard::higher_order(access, out, (from, &app.files[from]), i.imported.as_deref())?
            }
            None if declares(text, name) => {
                guard::higher_order(access, out, (file, text), Some(name))?
            }
            None => return Err("neither imported nor declared in this file".into()),
        };
        if found.is_empty() {
            return Ok(AccessState::Undeclared);
        }
        if found.iter().any(|c| matches!(c, Condition::Unknown(_))) {
            return Err("it holds a check that is not read".into());
        }
        Ok(AccessState::Protected(
            match tanstack::check(name, access) {
                Ok(Some(c)) => c,
                _ => Condition::Custom(name.into()),
            },
        ))
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
    fn handlers(
        ctx: (&App, &Access, &mut Vec<UiFlowDiagnostic>),
        kind: &Routed,
        (file, text): (&str, &str),
    ) -> Vec<(String, u32, AccessState)> {
        let (app, access, diags) = ctx;
        if let Routed::Api = kind {
            return vec![match default_export(text) {
                Some((pos, _)) => (
                    ANY_METHOD.into(),
                    line_of(text, pos),
                    wrapped(
                        (app, access, &mut *diags),
                        (file, text),
                        pos + "export default".len(),
                    ),
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
                    wrapped((app, access, &mut *diags), (file, text), pos + pat.len())
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
        access: &Access,
        screens: &mut Vec<(Screen, Owner)>,
        endpoints: &mut Vec<EndpointAccess>,
        out: &mut Vec<UiFlowDiagnostic>,
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
            for (method, line, state) in handlers((app, access, out), &kind, (file, text)) {
                endpoints.push(EndpointAccess {
                    method,
                    path: route.clone(),
                    access: state,
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
}

/// Next.js `proxy` / `middleware`: the matcher scopes, the body decides.
mod proxy {
    use super::next::{default_export, stem};
    use super::tanstack::{Callable, callable_at, declared_body};
    use super::*;

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

    /// One `matcher` pattern.
    enum Pattern {
        /// A literal path, the last segment optionally `:p*`, `:p+` or `:p?`.
        Path(String),
        /// `/((?!a|b).*)`: every path that starts with none of the literals.
        Lookahead(Vec<String>),
        /// Any other regular expression.
        Opaque,
    }

    struct Entry {
        pattern: Pattern,
        /// `has`, `missing` or `locale`: the match depends on the request.
        conditional: bool,
    }

    fn pattern(s: &str) -> Pattern {
        if literal_matcher(s) {
            return Pattern::Path(s.to_string());
        }
        let alts = s
            .strip_prefix("/((?!")
            .and_then(|r| r.strip_suffix(").*)"))
            .map(|inner| inner.split('|').collect::<Vec<_>>())
            .filter(|alts| {
                alts.iter().all(|a| {
                    !a.is_empty()
                        && a.chars()
                            .all(|c| is_ident_char(c) || matches!(c, '-' | '.' | '/'))
                })
            });
        match alts {
            Some(a) => Pattern::Lookahead(a.into_iter().map(String::from).collect()),
            None => Pattern::Opaque,
        }
    }

    /// Whether the literal matcher `m` covers `route`. A bare path (no
    /// modifier) end-anchors in Next's source but not in its docs: descendants
    /// are undecided.
    fn path_hit(m: &str, route: &str) -> Tri {
        let ms: Vec<&str> = m.split('/').filter(|s| !s.is_empty()).collect();
        let rs: Vec<&str> = route.split('/').filter(|s| !s.is_empty()).collect();
        let (prefix, modifier) = match ms.split_last() {
            Some((last, rest)) if last.starts_with(':') && last.ends_with(['*', '?', '+']) => {
                (rest, last.chars().last())
            }
            _ => (&ms[..], None),
        };
        if rs.len() < prefix.len() {
            return Tri::No;
        }
        let head = prefix_hit(prefix, &rs);
        match (head, tail_hit(modifier, rs.len() - prefix.len())) {
            (Tri::No, _) | (_, Tri::No) => Tri::No,
            (Tri::Yes, Tri::Yes) if !rs.contains(&"*") => Tri::Yes,
            _ => Tri::Maybe,
        }
    }

    /// The literal segments of the matcher against the route's: a `:param` of
    /// the route is not decidable.
    fn prefix_hit(prefix: &[&str], route: &[&str]) -> Tri {
        let mut head = Tri::Yes;
        for (p, r) in prefix.iter().zip(route) {
            if p.starts_with(':') {
                continue;
            }
            if r.starts_with(':') || *r == "*" {
                head = Tri::Maybe;
            } else if !p.eq_ignore_ascii_case(r) {
                return Tri::No;
            }
        }
        head
    }

    /// The segments after the matcher's literal part, by its modifier.
    fn tail_hit(modifier: Option<char>, tail: usize) -> Tri {
        match modifier {
            Some('*') => Tri::Yes,
            Some('+') if tail >= 1 => Tri::Yes,
            Some('?') if tail <= 1 => Tri::Yes,
            Some(_) => Tri::No,
            None if tail == 0 => Tri::Yes,
            None => Tri::Maybe,
        }
    }

    fn hit(e: &Entry, route: &str) -> Tri {
        let t = match &e.pattern {
            Pattern::Path(m) => path_hit(m, route),
            Pattern::Lookahead(alts) => {
                let mut t = Tri::Yes;
                for a in alts {
                    match paths::route_starts_with(route, &format!("/{a}")) {
                        Tri::Yes => return Tri::No,
                        Tri::Maybe => t = Tri::Maybe,
                        Tri::No => {}
                    }
                }
                t
            }
            Pattern::Opaque => Tri::Maybe,
        };
        match t {
            Tri::Yes if e.conditional => Tri::Maybe,
            t => t,
        }
    }

    /// Whether the matcher entries cover `route`; `None` entries cover all.
    fn covers(entries: &Option<Vec<Entry>>, route: &str) -> Tri {
        let Some(entries) = entries else {
            return Tri::Yes;
        };
        let mut best = Tri::No;
        for e in entries {
            match hit(e, route) {
                Tri::Yes => return Tri::Yes,
                Tri::Maybe => best = Tri::Maybe,
                Tri::No => {}
            }
        }
        best
    }

    /// The `matcher` entries of `config`; `None` covers every path: no matcher,
    /// or one that is not a literal (Next then ignores it, and so is a gap).
    fn matchers(file: &str, text: &str, out: &mut Vec<UiFlowDiagnostic>) -> Option<Vec<Entry>> {
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
        if let [one] = list[..]
            && matches!(one.value, Value::Raw(_))
        {
            let message = format!(
                "Middleware matcher is not a literal ({}); every path is treated as covered.",
                one.source(text)
            );
            diag(out, file, line_of(text, one.at), message);
            return None;
        }
        Some(
            list.into_iter()
                .map(|it| match &it.value {
                    Value::Str(s) => Entry {
                        pattern: pattern(s),
                        conditional: false,
                    },
                    Value::Obj(fields) => {
                        let get = |k: &str| fields.iter().find(|(f, _)| f == k);
                        Entry {
                            pattern: match get("source").map(|(_, v)| &v.value) {
                                Some(Value::Str(s)) => pattern(s),
                                _ => Pattern::Opaque,
                            },
                            conditional: ["has", "missing", "locale"]
                                .iter()
                                .any(|k| get(k).is_some()),
                        }
                    }
                    _ => Entry {
                        pattern: Pattern::Opaque,
                        conditional: false,
                    },
                })
                .collect(),
        )
    }

    /// What a middleware or proxy file does with a request.
    enum Body {
        /// A Clerk middleware without callback.
        Passes,
        Range(usize, usize),
        Unreadable(String),
    }

    fn body_of(text: &str, m: &str) -> Body {
        for name in ["proxy", "middleware"] {
            if let Some((s, e)) = declared_body(m, name) {
                return Body::Range(s, e);
            }
        }
        let Some((pos, word)) = default_export(m) else {
            return Body::Unreadable("no function named proxy or middleware".into());
        };
        let at = pos + "export default".len();
        let b = m.as_bytes();
        if (imports(text).iter()).any(|i| {
            i.local == word
                && i.module.starts_with("@clerk/nextjs")
                && i.imported.as_deref() == Some("clerkMiddleware")
        }) {
            let open = skip_ws(b, skip_ws(b, at) + word.len());
            let empty = b.get(open) == Some(&b'(') && b.get(skip_ws(b, open + 1)) == Some(&b')');
            return if empty {
                Body::Passes
            } else {
                Body::Unreadable("clerkMiddleware callback".into())
            };
        }
        match callable_at(m, at) {
            Callable::Body(s, e) => Body::Range(s, e),
            Callable::Reference(n) => declared_body(m, &n)
                .map(|(s, e)| Body::Range(s, e))
                .unwrap_or_else(|| Body::Unreadable(format!("{n} is not declared in the file"))),
            Callable::Unreadable => Body::Unreadable("the default export".into()),
        }
    }

    /// The proxy or middleware file of the application: the proxy wins, having
    /// both is a gap.
    fn proxy_file<'a>(
        app: &'a App,
        out: &mut Vec<UiFlowDiagnostic>,
    ) -> Option<(&'a String, &'a String)> {
        let present: Vec<(&String, &String)> =
            ["proxy", "src/proxy", "middleware", "src/middleware"]
                .iter()
                .flat_map(|n| [".ts", ".js"].map(|e| format!("{}{n}{e}", app.prefix)))
                .filter_map(|f| app.files.get_key_value(&f))
                .collect();
        let is = |f: &str, n: &str| stem(f.rsplit('/').next().unwrap_or(f)) == n;
        let proxy = present.iter().find(|(f, _)| is(f, "proxy"));
        let found = proxy.or(present.first()).copied()?;
        if proxy.is_some() && present.iter().any(|(f, _)| is(f, "middleware")) {
            let message = "Both a proxy and a middleware file exist; Next.js accepts only one. The proxy is read.";
            diag(out, found.0, 1, message);
        }
        base_path(app, out);
        Some(found)
    }

    /// A `basePath` in the Next.js config: the routes here are read without it.
    fn base_path(app: &App, out: &mut Vec<UiFlowDiagnostic>) {
        for name in ["next.config.js", "next.config.ts"] {
            let config = format!("{}{name}", app.prefix);
            if let Some(t) = app.files.get(&config)
                && let Some(&at) = words(t, "basePath").first()
            {
                let message = "basePath is set; matchers and tests are read on paths without it, as the routes here are.";
                diag(out, &config, line_of(t, at), message);
            }
        }
    }

    /// One proxy or middleware file, read once and asked per route.
    struct Proxy<'a> {
        file: &'a str,
        name: String,
        /// The masked source.
        m: String,
        access: &'a Access,
        entries: Option<Vec<Entry>>,
        body: Body,
    }

    impl Proxy<'_> {
        fn state(&self, route: &str, out: &mut Vec<UiFlowDiagnostic>) -> AccessState {
            match covers(&self.entries, route) {
                Tri::No => AccessState::Undeclared,
                Tri::Maybe => {
                    let message = format!(
                        "Whether the {} matcher covers {route} cannot be decided (a regular expression, has/missing, or a bare path); the route is unknown.",
                        self.name
                    );
                    diag(out, self.file, 1, message);
                    AccessState::Protected(Condition::Unknown(format!("{} matcher", self.name)))
                }
                Tri::Yes => self.body_state(route, out),
            }
        }

        /// What the body requires of `route`; an unreadable body is unknown.
        fn body_state(&self, route: &str, out: &mut Vec<UiFlowDiagnostic>) -> AccessState {
            let why = match &self.body {
                Body::Passes => return AccessState::Undeclared,
                Body::Unreadable(why) => why.clone(),
                Body::Range(s, e) => match self.walked((*s, *e), route, out) {
                    Ok(state) => return state,
                    Err(why) => why,
                },
            };
            let message = format!(
                "The {} is not read ({why}); every route it covers is unknown.",
                self.name
            );
            diag(out, self.file, 1, message);
            AccessState::Protected(Condition::Unknown(format!("{} {why}", self.name)))
        }

        fn walked(
            &self,
            (s, e): (usize, usize),
            route: &str,
            out: &mut Vec<UiFlowDiagnostic>,
        ) -> Result<AccessState, String> {
            let mut walker = tanstack::Walker {
                m: &self.m,
                file: self.file,
                access: self.access,
                out,
                found: Vec::new(),
                mode: tanstack::Mode::Handler,
                route: Some(route),
                allowed: false,
                catch_param: None,
                declared: Vec::new(),
                seen: String::new(),
            };
            walker.walk(s, e, &mut Vec::new())?;
            let found = std::mem::take(&mut walker.found);
            Ok(AccessState::from_parts(tanstack::all(found), false))
        }
    }

    pub(super) fn middleware(
        app: &App,
        access: &Access,
        screens: &mut [(Screen, Owner)],
        endpoints: &mut [EndpointAccess],
        out: &mut Vec<UiFlowDiagnostic>,
    ) {
        let Some((file, text)) = proxy_file(app, out) else {
            return;
        };
        let m = mask(text, true);
        let proxy = Proxy {
            file,
            name: stem(file.rsplit('/').next().unwrap_or(file)).to_string(),
            access,
            entries: matchers(file, &m, out),
            body: body_of(text, &m),
            m,
        };
        for (screen, _) in screens.iter_mut() {
            if let Some(route) = screen.route.clone() {
                let s = proxy.state(&route, out);
                screen.access = std::mem::take(&mut screen.access).and(s);
            }
        }
        for e in endpoints.iter_mut() {
            let s = proxy.state(&e.path, out);
            e.access = std::mem::take(&mut e.access).and(s);
        }
    }
}

/// Reading the body of a wrapper component or a higher-order handler function
/// for deny exits (S3, S6).
mod guard {
    use super::paths::*;
    use super::tanstack::*;
    use super::*;

    /// `(start, end)` of the expression of `const name = (…) => expr` (no block).
    fn expr_arrow(m: &str, name: &str) -> Option<(usize, usize)> {
        let b = m.as_bytes();
        for pos in words(m, name) {
            let before = m[..pos].trim_end();
            let j = skip_ws(b, pos + name.len());
            if !matches!(
                trailing_ident(before).as_deref(),
                Some("const" | "let" | "var")
            ) || b.get(j) != Some(&b'=')
            {
                continue;
            }
            let mut i = skip_ws(b, j + 1);
            if ident_at(m, i) == "async" {
                i = skip_ws(b, i + "async".len());
            }
            let after = if b.get(i) == Some(&b'(') {
                block_end(m, i, b'(', b')')
            } else {
                i + ident_at(m, i).len()
            };
            let mut k = skip_ws(b, after);
            if b.get(k) == Some(&b':')
                && let Some(d) = m[k..].find("=>")
            {
                k += d;
            }
            if m[k..].starts_with("=>") {
                let s = skip_ws(b, k + 2);
                return (b.get(s) != Some(&b'{')).then(|| (s, simple_end(m, s)));
            }
        }
        None
    }

    /// The declared name: the given one, or the file's default export.
    fn declared_name(m: &str, name: Option<&str>) -> Result<String, String> {
        match name {
            Some(n) => Ok(n.to_string()),
            None => next::default_name(m)
                .ok_or_else(|| "its default export is not a declared name".to_string()),
        }
    }

    /// What component `name` (`None`: the default export) of a file requires of
    /// what it renders. Deny exits are early `return`s that do not render the
    /// children; a component that never renders them, or renders them under a
    /// condition, is not read.
    pub(super) fn component(
        access: &Access,
        out: &mut Vec<UiFlowDiagnostic>,
        (file, text): (&str, &str),
        name: Option<&str>,
    ) -> Result<Vec<Condition>, String> {
        let m = mask(text, true);
        let name = declared_name(&m, name)?;
        if let Some((s, e)) = declared_body(&m, &name) {
            let mut walker = Walker {
                m: &m,
                file,
                access,
                out,
                found: Vec::new(),
                mode: Mode::Wrapper,
                route: None,
                allowed: false,
                catch_param: None,
                declared: Vec::new(),
                seen: String::new(),
            };
            walker.walk(s, e, &mut Vec::new())?;
            if !walker.allowed {
                return Err("it never renders its children".into());
            }
            if let Some(n) = walker
                .declared
                .iter()
                .find(|n| words(&walker.seen, n).is_empty())
            {
                return Err(format!("{n} comes from a call and is used in no check"));
            }
            return Ok(all(walker.found).into_iter().collect());
        }
        let (s, e) = expr_arrow(&m, &name)
            .ok_or_else(|| format!("{name} is not a function declaration in {file}"))?;
        let expr = &m[s..e];
        if words(expr, "children").is_empty() && words(expr, "Outlet").is_empty() {
            return Err("it never renders its children".into());
        }
        if children_branching(expr) {
            return Err("it renders its children under a condition".into());
        }
        Ok(Vec::new())
    }

    /// What higher-order function `name` of a file requires of the handler it
    /// wraps: the deny exits of the function it returns.
    pub(super) fn higher_order(
        access: &Access,
        out: &mut Vec<UiFlowDiagnostic>,
        (file, text): (&str, &str),
        name: Option<&str>,
    ) -> Result<Vec<Condition>, String> {
        let m = mask(text, true);
        let name = declared_name(&m, name)?;
        let b = m.as_bytes();
        let from = if let Some((s, e)) = declared_body(&m, &name) {
            let r = words(&m[s..e], "return")
                .into_iter()
                .next()
                .ok_or("it returns nothing")?;
            s + r + "return".len()
        } else {
            expr_arrow(&m, &name)
                .map(|(s, _)| s)
                .ok_or_else(|| format!("{name} is not a function declaration in {file}"))?
        };
        match callable_at(&m, from) {
            Callable::Body(s, e) => {
                let mut walker = Walker {
                    m: &m,
                    file,
                    access,
                    out,
                    found: Vec::new(),
                    mode: Mode::Handler,
                    route: None,
                    allowed: false,
                    catch_param: None,
                    declared: Vec::new(),
                    seen: String::new(),
                };
                walker.walk(s, e, &mut Vec::new())?;
                Ok(all(walker.found).into_iter().collect())
            }
            // It returns the wrapped handler: transparent.
            Callable::Reference(_) => Ok(Vec::new()),
            Callable::Unreadable => {
                let i = skip_ws(b, from);
                let id = ident_at(&m, i);
                let rest = m[i + id.len()..].trim_start();
                if !id.is_empty() && (rest.is_empty() || rest.starts_with([';', '}'])) {
                    Ok(Vec::new())
                } else {
                    Err("it returns something that is not read".into())
                }
            }
        }
    }
}

/// Where code sits: the open bracket, the function, and whether it runs
/// conditionally.
mod scope {
    use super::shape::*;
    use super::*;

    /// The nearest `(`, `[` or `{` before `at` that is still open; with `stop`, a
    /// `;` at the same level ends the search (a parameter list has none).
    pub(super) fn opener_before(bare: &str, at: usize, stop: bool) -> Option<usize> {
        let b = bare.as_bytes();
        let mut depth = 0;
        for i in (0..at).rev() {
            match b[i] {
                b')' | b']' | b'}' => depth += 1,
                b'(' | b'[' | b'{' if depth == 0 => return Some(i),
                b'(' | b'[' | b'{' => depth -= 1,
                b';' if depth == 0 && stop => return None,
                _ => {}
            }
        }
        None
    }

    /// Whether a block opened after `head` is a function body.
    fn is_function_head(head: &str) -> bool {
        let h = head.trim_end();
        if h.ends_with("=>") {
            return true;
        }
        let Some(open) = h
            .strip_suffix(')')
            .and_then(|_| opener_before(h, h.len() - 1, false))
        else {
            return false;
        };
        let word = trailing_ident(h[..open].trim_end()).unwrap_or_default();
        !matches!(
            word.as_str(),
            "if" | "for" | "while" | "switch" | "catch" | "with"
        )
    }

    /// The `{` of the function body around `at`, if it sits in a block body.
    fn fn_body_start(bare: &str, at: usize) -> Option<usize> {
        let mut at = at;
        loop {
            let o = opener_before(bare, at, false)?;
            if bare.as_bytes()[o] == b'{' && is_function_head(&bare[..o]) {
                return Some(o);
            }
            at = o;
        }
    }

    /// The `return` statements of the function body at `fb`, not those of
    /// functions nested in it.
    fn owned_returns(bare: &str, fb: usize) -> usize {
        let end = close(bare, fb).min(bare.len());
        words(&bare[..end], "return")
            .into_iter()
            .filter(|&p| p > fb && fn_body_start(bare, p) == Some(fb))
            .count()
    }

    /// Whether the text before `pos` ends in `&&`, `||`, `??`, `?` or `:` (past
    /// spaces and `(`): the element is one branch of a conditional.
    fn conditional_before(bare: &str, pos: usize) -> bool {
        let before = bare[..pos].trim_end_matches(|c: char| c.is_whitespace() || c == '(');
        ["&&", "||", "??", "?", ":"]
            .iter()
            .any(|op| before.ends_with(op))
    }

    /// Why the routes rendered at `pos` (a `<Routes>` start or a router call) may
    /// not be rendered: inside a `{…}` container or behind an operator, or in a
    /// function with more than one `return`.
    pub(super) fn rendered_conditionally(text: &str, bare: &str, pos: usize) -> Option<String> {
        let open = open_elements(text, pos).unwrap_or_default();
        let starts: Vec<usize> = open.iter().map(|e| e.2).chain([pos]).collect();
        let in_container =
            (open.iter().enumerate()).any(|(i, e)| brace_depth(bare, e.1, starts[i + 1]) > 0);
        let why = if in_container || conditional_before(bare, starts[0]) {
            "inside a condition or expression"
        } else if fn_body_start(bare, pos).is_some_and(|fb| owned_returns(bare, fb) > 1) {
            "after an early return"
        } else {
            return None;
        };
        Some(format!(
            "Routes are rendered under a condition ({why}); whether they are reachable is unknown."
        ))
    }

    /// The condition around the `<Routes>` element holding the route at `pos`.
    pub(super) fn routes_condition(text: &str, bare: &str, pos: usize) -> Option<(usize, String)> {
        let open = open_elements(text, pos)?;
        let start = open.iter().rev().find(|e| e.0 == "Routes")?.2;
        rendered_conditionally(text, bare, start).map(|m| (start, m))
    }

    /// Whether the `<Route` at `pos` is an argument of `createRoutesFromElements(`.
    pub(super) fn in_routes_call(bare: &str, pos: usize) -> bool {
        let Some(o) = opener_before(bare, pos, false) else {
            return false;
        };
        let head = bare[..o].trim_end();
        bare.as_bytes()[o] == b'('
            && ["createRoutesFromElements", "createRoutesFromChildren"]
                .iter()
                .any(|c| head.ends_with(c))
    }

    /// The `{` of each non-empty container directly in a `<Routes>` or `<Route>`
    /// body that is not a whole `.map` call (`maps` are their offsets): a name,
    /// a call, a condition. React Router renders only routes there.
    pub(super) fn container_gaps(text: &str, bare: &str, maps: &[usize]) -> Vec<usize> {
        let mut out = Vec::new();
        for (q, _) in bare.match_indices('{') {
            let Some(open) = open_elements(text, q) else {
                continue;
            };
            let Some(a) = open.iter().rposition(|e| matches!(e.0, "Route" | "Routes")) else {
                continue;
            };
            let end = close(bare, q).min(bare.len());
            let empty = bare[q + 1..end.saturating_sub(1).max(q + 1)]
                .trim()
                .is_empty();
            let whole_map = maps.iter().any(|&m| m > q && m < end);
            if brace_depth(bare, open[a].1, q) == 0 && !empty && !whole_map {
                out.push(q);
            }
        }
        out
    }
}

/// Reading `.map` callbacks and where a `<Route>` sits (see `router`).
mod shape {
    use super::scope::*;
    use super::*;

    /// The object parameter of a callback: its name, or its destructuring.
    pub(super) type Param<'a> = (Option<&'a str>, Option<(&'a str, &'a str)>);

    /// Offsets of the `<Route` start tags in `text[from..to]`.
    pub(super) fn route_tags(text: &str, from: usize, to: usize) -> Vec<usize> {
        text[from..to.min(text.len())]
            .match_indices("<Route")
            .map(|(i, _)| from + i)
            .filter(|i| {
                !text[i + "<Route".len()..].starts_with(|c: char| is_ident_char(c) || c == '.')
            })
            .collect()
    }

    /// The destructuring `{ element, ...rest }` as `(element, rest)`.
    pub(super) fn destructured(s: &str) -> Option<(&str, &str)> {
        let parts: Vec<&str> = s.split(',').map(str::trim).collect();
        let [element, rest] = parts[..] else {
            return None;
        };
        let rest = rest.strip_prefix("...")?;
        (element == "element" && is_ident(rest)).then_some((element, rest))
    }

    /// Parameters and body of an arrow or `function` callback; a `function` has a block.
    pub(super) fn callback_parts(inner: &str) -> Option<(&str, &str)> {
        let Some(rest) = inner.strip_prefix("function") else {
            return inner.split_once("=>");
        };
        let rest = rest.trim_start().trim_start_matches(is_ident_char);
        let (params, body) = rest.trim_start().strip_prefix('(')?.split_once(')')?;
        let body = body.trim();
        body.starts_with('{').then_some((params, body))
    }

    pub(super) fn object_param(params: &str) -> Option<Param<'_>> {
        let params = params.trim();
        let params = params
            .strip_prefix('(')
            .and_then(|p| p.strip_suffix(')'))
            .unwrap_or(params);
        if let Some(rest) = params.strip_prefix('{') {
            let (inside, tail) = rest.split_once('}')?;
            let tail = tail.trim();
            let ok = tail.is_empty() || tail.starts_with([',', ':']);
            let d = destructured(inside).filter(|_| ok)?;
            return Some((None, Some(d)));
        }
        let name = params.split(',').next()?.split(':').next()?.trim();
        is_ident(name).then_some((Some(name), None))
    }

    /// `const { element, ...rest } = param;` at the start of a block: the
    /// destructuring and what follows.
    pub(super) fn const_destructure<'a>(
        block: &'a str,
        param: Option<&str>,
    ) -> Option<((&'a str, &'a str), &'a str)> {
        let decl = block.strip_prefix("const")?.trim();
        let (inside, tail) = decl.strip_prefix('{')?.split_once('}')?;
        let tail = tail.trim().strip_prefix('=')?.trim();
        let name = ident_at(tail, 0);
        let rest = tail[name.len()..].trim_start().strip_prefix(';')?;
        (param == Some(name)).then_some(())?;
        Some((destructured(inside)?, rest))
    }

    /// The returned element of a callback body, and the destructuring the
    /// block adds to the parameter's.
    pub(super) fn returned<'a>(
        body: &'a str,
        param: &Param<'a>,
    ) -> Option<(&'a str, Option<(&'a str, &'a str)>)> {
        let body = body.trim();
        let Some(block) = body.strip_prefix('{') else {
            return Some((body, param.1));
        };
        let block = block.strip_suffix('}')?.trim();
        let (destructure, after) = match const_destructure(block, param.0) {
            Some(_) if param.1.is_some() => return None,
            Some((d, rest)) => (Some(d), rest),
            None => (param.1, block),
        };
        let ret = after.trim().strip_prefix("return")?.trim();
        Some((ret.strip_suffix(';').unwrap_or(ret).trim(), destructure))
    }

    /// Only the `key` and `element` attributes (as read): the spread carries the rest.
    pub(super) fn plain_attrs(text: &str, route: &Tag, element: &str) -> bool {
        route.attrs.iter().all(|a| match (&a.name[..], &a.value) {
            ("key", _) => true,
            ("element", AttrValue::Expr(at)) => {
                Parser { s: text, i: *at }.value(0).source(text) == element
            }
            _ => false,
        })
    }

    /// The JSX elements open at `at`, outermost first, with where their children
    /// and their start tag begin; fragments have the name `""`. `None` inside a start tag.
    pub(super) fn open_elements(text: &str, at: usize) -> Option<Vec<(&str, usize, usize)>> {
        let before = &text[..at];
        // `useState<Foo>(` and `a<b` are no elements: JSX never follows a name.
        let jsx_start = |i: &usize| *i == 0 || !is_ident_char(before.as_bytes()[*i - 1] as char);
        let mut events: Vec<(usize, Option<&str>)> = start_tags(before)
            .into_iter()
            .map(|(i, name)| (i, Some(name)))
            .chain(before.match_indices("<>").map(|(i, _)| (i, Some(""))))
            .filter(|(i, name)| name.is_none() || jsx_start(i))
            .chain(before.match_indices("</").map(|(i, _)| (i, None)))
            .collect();
        events.sort();
        let mut open = Vec::new();
        for (pos, name) in events {
            let Some(name) = name else {
                open.pop();
                continue;
            };
            let (end, closing) = match name {
                "" => (pos + 2, false),
                _ => {
                    let t = tag(text, pos + 1 + name.len());
                    (t.end, t.self_closing)
                }
            };
            if end > at {
                return None;
            }
            if !closing {
                open.push((if name.ends_with("Fragment") { "" } else { name }, end, pos));
            }
        }
        Some(open)
    }

    /// `{` minus `}` in `bare[from..to]` (strings and comments blanked).
    pub(super) fn brace_depth(bare: &str, from: usize, to: usize) -> i32 {
        bare[from..to].bytes().fold(0, |d, c| match c {
            b'{' => d + 1,
            b'}' => d - 1,
            _ => d,
        })
    }

    /// Whether the `<Route` at `pos` is no route to read: in a `{…}` container
    /// below its `<Routes>` or `<Route>` (a condition, an array, a fragment behind
    /// an operator), or in no such element and no `createRoutesFrom…(` call
    /// (defined elsewhere, rendered through a name).
    pub(super) fn route_in_container(text: &str, bare: &str, pos: usize) -> bool {
        let Some(open) = open_elements(text, pos) else {
            return false;
        };
        match open.iter().rposition(|e| matches!(e.0, "Route" | "Routes")) {
            Some(a) => brace_depth(bare, open[a].1, pos) > 0,
            None => !in_routes_call(bare, pos),
        }
    }

    /// `"Routes"` or `"Route"` when the `.map` at `pos` is the whole container
    /// directly in that element (fragments aside), else `None`.
    pub(super) fn map_place<'t>(text: &'t str, bare: &str, pos: usize) -> Option<&'t str> {
        let open = open_elements(text, pos)?;
        let at = open
            .iter()
            .rposition(|e| matches!(e.0, "Route" | "Routes"))?;
        let direct = open[at + 1..].iter().all(|e| e.0.is_empty());
        (direct && brace_depth(bare, open[at].1, pos) == 1).then_some(open[at].0)
    }

    /// Whether `src` is one JSX element tree and nothing else: no condition,
    /// call or identifier around it.
    pub(super) fn single_element(src: &str) -> bool {
        let mut t = src.trim();
        while let Some(inner) = t.strip_prefix('(').and_then(|r| r.strip_suffix(')')) {
            t = inner.trim();
        }
        if let Some(inner) = t.strip_prefix("<>").and_then(|r| r.strip_suffix("</>")) {
            return single_element(inner);
        }
        let Some((0, name)) = start_tags(t).into_iter().next() else {
            return false;
        };
        let open = tag(t, 1 + name.len());
        if open.self_closing {
            return open.end == t.len();
        }
        let mut events: Vec<(usize, Option<bool>)> = start_tags(t)
            .into_iter()
            .map(|(i, n)| (i, Some(n.len())))
            .map(|(i, n)| (i, n.map(|n| tag(t, i + 1 + n).self_closing)))
            .chain(t.match_indices("<>").map(|(i, _)| (i, Some(false))))
            .chain(t.match_indices("</").map(|(i, _)| (i, None)))
            .filter(|(i, _)| *i >= open.end)
            .collect();
        events.sort();
        let mut depth = 1;
        for (pos, kind) in events {
            match kind {
                Some(false) => depth += 1,
                Some(true) => {}
                None => {
                    depth -= 1;
                    if depth == 0 {
                        let end = t[pos..].find('>').map(|e| pos + e + 1);
                        return end == Some(t.len());
                    }
                }
            }
        }
        false
    }

    pub(super) const NOT_DECLARED: &str = "not declared in this file";

    const MUTATORS: [&str; 9] = [
        "push",
        "pop",
        "shift",
        "unshift",
        "splice",
        "sort",
        "reverse",
        "fill",
        "copyWithin",
    ];

    fn is_assign(rest: &str) -> bool {
        let plain = rest.starts_with('=') && !rest.starts_with("==") && !rest.starts_with("=>");
        plain
            || [
                "+=", "-=", "*=", "/=", "%=", "**=", "||=", "&&=", "??=", "<<=", ">>=", "&=", "|=",
                "^=",
            ]
            .iter()
            .any(|op| rest.starts_with(op))
    }

    fn ends_with_word(head: &str, word: &str) -> bool {
        head.strip_suffix(word)
            .is_some_and(|r| !r.ends_with(is_ident_char))
    }

    /// Whether the parenthesis at `o` is the parameter list of an arrow or `function`.
    fn is_param_list(bare: &str, o: usize) -> bool {
        let after = bare[close(bare, o).min(bare.len())..].trim_start();
        let head = bare[..o].trim_end();
        let named = head.trim_end_matches(is_ident_char).trim_end();
        after.starts_with("=>")
            || ends_with_word(named, "function")
            || ends_with_word(head, "function")
    }

    /// Whether the name at `p` is bound by a parameter list (a name, a
    /// destructuring, a default, a rest) or by `const { … }` / `[ … ]`.
    fn is_bound(bare: &str, p: usize) -> bool {
        let mut at = p;
        while let Some(o) = opener_before(bare, at, true) {
            if bare.as_bytes()[o] == b'(' {
                return is_param_list(bare, o);
            }
            let head = bare[..o].trim_end();
            if ["const", "let", "var"]
                .iter()
                .any(|k| ends_with_word(head, k))
            {
                return true;
            }
            at = o;
        }
        false
    }

    /// Whether `rest` (what follows a name) mutates through members or indexes:
    /// `.push(`, `[0].element =`, `.children.splice(`.
    fn mutates_through<'a>(bare: &'a str, mut rest: &'a str) -> bool {
        let mut links = 0;
        loop {
            if let Some(member) = rest.strip_prefix('.') {
                let member = member.trim_start();
                let id = ident_at(member, 0);
                let tail = member[id.len()..].trim_start();
                if id.is_empty() || (MUTATORS.contains(&id) && tail.starts_with('(')) {
                    return !id.is_empty();
                }
                rest = tail;
            } else if rest.starts_with('[') {
                let open = bare.len() - rest.len();
                rest = bare[close(bare, open).min(bare.len())..].trim_start();
            } else {
                return links > 0 && is_assign(rest);
            }
            links += 1;
        }
    }

    /// How the declaration of `name` at `dp` is changed or hidden elsewhere.
    fn changed(bare: &str, name: &str, dp: usize) -> Option<&'static str> {
        for p in words(bare, name).into_iter().filter(|&p| p != dp) {
            let before = bare[..p].trim_end();
            if before.ends_with('.') && !before.ends_with("...") {
                continue;
            }
            let rest = bare[p + name.len()..].trim_start();
            if is_assign(rest) {
                return Some("reassigned");
            }
            if rest.starts_with("=>") || is_bound(bare, p) {
                return Some("shadowed by a parameter or a pattern");
            }
            if mutates_through(bare, rest) {
                return Some("mutated");
            }
        }
        None
    }

    /// The offset of the `[` of `const NAME = [`, or why `NAME` cannot be read
    /// as that array: declared twice, not `const`, changed anywhere in the file,
    /// or not initialised with an array literal.
    pub(super) fn const_array(text: &str, name: &str) -> Result<usize, String> {
        let bare = mask(text, false);
        let b = bare.as_bytes();
        let decls: Vec<(&str, usize)> = ["const", "let", "var", "function", "class"]
            .into_iter()
            .flat_map(|kw| words(&bare, kw).into_iter().map(move |p| (kw, p)))
            .filter(|&(kw, p)| ident_at(&bare, skip_ws(b, p + kw.len())) == name)
            .collect();
        let (kw, pos) = match decls[..] {
            [] => return Err(NOT_DECLARED.into()),
            [one] => one,
            _ => return Err("declared more than once (shadowed)".into()),
        };
        if kw != "const" {
            return Err(format!("declared with {kw}, not const"));
        }
        let dp = skip_ws(b, pos + kw.len());
        if let Some(how) = changed(&bare, name, dp) {
            return Err(format!("{how} in this file"));
        }
        let k = dp + name.len();
        let eq = bare[k..].find(['=', ';']).map(|e| k + e);
        let after = eq.filter(|&e| b[e] == b'=').map(|e| skip_ws(b, e + 1));
        after
            .filter(|&a| b.get(a) == Some(&b'['))
            .ok_or_else(|| "not initialised with an array literal".into())
    }
}

/// Routes rendered behind a conditional operator.
mod conditional {
    use super::shape::*;

    /// `(not read, condition)` of the `<Route` at `pos`: inside a `{…}` container
    /// that is not read, or behind a conditional operator in one.
    pub(super) fn container_state(text: &str, bare: &str, pos: usize) -> (bool, Option<String>) {
        if !route_in_container(text, bare, pos) {
            return (false, None);
        }
        let cond = conditional_container(text, bare, pos);
        (cond.is_none(), cond)
    }

    /// The condition text when the `<Route` at `pos` stands right behind a
    /// conditional operator in a `{…}` container (`{c && <Route/>}`,
    /// `{c ? <Route/> : null}`, also inside a fragment there): the route exists
    /// only under that condition.
    pub(super) fn conditional_container(text: &str, bare: &str, pos: usize) -> Option<String> {
        let open = open_elements(text, pos)?;
        let at = open
            .iter()
            .rposition(|e| matches!(e.0, "Route" | "Routes"))?;
        let from = open[at].1;
        let mut depth = 0i32;
        let brace = bare[from..pos]
            .bytes()
            .enumerate()
            .rev()
            .find_map(|(i, c)| {
                match c {
                    b'}' => depth += 1,
                    b'{' if depth == 0 => return Some(from + i),
                    b'{' => depth -= 1,
                    _ => {}
                }
                None
            })?;
        let mut pre = text[brace + 1..pos].trim();
        loop {
            let t = pre
                .trim_end_matches(['(', ' ', '\n', '\t'])
                .trim_end_matches("<>")
                .trim_end_matches("<React.Fragment>")
                .trim_end();
            if t.len() == pre.len() {
                break;
            }
            pre = t;
        }
        let cond = ["&&", "||", "??", "?"]
            .iter()
            .find_map(|op| pre.strip_suffix(op))
            .or_else(|| pre.ends_with(':').then_some(pre))?;
        let cond = cond.trim();
        (!cond.is_empty() && !cond.contains("<Route")).then(|| cond.to_string())
    }
}

/// React Router route trees from JSX and route objects.
mod router {
    use super::conditional::*;
    use super::scope::*;
    use super::shape::*;
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
        /// Indices of the `.map` route arrays rendered inside this route.
        maps: Vec<usize>,
        /// Sits in a conditional expression (`a && <Route/>`): not read.
        stray: bool,
        /// The condition text when the route is the direct operand of `c && <Route/>`
        /// or `c ? <Route/> : …`: the route exists only under it.
        container: Option<String>,
        /// Where the `<Routes>` or router call holding this route is rendered
        /// under a condition, and why.
        cond: Option<(usize, String)>,
    }

    struct Ctx<'a> {
        app: &'a App,
        access: &'a Access,
        file: &'a str,
        text: &'a str,
        maps: &'a [Mapped<'a>],
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
        access: &Access,
        file: &str,
        text: &str,
        screens: &mut Vec<(Screen, Owner)>,
        out: &mut Vec<UiFlowDiagnostic>,
    ) {
        let mapped = mapped(text);
        let mut ctx = Ctx {
            app,
            access,
            file,
            text,
            maps: &mapped,
            screens,
            out,
        };
        let bare = mask(text, false);
        let (mut roots, loose, stray) = jsx(text, &bare, &mapped);
        roots.extend(objects(&mut ctx, &bare));
        let mut conds = BTreeMap::new();
        for root in &roots {
            let inherited: Vec<Condition> = (root.cond.iter())
                .map(|(at, why)| {
                    conds.insert(*at, why.clone());
                    Condition::Unknown(why.clone())
                })
                .collect();
            walk(&mut ctx, root, "", &inherited, 0);
        }
        for i in loose {
            let cond = routes_condition(text, &bare, mapped[i].at);
            let inherited: Vec<Condition> = (cond.iter())
                .map(|(at, why)| {
                    conds.insert(*at, why.clone());
                    Condition::Unknown(why.clone())
                })
                .collect();
            map_routes(&mut ctx, &mapped[i], "", &inherited);
        }
        for (at, why) in conds {
            ctx.diag(at, why);
        }
        if text.contains("<Route") {
            let maps: Vec<usize> = (mapped.iter().filter(|m| m.exact)).map(|m| m.at).collect();
            for q in container_gaps(text, &bare, &maps) {
                let message = "A {…} container among the routes is not read (only a whole .map call is); the routes it renders are unknown.";
                ctx.diag(q, message.into());
            }
        }
        for i in stray {
            let message = "A .map among the routes is not read (only `{array.map(plain callback)}` directly in <Routes> or a <Route> is); its routes are unknown.";
            ctx.diag(mapped[i].at, message.into());
        }
    }

    /// `NAME.map(cb)` whose callback returns `<Route {...x}>`: where the call
    /// is, its callback's span and the Route tag.
    struct Mapped<'t> {
        at: usize,
        name: Option<String>,
        span: (usize, usize),
        route: Option<(&'t str, Tag)>,
        /// The call is the whole `{…}` container: nothing before or after.
        exact: bool,
        /// `.flatMap`, which is never read.
        flat: bool,
    }

    fn mapped(text: &str) -> Vec<Mapped<'_>> {
        let masked = mask(text, true);
        let mut calls: Vec<(usize, usize, bool)> = text
            .match_indices(".map(")
            .map(|(i, _)| (i, i + 4, false))
            .chain(
                text.match_indices(".flatMap(")
                    .map(|(i, _)| (i, i + 8, true)),
            )
            .collect();
        calls.sort();
        let mut out = Vec::new();
        for (pos, open, flat) in calls {
            let before = &text[..pos];
            let span = (open, close(text, open));
            let route = route_tags(text, span.0, span.1).into_iter().find_map(|i| {
                let tag = tag(text, i + "<Route".len());
                let head = &text[i..tag.end];
                (flat || head.contains("{...")).then_some((head, tag))
            });
            let name =
                trailing_ident(before).filter(|n| !before[..before.len() - n.len()].ends_with('.'));
            let exact = name.as_ref().is_some_and(|n| {
                let (head, tail) = (
                    &masked[..pos - n.len()],
                    &masked[span.1.min(masked.len())..],
                );
                head.trim_end().ends_with('{') && tail.trim_start().starts_with('}')
            });
            // A `.map` without a route in it only matters as a whole container.
            if route.is_none() && (flat || !exact) {
                continue;
            }
            out.push(Mapped {
                at: pos,
                name,
                span,
                route,
                exact,
                flat,
            });
        }
        out
    }

    /// Whether the callback of `m` is one of the two shapes that render the
    /// array's objects unchanged: `(p) => <Route {...p} />`, or a block that
    /// only destructures `p` into `{ element, ...rest }` and returns
    /// `<Route {...rest} element={element} />`. Conditions, wrappers and extra
    /// statements are not read.
    fn callback_is_read(text: &str, m: &Mapped, (head, route): &(&str, Tag)) -> bool {
        let (open, end) = m.span;
        let inner = text[open + 1..end.saturating_sub(1).max(open + 1)].trim();
        let offset = |s: &str| s.as_ptr() as usize - text.as_ptr() as usize;
        let one_route = route_tags(text, open, end).len() == 1 && head.matches("{...").count() == 1;
        let parsed = callback_parts(inner).and_then(|(params, body)| {
            let param = object_param(params)?;
            Some((returned(body, &param)?, param.0))
        });
        let Some(((ret, destructure), param)) = parsed.filter(|_| one_route) else {
            return false;
        };
        let ret = ret
            .strip_prefix('(')
            .and_then(|r| r.strip_suffix(')'))
            .unwrap_or(ret)
            .trim();
        let whole = ret.starts_with("<Route")
            && offset(ret) == offset(head)
            && route.end == offset(ret) + ret.len()
            && route.self_closing;
        let spread =
            (head.split_once("{...").and_then(|(_, r)| r.split_once('}'))).map(|(n, _)| n.trim());
        let (expected_spread, element) = match (destructure, param) {
            (Some((element, rest)), _) => (rest, element.to_string()),
            (None, Some(p)) => (p, format!("{p}.element")),
            (None, None) => return false,
        };
        whole
            && spread == Some(expected_spread)
            && (destructure.is_none() || route.attrs.iter().any(|a| a.name == "element"))
            && plain_attrs(text, route, &element)
    }

    /// The routes of the array literal a `.map` call renders as `<Route {...x}>`:
    /// local, or imported (default or named) from a scanned file. Any other
    /// shape is a gap.
    fn map_routes(ctx: &mut Ctx, m: &Mapped, prefix: &str, inherited: &[Condition]) {
        let (app, text, file) = (ctx.app, ctx.text, ctx.file);
        let head = m.route.as_ref().map_or("", |r| r.0);
        let Some(name) = m.name.as_deref().filter(|_| {
            m.exact
                && !m.flat
                && m.route
                    .as_ref()
                    .is_some_and(|r| callback_is_read(text, m, r))
        }) else {
            let message = format!(
                "Routes rendered by .map are not read (only `{{array.map(plain callback)}}` as a whole container); they are unknown: {head}"
            );
            return ctx.diag(m.at, message);
        };
        let (file, text, at) = match locate_array(app, file, text, name) {
            Ok(found) => found,
            Err(why) => {
                let message = format!("Route array {name} is {why}; its routes are unknown.");
                return ctx.diag(m.at, message);
            }
        };
        let mut other = Ctx {
            app,
            access: ctx.access,
            file,
            text,
            maps: &[],
            screens: &mut *ctx.screens,
            out: &mut *ctx.out,
        };
        let items = Parser { s: text, i: at }.array(0);
        let nodes: Vec<Node> = items
            .iter()
            .filter_map(|it| object(&mut other, it, 0))
            .collect();
        for node in &nodes {
            walk(&mut other, node, prefix, inherited, 0);
        }
    }

    /// The file, text and offset of the array `name` stands for: declared in
    /// this file, or imported (default or named) from a scanned file.
    fn locate_array<'a>(
        app: &'a App,
        file: &'a str,
        text: &'a str,
        name: &str,
    ) -> Result<(&'a str, &'a str, usize), String> {
        match const_array(text, name) {
            Ok(at) => return Ok((file, text, at)),
            Err(why) if why != NOT_DECLARED => return Err(why),
            Err(_) => {}
        }
        let import = (imports(text).into_iter().find(|i| i.local == name))
            .ok_or("not declared in this file or imported")?;
        let target = app
            .resolve(file, &import.module)
            .ok_or_else(|| format!("imported from {}, which resolves to no file", import.module))?;
        let (target, other) = app.files.get_key_value(target).expect("scanned file");
        let at = match import.imported.as_deref() {
            None => default_array(other)?,
            Some("*") => return Err("a namespace import".into()),
            Some(n) => const_array(other, n)?,
        };
        Ok((target.as_str(), other.as_str(), at))
    }

    /// The offset of the array `export default` names or holds.
    fn default_array(text: &str) -> Result<usize, String> {
        let b = text.as_bytes();
        let Some(found) = text.find("export default ") else {
            return Err("not exported by default".into());
        };
        let at = skip_ws(b, found + "export default ".len());
        match b.get(at) {
            Some(b'[') => Ok(at),
            _ => const_array(text, ident_at(text, at)),
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

    /// `(offset, kind)` of the route events of `text`, in order: 0 closes a
    /// route, 1 opens one, `2 + n` is `.map` number `n`. Routes inside a mapped
    /// callback are left to the array.
    fn route_events(text: &str, mapped: &[Mapped]) -> Vec<(usize, usize)> {
        let skip = |i: &usize| {
            let inside = |m: &&Mapped| m.route.is_some() && (m.span.0..m.span.1).contains(i);
            !mapped.iter().any(|m| inside(&m))
        };
        let mut events: Vec<(usize, usize)> = text
            .match_indices("<Route")
            .filter(|(i, _)| {
                !text[i + "<Route".len()..].starts_with(|c: char| is_ident_char(c) || c == '.')
            })
            .map(|(i, _)| (i, 1))
            .chain(text.match_indices("</Route>").map(|(i, _)| (i, 0)))
            .filter(|(i, _)| skip(i))
            .chain(mapped.iter().enumerate().map(|(n, m)| (m.at, 2 + n)))
            .collect();
        events.sort();
        events
    }

    /// Where a `.map` goes: into the open route, among the loose ones outside
    /// any route, or nowhere it is read.
    enum Place {
        Child,
        Loose,
        Stray,
        Ignore,
    }

    fn place(text: &str, bare: &str, pos: usize, reads: bool, parent: bool) -> Place {
        match (map_place(text, bare, pos), parent) {
            (Some("Route"), true) if reads => Place::Child,
            (Some("Routes"), false) if reads => Place::Loose,
            (Some(_), _) => Place::Stray,
            (None, _) if reads => Place::Stray,
            _ => Place::Ignore,
        }
    }

    /// The `<Route>` tree of `text`, and the `.map` arrays outside any route.
    fn jsx(text: &str, bare: &str, mapped: &[Mapped]) -> (Vec<Node>, Vec<usize>, Vec<usize>) {
        let attach =
            |stack: &mut Vec<Node>, roots: &mut Vec<Node>, node: Node| match stack.last_mut() {
                Some(parent) => parent.children.push(node),
                None => roots.push(node),
            };
        let (mut stack, mut roots) = (Vec::new(), Vec::new());
        let (mut loose, mut stray) = (Vec::new(), Vec::new());
        for (pos, kind) in route_events(text, mapped) {
            match kind {
                0 => {
                    if let Some(node) = stack.pop() {
                        attach(&mut stack, &mut roots, node);
                    }
                }
                1 => {
                    let tag = tag(text, pos + "<Route".len());
                    let mut node = jsx_node(text, pos, &tag);
                    (node.stray, node.container) = container_state(text, bare, pos);
                    if stack.is_empty() {
                        node.cond = routes_condition(text, bare, pos);
                    }
                    if tag.self_closing {
                        attach(&mut stack, &mut roots, node);
                    } else {
                        stack.push(node);
                    }
                }
                n => {
                    let reads = mapped[n - 2].route.is_some();
                    match place(text, bare, pos, reads, !stack.is_empty()) {
                        Place::Child => stack.last_mut().expect("parent").maps.push(n - 2),
                        Place::Loose => loose.push(n - 2),
                        Place::Stray => stray.push(n - 2),
                        Place::Ignore => {}
                    }
                }
            }
        }
        while let Some(node) = stack.pop() {
            attach(&mut stack, &mut roots, node);
        }
        (roots, loose, stray)
    }

    fn objects(ctx: &mut Ctx, bare: &str) -> Vec<Node> {
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
                    match const_array(text, name) {
                        Ok(at) => at,
                        Err(why) => {
                            let message =
                                format!("Route array {name} is {why}; its routes are unknown.");
                            ctx.diag(pos, message);
                            continue;
                        }
                    }
                };
                let items = Parser { s: text, i: at }.array(0);
                let cond = rendered_conditionally(text, bare, pos).map(|m| (pos, m));
                roots.extend(
                    (items.iter().filter_map(|it| object(ctx, it, 0))).map(|mut n| {
                        n.cond = cond.clone();
                        n
                    }),
                );
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
            maps: Vec::new(),
            stray: false,
            container: None,
            cond: None,
        })
    }

    /// The components around a route element's screen component (outermost
    /// first), the screen component, and whether the element only redirects.
    #[derive(Default)]
    struct Chain<'e> {
        wrappers: Vec<&'e str>,
        screen: Option<&'e str>,
        redirect: bool,
    }

    /// The screen is the first self-closing component tag; every component
    /// element open around it is a wrapper. Tags inside attributes and plain
    /// DOM tags are skipped; a fragment is transparent.
    fn chain(element: &str) -> Chain<'_> {
        let mut events: Vec<(usize, Option<&str>)> = start_tags(element)
            .into_iter()
            .map(|(i, n)| (i, Some(n)))
            .chain(element.match_indices("<>").map(|(i, _)| (i, Some(""))))
            .chain(element.match_indices("</").map(|(i, _)| (i, None)))
            .collect();
        events.sort();
        let (mut stack, mut skip, mut first): (Vec<&str>, usize, Option<&str>) =
            (Vec::new(), 0, None);
        fn done<'e>(screen: &'e str, stack: &[&'e str]) -> Chain<'e> {
            Chain {
                wrappers: stack.iter().copied().filter(|w| !w.is_empty()).collect(),
                screen: Some(screen),
                redirect: screen == "Navigate",
            }
        }
        for (pos, name) in events {
            if pos < skip {
                continue;
            }
            let Some(name) = name else {
                stack.pop();
                continue;
            };
            if name.is_empty() {
                skip = pos + 2;
                stack.push("");
                continue;
            }
            let t = tag(element, pos + 1 + name.len());
            skip = t.end;
            let component =
                name.starts_with(|c: char| c.is_ascii_uppercase()) && !name.ends_with("Fragment");
            if !component {
                if !t.self_closing {
                    stack.push("");
                }
                continue;
            }
            first.get_or_insert(name);
            if t.self_closing {
                return done(name, &stack);
            }
            stack.push(name);
        }
        first.map_or_else(Chain::default, |f| done(f, &[]))
    }

    /// React's own components, transparent as wrappers.
    const REACT_BUILTINS: [&str; 4] = ["Fragment", "Suspense", "StrictMode", "Profiler"];

    /// What wrapper `name`, used in this file, requires of what it renders: its
    /// body read by S3 when it is a project component, nothing for React's
    /// own, else `Unknown(name)` and a gap.
    fn wrapper(ctx: &mut Ctx, at: usize, name: &str) -> Vec<Condition> {
        match read_wrapper(ctx, name) {
            Ok(conditions) => conditions,
            Err(why) => {
                let message = format!(
                    "Wrapper {name} is not read ({why}); the access of the route is unknown."
                );
                ctx.diag(at, message);
                vec![Condition::Unknown(name.into())]
            }
        }
    }

    fn read_wrapper(ctx: &mut Ctx, name: &str) -> Result<Vec<Condition>, String> {
        if name.ends_with("Fragment") {
            return Ok(Vec::new());
        }
        let (head, member) = name
            .split_once('.')
            .map_or((name, None), |(h, m)| (h, Some(m)));
        let all = imports(ctx.text);
        let import = all.iter().find(|i| i.local == head);
        let builtin = |m: &str| REACT_BUILTINS.contains(&m);
        match (import, member) {
            (Some(i), Some(m)) if i.module == "react" && builtin(m) => Ok(Vec::new()),
            (_, Some(_)) => Err("a member of an object".into()),
            (Some(i), None) if i.module == "react" => match i.imported.as_deref() {
                Some(n) if builtin(n) => Ok(Vec::new()),
                _ => Err("imported from react".into()),
            },
            (Some(i), None) => {
                let Some(file) = ctx.app.resolve(ctx.file, &i.module) else {
                    return Err(format!(
                        "imported from {}, which is no scanned file",
                        i.module
                    ));
                };
                let text = &ctx.app.files[file];
                super::guard::component(ctx.access, ctx.out, (file, text), i.imported.as_deref())
            }
            (None, None) if declares(ctx.text, name) => {
                super::guard::component(ctx.access, ctx.out, (ctx.file, ctx.text), Some(name))
            }
            (None, None) => Err("neither imported nor declared in this file".into()),
        }
    }

    /// The guards around the route's element, its screen component and, when
    /// the route has children, what that component requires of them (a
    /// layout); an element that is no component is a diagnostic.
    fn component(ctx: &mut Ctx, node: &Node) -> (Vec<Condition>, Option<String>, Vec<Condition>) {
        let parts = node.element.as_deref().map(chain).unwrap_or_default();
        let mut guards = Vec::new();
        for w in &parts.wrappers {
            guards.extend(wrapper(ctx, node.at, w));
        }
        let redirect = parts.redirect;
        let element_component = parts.screen.filter(|_| !redirect).map(String::from);
        let component = node.component.clone().or(element_component);
        let layout = match &component {
            Some(name)
                if !redirect
                    && node.path.is_none()
                    && !node.index
                    && !(node.children.is_empty() && node.maps.is_empty()) =>
            {
                wrapper(ctx, node.at, name)
            }
            _ => Vec::new(),
        };
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
            && parts.wrappers.is_empty()
            && !redirect
        {
            let message =
                format!("Route element is not a JSX component; the screen is unknown: {element}");
            ctx.diag(node.at, message);
        }
        (guards, component, layout)
    }

    fn push_screen(ctx: &mut Ctx, node: &Node, full: &str, name: String, guards: &[Condition]) {
        let line = line_of(ctx.text, node.at);
        let mut screen = ctx
            .app
            .screen(format!("/{full}"), name.clone(), ctx.file, line);
        screen.access = match guards {
            [] => archguard_core::AccessState::Undeclared,
            [one] => archguard_core::AccessState::Protected(one.clone()),
            all => archguard_core::AccessState::Protected(Condition::All(all.to_vec())),
        };
        ctx.screens.push((screen, Owner::Component(name)));
    }

    /// Why a route is not read where it stands: in a conditional expression,
    /// or an absolute path outside its parent's (React Router v6 throws).
    fn misplaced(node: &Node, path: Option<&str>, prefix: &str) -> Option<String> {
        if node.stray {
            return Some("Route inside a conditional expression is not read; the route and its children are unknown.".into());
        }
        if let Some(e) = node.element.as_deref().filter(|e| !single_element(e)) {
            return Some(format!(
                "Route element is not one JSX element (a condition, call or name); the route and its children are unknown: {e}"
            ));
        }
        let p = path.filter(|p| p.starts_with('/') && !prefix.is_empty())?;
        let t = p.trim_matches('/');
        (t != prefix && !t.starts_with(&format!("{prefix}/"))).then(|| {
            format!("Absolute route path {p} does not start with its parent's path /{prefix}; the route and its children are unknown.")
        })
    }

    /// The children of `node` and the `.map` arrays rendered in it.
    fn descend(ctx: &mut Ctx, node: &Node, full: &str, guards: &[Condition], depth: usize) {
        for child in &node.children {
            walk(ctx, child, full, guards, depth + 1);
        }
        let maps = ctx.maps;
        for &i in &node.maps {
            map_routes(ctx, &maps[i], full, guards);
        }
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
        if let Some(message) = misplaced(node, path, prefix) {
            return ctx.diag(node.at, message);
        }
        let mut inherited = inherited.to_vec();
        if let Some(cond) = &node.container {
            // The route exists only where the condition holds; what it is, is not read.
            let message =
                format!("Route rendered under a condition ({cond}); the route is unknown.");
            ctx.diag(node.at, message);
            inherited.push(Condition::Unknown(cond.clone()));
        }
        let inherited = &inherited[..];
        let full = match path {
            Some(p) if p.starts_with('/') => p.trim_matches('/').to_string(),
            Some(p) => [prefix, p.trim_matches('/')]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("/"),
            None => prefix.to_string(),
        };
        let (mut guards_here, component, mut layout_guards) = component(ctx, node);
        let mut guards = inherited.to_vec();
        guards.append(&mut guards_here);
        let layout = node.children.iter().any(|c| c.index);
        if let Some(name) = component
            && (path.is_some() || node.index)
            && !layout
        {
            push_screen(ctx, node, &full, name, &guards);
        }
        // What the layout component requires holds for the routes it renders.
        guards.append(&mut layout_guards);
        descend(ctx, node, &full, &guards, depth);
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
                    weak: false,
                    from: from.clone(),
                    target,
                    trigger: nav.trigger.clone(),
                    file: file.into(),
                    line: nav.line,
                    condition: archguard_core::AccessState::Undeclared,
                    source: SOURCE.into(),
                });
            }
        }
    }
}

/// Pure text helpers of the deny-exit walker: branching tests, `pathname` tests on a route.
mod paths {
    use super::tanstack::simple_end;
    use super::*;

    pub(super) fn head_of(s: &str) -> String {
        s.lines()
            .next()
            .unwrap_or("")
            .trim()
            .chars()
            .take(80)
            .collect()
    }

    /// Whether `text` branches: `&&`, `||`, `??` or a `?` that is no `?.`.
    pub(super) fn has_branching(text: &str) -> bool {
        let b = text.as_bytes();
        text.contains("&&")
            || text.contains("||")
            || text.contains("??")
            || (0..b.len()).any(|i| b[i] == b'?' && b.get(i + 1) != Some(&b'.'))
    }

    /// Whether the children (or `<Outlet/>`) are rendered under a condition in
    /// the `return` expression `stmt`: a branching return value, or a `{…}`
    /// group that mentions them.
    pub(super) fn children_branching(stmt: &str) -> bool {
        let body = stmt.trim().strip_prefix("return").unwrap_or(stmt).trim();
        if !body.trim_start_matches('(').trim_start().starts_with('<') {
            return has_branching(body);
        }
        let b = body.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'{' {
                let e = block_end(body, i, b'{', b'}');
                let g = &body[i..e];
                if (!words(g, "children").is_empty() || !words(g, "Outlet").is_empty())
                    && has_branching(g)
                {
                    return true;
                }
                i = e;
            } else {
                i += 1;
            }
        }
        false
    }

    /// Words that make a statement a deny exit in a handler (S2).
    pub(super) const DENY_CALLS: [&str; 5] = [
        "redirect",
        "notFound",
        "unauthorized",
        "forbidden",
        "protect",
    ];

    pub(super) fn denies(stmt: &str) -> bool {
        DENY_CALLS.iter().any(|w| {
            words(stmt, w)
                .into_iter()
                .any(|p| stmt[p + w.len()..].trim_start().starts_with('('))
        }) || ["401", "403"].iter().any(|code| {
            words(stmt, code).into_iter().any(|p| {
                let before = stmt[..p].trim_end();
                before.ends_with("status:")
                    || before.ends_with("status(")
                    || before.ends_with("status")
            })
        })
    }

    /// The literal a `pathname` test compares with: a string, or a constant of the file.
    pub(super) fn literal_of(arg: &str) -> Option<String> {
        let a = arg.trim();
        let q = a.chars().next().filter(|c| matches!(c, '\'' | '"' | '`'))?;
        if a.len() >= 2 && a.ends_with(q) {
            return Some(a[1..a.len() - 1].to_string());
        }
        None
    }

    pub(super) fn constant<'m>(m: &'m str, name: &str) -> Option<&'m str> {
        words(m, name).into_iter().find_map(|p| {
            let before = m[..p].trim_end();
            matches!(
                trailing_ident(before).as_deref(),
                Some("const" | "let" | "var")
            )
            .then_some(())?;
            let b = m.as_bytes();
            let j = skip_ws(b, p + name.len());
            (b.get(j) == Some(&b'=') && b.get(j + 1) != Some(&b'=')).then(|| {
                let s = skip_ws(b, j + 1);
                m[s..simple_end(m, s)].trim().trim_end_matches(';')
            })
        })
    }

    /// Whether route pattern `route` starts with `lit` (a difference inside a
    /// `:param` or `*` segment is undecidable).
    pub(super) fn route_starts_with(route: &str, lit: &str) -> Tri {
        let (r, l) = (route.as_bytes(), lit.as_bytes());
        let mut seg = 0;
        for i in 0..l.len() {
            if i >= r.len() {
                return Tri::No;
            }
            if r[i] == b'/' {
                seg = i + 1;
            }
            if r[i] != l[i] {
                return match r.get(seg) {
                    Some(b':' | b'*') => Tri::Maybe,
                    _ => Tri::No,
                };
            }
        }
        Tri::Yes
    }

    /// Whether route pattern `route` is exactly `lit`.
    pub(super) fn route_equals(route: &str, lit: &str) -> Tri {
        let (rs, ls): (Vec<&str>, Vec<&str>) =
            (route.split('/').collect(), lit.split('/').collect());
        if rs.contains(&"*") {
            return Tri::Maybe;
        }
        if rs.len() != ls.len() {
            return Tri::No;
        }
        let mut maybe = false;
        for (r, l) in rs.iter().zip(&ls) {
            if r.starts_with(':') {
                maybe = true;
            } else if r != l {
                return Tri::No;
            }
        }
        if maybe { Tri::Maybe } else { Tri::Yes }
    }

    /// A `pathname` test (`pathname.startsWith('/x')`, `pathname === '/x'`, or a
    /// constant holding one) evaluated on `route`; `None` when `t` is no such test.
    pub(super) fn path_test(m: &str, t: &str, route: &str) -> Option<Tri> {
        let t = t.trim();
        let is_path = |x: &str| x == "pathname" || x.ends_with(".pathname");
        if let Some((x, rest)) = t.split_once(".startsWith(") {
            let arg = rest.strip_suffix(')')?;
            let lit = literal_of(arg).or_else(|| {
                let c = constant(m, arg.trim())?;
                literal_of(c)
            })?;
            return is_path(x.trim()).then(|| route_starts_with(route, &lit));
        }
        for (op, yes) in [("===", true), ("!==", false)] {
            if let Some((x, lit)) = t.split_once(op) {
                let lit = literal_of(lit)?;
                if !is_path(x.trim()) {
                    return None;
                }
                return Some(match (route_equals(route, &lit), yes) {
                    (Tri::Maybe, _) => Tri::Maybe,
                    (Tri::Yes, true) | (Tri::No, false) => Tri::Yes,
                    _ => Tri::No,
                });
            }
        }
        if is_name(t) && !t.contains('.') {
            return path_test(m, constant(m, t)?, route);
        }
        None
    }

    /// Splits `t` at the top-level `op`.
    pub(super) fn split_top<'t>(t: &'t str, op: &str) -> Vec<&'t str> {
        let (b, mut depth, mut from, mut out) = (t.as_bytes(), 0i32, 0, Vec::new());
        let mut i = 0;
        while i < b.len() {
            match b[i] {
                b'\'' | b'"' | b'`' => {
                    i = Parser { s: t, i }.string_end();
                    continue;
                }
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                _ if depth == 0 && t[i..].starts_with(op) => {
                    out.push(&t[from..i]);
                    from = i + op.len();
                    i += op.len();
                    continue;
                }
                _ => {}
            }
            i += 1;
        }
        out.push(&t[from..]);
        out
    }

    /// A control-flow statement (`switch`, `for`, …) that holds an exit.
    pub(super) fn nested_exit(stmt: &str, first: &str, handler: bool) -> bool {
        matches!(first, "switch" | "try" | "for" | "while" | "do")
            && (!words(stmt, "return").is_empty()
                || !words(stmt, "throw").is_empty()
                || (handler && denies(stmt)))
    }

    /// A bare statement that is a deny call: `redirect(…)`, `await auth.protect()`.
    pub(super) fn is_bare_deny(stmt: &str, first: &str) -> bool {
        let call = first == "await"
            || matches!(
                first,
                "redirect" | "permanentRedirect" | "notFound" | "unauthorized" | "forbidden"
            );
        call && denies(stmt) && !stmt.contains("=>") && !has_branching(stmt)
    }
    /// React's own hooks (<https://react.dev/reference/react/hooks>) that, called
    /// as a bare statement, say nothing about access. The effect hooks are not
    /// among them: their callback may navigate.
    const PLAIN_HOOKS: [&str; 11] = [
        "useState",
        "useReducer",
        "useContext",
        "useRef",
        "useImperativeHandle",
        "useMemo",
        "useCallback",
        "useId",
        "useDebugValue",
        "useSyncExternalStore",
        "useTransition",
    ];

    /// A statement of a wrapper body (no `return`/`throw`) that acts without
    /// its result being read: a bare call that is no plain React hook (a custom
    /// hook may redirect), an effect hook, or an imperative navigation.
    pub(super) fn wrapper_side_effect(stmt: &str) -> Result<(), String> {
        let first = ident_at(stmt, 0);
        let rest = stmt[first.len()..].trim_start_matches(|c: char| is_ident_char(c) || c == '.');
        let bare_call = !first.is_empty()
            && !matches!(first, "const" | "let" | "var" | "function" | "class" | "if")
            && rest.starts_with('(');
        let navigates = ["navigate", "redirect", "push", "replace"].iter().any(|w| {
            words(stmt, w)
                .into_iter()
                .any(|p| stmt[p + w.len()..].trim_start().starts_with('('))
        });
        if navigates || (bare_call && !PLAIN_HOOKS.contains(&first)) {
            return Err(format!(
                "it calls code that may redirect: {}",
                head_of(stmt)
            ));
        }
        Ok(())
    }
    /// The names a `const`/`let`/`var` statement binds when its initialiser is a
    /// call that is no plain React hook (`const user = useAuth()`).
    pub(super) fn declared_by_call(stmt: &str) -> Vec<String> {
        let kw = ident_at(stmt, 0);
        if !matches!(kw, "const" | "let" | "var") {
            return Vec::new();
        }
        let rest = &stmt[kw.len()..];
        let Some(eq) = rest
            .find('=')
            .filter(|&i| !rest[i + 1..].starts_with(['>', '=']))
        else {
            return Vec::new();
        };
        let init = rest[eq + 1..].trim();
        let init = init.strip_prefix("await ").unwrap_or(init).trim_start();
        let callee = init.trim_start_matches(|c: char| is_ident_char(c) || c == '.');
        let name_len = init.len() - callee.len();
        let last = init[..name_len].rsplit('.').next().unwrap_or("");
        if name_len == 0 || !callee.starts_with('(') || PLAIN_HOOKS.contains(&last) {
            return Vec::new();
        }
        let raw = rest[..eq].trim();
        let parts: Vec<&str> = match raw.as_bytes().first() {
            Some(&o @ (b'{' | b'[')) => {
                let close = block_end(raw, 0, o, if o == b'{' { b'}' } else { b']' });
                raw[1..close.saturating_sub(1).max(1)]
                    .split(',')
                    .map(|p| p.rsplit(':').next().unwrap_or(p))
                    .collect()
            }
            _ => vec![raw.split(':').next().unwrap_or(raw)],
        };
        parts
            .into_iter()
            .map(|p| p.trim().trim_start_matches("...").trim())
            .filter(|p| is_ident(p))
            .map(String::from)
            .collect()
    }
}

/// TanStack Router file routes (`createFileRoute('/a/$id')`) and `beforeLoad` guards.
mod tanstack {
    use super::paths::*;
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
    pub(super) fn check(text: &str, access: &Access) -> Result<Option<Condition>, String> {
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

    pub(super) fn negate(c: Option<Condition>) -> Option<Condition> {
        match c {
            None => Some(Condition::Never),
            Some(Condition::Never) => None,
            Some(Condition::Not(inner)) => Some(*inner),
            Some(c) => Some(Condition::Not(Box::new(c))),
        }
    }

    /// A readable function: the interior of its block body, or a function name.
    pub(super) enum Callable {
        Body(usize, usize),
        Reference(String),
        Unreadable,
    }

    /// The interior of the `{ … }` after offset `at` (a `: Type` may come first).
    pub(super) fn block_after(m: &str, at: usize) -> Callable {
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
    pub(super) fn callable_at(m: &str, at: usize) -> Callable {
        let b = m.as_bytes();
        let mut i = skip_ws(b, at);
        if ident_at(m, i) == "async" {
            i = skip_ws(b, i + "async".len());
        }
        let arrow = |after_params: usize| {
            let mut j = skip_ws(b, after_params);
            // shortcut: a return type annotation is skipped to the first `=>`, upgrade if a function type appears in it.
            if b.get(j) == Some(&b':')
                && let Some(k) = m[j..].find("=>")
            {
                j += k;
            }
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
    pub(super) fn declared_body(m: &str, name: &str) -> Option<(usize, usize)> {
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

    /// What a [`Walker`] reads: a TanStack `beforeLoad` (`throw redirect(…)` only),
    /// a wrapper component (every `return`/`throw` that does not render the
    /// children denies), or a Next handler (S2 deny exits).
    #[derive(Clone, Copy, PartialEq)]
    pub(super) enum Mode {
        Redirect,
        Wrapper,
        Handler,
    }

    /// The statements of one body, read with fixed rules: `if`/`else` chains,
    /// early exits. A deny exit anywhere else is not read.
    pub(super) struct Walker<'a> {
        pub(super) m: &'a str,
        pub(super) file: &'a str,
        pub(super) access: &'a Access,
        pub(super) out: &'a mut Vec<UiFlowDiagnostic>,
        /// `not (path conditions)` for each deny exit found.
        pub(super) found: Vec<Condition>,
        pub(super) mode: Mode,
        /// Handler mode: the route whose requirement is read; `pathname` tests
        /// are evaluated against it.
        pub(super) route: Option<&'a str>,
        /// Wrapper mode: a `return` renders the children.
        pub(super) allowed: bool,
        /// Handler mode: the parameter of the `catch` being read; `throw` of it re-throws.
        pub(super) catch_param: Option<String>,
        /// Wrapper mode: names bound by a call that is no plain hook.
        pub(super) declared: Vec<String>,
        /// Wrapper mode: the texts of the checks and exits read so far.
        pub(super) seen: String,
    }

    /// How control leaves a statement list.
    #[derive(Clone, Copy, PartialEq)]
    pub(super) enum Ended {
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
    pub(super) fn simple_end(m: &str, i: usize) -> usize {
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
            if self.mode != Mode::Redirect {
                self.seen.push_str(text);
                self.seen.push(' ');
                return self.cond_ext(text, at);
            }
            self.atom(text, at)
        }

        /// Wrapper and handler checks: `a && b`, `!a`, `pathname` tests on the
        /// route being read, then the same atoms as `beforeLoad`.
        fn cond_ext(&mut self, text: &str, at: usize) -> Option<Condition> {
            let mut t = text.trim();
            while t.starts_with('(') && block_end(t, 0, b'(', b')') == t.len() {
                t = t[1..t.len() - 1].trim();
            }
            let parts = split_top(t, "&&");
            if parts.len() > 1 {
                let cs: Vec<Option<Condition>> =
                    parts.iter().map(|p| self.cond_ext(p, at)).collect();
                if cs.contains(&Some(Condition::Never)) {
                    return Some(Condition::Never);
                }
                return all(cs.into_iter().flatten().collect());
            }
            if let Some(rest) = t.strip_prefix('!')
                && !rest.starts_with('=')
            {
                let inner = self.cond_ext(rest, at);
                return negate(inner);
            }
            if let Some(tri) = self.route.and_then(|r| path_test(self.m, t, r)) {
                return match tri {
                    Tri::Yes => None,
                    Tri::No => Some(Condition::Never),
                    Tri::Maybe => {
                        let message = format!(
                            "Path test {t} cannot be decided for a parameterised route; the condition is unknown."
                        );
                        diag(self.out, self.file, line_of(self.m, at), message);
                        Some(Condition::Unknown(t.to_string()))
                    }
                };
            }
            self.atom(t, at)
        }

        fn atom(&mut self, text: &str, at: usize) -> Option<Condition> {
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

        /// `try { A } catch (e) { B } finally { C }` in a handler: the blocks are
        /// walked under the same path; what `B` denies holds only after an exception,
        /// which is an unknown condition. `throw e` of the caught value is no deny.
        fn try_catch(
            &mut self,
            (i, end): (usize, usize),
            path: &[Option<Condition>],
        ) -> Result<Ended, String> {
            let (m, b) = (self.m, self.m.as_bytes());
            let block = |at: usize| -> Result<(usize, usize, usize), String> {
                let open = skip_ws(b, at);
                if b.get(open) != Some(&b'{') {
                    return Err(format!("try without a block: {}", head_of(&m[i..end])));
                }
                let close = block_end(m, open, b'{', b'}');
                Ok((open + 1, close - 1, close))
            };
            let (s, e, mut at) = block(i + "try".len())?;
            self.walk(s, e, &mut path.to_vec())?;
            at = skip_ws(b, at);
            if ident_at(m, at) == "catch" {
                let mut j = skip_ws(b, at + "catch".len());
                let mut param = None;
                if b.get(j) == Some(&b'(') {
                    let close = block_end(m, j, b'(', b')');
                    param = Some(ident_at(m, skip_ws(b, j + 1)).to_string());
                    j = close;
                }
                let (s, e, after) = block(j)?;
                let mut p = path.to_vec();
                p.push(Some(Condition::Unknown("an exception".into())));
                let (before, saved) = (self.found.len(), self.catch_param.take());
                self.catch_param = param;
                let walked = self.walk(s, e, &mut p);
                self.catch_param = saved;
                walked?;
                if self.found.len() > before {
                    let message = "A deny exit in a catch block holds only after an exception; the condition is unknown.";
                    diag(self.out, self.file, line_of(m, s), message.to_string());
                }
                at = skip_ws(b, after);
            }
            if ident_at(m, at) == "finally" {
                let (s, e, _) = block(at + "finally".len())?;
                self.walk(s, e, &mut path.to_vec())?;
            }
            Ok(Ended::No)
        }

        /// A statement of a wrapper or handler body that is no `if` or block.
        fn exit(
            &mut self,
            (i, end): (usize, usize),
            path: &[Option<Condition>],
        ) -> Result<Ended, String> {
            let stmt = self.m[i..end].trim();
            let first = ident_at(stmt, 0);
            if first == "try" && self.mode == Mode::Handler {
                return self.try_catch((i, end), path);
            }
            if nested_exit(stmt, first, self.mode == Mode::Handler) {
                return Err(format!("exit inside {first}: {}", head_of(stmt)));
            }
            match self.mode {
                Mode::Wrapper => self.exit_wrapper(stmt, first, path),
                _ => self.exit_handler(stmt, first, path),
            }
        }

        /// A wrapper's `return`/`throw`: it denies unless it renders the children.
        fn exit_wrapper(
            &mut self,
            stmt: &str,
            first: &str,
            path: &[Option<Condition>],
        ) -> Result<Ended, String> {
            if !matches!(first, "return" | "throw") {
                self.declared.extend(declared_by_call(stmt));
                return wrapper_side_effect(stmt).map(|()| Ended::No);
            }
            let renders = !words(stmt, "children").is_empty() || !words(stmt, "Outlet").is_empty();
            if !renders {
                self.seen.push_str(stmt);
                self.seen.push(' ');
                self.reached(path);
                return Ok(Ended::Redirect);
            }
            if children_branching(stmt) {
                let why = head_of(stmt);
                return Err(format!("children rendered conditionally: {why}"));
            }
            self.allowed = true;
            Ok(Ended::Stop)
        }

        /// A handler statement: S2 deny exits, a `throw` of the caught value excepted.
        fn exit_handler(
            &mut self,
            stmt: &str,
            first: &str,
            path: &[Option<Condition>],
        ) -> Result<Ended, String> {
            let thrown = stmt
                .strip_prefix("throw")
                .map(|t| t.trim().trim_end_matches(';').trim());
            if thrown.is_some() && thrown == self.catch_param.as_deref() {
                return Ok(Ended::Stop);
            }
            let deny = match first {
                "return" => denies(stmt),
                "throw" => true,
                _ => is_bare_deny(stmt, first),
            };
            if deny {
                self.reached(path);
                return Ok(Ended::Redirect);
            }
            match first {
                "return" => Ok(Ended::Stop),
                _ if denies(stmt) => Err(format!(
                    "deny exit in an unsupported form: {}",
                    head_of(stmt)
                )),
                _ => Ok(Ended::No),
            }
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
                _ if self.mode != Mode::Redirect => self.exit((i, end), path)?,
                _ if has_redirect(stmt) => self.throw_redirect(stmt, path)?,
                "throw" | "return" => Ended::Stop,
                _ => Ended::No,
            })
        }

        /// Walks the statements in `s..e` and tells how control leaves them.
        pub(super) fn walk(
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
            mode: Mode::Redirect,
            route: None,
            allowed: false,
            catch_param: None,
            declared: Vec::new(),
            seen: String::new(),
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

    pub(super) fn all(conditions: Vec<Condition>) -> Option<Condition> {
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
            screen.access = archguard_core::AccessState::from_parts(all(conditions), false);
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
                    weak: false,
                    screen: from.clone(),
                    trigger: None,
                    call: call.clone(),
                    file: file.into(),
                    line: *line,
                    condition: archguard_core::AccessState::Undeclared,
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
            .map(|s| (s.id.clone(), s.name.clone(), s.access.condition().cloned()))
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
                "const handler = (req, res) => res.json({});\nexport default withAuth(handler);\nfunction withAuth(h) { return (req, res) => { if (!user) return res.status(401).end(); return h(req, res); }; }\n",
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
                "export function middleware(request) { if (!s) return NextResponse.redirect(new URL('/login', request.url)); }
export const config = { matcher: ['/orders/:path*', '/api/items'] };\n",
            ),
        ]);
        let flow = extract(dir.path());
        assert_eq!(
            screens(&flow),
            vec![
                screen("web:/", "Home", None),
                screen("web:/orders/:id", "OrderPage", Some("s")),
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
        let mw = AccessState::Protected(Condition::Custom("s".into()));
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
import { RequireAuth, ProtectedRoute } from './guards';
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
                "spa/src/guards.tsx",
                "export function RequireAuth({ children }) {
  if (!authed) return <Navigate to=\"/login\" />;
  return children;
}
export function ProtectedRoute() {
  if (!admin) return <Navigate to=\"/\" />;
  return <Outlet />;
}
",
            ),
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
                screen("spa:/admin", "Admin", Some("admin")),
                screen("spa:/orders", "Orders", Some("authed")),
                screen("spa:/orders/:id", "OrderDetail", Some("authed")),
            ]
        );
        let orders = &flow.screens[2];
        assert_eq!((orders.file.as_str(), orders.line), ("spa/src/App.tsx", 8));
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
                "import { AuthGuard } from './AuthGuard';
export const router = createBrowserRouter([
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
                "dash/src/AuthGuard.tsx",
                "export function AuthGuard({ children }) { if (!ok) return <Navigate to=\"/\" />; return children; }\n",
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
                screen("dash:/settings", "Settings", Some("ok")),
                screen("dash:/settings/profile", "Profile", Some("ok")),
            ]
        );
        assert!(has_diag(&flow, "dash/src/router.tsx", 8));
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
            .map(|s| (s.id.clone(), s.name.clone(), s.access.condition().cloned()))
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
            archguard_core::AccessState::Protected(Condition::Custom("isLoggedIn".into()))
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
            others: BTreeSet::new(),
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
            .condition()
            .cloned();
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
function ProtectedRoute({ children }) { if (!ok) return <Navigate to=\"/\" />; return children; }
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
                screen("c:/weather", "Weather", Some("ok")),
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

    fn mapped_in(app: &str, routes: &str) -> UiFlow {
        let dir = project(&[
            ("c/package.json", ROUTER_PKG),
            ("c/src/App.jsx", app),
            ("c/src/routes.jsx", routes),
        ]);
        extract(dir.path())
    }

    #[test]
    fn mapped_routes_inside_a_route_take_its_guards() {
        let app = "import { secret } from './routes';
export const App = () => (
<Routes><Route element={<ProtectedRoute />}>
  {secret.map((r) => <Route key={r.path} {...r} />)}
  <Route path=\"/open\" element={<Open />} />
</Route></Routes>);
function ProtectedRoute() { if (!ok) return <Navigate to=\"/\" />; return <Outlet />; }
";
        let routes = "export const secret = [{ path: '/vault', element: <Vault /> }];\n";
        let flow = mapped_in(app, routes);
        assert_eq!(
            screens(&flow),
            vec![
                screen("c:/open", "Open", Some("ok")),
                screen("c:/vault", "Vault", Some("ok")),
            ]
        );
    }

    #[test]
    fn mapped_routes_inside_a_route_take_its_path_prefix() {
        let app = "import { adminRoutes } from './routes';
export const App = () => (
<Routes><Route path=\"/admin\" element={<Admin />}>
  {adminRoutes.map(r => <Route key={r.path} {...r} />)}
</Route></Routes>);
";
        let routes = "export const adminRoutes = [{ path: 'users', element: <Users /> }];\n";
        let flow = mapped_in(app, routes);
        assert_eq!(
            screens(&flow),
            vec![
                screen("c:/admin", "Admin", None),
                screen("c:/admin/users", "Users", None),
            ]
        );
    }

    #[test]
    fn a_catch_all_paths_pattern_does_not_make_packages_project_imports() {
        let page = "import axios from 'axios';
import { here } from 'util/here';
export function Page() { axios.get('/api/h'); here(); return null; }
";
        let dir = project(&[
            ("c/package.json", ROUTER_PKG),
            (
                "c/tsconfig.json",
                r#"{"compilerOptions":{"paths":{"*":["./src/*"]}}}"#,
            ),
            (
                "c/src/App.jsx",
                "export const App = () => <Routes><Route path=\"/p\" element={<Page />} /></Routes>;\n",
            ),
            ("c/src/Page.jsx", page),
            ("c/src/util/here.js", "export function here() {}\n"),
        ]);
        let flow = extract(dir.path());
        assert!(
            flow.diagnostics
                .iter()
                .all(|d| !d.message.contains("axios")),
            "{:?}",
            flow.diagnostics
        );
    }

    #[test]
    fn existing_non_source_files_and_module_scripts_are_not_missing() {
        let page = "import items from './items.json';
import { load } from './client.mjs';
export function Page() { items.map((i) => i); load(); return null; }
";
        let dir = project(&[
            ("c/package.json", ROUTER_PKG),
            (
                "c/src/App.jsx",
                "export const App = () => <Routes><Route path=\"/p\" element={<Page />} /></Routes>;\n",
            ),
            ("c/src/Page.jsx", page),
            ("c/src/items.json", "[]"),
            ("c/src/client.mjs", "export function load() {}\n"),
        ]);
        let flow = extract(dir.path());
        assert!(flow.diagnostics.is_empty(), "{:?}", flow.diagnostics);
    }

    /// `c/src/App.jsx` with the array `xs` on line 1 and `body` from line 2 on;
    /// the `.map` is on line 3.
    fn map_app(body: &str) -> UiFlow {
        let app = format!(
            "const xs = [{{ path: '/priv', element: <Priv /> }}];\nexport const App = () => (\n{body});\n"
        );
        let dir = project(&[("c/package.json", ROUTER_PKG), ("c/src/App.jsx", &app)]);
        extract(dir.path())
    }

    fn routes_map(callback: &str) -> UiFlow {
        map_app(&format!("<Routes>{{xs.map({callback})}}</Routes>"))
    }

    fn is_gap_without_screens(flow: &UiFlow) {
        assert!(flow.screens.is_empty(), "{:?}", screens(flow));
        assert!(has_diag(flow, "c/src/App.jsx", 3), "{:?}", flow.diagnostics);
        assert!(!flow.complete);
    }

    #[test]
    fn mapped_callbacks_with_conditions_are_gaps() {
        for callback in [
            "(r) => { if (r.private && !user) return null; return <Route key={r.path} {...r} />; }",
            "(r) => (r.private && !user) ? null : <Route {...r} />",
            "(r) => !r.private ? <Route {...r} /> : <Route {...r} element={<RequireAuth>{r.element}</RequireAuth>} />",
            "(r) => user && <Route {...r} />",
            "(r) => <Route {...r} element={<RequireAuth>{r.element}</RequireAuth>} />",
            "(r) => { log(r); return <Route {...r} />; }",
        ] {
            is_gap_without_screens(&routes_map(callback));
        }
    }

    #[test]
    fn mapped_callbacks_of_the_whitelisted_shapes_are_read() {
        for callback in [
            "(r) => <Route key={r.path} {...r} />",
            "r => (<Route {...r} element={r.element} />)",
            "({ element, ...rest }) => { return <Route {...rest} element={element} />; }",
            "(route, i) => { const { element, ...rest } = route; return <Route key={i} {...rest} element={element} />; }",
        ] {
            let flow = routes_map(callback);
            assert_eq!(
                screens(&flow),
                vec![screen("c:/priv", "Priv", None)],
                "{callback}: {:?}",
                flow.diagnostics
            );
            assert!(flow.diagnostics.is_empty(), "{callback}");
        }
    }

    #[test]
    fn mapped_routes_outside_routes_and_route_children_are_gaps() {
        let cb = "(r) => <Route key={r.path} {...r} />";
        for body in [
            format!(
                "<Routes><Route path=\"/e\" element={{<RequireAuth><Shell>{{xs.map({cb})}}</Shell></RequireAuth>}} /></Routes>"
            ),
            format!("<Routes><RequireAuth>{{xs.map({cb})}}</RequireAuth></Routes>"),
        ] {
            let flow = map_app(&format!("{}\n", body.replace("{xs", "\n{xs")));
            assert!(
                flow.screens.iter().all(|s| s.id != "c:/priv"),
                "{body}: {:?}",
                screens(&flow)
            );
            assert!(has_diag(&flow, "c/src/App.jsx", 4), "{body}");
        }
    }

    #[test]
    fn a_wrapper_element_between_a_route_and_its_mapped_routes_is_a_gap() {
        let flow = map_app(
            "<Routes><Route path=\"/l\" element={<ProtectedRoute />}><Layout>\n{xs.map((r) => <Route key={r.path} {...r} />)}\n</Layout></Route></Routes>",
        );
        assert!(
            flow.screens.iter().all(|s| s.id != "c:/priv"),
            "{:?}",
            screens(&flow)
        );
        assert!(
            has_diag(&flow, "c/src/App.jsx", 4),
            "{:?}",
            flow.diagnostics
        );
    }

    fn array_app(array: &str, callback: &str) -> UiFlow {
        let app = format!(
            "{array}\nexport const App = () => (\n<Routes>{{xs.map({callback})}}</Routes>);\n"
        );
        let dir = project(&[("c/package.json", ROUTER_PKG), ("c/src/App.jsx", &app)]);
        extract(dir.path())
    }

    #[test]
    fn a_destructured_name_other_than_element_is_a_gap() {
        let array = "const xs = [{ path: '/priv', element: <Priv />, guarded: <RequireAuth><Priv /></RequireAuth> }];";
        for callback in [
            "({ guarded, ...rest }) => <Route {...rest} element={guarded} />",
            "(r) => { const { guarded, ...rest } = r; return <Route {...rest} element={guarded} />; }",
        ] {
            is_gap_without_screens(&array_app(array, callback));
        }
    }

    #[test]
    fn a_map_is_read_only_as_the_whole_jsx_container() {
        let cb = "(r) => <Route key={r.path} {...r} />";
        for body in [
            format!("<Routes>{{user && xs.map({cb})}}</Routes>"),
            format!("<Routes>{{user ? xs.map({cb}) : null}}</Routes>"),
            format!("<Routes>{{xs.map({cb}).filter(() => user)}}</Routes>"),
            format!("<Routes>{{xs.map({cb})[0]}}</Routes>"),
        ] {
            let flow = map_app(&body);
            assert!(flow.screens.is_empty(), "{body}: {:?}", screens(&flow));
            assert!(has_diag(&flow, "c/src/App.jsx", 3), "{body}");
        }
        let flow = map_app(&format!(
            "<Routes>{{/* all */ xs.map({cb}) /* done */}}</Routes>"
        ));
        assert_eq!(
            screens(&flow),
            vec![screen("c:/priv", "Priv", None)],
            "{:?}",
            flow.diagnostics
        );
    }

    #[test]
    fn routes_inside_a_conditional_expression_exist_with_an_unknown_condition() {
        for body in [
            "<Routes>{user && <Route path='/plain' element={<Plain />} />}</Routes>",
            "<Routes>{user ? <Route path='/plain' element={<Plain />} /> : null}</Routes>",
            "<Routes>{user && (<Route path='/plain' element={<Plain />} />)}</Routes>",
            "<Routes>{user || <Route path='/plain' element={<Plain />} />}</Routes>",
        ] {
            let flow = map_app(body);
            assert_eq!(flow.screens.len(), 1, "{body}: {:?}", screens(&flow));
            assert!(unknown_state(&flow.screens[0].access), "{body}");
            assert!(has_diag(&flow, "c/src/App.jsx", 3), "{body}");
            assert!(!flow.complete);
        }
    }

    #[test]
    fn flat_map_routes_are_gaps() {
        let flow = map_app("<Routes>{xs.flatMap((r) => [<Route key={r.path} {...r} />])}</Routes>");
        is_gap_without_screens(&flow);
    }

    #[test]
    fn absolute_child_paths_must_start_with_the_parent_path() {
        let app = "export const App = () => (
<Routes><Route path=\"/l\" element={<L />}>
  <Route path=\"/other\" element={<Other />} />
  <Route path=\"/l/ok\" element={<Ok />} />
</Route></Routes>);
";
        let dir = project(&[("c/package.json", ROUTER_PKG), ("c/src/App.jsx", app)]);
        let flow = extract(dir.path());
        let ids: Vec<String> = screens(&flow).into_iter().map(|s| s.0).collect();
        assert_eq!(ids, vec!["c:/l", "c:/l/ok"], "{:?}", flow.diagnostics);
        assert!(
            has_diag(&flow, "c/src/App.jsx", 3),
            "{:?}",
            flow.diagnostics
        );
    }

    #[test]
    fn typed_and_function_callbacks_are_read_when_equivalent() {
        for callback in [
            "(r: RouteObject) => <Route key={r.path} {...r} />",
            "({ element, ...rest }: RouteObject, i: number) => <Route {...rest} element={element} />",
            "function (r) { return <Route key={r.path} {...r} />; }",
            "function named(r: RouteObject) { const { element, ...rest } = r; return <Route {...rest} element={element} />; }",
        ] {
            let flow = routes_map(callback);
            assert_eq!(
                screens(&flow),
                vec![screen("c:/priv", "Priv", None)],
                "{callback}: {:?}",
                flow.diagnostics
            );
        }
        for callback in [
            "function (r) { if (!user) return null; return <Route {...r} />; }",
            "function (r) { return user && <Route {...r} />; }",
            "(r: RouteObject): JSX.Element => <Route {...r} />",
        ] {
            is_gap_without_screens(&routes_map(callback));
        }
    }

    #[test]
    fn a_conditional_element_makes_the_route_unknown_for_every_route_source() {
        let array = "const xs = [{ path: '/x', element: window.user ? <Admin /> : <Login /> }];";
        for array in [
            array,
            "const xs = [{ path: '/x', element: user && <Admin /> }];",
            "const xs = [{ path: '/x', element: render() }];",
        ] {
            let flow = array_app(array, "(r) => <Route key={r.path} {...r} />");
            assert!(flow.screens.is_empty(), "{array}: {:?}", screens(&flow));
            assert!(has_diag(&flow, "c/src/App.jsx", 1), "{array}");
        }
        for body in [
            "<Routes><Route path=\"/x\" element={user ? <A /> : <B />} /></Routes>",
            "<Routes><Route path=\"/x\" element={user || <B />} /></Routes>",
        ] {
            let flow = map_app(body);
            assert!(flow.screens.is_empty(), "{body}: {:?}", screens(&flow));
            assert!(has_diag(&flow, "c/src/App.jsx", 3), "{body}");
        }
        let app = "export const router = createBrowserRouter([\n{ path: '/x', element: cond ? <A /> : <B /> },\n]);\n";
        let dir = project(&[("c/package.json", ROUTER_PKG), ("c/src/App.jsx", app)]);
        let flow = extract(dir.path());
        assert!(flow.screens.is_empty(), "{:?}", screens(&flow));
        assert!(
            has_diag(&flow, "c/src/App.jsx", 2),
            "{:?}",
            flow.diagnostics
        );
        // A guard around the screen stays one element tree.
        let flow = map_app(
            "<Routes><Route path=\"/g\" element={<RequireAuth><G a={x ? 1 : 2} /></RequireAuth>} /></Routes>",
        );
        assert_eq!(flow.screens.len(), 1, "{:?}", flow.diagnostics);
        assert_eq!(flow.screens[0].name, "G");
        assert!(unknown_state(&flow.screens[0].access));
    }

    #[test]
    fn routes_in_any_container_other_than_a_whole_map_are_gaps() {
        let route = "<Route path=\"/admin\" element={<Admin />} />";
        for body in [
            format!("<Routes>{{user && <>{route}</>}}</Routes>"),
            format!("<Routes>{{user && <React.Fragment>{route}</React.Fragment>}}</Routes>"),
            format!(
                "<Routes>{{user && <>{route}<Route path=\"/b\" element={{<B />}} /></>}}</Routes>"
            ),
            "<Routes>{user && [<Route key=\"a\" path=\"/admin\" element={<Admin />} />]}</Routes>"
                .to_string(),
            format!(
                "<Routes>{{user ? <>{route}</> : <Route path=\"/login\" element={{<Login />}} />}}</Routes>"
            ),
            format!(
                "<Routes><Route path=\"/p\" element={{<P />}}>{{user && <>{route}</>}}</Route></Routes>"
            ),
        ] {
            let flow = map_app(&body);
            assert!(
                flow.screens.iter().all(|s| s.id != "c:/login"
                    && (s.id != "c:/admin" || unknown_state(&s.access))),
                "{body}: {:?}",
                screens(&flow)
            );
            assert!(has_diag(&flow, "c/src/App.jsx", 3), "{body}");
            assert!(!flow.complete, "{body}");
        }
        let flow = map_app(&format!("<Routes><>{route}</></Routes>"));
        assert_eq!(
            screens(&flow),
            vec![screen("c:/admin", "Admin", None)],
            "{:?}",
            flow.diagnostics
        );
        assert!(flow.diagnostics.is_empty());
        let flow = map_app("<Routes><>{xs.map((r) => <Route key={r.path} {...r} />)}</></Routes>");
        assert_eq!(
            screens(&flow),
            vec![screen("c:/priv", "Priv", None)],
            "{:?}",
            flow.diagnostics
        );
        let flow = map_app(
            "<Routes>{user && <>{xs.map((r) => <Route key={r.path} {...r} />)}</>}</Routes>",
        );
        is_gap_without_screens(&flow);
    }

    #[test]
    fn a_fragment_close_does_not_hide_a_wrapper_element() {
        let flow = map_app(
            "<Routes><RequireAuth><></>{xs.map((r) => <Route key={r.path} {...r} />)}</RequireAuth></Routes>",
        );
        is_gap_without_screens(&flow);
    }

    #[test]
    fn a_named_map_callback_is_a_gap() {
        let flow = map_app("<Routes>{xs.map(renderRoute)}</Routes>");
        is_gap_without_screens(&flow);
        let flow =
            map_app("<Routes>{xs.map(function renderRoute(r) { return r.route; })}</Routes>");
        is_gap_without_screens(&flow);
    }

    /// `decl` on line 1, `extra` on line 2, the `.map` on line 4.
    fn array_file(decl: &str, extra: &str) -> UiFlow {
        let app = format!(
            "{decl}\n{extra}\nexport const App = () => (\n<Routes>{{xs.map((r) => <Route key={{r.path}} {{...r}} />)}}</Routes>);\n"
        );
        let dir = project(&[("c/package.json", ROUTER_PKG), ("c/src/App.jsx", &app)]);
        extract(dir.path())
    }

    #[test]
    fn route_arrays_are_const_declared_once_and_never_changed() {
        let decl = "const xs = [{ path: '/priv', element: <Priv /> }];";
        for (decl, extra) in [
            (
                "let xs = [{ path: '/priv', element: <Priv /> }];",
                "if (!user) xs = [];",
            ),
            (decl, "if (!user) xs.splice(0);"),
            (decl, "xs.length = 0;"),
            (decl, "xs[0] = other;"),
            (decl, "xs.push(more);"),
            (decl, "xs ||= [];"),
            (decl, "function f() { const xs = []; return xs; }"),
            (decl, "const g = (xs) => xs.length;"),
            (decl, "const g = (a, xs) => a;"),
            (decl, "function h(xs) { return xs; }"),
            ("var xs = [{ path: '/priv', element: <Priv /> }];", ""),
        ] {
            let flow = array_file(decl, extra);
            assert!(
                flow.screens.is_empty(),
                "{decl} {extra}: {:?}",
                screens(&flow)
            );
            assert!(
                has_diag(&flow, "c/src/App.jsx", 4),
                "{decl} {extra}: {:?}",
                flow.diagnostics
            );
            assert!(!flow.complete);
        }
        for extra in [
            "const n = xs.length;",
            "const f = xs.filter((x) => x);",
            "log(xs);",
            "xs.forEach(log);",
        ] {
            let flow = array_file(decl, extra);
            assert_eq!(
                screens(&flow),
                vec![screen("c:/priv", "Priv", None)],
                "{extra}: {:?}",
                flow.diagnostics
            );
        }
        let flow = array_file("const xs = cond ? a : b;", "");
        let message = &flow.diagnostics[0].message;
        assert!(
            message.contains("array literal") && !message.contains("not declared"),
            "{message}"
        );
    }

    #[test]
    fn route_arrays_for_a_router_call_follow_the_same_rule() {
        let call = "export const router = createBrowserRouter(routes);\n";
        for (decl, ok) in [
            ("const routes = [{ path: '/x', element: <X /> }];", true),
            ("let routes = [{ path: '/x', element: <X /> }];", false),
            (
                "const routes = [{ path: '/x', element: <X /> }];\nroutes.pop();",
                false,
            ),
        ] {
            let dir = project(&[
                ("c/package.json", ROUTER_PKG),
                ("c/src/App.jsx", &format!("{decl}\n{call}")),
            ]);
            let flow = extract(dir.path());
            assert_eq!(
                !flow.screens.is_empty(),
                ok,
                "{decl}: {:?}",
                flow.diagnostics
            );
            assert_eq!(
                flow.diagnostics.is_empty(),
                ok,
                "{decl}: {:?}",
                flow.diagnostics
            );
        }
    }

    #[test]
    fn an_imported_route_array_that_is_changed_where_it_is_exported_is_a_gap() {
        let app = "import { xs } from './routes';\nexport const App = () => (\n<Routes>{xs.map((r) => <Route key={r.path} {...r} />)}</Routes>);\n";
        let dir = project(&[
            ("c/package.json", ROUTER_PKG),
            ("c/src/App.jsx", app),
            (
                "c/src/routes.jsx",
                "export const xs = [{ path: '/x', element: <X /> }];\nxs.shift();\n",
            ),
        ]);
        let flow = extract(dir.path());
        assert!(flow.screens.is_empty(), "{:?}", screens(&flow));
        assert!(
            has_diag(&flow, "c/src/App.jsx", 3),
            "{:?}",
            flow.diagnostics
        );
    }

    fn one_file(text: &str) -> UiFlow {
        let dir = project(&[("c/package.json", ROUTER_PKG), ("c/src/App.jsx", text)]);
        extract(dir.path())
    }

    const PRIV: &str = "const xs = [{ path: '/priv', element: <Priv /> }];\n";
    const MAPPED_XS: &str = "<Routes>{xs.map((r) => <Route key={r.path} {...r} />)}</Routes>";

    fn gap_and_no_screens(flow: &UiFlow, what: &str) {
        assert!(flow.screens.is_empty(), "{what}: {:?}", screens(flow));
        assert!(!flow.diagnostics.is_empty(), "{what}");
        assert!(!flow.complete, "{what}");
    }

    #[test]
    fn a_parameter_that_binds_the_array_name_hides_the_array() {
        for head in [
            "export function App({ xs }) { return",
            "export const App = ({ xs }) =>",
            "function App({ a: xs }) { return",
            "function App([xs]) { return",
            "function App(a, ...xs) { return",
            "const App = (xs = []) =>",
            "function App({ xs = [] }) { return",
            "const App = xs =>",
            "function App(a, { b: [xs] }) { return",
            "export default function ({ xs }: Props) { return",
        ] {
            let flow = one_file(&format!("{PRIV}{head} ({MAPPED_XS});\n"));
            gap_and_no_screens(&flow, head);
        }
        let flow = one_file(&format!(
            "{PRIV}function Other({{ ys }}) {{ return ys; }}\nexport function App() {{ const f = (a) => a; return ({MAPPED_XS}); }}\n"
        ));
        assert_eq!(
            screens(&flow),
            vec![screen("c:/priv", "Priv", None)],
            "{:?}",
            flow.diagnostics
        );
    }

    #[test]
    fn route_elements_rendered_through_a_name_or_call_are_gaps() {
        let admin = "<Route path=\"/admin\" element={<Admin />} />";
        for text in [
            format!(
                "const rs = user ? {admin} : null;\nexport const App = () => <Routes>{{rs}}</Routes>;\n"
            ),
            format!(
                "const rs = user ? {admin} : null;\nexport const App = () => <Routes><Route path=\"/p\" element={{<P />}}>{{rs}}</Route></Routes>;\n"
            ),
            format!(
                "const rs = user ? {admin} : null;\nexport const App = () => <Routes><Route path=\"/a\" element={{<A />}} />{{rs}}</Routes>;\n"
            ),
            format!(
                "const adminRoutes = () => user ? {admin} : null;\nexport const App = () => <Routes>{{adminRoutes()}}</Routes>;\n"
            ),
            format!("const rs = [{admin}];\nexport const App = () => <Routes>{{rs}}</Routes>;\n"),
            format!("const rs = {admin};\nexport const App = () => <Routes>{{rs}}</Routes>;\n"),
        ] {
            let flow = one_file(&text);
            assert!(
                flow.screens.iter().all(|s| s.id != "c:/admin"),
                "{text}: {:?}",
                screens(&flow)
            );
            assert!(!flow.diagnostics.is_empty(), "{text}");
            assert!(!flow.complete, "{text}");
        }
        let flow = one_file(
            "export const router = createBrowserRouter(createRoutesFromElements(<Route path=\"/r\" element={<Root />}><Route path=\"k\" element={<K />} /></Route>));\n",
        );
        let ids: Vec<String> = screens(&flow).into_iter().map(|s| s.0).collect();
        assert_eq!(ids, vec!["c:/r", "c:/r/k"], "{:?}", flow.diagnostics);
        assert!(flow.diagnostics.is_empty());
    }

    #[test]
    fn routes_rendered_under_a_condition_are_never_open() {
        let routes = "<Routes><Route path=\"/a\" element={<A />} /></Routes>";
        for text in [
            format!("export const App = () => (<div>{{user && {routes}}}</div>);\n"),
            format!("export const App = () => (<div>{{user ? {routes} : null}}</div>);\n"),
            format!("export const App = () => user ? {routes} : <Login />;\n"),
            format!("export function App() {{ if (!user) return <Login />; return {routes}; }}\n"),
            format!(
                "export function App() {{ if (!user) {{ return <Login />; }} return ({routes}); }}\n"
            ),
            format!("export function App() {{ return user && {routes}; }}\n"),
        ] {
            let flow = one_file(&text);
            let access: Vec<_> = flow
                .screens
                .iter()
                .map(|s| s.access.condition().cloned())
                .collect();
            assert_eq!(flow.screens.len(), 1, "{text}: {:?}", flow.diagnostics);
            assert!(unknown(&access[0]), "{text}: {access:?}");
            assert!(!flow.diagnostics.is_empty(), "{text}");
            assert!(!flow.complete, "{text}");
        }
        let flow = one_file(&format!(
            "export function App() {{ const f = useCallback(() => {{ return 1; }}, []); useEffect(() => {{ return () => {{}}; }}); return <div><Layout />{routes}</div>; }}\n"
        ));
        assert_eq!(
            screens(&flow),
            vec![screen("c:/a", "A", None)],
            "{:?}",
            flow.diagnostics
        );
        assert!(flow.diagnostics.is_empty());
    }

    #[test]
    fn changing_a_route_object_through_the_array_is_a_gap() {
        let decl = "const xs = [{ path: '/priv', element: <Priv /> }];";
        for extra in [
            "xs[0].element = <Login />;",
            "xs[0].path = '/x';",
            "xs[0].children.push(y);",
            "xs[0].children[0].element = <Login />;",
        ] {
            let flow = array_file(decl, extra);
            assert!(flow.screens.is_empty(), "{extra}: {:?}", screens(&flow));
            assert!(
                has_diag(&flow, "c/src/App.jsx", 4),
                "{extra}: {:?}",
                flow.diagnostics
            );
        }
        let flow = array_file(
            decl,
            "const e = xs[0].element; const n = xs[0].path === '/x';",
        );
        assert_eq!(
            screens(&flow),
            vec![screen("c:/priv", "Priv", None)],
            "{:?}",
            flow.diagnostics
        );
    }

    // ---- wave 2d: wrappers by body, Next middleware by body ----

    fn rr_flow(files: &[(&str, &str)]) -> UiFlow {
        let mut all = vec![("c/package.json", ROUTER_PKG)];
        all.extend_from_slice(files);
        extract(project(&all).path())
    }

    fn state(flow: &UiFlow, id: &str) -> AccessState {
        flow.screens
            .iter()
            .find(|s| s.id == id)
            .unwrap_or_else(|| panic!("no screen {id}: {:?}", screens(flow)))
            .access
            .clone()
    }

    fn protected(name: &str) -> AccessState {
        AccessState::Protected(custom(name))
    }

    fn unknown_state(s: &AccessState) -> bool {
        unknown(&s.condition().cloned())
    }

    const GATE: &str = "export function Gate({ children }) {
  if (!ok) return <Navigate to=\"/login\" />;
  return children;
}
";

    fn gated(app: &str, gate: &str) -> UiFlow {
        rr_flow(&[("c/src/App.jsx", app), ("c/src/Gate.jsx", gate)])
    }

    const GATE_APP: &str = "import { Routes, Route } from 'react-router-dom';
import { Gate } from './Gate';
export const App = () => <Routes><Route path=\"/a\" element={<Gate><A /></Gate>} /></Routes>;
";

    #[test]
    fn a_wrapper_with_a_deny_exit_protects_and_the_screen_is_the_inner_component() {
        let flow = gated(GATE_APP, GATE);
        assert_eq!(
            screens(&flow),
            vec![screen("c:/a", "A", Some("ok"))],
            "{:?}",
            flow.diagnostics
        );
    }

    #[test]
    fn a_transparent_wrapper_or_a_wrapper_named_like_a_guard_protects_nothing() {
        let flow = gated(
            GATE_APP,
            "export const Gate = ({ children }) => <div>{children}</div>;\n",
        );
        assert_eq!(screens(&flow), vec![screen("c:/a", "A", None)]);
        assert!(flow.complete, "{:?}", flow.diagnostics);
        let app = "import { Routes, Route } from 'react-router-dom';
import ProtectedRoute from './ProtectedRoute';
export const App = () => <Routes><Route path=\"/a\" element={<ProtectedRoute><A /></ProtectedRoute>} /></Routes>;
";
        let flow = rr_flow(&[
            ("c/src/App.jsx", app),
            (
                "c/src/ProtectedRoute.jsx",
                "export default function ProtectedRoute({ children }) { return children; }\n",
            ),
        ]);
        assert_eq!(screens(&flow), vec![screen("c:/a", "A", None)]);
        assert!(flow.complete, "{:?}", flow.diagnostics);
    }

    #[test]
    fn a_wrapper_from_a_package_is_unknown_and_a_gap() {
        let app = GATE_APP.replace("'./Gate'", "'some-pkg'");
        let flow = rr_flow(&[("c/src/App.jsx", &app)]);
        assert_eq!(
            state(&flow, "c:/a"),
            AccessState::Protected(Condition::Unknown("Gate".into()))
        );
        assert!(!flow.complete);
    }

    #[test]
    fn react_builtins_are_transparent_and_an_unreadable_wrapper_is_unknown() {
        let app = "import { Routes, Route } from 'react-router-dom';
import { Suspense } from 'react';
export const App = () => <Routes><Route path=\"/a\" element={<Suspense><A /></Suspense>} /></Routes>;
";
        let flow = rr_flow(&[("c/src/App.jsx", app)]);
        assert_eq!(screens(&flow), vec![screen("c:/a", "A", None)]);
        assert!(flow.complete, "{:?}", flow.diagnostics);
        // A ternary between children and something else is not read.
        let flow = gated(
            GATE_APP,
            "export const Gate = ({ children }) => ok ? children : <Login />;\n",
        );
        assert!(
            unknown_state(&state(&flow, "c:/a")),
            "{:?}",
            flow.diagnostics
        );
        assert!(!flow.complete);
        // A wrapper that never renders its children is not transparent.
        let flow = gated(GATE_APP, "export function Gate() { return <Login />; }\n");
        assert!(unknown_state(&state(&flow, "c:/a")));
        // Declared nowhere.
        let app = GATE_APP.replace("import { Gate } from './Gate';\n", "");
        let flow = rr_flow(&[("c/src/App.jsx", &app)]);
        assert!(unknown_state(&state(&flow, "c:/a")));
    }

    const PATHLESS: &str = "import { Routes, Route } from 'react-router-dom';
import { Gate } from './Gate';
export const App = () => (
  <Routes>
    <Route element={<Gate />}>
      <Route path=\"a\" element={<A />} />
      <Route path=\"b\" element={<B />} />
    </Route>
    <Route path=\"c\" element={<C />} />
  </Routes>
);
";

    #[test]
    fn a_pathless_layout_route_protects_its_children_by_its_body() {
        let gate = "export function Gate() {
  if (!ok) return <Navigate to=\"/login\" />;
  return <Outlet />;
}
";
        let flow = gated(PATHLESS, gate);
        assert_eq!(
            screens(&flow),
            vec![
                screen("c:/a", "A", Some("ok")),
                screen("c:/b", "B", Some("ok")),
                screen("c:/c", "C", None),
            ],
            "{:?}",
            flow.diagnostics
        );
        let flow = gated(PATHLESS, "export const Gate = () => <Outlet />;\n");
        assert_eq!(
            screens(&flow),
            vec![
                screen("c:/a", "A", None),
                screen("c:/b", "B", None),
                screen("c:/c", "C", None),
            ]
        );
    }

    #[test]
    fn a_guard_wrapped_in_a_fragment_is_still_read() {
        let app = GATE_APP.replace("<Gate><A /></Gate>", "<><Gate><A /></Gate></>");
        let flow = gated(&app, GATE);
        assert_eq!(
            screens(&flow),
            vec![screen("c:/a", "A", Some("ok"))],
            "{:?}",
            flow.diagnostics
        );
    }

    #[test]
    fn a_route_in_a_conditional_expression_exists_with_an_unknown_condition() {
        let app = "import { Routes, Route } from 'react-router-dom';
export const App = () => <Routes>{flag && <Route path=\"/x\" element={<X />} />}</Routes>;
";
        let flow = rr_flow(&[("c/src/App.jsx", app)]);
        let s = state(&flow, "c:/x");
        assert!(unknown_state(&s), "{s:?}");
        assert!(!flow.complete);
    }

    #[test]
    fn tsconfig_app_and_jsconfig_aliases_resolve_wrapper_imports() {
        let app = GATE_APP.replace("'./Gate'", "'@/Gate'");
        for config in ["c/jsconfig.json", "c/tsconfig.app.json"] {
            let flow = rr_flow(&[
                ("c/src/App.jsx", &app),
                ("c/src/Gate.jsx", GATE),
                (config, r#"{"compilerOptions":{"paths":{"@/*":["src/*"]}}}"#),
            ]);
            assert_eq!(
                screens(&flow),
                vec![screen("c:/a", "A", Some("ok"))],
                "{config}: {:?}",
                flow.diagnostics
            );
            assert!(flow.complete, "{config}: {:?}", flow.diagnostics);
        }
    }

    // ---- Next middleware / proxy ----

    const DENY: &str = "import { NextResponse } from 'next/server';
export function middleware(request) {
  if (!s) return NextResponse.redirect(new URL('/login', request.url));
  return NextResponse.next();
}
";

    fn next_flow(mw_name: &str, mw: &str, extra: &[(&str, &str)]) -> UiFlow {
        let page = |n: &str| format!("export default function {n}() {{ return null; }}\n");
        let pages = [
            ("web/app/page.tsx", "Home"),
            ("web/app/about/page.tsx", "About"),
            ("web/app/about/team/page.tsx", "Team"),
            ("web/app/blog/about/page.tsx", "BlogAbout"),
            ("web/app/dashboard/page.tsx", "Dash"),
            ("web/app/dashboard/a/b/page.tsx", "Deep"),
            ("web/app/dashboardx/page.tsx", "Dx"),
            ("web/app/x/page.tsx", "X"),
            ("web/app/x/a/page.tsx", "Xa"),
            ("web/app/x/a/b/page.tsx", "Xab"),
            ("web/app/apiary/page.tsx", "Apiary"),
        ];
        let texts: Vec<String> = pages.iter().map(|(_, n)| page(n)).collect();
        let mut all: Vec<(&str, &str)> = vec![
            ("web/package.json", NEXT_PKG),
            (
                "web/app/api/user/route.ts",
                "export async function GET() { return Response.json({}); }\n",
            ),
        ];
        all.extend(pages.iter().zip(&texts).map(|((p, _), t)| (*p, t.as_str())));
        all.push((mw_name, mw));
        all.extend_from_slice(extra);
        extract(project(&all).path())
    }

    /// Routes that are `Protected(custom(s))`; endpoints as `GET /path`.
    fn guarded(flow: &UiFlow) -> Vec<String> {
        let mut got: Vec<String> = flow
            .screens
            .iter()
            .filter(|s| s.access == protected("s"))
            .map(|s| s.route.clone().unwrap())
            .chain(
                flow.endpoints
                    .iter()
                    .filter(|e| e.access == protected("s"))
                    .map(|e| format!("GET {}", e.path)),
            )
            .collect();
        got.sort();
        got
    }

    fn with_config(config: &str) -> String {
        format!("{DENY}export const config = {{ matcher: {config} }};\n")
    }

    #[test]
    fn no_matcher_covers_every_page_and_handler() {
        let flow = next_flow("web/middleware.ts", DENY, &[]);
        let all = guarded(&flow);
        assert_eq!(all.len(), 12, "{all:?}");
        assert!(all.contains(&"GET /api/user".to_string()));
        assert!(flow.complete, "{:?}", flow.diagnostics);
    }

    #[test]
    fn a_bare_literal_matcher_is_unknown_for_descendants() {
        let flow = next_flow("web/middleware.ts", &with_config("'/about'"), &[]);
        assert_eq!(guarded(&flow), vec!["/about"]);
        assert!(unknown_state(&state(&flow, "web:/about/team")));
        assert_eq!(state(&flow, "web:/blog/about"), AccessState::Undeclared);
        assert!(!flow.complete);
    }

    #[test]
    fn path_star_plus_and_optional_matchers() {
        let flow = next_flow(
            "web/middleware.ts",
            &with_config("['/dashboard/:path*']"),
            &[],
        );
        assert_eq!(guarded(&flow), vec!["/dashboard", "/dashboard/a/b"]);
        let flow = next_flow("web/middleware.ts", &with_config("'/x/:p+'"), &[]);
        assert_eq!(guarded(&flow), vec!["/x/a", "/x/a/b"]);
        let flow = next_flow("web/middleware.ts", &with_config("'/x/:p?'"), &[]);
        assert_eq!(guarded(&flow), vec!["/x", "/x/a"]);
        assert_eq!(state(&flow, "web:/x/a/b"), AccessState::Undeclared);
    }

    #[test]
    fn the_negative_lookahead_matcher_excludes_by_prefix() {
        let m = "'/((?!api|_next/static|_next/image|favicon.ico).*)'";
        let flow = next_flow("web/middleware.ts", &with_config(m), &[]);
        let got = guarded(&flow);
        assert!(got.contains(&"/".to_string()) && got.contains(&"/dashboard".to_string()));
        assert!(!got.contains(&"GET /api/user".to_string()), "{got:?}");
        assert!(!got.contains(&"/apiary".to_string()), "{got:?}");
        assert!(flow.complete, "{:?}", flow.diagnostics);
    }

    #[test]
    fn has_and_missing_make_the_matched_routes_unknown() {
        let m = "[{ source: '/api/:path*', missing: [{ type: 'header', key: 'x-internal' }] }]";
        let flow = next_flow("web/middleware.ts", &with_config(m), &[]);
        let e = (flow.endpoints.iter()).find(|e| e.path == "/api/user");
        let e = e.unwrap();
        assert!(
            matches!(&e.access, AccessState::Protected(Condition::Unknown(_))),
            "{:?}",
            e.access
        );
        assert!(guarded(&flow).is_empty());
        assert!(!flow.complete);
    }

    #[test]
    fn a_non_constant_matcher_covers_everything_and_is_a_gap() {
        let mw = format!("const m = ['/a'];\n{DENY}export const config = {{ matcher: m }};\n");
        let flow = next_flow("web/middleware.ts", &mw, &[]);
        assert_eq!(guarded(&flow).len(), 12);
        assert!(!flow.complete);
    }

    #[test]
    fn proxy_ts_is_read_like_middleware_and_both_files_are_a_gap() {
        let proxy = DENY.replace("function middleware", "function proxy");
        let flow = next_flow("web/proxy.ts", &proxy, &[]);
        assert_eq!(guarded(&flow).len(), 12);
        assert!(flow.complete, "{:?}", flow.diagnostics);
        let flow = next_flow("web/src/proxy.ts", &proxy, &[]);
        assert_eq!(guarded(&flow).len(), 12);
        let flow = next_flow("web/proxy.ts", &proxy, &[("web/middleware.ts", DENY)]);
        assert!(!flow.complete);
        let cfg = "module.exports = { basePath: '/app' };\n";
        let flow = next_flow("web/proxy.ts", &proxy, &[("web/next.config.js", cfg)]);
        assert!(!flow.complete);
        assert_eq!(guarded(&flow).len(), 12);
    }

    #[test]
    fn the_body_decides_which_paths_are_protected() {
        let mw = "import { NextResponse } from 'next/server';
export async function middleware(request) {
  const { pathname } = request.nextUrl;
  if (pathname.startsWith('/dashboard') && !s) {
    return NextResponse.redirect(new URL('/sign-in', request.url));
  }
  return NextResponse.next();
}
";
        let flow = next_flow("web/middleware.ts", mw, &[]);
        assert_eq!(
            guarded(&flow),
            vec!["/dashboard", "/dashboard/a/b", "/dashboardx"]
        );
        assert_eq!(state(&flow, "web:/about"), AccessState::Undeclared);
        assert!(flow.complete, "{:?}", flow.diagnostics);
    }

    #[test]
    fn a_body_without_a_deny_exit_protects_nothing() {
        let mw = "import { NextResponse } from 'next/server';
export function middleware(request) {
  const res = NextResponse.next();
  res.cookies.set('seen', '1');
  return res;
}
";
        let flow = next_flow("web/middleware.ts", mw, &[]);
        assert!(
            flow.screens
                .iter()
                .all(|s| s.access == AccessState::Undeclared)
        );
        assert!(
            flow.endpoints
                .iter()
                .all(|e| e.access == AccessState::Undeclared)
        );
        assert!(flow.complete, "{:?}", flow.diagnostics);
    }

    #[test]
    fn an_unreadable_body_is_unknown_on_every_covered_route() {
        let mw = "import { NextResponse } from 'next/server';
export function middleware(request) {
  switch (true) {
    case /^\\/admin/.test(request.nextUrl.pathname):
      return NextResponse.redirect(new URL('/login', request.url));
  }
  return NextResponse.next();
}
";
        let flow = next_flow("web/middleware.ts", mw, &[]);
        assert!(
            flow.screens.iter().all(|s| unknown_state(&s.access)),
            "{:?}",
            screens(&flow)
        );
        assert!(!flow.complete);
    }

    #[test]
    fn a_pass_through_hof_protects_nothing_and_a_denying_one_does() {
        let handler = "import { withAuth } from '../../../lib/auth';
export const GET = withAuth(async () => Response.json({}));
";
        let base = |lib: &str| {
            extract(
                project(&[
                    ("web/package.json", NEXT_PKG),
                    ("web/app/api/user/route.ts", handler),
                    ("web/lib/auth.ts", lib),
                ])
                .path(),
            )
        };
        let flow = base("export function withAuth(h) { return h; }\n");
        assert_eq!(flow.endpoints[0].access, AccessState::Undeclared);
        let flow = base(
            "export function withAuth(h) {
  return async (req) => {
    if (!user) return new Response(null, { status: 401 });
    return h(req);
  };
}
",
        );
        assert_eq!(
            flow.endpoints[0].access,
            protected("withAuth"),
            "{:?}",
            flow.diagnostics
        );
        let pkg = handler.replace("../../../lib/auth", "some-pkg");
        let flow = extract(
            project(&[
                ("web/package.json", NEXT_PKG),
                ("web/app/api/user/route.ts", &pkg),
            ])
            .path(),
        );
        assert_eq!(
            flow.endpoints[0].access,
            AccessState::Protected(Condition::Unknown("withAuth".into()))
        );
        assert!(!flow.complete);
    }

    #[test]
    fn clerk_middleware_without_a_callback_protects_nothing() {
        let mw = "import { clerkMiddleware } from '@clerk/nextjs/server';
export default clerkMiddleware();
export const config = { matcher: ['/((?!_next).*)'] };
";
        let flow = next_flow("web/middleware.ts", mw, &[]);
        assert!(
            flow.screens
                .iter()
                .all(|s| s.access == AccessState::Undeclared),
            "{:?}",
            screens(&flow)
        );
        assert!(flow.complete, "{:?}", flow.diagnostics);
    }

    #[test]
    fn try_catch_in_a_middleware_keeps_the_dashboard_protected_and_the_rest_undeclared() {
        let mw = "import { NextResponse } from 'next/server';
const protectedRoutes = '/dashboard';
export async function middleware(request) {
  const { pathname } = request.nextUrl;
  const s = request.cookies.get('session');
  const isProtectedRoute = pathname.startsWith(protectedRoutes);
  if (isProtectedRoute && !s) {
    return NextResponse.redirect(new URL('/sign-in', request.url));
  }
  let res = NextResponse.next();
  if (s && request.method === 'GET') {
    try {
      res.cookies.set('session', 'x');
    } catch (error) {
      res.cookies.delete('session');
      if (isProtectedRoute) {
        return NextResponse.redirect(new URL('/sign-in', request.url));
      }
    }
  }
  return res;
}
";
        let flow = next_flow("web/middleware.ts", mw, &[]);
        assert!(matches!(
            state(&flow, "web:/dashboard"),
            AccessState::Protected(_)
        ));
        assert_eq!(state(&flow, "web:/about"), AccessState::Undeclared);
        assert_eq!(state(&flow, "web:/"), AccessState::Undeclared);
        assert!(!flow.complete, "the unknown exception condition is a gap");
    }

    #[test]
    fn a_typed_pass_through_wrapper_that_logs_and_rethrows_protects_nothing() {
        let lib = "export const Instrument = (handler: Handler): Handler => {
  return async (req, res) => {
    try {
      await handler(req, res);
    } catch (error) {
      log(error);
      throw error;
    } finally {
      done();
    }
  };
};
export default Instrument;
";
        let handler = "import Instrument from '../../lib/instrument';
export default Instrument(handler);
";
        let flow = extract(
            project(&[
                ("web/package.json", NEXT_PKG),
                ("web/pages/api/x.ts", handler),
                ("web/lib/instrument.ts", lib),
            ])
            .path(),
        );
        assert_eq!(
            flow.endpoints[0].access,
            AccessState::Undeclared,
            "{:?}",
            flow.diagnostics
        );
        assert!(flow.complete, "{:?}", flow.diagnostics);
    }

    #[test]
    fn hooks_and_imperative_navigation_in_a_wrapper_are_unknown() {
        for gate in [
            "export function Gate({ children }) { useRequireAuth(); return children; }\n",
            "export const Gate = ({ children }) => { useRequireAuth(); return children; };\n",
            "export function Gate({ children }) { useEffect(() => { if (!user) nav('/login'); }); return children; }\n",
            "export function Gate({ children }) { if (!user) { navigate('/login'); } return children; }\n",
        ] {
            let flow = gated(GATE_APP, gate);
            assert!(
                unknown_state(&state(&flow, "c:/a")),
                "{gate}: {:?}",
                flow.diagnostics
            );
            assert!(!flow.complete, "{gate}");
        }
        // Built-in hooks and a hook whose result is used stay readable.
        let gate = "export function Gate({ children }) {
  const [n] = useState(0);
  const user = useAuth();
  if (!user) return <Navigate to=\"/login\" />;
  return children;
}
";
        let flow = gated(GATE_APP, gate);
        assert_eq!(
            state(&flow, "c:/a"),
            protected("user"),
            "{:?}",
            flow.diagnostics
        );
    }

    #[test]
    fn nested_handler_wrappers_are_all_read() {
        let handler = "import { withA, withB } from '../../../lib/auth';
export const GET = withA(withB(async () => Response.json({})));
";
        let deny = "return async (req) => { if (!user) return new Response(null, { status: 401 }); return h(req); };";
        let run = |lib: &str, handler: &str| {
            extract(
                project(&[
                    ("web/package.json", NEXT_PKG),
                    ("web/app/api/user/route.ts", handler),
                    ("web/lib/auth.ts", lib),
                ])
                .path(),
            )
        };
        let pass = "export function withA(h) { return h; }\n";
        let lib = format!("{pass}export function withB(h) {{ {deny} }}\n");
        let flow = run(&lib, handler);
        assert_eq!(
            flow.endpoints[0].access,
            protected("withB"),
            "{:?}",
            flow.diagnostics
        );
        let lib = format!(
            "export function withA(h) {{ {deny} }}\nexport function withB(h) {{ return h; }}\n"
        );
        let flow = run(&lib, handler);
        assert_eq!(
            flow.endpoints[0].access,
            protected("withA"),
            "{:?}",
            flow.diagnostics
        );
        let pkg = handler.replace(
            "import { withA, withB } from '../../../lib/auth';",
            "import { withA } from '../../../lib/auth';\nimport { withB } from 'some-pkg';",
        );
        let flow = run(pass, &pkg);
        assert!(
            matches!(&flow.endpoints[0].access, AccessState::Protected(Condition::Unknown(n)) if n == "withB"),
            "{:?}",
            flow.endpoints[0].access
        );
        assert!(!flow.complete);
    }

    #[test]
    fn a_wrapper_declaration_from_an_unused_call_is_unknown() {
        for gate in [
            "export function Gate({ children }) { const _ = useRequireAuth(); return children; }\n",
            "export function Gate({ children }) { const user = useAuth(); return children; }\n",
            "export const Gate = ({ children }) => { const { user } = useSession(); return children; };\n",
        ] {
            let flow = gated(GATE_APP, gate);
            assert!(
                unknown_state(&state(&flow, "c:/a")),
                "{gate}: {:?}",
                flow.diagnostics
            );
            assert!(!flow.complete, "{gate}");
        }
        // Used in the check, or a plain hook: still read.
        let gate = "export function Gate({ children }) {
  const [n] = useState(0);
  const { user } = useAuth();
  if (!user) return <Navigate to=\"/login\" />;
  return children;
}
";
        let flow = gated(GATE_APP, gate);
        assert_eq!(
            state(&flow, "c:/a"),
            protected("user"),
            "{:?}",
            flow.diagnostics
        );
    }

    #[test]
    fn a_hook_value_only_passed_to_the_rendered_children_is_unknown() {
        for gate in [
            "export function Gate({ children }) { const user = useRequireAuth(); return <Layout user={user}>{children}</Layout>; }\n",
            "export function Gate({ children }) { const user = useRequireAuth(); if (loading) return null; return <Layout user={user}>{children}</Layout>; }\n",
        ] {
            let flow = gated(GATE_APP, gate);
            assert!(
                unknown_state(&state(&flow, "c:/a")),
                "{gate}: {:?}",
                flow.diagnostics
            );
            assert!(!flow.complete, "{gate}");
        }
    }
}
