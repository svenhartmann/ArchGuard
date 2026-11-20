//! Express backend endpoint access extractor for ui-flow/v1 (ADR 0021).
//!
//! Text level, no TypeScript parser: comments and string contents are blanked
//! for searching, literals are read with the shared `lang_ts` parser. It fills
//! `endpoints`, `handler_sites` and `diagnostics`.
//!
//! An application is a directory whose `package.json` has the dependency
//! `express`; its directory name is the endpoint's `application`. Its `.js`,
//! `.mjs`, `.cjs` and `.ts` files are read (not `.d.ts`, tests or test folders).
//! A directory with `fastify` or `koa` and no `express` is not read (a note).
//!
//! Recognized:
//! - Application and routers: `x = express()` is an application, `x =
//!   express.Router()` (or `Router()` imported from `express`) a router. Only
//!   these variables are read; `this.router.get(...)` and the like are not.
//! - Endpoints: `x.get|post|put|patch|delete|options|head|all(path, ...handlers)`
//!   (`all` gives method `ANY`), `x.route(path).get(...).post(...)`. A path is a
//!   string literal or an array of them; the last handler argument is the
//!   handler, the others (also inside arrays) are its middleware. A call with
//!   only a path (`app.get('env')`) is no route.
//! - Composition: `x.use(prefix, router)` and `x.use(router)` with a router
//!   declared in the same file or imported from another file of the application
//!   (ES `import`, `require`, relative paths, `index` files; a default export is
//!   `export default r` / `module.exports = r`, else the file's only router).
//!   `list.forEach((r) => x.use(r.path, r.route))` over a literal array of
//!   `{ path, route }` objects in the same file mounts each entry. Prefixes add
//!   up across files; the endpoint path is the full path.
//! - Access: every `use(mw)` / `use(path, mw)` before a route applies to it
//!   (`use(path, mw)` only to paths below `path`), and so does each middleware
//!   in the route's handler list; a router inherits what stood before its mount.
//!   A middleware maps to a condition ONLY through `[views.access]` by its name
//!   (`auth`, `authController.protect`; `auth('manageUsers')` with
//!   `auth = "policy:$arg"` takes literal arguments, a non-literal one is a
//!   gap); an `open` middleware adds none. A middleware that is not listed is
//!   `unknown(name)` with a note, never guessed from its name. Only middleware
//!   of packages known not to decide access is skipped: `helmet`, `cors`,
//!   `compression`, `morgan`, `cookie-parser`, `body-parser`, `serve-favicon`,
//!   `xss-clean`, `express-mongo-sanitize` (by the package the name is imported from),
//!   `express.json/urlencoded/static/raw/text` and `passport.initialize`; a name
//!   listed in `[views.access]` wins. No middleware at all is `open`; all
//!   conditions must hold (`all`). Order counts: a `use` after the route is
//!   ignored.
//! - An inline error handler (four parameters) adds no condition: Express calls
//!   it only after an error.
//! - `handler` is the last handler argument when it is a name (`ctrl.fn`);
//!   a handler site is added when that name's declaration is found in the
//!   file it is imported from (or the same file), otherwise the endpoint's
//!   own `file:line` (where it is registered) is the only site.
//!
//! Not recognized (gap, never a guess): non-literal paths and prefixes,
//! non-literal middleware arguments, regular-expression paths, relative modules
//! that are not read (a `bin/` folder is excluded by discovery) in `use(...)`,
//! routers that are not mounted by any
//! application, mounts that cannot be followed (aliases, factories, inline
//! routers), mount cycles. Mounts inside `if` blocks count as mounted.
use crate::extract_ctx::{
    ExtractContext, access_condition, dir_name, finish, gap, read_source, walk, with_root_config,
};
use crate::lang_ts::{
    Item, Parser, Value, call_parts, close, dir, ident_at, is_name, join, line_of, skip_ws,
    trailing_ident, words,
};
use crate::lang_ts_decl::{Import, declared_value, has_dep, imports, mask};
use crate::services::support::test_path;
use archguard_core::{
    ANY_METHOD, AccessMeaning, AccessState, Condition, DiagnosticSeverity, EndpointAccess,
    HandlerSite, UiFlow, UiFlowDiagnostic,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

const MAX_FILES: usize = 20_000;
const MAX_DEPTH: usize = 32;
const EXTS: [&str; 4] = [".js", ".mjs", ".cjs", ".ts"];
const VERBS: [(&str, &str); 8] = [
    ("get", "GET"),
    ("post", "POST"),
    ("put", "PUT"),
    ("patch", "PATCH"),
    ("delete", "DELETE"),
    ("options", "OPTIONS"),
    ("head", "HEAD"),
    ("all", ANY_METHOD),
];
/// Packages whose middleware never decides access.
const PASSIVE: [&str; 9] = [
    "helmet",
    "serve-favicon",
    "cors",
    "compression",
    "morgan",
    "cookie-parser",
    "body-parser",
    "xss-clean",
    "express-mongo-sanitize",
];
const PASSIVE_EXPRESS: [&str; 5] = [
    "express.json",
    "express.urlencoded",
    "express.static",
    "express.raw",
    "express.text",
];

/// Endpoints of every Express application below `root` with their declared
/// access; `[views.access]` comes from the root's configuration.
pub fn extract(root: &Path) -> UiFlow {
    with_root_config(root, extract_with)
}

/// [`extract`] for one run.
pub fn extract_with(ctx: &ExtractContext) -> UiFlow {
    let mut flow = UiFlow::default();
    let apps = read_apps(ctx.root, &mut flow.diagnostics);
    if let (Some(e), false) = (&ctx.config_error, apps.is_empty()) {
        let message = format!("Configuration not read; [views.access] not applied: {e}");
        note(&mut flow.diagnostics, ".", 0, message);
    }
    for (name, files) in &apps {
        let mut scan = Scan {
            files,
            access: &ctx.config.views.access,
            app: name,
            flow: &mut flow,
            visited: BTreeSet::new(),
            stack: vec![],
        };
        scan.run();
    }
    finish(&mut flow);
    flow
}

fn note(out: &mut Vec<UiFlowDiagnostic>, file: &str, line: u32, message: String) {
    out.push(UiFlowDiagnostic {
        file: file.into(),
        line,
        message,
        severity: DiagnosticSeverity::Note,
    });
}

/// A source: `code` has comments and string contents blanked (offsets
/// unchanged), `raw` is read for literals.
struct Src {
    raw: String,
    code: String,
    binds: Vec<Import>,
    /// Router and application variables: (is application, offset).
    routers: BTreeMap<String, (bool, usize)>,
}

type Files = BTreeMap<String, Src>;

impl Src {
    fn new(raw: String) -> Self {
        let code = mask(&raw, false);
        let mut binds = imports(&raw);
        binds.extend(requires(&raw, &code));
        let routers = routers(&code, &binds);
        Src {
            raw,
            code,
            binds,
            routers,
        }
    }
}

/// `const x = require('m')`, `const { a, b: c } = require('m')`, `const x =
/// require('m').y`, as imports.
fn requires(raw: &str, code: &str) -> Vec<Import> {
    let b = code.as_bytes();
    let mut out = Vec::new();
    for pos in words(code, "require") {
        let k = skip_ws(b, pos + "require".len());
        if b.get(k) != Some(&b'(') {
            continue;
        }
        let args = Parser { s: raw, i: k }.array(0);
        let Some(Value::Str(module)) = args.first().map(|a| &a.value) else {
            continue;
        };
        let end = close(code, k);
        let member = (code[end..].starts_with('.'))
            .then(|| ident_at(code, end + 1).to_string())
            .filter(|m| !m.is_empty());
        let before = code[..pos].trim_end();
        let Some(lhs) = before.strip_suffix('=').map(str::trim_end) else {
            continue;
        };
        let mut bind = |local: &str, imported: Option<String>| {
            if is_name(local) {
                out.push(Import {
                    local: local.into(),
                    imported,
                    module: module.clone(),
                });
            }
        };
        match (lhs.ends_with('}'), lhs.rfind('{')) {
            (true, Some(open)) => {
                for part in lhs[open + 1..lhs.len() - 1].split(',') {
                    let (key, local) = part.split_once(':').unwrap_or((part, part));
                    bind(local.trim(), Some(key.trim().into()));
                }
            }
            _ => {
                if let Some(local) = trailing_ident(lhs) {
                    bind(&local, member);
                }
            }
        }
    }
    out
}

/// `x = express()` (application), `x = express.Router()` or `Router()`
/// imported from `express` (router).
fn routers(code: &str, binds: &[Import]) -> BTreeMap<String, (bool, usize)> {
    let b = code.as_bytes();
    let mut found: Vec<(usize, bool)> = Vec::new();
    for pos in words(code, "express") {
        let k = pos + "express".len();
        if b.get(k) == Some(&b'(') {
            found.push((pos, true));
        } else if code[k..].starts_with(".Router") && b.get(skip_ws(b, k + 7)) == Some(&b'(') {
            found.push((pos, false));
        }
    }
    if (binds.iter()).any(|i| i.module == "express" && i.imported.as_deref() == Some("Router")) {
        for pos in words(code, "Router") {
            if b.get(skip_ws(b, pos + 6)) == Some(&b'(') && !code[..pos].ends_with('.') {
                found.push((pos, false));
            }
        }
    }
    let mut out = BTreeMap::new();
    for (pos, is_app) in found {
        let Some(lhs) = code[..pos].trim_end().strip_suffix('=') else {
            continue;
        };
        let lhs = lhs.trim_end();
        let lhs = lhs.rsplit_once(':').map_or(lhs, |(a, _)| a); // `r: Router =`
        if let Some(name) = trailing_ident(lhs.trim_end()) {
            out.insert(name, (is_app, pos));
        }
    }
    out
}

/// An argument as written: its source and, when it is a string literal or an
/// array of them, the strings.
#[derive(Clone)]
struct Arg {
    src: String,
    lit: Option<String>,
    strs: Option<Vec<String>>,
}

fn arg(raw: &str, item: &Item) -> Arg {
    let strs = match &item.value {
        Value::Arr(items) => (items.iter())
            .map(|i| match &i.value {
                Value::Str(s) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        _ => None,
    };
    Arg {
        src: item.source(raw),
        lit: match &item.value {
            Value::Str(s) => Some(s.clone()),
            _ => None,
        },
        strs,
    }
}

/// Arguments with arrays of middleware flattened.
fn flat(raw: &str, items: &[Item]) -> Vec<Arg> {
    let mut out = Vec::new();
    for item in items {
        match &item.value {
            Value::Arr(inner) => out.extend(flat(raw, inner)),
            _ => out.push(arg(raw, item)),
        }
    }
    out
}

struct RouteEv {
    verb: &'static str,
    path: Arg,
    handlers: Vec<Arg>,
}

enum Ev {
    Use(Vec<Arg>),
    Route(RouteEv),
}

fn verb_of(name: &str) -> Option<&'static str> {
    VERBS.iter().find(|(v, _)| *v == name).map(|(_, m)| *m)
}

/// A call `name(args)` whose name starts at `at`.
struct Call<'a> {
    at: usize,
    name: &'a str,
    items: Vec<Item>,
    end: usize,
}

fn call_at(src: &Src, at: usize) -> Option<Call<'_>> {
    let name = ident_at(&src.code, at);
    let k = skip_ws(src.code.as_bytes(), at + name.len());
    if src.code.as_bytes().get(k) != Some(&b'(') {
        return None;
    }
    let items = Parser { s: &src.raw, i: k }.array(0);
    let end = close(&src.code, k);
    Some(Call {
        at,
        name,
        items,
        end,
    })
}

/// The routes of `x.route(path).get(...).post(...)`.
fn chain(src: &Src, route: &Call) -> Vec<(usize, Ev)> {
    let Some(path) = route.items.first().map(|p| arg(&src.raw, p)) else {
        return vec![];
    };
    let (b, mut end, mut out) = (src.code.as_bytes(), route.end, Vec::new());
    while b.get(skip_ws(b, end)) == Some(&b'.') {
        let Some(call) = call_at(src, skip_ws(b, skip_ws(b, end) + 1)) else {
            break;
        };
        end = call.end;
        if let Some(verb) = verb_of(call.name) {
            let (path, handlers) = (path.clone(), flat(&src.raw, &call.items));
            let ev = Ev::Route(RouteEv {
                verb,
                path,
                handlers,
            });
            out.push((call.at, ev));
        }
    }
    out
}

/// The `use` and route calls on variable `var`, in source order.
fn events(src: &Src, var: &str) -> Vec<(usize, Ev)> {
    let (code, b) = (&src.code, src.code.as_bytes());
    let mut out = Vec::new();
    for pos in words(code, var) {
        let j = skip_ws(b, pos + var.len());
        if code[..pos].ends_with('.') || b.get(j) != Some(&b'.') {
            continue;
        }
        let Some(call) = call_at(src, skip_ws(b, j + 1)) else {
            continue;
        };
        if call.name == "use" {
            out.push((pos, Ev::Use(flat(&src.raw, &call.items))));
        } else if let (Some(verb), Some((path, rest))) =
            (verb_of(call.name), call.items.split_first())
        {
            let (path, handlers) = (arg(&src.raw, path), flat(&src.raw, rest));
            let ev = Ev::Route(RouteEv {
                verb,
                path,
                handlers,
            });
            out.push((pos, ev));
        } else if call.name == "route" {
            out.extend(chain(src, &call));
        }
    }
    out.sort_by_key(|(at, _)| *at);
    out
}

fn is_source(rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    EXTS.iter().any(|e| name.ends_with(e))
        && !name.ends_with(".d.ts")
        && !name.contains(".spec.")
        && !name.contains(".test.")
        && !test_path(rel)
}

/// Directories whose `package.json` has the dependency `express`: path to
/// (application name, source paths).
fn find_apps(
    root: &Path,
    files: &[String],
    out: &mut Vec<UiFlowDiagnostic>,
) -> BTreeMap<String, (String, Vec<String>)> {
    let mut apps = BTreeMap::new();
    for rel in files {
        let (dir, name) = rel.rsplit_once('/').unwrap_or(("", rel));
        let Some(Ok(text)) = (name == "package.json").then(|| read_source(root, rel)) else {
            continue;
        };
        if has_dep(&text, "express") {
            apps.insert(dir.into(), (dir_name(root, dir), vec![]));
        } else if ["fastify", "koa"].iter().any(|d| has_dep(&text, d)) {
            let message = "Fastify and Koa applications are not read; only Express is.".into();
            note(out, rel, 0, message);
        }
    }
    apps
}

/// Applications and their sources: (directory name, files by path).
fn read_apps(root: &Path, out: &mut Vec<UiFlowDiagnostic>) -> Vec<(String, Files)> {
    let walk = match walk(root) {
        Ok(walk) => walk,
        Err(e) => {
            gap(out, ".", 0, format!("Discovery settings not read: {e}"));
            return vec![];
        }
    };
    let mut apps = find_apps(root, &walk.files, out);
    if apps.is_empty() {
        return vec![];
    }
    for e in walk.errors {
        gap(out, ".", 0, format!("Directory entry not read: {e}"));
    }
    for rel in walk.files.into_iter().filter(|r| is_source(r)) {
        let owner = (apps.keys())
            .filter(|d| d.is_empty() || rel.starts_with(&format!("{d}/")))
            .max_by_key(|d| d.len())
            .cloned();
        let Some(files) = owner.and_then(|d| apps.get_mut(&d)).map(|a| &mut a.1) else {
            continue;
        };
        if files.len() == MAX_FILES {
            let message = format!(
                "Express scan stopped after {MAX_FILES} source files; later files are unknown."
            );
            gap(out, &rel, 0, message);
        }
        files.push(rel);
    }
    (apps.into_values())
        .map(|(name, rels)| {
            let files = (rels.into_iter().take(MAX_FILES))
                .filter_map(|rel| match read_source(root, &rel) {
                    Ok(text) => Some((rel, Src::new(text))),
                    Err(message) => {
                        gap(out, &rel, 0, message);
                        None
                    }
                })
                .collect();
            (name, files)
        })
        .collect()
}

/// A middleware's condition that holds for paths below `prefix`.
#[derive(Clone)]
struct Guard {
    prefix: String,
    cond: Condition,
}

struct Scan<'a> {
    files: &'a Files,
    access: &'a BTreeMap<String, AccessMeaning>,
    app: &'a str,
    flow: &'a mut UiFlow,
    visited: BTreeSet<(String, String)>,
    stack: Vec<(String, String)>,
}

/// `/a/b` from the parts, `""` for none.
fn path_of(parts: &[&str]) -> String {
    let parts: Vec<&str> = (parts.iter())
        .map(|p| p.trim_matches('/'))
        .filter(|p| !p.is_empty())
        .collect();
    match parts.is_empty() {
        true => String::new(),
        false => format!("/{}", parts.join("/")),
    }
}

/// Whether `path` is `prefix` or below it, segment-wise.
fn below(path: &str, prefix: &str) -> bool {
    prefix.is_empty()
        || path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|r| r.starts_with('/'))
}

impl Scan<'_> {
    fn run(&mut self) {
        let files = self.files;
        for (rel, src) in files {
            for (var, (is_app, _)) in &src.routers {
                if *is_app {
                    self.walk(rel, var, "", vec![]);
                }
            }
        }
        for (rel, src) in files {
            for (var, (_, at)) in &src.routers {
                if !self.visited.contains(&(rel.clone(), var.clone())) {
                    let message = format!(
                        "Router `{var}` is not mounted by an application; its routes are not listed."
                    );
                    gap(
                        &mut self.flow.diagnostics,
                        rel,
                        line_of(&src.code, *at),
                        message,
                    );
                }
            }
        }
    }

    fn walk(&mut self, file: &str, var: &str, prefix: &str, mut guards: Vec<Guard>) {
        let key = (file.to_string(), var.to_string());
        self.visited.insert(key.clone());
        if self.stack.contains(&key) || self.stack.len() >= MAX_DEPTH {
            let message =
                format!("Router `{var}` is mounted in a cycle or too deep; not followed.");
            gap(&mut self.flow.diagnostics, file, 0, message);
            return;
        }
        self.stack.push(key);
        let files = self.files;
        let src = &files[file];
        for (at, ev) in events(src, var) {
            let line = line_of(&src.code, at);
            match ev {
                Ev::Use(args) => self.use_call(file, at, prefix, &mut guards, args),
                Ev::Route(r) => self.route(file, line, prefix, &guards, &r),
            }
        }
        self.stack.pop();
    }

    fn use_call(
        &mut self,
        file: &str,
        at: usize,
        prefix: &str,
        guards: &mut Vec<Guard>,
        args: Vec<Arg>,
    ) {
        let line = line_of(&self.files[file].code, at);
        let lists = match self.each_mount(file, at, &args) {
            Some(lists) => lists,
            None => vec![args],
        };
        for args in lists {
            let (local, rest) = match args.first() {
                Some(a) if a.lit.is_some() => (a.lit.clone().unwrap_or_default(), &args[1..]),
                Some(a) if a.src.starts_with(['`', '[']) => {
                    let message = format!("Path of `use` is not a literal: {}", a.src);
                    gap(&mut self.flow.diagnostics, file, line, message);
                    let cond = Condition::Unknown(a.src.clone());
                    guards.push(Guard {
                        prefix: String::new(),
                        cond,
                    });
                    continue;
                }
                _ => (String::new(), &args[..]),
            };
            let abs = path_of(&[prefix, &local]);
            for a in rest {
                match self.mount_target(file, &a.src) {
                    Some((f, v)) => self.walk(&f, &v, &abs, guards.clone()),
                    None => {
                        if self.unread_module(file, &a.src) {
                            let message = format!(
                                "`{}` comes from a module that is not read (excluded folder, \
                                 alias or package); if it is a router, its routes are unknown.",
                                a.src
                            );
                            gap(&mut self.flow.diagnostics, file, line, message);
                        }
                        let cond = self.middleware(file, line, &a.src);
                        guards.extend(cond.map(|cond| Guard {
                            prefix: abs.clone(),
                            cond,
                        }));
                    }
                }
            }
        }
    }

    /// `list.forEach((r) => x.use(r.path, r.route))` over a literal array of
    /// `{ path, route }` in the same file: one argument list per entry.
    fn each_mount(&self, file: &str, at: usize, args: &[Arg]) -> Option<Vec<Vec<Arg>>> {
        let [path, route] = args else { return None };
        let p = path.src.strip_suffix(".path")?;
        (route.src == format!("{p}.route")).then_some(())?;
        let src = &self.files[file];
        let head = &src.code[..at];
        let call = head.rfind(".forEach(")?;
        let list = trailing_ident(&head[..call])?;
        let param = head[call + 9..].trim_start().trim_start_matches('(');
        (ident_at(param, 0) == p).then_some(())?;
        let Some(Item {
            value: Value::Arr(items),
            ..
        }) = declared_value(&src.raw, &list)
        else {
            return None;
        };
        (items.iter())
            .map(|item| {
                let Value::Obj(fields) = &item.value else {
                    return None;
                };
                let get = |k: &str| {
                    fields
                        .iter()
                        .find(|(n, _)| n == k)
                        .map(|(_, i)| arg(&src.raw, i))
                };
                Some(vec![get("path")?, get("route")?])
            })
            .collect()
    }

    /// The router a `use` argument names: a variable of this file, an import
    /// or `require('./x')`.
    fn mount_target(&self, file: &str, text: &str) -> Option<(String, String)> {
        let src = &self.files[file];
        let text = text.trim();
        if let Some((_, args)) = call_parts(text).filter(|(n, _)| *n == "require") {
            let Some(Value::Str(module)) = args.first().map(|a| &a.value) else {
                return None;
            };
            return self.router_of(&self.resolve(file, module)?, None);
        }
        if !is_name(text) {
            return None;
        }
        let (first, member) = match text.split_once('.') {
            Some((f, m)) => (f, Some(m)),
            None => (text, None),
        };
        if member.is_none() && src.routers.contains_key(first) {
            return Some((file.into(), first.into()));
        }
        let bind = src.binds.iter().find(|i| i.local == first)?;
        let target = self.resolve(file, &bind.module)?;
        match (member, bind.imported.as_deref()) {
            (None, imported) => self.router_of(&target, imported),
            (Some(m), None | Some("*")) => self.router_of(&target, Some(m)),
            _ => None,
        }
    }

    /// Whether `text` names an import or `require` of a relative module that is
    /// no file of the application.
    fn unread_module(&self, file: &str, text: &str) -> bool {
        let text = text.trim();
        let relative = |m: &str| m.starts_with('.') && self.resolve(file, m).is_none();
        if let Some((_, args)) = call_parts(text).filter(|(n, _)| *n == "require") {
            return matches!(args.first().map(|a| &a.value), Some(Value::Str(m)) if relative(m));
        }
        let first = text.split('.').next().unwrap_or(text);
        (self.files[file].binds.iter()).any(|i| i.local == first && relative(&i.module))
    }

    /// The application file `module` (relative) names, with extension or `index`.
    fn resolve(&self, from: &str, module: &str) -> Option<String> {
        if !module.starts_with('.') {
            return None;
        }
        let base = join(dir(from), module);
        let candidates = [base.clone()]
            .into_iter()
            .chain(EXTS.iter().map(|e| format!("{base}{e}")))
            .chain(EXTS.iter().map(|e| format!("{base}/index{e}")));
        candidates.into_iter().find(|c| self.files.contains_key(c))
    }

    /// The router `file` exports under `imported` (`None`: the default export).
    fn router_of(&self, file: &str, imported: Option<&str>) -> Option<(String, String)> {
        let src = self.files.get(file)?;
        let mine = |v: &str| Some((file.to_string(), v.to_string()));
        if let Some(name) = imported.filter(|n| *n != "*") {
            return src.routers.contains_key(name).then(|| mine(name))?;
        }
        for marker in ["export default", "module.exports ="] {
            if let Some(i) = src.code.find(marker) {
                let at = skip_ws(src.code.as_bytes(), i + marker.len());
                let name = ident_at(&src.code, at);
                if src.routers.contains_key(name) {
                    return mine(name);
                }
            }
        }
        match src.routers.keys().collect::<Vec<_>>().as_slice() {
            [only] => mine(only),
            _ => None,
        }
    }

    /// The condition of middleware `text`; `None` when it adds none.
    fn middleware(&mut self, file: &str, line: u32, text: &str) -> Option<Condition> {
        let text = text.trim();
        if error_handler(text) {
            return None;
        }
        let (name, args) = match call_parts(text) {
            Some((name, args)) => (name.to_string(), args),
            None if is_name(text) => (text.to_string(), vec![]),
            None => {
                let message = format!("Inline middleware is not read: {text}");
                note(&mut self.flow.diagnostics, file, line, message);
                return Some(Condition::Unknown(text.chars().take(80).collect()));
            }
        };
        if self.access.contains_key(&name) {
            return access_condition(self.access, &name, &args).unwrap_or_else(|name| {
                let message = format!("Arguments of middleware `{name}` are not literal: {text}");
                gap(&mut self.flow.diagnostics, file, line, message);
                Some(Condition::Unknown(text.into()))
            });
        }
        if self.passive(file, &name) {
            return None;
        }
        let message =
            format!("Middleware `{name}` is not listed in [views.access]; its access is unknown.");
        note(&mut self.flow.diagnostics, file, line, message);
        Some(Condition::Unknown(name))
    }

    /// Middleware of packages known not to decide access.
    fn passive(&self, file: &str, name: &str) -> bool {
        let first = name.split('.').next().unwrap_or(name);
        let module = (self.files[file].binds.iter())
            .find(|i| i.local == first && matches!(i.imported.as_deref(), None | Some("*")))
            .map(|i| i.module.as_str());
        PASSIVE_EXPRESS.contains(&name)
            || (name == "passport.initialize" && module == Some("passport"))
            || module.is_some_and(|m| PASSIVE.contains(&m))
    }

    fn route(&mut self, file: &str, line: u32, prefix: &str, guards: &[Guard], r: &RouteEv) {
        let RouteEv {
            verb,
            path,
            handlers,
        } = r;
        let Some((handler, middleware)) = handlers.split_last() else {
            return; // `app.get('env')`
        };
        let paths = match (&path.lit, &path.strs) {
            (Some(p), _) => vec![p.clone()],
            (_, Some(ps)) => ps.clone(),
            _ => {
                let message = format!("Route path is not a literal: {}", path.src);
                gap(&mut self.flow.diagnostics, file, line, message);
                return;
            }
        };
        let own: Vec<Condition> = (middleware.iter())
            .filter_map(|m| self.middleware(file, line, &m.src))
            .collect();
        let name = is_name(&handler.src).then(|| handler.src.clone());
        let site = name.as_deref().and_then(|h| self.declaration(file, h));
        for p in paths {
            let full = path_of(&[prefix, &p]);
            let mut conds: Vec<Condition> = (guards.iter())
                .filter(|g| below(&full, &g.prefix))
                .map(|g| g.cond.clone())
                .chain(own.iter().cloned())
                .collect();
            let mut seen = BTreeSet::new();
            conds.retain(|c| seen.insert(c.clone()));
            let access = match conds.len() {
                0 => AccessState::Open,
                1 => AccessState::Protected(conds.remove(0)),
                _ => AccessState::Protected(Condition::All(conds)),
            };
            let path = if full.is_empty() { "/".into() } else { full };
            self.flow.endpoints.push(EndpointAccess {
                method: (*verb).into(),
                path,
                access,
                application: Some(self.app.into()),
                file: file.into(),
                line,
                source: "express".into(),
                handler: name.clone(),
                sends: vec![],
            });
            if let (Some(handler), Some((f, l))) = (&name, &site) {
                self.flow.handler_sites.push(HandlerSite {
                    endpoint_file: file.into(),
                    endpoint_line: line,
                    handler: handler.clone(),
                    file: f.clone(),
                    line: *l,
                });
            }
        }
    }

    /// Where handler `h` (`fn` or `ctrl.fn`) is declared: in this file, or in
    /// the file its object or name is imported from.
    fn declaration(&self, file: &str, h: &str) -> Option<(String, u32)> {
        let src = &self.files[file];
        let find = |f: &str, name: &str| {
            let t = self.files.get(f)?;
            decl_line(t, name).map(|l| (f.to_string(), l))
        };
        match h.split_once('.') {
            None => find(file, h).or_else(|| {
                let bind = src.binds.iter().find(|i| i.local == h)?;
                find(
                    &self.resolve(file, &bind.module)?,
                    bind.imported.as_deref()?,
                )
            }),
            Some((obj, name)) => {
                let bind = src.binds.iter().find(|i| i.local == obj)?;
                find(&self.resolve(file, &bind.module)?, name)
            }
        }
    }
}

/// An inline error handler, `(err, req, res, next) => …` or `function (err, req,
/// res, next) {…}`: Express calls four-parameter functions only after an error,
/// so one never decides access.
fn error_handler(text: &str) -> bool {
    let Some(open) = text.find('(') else {
        return false;
    };
    let prefix = text[..open].trim();
    let prefix = prefix.strip_prefix("async").map_or(prefix, str::trim);
    let arrow = prefix.is_empty() && text[close(text, open)..].trim_start().starts_with("=>");
    let function = prefix.split_whitespace().next() == Some("function");
    (arrow || function) && Parser { s: text, i: open }.array(0).len() == 4
}

/// The line where `name` is declared as `const|let|var|function name` or
/// `exports.name =`.
fn decl_line(src: &Src, name: &str) -> Option<u32> {
    words(&src.code, name).into_iter().find_map(|pos| {
        let before = src.code[..pos].trim_end();
        let declared = before.ends_with("exports.")
            || matches!(
                trailing_ident(before).as_deref(),
                Some("const" | "let" | "var" | "function")
            );
        declared.then(|| line_of(&src.code, pos))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PKG: &str = r#"{"dependencies":{"express":"4"}}"#;

    fn run(files: &[(&str, &str)], access: &str) -> UiFlow {
        let dir = tempfile::tempdir().unwrap();
        let config = format!("[views.access]\n{access}");
        let config = (!access.is_empty()).then_some((".archguard/archguard.toml", config.as_str()));
        for (rel, text) in files.iter().copied().chain(config) {
            let path = dir.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        extract(dir.path())
    }

    fn ep<'a>(flow: &'a UiFlow, method: &str, path: &str) -> &'a EndpointAccess {
        (flow.endpoints.iter())
            .find(|e| e.method == method && e.path == path)
            .unwrap_or_else(|| panic!("no {method} {path} in {:#?}", flow.endpoints))
    }

    fn prot(c: Condition) -> AccessState {
        AccessState::Protected(c)
    }

    fn gaps(flow: &UiFlow) -> Vec<&str> {
        (flow.diagnostics.iter())
            .filter(|d| d.severity == DiagnosticSeverity::Gap)
            .map(|d| d.message.as_str())
            .collect()
    }

    const APP: &str = "const express = require('express');\nconst helmet = require('helmet');\n\
        const users = require('./routes/users');\nconst app = express();\napp.use(helmet());\n\
        app.use(express.json());\napp.get('/health', (req, res) => res.end());\n\
        app.use('/api/v1', users);\n";
    const USERS: &str = "const express = require('express');\nconst auth = require('../auth');\n\
        const ctl = require('../ctl');\nconst router = express.Router();\n\
        router.get('/ping', ctl.ping);\n\
        router\n  .route('/')\n  .post(auth('manageUsers'), ctl.create)\n  .get(auth('getUsers'), ctl.list);\n\
        module.exports = router;\n";

    #[test]
    fn composes_prefixes_across_files_with_route_chains_and_policy_args() {
        let flow = run(
            &[
                ("package.json", PKG),
                ("app.js", APP),
                ("routes/users.js", USERS),
                (
                    "ctl.js",
                    "const create = async () => {};\nexports.list = () => {};\n",
                ),
            ],
            "auth = \"policy:$arg|authenticated\"\n",
        );
        assert!(flow.complete, "{:#?}", flow.diagnostics);
        assert_eq!(flow.endpoints.len(), 4, "{:#?}", flow.endpoints);
        assert_eq!(ep(&flow, "GET", "/health").access, AccessState::Open);
        let post = ep(&flow, "POST", "/api/v1");
        assert_eq!(post.access, prot(Condition::Policy("manageUsers".into())));
        assert_eq!((post.file.as_str(), post.line), ("routes/users.js", 8));
        assert_eq!(post.handler.as_deref(), Some("ctl.create"));
        assert_eq!(
            ep(&flow, "GET", "/api/v1").access,
            prot(Condition::Policy("getUsers".into()))
        );
        assert_eq!(ep(&flow, "GET", "/api/v1/ping").access, AccessState::Open);
        let site = (flow.handler_sites.iter()).find(|s| s.handler == "ctl.create");
        assert_eq!(site.map(|s| (s.file.as_str(), s.line)), Some(("ctl.js", 1)));
    }

    #[test]
    fn bare_middleware_without_arguments_uses_the_fallback() {
        let flow = run(
            &[
                ("package.json", PKG),
                (
                    "app.js",
                    "const express = require('express');\nconst app = express();\n\
                     app.post('/a', auth, h);\napp.post('/b', auth('x'), h);\n",
                ),
            ],
            "auth = \"policy:$arg|authenticated\"\n",
        );
        assert_eq!(
            ep(&flow, "POST", "/a").access,
            prot(Condition::Authenticated)
        );
        assert_eq!(
            ep(&flow, "POST", "/b").access,
            prot(Condition::Policy("x".into()))
        );
    }

    #[test]
    fn unlisted_middleware_is_unknown_with_a_note_not_a_guess() {
        let flow = run(
            &[
                ("package.json", PKG),
                (
                    "app.js",
                    "const express = require('express');\nconst app = express();\n\
                     app.get('/a', isAuthenticated, h);\n",
                ),
            ],
            "",
        );
        assert_eq!(
            ep(&flow, "GET", "/a").access,
            prot(Condition::Unknown("isAuthenticated".into()))
        );
        assert!(flow.diagnostics.iter().any(|d| {
            d.severity == DiagnosticSeverity::Note && d.message.contains("isAuthenticated")
        }));
    }

    #[test]
    fn unknown_use_before_routes_is_unknown_not_open_and_scoped_use_stays_scoped() {
        let flow = run(
            &[
                ("package.json", PKG),
                (
                    "app.js",
                    "const express = require('express');\nconst app = express();\n\
                     app.get('/before', h);\napp.use(x);\napp.get('/after', h);\n\
                     app.use('/admin', gate);\napp.get('/admin/a', h);\napp.get('/other', h);\n",
                ),
            ],
            "gate = \"role:admin\"\n",
        );
        assert_eq!(ep(&flow, "GET", "/before").access, AccessState::Open);
        let unknown = prot(Condition::Unknown("x".into()));
        assert_eq!(ep(&flow, "GET", "/after").access, unknown);
        assert_eq!(
            ep(&flow, "GET", "/admin/a").access,
            prot(Condition::All(vec![
                Condition::Unknown("x".into()),
                Condition::Role("admin".into())
            ]))
        );
        assert_eq!(ep(&flow, "GET", "/other").access, unknown);
    }

    #[test]
    fn open_listed_middleware_adds_nothing() {
        let flow = run(
            &[
                ("package.json", PKG),
                (
                    "app.js",
                    "const express = require('express');\nconst app = express();\n\
                     app.use(limiter);\napp.get('/a', h);\n",
                ),
            ],
            "limiter = \"open\"\n",
        );
        assert_eq!(ep(&flow, "GET", "/a").access, AccessState::Open);
    }

    #[test]
    fn non_literal_path_and_non_literal_argument_are_gaps() {
        let flow = run(
            &[
                ("package.json", PKG),
                (
                    "app.js",
                    "const express = require('express');\nconst app = express();\n\
                     app.get(PATH, h);\napp.get('/ok', auth(perm), h);\n",
                ),
            ],
            "auth = \"policy:$arg\"\n",
        );
        assert!(!flow.complete);
        let gaps = gaps(&flow);
        assert!(
            gaps.iter()
                .any(|g| g.contains("Route path is not a literal: PATH")),
            "{gaps:?}"
        );
        assert!(gaps.iter().any(|g| g.contains("not literal")), "{gaps:?}");
        assert_eq!(flow.endpoints.len(), 1);
        assert!(matches!(
            &ep(&flow, "GET", "/ok").access,
            AccessState::Protected(Condition::Unknown(_))
        ));
    }

    #[test]
    fn unmounted_router_and_commented_routes() {
        let flow = run(
            &[
                ("package.json", PKG),
                (
                    "app.js",
                    "const express = require('express');\nconst app = express();\n\
                     // app.get('/gone', h);\n/* app.get('/gone2', h); */\napp.get('/real', h);\n",
                ),
                (
                    "lost.js",
                    "const express = require('express');\nconst r = express.Router();\nr.get('/x', h);\n",
                ),
            ],
            "",
        );
        assert_eq!(flow.endpoints.len(), 1, "{:#?}", flow.endpoints);
        assert!(
            gaps(&flow)
                .iter()
                .any(|g| g.contains("Router `r` is not mounted"))
        );
    }

    #[test]
    fn for_each_mount_list_and_all_verb() {
        let flow = run(
            &[
                ("package.json", PKG),
                (
                    "app.js",
                    "const express = require('express');\nconst a = require('./a');\n\
                     const app = express();\nconst list = [{ path: '/a', route: a }];\n\
                     list.forEach((r) => {\n  app.use(r.path, r.route);\n});\n",
                ),
                (
                    "a.js",
                    "const { Router } = require('express');\nconst r = Router();\nr.all('/x', h);\nmodule.exports = r;\n",
                ),
            ],
            "",
        );
        assert!(flow.complete, "{:#?}", flow.diagnostics);
        assert_eq!(ep(&flow, ANY_METHOD, "/a/x").access, AccessState::Open);
    }

    #[test]
    fn error_handlers_add_nothing_and_unread_router_modules_are_gaps() {
        let flow = run(
            &[
                ("package.json", PKG),
                (
                    "app.js",
                    "const express = require('express');\nconst lost = require('./bin/lost');\n\
                     const app = express();\napp.use((err, req, res, next) => next());\n\
                     app.use(function (err, req, res, next) {});\napp.get('/a', h);\n\
                     app.use('/x', lost);\n",
                ),
            ],
            "",
        );
        assert_eq!(ep(&flow, "GET", "/a").access, AccessState::Open);
        assert!(
            gaps(&flow)
                .iter()
                .any(|g| g.contains("`lost` comes from a module"))
        );
    }

    #[test]
    fn fastify_only_is_not_read() {
        let flow = run(
            &[("package.json", r#"{"dependencies":{"fastify":"4"}}"#)],
            "",
        );
        assert!(flow.endpoints.is_empty());
        assert!(
            flow.diagnostics
                .iter()
                .any(|d| d.message.contains("Fastify"))
        );
    }
}
