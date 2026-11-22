//! Flask endpoints and their access for ui-flow/v1 (ADR 0021).
//!
//! A text-level scan of the `.py` files below the root (test files left out)
//! on the shared Python helpers of `lang_python`; only when some file creates
//! a `Flask` app or a `Blueprint`. It fills `endpoints` and `diagnostics`.
//! Templates (screens, links) are not read.
//!
//! Objects: `x = Flask(…)` and `x = Blueprint(name, …, url_prefix=…)` (also
//! inside an app factory), `p.register_blueprint(c, url_prefix=…)` (the
//! register-time prefix wins; nested blueprints join prefixes as Flask does).
//! Names follow `import` / `from … import` through the project. Routes:
//! `@x.route(path, methods=[…])` (`GET` without `methods`), `@x.get/post/put/
//! patch/delete`, `x.add_url_rule(path, view_func=f | V.as_view(…))` with the
//! methods of a `MethodView` class. A path converter `<int:id>` reads `:id`.
//!
//! Access: only the decorators BELOW a route decorator apply (one above it
//! wraps nothing that is registered: a note). A decorator is read by its
//! resolved import: `flask_login` (`login_required`, `fresh_login_required`),
//! `flask_security` (`auth_required`, `roles_*`, `permissions_*`,
//! `anonymous_user_required`, …), `flask_jwt_extended.jwt_required`, a
//! `flask_principal.Permission(…)` object's `.require(…)` and a
//! `flask_httpauth` object's `.login_required(role=…, optional=…)`. Names
//! listed in `[views.access]` take their meaning; any other decorator on a
//! route is `unknown` with a gap, never open. `Open` only from an explicit
//! marker (`jwt_required(optional=True)`, `optional=True`, a listed `open`)
//! and only when nothing else guards. A `before_request` /
//! `before_app_request` function that carries a guard decorator guards its
//! blueprint (and nested ones) or the whole app; a body-only hook is a note,
//! unless it is listed in `[views.access]`: then its allow-list is unreadable
//! (`unknown` + gap) when the body reads `request.endpoint`/`path`/…. A class
//! view's `decorators = […]` apply to its methods; a decorator on the class
//! itself does not. A `LOGIN_DISABLED` setting is a gap.
//! Not read: Flask-RESTful/-RESTX/-Smorest routes, `app.url_map`, route
//! decorators on class methods other than a `MethodView`.
use crate::extract_ctx::{ExtractContext, ancestors, cap, dir_name, finish, gap, read_text, walk};
use crate::lang_python::{self as py, Arg, Def, DefKind, Import, Stmt};
use crate::lang_text::line;
use crate::services::support::test_path;
use crate::ui_flow_fastapi::all;
use archguard_core::{
    AccessMeaning, AccessState, Condition, DiagnosticSeverity, EndpointAccess, UiFlow,
    UiFlowDiagnostic,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};

const MAX_FILES: usize = 20_000;
const BUILD_FILES: [&str; 4] = [
    "pyproject.toml",
    "setup.py",
    "setup.cfg",
    "requirements.txt",
];
const VERBS: [&str; 5] = ["get", "post", "put", "patch", "delete"];
/// What a `before_request` body reads when it keeps an allow-list.
const EXEMPT: [&str; 6] = [
    "request.endpoint",
    "request.path",
    "request.blueprint",
    "request.url_rule",
    "request.full_path",
    "request.view_args",
];

type Key = (usize, String);

/// [`crate::ui_flow::extract_all`]'s entry: endpoints of every Flask app below the root.
pub fn extract_with(ctx: &ExtractContext) -> UiFlow {
    let mut flow = UiFlow::default();
    let walked = match walk(ctx.root) {
        Ok(w) => w,
        Err(e) => {
            let message = format!("Discovery exclusions not read; no Flask app analyzed: {e}");
            gap(&mut flow.diagnostics, "", 0, message);
            finish(&mut flow);
            return flow;
        }
    };
    let (mut paths, mut build_dirs) = (Vec::new(), BTreeSet::new());
    for rel in walked.files {
        let (dir, name) = rel.rsplit_once('/').unwrap_or(("", &rel));
        if BUILD_FILES.contains(&name) {
            build_dirs.insert(dir.to_string());
        }
        if rel.ends_with(".py") && !test_path(&rel) {
            paths.push(rel);
        }
    }
    paths.sort();
    let cut = cap(&mut paths, MAX_FILES);
    let mut files = Vec::new();
    let mut unread = Vec::new();
    for rel in paths {
        match read_text(ctx.root, &rel, 1 << 20) {
            Ok(text) => {
                let application = (ancestors(&rel))
                    .find(|d| build_dirs.contains(*d))
                    .map(|d| dir_name(ctx.root, d));
                files.push(FileInfo::new(rel, text, application));
            }
            Err(e) => unread.push((rel, format!("File not read ({e}); its routes are unknown."))),
        }
    }
    if !(files.iter()).any(|f| f.src.contains("Flask(") || f.src.contains("Blueprint(")) {
        return flow;
    }
    if let Some(first) = cut {
        let message =
            format!("Flask scan stopped after {MAX_FILES} Python files; later files are unknown.");
        gap(&mut flow.diagnostics, &first, 0, message);
    }
    for (rel, message) in unread {
        gap(&mut flow.diagnostics, &rel, 0, message);
    }
    if let Some(e) = &ctx.config_error {
        let message = format!("Configuration not read; [views.access] not applied: {e}");
        note(&mut flow.diagnostics, ".", 0, message);
    }
    let set = files.iter().map(|f| f.rel.clone()).collect();
    let index = files
        .iter()
        .enumerate()
        .map(|(i, f)| (f.rel.clone(), i))
        .collect();
    let project = Project {
        files,
        set,
        index,
        access: &ctx.config.views.access,
    };
    project.emit(&mut flow);
    finish(&mut flow);
    flow
}

fn note(out: &mut Vec<UiFlowDiagnostic>, file: &str, at: u32, message: impl Into<String>) {
    let severity = DiagnosticSeverity::Note;
    let (file, line, message) = (file.into(), at, message.into());
    out.push(UiFlowDiagnostic {
        file,
        line,
        message,
        severity,
    });
}

struct FileInfo {
    rel: String,
    src: String,
    masked: String,
    stmts: Vec<Stmt>,
    defs: Vec<Def>,
    imports: Vec<Import>,
    /// Name → value range of its first assignment (any depth).
    assigns: HashMap<String, (usize, usize)>,
    application: Option<String>,
}

impl FileInfo {
    fn new(rel: String, src: String, application: Option<String>) -> Self {
        let masked = py::mask(&src);
        let stmts = py::code_statements(&src, &masked);
        let mut assigns = HashMap::new();
        for s in &stmts {
            if let Some(a) = py::assignment(&src, &masked, s)
                && !a.augmented
                && !a.target.contains('.')
            {
                assigns
                    .entry(a.target.to_string())
                    .or_insert((a.value_at, a.value_at + a.value.len()));
            }
        }
        let (defs, imports) = (py::defs(&masked), py::imports(&masked));
        Self {
            rel,
            src,
            masked,
            stmts,
            defs,
            imports,
            assigns,
            application,
        }
    }

    fn line(&self, at: usize) -> u32 {
        line(&self.src, at)
    }

    /// `callee(…)` starting at `at`: the dotted callee and the offset of `(`.
    fn call_at(&self, at: usize) -> Option<(&str, usize)> {
        let b = self.masked.as_bytes();
        let n = (b[at..].iter())
            .take_while(|&&c| c.is_ascii_alphanumeric() || c == b'_' || c == b'.')
            .count();
        (n > 0 && b.get(at + n) == Some(&b'(')).then(|| (&self.masked[at..at + n], at + n))
    }

    /// Offset of the value of `a` (a keyword argument's `at` is its name's).
    fn vat(&self, a: &Arg<'_>) -> usize {
        match a.name {
            Some(n) => {
                let from = a.at + n.len();
                from + self.src[from..].find(a.value).unwrap_or(0)
            }
            None => a.at,
        }
    }

    /// `callee(args)` or a bare dotted name exactly covering `at..end`.
    fn callish(&self, at: usize, end: usize) -> Option<(&str, Vec<Arg<'_>>)> {
        if let Some(call) = self.whole_call(at, end) {
            return Some(call);
        }
        let text = &self.masked[at..end];
        let plain = text
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'.');
        (!text.is_empty() && plain).then(|| (text, Vec::new()))
    }

    /// The call exactly covering `at..end`: callee and arguments.
    fn whole_call(&self, at: usize, end: usize) -> Option<(&str, Vec<Arg<'_>>)> {
        let (callee, open) = self.call_at(at)?;
        let (args, close) = py::call_args(&self.src, &self.masked, open)?;
        (close + 1 == end).then_some((callee, args))
    }

    /// The items of the list or tuple `text` at `at`.
    fn items(&self, at: usize, text: &str) -> Option<Vec<(usize, &str)>> {
        let list = (text.starts_with('[') && text.ends_with(']'))
            || (text.starts_with('(') && text.ends_with(')'));
        list.then(|| {
            (py::split_commas(&self.masked, at + 1, at + text.len() - 1).into_iter())
                .map(|(a, e)| (a, self.src[a..e].trim()))
                .filter(|(_, t)| !t.is_empty())
                .collect()
        })
    }

    fn strings(&self, at: usize, text: &str) -> Option<Vec<String>> {
        let items = self.items(at, text)?;
        items
            .into_iter()
            .map(|(_, t)| py::string_value(t))
            .collect()
    }
}

/// What a name resolves to.
enum Q {
    /// An unscanned module's member, dotted (`flask_login.login_required`).
    Ext(String),
    /// A name assigned or defined in a project file.
    Local(usize, String),
    Gone,
}

enum Obj {
    App,
    /// The constructor's `url_prefix`: `Err` with its text when unreadable.
    Bp(Option<Result<String, String>>),
    /// Needs of a literal `Permission(…)`; whether one is unreadable.
    Perm(Vec<Condition>, bool),
    /// `p.union(q)` and the like on a known permission.
    Derived,
    Auth,
}

/// The access effect of decorators.
#[derive(Clone, Default)]
struct Eff {
    conds: Vec<Condition>,
    open: bool,
    gaps: Vec<String>,
}

impl Eff {
    fn cond(c: Condition) -> Self {
        Self {
            conds: vec![c],
            ..Self::default()
        }
    }

    fn open() -> Self {
        Self {
            open: true,
            ..Self::default()
        }
    }

    fn unknown(name: &str, why: &str) -> Self {
        let gaps = vec![format!(
            "Decorator `{name}` is not read ({why}); this endpoint's access is unknown."
        )];
        Self {
            conds: vec![Condition::Unknown(name.into())],
            gaps,
            ..Self::default()
        }
    }

    fn extend(&mut self, other: Eff) {
        self.conds.extend(other.conds);
        self.open |= other.open;
        self.gaps.extend(other.gaps);
    }

    fn is_empty(&self) -> bool {
        self.conds.is_empty() && !self.open && self.gaps.is_empty()
    }

    fn state(&self) -> AccessState {
        match (self.conds.is_empty(), self.open) {
            (false, _) => AccessState::Protected(all(self.conds.clone())),
            (true, true) => AccessState::Open,
            (true, false) => AccessState::Undeclared,
        }
    }
}

fn any_of(mut items: Vec<Condition>) -> Condition {
    match items.len() {
        1 => items.remove(0),
        _ => Condition::Any(items),
    }
}

fn root_last(full: &str) -> (&str, &str) {
    let root = full.split('.').next().unwrap_or("");
    (root, full.rsplit('.').next().unwrap_or(""))
}

/// `callee(a, k=v)` as written.
fn show(callee: &str, args: &[Arg<'_>]) -> String {
    let parts: Vec<String> = (args.iter())
        .map(|a| match a.name {
            Some(n) => format!("{n}={}", a.value),
            None => a.value.to_string(),
        })
        .collect();
    format!("{callee}({})", parts.join(", "))
}

fn bool_lit(text: &str) -> Option<bool> {
    match text {
        "True" => Some(true),
        "False" => Some(false),
        _ => None,
    }
}

struct Project<'a> {
    files: Vec<FileInfo>,
    set: BTreeSet<String>,
    index: HashMap<String, usize>,
    access: &'a BTreeMap<String, AccessMeaning>,
}

impl Project<'_> {
    /// What dotted `name` of file `f` is: a name of the project (with the
    /// attributes left over) or a member of an unscanned module.
    fn qualify(&self, f: usize, name: &str) -> (Q, Vec<String>) {
        let mut segs = name.split('.').map(String::from);
        let first = segs.next().unwrap_or_default();
        self.chase(f, &first, segs.collect(), 0)
    }

    fn chase(&self, f: usize, name: &str, rest: Vec<String>, depth: usize) -> (Q, Vec<String>) {
        let fi = &self.files[f];
        if fi.assigns.contains_key(name) || fi.defs.iter().any(|d| d.name == name) {
            return (Q::Local(f, name.into()), rest);
        }
        let Some(imp) = fi
            .imports
            .iter()
            .find(|i| i.local == name)
            .filter(|_| depth < 16)
        else {
            return (Q::Gone, rest);
        };
        match py::resolve_import(&fi.rel, imp, &self.set) {
            Some((file, Some(sym))) => self.chase(self.index[&file], &sym, rest, depth + 1),
            Some((file, None)) => match rest.split_first() {
                Some((next, tail)) => self.chase(self.index[&file], next, tail.to_vec(), depth + 1),
                None => (Q::Gone, rest),
            },
            None => {
                let mut full = imp.module.clone();
                full.extend(imp.name.iter().map(|n| format!(".{n}")));
                full.extend(rest.iter().map(|r| format!(".{r}")));
                (Q::Ext(full), Vec::new())
            }
        }
    }

    fn obj(&self, f: usize, name: &str, depth: usize) -> Option<Obj> {
        let fi = &self.files[f];
        let &(a, _) = fi.assigns.get(name)?;
        let (callee, open) = fi.call_at(a)?;
        match self.qualify(f, callee) {
            (Q::Ext(full), _) => match root_last(&full) {
                ("flask", "Flask") => Some(Obj::App),
                ("flask", "Blueprint") => {
                    let (args, _) = py::call_args(&fi.src, &fi.masked, open)?;
                    Some(Obj::Bp(prefix_arg(&args)))
                }
                ("flask_principal", "Permission") => self.permission(f, open),
                ("flask_httpauth", last) if last.starts_with("HTTP") || last == "MultiAuth" => {
                    Some(Obj::Auth)
                }
                _ => None,
            },
            (Q::Local(lf, ln), attrs) if attrs.len() == 1 && depth < 8 => {
                matches!(self.obj(lf, &ln, depth + 1), Some(Obj::Perm(..))).then_some(Obj::Derived)
            }
            _ => None,
        }
    }

    fn permission(&self, f: usize, open: usize) -> Option<Obj> {
        let fi = &self.files[f];
        let (args, _) = py::call_args(&fi.src, &fi.masked, open)?;
        let needs: Vec<Condition> = (args.iter().filter(|x| x.name.is_none()))
            .map(|x| self.need(f, x))
            .collect();
        let bad = needs.is_empty() || needs.iter().any(|c| matches!(c, Condition::Unknown(_)));
        let needs = match needs.is_empty() {
            true => vec![Condition::Unknown("Permission()".into())],
            false => needs,
        };
        Some(Obj::Perm(needs, bad))
    }

    fn need(&self, f: usize, arg: &Arg<'_>) -> Condition {
        let fi = &self.files[f];
        let unread = || Condition::Unknown(arg.value.into());
        let Some((callee, inner)) = fi.whole_call(arg.at, arg.at + arg.value.len()) else {
            return unread();
        };
        let lit = match inner.as_slice() {
            [a] if a.name.is_none() => py::string_value(a.value).or_else(|| {
                a.value
                    .bytes()
                    .all(|c| c.is_ascii_digit())
                    .then(|| a.value.to_string())
            }),
            _ => None,
        };
        match (callee.rsplit('.').next(), lit) {
            (Some("RoleNeed"), Some(v)) => Condition::Role(v),
            (Some("ActionNeed"), Some(v)) => Condition::Policy(v),
            (Some("UserNeed"), Some(v)) => Condition::Custom(format!("user:{v}")),
            _ => unread(),
        }
    }

    /// The app or blueprint a dotted receiver names.
    fn owner(&self, f: usize, receiver: &str) -> Option<(Key, Obj)> {
        match self.qualify(f, receiver) {
            (Q::Local(lf, ln), attrs) if attrs.is_empty() => {
                let o = self.obj(lf, &ln, 0)?;
                matches!(o, Obj::App | Obj::Bp(_)).then_some(((lf, ln), o))
            }
            _ => None,
        }
    }

    /// The effect of one decorator expression `src[at..end]` of file `f`.
    fn decorator(&self, f: usize, at: usize, end: usize) -> Eff {
        let fi = &self.files[f];
        let text = fi.src[at..end].trim_end();
        let end = at + text.len();
        let (callee, args) = match fi.whole_call(at, end) {
            Some((c, a)) => (c, Some(a)),
            None => match fi.call_at(at) {
                None if text
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'.') =>
                {
                    (text, None)
                }
                _ => return Eff::unknown(text, "not a plain name or call"),
            },
        };
        let shown = |args: &Option<Vec<Arg<'_>>>| match args {
            Some(a) => show(callee, a),
            None => callee.to_string(),
        };
        let (q, attrs) = self.qualify(f, callee);
        let by_name = || self.listed(callee, args.as_deref().unwrap_or(&[]));
        match (q, attrs.as_slice()) {
            (Q::Ext(full), []) => library(&full, &args, &shown(&args)).unwrap_or_else(by_name),
            (Q::Local(lf, ln), [attr]) => match (self.obj(lf, &ln, 0), attr.as_str()) {
                (Some(Obj::Perm(needs, bad)), "require") => {
                    let mut e = Eff::cond(any_of(needs));
                    if bad {
                        e.gaps.push(format!("Permission `{ln}` has a need that is not a literal; its access is unknown."));
                    }
                    e
                }
                (Some(Obj::Derived), "require") => {
                    Eff::unknown(&ln, "a derived Permission is not read")
                }
                (Some(Obj::Auth), "login_required") => self.httpauth(f, callee, &args),
                _ => by_name(),
            },
            _ => by_name(),
        }
    }

    /// `[views.access]` meaning of decorator `callee` (by its last segment).
    fn listed(&self, callee: &str, args: &[Arg<'_>]) -> Eff {
        let name = callee.rsplit('.').next().unwrap_or(callee);
        let Some(m) = self.access.get(name) else {
            return Eff::unknown(callee, "not a known guard and not listed in [views.access]");
        };
        let lits: Vec<String> = (args.iter())
            .filter(|a| a.name.is_none())
            .filter_map(|a| py::string_value(a.value))
            .collect();
        let from_args = |make: fn(String) -> Condition| match lits.is_empty() {
            true => Eff::unknown(name, "its role or policy arguments are not literals"),
            false => Eff::cond(any_of(lits.iter().cloned().map(make).collect())),
        };
        match m.at_use_site(!args.is_empty()) {
            AccessMeaning::Open => Eff::open(),
            AccessMeaning::Authenticated => Eff::cond(Condition::Authenticated),
            AccessMeaning::Never => Eff::cond(Condition::Never),
            AccessMeaning::Role(n) => Eff::cond(Condition::Role(n.clone())),
            AccessMeaning::Policy(n) => Eff::cond(Condition::Policy(n.clone())),
            AccessMeaning::RoleFromArguments => from_args(Condition::Role),
            AccessMeaning::PolicyFromArguments => from_args(Condition::Policy),
            AccessMeaning::FromArgumentsOr(..) => Eff::unknown(name, "unresolved fallback meaning"),
        }
    }

    /// `X.login_required` of a Flask-HTTPAuth object, bare or called.
    fn httpauth(&self, f: usize, callee: &str, args: &Option<Vec<Arg<'_>>>) -> Eff {
        let Some(a) = args else {
            return Eff::cond(Condition::Authenticated);
        };
        let shown = show(callee, a);
        let fi = &self.files[f];
        match a
            .iter()
            .find(|x| x.name == Some("optional"))
            .map(|x| bool_lit(x.value))
        {
            Some(Some(true)) => return Eff::open(),
            Some(None) => return Eff::unknown(&shown, "optional is not a literal"),
            _ => {}
        }
        let Some(role) = a.iter().find(|x| x.name == Some("role")) else {
            return Eff::cond(Condition::Authenticated);
        };
        let one = |t: &str| py::string_value(t).map(Condition::Role);
        let parsed = one(role.value).or_else(|| {
            let items = fi.items(fi.vat(role), role.value)?;
            let conds: Option<Vec<Condition>> = items
                .iter()
                .map(|(at, t)| match fi.items(*at, t) {
                    Some(inner) => inner
                        .iter()
                        .map(|(_, s)| one(s))
                        .collect::<Option<Vec<_>>>()
                        .filter(|v| !v.is_empty())
                        .map(all),
                    None => one(t),
                })
                .collect();
            conds.filter(|c| !c.is_empty()).map(any_of)
        });
        match parsed {
            Some(r) => Eff::cond(all(vec![Condition::Authenticated, r])),
            None => Eff::unknown(&shown, "role is not a literal"),
        }
    }
}

/// Guards of the supported libraries by resolved import; `None` for any other.
fn library(full: &str, args: &Option<Vec<Arg<'_>>>, shown: &str) -> Option<Eff> {
    let (root, last) = root_last(full);
    let none: &[Arg<'_>] = &[];
    let a = args.as_deref().unwrap_or(none);
    let auth = || Eff::cond(Condition::Authenticated);
    let fresh = |what: &str| {
        Eff::cond(Condition::All(vec![
            Condition::Authenticated,
            Condition::Custom(what.into()),
        ]))
    };
    match (root, last) {
        ("flask_login", "login_required") => Some(auth()),
        ("flask_login", "fresh_login_required") => Some(fresh("fresh_login")),
        ("flask_security", "auth_required") => Some(auth_required(a, shown)),
        ("flask_security", "auth_token_required" | "http_auth_required") => Some(auth()),
        ("flask_security", "anonymous_user_required") => Some(Eff::cond(Condition::Not(Box::new(
            Condition::Authenticated,
        )))),
        ("flask_security", "unauth_csrf") => Some(Eff::default()),
        (
            "flask_security",
            "roles_required" | "roles_accepted" | "permissions_required" | "permissions_accepted",
        ) => Some(role_list(last, a, shown)),
        ("flask_jwt_extended", "jwt_required") if args.is_some() => Some(jwt_required(a, shown)),
        _ => None,
    }
}

fn kwarg<'a>(args: &[Arg<'a>], key: &str) -> Option<&'a str> {
    args.iter().find(|x| x.name == Some(key)).map(|x| x.value)
}

fn unreadable(shown: &str) -> Eff {
    Eff::unknown(shown, "an argument is not a literal")
}

/// `auth_required("token", within=30)`: mechanisms are literal, `within` freshens.
fn auth_required(a: &[Arg<'_>], shown: &str) -> Eff {
    let mechanisms =
        (a.iter().filter(|x| x.name.is_none())).all(|x| py::string_value(x.value).is_some());
    let within = kwarg(a, "within").map(|v| v.parse::<i64>().ok());
    if !mechanisms || within == Some(None) {
        return unreadable(shown);
    }
    match within {
        Some(Some(n)) if n > 0 => Eff::cond(Condition::All(vec![
            Condition::Authenticated,
            Condition::Custom("fresh_login".into()),
        ])),
        _ => Eff::cond(Condition::Authenticated),
    }
}

/// `roles_required("a", "b")` (all), `roles_accepted` (any), `permissions_*`.
fn role_list(name: &str, a: &[Arg<'_>], shown: &str) -> Eff {
    let pos: Vec<&Arg<'_>> = a.iter().filter(|x| x.name.is_none()).collect();
    let names: Option<Vec<String>> = pos.iter().map(|x| py::string_value(x.value)).collect();
    let Some(names) = names.filter(|n| !n.is_empty()) else {
        return unreadable(shown);
    };
    let make = if name.starts_with("roles") {
        Condition::Role
    } else {
        Condition::Policy
    };
    let items = names.into_iter().map(make).collect();
    Eff::cond(if name.ends_with("required") {
        all(items)
    } else {
        any_of(items)
    })
}

/// `jwt_required(optional=…, fresh=…, refresh=…)`; `optional=True` is open.
fn jwt_required(a: &[Arg<'_>], shown: &str) -> Eff {
    let flag = |k: &str| kwarg(a, k).map_or(Some(false), bool_lit);
    let (Some(optional), Some(fresh), Some(refresh)) =
        (flag("optional"), flag("fresh"), flag("refresh"))
    else {
        return unreadable(shown);
    };
    match (optional, refresh, fresh) {
        (true, ..) => Eff::open(),
        (_, true, _) => Eff::cond(Condition::Custom("refresh_token".into())),
        (_, _, true) => Eff::cond(Condition::All(vec![
            Condition::Authenticated,
            Condition::Custom("fresh_token".into()),
        ])),
        _ => Eff::cond(Condition::Authenticated),
    }
}

struct Reg {
    parent: Key,
    child: Key,
    prefix: Option<Result<String, String>>,
    file: usize,
    line: u32,
}

struct Item {
    method: String,
    eff: Eff,
    handler: String,
}

struct Route {
    owner: Key,
    file: usize,
    line: u32,
    path: String,
    items: Vec<Item>,
}

struct Hook {
    owner: Key,
    /// `before_app_request`: guards the whole app, not just the blueprint.
    app_wide: bool,
    eff: Eff,
}

#[derive(Default)]
struct Scan {
    regs: Vec<Reg>,
    routes: Vec<Route>,
    hooks: Vec<Hook>,
    diags: Vec<UiFlowDiagnostic>,
}

struct Place {
    chain: Vec<Key>,
    prefix: String,
}

fn join_prefix(a: &str, b: &str) -> String {
    match (a.is_empty(), b.is_empty()) {
        (true, _) => b.to_string(),
        (_, true) => a.to_string(),
        _ => format!("{}/{}", a.trim_end_matches('/'), b.trim_start_matches('/')),
    }
}

/// `/u/<int:id>` → `/u/:id`.
fn convert(path: &str) -> String {
    let (mut out, mut rest) = (String::new(), path);
    while let Some((head, tail)) = rest.split_once('<') {
        let Some((inner, after)) = tail.split_once('>') else {
            break;
        };
        out.push_str(head);
        out.push(':');
        out.push_str(inner.rsplit(':').next().unwrap_or(inner));
        rest = after;
    }
    out.push_str(rest);
    out
}

impl Project<'_> {
    fn module(&self, f: usize) -> String {
        py::module_name(&self.files[f].rel, &self.set)
    }
}

/// Reads registrations, routes and request hooks of every file.
struct Scanner<'a> {
    p: &'a Project<'a>,
    out: Scan,
}

/// A decorator of a function that registers it on an app or blueprint.
struct Registered<'a> {
    verb: String,
    owner: Key,
    args: Vec<Arg<'a>>,
}

impl<'a> Scanner<'a> {
    fn run(p: &'a Project<'a>) -> Scan {
        let mut s = Scanner {
            p,
            out: Scan::default(),
        };
        for f in 0..p.files.len() {
            s.calls(f);
            s.defs(f);
            s.login_disabled(f);
        }
        s.out
    }

    fn login_disabled(&mut self, f: usize) {
        let fi = &self.p.files[f];
        for s in &fi.stmts {
            let text = &fi.src[s.at..s.end];
            if text.contains("LOGIN_DISABLED") && !text.contains("False") {
                let message = "LOGIN_DISABLED is set: login checks may be switched off by configuration; protection by login decorators is unknown there.";
                gap(&mut self.out.diags, &fi.rel, fi.line(s.at), message);
            }
        }
    }

    fn calls(&mut self, f: usize) {
        let p = self.p;
        for s in &p.files[f].stmts {
            self.call_stmt(f, s);
        }
    }

    /// `x.register_blueprint(…)`, `x.add_url_rule(…)`, `x.before_request(f)`.
    fn call_stmt(&mut self, f: usize, s: &Stmt) -> Option<()> {
        let p = self.p;
        let fi = &p.files[f];
        let (callee, open) = fi.call_at(s.at)?;
        let (receiver, verb) = callee.rsplit_once('.')?;
        if !matches!(
            verb,
            "register_blueprint" | "add_url_rule" | "before_request"
        ) {
            return None;
        }
        let (owner, _) = p.owner(f, receiver)?;
        let (args, _) = py::call_args(&fi.src, &fi.masked, open)?;
        let at = fi.line(s.at);
        match verb {
            "register_blueprint" => self.register(f, owner, &args, at),
            "add_url_rule" => self.add_url_rule(f, owner, &args, at),
            _ => self.hook_call(f, owner, &args),
        }
        Some(())
    }

    fn register(&mut self, f: usize, owner: Key, args: &[Arg<'_>], at: u32) {
        let p = self.p;
        let first = args.iter().find(|a| a.name.is_none());
        match first.and_then(|a| p.local_bp(f, a.value)) {
            Some(child) => self.out.regs.push(Reg {
                parent: owner,
                child,
                prefix: prefix_arg(args),
                file: f,
                line: at,
            }),
            None => {
                let text = first.map_or("", |a| a.value);
                let message = format!(
                    "`register_blueprint({text})` names no readable blueprint; its routes are unknown."
                );
                gap(&mut self.out.diags, &p.files[f].rel, at, message);
            }
        }
    }

    /// `app.before_request(f)`: every decorator of `f` is applied.
    fn hook_call(&mut self, f: usize, owner: Key, args: &[Arg<'_>]) {
        let p = self.p;
        let first = args.iter().find(|a| a.name.is_none());
        let Some((lf, di)) = first.and_then(|a| p.local_def(f, a.value)) else {
            return;
        };
        let d = &p.files[lf].defs[di];
        if d.kind == DefKind::Function {
            let eff = self.applied(lf, d, 0, &[]);
            self.push_hook(lf, di, owner, false, eff);
        }
    }

    /// The effect of the decorators of `d` from index `from`, those flagged
    /// in `skip` (registrations) left out.
    fn applied(&self, f: usize, d: &Def, from: usize, skip: &[bool]) -> Eff {
        let mut eff = Eff::default();
        for (j, &(a, e)) in d.decorators.iter().enumerate() {
            if j >= from && !skip.get(j).copied().unwrap_or(false) {
                eff.extend(self.p.decorator(f, a, e));
            }
        }
        eff
    }

    fn push_hook(&mut self, f: usize, di: usize, owner: Key, app_wide: bool, mut eff: Eff) {
        let p = self.p;
        let fi = &p.files[f];
        let d = &fi.defs[di];
        eff.open = false;
        if eff.is_empty() {
            let body = &fi.masked[d.body.0..d.body.1];
            if p.access.contains_key(&d.name) {
                eff = match EXEMPT.iter().any(|k| body.contains(k)) {
                    true => Eff::unknown(&d.name, "its allow-list in the body is not read"),
                    false => p.listed(&d.name, &[]),
                };
                eff.open = false;
            } else {
                let message = format!(
                    "`{}` is registered as a request hook without a guard decorator; its body is not interpreted, so it gives no access rule.",
                    d.name
                );
                note(&mut self.out.diags, &fi.rel, d.line, message);
            }
        }
        self.out.hooks.push(Hook {
            owner,
            app_wide,
            eff,
        });
    }

    fn defs(&mut self, f: usize) {
        let p = self.p;
        let fi = &p.files[f];
        for (di, d) in fi.defs.iter().enumerate() {
            let in_class = d.parent.is_some_and(|c| fi.defs[c].kind == DefKind::Class);
            if d.kind == DefKind::Function && !d.decorators.is_empty() && !in_class {
                self.def(f, di);
            }
        }
    }

    fn def(&mut self, f: usize, di: usize) {
        let p = self.p;
        let fi = &p.files[f];
        let d = &fi.defs[di];
        let regs: Vec<Option<Registered<'a>>> = (d.decorators.iter())
            .map(|&(a, e)| {
                let (callee, args) = fi.callish(a, a + fi.src[a..e].trim_end().len())?;
                let (receiver, verb) = callee.rsplit_once('.')?;
                let (owner, _) = p.owner(f, receiver)?;
                let verb = verb.to_string();
                Some(Registered { verb, owner, args })
            })
            .collect();
        let skip: Vec<bool> = regs.iter().map(Option::is_some).collect();
        for (i, r) in regs.iter().enumerate() {
            if let Some(r) = r {
                self.registration(f, (di, i), r, &skip);
            }
        }
    }

    /// Decorator `i` of def `di` registers a route or a request hook.
    fn registration(
        &mut self,
        f: usize,
        (di, i): (usize, usize),
        r: &Registered<'_>,
        skip: &[bool],
    ) {
        let p = self.p;
        let fi = &p.files[f];
        let d = &fi.defs[di];
        let hook = matches!(r.verb.as_str(), "before_request" | "before_app_request");
        if !hook && r.verb != "route" && !VERBS.contains(&r.verb.as_str()) {
            return;
        }
        let line_at = fi.line(d.decorators[i].0);
        if (0..i).any(|j| !skip[j]) {
            let message = "A decorator above the route/hook decorator wraps nothing that is registered; it does not apply to this endpoint.";
            note(&mut self.out.diags, &fi.rel, line_at, message);
        }
        let eff = self.applied(f, d, i + 1, skip);
        if hook {
            let wide = r.verb == "before_app_request";
            return self.push_hook(f, di, r.owner.clone(), wide, eff);
        }
        match route_target(fi, r) {
            Ok((path, methods)) => {
                let handler = format!("{}.{}", p.module(f), d.name);
                let items = (methods.into_iter())
                    .map(|method| Item {
                        method,
                        eff: eff.clone(),
                        handler: handler.clone(),
                    })
                    .collect();
                let owner = r.owner.clone();
                let route = Route {
                    owner,
                    file: f,
                    line: line_at,
                    path,
                    items,
                };
                self.out.routes.push(route);
            }
            Err(message) => gap(&mut self.out.diags, &fi.rel, line_at, message),
        }
    }

    fn add_url_rule(&mut self, f: usize, owner: Key, args: &[Arg<'_>], line_at: u32) {
        let p = self.p;
        let fi = &p.files[f];
        let pos: Vec<&Arg<'_>> = args.iter().filter(|a| a.name.is_none()).collect();
        let named = |k: &str| args.iter().find(|a| a.name == Some(k));
        let path = (pos.first().copied().or(named("rule"))).and_then(|a| py::string_value(a.value));
        let view = named("view_func").or(pos.get(2).copied());
        let (Some(path), Some(view)) = (path, view) else {
            let message = "`add_url_rule` has no readable literal path and `view_func`; this endpoint is skipped.";
            return gap(&mut self.out.diags, &fi.rel, line_at, message);
        };
        match self.view_items(f, view, named("methods"), line_at) {
            Some(items) => self.out.routes.push(Route {
                owner,
                file: f,
                line: line_at,
                path,
                items,
            }),
            None => {
                let message = format!(
                    "`view_func={}` is no function or `as_view` class of the project; this endpoint is skipped.",
                    view.value
                );
                gap(&mut self.out.diags, &fi.rel, line_at, message);
            }
        }
    }

    fn view_items(
        &mut self,
        f: usize,
        view: &Arg<'_>,
        methods: Option<&Arg<'_>>,
        line_at: u32,
    ) -> Option<Vec<Item>> {
        let p = self.p;
        let fi = &p.files[f];
        let at = fi.vat(view);
        let (class, name) = match fi.whole_call(at, at + view.value.len()) {
            Some((callee, _)) => (true, callee.strip_suffix(".as_view")?),
            None => (false, view.value),
        };
        let (lf, di) = p.local_def(f, name)?;
        let d = &p.files[lf].defs[di];
        if (d.kind == DefKind::Class) != class {
            return None;
        }
        if class {
            return self.class_items(f, (lf, di), line_at);
        }
        let verbs: Vec<String> = match methods {
            Some(m) => upper(fi.strings(fi.vat(m), m.value)?),
            None => vec!["GET".to_string()],
        };
        let eff = self.applied(lf, d, 0, &[]);
        let handler = format!("{}.{}", p.module(lf), d.name);
        Some(
            (verbs.into_iter())
                .map(|method| Item {
                    method,
                    eff: eff.clone(),
                    handler: handler.clone(),
                })
                .collect(),
        )
    }

    /// The methods of a `MethodView` class (file `f` registers it).
    fn class_items(
        &mut self,
        f: usize,
        (lf, di): (usize, usize),
        line_at: u32,
    ) -> Option<Vec<Item>> {
        let p = self.p;
        let (vf, d) = (&p.files[lf], &p.files[lf].defs[di]);
        let (module, ln) = (p.module(lf), &d.name);
        if !d.decorators.is_empty() {
            let message = "A decorator on a class view does not apply to its methods; list it in `decorators = [...]`.";
            note(&mut self.out.diags, &p.files[f].rel, line_at, message);
        }
        let mut eff = Eff::default();
        if let Some((at, text)) = class_attr(vf, d, "decorators") {
            match vf.items(at, text) {
                Some(items) => (items.iter()).for_each(|(a, t)| {
                    eff.extend(p.decorator(lf, *a, a + t.len()));
                }),
                None => eff.extend(Eff::unknown(
                    text,
                    "class `decorators` is not a literal list",
                )),
            }
        }
        let mut items: Vec<Item> = (vf.defs.iter())
            .filter(|m| m.parent == Some(di) && VERBS.contains(&m.name.as_str()))
            .map(|m| {
                let mut e = eff.clone();
                e.extend(self.applied(lf, m, 0, &[]));
                let handler = format!("{module}.{ln}.{}", m.name);
                Item {
                    method: m.name.to_uppercase(),
                    eff: e,
                    handler,
                }
            })
            .collect();
        if items.is_empty() {
            let verbs: Vec<String> = match class_attr(vf, d, "methods") {
                Some((at, text)) => upper(vf.strings(at, text)?),
                None => vec!["GET".to_string()],
            };
            let handler = format!("{module}.{ln}.dispatch_request");
            items = (verbs.into_iter())
                .map(|method| Item {
                    method,
                    eff: eff.clone(),
                    handler: handler.clone(),
                })
                .collect();
        }
        Some(items)
    }
}

fn upper(v: Vec<String>) -> Vec<String> {
    v.iter().map(|s| s.to_uppercase()).collect()
}

fn prefix_arg(args: &[Arg<'_>]) -> Option<Result<String, String>> {
    let a = args.iter().find(|a| a.name == Some("url_prefix"))?;
    Some(py::string_value(a.value).ok_or_else(|| a.value.to_string()))
}

/// The path and methods of a route decorator, or why it is skipped.
fn route_target(fi: &FileInfo, r: &Registered<'_>) -> Result<(String, Vec<String>), String> {
    let first = (r.args.iter()).find(|a| a.name.is_none() || a.name == Some("rule"));
    let Some(path) = first.and_then(|a| py::string_value(a.value)) else {
        let text = first.map_or("", |a| a.value);
        return Err(format!(
            "Route path `{text}` is not a readable literal; this endpoint is skipped."
        ));
    };
    let methods = match (
        r.verb.as_str(),
        r.args.iter().find(|a| a.name == Some("methods")),
    ) {
        ("route", Some(m)) => fi.strings(fi.vat(m), m.value).map(upper),
        ("route", None) => Some(vec!["GET".to_string()]),
        (v, _) => Some(vec![v.to_uppercase()]),
    };
    let message = "Route `methods=` is not a literal list of strings; this endpoint is skipped.";
    Ok((path, methods.ok_or(message.to_string())?))
}

/// The value of the class attribute `name` of class `d`.
fn class_attr<'f>(fi: &'f FileInfo, d: &Def, name: &str) -> Option<(usize, &'f str)> {
    (fi.stmts.iter())
        .filter(|s| s.at >= d.body.0 && s.end <= d.body.1)
        .find_map(|s| {
            let a = py::assignment(&fi.src, &fi.masked, s)?;
            (a.target == name).then_some((a.value_at, a.value))
        })
}

impl Project<'_> {
    /// The blueprint a dotted name of file `f` is.
    fn local_bp(&self, f: usize, name: &str) -> Option<Key> {
        match self.qualify(f, name) {
            (Q::Local(lf, ln), attrs) if attrs.is_empty() => {
                matches!(self.obj(lf, &ln, 0), Some(Obj::Bp(_))).then_some((lf, ln))
            }
            _ => None,
        }
    }

    /// The function or class (file, index in its `defs`) a dotted name is.
    fn local_def(&self, f: usize, name: &str) -> Option<(usize, usize)> {
        let (Q::Local(lf, ln), attrs) = self.qualify(f, name) else {
            return None;
        };
        let di = self.files[lf].defs.iter().position(|d| d.name == ln)?;
        attrs.is_empty().then_some((lf, di))
    }

    /// Apps, and blueprints no readable `register_blueprint` registers.
    fn roots(&self, scan: &Scan, diags: &mut Vec<UiFlowDiagnostic>) -> Vec<Key> {
        let (mut roots, mut bps) = (Vec::new(), Vec::new());
        for (f, fi) in self.files.iter().enumerate() {
            let mut names: Vec<&String> = fi.assigns.keys().collect();
            names.sort();
            for name in names {
                match self.obj(f, name, 0) {
                    Some(Obj::App) => roots.push((f, name.clone())),
                    Some(Obj::Bp(_)) => bps.push((f, name.clone())),
                    _ => {}
                }
            }
        }
        let registered: BTreeSet<&Key> = scan.regs.iter().map(|r| &r.child).collect();
        for key in bps.into_iter().filter(|k| !registered.contains(k)) {
            if let Some(r) = scan.routes.iter().find(|r| r.owner == key) {
                let message = format!(
                    "Blueprint `{}` is registered by no readable `register_blueprint`; its paths start at its own prefix.",
                    key.1
                );
                gap(diags, &self.files[r.file].rel, r.line, message);
            }
            roots.push(key);
        }
        roots
    }

    /// The own prefix of a root; `None` (with a gap) when unreadable.
    fn root_prefix(&self, root: &Key, diags: &mut Vec<UiFlowDiagnostic>) -> Option<String> {
        match self.obj(root.0, &root.1, 0) {
            Some(Obj::Bp(Some(Ok(p)))) => Some(p),
            Some(Obj::Bp(Some(Err(t)))) => {
                let message = format!(
                    "Blueprint `{}` has `url_prefix={t}`, which is not a readable literal; its routes are skipped.",
                    root.1
                );
                gap(diags, &self.files[root.0].rel, 0, message);
                None
            }
            _ => Some(String::new()),
        }
    }

    fn emit(&self, flow: &mut UiFlow) {
        let scan = Scanner::run(self);
        flow.diagnostics.extend(scan.diags.iter().cloned());
        for root in self.roots(&scan, &mut flow.diagnostics) {
            let Some(start) = self.root_prefix(&root, &mut flow.diagnostics) else {
                continue;
            };
            let mut places = Vec::new();
            let diags = &mut flow.diagnostics;
            self.walk(&scan, &mut vec![root], &start, &mut places, diags);
            self.emit_places(&scan, &places, flow);
        }
    }

    /// The routes of every place of one app tree, with the hooks that guard them.
    fn emit_places(&self, scan: &Scan, places: &[Place], flow: &mut UiFlow) {
        let app_wide: Vec<&Hook> = (scan.hooks.iter())
            .filter(|h| h.app_wide && places.iter().any(|p| p.chain.contains(&h.owner)))
            .collect();
        for place in places {
            let hooks: Vec<&Hook> = (scan.hooks.iter())
                .filter(|h| !h.app_wide && place.chain.contains(&h.owner))
                .chain(app_wide.iter().copied())
                .collect();
            let Some(last) = place.chain.last() else {
                continue;
            };
            for route in scan.routes.iter().filter(|r| &r.owner == last) {
                self.endpoints(route, place, &hooks, flow);
            }
        }
    }

    fn walk(
        &self,
        scan: &Scan,
        chain: &mut Vec<Key>,
        prefix: &str,
        out: &mut Vec<Place>,
        diags: &mut Vec<UiFlowDiagnostic>,
    ) {
        out.push(Place {
            chain: chain.clone(),
            prefix: prefix.to_string(),
        });
        let Some(here) = chain.last().cloned() else {
            return;
        };
        for reg in scan.regs.iter().filter(|r| r.parent == here) {
            if chain.contains(&reg.child) {
                continue;
            }
            let own = match &reg.prefix {
                Some(p) => Some(p.clone()),
                None => match self.obj(reg.child.0, &reg.child.1, 0) {
                    Some(Obj::Bp(p)) => p,
                    _ => None,
                },
            };
            let child_prefix = match own {
                Some(Ok(p)) => join_prefix(prefix, &p),
                Some(Err(text)) => {
                    let message = format!(
                        "`url_prefix={text}` is not a readable literal; the routes of blueprint `{}` are skipped.",
                        reg.child.1
                    );
                    gap(diags, &self.files[reg.file].rel, reg.line, message);
                    continue;
                }
                None => prefix.to_string(),
            };
            chain.push(reg.child.clone());
            self.walk(scan, chain, &child_prefix, out, diags);
            chain.pop();
        }
    }

    fn endpoints(&self, route: &Route, place: &Place, hooks: &[&Hook], flow: &mut UiFlow) {
        let fi = &self.files[route.file];
        let mut path = join_prefix(&place.prefix, &convert(&route.path));
        if !path.starts_with('/') {
            path.insert(0, '/');
        }
        for item in &route.items {
            let mut eff = item.eff.clone();
            hooks.iter().for_each(|h| eff.extend(h.eff.clone()));
            for message in &eff.gaps {
                gap(&mut flow.diagnostics, &fi.rel, route.line, message.clone());
            }
            flow.endpoints.push(EndpointAccess {
                method: item.method.clone(),
                path: path.clone(),
                access: eff.state(),
                application: fi.application.clone(),
                file: fi.rel.clone(),
                line: route.line,
                source: "flask".into(),
                handler: Some(item.handler.clone()),
                sends: Vec::new(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{AccessState, Condition, Config, DiagnosticSeverity};

    fn run(files: &[(&str, &str)], access: &str) -> UiFlow {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in files {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let config: Config = toml::from_str(&format!("[views.access]\n{access}")).unwrap();
        extract_with(&ExtractContext::new(dir.path(), &config))
    }

    /// One `app.py` with a `bp` at `/m` and the given body.
    fn one(body: &str, access: &str) -> UiFlow {
        let head = "from flask import Flask, Blueprint\nfrom flask_login import login_required\n\
            app = Flask(__name__)\nbp = Blueprint('m', __name__)\n\
            app.register_blueprint(bp, url_prefix='/m')\n";
        run(&[("app.py", &format!("{head}{body}"))], access)
    }

    fn eps(flow: &UiFlow) -> Vec<(String, String, AccessState)> {
        (flow.endpoints.iter())
            .map(|e| (e.method.clone(), e.path.clone(), e.access.clone()))
            .collect()
    }

    fn prot(c: Condition) -> AccessState {
        AccessState::Protected(c)
    }
    fn role(n: &str) -> Condition {
        Condition::Role(n.into())
    }
    fn policy(n: &str) -> Condition {
        Condition::Policy(n.into())
    }
    fn unknown(n: &str) -> Condition {
        Condition::Unknown(n.into())
    }
    fn get(path: &str, access: AccessState) -> (String, String, AccessState) {
        ("GET".into(), path.into(), access)
    }
    fn severities(flow: &UiFlow) -> Vec<DiagnosticSeverity> {
        flow.diagnostics.iter().map(|d| d.severity).collect()
    }
    fn no_gap(flow: &UiFlow) -> bool {
        !severities(flow).contains(&DiagnosticSeverity::Gap)
    }

    #[test]
    fn py46_1_login_required_below_route_with_registered_prefix() {
        let flow = one(
            "@bp.route('/a/<int:id>')\n@login_required\ndef a(id):\n    pass\n",
            "",
        );
        assert_eq!(
            eps(&flow),
            [get("/m/a/:id", prot(Condition::Authenticated))]
        );
        assert_eq!(flow.endpoints[0].handler.as_deref(), Some("app.a"));
        assert_eq!(flow.endpoints[0].source, "flask");
        assert!(flow.complete);
    }

    #[test]
    fn py46_2_login_required_above_route_protects_nothing() {
        let flow = one("@login_required\n@bp.route('/b')\ndef b():\n    pass\n", "");
        assert_eq!(eps(&flow), [get("/m/b", AccessState::Undeclared)]);
        assert_eq!(severities(&flow), [DiagnosticSeverity::Note]);
    }

    #[test]
    fn py46_3_before_request_with_guard_decorator_covers_its_blueprint_only() {
        let flow = one(
            "other = Blueprint('o', __name__)\napp.register_blueprint(other, url_prefix='/o')\n\
             @bp.before_request\n@login_required\ndef guard():\n    pass\n\
             @bp.route('/c')\ndef c():\n    pass\n@other.route('/d')\ndef d():\n    pass\n",
            "",
        );
        assert_eq!(
            eps(&flow),
            [
                get("/m/c", prot(Condition::Authenticated)),
                get("/o/d", AccessState::Undeclared)
            ]
        );
    }

    #[test]
    fn py46_4_body_only_before_app_request_is_no_guard() {
        let flow = one(
            "@bp.before_app_request\ndef touch():\n    if current_user.is_authenticated:\n        \
             current_user.last_seen = now()\n@bp.route('/e')\ndef e():\n    pass\n",
            "",
        );
        assert_eq!(eps(&flow), [get("/m/e", AccessState::Undeclared)]);
        assert!(no_gap(&flow));
    }

    #[test]
    fn py46_5_app_gate_is_unknown_only_when_listed() {
        let body = "@app.before_request\ndef gate():\n    if request.endpoint not in PUBLIC and \
                    not current_user.is_authenticated:\n        return redirect('/l')\n\
                    @bp.route('/f')\ndef f():\n    pass\n";
        let flow = one(body, "");
        assert_eq!(eps(&flow), [get("/m/f", AccessState::Undeclared)]);
        assert_eq!(severities(&flow), [DiagnosticSeverity::Note]);
        let flow = one(body, "gate = \"authenticated\"\n");
        assert_eq!(eps(&flow), [get("/m/f", prot(unknown("gate")))]);
        assert!(!no_gap(&flow));
    }

    #[test]
    fn py46_6_and_7_principal_permissions() {
        let flow = run(
            &[(
                "app.py",
                "from flask import Flask\nfrom flask_principal import Permission, RoleNeed, ActionNeed\n\
                 app = Flask(__name__)\nadmin = Permission(RoleNeed('admin'), RoleNeed('root'))\n\
                 one = Permission(ActionNeed('x'))\nfused = admin.union(one)\n\
                 @app.route('/a')\n@admin.require(http_exception=403)\ndef a():\n    pass\n\
                 @app.route('/b')\n@one.require()\ndef b():\n    pass\n\
                 @app.route('/c')\n@fused.require()\ndef c():\n    pass\n",
            )],
            "",
        );
        assert_eq!(
            eps(&flow),
            [
                get(
                    "/a",
                    prot(Condition::Any(vec![role("admin"), role("root")]))
                ),
                get("/b", prot(policy("x"))),
                get("/c", prot(unknown("fused"))),
            ]
        );
        assert_eq!(severities(&flow), [DiagnosticSeverity::Gap]);
    }

    #[test]
    fn py46_8_and_9_security_decorators() {
        let flow = run(
            &[(
                "app.py",
                "from flask import Flask\nfrom flask_security import auth_required, roles_required, \
                 roles_accepted, permissions_accepted, anonymous_user_required\napp = Flask(__name__)\n\
                 @app.route('/a')\n@auth_required()\n@roles_required('admin', 'editor')\ndef a():\n    pass\n\
                 @app.route('/b')\n@roles_accepted('a', 'b')\ndef b():\n    pass\n\
                 @app.route('/c')\n@permissions_accepted('x-read', 'x-write')\ndef c():\n    pass\n\
                 @app.route('/d')\n@anonymous_user_required\ndef d():\n    pass\n\
                 @app.route('/e')\n@roles_required(names)\ndef e():\n    pass\n",
            )],
            "",
        );
        assert_eq!(
            eps(&flow),
            [
                get(
                    "/a",
                    prot(Condition::All(vec![
                        Condition::Authenticated,
                        role("admin"),
                        role("editor")
                    ]))
                ),
                get("/b", prot(Condition::Any(vec![role("a"), role("b")]))),
                get(
                    "/c",
                    prot(Condition::Any(vec![policy("x-read"), policy("x-write")]))
                ),
                get(
                    "/d",
                    prot(Condition::Not(Box::new(Condition::Authenticated)))
                ),
                get("/e", prot(unknown("roles_required(names)"))),
            ]
        );
    }

    #[test]
    fn py46_10_jwt_required_variants() {
        let flow = run(
            &[(
                "app.py",
                "from flask import Flask\nfrom flask_jwt_extended import jwt_required\n\
                 app = Flask(__name__)\n\
                 @app.route('/a')\n@jwt_required()\ndef a():\n    pass\n\
                 @app.route('/b')\n@jwt_required(optional=True)\ndef b():\n    pass\n\
                 @app.route('/c')\n@jwt_required(optional=flag)\ndef c():\n    pass\n\
                 @app.route('/d')\n@jwt_required(refresh=True)\ndef d():\n    pass\n\
                 @app.route('/e')\n@jwt_required(optional=True)\n@mystery\ndef e():\n    pass\n",
            )],
            "",
        );
        assert_eq!(
            eps(&flow),
            [
                get("/a", prot(Condition::Authenticated)),
                get("/b", AccessState::Open),
                get("/c", prot(unknown("jwt_required(optional=flag)"))),
                get("/d", prot(Condition::Custom("refresh_token".into()))),
                get("/e", prot(unknown("mystery"))),
            ]
        );
        assert_eq!(
            severities(&flow),
            [DiagnosticSeverity::Gap, DiagnosticSeverity::Gap]
        );
    }

    #[test]
    fn py46_11_method_view_decorators_not_class_decorators() {
        let flow = run(
            &[(
                "app.py",
                "from flask import Flask\nfrom flask.views import MethodView\n\
                 from flask_login import login_required\napp = Flask(__name__)\n\
                 class U(MethodView):\n    decorators = [login_required]\n    def get(self):\n        pass\n    \
                 def delete(self):\n        pass\n\
                 @login_required\nclass V(MethodView):\n    def get(self):\n        pass\n\
                 app.add_url_rule('/u', view_func=U.as_view('u'))\n\
                 app.add_url_rule('/v', view_func=V.as_view('v'))\n",
            )],
            "",
        );
        let auth = prot(Condition::Authenticated);
        assert_eq!(
            eps(&flow),
            [
                ("DELETE".into(), "/u".into(), auth.clone()),
                get("/u", auth),
                get("/v", AccessState::Undeclared),
            ]
        );
    }

    #[test]
    fn py46_12_httpauth_roles_and_optional() {
        let flow = run(
            &[
                (
                    "app/auth.py",
                    "from flask_httpauth import HTTPBasicAuth, HTTPTokenAuth\n\
                     basic_auth = HTTPBasicAuth()\ntoken_auth = HTTPTokenAuth()\n",
                ),
                (
                    "app/__init__.py",
                    "from flask import Flask\nfrom app.auth import token_auth, basic_auth\n\
                     app = Flask(__name__)\n\
                     @app.route('/a')\n@token_auth.login_required(role=['admin', 'ops'])\ndef a():\n    pass\n\
                     @app.route('/b')\n@basic_auth.login_required\ndef b():\n    pass\n\
                     @app.route('/c')\n@token_auth.login_required(optional=True)\ndef c():\n    pass\n",
                ),
            ],
            "",
        );
        assert_eq!(
            eps(&flow),
            [
                get(
                    "/a",
                    prot(Condition::All(vec![
                        Condition::Any(vec![role("admin"), role("ops")]),
                        Condition::Authenticated
                    ]))
                ),
                get("/b", prot(Condition::Authenticated)),
                get("/c", AccessState::Open),
            ]
        );
    }

    #[test]
    fn py46_14_login_disabled_is_a_gap_never_open() {
        let flow = run(
            &[(
                "app.py",
                "from flask import Flask\nfrom flask_login import login_required\napp = Flask(__name__)\n\
                 class Testing:\n    LOGIN_DISABLED = True\n\
                 @app.route('/a')\n@login_required\ndef a():\n    pass\n",
            )],
            "",
        );
        assert_eq!(eps(&flow), [get("/a", prot(Condition::Authenticated))]);
        assert!(!no_gap(&flow));
    }

    #[test]
    fn py46_15_nested_blueprint_inherits_the_parent_guard() {
        let flow = run(
            &[(
                "app.py",
                "from flask import Flask, Blueprint\nfrom flask_login import login_required\n\
                 app = Flask(__name__)\nparent = Blueprint('p', __name__, url_prefix='/p')\n\
                 child = Blueprint('c', __name__, url_prefix='/c')\n\
                 parent.register_blueprint(child)\napp.register_blueprint(parent)\n\
                 @parent.before_request\n@login_required\ndef g():\n    pass\n\
                 @child.route('/x')\ndef x():\n    pass\n",
            )],
            "",
        );
        assert_eq!(eps(&flow), [get("/p/c/x", prot(Condition::Authenticated))]);
    }

    #[test]
    fn unknown_decorator_is_protected_unknown_and_listed_names_use_the_catalog() {
        let flow = one(
            "@bp.route('/a')\n@cache.cached()\ndef a():\n    pass\n\
             @bp.route('/b')\n@staff_only\ndef b():\n    pass\n\
             @bp.route('/c')\n@public_view\ndef c():\n    pass\n",
            "staff_only = \"role:Staff\"\npublic_view = \"open\"\n",
        );
        assert_eq!(
            eps(&flow),
            [
                get("/m/a", prot(unknown("cache.cached"))),
                get("/m/b", prot(role("Staff"))),
                get("/m/c", AccessState::Open),
            ]
        );
        assert_eq!(severities(&flow), [DiagnosticSeverity::Gap]);
    }

    #[test]
    fn unreadable_path_methods_and_prefix_are_gaps_and_skipped() {
        let flow = one(
            "@bp.route(PATH)\ndef a():\n    pass\n@bp.route('/b', methods=M)\ndef b():\n    pass\n\
             @bp.post('/c')\ndef c():\n    pass\n@bp.route('/d', methods=['GET', 'POST'])\ndef d():\n    pass\n",
            "",
        );
        let got = eps(&flow);
        assert_eq!(got.len(), 3, "{got:?}");
        assert_eq!(severities(&flow), [DiagnosticSeverity::Gap; 2]);
    }

    #[test]
    fn no_flask_gives_nothing() {
        let flow = run(&[("a.py", "def f():\n    pass\n")], "");
        assert!(flow.endpoints.is_empty() && flow.diagnostics.is_empty());
    }
}
