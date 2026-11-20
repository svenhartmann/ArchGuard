//! Laravel extractor for ui-flow/v1 (ADR 0021): routes, middleware,
//! controller checks and Blade templates, read as text through `lang_php`
//! (comments blanked, literals read), no PHP parser and no PHP run.
//!
//! An application is a folder holding `artisan` and `routes/web.php` or
//! `routes/api.php`; its folder name is the `application`. Rules:
//! - Routes: `routes/web.php` (group `web`) and `routes/api.php` (group
//!   `api`, prefix `api` or the literal `apiPrefix:` of `bootstrap/app.php`),
//!   unless `RouteServiceProvider.php` loads them itself (`require
//!   base_path(…)`, `->group(base_path(…))`, with its literal `prefix` and
//!   `middleware`). `Route::get/post/put/patch/delete/options/any/match/view`
//!   with a literal path (or an enum case / class constant of the scan);
//!   actions `[C::class, 'm']`, `'C@m'` (group `namespace`), `C::class`
//!   (`__invoke`), `'m'` under `Route::controller(C::class)`, or a closure.
//!   `Route::resource/apiResource('a.b', C::class)` gives the standard
//!   actions, `only`/`except` (argument or chained) honoured; the parameter is
//!   the resource name without a trailing `s`. Groups: `Route::prefix()`,
//!   `middleware()`, `name()`/`as`, `namespace()`, `controller()`,
//!   `withoutMiddleware()` chains before `->group(fn)`, `Route::group([…],
//!   fn)`, and `require __DIR__.'/x.php'` / `base_path('routes/x.php')`.
//!   After the verb: `->name()`, `->middleware()`, `->withoutMiddleware()`,
//!   `->can('ability')`. `Route::redirect` and `Route::fallback` give nothing;
//!   `Route::resources`, `singleton` and non-literal paths are gaps.
//! - Access (all must hold): `auth`, `auth:<guard>`, `auth.basic` = signed in;
//!   `guest` = not signed in; `can:ability,Model` and `->can()` =
//!   `policy(ability)`; spatie `role:a|b` = any role, `permission:a|b` = any
//!   policy, `role_or_permission:` = any of both. A name listed in
//!   `[views.access]` (alias or middleware class name, arguments after `:`)
//!   takes that meaning first. Middleware groups and aliases come from
//!   `$middlewareGroups`, `$middlewareAliases`/`$routeMiddleware` (Kernel) and
//!   `->alias([…])`, `->web/api(append:, prepend:)`, `->appendToGroup()` in
//!   `bootstrap/app.php`; group members without a meaning are skipped, a
//!   route-level name without one is a note. Controllers: `$this->middleware(…)`
//!   in `__construct` (`only`/`except`; a closure is read like an action
//!   body), `$this->authorizeResource(M::class)` (the resource abilities),
//!   and Laravel 11 `static middleware()` with `new Middleware(…, only:,
//!   except:)`. Action bodies: `$this->authorize('a', …)` and
//!   `Gate::authorize('a', …)` = `policy(a)`, and calls of functions listed in
//!   `[views.access]` (`$this->f`, `self::f`, `static::f`, `f`) with the first
//!   argument as `$arg` — each only as a statement at the top of the body; one
//!   inside a block or condition is a note. `withoutMiddleware('auth')`
//!   removing access makes the route `open` when nothing else guards it;
//!   nothing at all is `undeclared`. Policy classes (`*Policy`) give their
//!   public methods as policies whose requirement is `unknown`.
//! - Screens: a GET or ANY route whose action or closure returns
//!   `view('x.y')` / `View::make('x.y')` (literal), or `Route::view`; id
//!   `<app>:<route>`, name the first view. `handler` is `Class.method`.
//! - Blade (`resources/views/**.blade.php`): `<a href>` and `<form action>`
//!   with a literal path, `{{ url('/x/' . $id) }}` or `{{ url("/x/{$id}") }}`
//!   (expressions become parameter segments; a path that starts with one is
//!   not read), `{{ route('name', …) }}` or `/x/{{ $id }}`; query and
//!   fragment dropped, external links skipped. A target that is a screen
//!   is a transition; a GET route without a view an HTTP action; a form with
//!   another method (`@method`, `method_field`, `_method`) an HTTP action.
//!   Links of the route's views and their `@include`s start at that screen;
//!   templates named in `@extends` (layouts) and what they include start at
//!   every screen of the application. `@include`, `@includeIf`,
//!   `@includeWhen/Unless`, `@each`, `@component` and `<x-name>` (the
//!   template `components.name`). Conditions: `@can`, `@cannot`,
//!   `@canany`, `@auth`, `@guest`, `@if`/`@elseif`/`@unless` over `!`, `&&`,
//!   `||` with listed functions, `auth()->check()`, `Gate::allows()`;
//!   `@else` negates; an `@if` with none of these adds no condition, other
//!   parts of a mixed one stay `unknown`.
//!
//! Not read: route files beyond these, `Route::resources`, controller
//! middleware of base classes, checks in helper methods, view names built at
//! run time, class-based Blade components and `@includeFirst` (gaps), forms
//! without `action`.
use crate::extract_ctx::{
    ExtractContext, dir_name, finish, gap, read_text, walk, with_root_config,
};
use crate::lang_blank::{blank_regions, find};
use crate::lang_php::{self as php, Class};
use crate::lang_text::{
    collapse, ident_end, idents, is_id, line, matching, matching_back, next_nonws, prev_nonws,
};
use crate::services::support::test_path;
use access::{Acc, all, and, any, meaning, not};
use archguard_core::{
    ANY_METHOD, AccessMeaning, AccessState, Action, Call, Condition, DiagnosticSeverity,
    EndpointAccess, Origin, PolicyDef, Screen, ScreenKind, Target, Transition, UiFlow,
    UiFlowDiagnostic, best_route, path_patterns_match,
};
use index::App;
use routes::{Handler, Mw, RouteDef};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::Path;

const SOURCE: &str = "laravel";
const MAX_FILES: usize = 20_000;
/// Application folders without routes, controllers or middleware.
const SKIP: [&str; 7] = [
    "storage/",
    "resources/",
    "public/",
    "lang/",
    "database/",
    "config/",
    "bootstrap/cache/",
];

pub fn extract(root: &Path) -> UiFlow {
    with_root_config(root, extract_with)
}

/// [`extract`] for one run.
pub fn extract_with(ctx: &ExtractContext) -> UiFlow {
    let mut flow = UiFlow::default();
    let Ok(walk) = walk(ctx.root) else {
        return flow;
    };
    let files: BTreeSet<&str> = walk.files.iter().map(String::as_str).collect();
    let apps: Vec<&str> = (files.iter())
        .filter_map(|f| {
            let (dir, name) = f.rsplit_once('/').unwrap_or(("", f));
            let routes = ["routes/web.php", "routes/api.php"];
            let has_routes = routes
                .iter()
                .any(|r| files.contains(join_rel(dir, r).as_str()));
            (name == "artisan" && has_routes).then_some(dir)
        })
        .collect();
    if let (Some(e), false) = (&ctx.config_error, apps.is_empty()) {
        let message = format!("Configuration not read; [views.access] not applied: {e}");
        note(&mut flow.diagnostics, ".", 0, message);
    }
    for dir in apps {
        let app = App::read(ctx, dir, &walk.files, &mut flow.diagnostics);
        let out = std::mem::take(&mut flow.diagnostics);
        Run {
            app: &app,
            out,
            noted: BTreeSet::new(),
        }
        .extract(&mut flow);
    }
    finish(&mut flow);
    flow
}

fn join_rel(dir: &str, rel: &str) -> String {
    if dir.is_empty() {
        rel.to_string()
    } else {
        format!("{dir}/{rel}")
    }
}

fn note(out: &mut Vec<UiFlowDiagnostic>, file: &str, line: u32, message: String) {
    let severity = DiagnosticSeverity::Note;
    out.push(UiFlowDiagnostic {
        file: file.into(),
        line,
        message,
        severity,
    });
}

/// One PHP file: comments blanked (`code`), literal contents blanked too (`mask`).
struct Src {
    rel: String,
    code: String,
    mask: String,
    uses: BTreeMap<String, String>,
    namespace: String,
}

impl Src {
    fn new(rel: String, text: &str) -> Self {
        let mask = php::mask(text);
        let (uses, namespace) = (php::uses(&mask), php::namespace(&mask));
        Src {
            rel,
            code: php::code(text),
            uses,
            namespace,
            mask,
        }
    }
    fn line(&self, at: usize) -> u32 {
        line(&self.code, at)
    }
    fn text(&self, r: &Range<usize>) -> &str {
        self.code[r.clone()].trim()
    }
}

fn short(class: &str) -> &str {
    class.rsplit('\\').next().unwrap_or(class)
}

/// A middleware or alias target: a literal name, or `X::class` as `X`.
fn member(s: &Src, r: &Range<usize>) -> Option<String> {
    let text = s.text(r);
    let class = || {
        text.strip_suffix("::class")
            .map(|c| short(c.trim()).to_string())
    };
    php::literal(text).or_else(class)
}

/// Brace depth of `to` relative to `from` (masked text).
fn depth(b: &[u8], from: usize, to: usize) -> i32 {
    b[from..to]
        .iter()
        .map(|&c| i32::from(c == b'{') - i32::from(c == b'}'))
        .sum()
}

/// Whether a controller middleware with `only`/`except` applies to `method`.
fn applies(method: &str, only: &Option<Vec<String>>, except: &[String]) -> bool {
    only.as_ref().is_none_or(|o| o.iter().any(|m| m == method))
        && !except.iter().any(|m| m == method)
}

/// One application's extraction with its diagnostics.
struct Run<'a> {
    app: &'a App<'a>,
    out: Vec<UiFlowDiagnostic>,
    /// Middleware names already noted as without meaning.
    noted: BTreeSet<String>,
}

/// A screen with the views it renders and where its route is declared.
struct ScreenViews {
    id: String,
    views: Vec<String>,
    file: String,
    line: u32,
}

impl Run<'_> {
    fn extract(mut self, flow: &mut UiFlow) {
        let routes = self.routes();
        let mut screens: Vec<ScreenViews> = vec![];
        for r in &routes {
            let mut views = vec![];
            let access = self.access(r, &mut views);
            self.endpoints(flow, r, &access);
            let get = r.methods.iter().any(|m| m == "GET" || m == ANY_METHOD);
            let id = format!("{}:{}", self.app.name, r.path);
            if get && !views.is_empty() && !screens.iter().any(|s| s.id == id) {
                flow.screens.push(self.screen(&id, r, &views[0], access));
                screens.push(ScreenViews {
                    id,
                    views,
                    file: r.file.clone(),
                    line: r.line,
                });
            }
        }
        self.policies(flow);
        self.blade(flow, &routes, &screens);
        flow.diagnostics = self.out;
    }

    fn endpoints(&self, flow: &mut UiFlow, r: &RouteDef, access: &AccessState) {
        let handler = match &r.handler {
            Handler::Action(c, m) => Some(format!("{}.{m}", short(c))),
            _ => None,
        };
        for method in &r.methods {
            flow.endpoints.push(EndpointAccess {
                method: method.clone(),
                path: r.path.clone(),
                access: access.clone(),
                application: Some(self.app.name.clone()),
                file: r.file.clone(),
                line: r.line,
                source: SOURCE.into(),
                handler: handler.clone(),
                sends: vec![],
            });
        }
    }

    fn screen(&self, id: &str, r: &RouteDef, view: &str, access: AccessState) -> Screen {
        Screen {
            id: id.to_string(),
            name: view.to_string(),
            kind: ScreenKind::Page,
            route: Some(r.path.clone()),
            application: Some(self.app.name.clone()),
            file: r.file.clone(),
            line: r.line,
            access: match access {
                AccessState::Protected(c) => Some(c),
                _ => None,
            },
            source: SOURCE.into(),
        }
    }

    fn gap(&mut self, file: &str, line: u32, message: String) {
        gap(&mut self.out, file, line, message);
    }

    fn at(&mut self, si: usize, at: usize, message: String) {
        let s = &self.app.srcs[si];
        gap(&mut self.out, &s.rel, s.line(at), message);
    }

    /// Public methods of `*Policy` classes as policies with an unknown requirement.
    fn policies(&mut self, flow: &mut UiFlow) {
        for (si, c) in self.app.classes.values() {
            let s = &self.app.srcs[*si];
            if c.kind != "class" || !c.name.ends_with("Policy") {
                continue;
            }
            for m in &c.methods {
                let start = s.code[..m.at].rfind(['{', '}', ';']).unwrap_or(0);
                let modifiers = &s.mask[start..m.at];
                let hidden = modifiers.contains("private") || modifiers.contains("protected");
                if m.body.is_none() || m.name.starts_with("__") || m.name == "before" || hidden {
                    continue;
                }
                flow.policies.push(PolicyDef {
                    name: m.name.clone(),
                    requires: Condition::Unknown(format!("{}::{}", c.name, m.name)),
                    file: s.rel.clone(),
                    line: s.line(m.at),
                    source: SOURCE.into(),
                });
            }
        }
    }
}

/// The read sources of one application and what they declare.
mod index {
    use super::*;

    /// `Class::NAME` → value; `None` when two declarations disagree.
    type Values = BTreeMap<String, Option<String>>;

    fn record(map: &mut Values, key: String, value: Option<String>) {
        match map.get(&key) {
            Some(old) if *old != value => drop(map.insert(key, None)),
            Some(_) => {}
            None => drop(map.insert(key, value)),
        }
    }

    /// Middleware aliases (alias → class short name or name) and groups.
    #[derive(Default)]
    pub(super) struct Tables {
        pub(super) aliases: BTreeMap<String, String>,
        pub(super) groups: BTreeMap<String, Vec<String>>,
    }

    pub(super) struct App<'a> {
        pub(super) name: String,
        /// The folder with a trailing `/`, `""` for the root.
        pub(super) dir: String,
        pub(super) access: &'a BTreeMap<String, AccessMeaning>,
        pub(super) srcs: Vec<Src>,
        /// Fully qualified name → (source, class).
        pub(super) classes: BTreeMap<String, (usize, Class)>,
        short: BTreeMap<String, Vec<String>>,
        /// Enum cases and class constants (`Permission::View`).
        values: Values,
        /// Enum methods `return 'lit' . $this->value;` as `Enum::method` → `lit`.
        prefixes: Values,
        pub(super) mw: Tables,
        pub(super) api_prefix: String,
        /// View name → (file, text).
        pub(super) templates: BTreeMap<String, (String, String)>,
    }

    impl<'a> App<'a> {
        pub(super) fn read(
            ctx: &'a ExtractContext,
            dir: &str,
            files: &[String],
            out: &mut Vec<UiFlowDiagnostic>,
        ) -> Self {
            let prefix = if dir.is_empty() {
                String::new()
            } else {
                format!("{dir}/")
            };
            let mut app = App {
                name: dir_name(ctx.root, dir),
                dir: prefix.clone(),
                access: &ctx.config.views.access,
                srcs: vec![],
                classes: BTreeMap::new(),
                short: BTreeMap::new(),
                values: BTreeMap::new(),
                prefixes: BTreeMap::new(),
                mw: Tables::default(),
                api_prefix: "api".into(),
                templates: BTreeMap::new(),
            };
            let (sources, views) = app_files(&prefix, files, out);
            let unread = "File not read: larger than 1 MiB, not a regular file or not readable; \
                          its routes, checks and links are unknown.";
            for rel in sources {
                match read_text(ctx.root, rel, 1 << 20) {
                    Ok(text) => app.srcs.push(Src::new(rel.clone(), &text)),
                    Err(_) => gap(out, rel, 0, unread),
                }
            }
            for (rel, name) in views {
                match read_text(ctx.root, rel, 1 << 20) {
                    Ok(text) => drop(app.templates.insert(name, (rel.clone(), text))),
                    Err(_) => gap(out, rel, 0, unread),
                }
            }
            app.index();
            app
        }

        fn index(&mut self) {
            for (i, s) in self.srcs.iter().enumerate() {
                for c in php::classes(&s.mask) {
                    let fq = match s.namespace.as_str() {
                        "" => c.name.clone(),
                        ns => format!("{ns}\\{}", c.name),
                    };
                    declared(s, &c, &mut self.values, &mut self.prefixes);
                    kernel(s, &c, &mut self.mw);
                    self.short
                        .entry(c.name.clone())
                        .or_default()
                        .push(fq.clone());
                    self.classes.insert(fq, (i, c));
                }
            }
            let bootstrap = format!("{}bootstrap/app.php", self.dir);
            if let Some(s) = self.srcs.iter().find(|s| s.rel == bootstrap) {
                bootstrap_calls(s, &mut self.mw);
                if let Some(p) = api_prefix(s) {
                    self.api_prefix = p;
                }
            }
        }

        /// The class `name` (fully qualified, else a short name that is unique).
        pub(super) fn class(&self, name: &str) -> Option<&(usize, Class)> {
            self.classes
                .get(name)
                .or_else(|| match self.short.get(short(name))?.as_slice() {
                    [one] => self.classes.get(one),
                    _ => None,
                })
        }

        /// A literal, an enum case or class constant (`E::X`, `E::X->value`) or
        /// an enum method of the `'lit' . $this->value` form (`E::X->m()`).
        pub(super) fn value_of(&self, text: &str) -> Option<String> {
            if let Some(v) = php::literal(text) {
                return Some(v);
            }
            let t: String = text.chars().filter(|c| !c.is_whitespace()).collect();
            let (base, method) = match t.split_once("->") {
                Some((b, m)) => (b, Some(m)),
                None => (t.as_str(), None),
            };
            let (class, case) = base.rsplit_once("::")?;
            let class = short(class);
            let value = self.values.get(&format!("{class}::{case}"))?.clone()?;
            match method {
                None | Some("value") => Some(value),
                Some(m) => {
                    let m = m.strip_suffix("()")?;
                    let prefix = self.prefixes.get(&format!("{class}::{m}"))?.clone()?;
                    Some(format!("{prefix}{value}"))
                }
            }
        }

        pub(super) fn value(&self, s: &Src, r: &Range<usize>) -> Option<String> {
            self.value_of(s.text(r))
        }

        pub(super) fn src(&self, rel: &str) -> Option<usize> {
            self.srcs.iter().position(|s| s.rel == rel)
        }

        /// Adds what middleware `text` (`name:arg,…`) guards with; `false` when
        /// its name has no meaning by rule.
        pub(super) fn middleware(&self, text: &str, acc: &mut Acc, depth: u8) -> bool {
            let (name, args): (&str, Vec<String>) = match text.split_once(':') {
                Some((n, a)) => {
                    let args = a.split(',').map(|x| x.trim().to_string());
                    (n.trim(), args.filter(|x| !x.is_empty()).collect())
                }
                None => (text.trim(), vec![]),
            };
            let class = self.mw.aliases.get(name).map(|c| short(c));
            if let Some(m) = self
                .access
                .get(name)
                .or_else(|| class.and_then(|c| self.access.get(c)))
            {
                acc.add(meaning(m, name, &args));
                return true;
            }
            if let Some(members) = self.mw.groups.get(name) {
                for m in members.iter().filter(|_| depth < 8) {
                    self.middleware(m, acc, depth + 1);
                }
                return true;
            }
            if access::builtin(name, &args, acc) {
                return true;
            }
            let alias = (self.mw.aliases.iter())
                .find(|(_, c)| short(c) == name)
                .map(|(a, _)| a.as_str());
            let alias = alias.or_else(|| access::builtin_class(name));
            alias.is_some_and(|a| a != name && access::builtin(a, &args, acc))
        }
    }

    /// The PHP sources and Blade views (file, view name) of the application.
    fn app_files<'f>(
        prefix: &str,
        files: &'f [String],
        out: &mut Vec<UiFlowDiagnostic>,
    ) -> (Vec<&'f String>, Vec<(&'f String, String)>) {
        let (mut sources, mut views) = (vec![], vec![]);
        for f in files {
            let Some(inner) = f.strip_prefix(prefix) else {
                continue;
            };
            let view = inner
                .strip_prefix("resources/views/")
                .and_then(|v| v.strip_suffix(".blade.php"));
            if let Some(v) = view {
                views.push((f, v.replace('/', ".")));
            } else if f.ends_with(".php")
                && !test_path(inner)
                && !SKIP.iter().any(|d| inner.starts_with(d))
            {
                sources.push(f);
            }
        }
        if sources.len() > MAX_FILES {
            let message = format!("More than {MAX_FILES} PHP files; the rest were not read.");
            gap(out, sources[MAX_FILES], 0, message);
            sources.truncate(MAX_FILES);
        }
        (sources, views)
    }

    /// Enum cases, class constants and `'lit' . $this->value` methods of `c`.
    fn declared(s: &Src, c: &Class, values: &mut Values, prefixes: &mut Values) {
        let b = s.mask.as_bytes();
        let mut at: Vec<usize> = php::words(&s.mask, c.body.clone(), "case");
        at.extend(php::words(&s.mask, c.body.clone(), "const"));
        for k in at.into_iter().filter(|&k| depth(b, c.body.start, k) == 0) {
            let end = php::statement_end(&s.mask, k);
            let Some(eq) = s.mask[k..end].find('=').map(|e| k + e) else {
                continue;
            };
            let name = s.mask[k..eq].split_whitespace().last().unwrap_or_default();
            record(
                values,
                format!("{}::{name}", c.name),
                php::literal(&s.code[eq + 1..end]),
            );
        }
        for m in &c.methods {
            let Some(body) = &m.body else { continue };
            let lit = (s.text(body).strip_prefix("return"))
                .and_then(|t| t.trim().strip_suffix(';'))
                .and_then(|t| t.trim().strip_suffix("$this->value"))
                .and_then(|t| t.trim().strip_suffix('.'))
                .and_then(php::literal);
            if lit.is_some() {
                record(prefixes, format!("{}::{}", c.name, m.name), lit);
            }
        }
    }

    /// The Kernel's `$middlewareAliases`/`$routeMiddleware` and `$middlewareGroups`.
    fn kernel(s: &Src, c: &Class, tables: &mut Tables) {
        let b = s.mask.as_bytes();
        for prop in [
            "$middlewareAliases",
            "$routeMiddleware",
            "$middlewareGroups",
        ] {
            let open = find(&b[..c.body.end], c.body.start, prop.as_bytes())
                .and_then(|k| next_nonws(b, k + prop.len()))
                .filter(|&k| b[k] == b'=')
                .and_then(|k| next_nonws(b, k + 1))
                .filter(|&k| b[k] == b'[');
            for (key, value) in open
                .map(|o| php::array_items(&s.mask, o))
                .unwrap_or_default()
            {
                let Some(key) = key.and_then(|k| php::literal(s.text(&k))) else {
                    continue;
                };
                if prop == "$middlewareGroups" {
                    tables
                        .groups
                        .entry(key)
                        .or_default()
                        .extend(members(s, &value));
                } else if let Some(m) = member(s, &value) {
                    tables.aliases.insert(key, m);
                }
            }
        }
    }

    fn members(s: &Src, r: &Range<usize>) -> Vec<String> {
        match s.mask.as_bytes()[r.start] {
            b'[' => (php::array_items(&s.mask, r.start).iter())
                .filter_map(|(_, v)| member(s, v))
                .collect(),
            _ => member(s, r).into_iter().collect(),
        }
    }

    /// Laravel 11 `bootstrap/app.php`: `->alias()`, `->web/api(append:,
    /// prepend:)`, `->appendToGroup/prependToGroup()`.
    fn bootstrap_calls(s: &Src, tables: &mut Tables) {
        let b = s.mask.as_bytes();
        for call in ["alias", "web", "api", "appendToGroup", "prependToGroup"] {
            let pat = format!("->{call}(");
            let mut from = 0;
            while let Some(k) = find(b, from, pat.as_bytes()) {
                from = k + 1;
                if let Some((args, _)) = php::args(&s.mask, k + pat.len() - 1) {
                    bootstrap_call(s, call, &args, tables);
                }
            }
        }
    }

    fn bootstrap_call(s: &Src, call: &str, args: &[Range<usize>], tables: &mut Tables) {
        match call {
            "alias" => {
                let items = args.first().map(|a| php::array_items(&s.mask, a.start));
                for (key, value) in items.unwrap_or_default() {
                    let key = key.and_then(|k| php::literal(s.text(&k)));
                    if let (Some(key), Some(m)) = (key, member(s, &value)) {
                        tables.aliases.insert(key, m);
                    }
                }
            }
            "web" | "api" => {
                for a in args {
                    let named = s.mask[a.clone()].split_once(':').map(|(n, _)| n.trim());
                    let open = (named.filter(|n| ["append", "prepend"].contains(n)))
                        .and_then(|_| s.mask[a.clone()].find('[').map(|o| a.start + o));
                    if let Some(open) = open {
                        tables
                            .groups
                            .entry(call.into())
                            .or_default()
                            .extend(members(s, &(open..a.end)));
                    }
                }
            }
            _ => {
                let group = args.first().and_then(|g| php::literal(s.text(g)));
                if let (Some(g), Some(m)) = (group, args.get(1)) {
                    tables.groups.entry(g).or_default().extend(members(s, m));
                }
            }
        }
    }

    /// The literal `apiPrefix:` of `bootstrap/app.php`.
    fn api_prefix(s: &Src) -> Option<String> {
        let b = s.mask.as_bytes();
        let q = find(b, 0, b"apiPrefix:").and_then(|k| next_nonws(b, k + 10))?;
        (matches!(b[q], b'\'' | b'"')).then_some(())?;
        let end = find(b, q + 1, &b[q..q + 1])?;
        php::literal(&s.code[q..=end])
    }
}

/// Conditions and the access meanings of middleware.
mod access {
    use super::*;

    /// What guards one route so far.
    #[derive(Default)]
    pub(super) struct Acc {
        pub(super) conds: Vec<Condition>,
        /// A listed `open` meaning.
        pub(super) open: bool,
        /// `withoutMiddleware` removed a middleware that guards.
        pub(super) removed: bool,
    }

    impl Acc {
        pub(super) fn add(&mut self, c: Option<Condition>) {
            match c {
                Some(c) => self.conds.push(c),
                None => self.open = true,
            }
        }
    }

    /// All of `conds`, flattened and deduplicated; none when empty.
    pub(super) fn all(conds: Vec<Condition>) -> Option<Condition> {
        let mut out: Vec<Condition> = vec![];
        for c in conds {
            let items = match c {
                Condition::All(inner) => inner,
                c => vec![c],
            };
            for c in items {
                if !out.contains(&c) {
                    out.push(c);
                }
            }
        }
        match out.len() {
            0 => None,
            1 => out.pop(),
            _ => Some(Condition::All(out)),
        }
    }

    pub(super) fn and(a: Option<Condition>, b: Option<Condition>) -> Option<Condition> {
        all(a.into_iter().chain(b).collect())
    }

    pub(super) fn any(mut items: Vec<Condition>) -> Condition {
        if items.len() == 1 {
            items.remove(0)
        } else {
            Condition::Any(items)
        }
    }

    pub(super) fn not(c: Condition) -> Condition {
        match c {
            Condition::Not(inner) => *inner,
            c => Condition::Not(Box::new(c)),
        }
    }

    /// A `[views.access]` meaning at a use site with `args`; `None` is open.
    pub(super) fn meaning(m: &AccessMeaning, name: &str, args: &[String]) -> Option<Condition> {
        use AccessMeaning as M;
        let each = |make: fn(String) -> Condition| match args {
            [] => Condition::Unknown(name.into()),
            _ => any(args.iter().cloned().map(make).collect()),
        };
        Some(match m.at_use_site(!args.is_empty()) {
            M::Open => return None,
            M::Authenticated => Condition::Authenticated,
            M::Never => Condition::Never,
            M::Role(n) => Condition::Role(n.clone()),
            M::Policy(n) => Condition::Policy(n.clone()),
            M::RoleFromArguments => each(Condition::Role),
            M::PolicyFromArguments => each(Condition::Policy),
            // `at_use_site` resolved the fallback; it is never a `$arg` form.
            M::FromArgumentsOr(..) => Condition::Unknown(name.into()),
        })
    }

    /// Built-in middleware by name; `false` when the name is none of them.
    pub(super) fn builtin(name: &str, args: &[String], acc: &mut Acc) -> bool {
        let names = || {
            let split = args
                .iter()
                .flat_map(|a| a.split('|'))
                .map(|a| a.trim().to_string());
            split.filter(|a| !a.is_empty())
        };
        let both = |n: String| [Condition::Role(n.clone()), Condition::Policy(n)];
        let c = match name {
            "auth" | "auth.basic" => Condition::Authenticated,
            "guest" => not(Condition::Authenticated),
            "can" => args.first().map_or(Condition::Unknown("can".into()), |a| {
                Condition::Policy(a.clone())
            }),
            "role" => any(names().map(Condition::Role).collect()),
            "permission" => any(names().map(Condition::Policy).collect()),
            "role_or_permission" => any(names().flat_map(both).collect()),
            "throttle" | "verified" | "signed" | "bindings" | "cache.headers"
            | "password.confirm" | "precognitive" | "auth.session" | "web" | "api" => return true,
            _ => return false,
        };
        acc.conds.push(c);
        true
    }

    /// Laravel's own middleware classes of the built-in aliases.
    pub(super) fn builtin_class(class: &str) -> Option<&'static str> {
        Some(match class {
            "Authenticate" => "auth",
            "AuthenticateWithBasicAuth" => "auth.basic",
            "RedirectIfAuthenticated" => "guest",
            "Authorize" => "can",
            "ThrottleRequests" => "throttle",
            _ => return None,
        })
    }

    /// Ability per resource action (`authorizeResource`).
    pub(super) fn resource_ability(method: &str) -> Option<&'static str> {
        Some(match method {
            "index" => "viewAny",
            "show" => "view",
            "create" | "store" => "create",
            "edit" | "update" => "update",
            "destroy" => "delete",
            _ => return None,
        })
    }
}

/// Route files into [`RouteDef`]s.
mod routes {
    use super::*;

    #[derive(Clone, Debug)]
    pub(super) enum Mw {
        Name(String),
        /// Source text of a middleware the rules cannot read.
        Unread(String),
    }

    #[derive(Clone, Default)]
    pub(super) struct Group {
        prefix: String,
        name: String,
        mw: Vec<Mw>,
        without: Vec<String>,
        controller: Option<String>,
        namespace: String,
    }

    pub(super) enum Handler {
        Action(String, String),
        Closure(usize, Range<usize>),
        View(String),
        Unread,
    }

    pub(super) struct RouteDef {
        pub(super) methods: Vec<String>,
        pub(super) path: String,
        pub(super) name: Option<String>,
        pub(super) mw: Vec<Mw>,
        pub(super) without: Vec<String>,
        pub(super) handler: Handler,
        pub(super) file: String,
        pub(super) line: u32,
    }

    /// One call of a chain: its name and argument ranges.
    pub(super) struct CallAt {
        pub(super) name: String,
        pub(super) args: Vec<Range<usize>>,
    }

    /// `Route::a(…)->b(…)…` starting at `k` (the `Route` word).
    fn chain(mask: &str, k: usize) -> Vec<CallAt> {
        calls(mask, k + 5, b"::")
    }

    /// `->a(…)->b(…)` calls right after `from`.
    pub(super) fn chain_after(mask: &str, from: usize) -> Vec<CallAt> {
        calls(mask, from, b"->")
    }

    /// Calls `<first>name(…)` from `from`, then `->name(…)` while they follow.
    fn calls(mask: &str, from: usize, first: &[u8]) -> Vec<CallAt> {
        let b = mask.as_bytes();
        let (mut out, mut i, mut sep) = (vec![], from, first);
        while let Some(p) = next_nonws(b, i).filter(|&p| b[p..].starts_with(sep)) {
            let Some(n) = next_nonws(b, p + sep.len()) else {
                break;
            };
            let ne = ident_end(b, n);
            let Some(open) = next_nonws(b, ne).filter(|&o| ne > n && b[o] == b'(') else {
                break;
            };
            let Some((args, close)) = php::args(mask, open) else {
                break;
            };
            out.push(CallAt {
                name: mask[n..ne].to_string(),
                args,
            });
            i = close + 1;
            sep = b"->";
        }
        out
    }

    /// `function (…) use (…) { … }` or `fn (…) => …` spanning `r`: its body.
    pub(super) fn closure_body(mask: &str, r: &Range<usize>) -> Option<Range<usize>> {
        let b = mask.as_bytes();
        let mut start = r.start;
        if php::word_at(b, start, "static") {
            start = next_nonws(b, start + 6)?;
        }
        if php::word_at(b, start, "function") {
            let open = (start..r.end).find(|&j| b[j] == b'{')?;
            Some(open + 1..matching(b, open)?)
        } else if php::word_at(b, start, "fn") {
            let arrow = php::top_level(mask, start..r.end, "=>")?;
            Some(php::trim(mask, arrow + 2..r.end))
        } else {
            None
        }
    }

    fn join_path(prefix: &str, path: &str) -> String {
        let parts: Vec<&str> = prefix
            .split('/')
            .chain(path.split('/'))
            .filter(|s| !s.is_empty())
            .collect();
        format!("/{}", parts.join("/"))
    }

    /// shortcut: Laravel's `Str::singular` is not reproduced; `ies` becomes `y` and
    /// a trailing `s` is dropped, so a parameter name may differ (the shape does not).
    fn singular(word: &str) -> String {
        match word.strip_suffix("ies") {
            Some(stem) => format!("{stem}y"),
            None => word.strip_suffix('s').unwrap_or(word).to_string(),
        }
    }

    /// The ranges of `mask[range]` split at `op` outside brackets.
    pub(super) fn split_op(mask: &str, range: Range<usize>, op: &str) -> Vec<Range<usize>> {
        let mut out = vec![];
        let mut start = range.start;
        while let Some(k) = php::top_level(mask, start..range.end, op) {
            out.push(php::trim(mask, start..k));
            start = k + op.len();
        }
        out.push(php::trim(mask, start..range.end));
        out
    }

    const TERMINALS: [&str; 19] = [
        "group",
        "get",
        "post",
        "put",
        "patch",
        "delete",
        "options",
        "any",
        "match",
        "view",
        "resource",
        "apiResource",
        "resources",
        "apiResources",
        "singleton",
        "apiSingleton",
        "redirect",
        "permanentRedirect",
        "fallback",
    ];

    type ResourceAction = (&'static str, &'static [&'static str], &'static str);
    const RESOURCE: [ResourceAction; 7] = [
        ("index", &["GET"], ""),
        ("create", &["GET"], "/create"),
        ("store", &["POST"], ""),
        ("show", &["GET"], "/{}"),
        ("edit", &["GET"], "/{}/edit"),
        ("update", &["PUT", "PATCH"], "/{}"),
        ("destroy", &["DELETE"], "/{}"),
    ];

    impl Run<'_> {
        /// Every route of the application.
        pub(super) fn routes(&mut self) -> Vec<RouteDef> {
            let app = self.app;
            let mut routes = Routes {
                run: self,
                routes: vec![],
                loaded: BTreeSet::new(),
            };
            let provider = app
                .srcs
                .iter()
                .position(|s| s.rel.ends_with("/RouteServiceProvider.php"));
            if let Some(i) = provider {
                routes.block(i, 0..app.srcs[i].mask.len(), &Group::default(), 0);
            }
            let defaults = [
                ("routes/web.php", "web", ""),
                ("routes/api.php", "api", app.api_prefix.as_str()),
            ];
            for (file, group, prefix) in defaults {
                let rel = format!("{}{file}", app.dir);
                if !routes.loaded.contains(&rel) && app.src(&rel).is_some() {
                    let mw = vec![Mw::Name(group.into())];
                    let g = Group {
                        prefix: join_path("", prefix),
                        mw,
                        ..Group::default()
                    };
                    routes.file(&rel, &g, 0);
                }
            }
            routes.routes
        }
    }

    struct Routes<'r, 'a> {
        run: &'r mut Run<'a>,
        routes: Vec<RouteDef>,
        loaded: BTreeSet<String>,
    }

    impl Routes<'_, '_> {
        fn file(&mut self, rel: &str, g: &Group, depth: u8) {
            if depth > 16 {
                return;
            }
            self.loaded.insert(rel.to_string());
            match self.run.app.src(rel) {
                Some(i) => self.block(i, 0..self.run.app.srcs[i].mask.len(), g, depth),
                None => self.run.gap(
                    rel,
                    0,
                    "Route file not found or not read; its routes are unknown.".into(),
                ),
            }
        }

        /// The `Route::` statements and `require`s of `range`, in order.
        fn block(&mut self, si: usize, range: Range<usize>, g: &Group, depth: u8) {
            let app = self.run.app;
            let s = &app.srcs[si];
            let b = s.mask.as_bytes();
            let mut starts: Vec<usize> = (php::words(&s.mask, range.clone(), "Route").into_iter())
                .filter(|&k| b[k + 5..].starts_with(b"::"))
                .collect();
            for w in ["require", "require_once", "include", "include_once"] {
                starts.extend(php::words(&s.mask, range.clone(), w));
            }
            starts.sort_unstable();
            let mut done = range.start;
            for k in starts.into_iter().filter(|_| depth <= 16) {
                if k < done {
                    continue;
                }
                done = php::statement_end(&s.mask, k).min(range.end);
                if b[k] == b'R' {
                    self.statement(si, k, g, depth);
                } else {
                    self.include(si, k, done, g, depth);
                }
            }
        }

        fn include(&mut self, si: usize, k: usize, end: usize, g: &Group, depth: u8) {
            let s = &self.run.app.srcs[si];
            let expr = php::trim(&s.mask, ident_end(s.mask.as_bytes(), k)..end);
            match self.file_ref(si, &expr) {
                Some(rel) => self.file(&rel, g, depth + 1),
                None => {
                    let text = collapse(s.text(&expr));
                    let message = format!(
                        "Included file {text} is not read literally; its routes are unknown."
                    );
                    self.run.at(si, k, message);
                }
            }
        }

        /// `base_path('routes/x.php')` or `__DIR__ . '/x.php'` as a file of the scan.
        fn file_ref(&self, si: usize, r: &Range<usize>) -> Option<String> {
            let app = self.run.app;
            let s = &app.srcs[si];
            let b = s.mask.as_bytes();
            let mut r = r.clone();
            while b.get(r.start) == Some(&b'(') && matching(b, r.start) == Some(r.end - 1) {
                r = php::trim(&s.mask, r.start + 1..r.end - 1);
            }
            let text = &s.mask[r.clone()];
            let path = if let Some(rest) = text.strip_prefix("base_path") {
                let (args, _) = php::args(&s.mask, r.start + text.len() - rest.trim_start().len())?;
                format!("{}{}", app.dir, app.value(s, args.first()?)?)
            } else {
                let pieces = split_op(&s.mask, r.clone(), ".");
                let (first, rest) = pieces.split_first()?;
                (s.text(first) == "__DIR__").then_some(())?;
                let tail: Option<String> = rest.iter().map(|p| php::literal(s.text(p))).collect();
                format!(
                    "{}/{}",
                    s.rel.rsplit_once('/').map_or("", |(d, _)| d),
                    tail?
                )
            };
            let mut parts: Vec<&str> = vec![];
            for p in path.split('/') {
                match p {
                    "" | "." => {}
                    ".." => drop(parts.pop()),
                    p => parts.push(p),
                }
            }
            Some(parts.join("/"))
        }

        fn statement(&mut self, si: usize, k: usize, g: &Group, depth: u8) {
            let app = self.run.app;
            let s = &app.srcs[si];
            let calls = chain(&s.mask, k);
            let Some(t) = calls
                .iter()
                .position(|c| TERMINALS.contains(&c.name.as_str()))
            else {
                return;
            };
            let mut g = g.clone();
            for c in &calls[..t] {
                self.attr(si, &c.name, &c.args, &mut g);
            }
            let (term, after) = (&calls[t], &calls[t + 1..]);
            match term.name.as_str() {
                "group" => self.group(si, k, term, g, depth),
                "resource" | "apiResource" => self.resource(si, k, term, after, &g),
                "redirect" | "permanentRedirect" | "fallback" => {}
                "resources" | "apiResources" | "singleton" | "apiSingleton" => {
                    let message =
                        format!("Route::{} is not read; its routes are unknown.", term.name);
                    self.run.at(si, k, message);
                }
                _ => self.verb(si, k, term, after, &g),
            }
        }

        /// A group attribute or a registrar call before the verb.
        fn attr(&mut self, si: usize, name: &str, args: &[Range<usize>], g: &mut Group) {
            let app = self.run.app;
            let s = &app.srcs[si];
            let first = args.first();
            let value = first.and_then(|r| app.value(s, r));
            match (name, value) {
                ("prefix", Some(p)) => g.prefix = join_path(&g.prefix, &p),
                ("prefix", None) => {
                    let text = first.map_or(String::new(), |r| collapse(s.text(r)));
                    let message = format!(
                        "Route prefix {text} is not read literally; the paths below it are unknown."
                    );
                    self.run.at(si, first.map_or(0, |r| r.start), message);
                }
                ("middleware", _) => g.mw.extend(self.run.mws(si, args)),
                ("name" | "as", v) => g.name.push_str(&v.unwrap_or_default()),
                ("namespace", v) => {
                    g.namespace = v.unwrap_or_default().trim_start_matches('\\').to_string()
                }
                ("controller", _) => g.controller = first.and_then(|r| self.class_arg(si, r)),
                ("withoutMiddleware" | "excluded_middleware", _) => {
                    g.without.extend(self.names(si, args))
                }
                _ => {}
            }
        }

        /// Readable middleware names of arguments.
        fn names(&mut self, si: usize, args: &[Range<usize>]) -> Vec<String> {
            (self.run.mws(si, args).into_iter())
                .filter_map(|m| match m {
                    Mw::Name(n) => Some(n),
                    Mw::Unread(_) => None,
                })
                .collect()
        }

        fn group(&mut self, si: usize, k: usize, term: &CallAt, mut g: Group, depth: u8) {
            let app = self.run.app;
            let s = &app.srcs[si];
            let (attrs, body) = match term.args.as_slice() {
                [a, body] => (Some(a), body),
                [body] => (None, body),
                _ => {
                    return self
                        .run
                        .at(si, k, "Route group without a body is not read.".into());
                }
            };
            let attrs = attrs.filter(|a| s.mask.as_bytes()[a.start] == b'[');
            for (key, value) in attrs
                .map(|a| php::array_items(&s.mask, a.start))
                .unwrap_or_default()
            {
                if let Some(key) = key.and_then(|k| php::literal(s.text(&k))) {
                    self.attr(si, &key, &[value], &mut g);
                }
            }
            if let Some(cb) = closure_body(&s.mask, body) {
                self.block(si, cb, &g, depth + 1);
            } else if let Some(rel) = self.file_ref(si, body) {
                self.file(&rel, &g, depth + 1);
            } else {
                let text = collapse(s.text(body));
                let message =
                    format!("Route group body {text} is not read; its routes are unknown.");
                self.run.at(si, body.start, message);
            }
        }

        fn class_arg(&self, si: usize, r: &Range<usize>) -> Option<String> {
            let s = &self.run.app.srcs[si];
            let name = s.text(r).strip_suffix("::class")?.trim();
            Some(php::qualify(name, &s.uses, &s.namespace))
        }

        fn handler(&mut self, si: usize, r: &Range<usize>, g: &Group) -> Handler {
            let app = self.run.app;
            let s = &app.srcs[si];
            let found = if s.mask.as_bytes()[r.start] == b'[' {
                match php::array_items(&s.mask, r.start).as_slice() {
                    [(_, c), (_, m)] => self
                        .class_arg(si, c)
                        .zip(app.value(s, m))
                        .map(|(c, m)| Handler::Action(c, m)),
                    _ => None,
                }
            } else if let Some(v) = php::literal(s.text(r)) {
                string_action(&v, g)
            } else if let Some(c) = self.class_arg(si, r) {
                Some(Handler::Action(c, "__invoke".into()))
            } else {
                closure_body(&s.mask, r).map(|body| Handler::Closure(si, body))
            };
            found.unwrap_or_else(|| {
                let text = collapse(s.text(r));
                let message = format!(
                    "Route action {text} is not read; its handler, checks and view are unknown."
                );
                self.run.at(si, r.start, message);
                Handler::Unread
            })
        }

        fn verb(&mut self, si: usize, k: usize, term: &CallAt, after: &[CallAt], g: &Group) {
            let app = self.run.app;
            let s = &app.srcs[si];
            let args = &term.args;
            let (methods, at) = match term.name.as_str() {
                "match" => (
                    self.run
                        .list(si, &args[..1.min(args.len())])
                        .iter()
                        .map(|m| m.to_uppercase())
                        .collect(),
                    1,
                ),
                "any" => (vec![ANY_METHOD.to_string()], 0),
                "view" => (vec!["GET".to_string()], 0),
                v => (vec![v.to_uppercase()], 0),
            };
            let Some(path_arg) = args.get(at) else { return };
            let Some(path) = app.value(s, path_arg) else {
                let text = collapse(s.text(path_arg));
                return self.run.at(
                    si,
                    k,
                    format!("Route path {text} is not read literally; the route is skipped."),
                );
            };
            let handler = match (term.name.as_str(), args.get(at + 1)) {
                ("view", Some(v)) => app.value(s, v).map_or(Handler::Unread, Handler::View),
                (_, Some(a)) => self.handler(si, a, g),
                (_, None) => Handler::Unread,
            };
            let route = RouteDef {
                methods,
                path: join_path(&g.prefix, &path),
                name: None,
                mw: g.mw.clone(),
                without: g.without.clone(),
                handler,
                file: s.rel.clone(),
                line: s.line(k),
            };
            let mut routes = vec![route];
            self.after(si, after, &mut routes, g);
            self.routes.extend(routes);
        }

        /// Calls after the verb: `name`, `middleware`, `withoutMiddleware`, `can`.
        fn after(&mut self, si: usize, calls: &[CallAt], routes: &mut [RouteDef], g: &Group) {
            let app = self.run.app;
            let s = &app.srcs[si];
            for c in calls {
                let first = c.args.first();
                match c.name.as_str() {
                    "name" => {
                        let name = first
                            .and_then(|r| app.value(s, r))
                            .map(|n| format!("{}{n}", g.name));
                        routes.iter_mut().for_each(|r| r.name = name.clone());
                    }
                    "middleware" => {
                        let mws = self.run.mws(si, &c.args);
                        routes
                            .iter_mut()
                            .for_each(|r| r.mw.extend(mws.iter().cloned()));
                    }
                    "withoutMiddleware" => {
                        let names = self.names(si, &c.args);
                        routes
                            .iter_mut()
                            .for_each(|r| r.without.extend(names.iter().cloned()));
                    }
                    "can" => {
                        let mw = match first.and_then(|r| app.value(s, r)) {
                            Some(a) => Mw::Name(format!("can:{a}")),
                            None => Mw::Unread(first.map_or("can".into(), |r| collapse(s.text(r)))),
                        };
                        routes.iter_mut().for_each(|r| r.mw.push(mw.clone()));
                    }
                    _ => {}
                }
            }
        }

        fn resource(&mut self, si: usize, k: usize, term: &CallAt, after: &[CallAt], g: &Group) {
            let app = self.run.app;
            let s = &app.srcs[si];
            let name = term.args.first().and_then(|r| app.value(s, r));
            let class = term.args.get(1).and_then(|r| self.class_arg(si, r));
            let (Some(name), Some(class)) = (name, class) else {
                let message = format!(
                    "Route::{} is not read literally; its routes are unknown.",
                    term.name
                );
                return self.run.at(si, k, message);
            };
            let (only, except) = self.resource_filters(si, term, after);
            let (base, param) = resource_path(&g.prefix, &name);
            let mut routes = vec![];
            for (action, methods, tail) in RESOURCE {
                let api_only = term.name == "apiResource" && matches!(action, "create" | "edit");
                if api_only || !applies(action, &only, &except) {
                    continue;
                }
                routes.push(RouteDef {
                    methods: methods.iter().map(|m| m.to_string()).collect(),
                    path: join_path(&base, &tail.replace("{}", &param)),
                    name: Some(format!("{}{name}.{action}", g.name)),
                    mw: g.mw.clone(),
                    without: g.without.clone(),
                    handler: Handler::Action(class.clone(), action.into()),
                    file: s.rel.clone(),
                    line: s.line(k),
                });
            }
            let calls: Vec<CallAt> = (after.iter().filter(|c| c.name != "name"))
                .map(|c| CallAt {
                    name: c.name.clone(),
                    args: c.args.clone(),
                })
                .collect();
            self.after(si, &calls, &mut routes, g);
            self.routes.extend(routes);
        }

        /// `only`/`except` of a resource: options array or chained calls.
        fn resource_filters(
            &mut self,
            si: usize,
            term: &CallAt,
            after: &[CallAt],
        ) -> (Option<Vec<String>>, Vec<String>) {
            let s = &self.run.app.srcs[si];
            let (mut only, mut except) = (None, vec![]);
            let options = term
                .args
                .get(2)
                .map(|o| php::array_items(&s.mask, o.start))
                .unwrap_or_default();
            let options = (options.into_iter())
                .filter_map(|(k, v)| Some((php::literal(s.text(&k?))?, vec![v])));
            let chained = after.iter().map(|c| (c.name.clone(), c.args.clone()));
            for (key, args) in options.chain(chained) {
                match key.as_str() {
                    "only" => only = Some(self.run.list(si, &args)),
                    "except" => except = self.run.list(si, &args),
                    _ => {}
                }
            }
            (only, except)
        }
    }

    /// `'C@m'`, or `'m'` under `Route::controller(C::class)`.
    fn string_action(v: &str, g: &Group) -> Option<Handler> {
        match (v.split_once('@'), &g.controller) {
            (Some((c, m)), _) => {
                let c = match (g.namespace.as_str(), c.strip_prefix('\\')) {
                    (_, Some(abs)) => abs.to_string(),
                    ("", None) => c.to_string(),
                    (ns, None) => format!("{ns}\\{c}"),
                };
                Some(Handler::Action(c, m.to_string()))
            }
            (None, Some(c)) => Some(Handler::Action(c.clone(), v.to_string())),
            (None, None) => None,
        }
    }

    /// The base path of resource `name` (`a.b` nests) and its `{param}`.
    fn resource_path(prefix: &str, name: &str) -> (String, String) {
        let parts: Vec<&str> = name.split('.').collect();
        let mut base = prefix.to_string();
        for (i, p) in parts.iter().enumerate() {
            base = join_path(&base, p);
            if i + 1 < parts.len() {
                base = join_path(&base, &format!("{{{}}}", singular(p)));
            }
        }
        (
            base,
            format!(
                "{{{}}}",
                singular(parts.last().copied().unwrap_or_default())
            ),
        )
    }
}

/// Access of routes: middleware, controller middleware and checks in bodies.
mod controllers {
    use super::routes::{chain_after, closure_body};
    use super::*;

    impl Run<'_> {
        /// The access of route `r`; the literal views its action returns go to `views`.
        pub(super) fn access(&mut self, r: &RouteDef, views: &mut Vec<String>) -> AccessState {
            let mut acc = Acc::default();
            self.route_middleware(r, &mut acc);
            match &r.handler {
                Handler::Action(class, method) => {
                    self.controller(class, method, r, &mut acc, views)
                }
                Handler::Closure(si, body) => {
                    self.checks(*si, body, &mut acc);
                    views.extend(self.views(*si, body));
                }
                Handler::View(v) => views.push(v.clone()),
                Handler::Unread => {}
            }
            match all(acc.conds) {
                Some(c) => AccessState::Protected(c),
                None if acc.open || acc.removed => AccessState::Open,
                None => AccessState::Undeclared,
            }
        }

        fn route_middleware(&mut self, r: &RouteDef, acc: &mut Acc) {
            for m in &r.mw {
                match m {
                    Mw::Name(n) if self.removed(n, &r.without) => {
                        let mut probe = Acc::default();
                        self.app.middleware(n, &mut probe, 0);
                        acc.removed |= !probe.conds.is_empty();
                    }
                    Mw::Name(n) if !self.app.middleware(n, acc, 0) => {
                        self.unknown_middleware(n, &r.file, r.line)
                    }
                    Mw::Name(_) => {}
                    Mw::Unread(text) => acc.conds.push(Condition::Unknown(text.clone())),
                }
            }
        }

        fn unknown_middleware(&mut self, name: &str, file: &str, line: u32) {
            let base = name.split(':').next().unwrap_or(name).to_string();
            if self.noted.insert(base.clone()) {
                let message = format!(
                    "Middleware `{base}` has no access meaning by rule; list it in [views.access] if it guards access."
                );
                note(&mut self.out, file, line, message);
            }
        }

        /// Whether `withoutMiddleware` names `name` (by name, alias or class).
        fn removed(&self, name: &str, without: &[String]) -> bool {
            let base = name.split(':').next().unwrap_or(name);
            let aliases = &self.app.mw.aliases;
            without.iter().any(|w| {
                w == base
                    || aliases.get(base).is_some_and(|c| c == w)
                    || aliases.get(w.as_str()).is_some_and(|c| c == base)
            })
        }

        fn controller(
            &mut self,
            class: &str,
            method: &str,
            r: &RouteDef,
            acc: &mut Acc,
            views: &mut Vec<String>,
        ) {
            let app = self.app;
            let Some((ci, c)) = app.class(class) else {
                let message = format!(
                    "Controller {class} not found in the scan; its checks and views are unknown."
                );
                return self.gap(&r.file, r.line, message);
            };
            self.constructor(*ci, c, method, acc);
            self.static_middleware(*ci, c, method, acc);
            let Some(m) = c.methods.iter().find(|m| m.name == method) else {
                let message = format!(
                    "Controller method {}.{method} not found; its checks and views are unknown.",
                    c.name
                );
                return self.gap(&r.file, r.line, message);
            };
            if let Some(body) = &m.body {
                self.checks(*ci, body, acc);
                views.extend(self.views(*ci, body));
            }
        }

        /// `$this->middleware(…)` and `$this->authorizeResource(…)` in `__construct`.
        fn constructor(&mut self, ci: usize, c: &Class, method: &str, acc: &mut Acc) {
            let s = &self.app.srcs[ci];
            let Some(body) = c
                .methods
                .iter()
                .find(|m| m.name == "__construct")
                .and_then(|m| m.body.clone())
            else {
                return;
            };
            let b = s.mask.as_bytes();
            for pat in ["$this->middleware(", "$this->authorizeResource("] {
                let mut from = body.start;
                while let Some(k) = find(&b[..body.end], from, pat.as_bytes()) {
                    from = k + 1;
                    let Some((args, close)) = php::args(&s.mask, k + pat.len() - 1) else {
                        continue;
                    };
                    if args.is_empty() {
                        continue;
                    }
                    if pat.ends_with("authorizeResource(") {
                        let ability = access::resource_ability(method);
                        acc.conds
                            .extend(ability.map(|a| Condition::Policy(a.into())));
                    } else {
                        self.constructor_middleware(ci, &args, close, method, acc);
                    }
                }
            }
        }

        /// One `$this->middleware(x, ['only' => …])->except(…)` call.
        fn constructor_middleware(
            &mut self,
            ci: usize,
            args: &[Range<usize>],
            close: usize,
            method: &str,
            acc: &mut Acc,
        ) {
            let s = &self.app.srcs[ci];
            let (mut only, mut except) = (None, vec![]);
            let options = args
                .get(1)
                .map(|o| php::array_items(&s.mask, o.start))
                .unwrap_or_default();
            let options = (options.into_iter())
                .filter_map(|(k, v)| Some((php::literal(s.text(&k?))?, vec![v])));
            let chained = chain_after(&s.mask, close + 1)
                .into_iter()
                .map(|c| (c.name, c.args));
            for (key, list) in options.chain(chained) {
                match key.as_str() {
                    "only" => only = Some(self.list(ci, &list)),
                    "except" => except = self.list(ci, &list),
                    _ => {}
                }
            }
            if !applies(method, &only, &except) {
                return;
            }
            match closure_body(&s.mask, &args[0]) {
                Some(cb) => self.checks(ci, &cb, acc),
                None => self.guards(ci, &args[..1], acc),
            }
        }

        /// Laravel 11 `public static function middleware()`: names and `new Middleware(…)`.
        fn static_middleware(&mut self, ci: usize, c: &Class, method: &str, acc: &mut Acc) {
            let s = &self.app.srcs[ci];
            let b = s.mask.as_bytes();
            let Some(body) = c
                .methods
                .iter()
                .find(|m| m.name == "middleware")
                .and_then(|m| m.body.clone())
            else {
                return;
            };
            let open = (php::words(&s.mask, body, "return").first())
                .and_then(|&k| next_nonws(b, k + 6))
                .filter(|&o| b[o] == b'[');
            for (_, item) in open
                .map(|o| php::array_items(&s.mask, o))
                .unwrap_or_default()
            {
                let text = &s.mask[item.clone()];
                let rest = text
                    .strip_prefix("new")
                    .map(str::trim_start)
                    .filter(|t| t.starts_with("Middleware"));
                let Some(rest) = rest else {
                    self.guards(ci, std::slice::from_ref(&item), acc);
                    continue;
                };
                let open = item.start + text.len() - rest.len() + "Middleware".len();
                let Some((args, _)) = next_nonws(b, open).and_then(|o| php::args(&s.mask, o))
                else {
                    continue;
                };
                let (mut only, mut except) = (None, vec![]);
                for a in args.iter().skip(1) {
                    let Some((name, _)) = s.mask[a.clone()].split_once(':') else {
                        continue;
                    };
                    let list =
                        self.list(ci, &[php::trim(&s.mask, a.start + name.len() + 1..a.end)]);
                    match name.trim() {
                        "only" => only = Some(list),
                        "except" => except = list,
                        _ => {}
                    }
                }
                if applies(method, &only, &except) {
                    self.guards(ci, &args[..1.min(args.len())], acc);
                }
            }
        }

        /// Controller-level middleware of arguments.
        fn guards(&mut self, si: usize, args: &[Range<usize>], acc: &mut Acc) {
            for m in self.mws(si, args) {
                match m {
                    Mw::Name(n) if !self.app.middleware(&n, acc, 0) => {
                        let s = &self.app.srcs[si];
                        let (rel, line) = (s.rel.clone(), s.line(args[0].start));
                        self.unknown_middleware(&n, &rel, line);
                    }
                    Mw::Name(_) => {}
                    Mw::Unread(text) => acc.conds.push(Condition::Unknown(text)),
                }
            }
        }

        /// Literal values of an array argument or of variadic arguments.
        pub(super) fn list(&self, si: usize, args: &[Range<usize>]) -> Vec<String> {
            let s = &self.app.srcs[si];
            (args.iter())
                .flat_map(|a| match s.mask.as_bytes()[a.start] {
                    b'[' => php::array_items(&s.mask, a.start)
                        .into_iter()
                        .map(|(_, v)| v)
                        .collect(),
                    _ => vec![a.clone()],
                })
                .filter_map(|r| self.app.value(s, &r))
                .collect()
        }

        /// Middleware of arguments: names, arrays, `X::class`, values of the scan.
        pub(super) fn mws(&mut self, si: usize, args: &[Range<usize>]) -> Vec<Mw> {
            let app = self.app;
            let s = &app.srcs[si];
            let mut items = vec![];
            for a in args {
                match s.mask.as_bytes()[a.start] {
                    b'[' => items.extend(
                        php::array_items(&s.mask, a.start)
                            .into_iter()
                            .map(|(_, v)| v),
                    ),
                    _ => items.push(a.clone()),
                }
            }
            let mut out = vec![];
            for r in items {
                match app.value(s, &r).or_else(|| member(s, &r)) {
                    Some(n) => out.push(Mw::Name(n)),
                    None => {
                        let text = collapse(s.text(&r));
                        let message = format!(
                            "Middleware {text} is not read literally; the access of its routes is unknown."
                        );
                        self.at(si, r.start, message);
                        out.push(Mw::Unread(text));
                    }
                }
            }
            out
        }

        /// Checks at the top of a body: `authorize`, `Gate::authorize`, listed functions.
        fn checks(&mut self, si: usize, body: &Range<usize>, acc: &mut Acc) {
            let app = self.app;
            let s = &app.srcs[si];
            for (k, name) in idents(&s.mask, body.start, body.end) {
                let Some((open, listed, top)) = self.check_at(si, body, k, name) else {
                    continue;
                };
                if !top {
                    let message = format!(
                        "Check `{name}` inside a block or condition; it is not counted as the endpoint's access."
                    );
                    note(&mut self.out, &s.rel, s.line(k), message);
                    continue;
                }
                let args = php::args(&s.mask, open).map(|a| a.0).unwrap_or_default();
                let arg = match args
                    .first()
                    .map(|r| app.value(s, r).ok_or_else(|| collapse(s.text(r))))
                {
                    Some(Ok(v)) => vec![v],
                    Some(Err(text)) => {
                        let message = format!(
                            "Check `{name}` with argument {text} is not read literally; its condition is unknown."
                        );
                        self.at(si, k, message);
                        acc.conds.push(Condition::Unknown(text));
                        continue;
                    }
                    None => vec![],
                };
                match listed {
                    Some(m) => acc.add(meaning(m, name, &arg)),
                    None => {
                        acc.conds
                            .push(arg.first().map_or(Condition::Unknown(name.into()), |a| {
                                Condition::Policy(a.clone())
                            }))
                    }
                }
            }
        }

        /// Whether identifier `name` at `k` is a check call: its `(`, the
        /// listed meaning (none for `authorize`) and whether it is a statement
        /// at the top of the body.
        fn check_at(
            &self,
            si: usize,
            body: &Range<usize>,
            k: usize,
            name: &str,
        ) -> Option<(usize, Option<&AccessMeaning>, bool)> {
            let app = self.app;
            let s = &app.srcs[si];
            let b = s.mask.as_bytes();
            let open = next_nonws(b, k + name.len()).filter(|&o| o < body.end && b[o] == b'(')?;
            let before = &s.mask[body.start..k];
            let receiver = ["$this->", "self::", "static::", "Gate::"]
                .into_iter()
                .find(|r| before.ends_with(r));
            if receiver.is_none() && (before.ends_with("->") || before.ends_with("::")) {
                return None;
            }
            let ability = name == "authorize" && matches!(receiver, Some("$this->" | "Gate::"));
            let listed = (receiver != Some("Gate::"))
                .then(|| app.access.get(name))
                .flatten();
            if !ability && listed.is_none() {
                return None;
            }
            let start = k - receiver.map_or(0, str::len);
            let after_statement = prev_nonws(b, start)
                .is_none_or(|p| p < body.start || matches!(b[p], b';' | b'{' | b'}'));
            Some((
                open,
                listed,
                after_statement && depth(b, body.start, start) == 0,
            ))
        }

        /// Literal view names a body renders (`view('x')`, `View::make('x')`).
        fn views(&mut self, si: usize, body: &Range<usize>) -> Vec<String> {
            let app = self.app;
            let s = &app.srcs[si];
            let b = s.mask.as_bytes();
            let mut out = vec![];
            for (k, name) in idents(&s.mask, body.start, body.end) {
                let before = &s.mask[body.start..k];
                let call = (name == "view" && !before.ends_with("->") && !before.ends_with("::"))
                    || (name == "make" && before.ends_with("View::"));
                let Some(open) = next_nonws(b, k + name.len()).filter(|&o| call && b[o] == b'(')
                else {
                    continue;
                };
                let Some(first) = php::args(&s.mask, open).and_then(|a| a.0.into_iter().next())
                else {
                    continue;
                };
                match app.value(s, &first) {
                    Some(v) => out.push(v),
                    None => {
                        let text = collapse(s.text(&first));
                        let message = format!(
                            "View name {text} is not read literally; the screen's template is unknown."
                        );
                        self.at(si, k, message);
                    }
                }
            }
            out
        }
    }
}

/// Blade templates: links, includes, layouts and the conditions around them.
mod blade {
    use super::routes::split_op;
    use super::*;

    /// A navigation target of a link or form.
    #[derive(Clone, Debug)]
    pub(super) enum Dest {
        Path(String),
        Name(String),
    }

    pub(super) struct Link {
        pub(super) line: u32,
        pub(super) cond: Option<Condition>,
        /// Upper-case form method; none for `<a>`.
        pub(super) method: Option<String>,
        /// None when the value is not readable.
        pub(super) dest: Option<Dest>,
        /// The attribute value as written.
        pub(super) source: String,
        pub(super) trigger: Option<String>,
    }

    #[derive(Default)]
    pub(super) struct Tpl {
        pub(super) rel: String,
        pub(super) extends: Vec<(String, u32)>,
        pub(super) includes: Vec<(String, Option<Condition>, u32)>,
        pub(super) links: Vec<Link>,
    }

    /// One open `@if`-like block: the recognised conditions of its earlier
    /// branches, the own one of the branch in force, and what holds there.
    struct Frame {
        seen: Vec<Condition>,
        own: Option<Condition>,
        current: Option<Condition>,
    }

    /// The parse of one template.
    struct Parse {
        src: String,
        tpl: Tpl,
        frames: Vec<Frame>,
        /// Index of the link of the open `<form>`.
        form: Option<usize>,
    }

    impl Parse {
        fn now(&self) -> Option<Condition> {
            all(self
                .frames
                .iter()
                .filter_map(|f| f.current.clone())
                .collect())
        }

        fn line(&self, i: usize) -> u32 {
            line(&self.src, i)
        }

        fn spoof(&mut self, method: Option<String>) {
            if let (Some(f), Some(m)) = (self.form, method) {
                self.tpl.links[f].method = Some(m.to_uppercase());
            }
        }

        /// `{{ … }}` or `{!! … !!}` at `i`: its end; reads `method_field()`.
        fn echo(&mut self, i: usize) -> usize {
            let b = self.src.as_bytes();
            let (open, close): (usize, &[u8]) = if b[i + 1] == b'{' {
                (2, b"}}")
            } else {
                (3, b"!!}")
            };
            let end = find(b, i + open, close).unwrap_or(b.len());
            let method = call_args(&self.src[i + open..end], "method_field")
                .and_then(|m| m.first().and_then(|a| php::literal(a)));
            self.spoof(method);
            end + close.len()
        }

        fn branch(&mut self, own: Option<Condition>) {
            if let Some(f) = self.frames.last_mut() {
                f.seen.extend(f.own.take());
                let negated = f.seen.iter().cloned().map(not).collect::<Vec<_>>();
                f.current = and(all(negated), own.clone());
                f.own = own;
            }
        }
    }

    impl Run<'_> {
        /// Links, includes, layouts and conditions of one Blade template.
        pub(super) fn template(&mut self, rel: &str, text: &str) -> Tpl {
            let src = blank_regions(text, |b, i| {
                let (open, close): (&[u8], &[u8]) = match () {
                    _ if b[i..].starts_with(b"{{--") => (b"{{--", b"--}}"),
                    _ if b[i..].starts_with(b"<!--") => (b"<!--", b"-->"),
                    _ => return None,
                };
                let end = find(b, i + open.len(), close).map_or(b.len(), |p| p + close.len());
                Some((i, end, end))
            });
            let tpl = Tpl {
                rel: rel.to_string(),
                ..Tpl::default()
            };
            let mut p = Parse {
                src,
                tpl,
                frames: vec![],
                form: None,
            };
            let mut i = 0;
            while i < p.src.len() {
                let b = p.src.as_bytes();
                i = if b[i..].starts_with(b"{{") || b[i..].starts_with(b"{!!") {
                    p.echo(i)
                } else if b[i] == b'@' && !(i > 0 && (is_id(b[i - 1]) || b[i - 1] == b'@')) {
                    self.directive(&mut p, i)
                } else if b[i] == b'<' {
                    self.tag(&mut p, i)
                } else {
                    i + 1
                };
            }
            p.tpl
        }

        /// The directive at `i`; returns where parsing goes on.
        fn directive(&mut self, p: &mut Parse, i: usize) -> usize {
            let b = p.src.as_bytes();
            let ne = ident_end(b, i + 1);
            let name = p.src[i + 1..ne].to_string();
            let open = (ne..b.len())
                .find(|&j| b[j] != b' ')
                .filter(|&j| b[j] == b'(' && takes_args(&name));
            let (arg, next) = match open.and_then(|o| Some((o, php::close(b, o)?))) {
                Some((o, z)) => (Some(p.src[o + 1..z].to_string()), z + 1),
                None => (None, ne),
            };
            let line = p.line(i);
            let args: Vec<String> = arg
                .as_deref()
                .map(split_args)
                .unwrap_or_default()
                .into_iter()
                .map(String::from)
                .collect();
            let arg = arg.unwrap_or_default();
            match name.as_str() {
                "extends" => match args.first().and_then(|a| php::literal(a)) {
                    Some(v) => p.tpl.extends.push((v, line)),
                    None => self.gap(
                        &p.tpl.rel,
                        line,
                        format!("Layout {arg} is not read literally; its links are unknown."),
                    ),
                },
                "include" | "includeIf" | "each" | "component" | "includeWhen"
                | "includeUnless" => {
                    self.include(p, &name, &args, line);
                }
                "includeFirst" => {
                    let message =
                        "@includeFirst is not read; the links of its templates are unknown."
                            .to_string();
                    self.gap(&p.tpl.rel, line, message);
                }
                "method" => p.spoof(args.first().and_then(|a| php::literal(a))),
                _ => self.block_directive(p, &name, &arg, &args, open.is_some()),
            }
            next.max(i + 1)
        }

        /// `@if`-like directives: open, branch or close a [`Frame`].
        fn block_directive(
            &self,
            p: &mut Parse,
            name: &str,
            arg: &str,
            args: &[String],
            parens: bool,
        ) {
            let open = |p: &mut Parse, current: Option<Condition>| {
                p.frames.push(Frame {
                    seen: vec![],
                    own: current.clone(),
                    current,
                });
            };
            match name {
                "if" | "unless" | "can" | "cannot" | "canany" | "auth" | "guest" | "isset" => {
                    open(p, self.directive_cond(name, arg, args));
                }
                "empty" if parens => open(p, None),
                "elseif" => p.branch(self.expr_cond(arg)),
                "else" => p.branch(None),
                "elsecan" | "elsecannot" | "elseauth" | "elseguest" => {
                    p.branch(self.directive_cond(&name[4..], arg, args))
                }
                "endif" | "endunless" | "endcan" | "endcannot" | "endcanany" | "endauth"
                | "endguest" | "endisset" | "endempty" => drop(p.frames.pop()),
                _ => {}
            }
        }

        /// `@include('x')` and its variants (`@includeWhen(cond, 'x')`).
        fn include(&mut self, p: &mut Parse, name: &str, args: &[String], line: u32) {
            let conditional = name.starts_with("includeWhen") || name.starts_with("includeUnless");
            let when = conditional
                .then(|| args.first().and_then(|e| self.expr_cond(e)))
                .flatten();
            let when = if name == "includeUnless" {
                when.map(not)
            } else {
                when
            };
            match args
                .get(usize::from(conditional))
                .and_then(|a| php::literal(a))
            {
                Some(v) => p.tpl.includes.push((v, and(p.now(), when), line)),
                None => {
                    let text = args.join(", ");
                    self.gap(
                        &p.tpl.rel,
                        line,
                        format!(
                            "Included template {text} is not read literally; its links are unknown."
                        ),
                    );
                }
            }
        }

        /// The tag at `i`; returns where parsing goes on.
        fn tag(&mut self, p: &mut Parse, i: usize) -> usize {
            let b = p.src.as_bytes();
            let tag = (i + 1..b.len())
                .find(|&k| !(is_id(b[k]) || matches!(b[k], b'-' | b'.' | b':')))
                .unwrap_or(b.len());
            let name = p.src[i + 1..tag].to_ascii_lowercase();
            if name.is_empty() {
                if p.src[i..].starts_with("</form") {
                    p.form = None;
                }
                return i + 1;
            }
            let end = tag_end(b, i);
            let attrs = attributes(&p.src[tag..end]);
            let attr = |n: &str| attrs.iter().find(|(k, _)| k == n).map(|(_, v)| v.clone());
            let line = p.line(i);
            match name.as_str() {
                "a" => {
                    if let Some(href) = attr("href") {
                        let inner_end = find(b, end, b"</a").unwrap_or(end);
                        let text = p.src.get(end + 1..inner_end).unwrap_or("");
                        let trigger = trigger(&attrs).or_else(|| label(text));
                        let link = self.link(&href, None, p.now(), line, trigger);
                        p.tpl.links.extend(link);
                    }
                }
                "form" => {
                    let method = attr("method").unwrap_or("GET".into()).to_uppercase();
                    let link = attr("action")
                        .and_then(|a| self.link(&a, Some(method), p.now(), line, trigger(&attrs)));
                    p.form = link.is_some().then_some(p.tpl.links.len());
                    p.tpl.links.extend(link);
                }
                "input" if attr("name").as_deref() == Some("_method") => p.spoof(attr("value")),
                n if n.starts_with("x-")
                    && !matches!(n, "x-slot" | "x-dynamic-component")
                    && !n.starts_with("x-slot:") =>
                {
                    let view = format!("components.{}", &p.src[i + 3..tag]);
                    if self.app.templates.contains_key(&view) {
                        p.tpl.includes.push((view, p.now(), line));
                    } else {
                        let message = format!(
                            "Component <{}> has no template {view}; its links are unknown.",
                            &p.src[i + 1..tag]
                        );
                        self.gap(&p.tpl.rel, line, message);
                    }
                }
                _ => {}
            }
            end.max(i + 1)
        }

        fn link(
            &self,
            value: &str,
            method: Option<String>,
            cond: Option<Condition>,
            line: u32,
            trigger: Option<String>,
        ) -> Option<Link> {
            let dest = self.dest(value)?;
            Some(Link {
                line,
                cond,
                method,
                dest,
                source: value.trim().to_string(),
                trigger,
            })
        }

        /// The target of an `href`/`action` value; `None` for external links and
        /// fragments, `Some(None)` when it is not readable.
        fn dest(&self, value: &str) -> Option<Option<Dest>> {
            let v = value.trim();
            let lower = v.to_ascii_lowercase();
            let external = [
                "javascript:",
                "mailto:",
                "tel:",
                "http:",
                "https:",
                "//",
                "data:",
            ];
            if v.is_empty() || v.starts_with('#') || external.iter().any(|p| lower.starts_with(p)) {
                return None;
            }
            let pieces = echo_pieces(v);
            if let [(true, e)] = pieces.as_slice()
                && let Some(args) = call_args(e, "route")
            {
                return Some(
                    args.first()
                        .and_then(|a| self.app.value_of(a))
                        .map(Dest::Name),
                );
            }
            let Some(path) = self.path_of(&pieces) else {
                return Some(None);
            };
            let segment_ok = |s: &str| {
                !s.contains(['{', '}'])
                    || (s.starts_with('{') && s.ends_with('}') && s.matches('{').count() == 1)
            };
            Some(
                (path.starts_with('/') && path.split('/').all(segment_ok))
                    .then_some(Dest::Path(path)),
            )
        }

        /// The path of literal and echo pieces, query and fragment dropped;
        /// none when a piece is not readable.
        fn path_of(&self, pieces: &[(bool, &str)]) -> Option<String> {
            let mut path = String::new();
            for (echo, t) in pieces {
                let piece = match (echo, call_args(t, "url")) {
                    (false, _) => t.to_string(),
                    (true, Some(args)) => {
                        let [arg] = args.as_slice() else { return None };
                        let p = self.concat_path(arg)?;
                        if p.starts_with('/') {
                            p
                        } else {
                            format!("/{p}")
                        }
                    }
                    (true, None) if call_args(t, "route").is_some() => return None,
                    (true, None) => format!("{{{}}}", param_name(t)),
                };
                if let Some(q) = piece.find(['?', '#']) {
                    path.push_str(&piece[..q]);
                    break;
                }
                path.push_str(&piece);
            }
            Some(path)
        }

        /// `'/books/' . $book->id . '/edit'` as `/books/{id}/edit`.
        fn concat_path(&self, expr: &str) -> Option<String> {
            let m = php::mask(expr);
            let mut out = String::new();
            for (n, r) in split_op(&m, 0..m.len(), ".").into_iter().enumerate() {
                let piece = &expr[r];
                let text = match self.app.value_of(piece) {
                    Some(v) => v,
                    None if piece.starts_with('"') => interpolated(piece)?,
                    // A path that starts with an expression is no path the rules can read.
                    None if n > 0
                        && (piece.starts_with('$')
                            || piece.contains("->")
                            || piece.contains('(')) =>
                    {
                        format!("{{{}}}", param_name(piece))
                    }
                    None => return None,
                };
                out.push_str(&text);
            }
            Some(out)
        }

        fn directive_cond(&self, name: &str, arg: &str, args: &[String]) -> Option<Condition> {
            let ability = |a: &str| {
                self.app
                    .value_of(a)
                    .map_or(Condition::Unknown(collapse(a)), Condition::Policy)
            };
            match name {
                "if" => self.expr_cond(arg),
                "unless" => self.expr_cond(arg).map(not),
                "can" => args.first().map(|a| ability(a)),
                "cannot" => args.first().map(|a| not(ability(a))),
                "canany" => {
                    let list = args.first().map(|a| {
                        a.trim()
                            .trim_start_matches('[')
                            .trim_end_matches(']')
                            .to_string()
                    });
                    let list = list.as_deref().map(split_args).unwrap_or_default();
                    Some(any(list.iter().map(|a| ability(a)).collect()))
                }
                "auth" => Some(Condition::Authenticated),
                "guest" => Some(not(Condition::Authenticated)),
                _ => None,
            }
        }

        /// The access condition of an `@if` expression, when some part of it is
        /// a recognised check; the other parts stay `unknown`.
        pub(super) fn expr_cond(&self, e: &str) -> Option<Condition> {
            let (c, known) = self.expr(e.trim());
            known.then_some(c)
        }

        fn expr(&self, e: &str) -> (Condition, bool) {
            let m = php::mask(e);
            if e.starts_with('(') && matching(m.as_bytes(), 0) == Some(e.len() - 1) {
                return self.expr(e[1..e.len() - 1].trim());
            }
            for (op, either) in [
                ("||", true),
                (" or ", true),
                ("&&", false),
                (" and ", false),
            ] {
                let parts = split_op(&m, 0..m.len(), op);
                if parts.len() > 1 {
                    let items: Vec<(Condition, bool)> =
                        parts.into_iter().map(|r| self.expr(&e[r])).collect();
                    let known = items.iter().any(|i| i.1);
                    let conds = items.into_iter().map(|i| i.0).collect();
                    return (
                        if either {
                            Condition::Any(conds)
                        } else {
                            Condition::All(conds)
                        },
                        known,
                    );
                }
            }
            if let Some(rest) = e.strip_prefix('!') {
                let (c, known) = self.expr(rest.trim());
                return (not(c), known);
            }
            self.atom(e, &m)
        }

        /// One operand of an `@if` expression.
        fn atom(&self, e: &str, m: &str) -> (Condition, bool) {
            let flat: String = e.chars().filter(|c| !c.is_whitespace()).collect();
            match flat.as_str() {
                "auth()->check()" | "Auth::check()" => return (Condition::Authenticated, true),
                "auth()->guest()" | "Auth::guest()" => {
                    return (not(Condition::Authenticated), true);
                }
                _ => {}
            }
            let unknown = (Condition::Unknown(collapse(e)), false);
            let Some(open) = (e.ends_with(')'))
                .then(|| matching_back(m.as_bytes(), e.len() - 1))
                .flatten()
            else {
                return unknown;
            };
            let callee: String = e[..open].chars().filter(|c| !c.is_whitespace()).collect();
            let name_at = callee.len() - callee.bytes().rev().take_while(|&c| is_id(c)).count();
            let (receiver, name) = callee.split_at(name_at);
            let args = split_args(&e[open + 1..e.len() - 1]);
            let first = args
                .first()
                .map(|a| self.app.value_of(a).ok_or_else(|| collapse(a)));
            let ability = || match &first {
                Some(Ok(a)) => Condition::Policy(a.clone()),
                Some(Err(t)) => Condition::Unknown(t.clone()),
                None => Condition::Unknown(collapse(e)),
            };
            let user = [
                "auth()->user()->",
                "Auth::user()->",
                "auth()->user()?->",
                "Auth::user()?->",
            ];
            match (receiver, name) {
                ("" | "\\", n) if self.app.access.contains_key(n) => {
                    listed(&self.app.access[n], n, &first).map_or(unknown, |c| (c, true))
                }
                ("Gate::" | "\\Gate::", "allows" | "check") => (ability(), true),
                ("Gate::" | "\\Gate::", "denies") => (not(ability()), true),
                (r, "can") if user.contains(&r) => (ability(), true),
                _ => unknown,
            }
        }
    }

    /// A listed function's condition with its first argument (read or not).
    fn listed(
        m: &AccessMeaning,
        name: &str,
        first: &Option<Result<String, String>>,
    ) -> Option<Condition> {
        match first {
            Some(Ok(a)) => meaning(m, name, std::slice::from_ref(a)),
            Some(Err(t)) => Some(Condition::Unknown(t.clone())),
            None => meaning(m, name, &[]),
        }
    }

    /// Directives whose `(…)` belongs to them.
    fn takes_args(name: &str) -> bool {
        !matches!(
            name,
            "else"
                | "endif"
                | "endunless"
                | "endcan"
                | "endcannot"
                | "endcanany"
                | "endauth"
                | "endguest"
                | "endisset"
                | "endempty"
                | "endforeach"
                | "endsection"
                | "endforelse"
                | "endfor"
                | "endwhile"
                | "endpush"
                | "show"
                | "stop"
                | "append"
                | "overwrite"
                | "csrf"
                | "endphp"
                | "endswitch"
                | "break"
                | "continue"
                | "default"
                | "parent"
        )
    }

    /// Top-level arguments of a PHP argument list (text between the parentheses).
    fn split_args(text: &str) -> Vec<&str> {
        let m = format!("({})", php::mask(text));
        let items = php::args(&m, 0).map(|(items, _)| items).unwrap_or_default();
        items
            .into_iter()
            .map(|r| &text[r.start - 1..r.end - 1])
            .collect()
    }

    /// The arguments when `expr` is exactly a call of `name`.
    fn call_args<'e>(expr: &'e str, name: &str) -> Option<Vec<&'e str>> {
        let e = expr.trim();
        let rest = e.strip_prefix(name)?.trim_start();
        let open = e.len() - rest.len();
        let close = php::close(e.as_bytes(), open)?;
        (close == e.len() - 1).then(|| split_args(&e[open + 1..close]))
    }

    /// A literal value as `(false, text)` and `{{ … }}`/`{!! … !!}` echoes as `(true, expression)`.
    fn echo_pieces(v: &str) -> Vec<(bool, &str)> {
        let mut out = vec![];
        let mut rest = v;
        while let Some(start) = rest.find("{{").into_iter().chain(rest.find("{!!")).min() {
            let (open, close) = if rest[start..].starts_with("{!!") {
                (3, "!!}")
            } else {
                (2, "}}")
            };
            if start > 0 {
                out.push((false, &rest[..start]));
            }
            let end = rest[start + open..]
                .find(close)
                .map_or(rest.len(), |e| start + open + e);
            out.push((true, rest[start + open..end].trim()));
            rest = rest.get(end + close.len()..).unwrap_or("");
        }
        if !rest.is_empty() {
            out.push((false, rest));
        }
        out
    }

    /// A double-quoted string with `{$a->b}` or `$a` holes as a path with
    /// parameter segments (`"/x/{$role->id}"` → `/x/{id}`).
    fn interpolated(s: &str) -> Option<String> {
        let inner = s.strip_prefix('"')?.strip_suffix('"')?;
        let b = inner.as_bytes();
        let mut out = String::new();
        let mut i = 0;
        while i < b.len() {
            let (end, hole) = match b[i] {
                b'\\' => {
                    out.push(*b.get(i + 1)? as char);
                    i += 2;
                    continue;
                }
                b'"' => return None,
                b'{' if b.get(i + 1) == Some(&b'$') => {
                    let end = i + inner[i..].find('}')?;
                    (end + 1, &inner[i + 1..end])
                }
                b'$' => {
                    let mut end = ident_end(b, i + 1);
                    while inner[end..].starts_with("->") && ident_end(b, end + 2) > end + 2 {
                        end = ident_end(b, end + 2);
                    }
                    (end, &inner[i..end])
                }
                _ => {
                    let c = inner[i..].chars().next()?;
                    out.push(c);
                    i += c.len_utf8();
                    continue;
                }
            };
            out.push_str(&format!("{{{}}}", param_name(hole)));
            i = end;
        }
        Some(out)
    }

    /// A parameter name for an expression: its last identifier (`$b->id` → `id`).
    fn param_name(expr: &str) -> String {
        let words = idents(expr, 0, expr.len());
        let last = words
            .last()
            .map_or("param", |w| w.1.trim_start_matches('$'));
        if last.is_empty() {
            "param".into()
        } else {
            last.to_string()
        }
    }

    /// End of the tag starting at `i` (its `>`), skipping quoted values and echoes.
    fn tag_end(b: &[u8], i: usize) -> usize {
        let mut j = i + 1;
        while j < b.len() {
            match b[j] {
                b'{' if b[j..].starts_with(b"{{") => {
                    j = find(b, j + 2, b"}}").map_or(b.len(), |e| e + 2)
                }
                q @ (b'"' | b'\'') => j = quote_end(b, j + 1, q) + 1,
                b'>' => return j,
                _ => j += 1,
            }
        }
        b.len()
    }

    /// The closing `q` from `j`, skipping `{{ … }}`.
    fn quote_end(b: &[u8], mut j: usize, q: u8) -> usize {
        while j < b.len() && b[j] != q {
            j = if b[j..].starts_with(b"{{") {
                find(b, j + 2, b"}}").map_or(b.len(), |e| e + 2)
            } else {
                j + 1
            };
        }
        j
    }

    /// Attributes of a tag's text after its name: lower-case name and value.
    fn attributes(tag: &str) -> Vec<(String, String)> {
        let b = tag.as_bytes();
        let mut out = vec![];
        let mut i = 0;
        while i < b.len() {
            if b[i].is_ascii_whitespace() || b[i] == b'/' {
                i += 1;
                continue;
            }
            let start = i;
            while i < b.len() && !b[i].is_ascii_whitespace() && !matches!(b[i], b'=' | b'>' | b'/')
            {
                i = if b[i..].starts_with(b"{{") {
                    find(b, i + 2, b"}}").map_or(b.len(), |e| e + 2)
                } else {
                    i + 1
                };
            }
            let name = tag[start..i].to_ascii_lowercase();
            let (value, next) = attribute_value(tag, i);
            out.push((name, value));
            i = next.max(start + 1);
        }
        out
    }

    /// The value after an attribute name ending at `i`, and where to go on.
    fn attribute_value(tag: &str, i: usize) -> (String, usize) {
        let b = tag.as_bytes();
        let eq = next_nonws(b, i).filter(|&k| b[k] == b'=');
        match eq.and_then(|k| next_nonws(b, k + 1)) {
            Some(v) if matches!(b[v], b'"' | b'\'') => {
                let end = quote_end(b, v + 1, b[v]);
                (tag[v + 1..end.min(tag.len())].to_string(), end + 1)
            }
            Some(v) => {
                let end = (v..b.len())
                    .find(|&k| b[k].is_ascii_whitespace() || b[k] == b'>')
                    .unwrap_or(b.len());
                (tag[v..end].to_string(), end)
            }
            None => (String::new(), i),
        }
    }

    /// A stable element id: `data-testid`, `dusk` or `id`, when literal.
    fn trigger(attrs: &[(String, String)]) -> Option<String> {
        let literal = |n: &&str| {
            attrs
                .iter()
                .find(|(k, v)| k == n && !v.contains("{{") && !v.is_empty())
        };
        ["data-testid", "dusk", "id"]
            .iter()
            .find_map(|n| literal(n).map(|(_, v)| v.clone()))
    }

    /// A link's literal text (tags dropped); none when it holds Blade.
    fn label(inner: &str) -> Option<String> {
        if inner.contains("{{") || inner.contains("{!!") || inner.contains('@') {
            return None;
        }
        let mut text = String::new();
        let mut in_tag = false;
        for c in inner.chars() {
            match c {
                '<' => in_tag = true,
                '>' => in_tag = false,
                c if !in_tag => text.push(c),
                _ => {}
            }
        }
        let text = collapse(&text);
        (!text.is_empty() && text.chars().count() <= 80).then_some(text)
    }
}

/// Where the links of each template start, and the transitions and actions.
mod links {
    use super::blade::{Dest, Link, Tpl};
    use super::*;

    /// Origins (screen or every screen, with the include condition) per template.
    #[derive(Default)]
    struct Origins {
        map: BTreeMap<String, BTreeSet<(Origin, Option<Condition>)>>,
        /// Layouts still to visit: (name, file, line of `@extends`).
        layouts: Vec<(String, String, u32)>,
        stack: Vec<String>,
    }

    /// Where a link may lead: a screen, or a GET route without a view.
    enum Res {
        Screen(String),
        Endpoint(String),
    }

    impl Run<'_> {
        /// Transitions and actions of the Blade templates.
        pub(super) fn blade(
            &mut self,
            flow: &mut UiFlow,
            routes: &[RouteDef],
            screens: &[ScreenViews],
        ) {
            let app = self.app;
            let mut tpls: BTreeMap<&str, Tpl> = BTreeMap::new();
            for (name, (rel, text)) in &app.templates {
                let tpl = self.template(rel, text);
                tpls.insert(name.as_str(), tpl);
            }
            let origins = self.origins(&tpls, screens);
            let screen_routes: Vec<(&str, &str)> = (flow.screens.iter())
                .filter(|s| {
                    s.application.as_deref() == Some(app.name.as_str()) && s.source == SOURCE
                })
                .filter_map(|s| Some((s.id.as_str(), s.route.as_deref()?)))
                .collect();
            let (mut transitions, mut actions) = (vec![], vec![]);
            for (name, from) in &origins {
                let Some(tpl) = tpls.get(name.as_str()) else {
                    continue;
                };
                for link in &tpl.links {
                    let resolved = link
                        .dest
                        .as_ref()
                        .map(|d| resolve(&app.name, d, &screen_routes, routes));
                    for (origin, cond) in from {
                        let at = Emit {
                            tpl,
                            link,
                            origin,
                            condition: and(cond.clone(), link.cond.clone()),
                        };
                        match at.call(&resolved, &screen_routes) {
                            Ok(call) => actions.push(at.action(call)),
                            Err(target) => transitions.push(at.transition(target)),
                        }
                    }
                    if let Some(why) = why(link, &resolved) {
                        self.gap(
                            &tpl.rel,
                            link.line,
                            format!("Navigation target {} {why}.", link.source),
                        );
                    }
                }
            }
            flow.transitions.extend(transitions);
            flow.actions.extend(actions);
        }

        /// The origins of every template reached from a screen's views or a layout.
        fn origins(
            &mut self,
            tpls: &BTreeMap<&str, Tpl>,
            screens: &[ScreenViews],
        ) -> BTreeMap<String, BTreeSet<(Origin, Option<Condition>)>> {
            let mut o = Origins::default();
            for s in screens {
                for v in &s.views {
                    self.visit(
                        tpls,
                        v,
                        (&Origin::Screen(s.id.clone()), None),
                        (&s.file, s.line),
                        &mut o,
                    );
                }
            }
            let every = Origin::AnyScreen {
                application: self.app.name.clone(),
            };
            let mut seen = BTreeSet::new();
            while let Some((name, file, line)) = o.layouts.pop() {
                if seen.insert(name.clone()) {
                    self.visit(tpls, &name, (&every, None), (&file, line), &mut o);
                }
            }
            o.map
        }

        /// Records that template `name` renders on `from`, then its includes;
        /// its layouts go to `o.layouts`.
        fn visit(
            &mut self,
            tpls: &BTreeMap<&str, Tpl>,
            name: &str,
            from: (&Origin, Option<Condition>),
            site: (&str, u32),
            o: &mut Origins,
        ) {
            let (origin, cond) = from;
            if o.stack.iter().any(|s| s == name) || o.stack.len() > 16 {
                return;
            }
            let Some(tpl) = tpls.get(name) else {
                return self.gap(
                    site.0,
                    site.1,
                    format!("Template {name} not found; its links are unknown."),
                );
            };
            if !o
                .map
                .entry(name.to_string())
                .or_default()
                .insert((origin.clone(), cond.clone()))
            {
                return;
            }
            o.stack.push(name.to_string());
            for (include, c, line) in &tpl.includes {
                let next = and(cond.clone(), c.clone());
                self.visit(tpls, include, (origin, next), (&tpl.rel, *line), o);
            }
            for (layout, line) in &tpl.extends {
                o.layouts.push((layout.clone(), tpl.rel.clone(), *line));
            }
            o.stack.pop();
        }
    }

    /// One link placed at one origin.
    struct Emit<'e> {
        tpl: &'e Tpl,
        link: &'e Link,
        origin: &'e Origin,
        condition: Option<Condition>,
    }

    impl Emit<'_> {
        /// The HTTP call of the link, or the navigation target when it is one.
        fn call(
            &self,
            resolved: &Option<Result<Res, String>>,
            screens: &[(&str, &str)],
        ) -> Result<Call, Target> {
            let link = self.link;
            let form = link.method.as_deref().filter(|m| *m != "GET");
            match (form, &link.dest, resolved) {
                (Some(m), Some(Dest::Path(p)), _) => Ok(http(m, p)),
                (Some(m), Some(Dest::Name(_)), Some(Ok(Res::Screen(id)))) => Ok(http(
                    m,
                    screens
                        .iter()
                        .find(|s| s.0 == id.as_str())
                        .map_or("", |s| s.1),
                )),
                (Some(m), Some(Dest::Name(_)), Some(Ok(Res::Endpoint(p)))) => Ok(http(m, p)),
                (Some(_), _, _) => Ok(Call::Unresolved(link.source.clone())),
                (None, _, Some(Ok(Res::Screen(id)))) => Err(Target::Screen(id.clone())),
                (None, _, Some(Ok(Res::Endpoint(p)))) => Ok(http("GET", p)),
                (None, _, _) => Err(Target::Unresolved(link.source.clone())),
            }
        }

        fn action(&self, call: Call) -> Action {
            Action {
                screen: self.origin.clone(),
                trigger: self.link.trigger.clone(),
                call,
                file: self.tpl.rel.clone(),
                line: self.link.line,
                condition: self.condition.clone(),
                source: SOURCE.into(),
            }
        }

        fn transition(&self, target: Target) -> Transition {
            Transition {
                from: self.origin.clone(),
                target,
                trigger: self.link.trigger.clone(),
                file: self.tpl.rel.clone(),
                line: self.link.line,
                condition: self.condition.clone(),
                source: SOURCE.into(),
            }
        }
    }

    /// Why a link is a gap, if it is one.
    fn why(link: &Link, resolved: &Option<Result<Res, String>>) -> Option<String> {
        let form = link.method.as_deref().is_some_and(|m| m != "GET");
        match (&link.dest, resolved) {
            (None, _) => Some("is not read literally; its target is unknown".to_string()),
            (Some(Dest::Path(_)), _) if form => None,
            (_, Some(Err(why))) => Some(why.clone()),
            _ => None,
        }
    }

    fn http(method: &str, path: &str) -> Call {
        Call::Http {
            method: method.into(),
            path: path.into(),
        }
    }

    /// The screen or GET route `dest` names in application `app`, else why not.
    fn resolve(
        app: &str,
        dest: &Dest,
        screens: &[(&str, &str)],
        routes: &[RouteDef],
    ) -> Result<Res, String> {
        let get = |r: &&RouteDef| r.methods.iter().any(|m| m == "GET" || m == ANY_METHOD);
        match dest {
            Dest::Path(p) => match best_route(p, screens.iter().copied()) {
                Ok(id) => Ok(Res::Screen(id.to_string())),
                Err(tied) if !tied.is_empty() => Err(format!(
                    "matches several routes equally: {}",
                    tied.join(", ")
                )),
                Err(_)
                    if routes
                        .iter()
                        .filter(get)
                        .any(|r| path_patterns_match(&r.path, p)) =>
                {
                    Ok(Res::Endpoint(p.clone()))
                }
                Err(_) => Err("is not a route of this application".into()),
            },
            Dest::Name(n) => {
                let route = routes
                    .iter()
                    .find(|r| r.name.as_deref() == Some(n.as_str()));
                let Some(r) = route else {
                    return Err(format!("names no route of this application ({n})"));
                };
                let id = format!("{app}:{}", r.path);
                match screens.iter().any(|s| s.0 == id) {
                    true => Ok(Res::Screen(id)),
                    false => Ok(Res::Endpoint(r.path.clone())),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{
        ANY_METHOD, AccessState, Action, Call, Condition, EndpointAccess, Origin, Target,
        Transition,
    };

    fn app(files: &[(&str, &str)]) -> (tempfile::TempDir, UiFlow) {
        let dir = tempfile::tempdir().unwrap();
        for (rel, text) in [("shop/artisan", "#!/usr/bin/env php\n")]
            .iter()
            .chain(files)
        {
            let path = dir.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let flow = extract(&dir.path().join("shop"));
        (dir, flow)
    }

    fn endpoint<'a>(flow: &'a UiFlow, method: &str, path: &str) -> &'a EndpointAccess {
        (flow.endpoints.iter())
            .find(|e| e.method == method && e.path == path)
            .unwrap_or_else(|| panic!("no {method} {path}: {:#?}", flow.endpoints))
    }

    fn access(flow: &UiFlow, method: &str, path: &str) -> AccessState {
        endpoint(flow, method, path).access.clone()
    }

    fn all(items: Vec<Condition>) -> AccessState {
        AccessState::Protected(Condition::All(items))
    }

    fn policy(name: &str) -> Condition {
        Condition::Policy(name.into())
    }

    fn messages(flow: &UiFlow) -> Vec<String> {
        (flow.diagnostics.iter())
            .map(|d| format!("{}:{} {}", d.file, d.line, d.message))
            .collect()
    }

    fn controller(name: &str, body: &str) -> (String, String) {
        let path = format!("shop/app/Http/Controllers/{name}.php");
        let text = format!(
            "<?php\nnamespace App\\Http\\Controllers;\n\nuse App\\Models\\Book;\n\n\
             class {name} extends Controller\n{{\n{body}\n}}\n"
        );
        (path, text)
    }

    const WEB: &str = r#"<?php
use App\Http\Controllers\BookController;
use App\Http\Controllers as C;
use Illuminate\Support\Facades\Route;

Route::get('/', [C\HomeController::class, 'index'])->name('home');
Route::middleware('auth')->group(function () {
    Route::get('/books/{id}', [BookController::class, 'show'])->name('books.show');
    Route::post('/books', 'BookController@store');
    Route::prefix('admin')->name('admin.')->middleware(['can:manage-users'])->group(function () {
        Route::resource('users', C\UserController::class)->only(['index', 'destroy']);
    });
    // Route::get('/commented', [BookController::class, 'show']);
    Route::get('/public', fn () => view('welcome'))->withoutMiddleware('auth');
});
Route::view('/about', 'about');
Route::match(['get', 'post'], '/search', [C\SearchController::class, 'run']);
Route::any('/hook', [C\HookController::class, 'handle'])->middleware('auth:sanctum');
require __DIR__.'/auth.php';
"#;

    fn routes_app() -> (tempfile::TempDir, UiFlow) {
        let ctl = |n: &str, b: &str| controller(n, b);
        let files = [
            ctl(
                "HomeController",
                "public function index() { return view('home'); }",
            ),
            ctl(
                "BookController",
                "public function __construct()\n{\n$this->middleware('permission:edit books')->only(['store']);\n}\n\
                 public function show(Book $book)\n{\n$this->authorize('view', $book);\nreturn view('books.show', ['book' => $book]);\n}\n\
                 public function store() { return redirect('/books'); }",
            ),
            ctl(
                "UserController",
                "public function index() { return view('users.index'); }\npublic function destroy($user) { }",
            ),
            ctl(
                "SearchController",
                "public function run() { return view('search'); }",
            ),
            ctl(
                "HookController",
                "public function handle() { return response()->json([]); }",
            ),
            ctl(
                "LoginController",
                "public function show() { return view('auth.login'); }",
            ),
            ctl(
                "PhotoController",
                "public function index() {}\npublic function store() {}\npublic function show($p) {}\npublic function update($p) {}",
            ),
        ];
        let mut all: Vec<(&str, &str)> = files
            .iter()
            .map(|(p, t)| (p.as_str(), t.as_str()))
            .collect();
        all.extend([
            ("shop/routes/web.php", WEB),
            (
                "shop/routes/auth.php",
                "<?php\nRoute::get('/login', [App\\Http\\Controllers\\LoginController::class, 'show'])->middleware('guest');\n",
            ),
            (
                "shop/routes/api.php",
                "<?php\nuse App\\Http\\Controllers\\PhotoController;\n\n\
                 Route::apiResource('photos', PhotoController::class)->except(['destroy']);\n\
                 Route::get('/me', function () {\n    Gate::authorize('see-me');\n    return [];\n})->middleware('role:admin|editor');\n",
            ),
            (
                "shop/bootstrap/app.php",
                "<?php\nreturn Application::configure(basePath: dirname(__DIR__))\n\
                 ->withRouting(web: __DIR__.'/../routes/web.php', api: __DIR__.'/../routes/api.php', apiPrefix: 'api/v1')\n->create();\n",
            ),
        ]);
        app(&all)
    }

    #[test]
    fn routes_groups_resources_and_handlers() {
        let (_dir, flow) = routes_app();
        let rows: Vec<(String, String, Option<String>)> = (flow.endpoints.iter())
            .map(|e| (e.method.clone(), e.path.clone(), e.handler.clone()))
            .collect();
        let h = |s: &str| Some(s.to_string());
        let want = [
            ("DELETE", "/admin/users/{user}", h("UserController.destroy")),
            ("GET", "/", h("HomeController.index")),
            ("GET", "/about", None),
            ("GET", "/admin/users", h("UserController.index")),
            ("GET", "/api/v1/me", None),
            ("GET", "/api/v1/photos", h("PhotoController.index")),
            ("GET", "/api/v1/photos/{photo}", h("PhotoController.show")),
            ("GET", "/books/{id}", h("BookController.show")),
            ("GET", "/login", h("LoginController.show")),
            ("GET", "/public", None),
            ("GET", "/search", h("SearchController.run")),
            (
                "PATCH",
                "/api/v1/photos/{photo}",
                h("PhotoController.update"),
            ),
            ("POST", "/api/v1/photos", h("PhotoController.store")),
            ("POST", "/books", h("BookController.store")),
            ("POST", "/search", h("SearchController.run")),
            ("PUT", "/api/v1/photos/{photo}", h("PhotoController.update")),
        ];
        let mut want: Vec<(String, String, Option<String>)> = (want.iter())
            .map(|(m, p, h)| (m.to_string(), p.to_string(), h.clone()))
            .collect();
        want.push((
            ANY_METHOD.into(),
            "/hook".into(),
            h("HookController.handle"),
        ));
        want.sort();
        let mut rows = rows;
        rows.sort();
        assert_eq!(rows, want);
        assert!(flow.endpoints.iter().all(|e| e.source == "laravel"));
        assert!(
            flow.endpoints
                .iter()
                .all(|e| e.application.as_deref() == Some("shop"))
        );
        let books = endpoint(&flow, "GET", "/books/{id}");
        assert_eq!((books.file.as_str(), books.line), ("routes/web.php", 8));
        // Only the templates of these screens are missing.
        let found = messages(&flow);
        assert!(found.iter().all(|m| m.contains("Template ")), "{found:#?}");
    }

    #[test]
    fn access_from_middleware_policies_and_without_middleware() {
        let (_dir, flow) = routes_app();
        let auth = Condition::Authenticated;
        assert_eq!(access(&flow, "GET", "/"), AccessState::Undeclared);
        assert_eq!(
            access(&flow, "GET", "/books/{id}"),
            all(vec![auth.clone(), policy("view")])
        );
        assert_eq!(
            access(&flow, "POST", "/books"),
            all(vec![auth.clone(), policy("edit books")])
        );
        let admin = all(vec![auth.clone(), policy("manage-users")]);
        assert_eq!(access(&flow, "GET", "/admin/users"), admin);
        assert_eq!(access(&flow, "DELETE", "/admin/users/{user}"), admin);
        assert_eq!(access(&flow, "GET", "/public"), AccessState::Open);
        assert_eq!(
            access(&flow, ANY_METHOD, "/hook"),
            AccessState::Protected(auth.clone())
        );
        let guest = Condition::Not(Box::new(auth.clone()));
        assert_eq!(
            access(&flow, "GET", "/login"),
            AccessState::Protected(guest)
        );
        let roles = Condition::Any(vec![
            Condition::Role("admin".into()),
            Condition::Role("editor".into()),
        ]);
        assert_eq!(
            access(&flow, "GET", "/api/v1/me"),
            all(vec![roles, policy("see-me")])
        );
        assert_eq!(
            access(&flow, "GET", "/api/v1/photos"),
            AccessState::Undeclared
        );
    }

    #[test]
    fn screens_are_get_routes_returning_a_literal_view() {
        let (_dir, flow) = routes_app();
        let screens: Vec<(&str, &str, Option<&str>)> = (flow.screens.iter())
            .map(|s| (s.id.as_str(), s.name.as_str(), s.route.as_deref()))
            .collect();
        assert_eq!(
            screens,
            [
                ("shop:/", "home", Some("/")),
                ("shop:/about", "about", Some("/about")),
                ("shop:/admin/users", "users.index", Some("/admin/users")),
                ("shop:/books/{id}", "books.show", Some("/books/{id}")),
                ("shop:/login", "auth.login", Some("/login")),
                ("shop:/public", "welcome", Some("/public")),
                ("shop:/search", "search", Some("/search")),
            ]
        );
        let books = flow
            .screens
            .iter()
            .find(|s| s.id == "shop:/books/{id}")
            .unwrap();
        let want = Condition::All(vec![Condition::Authenticated, policy("view")]);
        assert_eq!(books.access, Some(want));
        assert_eq!(books.application.as_deref(), Some("shop"));
        let public = flow
            .screens
            .iter()
            .find(|s| s.id == "shop:/public")
            .unwrap();
        assert_eq!(public.access, None);
    }

    const KERNEL: &str = r#"<?php
namespace App\Http;

class Kernel extends HttpKernel
{
    protected $middlewareGroups = [
        'web' => [\App\Http\Middleware\EncryptCookies::class],
        'api' => [
            \App\Http\Middleware\ApiAuthenticate::class,
            'throttle:api',
        ],
    ];

    protected $middlewareAliases = [
        'auth' => \App\Http\Middleware\Authenticate::class,
        'can' => \App\Http\Middleware\CheckUserHasPermission::class,
        'mfa-setup' => \App\Http\Middleware\AuthenticatedOrPendingMfa::class,
    ];
}
"#;

    const PERMISSION: &str = r#"<?php
namespace App\Permissions;

enum Permission: string
{
    case SettingsManage = 'settings-manage';
    case UsersManage = 'users-manage';

    public function middleware(): string
    {
        return 'can:' . $this->value;
    }
}
"#;

    const PROVIDER: &str = r#"<?php
namespace App\Providers;

class RouteServiceProvider extends ServiceProvider
{
    public function boot(): void
    {
        $this->routes(function () {
            Route::group(['middleware' => 'web', 'namespace' => $this->namespace], function (Router $router) {
                require base_path('routes/web.php');
            });
            Route::group(['middleware' => 'api', 'prefix' => 'v2'], function ($router) {
                require base_path('routes/api.php');
            });
        });
    }
}
"#;

    fn configured_app() -> (tempfile::TempDir, UiFlow) {
        let (settings, settings_text) = controller(
            "SettingController",
            "public function __construct()\n{\n\
             $this->middleware(Permission::SettingsManage->middleware());\n\
             $this->middleware(function ($request, $next) {\n    $this->preventGuestAccess();\n    return $next($request);\n})->except(['index']);\n}\n\
             public function index()\n{\n$this->checkPermission(Permission::UsersManage);\nif ($x) {\n    $this->checkPermission('other');\n}\nreturn view('settings.index');\n}\n\
             public function x() { return view($name); }",
        );
        let settings_text = settings_text.replace(
            "use App\\Models\\Book;",
            "use App\\Permissions\\Permission;",
        );
        let files = [
            (
                ".archguard/archguard.toml",
                "[views.access]\nApiAuthenticate = \"authenticated\"\ncheckPermission = \"policy:$arg\"\n\
                 preventGuestAccess = \"authenticated\"\nuserCan = \"policy:$arg\"\n",
            ),
            ("app/Http/Kernel.php", KERNEL),
            ("app/Permissions/Permission.php", PERMISSION),
            ("app/Providers/RouteServiceProvider.php", PROVIDER),
            (settings.trim_start_matches("shop/"), settings_text.as_str()),
            (
                "routes/web.php",
                "<?php\nuse App\\Http\\Controllers\\SettingController;\n\
                 Route::get('/settings', [SettingController::class, 'index']);\n\
                 Route::get('/settings/x', [SettingController::class, 'x']);\n\
                 Route::get('/mfa', [SettingController::class, 'x'])->middleware('mfa-setup');\n\
                 Route::get($dynamic, [SettingController::class, 'x']);\n\
                 Route::get('/gone', [MissingController::class, 'x']);\n",
            ),
            (
                "routes/api.php",
                "<?php\nuse App\\Http\\Controllers\\SettingController;\nRoute::get('books', [SettingController::class, 'x']);\n",
            ),
            (
                "app/Policies/BookPolicy.php",
                "<?php\nnamespace App\\Policies;\nclass BookPolicy\n{\n    public function view(User $u, Book $b): bool { return true; }\n}\n",
            ),
        ];
        let files: Vec<(String, &str)> = files
            .iter()
            .map(|(p, t)| (format!("shop/{p}"), *t))
            .collect();
        let files: Vec<(&str, &str)> = files.iter().map(|(p, t)| (p.as_str(), *t)).collect();
        app(&files)
    }

    #[test]
    fn kernel_groups_aliases_enums_closures_and_views_access() {
        let (_dir, flow) = configured_app();
        let auth = Condition::Authenticated;
        let manage = policy("settings-manage");
        assert_eq!(
            access(&flow, "GET", "/settings"),
            all(vec![manage.clone(), policy("users-manage")])
        );
        assert_eq!(
            access(&flow, "GET", "/settings/x"),
            all(vec![manage.clone(), auth.clone()])
        );
        assert_eq!(
            access(&flow, "GET", "/mfa"),
            all(vec![manage.clone(), auth.clone()])
        );
        // The provider's `api` group (ApiAuthenticate) and prefix `v2`.
        assert_eq!(
            access(&flow, "GET", "/v2/books"),
            all(vec![auth.clone(), manage.clone()])
        );
        let found = messages(&flow);
        let has = |part: &str| found.iter().any(|m| m.contains(part));
        assert!(has("routes/web.php:5 Middleware `mfa-setup`"), "{found:#?}");
        assert!(has("routes/web.php:6 Route path $dynamic"), "{found:#?}");
        assert!(
            has("routes/web.php:7 Controller MissingController"),
            "{found:#?}"
        );
        assert!(
            has("SettingController.php:20 Check `checkPermission` inside a block"),
            "{found:#?}"
        );
        assert!(
            has("SettingController.php:24 View name $name"),
            "{found:#?}"
        );
        assert!(!flow.complete);
        let book = flow.policies.iter().find(|p| p.name == "view").unwrap();
        assert_eq!(book.requires, Condition::Unknown("BookPolicy::view".into()));
        assert_eq!(
            (book.file.as_str(), book.line),
            ("app/Policies/BookPolicy.php", 5)
        );
    }

    const LAYOUT: &str = r#"<html><nav>
<a href="{{ url('/') }}">Home</a>
@auth
  <a href="{{ route('books.index') }}" data-testid="books">Books</a>
@endauth
@include('layouts.menu')
</nav>
@yield('content')
</html>
"#;

    const MENU: &str = r#"@can('admin')
  <a href="/admin">Admin</a>
@else
  <a href="/help">Help</a>
@endcan
"#;

    const INDEX: &str = r#"@extends('layouts.app')
@section('content')
@foreach($books as $b)
  <a href="{{ url('/books/' . $b->id) }}">Open</a>
  <a href="{{ $b->url() }}">Other</a>
@endforeach
@if(userCan('book-create') && $ready)
  <a href="/books/create">New</a>
@endif
@include('books.parts.delete', ['id' => 1])
{{-- <a href="/gone">gone</a> --}}
<a href="/books/1/export?x=1">Export</a>
<a href="{{ url('/help?topic=' . urlencode($t)) }}" id="help">?</a>
<a href="https://example.com">Out</a>
@endsection
"#;

    const DELETE: &str = r#"<form action="{{ url('/books/' . $id) }}" method="POST">
  @method('DELETE')
  <button>Delete</button>
</form>
<form action="{{ url('/search') }}"><input name="q"></form>
<form action="{{ url("/books/{$id}/archive") }}" method="POST"></form>
<form action="{{ url($logoutPath) }}" method="post"></form>
"#;

    fn blade_app() -> (tempfile::TempDir, UiFlow) {
        let ctl = |n: &str, b: &str| controller(n, b);
        let books = ctl(
            "BookController",
            "public function index() { return view('books.index'); }\n\
             public function create() { return view('books.create'); }\n\
             public function show($id) { return view('books.show'); }\n\
             public function destroy($id) { }\n\
             public function export($id) { return response()->download('x'); }",
        );
        let pages = ctl(
            "PageController",
            "public function home() { return view('home'); }\npublic function admin() { return view('admin'); }\n\
             public function help() { return view('help'); }\npublic function search() { return view('search'); }",
        );
        let web = "<?php\nuse App\\Http\\Controllers\\BookController;\nuse App\\Http\\Controllers\\PageController;\n\
                   Route::get('/', [PageController::class, 'home']);\n\
                   Route::get('/admin', [PageController::class, 'admin']);\n\
                   Route::get('/help', [PageController::class, 'help']);\n\
                   Route::get('/search', [PageController::class, 'search']);\n\
                   Route::get('/books', [BookController::class, 'index'])->name('books.index');\n\
                   Route::get('/books/create', [BookController::class, 'create']);\n\
                   Route::get('/books/{id}', [BookController::class, 'show']);\n\
                   Route::delete('/books/{id}', [BookController::class, 'destroy']);\n\
                   Route::get('/books/{id}/export', [BookController::class, 'export']);\n";
        let files = [
            (books.0.as_str(), books.1.as_str()),
            (pages.0.as_str(), pages.1.as_str()),
            ("shop/routes/web.php", web),
            (
                "shop/.archguard/archguard.toml",
                "[views.access]\nuserCan = \"policy:$arg\"\n",
            ),
            ("shop/resources/views/layouts/app.blade.php", LAYOUT),
            ("shop/resources/views/layouts/menu.blade.php", MENU),
            ("shop/resources/views/books/index.blade.php", INDEX),
            ("shop/resources/views/books/parts/delete.blade.php", DELETE),
            (
                "shop/resources/views/books/create.blade.php",
                "@extends('layouts.app')\n",
            ),
        ];
        app(&files)
    }

    fn transitions(flow: &UiFlow) -> Vec<(String, String, Option<String>, Option<Condition>)> {
        let origin = |o: &Origin| match o {
            Origin::Screen(id) => id.clone(),
            Origin::AnyScreen { application } => format!("{application}:*"),
        };
        (flow.transitions.iter())
            .map(|t: &Transition| {
                let to = match &t.target {
                    Target::Screen(id) => id.clone(),
                    Target::Unresolved(text) => format!("? {text}"),
                };
                (origin(&t.from), to, t.trigger.clone(), t.condition.clone())
            })
            .collect()
    }

    #[test]
    fn blade_links_layouts_includes_and_conditions() {
        let (_dir, flow) = blade_app();
        let s = |x: &str| x.to_string();
        let not_admin = Condition::Not(Box::new(policy("admin")));
        let create = Condition::All(vec![
            policy("book-create"),
            Condition::Unknown("$ready".into()),
        ]);
        let want = vec![
            (s("shop:*"), s("shop:/"), Some(s("Home")), None),
            (
                s("shop:*"),
                s("shop:/admin"),
                Some(s("Admin")),
                Some(policy("admin")),
            ),
            (
                s("shop:*"),
                s("shop:/books"),
                Some(s("books")),
                Some(Condition::Authenticated),
            ),
            (
                s("shop:*"),
                s("shop:/help"),
                Some(s("Help")),
                Some(not_admin),
            ),
            (s("shop:/books"), s("shop:/help"), Some(s("help")), None),
            (
                s("shop:/books"),
                s("? {{ $b->url() }}"),
                Some(s("Other")),
                None,
            ),
            (
                s("shop:/books"),
                s("shop:/books/create"),
                Some(s("New")),
                Some(create),
            ),
            (
                s("shop:/books"),
                s("shop:/books/{id}"),
                Some(s("Open")),
                None,
            ),
            (s("shop:/books"), s("shop:/search"), None, None),
        ];
        let mut got = transitions(&flow);
        got.sort();
        let mut want = want;
        want.sort();
        assert_eq!(got, want);
        let found = messages(&flow);
        assert!(
            found
                .iter()
                .any(|m| m.starts_with("resources/views/books/index.blade.php:5 ")),
            "{found:#?}"
        );
        let actions: Vec<(&Origin, &Call)> = flow
            .actions
            .iter()
            .map(|a: &Action| (&a.screen, &a.call))
            .collect();
        let books = Origin::Screen("shop:/books".into());
        let http = |m: &str, p: &str| Call::Http {
            method: m.into(),
            path: p.into(),
        };
        assert_eq!(
            actions,
            [
                (&books, &http("DELETE", "/books/{id}")),
                (&books, &http("POST", "/books/{id}/archive")),
                // A path that is a variable as a whole stays unread.
                (&books, &Call::Unresolved("{{ url($logoutPath) }}".into())),
                (&books, &http("GET", "/books/1/export")),
            ]
        );
        let delete = &flow.actions[0];
        assert_eq!(
            (delete.file.as_str(), delete.line),
            ("resources/views/books/parts/delete.blade.php", 1)
        );
    }
}
