//! Flask endpoints and their access for ui-flow/v1 (ADR 0021).
//!
//! A text-level scan of the `.py` files below the root (test files left out)
//! on the shared Python helpers of `lang_python`; only when some file creates
//! a `Flask` app or a `Blueprint`. It fills `endpoints` and `diagnostics`.
//! Screens and links: a GET route whose handler calls `render_template('x')`
//! with a literal is a screen; the Jinja template (`templates/**`, with its
//! `{% include %}`s and `{% extends %}` layouts) gives its `<a href>` and
//! `<form action>` with a literal path or `url_for('bp.view')` as transitions
//! and actions. `{% if %}` / `{% elif %}` / `{% else %}` around a link read
//! `current_user.is_authenticated`, `is_anonymous`, `has_role('x')` and
//! `has_permission('x')` (with `and`/`or`/`not`); any other user check is
//! `unknown` with a gap, a condition without one gives no guard. `{% for %}`
//! `else` is not a branch. Not read: `url_for('.x')`, `template_folder`
//! lookup order (an ambiguous template name is a gap), `Permission.can()`.
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
use archguard_core::{
    AccessMeaning, AccessState, Action, Call, Condition, DiagnosticSeverity, EndpointAccess,
    Origin, Screen, ScreenKind, Target, Transition, UiFlow, UiFlowDiagnostic,
};
use archguard_flow_common::extract_ctx::all;
use archguard_flow_common::extract_ctx::{
    ExtractContext, ancestors, cap, dir_name, finish, gap, read_text, walk,
};
use archguard_flow_common::lang_python::{self as py, Arg, Def, DefKind, Import, Stmt};
use archguard_flow_common::lang_text::line;
use archguard_flow_common::support::test_path;
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

/// `crate::ui_flow::extract_all`'s entry: endpoints of every Flask app below the root.
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
    let all_files = walked.files.clone();
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
    let templates = jinja::read_templates(ctx, &all_files, &mut flow.diagnostics);
    let set = files.iter().map(|f| f.rel.clone()).collect();
    let index = files
        .iter()
        .enumerate()
        .map(|(i, f)| (f.rel.clone(), i))
        .collect();
    let login_unread = login_unread(&files);
    let project = Project {
        login_unread,
        templates,
        files,
        set,
        index,
        access: &ctx.config.views.access,
    };
    project.emit(&mut flow);
    finish(&mut flow);
    flow
}

/// Some `LOGIN_DISABLED` is set to something that is not a literal.
fn login_unread(files: &[FileInfo]) -> bool {
    let unread = |f: &FileInfo| f.login_settings().iter().any(|(_, v)| v.is_none());
    files.iter().any(unread)
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

    /// Each statement that names `LOGIN_DISABLED`: its line and the literal
    /// it is set to (`None`: set to or read as something not a literal).
    fn login_settings(&self) -> Vec<(u32, Option<bool>)> {
        let mut out = vec![];
        for s in &self.stmts {
            let text = &self.src[s.at..s.end];
            let Some(k) = text.find("LOGIN_DISABLED") else {
                continue;
            };
            let head = text[..k].trim_end_matches(['\'', '"']);
            let direct = head.trim().is_empty() || head.ends_with('[');
            let rest = text[k + "LOGIN_DISABLED".len()..]
                .trim_start_matches(['\'', '"', ']'])
                .trim_start();
            let value = rest
                .strip_prefix('=')
                .filter(|v| direct && !v.starts_with('='))
                .and_then(|v| bool_lit(v.trim()));
            out.push((self.line(s.at), value));
        }
        out
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
    /// `LOGIN_DISABLED` is set to something that is not a literal.
    login_unread: bool,
    /// Template name (below a `templates/` directory) → (file, text).
    templates: BTreeMap<String, Vec<(String, String)>>,
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
    /// A library decorator's effect; Flask-Login's is unknown while
    /// `LOGIN_DISABLED` is not a literal.
    fn external(&self, full: &str, args: &Option<Vec<Arg<'_>>>, shown: &str) -> Option<Eff> {
        let (root, last) = root_last(full);
        let login =
            root == "flask_login" && matches!(last, "login_required" | "fresh_login_required");
        match login && self.login_unread {
            true => Some(Eff::unknown(last, "LOGIN_DISABLED is not a literal")),
            false => library(full, args, shown),
        }
    }

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
            (Q::Ext(full), []) => self
                .external(&full, &args, &shown(&args))
                .unwrap_or_else(by_name),
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
    /// The function name, empty when `endpoint=` renames it or there is none.
    func: String,
    /// Literal `render_template(…)` names in the handler.
    views: Vec<String>,
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
        for (line, value) in fi.login_settings() {
            let message = match value {
                Some(false) => continue,
                Some(true) => {
                    "LOGIN_DISABLED is True: login checks are switched off by configuration; protection by login decorators is unknown there."
                }
                None => {
                    "LOGIN_DISABLED is not a literal True/False; whether login checks are switched off is unknown, so login decorators give an unknown access."
                }
            };
            gap(&mut self.out.diags, &fi.rel, line, message);
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
            "register_blueprint" | "add_url_rule" | "before_request" | "before_app_request"
        ) {
            return None;
        }
        let (owner, _) = p.owner(f, receiver)?;
        let (args, _) = py::call_args(&fi.src, &fi.masked, open)?;
        let at = fi.line(s.at);
        match verb {
            "register_blueprint" => self.register(f, owner, &args, at),
            "add_url_rule" => self.add_url_rule(f, owner, &args, at),
            _ => self.hook_call(f, owner, &args, verb == "before_app_request"),
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

    /// `app.before_request(f)` / `bp.before_app_request(f)`: every decorator of `f` is applied.
    fn hook_call(&mut self, f: usize, owner: Key, args: &[Arg<'_>], wide: bool) {
        let p = self.p;
        let first = args.iter().find(|a| a.name.is_none());
        let Some((lf, di)) = first.and_then(|a| p.local_def(f, a.value)) else {
            return;
        };
        let d = &p.files[lf].defs[di];
        if d.kind == DefKind::Function {
            let eff = self.applied(lf, d, 0, &[]);
            self.push_hook(lf, di, owner, wide, eff);
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
                let renamed = r.args.iter().any(|a| a.name == Some("endpoint"));
                let route = Route {
                    owner,
                    file: f,
                    line: line_at,
                    path,
                    items,
                    func: if renamed {
                        String::new()
                    } else {
                        d.name.clone()
                    },
                    views: self.render_templates(f, d),
                };
                self.out.routes.push(route);
            }
            Err(message) => gap(&mut self.out.diags, &fi.rel, line_at, message),
        }
    }

    /// The literal names of the `render_template(…)` calls in the body of `d`.
    fn render_templates(&mut self, f: usize, d: &Def) -> Vec<String> {
        let fi = &self.p.files[f];
        let body = &fi.masked[d.body.0..d.body.1];
        let (mut out, mut from) = (vec![], 0);
        while let Some(k) = body[from..].find("render_template(") {
            let at = d.body.0 + from + k;
            from += k + 1;
            if at > 0
                && (fi.masked.as_bytes()[at - 1].is_ascii_alphanumeric()
                    || fi.masked.as_bytes()[at - 1] == b'_')
            {
                continue;
            }
            let open = at + "render_template".len();
            let name = py::call_args(&fi.src, &fi.masked, open).and_then(|(args, _)| {
                let a = args.into_iter().find(|a| a.name.is_none())?;
                py::string_value(a.value)
            });
            match name {
                Some(n) => out.push(n),
                None => {
                    let message =
                        "`render_template` has no literal template name; its links are unknown.";
                    gap(&mut self.out.diags, &fi.rel, fi.line(at), message);
                }
            }
        }
        out
    }

    fn add_url_rule(&mut self, f: usize, owner: Key, args: &[Arg<'_>], line_at: u32) {
        let p = self.p;
        let fi = &p.files[f];
        let pos: Vec<&Arg<'_>> = args.iter().filter(|a| a.name.is_none()).collect();
        let named = |k: &str| args.iter().find(|a| a.name == Some(k));
        let path = (pos.first().copied().or(named("rule"))).and_then(|a| py::string_value(a.value));
        let view = named("view_func").or(pos.get(2).copied());
        let (Some(path), Some(view)) = (path, view) else {
            let message = "`add_url_rule` has no literal rule string or no view function argument; this endpoint is skipped.";
            return gap(&mut self.out.diags, &fi.rel, line_at, message);
        };
        match self.view_items(f, view, named("methods"), line_at) {
            Some(items) => self.out.routes.push(Route {
                owner,
                file: f,
                line: line_at,
                path,
                items,
                func: String::new(),
                views: vec![],
            }),
            None => {
                let message = match named("methods") {
                    Some(m) if p.local_def(f, view.value).is_some() => format!(
                        "`add_url_rule` `methods={}` is not a literal list of strings; this endpoint is skipped.",
                        m.value
                    ),
                    _ => format!(
                        "`add_url_rule` view `{}` is no function or `as_view` class of the project; this endpoint is skipped.",
                        view.value
                    ),
                };
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
        let mut acc = Acc::default();
        flow.diagnostics.extend(scan.diags.iter().cloned());
        for root in self.roots(&scan, &mut flow.diagnostics) {
            let Some(start) = self.root_prefix(&root, &mut flow.diagnostics) else {
                continue;
            };
            let mut places = Vec::new();
            let diags = &mut flow.diagnostics;
            self.walk(&scan, &mut vec![root], &start, &mut places, diags);
            self.emit_places(&scan, &places, &mut acc, flow);
        }
        self.emit_templates(&acc, flow);
    }

    /// The name of blueprint `key`, when it is a literal.
    fn bp_name(&self, key: &Key) -> Option<String> {
        let fi = &self.files[key.0];
        let &(a, _) = fi.assigns.get(&key.1)?;
        let (_, open) = fi.call_at(a)?;
        let (args, _) = py::call_args(&fi.src, &fi.masked, open)?;
        py::string_value(args.iter().find(|a| a.name.is_none())?.value)
    }

    /// The routes of every place of one app tree, with the hooks that guard them.
    fn emit_places(&self, scan: &Scan, places: &[Place], acc: &mut Acc, flow: &mut UiFlow) {
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
                self.endpoints(route, place, &hooks, acc, flow);
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

    fn endpoints(
        &self,
        route: &Route,
        place: &Place,
        hooks: &[&Hook],
        acc: &mut Acc,
        flow: &mut UiFlow,
    ) {
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
            if !route.func.is_empty() {
                let mut parts: Vec<String> =
                    place.chain.iter().filter_map(|k| self.bp_name(k)).collect();
                parts.push(route.func.clone());
                let key = (fi.application.clone(), parts.join("."));
                acc.names
                    .entry(key)
                    .or_default()
                    .push((item.method.clone(), path.clone()));
            }
            if item.method == "GET" && !route.views.is_empty() {
                let id = format!(
                    "{}:{}",
                    fi.application.as_deref().unwrap_or(""),
                    item.handler
                );
                flow.screens.push(Screen {
                    weak: false,
                    id: id.clone(),
                    name: item
                        .handler
                        .rsplit('.')
                        .next()
                        .unwrap_or(&item.handler)
                        .to_string(),
                    kind: ScreenKind::Page,
                    route: Some(path.clone()),
                    application: fi.application.clone(),
                    file: fi.rel.clone(),
                    line: route.line,
                    access: eff.state(),
                    source: "flask".into(),
                });
                acc.screens.push(ScreenViews {
                    id,
                    application: fi.application.clone(),
                    route: path.clone(),
                    file: fi.rel.clone(),
                    line: route.line,
                    views: route.views.clone(),
                });
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

/// (application, endpoint name).
type EndpointKey = (Option<String>, String);

/// What the routes give the template reader.
#[derive(Default)]
struct Acc {
    /// (application, endpoint name `bp.func`) → (method, path).
    names: BTreeMap<EndpointKey, Vec<(String, String)>>,
    screens: Vec<ScreenViews>,
}

struct ScreenViews {
    id: String,
    application: Option<String>,
    route: String,
    file: String,
    line: u32,
    views: Vec<String>,
}

/// Jinja templates: links, includes, layouts and the conditions around them.
mod jinja {
    use super::*;
    use archguard_flow_common::lang_markup::{Tag, all_tags, attributes, blank};

    /// The Jinja templates by the name `render_template` uses; unreadable ones are gaps.
    pub(super) fn read_templates(
        ctx: &ExtractContext,
        files: &[String],
        diags: &mut Vec<UiFlowDiagnostic>,
    ) -> BTreeMap<String, Vec<(String, String)>> {
        let mut templates: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
        let mut paths: Vec<&String> = files.iter().filter(|r| is_template(r)).collect();
        paths.sort();
        for rel in paths.into_iter().take(MAX_FILES) {
            match read_text(ctx.root, rel, 1 << 20) {
                Ok(text) => {
                    let name = template_name(rel).unwrap_or_default().to_string();
                    templates.entry(name).or_default().push((rel.clone(), text));
                }
                Err(e) => {
                    let message = format!("Template not read ({e}); its links are unknown.");
                    gap(diags, rel, 0, message);
                }
            }
        }
        templates
    }

    fn is_template(rel: &str) -> bool {
        template_name(rel).is_some() && !test_path(rel)
    }

    /// The name `render_template` uses for template file `rel`.
    fn template_name(rel: &str) -> Option<&str> {
        let ext = rel.rsplit_once('.')?.1;
        if !["html", "htm", "jinja", "jinja2", "j2"].contains(&ext) {
            return None;
        }
        rel.strip_prefix("templates/")
            .or_else(|| rel.split_once("/templates/").map(|x| x.1))
    }

    #[derive(Clone)]
    pub(super) enum Dest {
        Endpoint(String),
        Path(String),
        Unread,
    }

    #[derive(Clone)]
    pub(super) struct Link {
        pub(super) line: u32,
        pub(super) cond: Option<Condition>,
        /// Upper-case form method; none for `<a>`.
        pub(super) method: Option<String>,
        pub(super) dest: Dest,
        pub(super) source: String,
    }

    #[derive(Clone, Default)]
    pub(super) struct Tpl {
        pub(super) extends: Vec<(String, u32)>,
        pub(super) includes: Vec<(String, Option<Condition>, u32)>,
        pub(super) links: Vec<Link>,
    }

    /// An open `{% if %}`: earlier branches, the own condition, what holds.
    struct Frame {
        seen: Vec<Condition>,
        own: Option<Condition>,
        current: Option<Condition>,
    }

    fn and2(a: Option<Condition>, b: Option<Condition>) -> Option<Condition> {
        match (a, b) {
            (Some(a), Some(b)) => Some(all(vec![a, b])),
            (a, b) => a.or(b),
        }
    }

    fn not(c: Condition) -> Condition {
        match c {
            Condition::Not(inner) => *inner,
            c => Condition::Not(Box::new(c)),
        }
    }

    fn collapse(s: &str) -> String {
        s.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    fn now(stack: &[Option<Frame>]) -> Option<Condition> {
        let cs: Vec<Condition> = (stack.iter().flatten())
            .filter_map(|f| f.current.clone())
            .collect();
        (!cs.is_empty()).then(|| all(cs))
    }

    fn branch(f: &mut Frame, own: Option<Condition>) {
        f.seen.extend(f.own.take());
        let negated: Vec<Condition> = f.seen.iter().cloned().map(not).collect();
        f.current = and2((!negated.is_empty()).then(|| all(negated)), own.clone());
        f.own = own;
    }

    /// `text` split at top-level `word` (outside parentheses and quotes).
    fn split_top<'t>(text: &'t str, word: &str) -> Vec<&'t str> {
        let (b, mut depth, mut quote) = (text.as_bytes(), 0i32, None);
        let (mut out, mut last, mut i) = (vec![], 0, 0);
        while i < b.len() {
            match (quote, b[i]) {
                (Some(q), c) if c == q => quote = None,
                (Some(_), _) => {}
                (None, c @ (b'\'' | b'"')) => quote = Some(c),
                (None, b'(' | b'[') => depth += 1,
                (None, b')' | b']') => depth -= 1,
                (None, _) if depth == 0 && b[i..].starts_with(word.as_bytes()) => {
                    out.push(&text[last..i]);
                    i += word.len();
                    last = i;
                    continue;
                }
                _ => {}
            }
            i += 1;
        }
        out.push(&text[last..]);
        out
    }

    fn unparen(e: &str) -> &str {
        let e = e.trim();
        match e.strip_prefix('(').and_then(|r| r.strip_suffix(')')) {
            Some(inner) if balanced(inner) => unparen(inner),
            _ => e,
        }
    }

    fn balanced(e: &str) -> bool {
        let mut depth = 0i32;
        for c in e.chars() {
            depth += i32::from(c == '(') - i32::from(c == ')');
            if depth < 0 {
                return false;
            }
        }
        depth == 0
    }

    fn atom(e: &str, unread: &mut Vec<String>) -> Option<Condition> {
        match e {
            "current_user.is_authenticated" => return Some(Condition::Authenticated),
            "current_user.is_anonymous" => return Some(not(Condition::Authenticated)),
            _ => {}
        }
        type Make = fn(String) -> Condition;
        let makers: [(&str, Make); 2] = [
            ("current_user.has_role(", Condition::Role),
            ("current_user.has_permission(", Condition::Policy),
        ];
        for (head, make) in makers {
            if let Some(arg) = e.strip_prefix(head).and_then(|r| r.strip_suffix(')'))
                && let Some(v) = py::string_value(arg.trim())
            {
                return Some(make(v));
            }
        }
        const USER: [&str; 10] = [
            "current_user",
            ".can(",
            "is_authenticated",
            "is_anonymous",
            "has_role",
            "has_permission",
            "is_admin",
            "g.user",
            "session[",
            "session.get(",
        ];
        (e == "user" || USER.iter().any(|k| e.contains(k))).then(|| {
            unread.push(collapse(e));
            Condition::Unknown(collapse(e))
        })
    }

    /// The access condition of a Jinja expression when some part checks the
    /// user; the other parts of a mixed expression stay `unknown`.
    fn expr(e: &str, unread: &mut Vec<String>) -> Option<Condition> {
        let e = unparen(e);
        for (word, any) in [(" or ", true), (" and ", false)] {
            let parts = split_top(e, word);
            if parts.len() > 1 {
                let cs: Vec<Option<Condition>> = parts.iter().map(|p| expr(p, unread)).collect();
                if cs.iter().all(Option::is_none) {
                    return None;
                }
                let cs: Vec<Condition> = (cs.into_iter().zip(parts))
                    .map(|(c, p)| c.unwrap_or_else(|| Condition::Unknown(collapse(p))))
                    .collect();
                return Some(if any { any_of(cs) } else { all(cs) });
            }
        }
        match e.strip_prefix("not ") {
            Some(rest) => expr(rest, unread).map(not),
            None => atom(e, unread),
        }
    }

    fn cond(e: &str, rel: &str, line: u32, diags: &mut Vec<UiFlowDiagnostic>) -> Option<Condition> {
        let mut unread = vec![];
        let c = expr(e, &mut unread);
        for u in unread {
            let message = format!(
                "Jinja condition `{u}` checks the user but is not read; the links below it are Unknown."
            );
            gap(diags, rel, line, message);
        }
        c
    }

    /// The first word of `text` when it is a quoted literal.
    fn quoted(text: &str) -> Option<String> {
        let q = text.chars().next().filter(|c| matches!(c, '\'' | '"'))?;
        let end = text[1..].find(q)?;
        Some(text[1..1 + end].to_string())
    }

    fn url_for(inner: &str) -> Option<Dest> {
        let Some(args) = inner.strip_prefix("url_for(") else {
            return Some(Dest::Unread);
        };
        match quoted(args.trim_start()) {
            Some(n) if n == "static" || n.ends_with(".static") => None,
            Some(n) if !n.starts_with('.') => Some(Dest::Endpoint(n)),
            _ => Some(Dest::Unread),
        }
    }

    fn link(
        tag: &Tag<'_>,
        (rel, line): (&str, u32),
        cond: Option<Condition>,
        t: &mut Tpl,
        diags: &mut Vec<UiFlowDiagnostic>,
    ) {
        let form = tag.name.eq_ignore_ascii_case("form");
        let attrs = attributes(tag.attrs);
        let get = |n: &str| {
            (attrs.iter())
                .find(|(k, _)| k.eq_ignore_ascii_case(n))
                .map(|(_, v)| *v)
        };
        let Some(value) = get(if form { "action" } else { "href" }) else {
            return;
        };
        let value = value.trim();
        let dest = match value.strip_prefix("{{").and_then(|v| v.strip_suffix("}}")) {
            Some(inner) => url_for(inner.trim()),
            None if value.contains("{{") || value.contains("{%") => Some(Dest::Unread),
            None if value.starts_with('/') && !value.starts_with("//") => Some(Dest::Path(
                value.split(['?', '#']).next().unwrap_or(value).to_string(),
            )),
            None => None,
        };
        let Some(dest) = dest else {
            return;
        };
        if matches!(dest, Dest::Unread) {
            let message = format!(
                "Navigation target `{value}` is not read literally; its target is unknown."
            );
            gap(diags, rel, line, message);
        }
        t.links.push(Link {
            line,
            cond,
            method: form.then(|| get("method").unwrap_or("GET").trim().to_uppercase()),
            dest,
            source: value.to_string(),
        });
    }

    /// One `{% … %}` tag: opens, branches or closes a frame, or names a template.
    fn block(
        kw: &str,
        rest: &str,
        (rel, ln): (&str, u32),
        stack: &mut Vec<Option<Frame>>,
        t: &mut Tpl,
        diags: &mut Vec<UiFlowDiagnostic>,
    ) {
        match kw {
            "if" => {
                let c = cond(rest, rel, ln, diags);
                stack.push(Some(Frame {
                    seen: vec![],
                    own: c.clone(),
                    current: c,
                }));
            }
            "elif" => {
                let c = cond(rest, rel, ln, diags);
                if let Some(Some(f)) = stack.last_mut() {
                    branch(f, c);
                }
            }
            "else" => {
                if let Some(Some(f)) = stack.last_mut() {
                    branch(f, None);
                }
            }
            "for" | "block" | "macro" | "call" | "filter" | "with" | "autoescape" | "trans" => {
                stack.push(None);
            }
            "endif" | "endfor" | "endblock" | "endmacro" | "endcall" | "endfilter" | "endwith"
            | "endautoescape" | "endtrans" => {
                stack.pop();
            }
            "extends" | "include" | "import" | "from" => match quoted(rest) {
                Some(n) if kw == "extends" => t.extends.push((n, ln)),
                Some(n) => t.includes.push((n, now(stack), ln)),
                None => {
                    let message = format!(
                        "`{{% {kw} {rest} %}}` is not a literal template name; its links are unknown."
                    );
                    gap(diags, rel, ln, message);
                }
            },
            _ => {}
        }
    }

    enum Ev {
        Block(usize),
        Html(usize),
    }

    /// The links, includes, layouts and conditions of one template.
    pub(super) fn parse(rel: &str, text: &str, diags: &mut Vec<UiFlowDiagnostic>) -> Tpl {
        let text = blank(&blank(text, "{#", "#}"), "<!--", "-->");
        let tags = all_tags(&text);
        let mut events: Vec<(usize, Ev)> = vec![];
        let mut from = 0;
        while let Some(o) = text[from..].find("{%") {
            let open = from + o;
            let Some(c) = text[open..].find("%}") else {
                break;
            };
            events.push((open, Ev::Block(open + c)));
            from = open + c + 2;
        }
        for (i, t) in tags.iter().enumerate() {
            let wanted = t.name.eq_ignore_ascii_case("a") || t.name.eq_ignore_ascii_case("form");
            if !t.closing && wanted {
                events.push((t.at, Ev::Html(i)));
            }
        }
        events.sort_by_key(|e| e.0);
        let (mut t, mut stack) = (Tpl::default(), Vec::<Option<Frame>>::new());
        for (at, ev) in events {
            let ln = archguard_flow_common::lang_text::line(&text, at);
            let Ev::Block(close) = ev else {
                if let Ev::Html(i) = ev {
                    link(&tags[i], (rel, ln), now(&stack), &mut t, diags);
                }
                continue;
            };
            let inner = text[at + 2..close].trim().trim_matches('-').trim();
            let (kw, rest) = inner.split_once(char::is_whitespace).unwrap_or((inner, ""));
            let rest = rest.trim();
            block(kw, rest, (rel, ln), &mut stack, &mut t, diags);
        }
        t
    }

    /// Template walk: which template renders where, under what condition.
    struct Tr<'a> {
        templates: &'a BTreeMap<String, Vec<(String, String)>>,
        diags: Vec<UiFlowDiagnostic>,
        tpls: BTreeMap<String, Option<(String, Tpl)>>,
        map: BTreeMap<String, BTreeSet<(Origin, Option<Condition>)>>,
        /// (layout, file and line of its `extends`, application).
        layouts: Vec<(String, String, u32, String)>,
    }

    impl Tr<'_> {
        fn load(&mut self, name: &str, site: (&str, u32)) -> bool {
            if !self.tpls.contains_key(name) {
                let entry = match self.templates.get(name).map(Vec::as_slice) {
                    Some([(rel, text)]) => Some((rel.clone(), parse(rel, text, &mut self.diags))),
                    Some(many) if many.len() > 1 => {
                        let message = format!(
                            "Template `{name}` exists {} times; its links are unknown.",
                            many.len()
                        );
                        gap(&mut self.diags, site.0, site.1, message);
                        None
                    }
                    _ => {
                        let message =
                            format!("Template `{name}` not found; its links are unknown.");
                        gap(&mut self.diags, site.0, site.1, message);
                        None
                    }
                };
                self.tpls.insert(name.to_string(), entry);
            }
            self.tpls[name].is_some()
        }

        fn visit(
            &mut self,
            name: &str,
            from: (&Origin, Option<Condition>),
            app: &str,
            site: (&str, u32),
            stack: &mut Vec<String>,
        ) {
            let (origin, cond) = from;
            if stack.iter().any(|s| s == name) {
                return;
            }
            if stack.len() > 16 {
                let message = format!(
                    "Template `{name}` is nested deeper than 16 templates; its links are unknown."
                );
                return gap(&mut self.diags, site.0, site.1, message);
            }
            if !self.load(name, site) {
                return;
            }
            let fresh = (self.map.entry(name.to_string()).or_default())
                .insert((origin.clone(), cond.clone()));
            if !fresh {
                return;
            }
            let Some((rel, tpl)) = self.tpls[name].clone() else {
                return;
            };
            stack.push(name.to_string());
            for (include, c, line) in &tpl.includes {
                let next = and2(cond.clone(), c.clone());
                self.visit(include, (origin, next), app, (&rel, *line), stack);
            }
            for (layout, line) in &tpl.extends {
                self.layouts
                    .push((layout.clone(), rel.clone(), *line, app.to_string()));
            }
            stack.pop();
        }
    }

    impl Project<'_> {
        /// Transitions and actions of the templates the screens render.
        pub(super) fn emit_templates(&self, acc: &Acc, flow: &mut UiFlow) {
            if acc.screens.is_empty() {
                return;
            }
            let mut tr = Tr {
                templates: &self.templates,
                diags: vec![],
                tpls: BTreeMap::new(),
                map: BTreeMap::new(),
                layouts: vec![],
            };
            for s in &acc.screens {
                let app = s.application.clone().unwrap_or_default();
                for v in &s.views {
                    let origin = Origin::Screen(s.id.clone());
                    let site = (s.file.as_str(), s.line);
                    tr.visit(v, (&origin, None), &app, site, &mut vec![]);
                }
            }
            let mut seen = BTreeSet::new();
            while let Some((name, file, line, app)) = tr.layouts.pop() {
                if seen.insert((name.clone(), app.clone())) {
                    let every = Origin::AnyScreen {
                        application: app.clone(),
                    };
                    tr.visit(&name, (&every, None), &app, (&file, line), &mut vec![]);
                }
            }
            for (name, origins) in &tr.map {
                if let Some(Some((rel, tpl))) = tr.tpls.get(name) {
                    emit_links(acc, (rel, tpl), origins, &mut tr.diags, flow);
                }
            }
            flow.diagnostics.append(&mut tr.diags);
        }
    }

    /// Every link of one template at every origin it renders from.
    fn emit_links(
        acc: &Acc,
        (rel, tpl): (&str, &Tpl),
        origins: &BTreeSet<(Origin, Option<Condition>)>,
        diags: &mut Vec<UiFlowDiagnostic>,
        flow: &mut UiFlow,
    ) {
        for (link, (origin, c)) in tpl
            .links
            .iter()
            .flat_map(|l| origins.iter().map(move |o| (l, o)))
        {
            let app = match origin {
                Origin::Screen(id) => (acc.screens.iter())
                    .find(|s| &s.id == id)
                    .and_then(|s| s.application.clone()),
                Origin::AnyScreen { application } => {
                    (!application.is_empty()).then(|| application.clone())
                }
            };
            let emit = Emit {
                acc,
                rel,
                link,
                origin,
                app,
                condition: and2(c.clone(), link.cond.clone()),
            };
            emit.run(diags, flow);
        }
    }

    /// One link placed at one origin.
    struct Emit<'e> {
        acc: &'e Acc,
        rel: &'e str,
        link: &'e Link,
        origin: &'e Origin,
        app: Option<String>,
        condition: Option<Condition>,
    }

    impl Emit<'_> {
        fn run(&self, diags: &mut Vec<UiFlowDiagnostic>, flow: &mut UiFlow) {
            let link = self.link;
            let method = link.method.clone().unwrap_or_else(|| "GET".into());
            let target: Result<String, String> = match &link.dest {
                Dest::Path(p) => Ok(p.clone()),
                Dest::Endpoint(ep) => {
                    let routes = self.acc.names.get(&(self.app.clone(), ep.clone()));
                    let any = routes.and_then(|r| r.first());
                    let by_method = routes.and_then(|r| r.iter().find(|x| x.0 == method));
                    by_method
                        .or(if method == "GET" { None } else { any })
                        .map(|r| r.1.clone())
                        .ok_or_else(|| format!("Endpoint `{ep}` names no {method} route of the application; its target is unknown."))
                }
                Dest::Unread => Err(String::new()),
            };
            let condition = AccessState::from_parts(self.condition.clone(), false);
            let (file, line, source) = (self.rel.to_string(), link.line, "flask".to_string());
            let action = |call| Action {
                weak: false,
                screen: self.origin.clone(),
                trigger: None,
                call,
                file: file.clone(),
                line,
                condition: condition.clone(),
                source: source.clone(),
            };
            let transition = |target| Transition {
                weak: false,
                from: self.origin.clone(),
                target,
                trigger: None,
                file: file.clone(),
                line,
                condition: condition.clone(),
                source: source.clone(),
            };
            match target {
                Ok(path) if method == "GET" => {
                    let screen = (self.acc.screens.iter())
                        .find(|s| s.application == self.app && s.route == path);
                    match screen {
                        Some(s) => flow
                            .transitions
                            .push(transition(Target::Screen(s.id.clone()))),
                        None => flow.actions.push(action(Call::Http { method, path })),
                    }
                }
                Ok(path) => flow.actions.push(action(Call::Http { method, path })),
                Err(why) => {
                    if !why.is_empty() {
                        gap(diags, self.rel, line, why);
                    }
                    match method == "GET" {
                        true => flow
                            .transitions
                            .push(transition(Target::Unresolved(link.source.clone()))),
                        false => flow
                            .actions
                            .push(action(Call::Unresolved(link.source.clone()))),
                    }
                }
            }
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
    #[test]
    fn login_disabled_non_literal_makes_login_required_unknown() {
        let app = "from flask import Flask\nfrom flask_login import login_required\napp = Flask(__name__)\n\
                   @app.route('/a')\n@login_required\ndef a():\n    pass\n";
        let flow = run(
            &[(
                "app.py",
                &format!("{app}app.config['LOGIN_DISABLED'] = os.environ.get('X')\n"),
            )],
            "",
        );
        assert_eq!(eps(&flow), [get("/a", prot(unknown("login_required")))]);
        assert!(!no_gap(&flow));
        let flow = run(
            &[(
                "app.py",
                &format!("{app}app.config['LOGIN_DISABLED'] = False\n"),
            )],
            "",
        );
        assert_eq!(eps(&flow), [get("/a", prot(Condition::Authenticated))]);
        assert!(no_gap(&flow), "{:?}", flow.diagnostics);
    }

    #[test]
    fn before_app_request_called_as_function_guards_the_whole_app() {
        let flow = one(
            "@login_required\ndef guard():\n    pass\nbp.before_app_request(guard)\n\
             @app.route('/z')\ndef z():\n    pass\n",
            "",
        );
        assert_eq!(eps(&flow), [get("/z", prot(Condition::Authenticated))]);
    }

    #[test]
    fn add_url_rule_gap_names_the_real_cause() {
        let flow = one(
            "def f():\n    pass\napp.add_url_rule('/u', 'f', f, methods=METHODS)\n\
             app.add_url_rule('/v', 'g', g)\n",
            "",
        );
        let m: Vec<&str> = flow
            .diagnostics
            .iter()
            .map(|d| d.message.as_str())
            .collect();
        assert!(
            m.iter()
                .any(|t| t.contains("methods=METHODS") && t.contains("not a literal list")),
            "{m:?}"
        );
        assert!(
            m.iter()
                .any(|t| t.contains("`g`") && !t.contains("view_func=g")),
            "{m:?}"
        );
    }
    const JINJA_APP: &str = "from flask import Flask, Blueprint, render_template\n\
        app = Flask(__name__)\nmain = Blueprint('main', __name__)\nauth = Blueprint('auth', __name__)\n\
        app.register_blueprint(main)\napp.register_blueprint(auth, url_prefix='/auth')\n\
        @main.route('/')\ndef index():\n    return render_template('index.html')\n\
        @main.route('/user/<username>')\ndef user(username):\n    return render_template('user.html')\n\
        @auth.route('/login')\ndef login():\n    return render_template('login.html')\n";

    /// (target route, condition, trigger line) of every transition.
    fn jinja(view: &str) -> (UiFlow, Vec<(String, Option<AccessState>)>) {
        jinja_with(view, &[])
    }

    fn jinja_with(
        view: &str,
        extra: &[(&str, &str)],
    ) -> (UiFlow, Vec<(String, Option<AccessState>)>) {
        let mut files = vec![
            ("app.py", JINJA_APP),
            ("templates/index.html", view),
            ("templates/user.html", "<p>u</p>"),
            ("templates/login.html", "<p>l</p>"),
        ];
        files.extend_from_slice(extra);
        let flow = run(&files, "");
        let route = |id: &str| {
            let s = flow.screens.iter().find(|s| s.id == id);
            s.and_then(|s| s.route.clone()).unwrap_or(id.to_string())
        };
        let rows = (flow.transitions.iter())
            .map(|t| match &t.target {
                archguard_core::Target::Screen(id) => (route(id), Some(t.condition.clone())),
                archguard_core::Target::Unresolved(x) => (x.clone(), Some(t.condition.clone())),
            })
            .collect();
        (flow, rows)
    }

    #[test]
    fn jinja_if_around_links_is_the_screen_guard() {
        // fixture 13
        let view = "{% if current_user.is_anonymous %}<a href=\"{{ url_for('auth.login') }}\">in</a>\n\
                    {% else %}<a href=\"{{ url_for('main.user', username=current_user.username) }}\">me</a>\n\
                    {% endif %}<a href=\"{{ url_for('main.index') }}\">home</a>";
        let (flow, rows) = jinja(view);
        let want = [
            (
                "/auth/login",
                Some(prot(Condition::Not(Box::new(Condition::Authenticated)))),
            ),
            ("/user/:username", Some(prot(Condition::Authenticated))),
            ("/", Some(AccessState::Undeclared)),
        ];
        for (to, cond) in want {
            assert!(
                rows.iter().any(|r| r.0 == to && r.1 == cond),
                "{to} {rows:?}"
            );
        }
        assert!(no_gap(&flow), "{:?}", flow.diagnostics);
    }

    #[test]
    fn jinja_unreadable_user_check_is_unknown_and_a_gap() {
        let view = "{% if current_user.can_edit(post) %}<a href=\"{{ url_for('main.index') }}\">e</a>{% endif %}\
                    {% if posts %}<a href=\"{{ url_for('auth.login') }}\">l</a>{% endif %}";
        let (flow, rows) = jinja(view);
        let c = |to: &str| rows.iter().find(|r| r.0 == to).and_then(|r| r.1.clone());
        assert_eq!(c("/"), Some(prot(unknown("current_user.can_edit(post)"))));
        assert_eq!(c("/auth/login"), Some(AccessState::Undeclared));
        let gaps: Vec<_> = (flow.diagnostics.iter())
            .filter(|d| d.severity == DiagnosticSeverity::Gap)
            .collect();
        assert!(
            gaps.len() == 1 && gaps[0].file.ends_with("index.html") && gaps[0].line == 1,
            "{gaps:?}"
        );
    }
    fn cond_of(rows: &[(String, Option<AccessState>)], to: &str) -> Option<AccessState> {
        rows.iter().find(|r| r.0 == to).and_then(|r| r.1.clone())
    }

    fn gaps(flow: &UiFlow) -> Vec<String> {
        (flow.diagnostics.iter())
            .filter(|d| d.severity == DiagnosticSeverity::Gap)
            .map(|d| format!("{}:{} {}", d.file, d.line, d.message))
            .collect()
    }

    const HOME: &str = "<a href=\"{{ url_for('main.index') }}\">h</a>";

    #[test]
    fn jinja_non_ascii_condition_does_not_panic() {
        let view = format!("{{% if größe or current_user.is_authenticated %}}{HOME}{{% endif %}}");
        let (_, rows) = jinja(&view);
        let want = any_of(vec![unknown("größe"), Condition::Authenticated]);
        assert_eq!(cond_of(&rows, "/"), Some(prot(want)));
    }

    #[test]
    fn jinja_flask_user_idioms_are_unknown_and_a_gap() {
        for atom in ["g.user", "session['user_id']", "user"] {
            let (flow, rows) = jinja(&format!("{{% if {atom} %}}{HOME}{{% endif %}}"));
            assert_eq!(cond_of(&rows, "/"), Some(prot(unknown(atom))), "{atom}");
            let found = gaps(&flow);
            assert!(
                found.iter().any(|g| g.contains("index.html:1 ")),
                "{atom} {found:?}"
            );
        }
    }

    #[test]
    fn jinja_import_and_from_follow_the_macro_file() {
        let macros = "{% macro l() %}<a href=\"{{ url_for('auth.login') }}\">l</a>{% endmacro %}";
        let view = "{% import 'm.html' as m %}{% from 'm.html' import l %}";
        let (flow, rows) = jinja_with(view, &[("templates/m.html", macros)]);
        assert_eq!(
            cond_of(&rows, "/auth/login"),
            Some(AccessState::Undeclared),
            "{rows:?}"
        );
        let (flow2, _) = jinja("{% import 'zz.html' as z %}{% from v import x %}");
        assert!(no_gap(&flow), "{:?}", flow.diagnostics);
        let found = gaps(&flow2).join("\n");
        assert!(
            found.contains("`zz.html` not found") && found.contains("not a literal"),
            "{found}"
        );
    }

    #[test]
    fn jinja_include_chain_deeper_than_16_is_a_gap() {
        let owned: Vec<(String, String)> = (0..19)
            .map(|i| {
                (
                    format!("templates/t{i}.html"),
                    format!("{{% include 't{}.html' %}}", i + 1),
                )
            })
            .collect();
        let extra: Vec<(&str, &str)> = owned
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        let (flow, _) = jinja_with("{% include 't0.html' %}", &extra);
        let found = gaps(&flow).join("\n");
        assert!(found.contains("deeper than 16"), "{found}");
    }

    #[test]
    fn jinja_guard_nested_if_mixed_and_or_include_and_extends() {
        // guards: behaviour that held already
        let nested = format!(
            "{{% if current_user.is_authenticated %}}{{% if current_user.has_role('admin') %}}{HOME}\
             {{% endif %}}<a href=\"{{{{ url_for('auth.login') }}}}\">l</a>{{% endif %}}"
        );
        let (_, rows) = jinja(&nested);
        let admin = all(vec![Condition::Authenticated, role("admin")]);
        assert_eq!(cond_of(&rows, "/"), Some(prot(admin)));
        assert_eq!(
            cond_of(&rows, "/auth/login"),
            Some(prot(Condition::Authenticated))
        );

        let mixed = format!(
            "{{% if current_user.is_authenticated and current_user.has_role('a') or \
             current_user.has_permission('p') %}}{HOME}{{% endif %}}"
        );
        let (_, rows) = jinja(&mixed);
        let want = any_of(vec![
            all(vec![Condition::Authenticated, role("a")]),
            policy("p"),
        ]);
        assert_eq!(cond_of(&rows, "/"), Some(prot(want)));

        let layout = "{% if current_user.is_anonymous %}<a href=\"{{ url_for('auth.login') }}\">l</a>{% endif %}";
        let part = "<a href=\"{{ url_for('main.user', username='x') }}\">u</a>";
        let view = "{% extends 'layout.html' %}{% if current_user.is_authenticated %}{% include 'part.html' %}{% endif %}";
        let extra = [
            ("templates/layout.html", layout),
            ("templates/part.html", part),
        ];
        let (_, rows) = jinja_with(view, &extra);
        let anon = Condition::Not(Box::new(Condition::Authenticated));
        assert_eq!(cond_of(&rows, "/auth/login"), Some(prot(anon)));
        assert_eq!(
            cond_of(&rows, "/user/:username"),
            Some(prot(Condition::Authenticated))
        );
    }
}
