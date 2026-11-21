//! NestJS backend endpoint access extractor for ui-flow/v1 (ADR 0021).
//!
//! Text level, no TypeScript parser: comments and string contents are blanked
//! for searching, literals are read with the shared `lang_ts` parser. It fills
//! only `endpoints` and `diagnostics`.
//!
//! An application is a directory whose `package.json` names `@nestjs/common` or
//! `@nestjs/core`; its directory name is the endpoint's `application`. Its `.ts`
//! files are read (not `.d.ts`, `*.spec.ts`, `*.test.ts` or test folders).
//!
//! Recognized:
//! - Routes: `@Controller('prefix')`, `@Controller(['a', 'b'])`,
//!   `@Controller({ path, version })` on a class and `@Get/@Post/@Put/@Patch/
//!   @Delete/@Options/@Head/@All(path)` on its methods (`@All` gives method
//!   `ANY`); a path is a string, a template literal whose holes are constants,
//!   an array of those, or a constant: `const X = '…'`, `const X = […]`, an
//!   `enum E { M = '…' }` member or a `const O = { M: '…' }` field anywhere in
//!   the application. Route parameters (`:id`) stay. `app.setGlobalPrefix('api',
//!   { exclude })` anywhere in the application prefixes every route whose
//!   controller and method path is not listed in `exclude` (strings or
//!   `{ path }`, an array or a constant array). `app.enableVersioning({ type:
//!   VersioningType.URI, prefix, defaultVersion })` adds `/<prefix><version>`
//!   from `@Version(…)` on the method, else the controller's `version`, else
//!   `defaultVersion`; `VERSION_NEUTRAL` adds nothing; other versioning types
//!   leave the path alone.
//! - Guards: `@UseGuards(X, new Y(), AuthGuard('jwt'))` on class and method, and
//!   global guards (`{ provide: APP_GUARD, useClass: X }`, `app.useGlobalGuards(new
//!   X())`). Every guard must pass (`all`). A guard maps through `[views.access]`
//!   by its name (`role:$arg` and `policy:$arg` take the call's arguments; an
//!   `open` guard adds no condition); `AuthGuard(…)` imported from
//!   `@nestjs/passport` is `authenticated`; any other guard is `custom(name)`.
//!   Global guards apply to every controller of the application. When it has
//!   several root modules (`NestFactory.create(M)` with different `M`), a global
//!   guard declared in a root module (in its `providers`, directly or through a
//!   spread `const` array of the same file, or by `useGlobalGuards` in the file
//!   that creates it) applies to that module's `controllers` (names, or a spread
//!   constant array) only.
//! - Metadata: `@Roles('a', E.M)` and `@SetMetadata('roles', […])` mean any of
//!   the roles, but only together with a guard in force (global, class or method)
//!   that reads the key (`reflector.getAllAndOverride/getAllAndMerge/get(KEY, …)`
//!   in a class of the application); otherwise there is no condition and a `note`.
//!   The guard's lookup decides: override (method replaces class), merge (any of
//!   both) or handler only. Guards add up.
//! - Decorators listed in `[views.access]` by name: `open` (`@Public()`) is a
//!   marker for the metadata key its definition sets (`SetMetadata(KEY, true)`).
//!   It removes only the guards that read that key; `Open` only when no guard
//!   remains and at least one honoured it, else the remaining guards apply. An
//!   unreadable key is a gap and no marker. A route without any guard is
//!   `Undeclared`. Any other meaning is one more condition.
//! - Composite decorators defined in the application (`const D = (p) => …` or
//!   `function D(p) {…}` calling `UseGuards`, `Roles`, `SetMetadata` or a listed
//!   name) are followed one level: their guards, roles and listed parts count as
//!   if written at the use site; `Roles(...p)` takes the use site's arguments.
//!   `SetMetadata(key, p)` forwards the use site's options object (Immich's
//!   `@Authenticated({ permission: Permission.AssetRead, admin: true })`):
//!   `public: true` = `open`, `admin: true` = `role(admin)`, `role`/`roles` = any
//!   of the roles, `permission` = `policy(<value>)` with enum members resolved to
//!   their string (an unresolved member keeps its path, `false` adds nothing).
//!   Such a decorator counts as access only when some use site in the
//!   application passes one of these keys; its other keys are a `note` each.
//! - `handler` is `Controller.method`; `sends` are the `new X(…)` passed to
//!   `commandBus.execute` / `queryBus.execute` (`@nestjs/cqrs`). Service calls
//!   are no sends.
//!
//! Not recognized (gap, never a guess): non-constant paths, prefixes, versions,
//! guards (`...guards`) or roles; composite decorators defined several times or
//! outside the application; decorators applied by code (`applyDecorators(…)(C)`);
//! routes of controllers inherited from a base class; `RouterModule` paths;
//! guards that read other metadata (their meaning comes from `[views.access]`).
use crate::extract_ctx::{
    ExtractContext, dir_name, finish, gap, read_source, walk, with_root_config,
};
use crate::lang_ts::{
    Item, Parser, Value, block_end, call_parts, close, fill_all, ident_at, is_ident_char, is_name,
    line_of, skip_generic, skip_ws, words,
};
use crate::lang_ts_decl::{has_dep, imports, mask};
use crate::services::support::test_path;
use archguard_core::{
    ANY_METHOD, AccessMeaning, AccessState, Condition, DiagnosticSeverity, EndpointAccess, UiFlow,
    UiFlowDiagnostic,
};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

const MAX_FILES: usize = 20_000;
const NEST_DEPS: [&str; 2] = ["@nestjs/common", "@nestjs/core"];
const ROUTES: [(&str, &str); 8] = [
    ("Get", "GET"),
    ("Post", "POST"),
    ("Put", "PUT"),
    ("Patch", "PATCH"),
    ("Delete", "DELETE"),
    ("Options", "OPTIONS"),
    ("Head", "HEAD"),
    ("All", ANY_METHOD),
];
const MODIFIERS: [&str; 9] = [
    "public",
    "private",
    "protected",
    "async",
    "static",
    "readonly",
    "override",
    "abstract",
    "declare",
];
/// Option keys of a forwarded options object that carry access.
const OPTION_KEYS: [&str; 5] = ["public", "admin", "role", "roles", "permission"];

/// Endpoints of every NestJS application below `root` with their declared
/// access; `[views.access]` comes from the root's configuration.
pub fn extract(root: &Path) -> UiFlow {
    with_root_config(root, extract_with)
}

/// [`extract`] for one run.
pub fn extract_with(ctx: &ExtractContext) -> UiFlow {
    let mut flow = UiFlow::default();
    let apps = apps::read(ctx.root, &mut flow.diagnostics);
    if let (Some(e), false) = (&ctx.config_error, apps.is_empty()) {
        let message = format!("Configuration not read; [views.access] not applied: {e}");
        note(&mut flow.diagnostics, ".", 0, message);
    }
    for app in &apps {
        Scan::new(app, &ctx.config.views.access).endpoints(&mut flow);
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

/// Applications and their sources.
mod apps {
    use super::*;

    pub(super) struct App {
        pub(super) name: String,
        pub(super) files: Vec<Src>,
    }

    /// A source: `code` has comments and string contents blanked (offsets
    /// unchanged) for searching, `raw` is read for literals.
    pub(super) struct Src {
        pub(super) rel: String,
        pub(super) raw: String,
        pub(super) code: String,
        pub(super) decos: Vec<Deco>,
        /// Imports `AuthGuard` from `@nestjs/passport`.
        pub(super) passport: bool,
    }

    /// `@name` or `@name(args)`; `args` hold offsets into the source's `raw`.
    pub(super) struct Deco {
        pub(super) name: String,
        pub(super) at: usize,
        pub(super) end: usize,
        pub(super) args: Vec<Item>,
    }

    impl Src {
        fn new(rel: String, raw: String) -> Self {
            let code = mask(&raw, false);
            let b = code.as_bytes();
            let mut decos = Vec::new();
            for (at, _) in code.match_indices('@') {
                let mut j = at + 1;
                while let w @ 1.. = ident_at(&code, j).len() {
                    j += w;
                    if b.get(j) != Some(&b'.') {
                        break;
                    }
                    j += 1;
                }
                let name = code[at + 1..j].trim_end_matches('.');
                if name.is_empty() || name.starts_with(|c: char| c.is_ascii_digit()) {
                    continue;
                }
                let k = skip_ws(b, j);
                let (args, end) = if b.get(k) == Some(&b'(') {
                    (Parser { s: &raw, i: k }.array(0), close(&code, k))
                } else {
                    (vec![], at + 1 + name.len())
                };
                decos.push(Deco {
                    name: name.into(),
                    at,
                    end,
                    args,
                });
            }
            let passport = imports(&raw).iter().any(|i| {
                i.module == "@nestjs/passport" && i.imported.as_deref() == Some("AuthGuard")
            });
            Src {
                rel,
                raw,
                code,
                decos,
                passport,
            }
        }

        pub(super) fn line(&self, at: usize) -> u32 {
            line_of(&self.code, at)
        }
    }

    fn is_source(rel: &str) -> bool {
        let name = rel.rsplit('/').next().unwrap_or(rel);
        name.ends_with(".ts")
            && !name.ends_with(".d.ts")
            && !name.contains(".spec.")
            && !name.contains(".test.")
            && !test_path(rel)
    }

    /// NestJS applications below `root`; unreadable sources are gaps.
    pub(super) fn read(root: &Path, out: &mut Vec<UiFlowDiagnostic>) -> Vec<App> {
        let walk = match walk(root) {
            Ok(walk) => walk,
            Err(e) => {
                gap(out, ".", 0, format!("Discovery settings not read: {e}"));
                return vec![];
            }
        };
        let mut apps: BTreeMap<String, (String, Vec<String>)> = BTreeMap::new();
        for rel in &walk.files {
            let (dir, name) = rel.rsplit_once('/').unwrap_or(("", rel));
            if name == "package.json"
                && read_source(root, rel).is_ok_and(|t| NEST_DEPS.iter().any(|d| has_dep(&t, d)))
            {
                apps.insert(dir.into(), (dir_name(root, dir), vec![]));
            }
        }
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
            if let Some(dir) = owner {
                let files = &mut apps.get_mut(&dir).expect("key from the map").1;
                if files.len() == MAX_FILES {
                    let message = format!(
                        "NestJS scan stopped after {MAX_FILES} source files; later files are unknown."
                    );
                    gap(out, &rel, 0, message);
                }
                files.push(rel);
            }
        }
        (apps.into_values())
            .map(|(name, rels)| App {
                name,
                files: (rels.into_iter().take(MAX_FILES))
                    .filter_map(|rel| match read_source(root, &rel) {
                        Ok(text) => Some(Src::new(rel, text)),
                        Err(message) => {
                            gap(out, &rel, 0, message);
                            None
                        }
                    })
                    .collect(),
            })
            .collect()
    }
}
use apps::{App, Deco, Src};

/// A literal string or a list of item sources, by constant name
/// (`X`, `E.M`, `O.k`); `None` when two declarations disagree.
#[derive(Default)]
struct Consts(HashMap<String, Option<Def>>);

#[derive(Clone, PartialEq)]
enum Def {
    Str(String),
    List(Vec<String>),
}

impl Consts {
    fn read(app: &App) -> Self {
        let mut consts = Consts::default();
        for src in &app.files {
            let (code, b) = (&src.code, src.code.as_bytes());
            for pos in words(code, "enum") {
                let at = skip_ws(b, pos + 4);
                let name = ident_at(code, at);
                let open = skip_ws(b, at + name.len());
                if !name.is_empty() && b.get(open) == Some(&b'{') {
                    consts.enum_members(name, &src.raw, open);
                }
            }
            for pos in words(code, "const") {
                let at = skip_ws(b, pos + 5);
                let name = ident_at(code, at);
                if let (false, Some(eq)) = (name.is_empty(), assignment(code, at + name.len())) {
                    let item = Parser {
                        s: &src.raw,
                        i: eq + 1,
                    }
                    .value(0);
                    consts.collect(name, &src.raw, &item);
                }
            }
        }
        consts
    }

    /// String members `M = '…'` of `enum name {` at `open`.
    fn enum_members(&mut self, name: &str, raw: &str, open: usize) {
        let mut p = Parser {
            s: raw,
            i: open + 1,
        };
        loop {
            p.ws();
            let key = ident_at(raw, p.i).to_string();
            if key.is_empty() {
                return;
            }
            p.i += key.len();
            p.ws();
            if p.peek() == b'=' {
                p.i += 1;
                if let Value::Str(s) = p.value(0).value {
                    self.insert(format!("{name}.{key}"), Def::Str(s));
                }
            }
            p.ws();
            if p.peek() != b',' {
                return;
            }
            p.i += 1;
        }
    }

    fn collect(&mut self, name: &str, text: &str, item: &Item) {
        match &item.value {
            Value::Str(s) => self.insert(name.into(), Def::Str(s.clone())),
            Value::Arr(items) => {
                let list = items.iter().map(|i| i.source(text)).collect();
                self.insert(name.into(), Def::List(list));
            }
            Value::Obj(fields) => {
                for (key, v) in fields {
                    if let Value::Str(s) = &v.value {
                        self.insert(format!("{name}.{key}"), Def::Str(s.clone()));
                    }
                }
            }
            Value::Raw(r) => {
                if let Some(inner) = r.strip_suffix("as const").map(str::trim_end)
                    && !inner.ends_with("as const")
                {
                    self.collect(name, inner, &reparse(inner));
                }
            }
            Value::Tpl(_) => {}
        }
    }

    fn insert(&mut self, name: String, def: Def) {
        let entry = self.0.entry(name).or_insert_with(|| Some(def.clone()));
        if entry.as_ref() != Some(&def) {
            *entry = None;
        }
    }

    fn str(&self, name: &str) -> Option<String> {
        match self.0.get(name) {
            Some(Some(Def::Str(s))) => Some(s.clone()),
            _ => None,
        }
    }

    fn list(&self, name: &str) -> Option<&[String]> {
        match self.0.get(name) {
            Some(Some(Def::List(l))) => Some(l),
            _ => None,
        }
    }

    /// A string literal, a template literal whose holes are constants, or a constant.
    fn text(&self, item: &Item) -> Option<String> {
        match &item.value {
            Value::Str(s) => Some(s.clone()),
            Value::Tpl(t) => fill_all(t, |h| self.str(h.trim())),
            Value::Raw(r) if name_path(r) => self.str(r),
            _ => None,
        }
    }

    /// [`Self::text`], else a member path kept as written (`Permission.X`).
    fn name(&self, item: &Item) -> Option<String> {
        self.text(item).or_else(|| match &item.value {
            Value::Raw(r) if name_path(r) && r.contains('.') => Some(r.clone()),
            _ => None,
        })
    }

    /// Strings of a literal, an array (nested arrays flattened) or a constant list.
    fn strings(
        &self,
        text: &str,
        item: &Item,
        name: fn(&Self, &Item) -> Option<String>,
    ) -> Option<Vec<String>> {
        let items = |list: &[Item], text: &str| -> Option<Vec<String>> {
            let parts: Option<Vec<Vec<String>>> =
                list.iter().map(|i| self.strings(text, i, name)).collect();
            parts.map(|p| p.concat())
        };
        match &item.value {
            Value::Arr(list) => items(list, text),
            Value::Raw(r) if self.list(r).is_some() => {
                let parsed: Vec<(String, Item)> = (self.list(r)?.iter())
                    .map(|s| (s.clone(), reparse(s)))
                    .collect();
                let parts: Option<Vec<Vec<String>>> = (parsed.iter())
                    .map(|(s, i)| match i.value {
                        Value::Arr(_) => None, // shortcut: nested constant lists are not read
                        _ => self.strings(s, i, name),
                    })
                    .collect();
                parts.map(|p| p.concat())
            }
            _ => name(self, item).map(|s| vec![s]),
        }
    }
}

/// Offset of the `=` that assigns the declaration whose name ends at `from`
/// (a type annotation skipped); none for `==`, `=>` or a `;` first.
fn assignment(code: &str, from: usize) -> Option<usize> {
    let b = code.as_bytes();
    let mut depth = 0usize;
    for i in from..b.len() {
        match b[i] {
            b'(' | b'[' | b'{' | b'<' => depth += 1,
            b')' | b']' | b'}' | b'>' if b[i - 1] != b'=' => depth = depth.checked_sub(1)?,
            b';' => return None,
            b'=' if depth == 0 && !matches!(b.get(i + 1), Some(b'=' | b'>')) => return Some(i),
            _ => {}
        }
    }
    None
}

/// A name or member path (`X`, `E.M`); `lang_ts::is_name` also takes `...x`.
fn name_path(s: &str) -> bool {
    is_name(s) && s.split('.').all(|p| !p.is_empty())
}

fn reparse(text: &str) -> Item {
    Parser { s: text, i: 0 }.value(0)
}

fn field<'i>(item: &'i Item, key: &str) -> Option<&'i Item> {
    match &item.value {
        Value::Obj(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
        _ => None,
    }
}

fn raw_is(item: &Item, text: &str) -> bool {
    matches!(&item.value, Value::Raw(r) if r == text)
}

fn combine(mut items: Vec<Condition>, many: fn(Vec<Condition>) -> Condition) -> Option<Condition> {
    match items.len() {
        0 => None,
        1 => items.pop(),
        _ => Some(many(items)),
    }
}

/// Where a diagnostic points.
#[derive(Clone)]
struct Site<'a> {
    file: &'a str,
    line: u32,
}

/// What a guard or listed decorator grants.
#[derive(Clone)]
enum Grant {
    Open,
    Cond(Condition),
}

/// The access declarations of one level (global, class, method).
#[derive(Default)]
struct Level {
    /// A guard mapped to `open` in `[views.access]`.
    opened: bool,
    /// Guards by name (`None`: a listed decorator) with what they grant.
    guards: Vec<(Option<String>, Condition)>,
    /// Metadata conditions by key; they count only with a guard that reads the key.
    meta: Vec<(String, Condition)>,
    /// Public markers by metadata key (`""`: key not readable).
    public: Vec<String>,
}

/// How a guard looks metadata up: merged (`getAllAndMerge`) or the handler's
/// value over the class's, and which levels its targets cover (`getHandler()`,
/// `getClass()`; targets not readable count as both).
#[derive(Clone, Copy, PartialEq)]
struct Lookup {
    merge: bool,
    handler: bool,
    class: bool,
}

/// A composite decorator's definition: its first parameter and the calls of
/// its body, each with the source of its arguments.
struct Composite {
    param: Option<String>,
    calls: Vec<(String, String)>,
    file: String,
    line: u32,
}

/// A global guard and the controllers it applies to (`None`: all).
struct Global {
    grant: Grant,
    name: String,
    scope: Option<BTreeSet<String>>,
}

/// One application's reading.
struct Scan<'a> {
    app: &'a App,
    access: &'a BTreeMap<String, AccessMeaning>,
    consts: Consts,
    composites: HashMap<String, Vec<Composite>>,
    /// Composite decorators that forward options with an access key somewhere.
    keyed: HashSet<String>,
    /// Metadata key -> guards that read it with their lookup.
    readers: HashMap<String, Vec<(String, Lookup)>>,
}

impl<'a> Scan<'a> {
    fn new(app: &'a App, access: &'a BTreeMap<String, AccessMeaning>) -> Self {
        let mut scan = Scan {
            app,
            access,
            consts: Consts::read(app),
            composites: HashMap::new(),
            keyed: HashSet::new(),
            readers: HashMap::new(),
        };
        scan.readers = meta::readers(&scan);
        scan.composites = composite::definitions(&scan);
        let keyed: HashSet<String> = (app.files.iter().flat_map(|s| &s.decos))
            .filter(|d| {
                (scan.composites.get(&d.name))
                    .is_some_and(|defs| defs.iter().any(|c| scan.forwards(c)))
            })
            .filter(|d| {
                (d.args.first()).is_some_and(|a| OPTION_KEYS.iter().any(|k| field(a, k).is_some()))
            })
            .map(|d| d.name.clone())
            .collect();
        scan.keyed = keyed;
        scan
    }

    fn unknown(&self, out: &mut Vec<UiFlowDiagnostic>, site: &Site, text: &str) -> Condition {
        gap(
            out,
            site.file,
            site.line,
            format!("Access condition not readable: {text}"),
        );
        Condition::Unknown(text.into())
    }

    /// `[views.access]` meaning of `name` with the use site's arguments.
    fn listed(
        &self,
        out: &mut Vec<UiFlowDiagnostic>,
        site: &Site,
        name: &str,
        text: &str,
        args: &[Item],
    ) -> Option<Grant> {
        use AccessMeaning as M;
        let atoms = |atom: fn(String) -> Condition, out: &mut Vec<UiFlowDiagnostic>| {
            let names: Option<Vec<String>> = (args.iter())
                .map(|a| self.consts.strings(text, a, Consts::name))
                .collect::<Option<Vec<_>>>()
                .map(|n| n.concat());
            match names.and_then(|n| combine(n.into_iter().map(atom).collect(), Condition::Any)) {
                Some(c) => c,
                None => self.unknown(out, site, &format!("{name}({})", sources(text, args))),
            }
        };
        Some(Grant::Cond(
            match self.access.get(name)?.at_use_site(!args.is_empty()) {
                M::Open => return Some(Grant::Open),
                M::Authenticated => Condition::Authenticated,
                M::Never => Condition::Never,
                M::Role(n) => Condition::Role(n.clone()),
                M::Policy(n) => Condition::Policy(n.clone()),
                M::RoleFromArguments => atoms(Condition::Role, out),
                M::PolicyFromArguments => atoms(Condition::Policy, out),
                // `at_use_site` resolved it; a fallback is never a `$arg` form.
                M::FromArgumentsOr(..) => self.unknown(out, site, name),
            },
        ))
    }

    /// One `UseGuards` argument.
    fn guard(
        &self,
        out: &mut Vec<UiFlowDiagnostic>,
        site: &Site,
        src: &Src,
        text: &str,
        item: &Item,
    ) -> (Grant, String) {
        let source = item.source(text);
        let expr = source
            .strip_prefix("new ")
            .map_or(source.as_str(), str::trim_start);
        let (name, args, call) = match call_parts(expr) {
            Some((name, args)) => (name, args, true),
            None if name_path(expr) => (expr, vec![], false),
            None => return (Grant::Cond(self.unknown(out, site, &source)), source),
        };
        let grant = match self.listed(out, site, name, expr, &args) {
            Some(grant) => grant,
            None => Grant::Cond(if name == "AuthGuard" && call && src.passport {
                Condition::Authenticated
            } else {
                Condition::Custom(name.into())
            }),
        };
        (grant, name.into())
    }

    /// `@Roles(…)` arguments: any of the roles.
    fn roles(
        &self,
        out: &mut Vec<UiFlowDiagnostic>,
        site: &Site,
        text: &str,
        args: &[Item],
    ) -> Condition {
        let names: Option<Vec<Vec<String>>> = args
            .iter()
            .map(|a| self.consts.strings(text, a, Consts::name))
            .collect();
        match names.and_then(|n| {
            combine(
                n.concat().into_iter().map(Condition::Role).collect(),
                Condition::Any,
            )
        }) {
            Some(c) => c,
            None => self.unknown(out, site, &format!("roles {}", sources(text, args))),
        }
    }

    /// Adds a grant; with a metadata `key` it is metadata, else a listed decorator.
    fn add(level: &mut Level, grant: Grant, key: Option<&str>) {
        match (grant, key) {
            (Grant::Open, Some(k)) => level.public.push(k.into()),
            (Grant::Open, None) => level.opened = true,
            (Grant::Cond(c), Some(k)) => level.meta.push((k.into(), c)),
            (Grant::Cond(c), None) => level.guards.push((None, c)),
        }
    }

    /// The declarations of one class or method.
    fn level(&self, out: &mut Vec<UiFlowDiagnostic>, src: &Src, decos: &[&Deco]) -> Level {
        let mut level = Level::default();
        for d in decos {
            let site = Site {
                file: &src.rel,
                line: src.line(d.at),
            };
            let text = src.raw.as_str();
            if let Some(grant) = self.listed(out, &site, &d.name, text, &d.args) {
                // A listed decorator: `open` is the endpoint's, other meanings add up.
                meta::add_listed(self, out, &site, &mut level, grant, &d.name);
                continue;
            }
            match d.name.as_str() {
                "UseGuards" => {
                    for a in &d.args {
                        let guard = self.guard(out, &site, src, text, a);
                        meta::add_guard(&mut level, guard);
                    }
                }
                "Roles" => {
                    let c = self.roles(out, &site, text, &d.args);
                    Self::add(&mut level, Grant::Cond(c), Some(&meta::roles_key(self)));
                }
                "SetMetadata"
                    if d.args.first().and_then(|k| self.consts.text(k)).as_deref()
                        == Some("roles") =>
                {
                    let c = self.roles(out, &site, text, &d.args[1..]);
                    Self::add(&mut level, Grant::Cond(c), Some("roles"));
                }
                name if self.composites.contains_key(name) => {
                    composite::apply(self, out, &site, src, d, &mut level)
                }
                _ => {}
            }
        }
        level
    }

    /// Whether a composite forwards its parameter as metadata.
    fn forwards(&self, c: &Composite) -> bool {
        let Some(param) = &c.param else { return false };
        (c.calls.iter()).any(|(name, args)| {
            let args = Parser { s: args, i: 0 }.array(0);
            name == "SetMetadata" && args.get(1).is_some_and(|a| raw_is(a, param))
        })
    }
}

/// Composite decorators: definitions and their use.
mod composite {
    use super::*;

    const PARTS: [&str; 3] = ["UseGuards", "Roles", "SetMetadata"];

    /// Every `const D = (p) => …` / `function D(p) {…}` of the application whose
    /// body calls a part or a `[views.access]` name, by `D`.
    pub(super) fn definitions(scan: &Scan) -> HashMap<String, Vec<Composite>> {
        let mut defs: HashMap<String, Vec<Composite>> = HashMap::new();
        for src in &scan.app.files {
            for kw in ["const", "function"] {
                for pos in words(&src.code, kw) {
                    if let Some((name, def)) = definition(scan, src, kw, pos) {
                        defs.entry(name).or_default().push(def);
                    }
                }
            }
        }
        defs
    }

    /// The composite declared by keyword `kw` at `pos`, if it is one.
    fn definition(scan: &Scan, src: &Src, kw: &str, pos: usize) -> Option<(String, Composite)> {
        let (code, b) = (&src.code, src.code.as_bytes());
        let at = skip_ws(b, pos + kw.len());
        let name = ident_at(code, at);
        let reserved = PARTS.contains(&name) || ROUTES.iter().any(|(r, _)| *r == name);
        if name.is_empty() || reserved {
            return None;
        }
        let mut i = skip_ws(b, at + name.len());
        if kw == "const" {
            (b.get(i) == Some(&b'=')).then_some(())?;
            i = skip_ws(b, i + 1);
            if code[i..].starts_with("async") {
                i = skip_ws(b, i + 5);
            }
        }
        (b.get(i) == Some(&b'(')).then_some(())?;
        let params = close(code, i);
        let first = src
            .raw
            .get(i + 1..params.saturating_sub(1))
            .unwrap_or("")
            .trim_start();
        let first = ident_at(first.strip_prefix("...").unwrap_or(first), 0);
        let calls = calls_in(scan, src, body(code, params, kw == "const")?);
        let def = Composite {
            param: Some(first.to_string()).filter(|p| !p.is_empty()),
            calls,
            file: src.rel.clone(),
            line: src.line(pos),
        };
        (!def.calls.is_empty()).then(|| (name.to_string(), def))
    }

    /// Calls of parts and `[views.access]` names in `body`, in source order.
    fn calls_in(scan: &Scan, src: &Src, (from, to): (usize, usize)) -> Vec<(String, String)> {
        let (code, b) = (&src.code, src.code.as_bytes());
        let names = PARTS
            .iter()
            .copied()
            .chain(scan.access.keys().map(String::as_str));
        let mut calls = Vec::new();
        for part in names {
            for p in words(&code[from..to], part) {
                let open = skip_ws(b, from + p + part.len());
                if b.get(open) == Some(&b'(') && b.get((from + p).wrapping_sub(1)) != Some(&b'.') {
                    let args = src.raw[open..close(code, open)].to_string();
                    calls.push((from + p, part.to_string(), args));
                }
            }
        }
        calls.sort();
        calls.into_iter().map(|(_, n, a)| (n, a)).collect()
    }

    /// The body after a parameter list ending at `params`: a block, or for an
    /// arrow function an expression.
    fn body(code: &str, params: usize, arrow: bool) -> Option<(usize, usize)> {
        let b = code.as_bytes();
        let mut i = params;
        if arrow {
            // shortcut: the first `=>` after the parameters ends the return type;
            // a function type in the return type would cut it short.
            let after = code[params..].trim_start();
            if !(after.starts_with("=>") || after.starts_with(':')) {
                return None;
            }
            i = params + code[params..].find("=>")? + 2;
            i = skip_ws(b, i);
            if b.get(i) != Some(&b'{') {
                let end = Parser { s: code, i }.raw(i).end;
                return Some((i, end));
            }
        } else {
            i += code[params..].find('{')?;
        }
        Some((i, block_end(code, i, b'{', b'}')))
    }

    /// The one definition of composite `name`; several are a gap.
    fn single<'c>(
        scan: &'c Scan,
        out: &mut Vec<UiFlowDiagnostic>,
        site: &Site,
        name: &str,
    ) -> Option<&'c Composite> {
        let defs = &scan.composites[name];
        if let [c] = defs.as_slice() {
            return Some(c);
        }
        let places: Vec<String> = (defs.iter())
            .map(|c| format!("{}:{}", c.file, c.line))
            .collect();
        let message = format!(
            "Decorator @{name} is defined {} times ({}); it was not followed.",
            defs.len(),
            places.join(", ")
        );
        gap(out, site.file, site.line, message);
        None
    }

    /// `p` or `...p` for the composite's first parameter `p`.
    fn is_param(c: &Composite, a: &Item) -> bool {
        let p = c.param.as_deref().unwrap_or("\0");
        raw_is(a, p) || raw_is(a, &format!("...{p}"))
    }

    /// The parts of composite `d` at its use site.
    pub(super) fn apply(
        scan: &Scan,
        out: &mut Vec<UiFlowDiagnostic>,
        site: &Site,
        src: &Src,
        d: &Deco,
        level: &mut Level,
    ) {
        let Some(c) = single(scan, out, site, &d.name) else {
            return;
        };
        let is_param = |a: &Item| is_param(c, a);
        for (name, text) in &c.calls {
            let args = Parser { s: text, i: 0 }.array(0);
            if let Some(grant) = scan.listed(out, site, name, text, &args) {
                meta::add_listed(scan, out, site, level, grant, name);
                continue;
            }
            let key = args.first().and_then(|k| scan.consts.text(k));
            let roles_key = name == "SetMetadata" && key.as_deref() == Some("roles");
            let own = if roles_key { &args[1..] } else { &args[..] };
            // `Roles(...p)`, `UseGuards(...p)`: the use site's arguments.
            let (text, own) = if own.iter().any(is_param) {
                (src.raw.as_str(), d.args.as_slice())
            } else {
                (text.as_str(), own)
            };
            match name.as_str() {
                "UseGuards" => {
                    for a in own {
                        meta::add_guard(level, scan.guard(out, site, src, text, a));
                    }
                }
                "Roles" => {
                    let c = Grant::Cond(scan.roles(out, site, text, own));
                    Scan::add(level, c, Some(&meta::roles_key(scan)))
                }
                "SetMetadata" if roles_key => {
                    let c = Grant::Cond(scan.roles(out, site, text, own));
                    Scan::add(level, c, Some("roles"))
                }
                "SetMetadata"
                    if args.get(1).is_some_and(is_param) && scan.keyed.contains(&d.name) =>
                {
                    let key = meta::first_key(scan, text, &args);
                    options(scan, out, site, src, (d, &key), level);
                }
                _ => {}
            }
        }
    }

    /// The options object a composite forwards (Immich's `@Authenticated`).
    fn options(
        scan: &Scan,
        out: &mut Vec<UiFlowDiagnostic>,
        site: &Site,
        src: &Src,
        (d, meta_key): (&Deco, &str),
        level: &mut Level,
    ) {
        let Some(arg) = d.args.first() else { return };
        let Value::Obj(fields) = &arg.value else {
            let c = scan.unknown(out, site, &arg.source(&src.raw));
            return Scan::add(level, Grant::Cond(c), Some(meta_key));
        };
        for (key, v) in fields {
            if raw_is(v, "false") {
                continue;
            }
            let flag = |out: &mut Vec<UiFlowDiagnostic>, grant: Grant| {
                if raw_is(v, "true") {
                    grant
                } else {
                    Grant::Cond(scan.unknown(out, site, &format!("{key}: {}", v.source(&src.raw))))
                }
            };
            let grant = match key.as_str() {
                "public" => flag(out, Grant::Open),
                "admin" => flag(out, Grant::Cond(Condition::Role("admin".into()))),
                "role" | "roles" => {
                    Grant::Cond(scan.roles(out, site, &src.raw, std::slice::from_ref(v)))
                }
                "permission" => Grant::Cond(match scan.consts.name(v) {
                    Some(p) => Condition::Policy(p),
                    None => scan.unknown(out, site, &v.source(&src.raw)),
                }),
                _ => {
                    let message = format!("Option `{key}` of @{} is not evaluated.", d.name);
                    note(out, site.file, site.line, message);
                    continue;
                }
            };
            Scan::add(level, grant, Some(meta_key));
        }
    }
}

fn sources(text: &str, items: &[Item]) -> String {
    items
        .iter()
        .map(|i| i.source(text))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Classes, their members and the application-wide settings.
mod structure {
    use super::*;

    pub(super) struct Class<'s> {
        pub(super) name: String,
        pub(super) decos: Vec<&'s Deco>,
        pub(super) body: (usize, usize),
    }

    pub(super) struct Method<'s> {
        pub(super) name: String,
        pub(super) decos: Vec<&'s Deco>,
        pub(super) body: Option<(usize, usize)>,
    }

    pub(super) fn classes(src: &Src) -> Vec<Class<'_>> {
        let (code, b) = (&src.code, src.code.as_bytes());
        let mut out = Vec::new();
        for pos in words(code, "class") {
            let at = skip_ws(b, pos + 5);
            let name = ident_at(code, at);
            let Some(open) = code[at..].find('{').map(|o| at + o) else {
                continue;
            };
            if name.is_empty() {
                continue;
            }
            out.push(Class {
                name: name.into(),
                decos: before(src, pos),
                body: (open, block_end(code, open, b'{', b'}')),
            });
        }
        out
    }

    /// The decorators directly before `pos` (`export`, `default`, `abstract` between).
    fn before(src: &Src, pos: usize) -> Vec<&Deco> {
        let mut out = Vec::new();
        let mut q = pos;
        'outer: loop {
            let t = src.code[..q].trim_end();
            for kw in ["export", "default", "abstract", "declare"] {
                if t.ends_with(kw)
                    && !t[..t.len() - kw.len()].ends_with(|c: char| is_ident_char(c) || c == '.')
                {
                    q = t.len() - kw.len();
                    continue 'outer;
                }
            }
            match src.decos.iter().find(|d| d.end == t.len()) {
                Some(d) => {
                    out.push(d);
                    q = d.at;
                }
                None => break,
            }
        }
        out.reverse();
        out
    }

    /// Decorated methods of a class body.
    pub(super) fn methods<'s>(src: &'s Src, class: &Class) -> Vec<Method<'s>> {
        let (code, b) = (&src.code, src.code.as_bytes());
        let (open, end) = class.body;
        let mut out = Vec::new();
        let mut pending: Vec<&Deco> = Vec::new();
        let (mut i, mut depth) = (open + 1, 0usize);
        while i + 1 < end {
            match b[i] {
                b'{' | b'(' | b'[' => depth += 1,
                b'}' | b')' | b']' => depth = depth.saturating_sub(1),
                b'@' if depth == 0 => {
                    if let Some(d) = src.decos.iter().find(|d| d.at == i) {
                        pending.push(d);
                        i = d.end;
                        continue;
                    }
                }
                c if depth == 0 && !pending.is_empty() && is_ident_char(c as char) => {
                    let (name, k) = member_name(code, i);
                    if b.get(k) == Some(&b'(') {
                        let params = close(code, k);
                        let body = body_after(code, params);
                        let decos = std::mem::take(&mut pending);
                        out.push(Method {
                            name: name.into(),
                            decos,
                            body,
                        });
                        i = body.map_or(params, |(_, e)| e);
                    } else {
                        pending.clear();
                        i = k.max(i + 1);
                    }
                    continue;
                }
                _ => {}
            }
            i += 1;
        }
        out
    }

    /// A member's name after its modifiers at `i`, and the offset after it
    /// (past type parameters).
    fn member_name(code: &str, mut i: usize) -> (&str, usize) {
        let b = code.as_bytes();
        let name = loop {
            let w = ident_at(code, i);
            let next = skip_ws(b, i + w.len());
            if !MODIFIERS.contains(&w) || !b.get(next).is_some_and(|c| is_ident_char(*c as char)) {
                break w;
            }
            i = next;
        };
        let k = skip_ws(b, i + name.len());
        match b.get(k) {
            Some(b'<') => (name, skip_ws(b, skip_generic(b, k))),
            _ => (name, k),
        }
    }

    /// The `{…}` body after a parameter list, past a return type; `None` at `;`.
    fn body_after(code: &str, from: usize) -> Option<(usize, usize)> {
        let b = code.as_bytes();
        let (mut i, mut depth) = (from, 0usize);
        while i < b.len() {
            match b[i] {
                b'=' if b.get(i + 1) == Some(&b'>') => i += 1,
                b';' if depth == 0 => return None,
                b'{' if depth == 0 => {
                    let before = code[..i].trim_end();
                    if before.ends_with(|c: char| is_ident_char(c) || matches!(c, '>' | ']' | ')'))
                        && !before.ends_with("=>")
                    {
                        return Some((i, block_end(code, i, b'{', b'}')));
                    }
                    depth += 1;
                }
                b'<' | b'(' | b'[' | b'{' => depth += 1,
                b'>' | b')' | b']' | b'}' => depth = depth.checked_sub(1)?,
                _ => {}
            }
            i += 1;
        }
        None
    }

    /// `new X(…)` passed to `commandBus.execute` / `queryBus.execute` in `body`.
    pub(super) fn sends(src: &Src, body: Option<(usize, usize)>) -> Vec<String> {
        let Some((a, e)) = body else { return vec![] };
        let code = &src.code;
        let mut found = Vec::new();
        for bus in ["commandBus", "queryBus"] {
            for p in words(&code[a..e], bus) {
                found.extend(bus_send(code, a + p + bus.len()));
            }
        }
        found.sort();
        let mut names: Vec<String> = found.into_iter().map(|(_, n)| n).collect();
        names.dedup();
        names
    }

    /// After a bus name at `i`: `.execute<…>(new X(` gives the offset and `X`.
    fn bus_send(code: &str, i: usize) -> Option<(usize, String)> {
        let b = code.as_bytes();
        let step =
            |i: usize, word: &str| (ident_at(code, i) == word).then(|| skip_ws(b, i + word.len()));
        let i = skip_ws(b, i);
        (b.get(i) == Some(&b'.')).then_some(())?;
        let mut j = step(skip_ws(b, i + 1), "execute")?;
        if b.get(j) == Some(&b'<') {
            j = skip_ws(b, skip_generic(b, j));
        }
        (b.get(j) == Some(&b'(')).then_some(())?;
        let k = step(skip_ws(b, j + 1), "new")?;
        let name = ident_at(code, k);
        (!name.is_empty()).then(|| (k, name.to_string()))
    }

    /// Calls `name(…)` in the application: (source, offset of the name, arguments).
    pub(super) fn calls<'s>(app: &'s App, name: &str) -> Vec<(&'s Src, usize, Vec<Item>)> {
        let mut out = Vec::new();
        for src in &app.files {
            let b = src.code.as_bytes();
            for p in words(&src.code, name) {
                let open = skip_ws(b, p + name.len());
                if b.get(open) == Some(&b'(') {
                    out.push((
                        src,
                        p,
                        Parser {
                            s: &src.raw,
                            i: open,
                        }
                        .array(0),
                    ));
                }
            }
        }
        out
    }
}
use structure::{Class, Method, calls, classes, methods, sends};

/// API versions of a route; `None` is version neutral (no segment).
type Versions = Vec<Option<String>>;

/// `/<prefix><version>` per endpoint.
struct Versioning {
    prefix: String,
    default: Option<Versions>,
}

/// What every endpoint of an application shares.
struct Settings {
    prefix: Option<String>,
    excludes: Vec<String>,
    versioning: Option<Versioning>,
    globals: Vec<Global>,
}

impl Settings {
    /// The global guards that apply to controller `class`.
    fn global(&self, class: &str) -> Level {
        let mut level = Level::default();
        let applies = |g: &&Global| g.scope.as_ref().is_none_or(|s| s.contains(class));
        for g in self.globals.iter().filter(applies) {
            meta::add_guard(&mut level, (g.grant.clone(), g.name.clone()));
        }
        level
    }

    /// Every path of controller paths × versions × method paths.
    fn paths(&self, cpaths: &[String], versions: &Versions, mpaths: &[String]) -> Vec<String> {
        let mut out = Vec::new();
        for (cp, mp) in cpaths
            .iter()
            .flat_map(|c| mpaths.iter().map(move |m| (c, m)))
        {
            let route = join(&[cp, mp]);
            let excluded =
                (self.excludes.iter()).any(|e| e.trim_matches('/') == route.trim_matches('/'));
            let prefix = self.prefix.as_deref().filter(|_| !excluded).unwrap_or("");
            for v in versions {
                out.push(format!(
                    "/{}",
                    join(&[prefix, v.as_deref().unwrap_or(""), &route])
                ));
            }
        }
        out
    }
}

/// One controller class as its methods need it.
struct Ctl<'x> {
    src: &'x Src,
    name: String,
    paths: Vec<String>,
    version: Option<Versions>,
    global: Level,
    level: Level,
}

impl Scan<'_> {
    fn endpoints(&self, flow: &mut UiFlow) {
        let out = &mut flow.diagnostics;
        let (prefix, excludes) = self.prefix(out);
        let settings = Settings {
            prefix,
            excludes,
            versioning: self.versioning(out),
            globals: self.globals(out),
        };
        let mut found = Vec::new();
        for src in &self.app.files {
            for class in classes(src) {
                let Some(ctl) = self.controller_of(out, &settings, src, &class) else {
                    continue;
                };
                for m in methods(src, &class) {
                    found.extend(self.method_endpoints(out, &settings, &ctl, &m));
                }
            }
        }
        flow.endpoints.extend(found);
    }

    /// The class as a controller, when `@Controller(…)` marks it and is readable.
    fn controller_of<'x>(
        &self,
        out: &mut Vec<UiFlowDiagnostic>,
        settings: &Settings,
        src: &'x Src,
        class: &Class,
    ) -> Option<Ctl<'x>> {
        let ctrl = class.decos.iter().find(|d| d.name == "Controller")?;
        let site = Site {
            file: &src.rel,
            line: src.line(ctrl.at),
        };
        let (paths, version) = self.controller(out, &site, src, ctrl)?;
        Some(Ctl {
            src,
            name: class.name.clone(),
            paths,
            version,
            global: settings.global(&class.name),
            level: self.level(out, src, &class.decos),
        })
    }

    /// The endpoints of one method's route decorators.
    fn method_endpoints(
        &self,
        out: &mut Vec<UiFlowDiagnostic>,
        settings: &Settings,
        ctl: &Ctl,
        m: &Method,
    ) -> Vec<EndpointAccess> {
        let src = ctl.src;
        let routes: Vec<(&Deco, &str)> = (m.decos.iter())
            .filter_map(|d| {
                ROUTES
                    .iter()
                    .find(|(r, _)| d.name == *r)
                    .map(|(_, v)| (*d, *v))
            })
            .collect();
        if routes.is_empty() {
            return vec![];
        }
        let (access, notes) = meta::access(
            self,
            &ctl.global,
            &ctl.level,
            &self.level(out, src, &m.decos),
        );
        if let Some(d) = routes.first() {
            for n in notes {
                note(out, &src.rel, src.line(d.0.at), n);
            }
        }
        let Some(versions) = self.method_versions(out, settings, ctl, m) else {
            return vec![];
        };
        let (handler, sends) = (format!("{}.{}", ctl.name, m.name), sends(src, m.body));
        let mut found = Vec::new();
        for (d, method) in routes {
            let line = src.line(d.at);
            let mpaths = match d.args.first() {
                None => Some(vec![String::new()]),
                Some(a) => self.consts.strings(&src.raw, a, Consts::text),
            };
            let Some(mpaths) = mpaths else {
                let message = format!(
                    "Route path not readable: {}; the endpoint is skipped.",
                    sources(&src.raw, &d.args)
                );
                gap(out, &src.rel, line, message);
                continue;
            };
            found.extend(
                settings
                    .paths(&ctl.paths, &versions, &mpaths)
                    .into_iter()
                    .map(|path| EndpointAccess {
                        method: method.into(),
                        path,
                        access: access.clone(),
                        application: Some(self.app.name.clone()),
                        file: src.rel.clone(),
                        line,
                        source: "nestjs".into(),
                        handler: Some(handler.clone()),
                        sends: sends.clone(),
                    }),
            );
        }
        found
    }

    /// Version segments of a method: its `@Version`, else the controller's, else the default.
    fn method_versions(
        &self,
        out: &mut Vec<UiFlowDiagnostic>,
        settings: &Settings,
        ctl: &Ctl,
        m: &Method,
    ) -> Option<Versions> {
        let Some(v) = &settings.versioning else {
            return Some(vec![None]);
        };
        let read = match m.decos.iter().find(|d| d.name == "Version") {
            Some(d) => {
                let site = Site {
                    file: &ctl.src.rel,
                    line: ctl.src.line(d.at),
                };
                self.versions(out, &site, ctl.src, d.args.first())?
            }
            None => (ctl.version.clone())
                .or_else(|| v.default.clone())
                .unwrap_or(vec![None]),
        };
        Some(
            read.into_iter()
                .map(|x| x.map(|x| format!("{}{x}", v.prefix)))
                .collect(),
        )
    }

    /// Paths and version of a `@Controller(…)`; `None` (with a gap) when not readable.
    fn controller(
        &self,
        out: &mut Vec<UiFlowDiagnostic>,
        site: &Site,
        src: &Src,
        d: &Deco,
    ) -> Option<(Vec<String>, Option<Versions>)> {
        let Some(arg) = d.args.first() else {
            return Some((vec![String::new()], None));
        };
        let (path, version) = match &arg.value {
            Value::Obj(_) => (field(arg, "path"), field(arg, "version")),
            _ => (Some(arg), None),
        };
        let paths = match path {
            None => Some(vec![String::new()]),
            Some(p) => self.consts.strings(&src.raw, p, Consts::text),
        };
        let Some(paths) = paths else {
            let message = format!(
                "Controller path not readable: {}; its endpoints are unknown.",
                arg.source(&src.raw)
            );
            gap(out, site.file, site.line, message);
            return None;
        };
        let version = match version {
            None => None,
            Some(v) => Some(self.versions(out, site, src, Some(v))?),
        };
        Some((paths, version))
    }

    /// Versions of a `version` value: strings, `VERSION_NEUTRAL` (none), arrays.
    fn versions(
        &self,
        out: &mut Vec<UiFlowDiagnostic>,
        site: &Site,
        src: &Src,
        item: Option<&Item>,
    ) -> Option<Versions> {
        fn read(consts: &Consts, item: &Item) -> Option<Versions> {
            match &item.value {
                Value::Raw(r) if r == "VERSION_NEUTRAL" || r.ends_with(".VERSION_NEUTRAL") => {
                    Some(vec![None])
                }
                Value::Arr(items) => items
                    .iter()
                    .map(|i| read(consts, i))
                    .collect::<Option<Vec<_>>>()
                    .map(|v| v.concat()),
                _ => consts.text(item).map(|s| vec![Some(s)]),
            }
        }
        let read = item.and_then(|i| read(&self.consts, i));
        if read.is_none() {
            let text = item.map_or(String::new(), |i| i.source(&src.raw));
            let message = format!("API version not readable: {text}; the endpoint is skipped.");
            gap(out, site.file, site.line, message);
        }
        read
    }

    /// The global prefix and the routes it excludes.
    fn prefix(&self, out: &mut Vec<UiFlowDiagnostic>) -> (Option<String>, Vec<String>) {
        let mut found: Vec<(String, Vec<String>)> = Vec::new();
        for (src, at, args) in calls(self.app, "setGlobalPrefix") {
            let line = src.line(at);
            let Some(prefix) = args.first().and_then(|a| self.consts.text(a)) else {
                let message = format!(
                    "Global prefix not readable: {}; routes are shown without it.",
                    sources(&src.raw, &args)
                );
                gap(out, &src.rel, line, message);
                return (None, vec![]);
            };
            let mut excludes = Vec::new();
            if let Some(list) = args.get(1).and_then(|o| field(o, "exclude")) {
                let items: Vec<(String, Item)> = match &list.value {
                    Value::Raw(r) if self.consts.list(r).is_some() => {
                        (self.consts.list(r).unwrap_or_default().iter())
                            .map(|s| (s.clone(), reparse(s)))
                            .collect()
                    }
                    Value::Arr(items) => items
                        .iter()
                        .map(|i| (i.source(&src.raw), reparse(&i.source(&src.raw))))
                        .collect(),
                    _ => vec![(list.source(&src.raw), reparse(&list.source(&src.raw)))],
                };
                for (text, item) in items {
                    match self.consts.text(field(&item, "path").unwrap_or(&item)) {
                        Some(path) => excludes.push(path),
                        None => gap(
                            out,
                            &src.rel,
                            line,
                            format!("Global prefix exclusion not readable: {text}"),
                        ),
                    }
                }
            }
            found.push((prefix, excludes));
        }
        found.dedup();
        match found.len() {
            0 => (None, vec![]),
            1 => {
                let (prefix, excludes) = found.pop().expect("one");
                (Some(prefix), excludes)
            }
            _ => {
                let all: Vec<&str> = found.iter().map(|(p, _)| p.as_str()).collect();
                let message = format!(
                    "Several global prefixes ({}); routes are shown without one.",
                    all.join(", ")
                );
                gap(out, ".", 0, message);
                (None, vec![])
            }
        }
    }

    fn versioning(&self, out: &mut Vec<UiFlowDiagnostic>) -> Option<Versioning> {
        let mut all = calls(self.app, "enableVersioning");
        if all.len() > 1 {
            let (src, at, _) = &all[1];
            gap(
                out,
                &src.rel,
                src.line(*at),
                "Versioning enabled more than once; versions are not added.".to_string(),
            );
            return None;
        }
        let (src, at, args) = all.pop()?;
        let options = args.first();
        let get = |key| options.and_then(|o| field(o, key));
        if get("type").is_some_and(|t| !matches!(&t.value, Value::Raw(r) if r.ends_with("URI"))) {
            return None;
        }
        let prefix = match get("prefix") {
            None => "v".to_string(),
            Some(p) if raw_is(p, "false") => String::new(),
            Some(p) => match self.consts.text(p) {
                Some(p) => p,
                None => {
                    let message = format!(
                        "Version prefix not readable: {}; versions are not added.",
                        p.source(&src.raw)
                    );
                    gap(out, &src.rel, src.line(at), message);
                    return None;
                }
            },
        };
        let site = Site {
            file: &src.rel,
            line: src.line(at),
        };
        let default = match get("defaultVersion") {
            None => None,
            Some(d) => Some(self.versions(out, &site, src, Some(d))?),
        };
        Some(Versioning { prefix, default })
    }

    /// Global guards with the controllers they apply to.
    fn globals(&self, out: &mut Vec<UiFlowDiagnostic>) -> Vec<Global> {
        let roots = global::roots(self.app);
        let distinct: BTreeSet<&String> = roots.values().collect();
        let found = global::declared(self, out, &roots);
        let mut globals = Vec::new();
        for (grant, module, src, at) in found {
            let scope = match module {
                _ if distinct.len() < 2 => None,
                Some(m) if distinct.contains(&m) => self.module_controllers(&m).or_else(|| {
                    let message = format!(
                        "Controllers of root module {m} not readable; its global guard applies to every controller."
                    );
                    gap(out, &src.rel, src.line(at), message);
                    None
                }),
                m => {
                    let roots: Vec<&str> = distinct.iter().map(|s| s.as_str()).collect();
                    let message = format!(
                        "Global guard declared outside the root modules ({}) in {}; it applies to every controller.",
                        roots.join(", "),
                        m.as_deref().unwrap_or("no module")
                    );
                    gap(out, &src.rel, src.line(at), message);
                    None
                }
            };
            let (grant, name) = grant;
            globals.push(Global { grant, name, scope });
        }
        globals
    }

    /// The controllers a `@Module({ controllers })` class lists.
    fn module_controllers(&self, module: &str) -> Option<BTreeSet<String>> {
        let (src, d) = (self.app.files.iter())
            .flat_map(|s| s.decos.iter().map(move |d| (s, d)))
            .find(|(s, d)| d.name == "Module" && module_class(s, d).as_deref() == Some(module))?;
        let Value::Arr(items) = &field(d.args.first()?, "controllers")?.value else {
            return None;
        };
        let mut names = BTreeSet::new();
        for item in items {
            let text = item.source(&src.raw);
            match text.strip_prefix("...") {
                Some(list) => names.extend(self.consts.list(list.trim())?.iter().cloned()),
                None => {
                    names.insert(text);
                }
            }
        }
        names.iter().all(|n| name_path(n)).then_some(names)
    }
}

/// Root modules and the global guards an application declares.
mod global {
    use super::*;

    /// A global guard: what it grants, its module, where it is declared.
    pub(super) type Declared<'s> = ((Grant, String), Option<String>, &'s Src, usize);

    /// The module each file passes to `NestFactory.create(M)`, by file.
    pub(super) fn roots(app: &App) -> BTreeMap<String, String> {
        let mut roots = BTreeMap::new();
        for src in &app.files {
            let (code, b) = (&src.code, src.code.as_bytes());
            for p in words(code, "NestFactory") {
                let i = skip_ws(b, p + "NestFactory".len());
                if b.get(i) != Some(&b'.') || ident_at(code, skip_ws(b, i + 1)) != "create" {
                    continue;
                }
                let mut i = skip_ws(b, skip_ws(b, i + 1) + "create".len());
                if b.get(i) == Some(&b'<') {
                    i = skip_ws(b, skip_generic(b, i));
                }
                let root = ident_at(code, skip_ws(b, i + 1));
                if b.get(i) == Some(&b'(') && !root.is_empty() {
                    roots
                        .entry(src.rel.clone())
                        .or_insert_with(|| root.to_string());
                }
            }
        }
        roots
    }

    /// `{ provide: APP_GUARD, useClass: X }` and `useGlobalGuards(new X())`.
    pub(super) fn declared<'s>(
        scan: &Scan<'s>,
        out: &mut Vec<UiFlowDiagnostic>,
        roots: &BTreeMap<String, String>,
    ) -> Vec<Declared<'s>> {
        let mut found = Vec::new();
        for src in &scan.app.files {
            let code = &src.code;
            for pos in words(code, "APP_GUARD") {
                let before = code[..pos].trim_end();
                let Some(brace) = (before.strip_suffix(':').map(str::trim_end))
                    .filter(|t| t.ends_with("provide"))
                    .and_then(|t| t.rfind('{'))
                else {
                    continue;
                };
                let site = Site {
                    file: &src.rel,
                    line: src.line(pos),
                };
                let object = Parser {
                    s: &src.raw,
                    i: brace,
                }
                .value(0);
                let grant =
                    match field(&object, "useClass").or_else(|| field(&object, "useExisting")) {
                        Some(class) => scan.guard(out, &site, src, &src.raw, class),
                        None => {
                            let text = object.source(&src.raw);
                            (Grant::Cond(scan.unknown(out, &site, &text)), text)
                        }
                    };
                found.push((grant, module_of(src, pos), src, pos));
            }
        }
        for (s, at, args) in calls(scan.app, "useGlobalGuards") {
            let site = Site {
                file: &s.rel,
                line: s.line(at),
            };
            for a in &args {
                let grant = scan.guard(out, &site, s, &s.raw, a);
                found.push((grant, roots.get(&s.rel).cloned(), s, at));
            }
        }
        found
    }
}

/// The class a `@Module` decorator stands on.
fn module_class(src: &Src, d: &Deco) -> Option<String> {
    let at = d.end + words(&src.code[d.end..], "class").first()?;
    Some(ident_at(&src.code, skip_ws(src.code.as_bytes(), at + 5)).to_string())
        .filter(|n| !n.is_empty())
}

/// The module whose `@Module({…})` holds `pos`, directly or through a spread
/// `const` array of the same file.
fn module_of(src: &Src, pos: usize) -> Option<String> {
    let modules: Vec<&Deco> = src.decos.iter().filter(|d| d.name == "Module").collect();
    if let Some(d) = modules.iter().find(|d| d.at < pos && pos < d.end) {
        return module_class(src, d);
    }
    let (code, b) = (&src.code, src.code.as_bytes());
    for c in words(code, "const") {
        let at = skip_ws(b, c + 5);
        let name = ident_at(code, at);
        let Some(eq) = assignment(code, at + name.len()) else {
            continue;
        };
        let open = skip_ws(b, eq + 1);
        if b.get(open) != Some(&b'[') || !(open < pos && pos < close(code, open)) {
            continue;
        }
        let spread = |d: &&&Deco| {
            words(&code[d.at..d.end], name)
                .iter()
                .any(|&p| code[d.at..d.at + p].trim_end().ends_with("..."))
        };
        return modules
            .iter()
            .find(spread)
            .and_then(|d| module_class(src, d));
    }
    None
}

/// Metadata keys, the guards that read them, and what that does to access.
mod meta {
    use super::*;

    /// Lookup functions and whether they merge.
    const LOOKUPS: [(&str, bool); 3] = [
        ("getAllAndOverride", false),
        ("getAllAndMerge", true),
        ("get", false),
    ];

    /// The metadata key of a lookup or `SetMetadata` argument: its value, else as written.
    pub(super) fn key(scan: &Scan, text: &str, item: &Item) -> String {
        (scan.consts.text(item)).unwrap_or_else(|| item.source(text))
    }

    /// The key of the first argument in `args`' source `text`.
    pub(super) fn first_key(scan: &Scan, text: &str, args: &[Item]) -> String {
        args.first().map(|k| key(scan, text, k)).unwrap_or_default()
    }

    /// The key a decorator sets with `SetMetadata(KEY, …)` in its definition.
    fn decorator_key(scan: &Scan, name: &str) -> Option<String> {
        let [def] = scan.composites.get(name)?.as_slice() else {
            return None;
        };
        let (_, text) = def.calls.iter().find(|(n, _)| n == "SetMetadata")?;
        let args = Parser { s: text, i: 0 }.array(0);
        Some(key(scan, text, args.first()?))
    }

    /// The metadata key `@Roles` sets: its definition's, else `roles`.
    pub(super) fn roles_key(scan: &Scan) -> String {
        decorator_key(scan, "Roles").unwrap_or_else(|| "roles".into())
    }

    /// A `[views.access]` `open` decorator's metadata key; unreadable is a gap.
    fn public_key(scan: &Scan, out: &mut Vec<UiFlowDiagnostic>, site: &Site, name: &str) -> String {
        decorator_key(scan, name).unwrap_or_else(|| {
            let message =
                format!("Metadata key of @{name} not readable; it is not treated as public.");
            gap(out, site.file, site.line, message);
            String::new()
        })
    }

    /// Adds a guard by its name.
    pub(super) fn add_guard(level: &mut Level, (grant, name): (Grant, String)) {
        match grant {
            Grant::Open => level.opened = true,
            Grant::Cond(c) => level.guards.push((Some(name), c)),
        }
    }

    /// Adds a `[views.access]` decorator: `open` is a public marker for its key.
    pub(super) fn add_listed(
        scan: &Scan,
        out: &mut Vec<UiFlowDiagnostic>,
        site: &Site,
        level: &mut Level,
        grant: Grant,
        name: &str,
    ) {
        match grant {
            Grant::Open => Scan::add(level, grant, Some(&public_key(scan, out, site, name))),
            g => Scan::add(level, g, None),
        }
    }

    /// Calls of `reflector.<lookup>(KEY, …)` in `src`: (offset, key, lookup).
    fn lookups(scan: &Scan, src: &Src) -> Vec<(usize, String, Lookup)> {
        let (code, b) = (&src.code, src.code.as_bytes());
        let mut found = Vec::new();
        for (name, merge) in LOOKUPS {
            for at in words(code, name) {
                let receiver = code[..at].strip_suffix('.').unwrap_or("");
                let mut open = skip_ws(b, at + name.len());
                if b.get(open) == Some(&b'<') {
                    open = skip_ws(b, skip_generic(b, open));
                }
                if !receiver.to_lowercase().ends_with("reflector") || b.get(open) != Some(&b'(') {
                    continue;
                }
                let args = Parser {
                    s: &src.raw,
                    i: open,
                }
                .array(0);
                let targets: String = args.iter().skip(1).map(|a| a.source(&src.raw)).collect();
                let (h, c) = (targets.contains("getHandler"), targets.contains("getClass"));
                let both = h == c;
                let lookup = Lookup {
                    merge,
                    handler: h || both,
                    class: c || both,
                };
                found.extend(args.first().map(|k| (at, key(scan, &src.raw, k), lookup)));
            }
        }
        found
    }

    /// The name of the top-level `const`/`function` (at column 0) that holds `at`.
    fn top_level(code: &str, at: usize) -> Option<String> {
        code[..at].lines().rev().find_map(|l| {
            let l = l.strip_prefix("export ").unwrap_or(l);
            let l = (l.strip_prefix("const ")).or_else(|| l.strip_prefix("function "))?;
            Some(ident_at(l, 0).to_string()).filter(|n| !n.is_empty())
        })
    }

    /// Guards that read a metadata key: a class calling `reflector.getAllAndOverride(K, …)`,
    /// `getAllAndMerge` or `get`, directly or through a top-level helper function
    /// (declared at column 0) that does and that the class calls.
    pub(super) fn readers(scan: &Scan) -> HashMap<String, Vec<(String, Lookup)>> {
        let mut readers: HashMap<String, Vec<(String, Lookup)>> = HashMap::new();
        let mut helpers: Vec<(String, String, Lookup)> = Vec::new();
        for src in &scan.app.files {
            let classes = structure::classes(src);
            for (at, key, lookup) in lookups(scan, src) {
                let owner = classes.iter().find(|c| c.body.0 <= at && at < c.body.1);
                match owner {
                    Some(c) => readers
                        .entry(key)
                        .or_default()
                        .push((c.name.clone(), lookup)),
                    None => helpers.extend(top_level(&src.code, at).map(|f| (f, key, lookup))),
                }
            }
        }
        for src in &scan.app.files {
            let b = src.code.as_bytes();
            for class in structure::classes(src) {
                let body = &src.code[class.body.0..class.body.1];
                for (f, key, lookup) in &helpers {
                    let calls =
                        |p: &usize| b.get(skip_ws(b, class.body.0 + p + f.len())) == Some(&b'(');
                    if words(body, f).iter().any(calls) {
                        let list = readers.entry(key.clone()).or_default();
                        list.push((class.name.clone(), *lookup));
                    }
                }
            }
        }
        readers
    }

    type Guards<'g> = Vec<&'g (Option<String>, Condition)>;

    /// Whether guard `name` reads marker `key` at the marker's level: a method
    /// marker needs a lookup of the handler, a class marker one of the class.
    fn reads_at(scan: &Scan, name: &str, (key, at_method): (&String, bool)) -> bool {
        (scan.readers.get(key.as_str())).is_some_and(|r| {
            (r.iter()).any(|(g, l)| g == name && if at_method { l.handler } else { l.class })
        })
    }

    /// The guards that stay after the public markers `publics` removed those
    /// that read their key, and whether one was removed.
    fn surviving(
        scan: &Scan,
        guards: &Guards,
        publics: &[(&String, bool)],
    ) -> (Vec<Condition>, bool) {
        let honours = |name: &str| publics.iter().any(|p| reads_at(scan, name, *p));
        let kept: Vec<Condition> = (guards.iter())
            .filter(|(name, _)| !name.as_deref().is_some_and(honours))
            .map(|(_, c)| c.clone())
            .collect();
        let removed = kept.len() < guards.len();
        (kept, removed)
    }

    /// Any of the roles of `cs`, flattened.
    fn merged(cs: Vec<Condition>) -> impl Iterator<Item = Condition> {
        cs.into_iter().flat_map(|c| match c {
            Condition::Any(v) => v,
            c => vec![c],
        })
    }

    /// The conditions metadata `key` adds under the lookup of the first guard in
    /// force that reads it; none when no guard reads it.
    fn lookup_conditions(
        scan: &Scan,
        guards: &Guards,
        (class, method): (&Level, &Level),
        key: &str,
    ) -> Option<Vec<Condition>> {
        let readers = scan.readers.get(key).map_or(&[][..], Vec::as_slice);
        let lookup = (guards.iter())
            .filter_map(|(n, _)| n.as_deref())
            .find_map(|n| readers.iter().find(|(g, _)| g == n))
            .map(|(_, l)| *l)?;
        let own = |level: &Level| -> Vec<Condition> {
            (level.meta.iter())
                .filter(|(k, _)| k == key)
                .map(|(_, c)| c.clone())
                .collect()
        };
        let c = if lookup.class { own(class) } else { vec![] };
        let m = if lookup.handler { own(method) } else { vec![] };
        Some(match lookup.merge {
            false if m.is_empty() => c,
            false => m,
            true => combine(merged(c).chain(merged(m)).collect(), Condition::Any)
                .into_iter()
                .collect(),
        })
    }

    /// The access of an endpoint from its global, class and method declarations,
    /// with the notes for metadata nobody reads. Metadata counts only with a
    /// guard in force that reads its key; a public marker removes exactly the
    /// guards that read its key (`Open` when none remains).
    pub(super) fn access(
        scan: &Scan,
        global: &Level,
        class: &Level,
        method: &Level,
    ) -> (AccessState, Vec<String>) {
        let guards: Guards = [global, class, method]
            .into_iter()
            .flat_map(|l| &l.guards)
            .collect();
        let publics: Vec<(&String, bool)> = (class.public.iter().map(|k| (k, false)))
            .chain(method.public.iter().map(|k| (k, true)))
            .collect();
        let (mut conds, honoured) = surviving(scan, &guards, &publics);
        let mut notes = Vec::new();
        if !honoured {
            for (k, _) in publics.iter().filter(|(k, _)| !k.is_empty()) {
                notes.push(format!("Public marker `{k}` set, no guard reads it."));
            }
        }
        let mut keys: Vec<&String> = (class.meta.iter().chain(&method.meta))
            .map(|(k, _)| k)
            .collect();
        let mut seen = BTreeSet::new();
        keys.retain(|k| seen.insert(*k));
        for key in keys {
            match lookup_conditions(scan, &guards, (class, method), key) {
                Some(c) => conds.extend(c),
                None => notes.push(format!("Metadata `{key}` set, no guard reads it.")),
            }
        }
        let opened = global.opened || class.opened || method.opened;
        let state = match combine(conds, Condition::All) {
            Some(c) => AccessState::Protected(c),
            None if honoured || opened => AccessState::Open,
            None => AccessState::Undeclared,
        };
        (state, notes)
    }
}

/// Path segments joined with single `/`, empty ones left out.
fn join(parts: &[&str]) -> String {
    let parts: Vec<&str> = (parts.iter())
        .map(|p| p.trim_matches('/'))
        .filter(|p| !p.is_empty())
        .collect();
    parts.join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{AccessState, Condition, DiagnosticSeverity, EndpointAccess};

    const PKG: &str = r#"{"dependencies":{"@nestjs/common":"11","@nestjs/core":"11"}}"#;

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
    fn role(n: &str) -> Condition {
        Condition::Role(n.into())
    }
    fn custom(n: &str) -> Condition {
        Condition::Custom(n.into())
    }
    fn gaps(flow: &UiFlow) -> Vec<&str> {
        (flow.diagnostics.iter())
            .filter(|d| d.severity == DiagnosticSeverity::Gap)
            .map(|d| d.message.as_str())
            .collect()
    }

    #[test]
    fn routes_with_global_prefix_exclude_enum_constants_and_handlers() {
        let flow = run(
            &[
                ("api/package.json", PKG),
                (
                    "api/src/main.ts",
                    "const app = await NestFactory.create(AppModule);\n\
                     app.setGlobalPrefix('api', { exclude: excluded });\n",
                ),
                (
                    "api/src/constants.ts",
                    "export const excluded = ['/health', { path: 'ping', method: RequestMethod.GET }];\n\
                     export enum RouteKey { Asset = 'assets', User = \"users\" }\n",
                ),
                (
                    "api/src/orders.controller.ts",
                    r#"import { Controller, Get, Post, All } from '@nestjs/common';
// @Controller('commented-out')
@Controller('orders')
export class OrdersController {
  constructor(private readonly service: OrdersService) {}

  @Get()
  list() { return this.service.list(); }

  @Get(':id')
  async one(@Param('id') id: string): Promise<Order> { return this.service.one(id); }

  @Post(['', 'bulk'])
  create(@Body() dto: CreateDto) {}

  @All('any')
  any() {}
}

@Controller(RouteKey.Asset)
export class AssetController {
  @Get('/:id/thumb/')
  thumb() {}
}

@Controller()
export class AppController {
  @Get('health')
  health() {}
  @Get('ping')
  ping() {}
}
"#,
                ),
                (
                    "api/src/orders.controller.spec.ts",
                    "@Controller('spec') class S { @Get() x() {} }",
                ),
                ("web/package.json", r#"{"dependencies":{"react":"19"}}"#),
                (
                    "web/src/x.ts",
                    "@Controller('web') class W { @Get() x() {} }",
                ),
            ],
            "",
        );
        let paths: Vec<(&str, &str)> = (flow.endpoints.iter())
            .map(|e| (e.method.as_str(), e.path.as_str()))
            .collect();
        assert_eq!(
            paths,
            [
                ("ANY", "/api/orders/any"),
                ("GET", "/api/assets/:id/thumb"),
                ("GET", "/api/orders"),
                ("GET", "/api/orders/:id"),
                ("GET", "/health"),
                ("GET", "/ping"),
                ("POST", "/api/orders"),
                ("POST", "/api/orders/bulk"),
            ]
        );
        let one = ep(&flow, "GET", "/api/orders/:id");
        assert_eq!(one.handler.as_deref(), Some("OrdersController.one"));
        assert_eq!(one.application.as_deref(), Some("api"));
        assert_eq!(one.source, "nestjs");
        assert_eq!(one.access, AccessState::Undeclared);
        assert_eq!(
            (one.file.as_str(), one.line),
            ("api/src/orders.controller.ts", 10)
        );
        assert!(one.sends.is_empty(), "service calls are no sends");
        assert!(flow.complete, "{:#?}", flow.diagnostics);
    }

    #[test]
    fn guards_roles_and_passport_through_views_access() {
        let flow = run(
            &[
                ("package.json", PKG),
                (
                    "src/roles.guard.ts",
                    &roles_guard("getAllAndOverride('roles', [ctx.getHandler(), ctx.getClass()])"),
                ),
                (
                    "src/admin.controller.ts",
                    r#"import { AuthGuard } from '@nestjs/passport';
export enum Role { Owner = 'owner' }
@Controller('admin')
@UseGuards(AuthGuard('jwt'), RolesGuard)
@Roles('staff')
export class AdminController {
  @Get('a')
  @UseGuards(AdminGuard, new AuditGuard(x))
  a() {}

  @Get('b')
  @Roles('admin', Role.Owner)
  b() {}

  @Get('c')
  @SetMetadata('roles', ['ops'])
  c() {}

  @Get('d')
  d() {}
}
"#,
                ),
            ],
            "AdminGuard = \"role:Admin\"\n",
        );
        let auth = Condition::Authenticated;
        let rg = custom("RolesGuard");
        assert_eq!(
            ep(&flow, "GET", "/admin/a").access,
            prot(Condition::All(vec![
                auth.clone(),
                rg.clone(),
                role("Admin"),
                custom("AuditGuard"),
                role("staff")
            ]))
        );
        assert_eq!(
            ep(&flow, "GET", "/admin/b").access,
            prot(Condition::All(vec![
                auth.clone(),
                rg.clone(),
                Condition::Any(vec![role("admin"), role("owner")])
            ]))
        );
        assert_eq!(
            ep(&flow, "GET", "/admin/c").access,
            prot(Condition::All(vec![auth.clone(), rg.clone(), role("ops")]))
        );
        assert_eq!(
            ep(&flow, "GET", "/admin/d").access,
            prot(Condition::All(vec![auth, rg, role("staff")]))
        );
        assert!(flow.complete, "{:#?}", flow.diagnostics);
    }

    #[test]
    fn global_guards_apply_unless_they_read_the_public_marker() {
        let flow = run(
            &[
                ("package.json", PKG),
                (
                    "src/main.ts",
                    "const app = await NestFactory.create<NestExpressApplication>(AppModule);\n\
                     app.useGlobalGuards(new ThrottleGuard(reflector));\n",
                ),
                (
                    "src/app.module.ts",
                    "@Module({ controllers: [CatsController], providers: [{ provide: APP_GUARD, useClass: JwtGuard }] })\n\
                     export class AppModule {}\n",
                ),
                (
                    "src/cats.controller.ts",
                    "@Controller('cats')\nexport class CatsController {\n\
                     @Get() list() {}\n\
                     @Public()\n@Get('open') open() {}\n}\n",
                ),
                (
                    "src/jwt.guard.ts",
                    "export const Public = () => SetMetadata('isPublic', true);\n\
                     export class JwtGuard { canActivate(c) { return this.reflector.getAllAndOverride('isPublic', [c.getHandler()]); } }\n",
                ),
            ],
            "JwtGuard = \"authenticated\"\nPublic = \"open\"\n",
        );
        assert_eq!(
            ep(&flow, "GET", "/cats").access,
            prot(Condition::All(vec![
                Condition::Authenticated,
                custom("ThrottleGuard")
            ]))
        );
        // ThrottleGuard does not read the key, so it still applies.
        assert_eq!(
            ep(&flow, "GET", "/cats/open").access,
            prot(custom("ThrottleGuard"))
        );
    }

    #[test]
    fn global_guards_of_several_root_modules_stay_with_their_controllers() {
        let flow = run(
            &[
                ("package.json", PKG),
                ("src/api.ts", "await NestFactory.create(ApiModule);\n"),
                (
                    "src/maint.ts",
                    "await NestFactory.create(MaintenanceModule);\n",
                ),
                (
                    "src/app.module.ts",
                    "const middleware = [Logger, { provide: APP_GUARD, useClass: AuthGuard }];\n\
                     @Module({ controllers: [...controllers], providers: [...middleware] })\n\
                     export class ApiModule {}\n\
                     @Module({ controllers: [MaintController], providers: [{ provide: APP_GUARD, useClass: MaintGuard }] })\n\
                     export class MaintenanceModule {}\n",
                ),
                (
                    "src/controllers/index.ts",
                    "export const controllers = [CatsController];\n",
                ),
                (
                    "src/cats.controller.ts",
                    "@Controller('cats')\nexport class CatsController { @Get() list() {} }\n",
                ),
                (
                    "src/maint.controller.ts",
                    "@Controller('maint')\nexport class MaintController { @Get() status() {} }\n",
                ),
            ],
            "",
        );
        assert_eq!(ep(&flow, "GET", "/cats").access, prot(custom("AuthGuard")));
        assert_eq!(
            ep(&flow, "GET", "/maint").access,
            prot(custom("MaintGuard"))
        );
        assert!(flow.complete, "{:#?}", flow.diagnostics);
    }

    #[test]
    fn composite_decorators_are_followed_one_level() {
        let flow = run(
            &[
                ("package.json", PKG),
                (
                    "src/enum.ts",
                    "export enum Permission {\n  AssetRead = 'asset.read',\n  All = 'all',\n}\n",
                ),
                (
                    "src/auth.ts",
                    r#"export const Authenticated = (options: AuthenticatedOptions = {}): ReturnType<typeof applyDecorators> => {
  const decorators: MethodDecorator[] = [SetMetadata(MetadataKey.AuthRoute, options)];
  if (!options.public) { decorators.push(ApiBearerAuth()); }
  return applyDecorators(...decorators);
};
export function Auth(...roles: Role[]) {
  return applyDecorators(UseGuards(JwtGuard, RolesGuard), Roles(...roles), ApiBearerAuth());
}
export const Docs = (options) => SetMetadata('docs', options);
"#,
                ),
                (
                    "src/main.ts",
                    "await NestFactory.create(AppModule);\n\
                     @Module({ providers: [{ provide: APP_GUARD, useClass: AuthGuard }] }) class AppModule {}\n\
                     export enum MetadataKey { AuthRoute = 'auth_route' }\n\
                     export class AuthGuard { canActivate(c) { return this.reflector.getAllAndOverride<AuthenticatedOptions>(MetadataKey.AuthRoute, [c.getHandler(), c.getClass()]); } }\n",
                ),
                (
                    "src/roles.guard.ts",
                    &roles_guard("getAllAndOverride('roles', [ctx.getHandler(), ctx.getClass()])"),
                ),
                (
                    "src/assets.controller.ts",
                    r#"@Controller('assets')
export class AssetController {
  @Get() @Authenticated({ permission: Permission.AssetRead }) list() {}
  @Get('admin') @Authenticated({ permission: Permission.All, admin: true }) admin() {}
  @Get('public') @Authenticated({ public: true }) open() {}
  @Get('plain') @Authenticated() plain() {}
  @Get('none') @Authenticated({ permission: false }) none() {}
  @Get('shared') @Authenticated({ sharedLink: true, permission: Permission.AssetRead }) shared() {}
  @Get('docs') @Docs({ summary: 'x' }) docs() {}
  @Get('roles') @Auth('admin', 'owner') roles() {}
}
"#,
                ),
            ],
            "AuthGuard = \"authenticated\"\n",
        );
        let auth = Condition::Authenticated;
        let all = |c: Vec<Condition>| prot(Condition::All(c));
        let policy = |p: &str| Condition::Policy(p.into());
        assert_eq!(
            ep(&flow, "GET", "/assets").access,
            all(vec![auth.clone(), policy("asset.read")])
        );
        assert_eq!(
            ep(&flow, "GET", "/assets/admin").access,
            all(vec![auth.clone(), policy("all"), role("admin")])
        );
        assert_eq!(ep(&flow, "GET", "/assets/public").access, AccessState::Open);
        assert_eq!(ep(&flow, "GET", "/assets/plain").access, prot(auth.clone()));
        assert_eq!(ep(&flow, "GET", "/assets/none").access, prot(auth.clone()));
        assert_eq!(ep(&flow, "GET", "/assets/docs").access, prot(auth.clone()));
        assert_eq!(
            ep(&flow, "GET", "/assets/shared").access,
            all(vec![auth.clone(), policy("asset.read")])
        );
        assert!(
            (flow.diagnostics.iter()).any(|d| d.severity == DiagnosticSeverity::Note
                && d.message.contains("sharedLink")),
            "{:#?}",
            flow.diagnostics
        );
        assert_eq!(
            ep(&flow, "GET", "/assets/roles").access,
            all(vec![
                auth,
                custom("JwtGuard"),
                custom("RolesGuard"),
                Condition::Any(vec![role("admin"), role("owner")])
            ])
        );
        assert!(flow.complete, "{:#?}", flow.diagnostics);
    }

    #[test]
    fn cqrs_bus_commands_and_queries_are_sends() {
        let flow = run(
            &[
                ("package.json", PKG),
                (
                    "src/orders.controller.ts",
                    r#"@Controller('orders')
export class OrdersController {
  constructor(private readonly commandBus: CommandBus, private queryBus: QueryBus) {}
  @Post()
  async create(@Body() dto: Dto) {
    await this.service.audit(dto);
    return this.commandBus.execute(
      new CreateOrderCommand(dto.id),
    );
  }
  @Get(':id')
  one(@Param('id') id: string) { return this.queryBus.execute<GetOrderQuery, Order>(new GetOrderQuery(id)); }
}
"#,
                ),
            ],
            "",
        );
        assert_eq!(ep(&flow, "POST", "/orders").sends, ["CreateOrderCommand"]);
        assert_eq!(ep(&flow, "GET", "/orders/:id").sends, ["GetOrderQuery"]);
    }

    #[test]
    fn uri_versioning_from_literals() {
        let flow = run(
            &[
                ("package.json", PKG),
                (
                    "src/main.ts",
                    "app.enableVersioning({ type: VersioningType.URI, defaultVersion: '1' });\n",
                ),
                (
                    "src/cats.controller.ts",
                    "@Controller({ path: 'cats', version: '2' })\nexport class Cats {\n\
                     @Get() list() {}\n@Version(['3', VERSION_NEUTRAL])\n@Get('x') x() {}\n}\n\
                     @Controller('dogs')\nexport class Dogs { @Get() list() {} }\n",
                ),
            ],
            "",
        );
        let paths: Vec<&str> = flow.endpoints.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, ["/cats/x", "/v1/dogs", "/v2/cats", "/v3/cats/x"]);
    }

    #[test]
    fn unreadable_parts_are_gaps_never_guesses() {
        let flow = run(
            &[
                ("package.json", PKG),
                ("src/main.ts", "app.setGlobalPrefix(config.prefix);\n"),
                (
                    "src/x.controller.ts",
                    "@Controller('x')\nexport class X {\n\
                     @Get(dynamicPath) dynamic() {}\n\
                     @Get('g') @UseGuards(...guards) g() {}\n\
                     @Get('r') @Roles(someRole()) r() {}\n}\n",
                ),
            ],
            "",
        );
        assert!(!flow.complete);
        let gaps = gaps(&flow);
        assert!(gaps.iter().any(|m| m.contains("dynamicPath")), "{gaps:#?}");
        assert!(
            gaps.iter().any(|m| m.contains("config.prefix")),
            "{gaps:#?}"
        );
        assert!(
            !flow
                .endpoints
                .iter()
                .any(|e| e.handler.as_deref() == Some("X.dynamic"))
        );
        assert!(matches!(
            ep(&flow, "GET", "/x/g").access,
            AccessState::Protected(Condition::Unknown(_))
        ));
        // Unreadable roles are a gap; metadata no guard reads is no condition.
        assert_eq!(ep(&flow, "GET", "/x/r").access, AccessState::Undeclared);
        assert!(gaps.iter().any(|m| m.contains("someRole")), "{gaps:#?}");
    }

    const PUBLIC_GUARD: &str = r#"export const IS_PUBLIC_KEY = 'isPublic';
export const Public = () => SetMetadata(IS_PUBLIC_KEY, true);
@Injectable()
export class JwtAuthGuard implements CanActivate {
  constructor(private reflector: Reflector) {}
  canActivate(ctx: ExecutionContext) {
    const isPublic = this.reflector.getAllAndOverride<boolean>(IS_PUBLIC_KEY, [ctx.getHandler(), ctx.getClass()]);
    if (isPublic) return true;
    return super.canActivate(ctx);
  }
}
"#;

    fn roles_guard(call: &str) -> String {
        format!(
            "export const ROLES_KEY = 'roles';\n\
             export class RolesGuard implements CanActivate {{\n\
               canActivate(ctx) {{ const r = this.reflector.{call}; return !r || r.some(x => user.roles.includes(x)); }}\n\
             }}\n"
        )
    }

    fn notes(flow: &UiFlow) -> Vec<&str> {
        (flow.diagnostics.iter())
            .filter(|d| d.severity == DiagnosticSeverity::Note)
            .map(|d| d.message.as_str())
            .collect()
    }

    fn app_module(guards: &str) -> String {
        let providers: Vec<String> = (guards.split(',').filter(|g| !g.is_empty()))
            .map(|g| format!("{{ provide: APP_GUARD, useClass: {g} }}"))
            .collect();
        format!(
            "@Module({{ controllers: [A], providers: [{}] }})\nexport class AppModule {{}}\n",
            providers.join(", ")
        )
    }

    #[test]
    fn nf1_a_route_without_any_guard_is_undeclared() {
        let flow = run(
            &[
                ("package.json", PKG),
                ("src/a.ts", "@Controller('a')\nclass A { @Get() f() {} }\n"),
            ],
            "",
        );
        assert_eq!(ep(&flow, "GET", "/a").access, AccessState::Undeclared);
    }

    #[test]
    fn nf2_roles_without_a_reading_guard_is_a_note_not_a_condition() {
        let flow = run(
            &[
                ("package.json", PKG),
                (
                    "src/a.ts",
                    "@Controller('a')\nclass A { @Roles('admin') @Get() f() {} }\n",
                ),
            ],
            "",
        );
        assert_eq!(ep(&flow, "GET", "/a").access, AccessState::Undeclared);
        let notes = notes(&flow);
        assert!(
            notes
                .iter()
                .any(|n| n.contains("roles") && n.contains("no guard reads")),
            "{notes:#?}"
        );
    }

    #[test]
    fn nf3_public_is_open_when_the_only_guard_reads_its_key() {
        let flow = run(
            &[
                ("package.json", PKG),
                ("src/jwt.ts", PUBLIC_GUARD),
                ("src/app.module.ts", &app_module("JwtAuthGuard")),
                (
                    "src/a.ts",
                    "@Controller('a')\nclass A { @Public() @Get('login') login() {} @Get() f() {} }\n",
                ),
            ],
            "Public = \"open\"\n",
        );
        assert_eq!(ep(&flow, "GET", "/a/login").access, AccessState::Open);
        assert_eq!(ep(&flow, "GET", "/a").access, prot(custom("JwtAuthGuard")));
    }

    #[test]
    fn nf4_public_does_not_open_a_method_guard_that_ignores_the_key() {
        let flow = run(
            &[
                ("package.json", PKG),
                ("src/jwt.ts", PUBLIC_GUARD),
                (
                    "src/roles.ts",
                    &roles_guard(
                        "getAllAndOverride(ROLES_KEY, [ctx.getHandler(), ctx.getClass()])",
                    ),
                ),
                ("src/app.module.ts", &app_module("JwtAuthGuard")),
                (
                    "src/a.ts",
                    "@Controller('a')\nclass A {\n@Public() @UseGuards(RolesGuard) @Roles('admin') @Get('x') f() {}\n}\n",
                ),
            ],
            "Public = \"open\"\n",
        );
        assert_eq!(
            ep(&flow, "GET", "/a/x").access,
            prot(Condition::All(vec![custom("RolesGuard"), role("admin")]))
        );
    }

    #[test]
    fn nf5_public_leaves_the_global_guard_that_does_not_read_it() {
        let flow = run(
            &[
                ("package.json", PKG),
                ("src/jwt.ts", PUBLIC_GUARD),
                (
                    "src/key.ts",
                    "export class ApiKeyGuard implements CanActivate { canActivate(c) { return check(c); } }\n",
                ),
                ("src/app.module.ts", &app_module("JwtAuthGuard,ApiKeyGuard")),
                (
                    "src/a.ts",
                    "@Controller('a')\nclass A { @Public() @Get() f() {} }\n",
                ),
            ],
            "Public = \"open\"\n",
        );
        assert_eq!(ep(&flow, "GET", "/a").access, prot(custom("ApiKeyGuard")));
    }

    #[test]
    fn nf6_public_nobody_reads_has_no_effect() {
        let flow = run(
            &[
                ("package.json", PKG),
                (
                    "src/pub.ts",
                    "export const Public = () => SetMetadata('isPublic', true);\n",
                ),
                (
                    "src/a.ts",
                    "@Controller('a')\nclass A { @Public() @Get() f() {} }\n",
                ),
            ],
            "Public = \"open\"\n",
        );
        assert_eq!(ep(&flow, "GET", "/a").access, AccessState::Undeclared);
        let notes = notes(&flow);
        assert!(
            notes
                .iter()
                .any(|n| n.contains("isPublic") && n.contains("no guard reads")),
            "{notes:#?}"
        );
    }

    #[test]
    fn nf7_the_guards_lookup_decides_class_and_method_roles() {
        let run_with = |call: &str| {
            run(
                &[
                    ("package.json", PKG),
                    ("src/roles.ts", &roles_guard(call)),
                    (
                        "src/a.ts",
                        "@Controller('a')\n@UseGuards(RolesGuard)\n@Roles('a')\nclass A {\n\
                         @Roles('b') @Get('m') m() {}\n@Get('c') c() {}\n}\n",
                    ),
                ],
                "",
            )
        };
        let flow = run_with("getAllAndOverride(ROLES_KEY, [ctx.getHandler(), ctx.getClass()])");
        let g = custom("RolesGuard");
        let all = |c: Vec<Condition>| prot(Condition::All(c));
        assert_eq!(
            ep(&flow, "GET", "/a/m").access,
            all(vec![g.clone(), role("b")])
        );
        assert_eq!(
            ep(&flow, "GET", "/a/c").access,
            all(vec![g.clone(), role("a")])
        );
        let flow = run_with("getAllAndMerge(ROLES_KEY, [ctx.getHandler(), ctx.getClass()])");
        assert_eq!(
            ep(&flow, "GET", "/a/m").access,
            all(vec![g.clone(), Condition::Any(vec![role("a"), role("b")])])
        );
        let flow = run_with("get(ROLES_KEY, ctx.getHandler())");
        assert_eq!(
            ep(&flow, "GET", "/a/m").access,
            all(vec![g.clone(), role("b")])
        );
        assert_eq!(ep(&flow, "GET", "/a/c").access, prot(g));
    }

    #[test]
    fn a_guard_reads_the_key_through_a_helper_function_or_a_multiline_generic() {
        let flow = run(
            &[
                ("package.json", PKG),
                (
                    "src/auth.guard.ts",
                    "export const options = (reflector: Reflector, t) => {\n\
                     \x20 return reflector.getAllAndOverride<Opts | undefined>(KEY, t);\n};\n\
                     export const Public = () => SetMetadata(KEY, true);\n\
                     export class AuthGuard { canActivate(c) { return options(this.reflector, [c.getHandler()]); } }\n\
                     export class MaintGuard {\n  canActivate(c) {\n\
                     \x20   return this.reflector.getAllAndOverride<{ _x: never } | undefined>(\n      KEY,\n      [c.getHandler()],\n    );\n  }\n}\n",
                ),
                ("src/app.module.ts", &app_module("AuthGuard,MaintGuard")),
                (
                    "src/a.ts",
                    "@Controller('a')\nclass A { @Public() @Get() f() {} }\n",
                ),
            ],
            "Public = \"open\"\n",
        );
        assert_eq!(ep(&flow, "GET", "/a").access, AccessState::Open);
    }

    #[test]
    fn a_marker_counts_only_at_the_level_the_guards_lookup_covers() {
        let flow = run(
            &[
                ("package.json", PKG),
                (
                    "src/g.ts",
                    "export const Public = () => SetMetadata('pub', true);\n\
                     export class HandlerGuard { canActivate(c) { return this.reflector.get('pub', c.getHandler()); } }\n",
                ),
                ("src/app.module.ts", &app_module("HandlerGuard")),
                (
                    "src/a.ts",
                    "@Public()\n@Controller('c')\nclass C { @Get() f() {} }\n\
                     @Controller('m')\nclass M { @Public() @Get() f() {} }\n",
                ),
            ],
            "Public = \"open\"\n",
        );
        assert_eq!(ep(&flow, "GET", "/c").access, prot(custom("HandlerGuard")));
        assert_eq!(ep(&flow, "GET", "/m").access, AccessState::Open);
    }
}
