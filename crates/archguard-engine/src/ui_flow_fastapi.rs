//! FastAPI endpoints and their access for ui-flow/v1 (ADR 0021).
//!
//! A text-level scanner over the `.py` files below the root (test files left
//! out): comments and string bodies are blanked (`py`), brackets matched,
//! literals read from a token table. It fills only `endpoints` and
//! `diagnostics`, and only when some file creates a `FastAPI` or `APIRouter`.
//!
//! Routes: `x = FastAPI(dependencies=[…])`, `x = APIRouter(prefix=…,
//! dependencies=[…])` (also inside an app factory function), decorators
//! `@x.get/post/put/patch/delete/head/options/trace(path, dependencies=[…])` and
//! `@x.api_route(path, methods=[…])` (`GET` without `methods`, as in FastAPI),
//! and `x.include_router(y, prefix=…, dependencies=[…])` (also under an `if`,
//! counted as always included). Names follow `import` and `from … import`
//! through the project: relative imports, absolute ones by a unique module path
//! suffix (the nearest to the importing file wins). The path joins every
//! prefix from a `FastAPI` app down to the route path by plain concatenation,
//! as FastAPI does. A prefix or path is a literal (adjacent literals join, an
//! f-string without holes counts), a module constant, or `obj.FIELD` where
//! `obj = Cls()` and the body of class `Cls` gives `FIELD` a literal default
//! (pydantic `BaseSettings`, e.g. `settings.API_V1_STR`).
//!
//! Access: the dependencies of the app, of every router and include on the
//! chain, of the decorator and of the handler's parameters (`= Depends(f)`,
//! `Annotated[T, Depends(f)]`, an alias `X = Annotated[T, Depends(f)]` used as
//! annotation, `Security(f, scopes=[…])`) must all hold. One dependency `f`,
//! named by its last dotted segment:
//! - listed in `[views.access]`: that meaning alone; `$arg` reads the literal
//!   arguments of a factory call `Depends(f("a"))`, or the scopes of `Security`;
//! - an instance of a `fastapi.security` scheme class (`OAuth2PasswordBearer`,
//!   `HTTPBearer`, `APIKeyHeader`, …): `authenticated`, since the scheme answers
//!   401 without credentials; with a literal `auto_error=False`: `custom(f)`;
//! - a function of the project: `custom(f)` together with what its own
//!   parameters' dependencies require, when one of those is access (followed
//!   transitively, cycle-safe); otherwise it is no access (`get_db`). No body
//!   is read: `if not user.is_superuser: raise …` stays inside `custom(f)`;
//! - `Security(f, scopes=[…])` unlisted: `f` as above (`custom(f)` at least)
//!   and every scope as a `role` atom, all required (FastAPI hands the
//!   dependency every scope as required);
//! - not found in the project, or a factory call `f(…)` not listed: no access,
//!   with a note.
//!
//! No access dependency: `undeclared`; only `open`-listed ones: `open`.
//! Gaps: a path or prefix that is no readable literal (that endpoint or
//! included subtree is skipped), an included router not found, a router with
//! endpoints that no `FastAPI` app includes (its paths start at its own
//! prefix), a `$arg` meaning or scope list without readable arguments.
//! Not read: `add_api_route`, `mount` of sub-applications, websockets,
//! `Depends()` without argument (class dependencies), routers built in loops.
//!
//! Endpoint `application`: the directory name of the nearest `pyproject.toml`,
//! `setup.py`, `setup.cfg` or `requirements.txt`. `handler`: the dotted module
//! (packages by `__init__.py`) plus the function; `sends` stays empty.
use crate::extract_ctx::{
    ExtractContext, ancestors, cap, dir_name, finish, gap, read_text, walk, with_root_config,
};
use crate::services::support::test_path;
use archguard_core::{
    AccessMeaning, AccessState, Condition, DiagnosticSeverity, EndpointAccess, UiFlow,
    UiFlowDiagnostic,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use model::{Def, Dep, Kind, Val};
use project::{Project, Res};

const MAX_FILES: usize = 20_000;
const BUILD_FILES: [&str; 4] = [
    "pyproject.toml",
    "setup.py",
    "setup.cfg",
    "requirements.txt",
];

pub fn extract(root: &Path) -> UiFlow {
    with_root_config(root, extract_with)
}

/// [`extract`] for one run.
pub fn extract_with(ctx: &ExtractContext) -> UiFlow {
    let mut flow = UiFlow::default();
    let Some(project) = Project::read(ctx.root, &mut flow.diagnostics) else {
        finish(&mut flow);
        return flow;
    };
    if let Some(e) = &ctx.config_error {
        let message = format!("Configuration not read; [views.access] not applied: {e}");
        note(&mut flow.diagnostics, ".", 0, message);
    }
    let mut tree = Tree::new(&project, &ctx.config.views.access, &mut flow);
    tree.emit_all();
    flow.diagnostics.append(&mut tree.eval.diags);
    flow.endpoints.append(&mut tree.endpoints);
    finish(&mut flow);
    flow
}

fn note(out: &mut Vec<UiFlowDiagnostic>, file: &str, line: u32, message: impl Into<String>) {
    out.push(UiFlowDiagnostic {
        file: file.into(),
        line,
        message: message.into(),
        severity: DiagnosticSeverity::Note,
    });
}

/// All of `items`, nested `all` flattened, sorted and deduplicated.
fn all(items: Vec<Condition>) -> Condition {
    let mut flat = Vec::new();
    for c in items {
        match c {
            Condition::All(inner) => flat.extend(inner),
            c => flat.push(c),
        }
    }
    flat.sort();
    flat.dedup();
    if flat.len() == 1 {
        flat.remove(0)
    } else {
        Condition::All(flat)
    }
}

/// FastAPI's view of one Python source on top of `lang_python`: the masked
/// bytes and bracket pairs, with offset-range helpers.
pub(crate) mod py {
    pub(crate) use crate::lang_pyscan::{dotted, skip_sp, skip_ws, word};
    use crate::lang_python::{code_statements, mask, string_value};
    use std::collections::HashMap;

    /// One Python source: comments and string bodies blanked to spaces
    /// (newlines kept, so positions and lines match the text).
    pub(crate) struct Src {
        pub(crate) b: Vec<u8>,
        text: String,
        pairs: HashMap<usize, usize>,
    }

    impl Src {
        pub(crate) fn new(text: &str) -> Self {
            let b = mask(text).into_bytes();
            let pairs = pairs(&b);
            let text = text.to_string();
            Self { b, text, pairs }
        }

        /// 1-based line of byte `at`.
        pub(crate) fn line(&self, at: usize) -> u32 {
            crate::lang_text::line(&self.text, at)
        }

        /// The source text of `a..e`.
        pub(crate) fn text(&self, (a, e): (usize, usize)) -> &str {
            self.text.get(a..e).unwrap_or("")
        }

        pub(crate) fn close(&self, open: usize) -> Option<usize> {
            self.pairs.get(&open).copied()
        }

        /// The value of the string literals covering `a..e`; adjacent
        /// literals join. `None` for anything else or an f-string with holes.
        pub(crate) fn literal(&self, a: usize, e: usize) -> Option<String> {
            string_value(self.text((a, e)))
        }

        /// Statement starts: first non-blank byte and indent of every logical line.
        pub(crate) fn statements(&self) -> Vec<(usize, usize)> {
            let masked = String::from_utf8_lossy(&self.b);
            (code_statements(&self.text, &masked).iter())
                .map(|s| (s.at, s.indent))
                .collect()
        }

        /// Where the statement holding `i` ends: the first newline outside
        /// brackets that no backslash continues.
        pub(crate) fn stmt_end(&self, mut i: usize) -> usize {
            while i < self.b.len() {
                if let Some(c) = self.close(i) {
                    i = c + 1;
                    continue;
                }
                if self.b[i] == b'\n' && !self.b[..i].trim_ascii_end().ends_with(b"\\") {
                    return i;
                }
                i += 1;
            }
            i
        }

        /// The top-level comma-separated parts inside the bracket at `open`,
        /// trimmed, empty ones dropped.
        pub(crate) fn split(&self, open: usize) -> Vec<(usize, usize)> {
            let Some(close) = self.close(open) else {
                return Vec::new();
            };
            let (mut parts, mut a, mut i) = (Vec::new(), open + 1, open + 1);
            while i < close {
                if let Some(c) = self.close(i) {
                    i = c + 1;
                    continue;
                }
                if self.b[i] == b',' {
                    parts.push(self.trim(a, i));
                    a = i + 1;
                }
                i += 1;
            }
            parts.push(self.trim(a, close));
            parts.retain(|(a, e)| a < e);
            parts
        }

        pub(crate) fn trim(&self, mut a: usize, mut e: usize) -> (usize, usize) {
            a = skip_ws(&self.b, a).min(e);
            while e > a && self.b[e - 1].is_ascii_whitespace() {
                e -= 1;
            }
            (a, e)
        }

        /// `name=value` → (name, value range).
        pub(crate) fn named(&self, (a, e): (usize, usize)) -> Option<(&str, (usize, usize))> {
            let (name, j) = dotted(&self.b, a);
            let k = skip_ws(&self.b, j);
            let eq = self.b.get(k) == Some(&b'=') && self.b.get(k + 1) != Some(&b'=');
            (eq && !name.contains('.')).then(|| {
                (
                    std::str::from_utf8(&self.b[a..j]).unwrap_or(""),
                    self.trim(k + 1, e),
                )
            })
        }

        /// The keyword argument `key` among `parts`.
        pub(crate) fn kwarg(&self, parts: &[(usize, usize)], key: &str) -> Option<(usize, usize)> {
            parts
                .iter()
                .find_map(|p| self.named(*p).filter(|(n, _)| *n == key).map(|(_, v)| v))
        }

        /// The first positional argument among `parts`.
        pub(crate) fn first(&self, parts: &[(usize, usize)]) -> Option<(usize, usize)> {
            parts
                .iter()
                .copied()
                .find(|p| self.named(*p).is_none() && self.b[p.0] != b'*')
        }

        /// The dotted name exactly covering `a..e`.
        pub(crate) fn name(&self, (a, e): (usize, usize)) -> Option<String> {
            let (name, j) = dotted(&self.b, a);
            (!name.is_empty() && j == e).then_some(name)
        }

        /// `callee(…)` exactly covering `a..e`: the callee and its `(`.
        pub(crate) fn call(&self, (a, e): (usize, usize)) -> Option<(String, usize)> {
            let (name, j) = dotted(&self.b, a);
            let open = skip_ws(&self.b, j);
            let whole = self.b.get(open) == Some(&b'(') && self.close(open) == Some(e - 1);
            (!name.is_empty() && whole).then_some((name, open))
        }

        /// A list or tuple of string literals exactly covering `a..e`.
        pub(crate) fn strings(&self, (a, e): (usize, usize)) -> Option<Vec<String>> {
            let list = matches!(self.b[a], b'[' | b'(') && self.close(a) == Some(e - 1);
            if !list {
                return None;
            }
            self.split(a)
                .into_iter()
                .map(|(x, y)| self.literal(x, y))
                .collect()
        }
    }

    /// Bracket pairs of blanked text, open to close.
    fn pairs(b: &[u8]) -> HashMap<usize, usize> {
        let (mut stack, mut out) = (Vec::new(), HashMap::new());
        for (i, c) in b.iter().enumerate() {
            match c {
                b'(' | b'[' | b'{' => stack.push(i),
                b')' | b']' | b'}' => {
                    if let Some(o) = stack.pop() {
                        out.insert(o, i);
                    }
                }
                _ => {}
            }
        }
        out
    }
}

/// What one file declares, read statement by statement.
mod model {
    use super::py::{Src, dotted, skip_sp, skip_ws, word};
    use std::collections::HashMap;

    pub(super) enum Val {
        Lit(String),
        Name(String),
        /// Source text of anything else.
        Other(String),
    }

    #[derive(Clone, Copy, PartialEq)]
    pub(super) enum Kind {
        Depends,
        Security,
        /// A parameter annotation that may be an `Annotated` alias.
        Annotation,
    }

    pub(super) struct Dep {
        pub(super) file: usize,
        pub(super) at: usize,
        pub(super) kind: Kind,
        /// Dotted name; empty for `Depends()`, source text when not a name.
        pub(super) target: String,
        /// `Depends(f(…))`: a factory call.
        pub(super) call: bool,
        /// Literal call arguments; `None` when one is not a literal.
        pub(super) args: Option<Vec<String>>,
        pub(super) scopes: Option<Vec<String>>,
    }

    pub(super) enum Def {
        App {
            router: bool,
            prefix: Option<Val>,
            deps: Vec<Dep>,
            at: usize,
        },
        Alias(Vec<Dep>),
        Func(Vec<Dep>),
        Scheme {
            auto_error: bool,
        },
        Str(String),
        Class(HashMap<String, String>),
        /// `x = Cls(…)`: the callee.
        Instance(String),
    }

    pub(super) struct Route {
        pub(super) router: String,
        pub(super) methods: Vec<String>,
        pub(super) path: Option<Val>,
        pub(super) deps: Vec<Dep>,
        pub(super) func: String,
        pub(super) at: usize,
    }

    pub(super) struct Include {
        pub(super) parent: String,
        pub(super) child: Option<String>,
        pub(super) prefix: Option<Val>,
        pub(super) deps: Vec<Dep>,
        pub(super) at: usize,
    }

    /// `from module import name` (`name: None` for `import module`); the
    /// module as written, leading dots for relative imports.
    pub(super) struct Import {
        pub(super) module: String,
        pub(super) name: Option<String>,
    }

    #[derive(Default)]
    pub(super) struct Facts {
        pub(super) imports: HashMap<String, Import>,
        pub(super) defs: HashMap<String, Def>,
        pub(super) routes: Vec<Route>,
        pub(super) includes: Vec<Include>,
        /// Decorator routes whose `methods=` is not a literal list.
        pub(super) unread: Vec<(usize, String)>,
    }

    const METHODS: [&str; 8] = [
        "get", "post", "put", "patch", "delete", "head", "options", "trace",
    ];
    const SCHEMES: [&str; 11] = [
        "OAuth2PasswordBearer",
        "OAuth2AuthorizationCodeBearer",
        "OAuth2",
        "HTTPBearer",
        "HTTPBasic",
        "HTTPDigest",
        "APIKeyHeader",
        "APIKeyQuery",
        "APIKeyCookie",
        "OpenIdConnect",
        "HTTPBase",
    ];

    impl Facts {
        pub(super) fn read(src: &Src, file: usize) -> Facts {
            let mut f = Facts::default();
            let stmts = src.statements();
            let mut decorators = Vec::new();
            for (k, &(at, indent)) in stmts.iter().enumerate() {
                let b = &src.b;
                if b[at] == b'@' {
                    decorators.push(at + 1);
                    continue;
                }
                let def = if word(b, at, "async") {
                    skip_sp(b, at + 5)
                } else {
                    at
                };
                if word(b, def, "def") {
                    let decorators = std::mem::take(&mut decorators);
                    f.func(src, file, (skip_sp(b, def + 3), indent), &decorators);
                    continue;
                }
                decorators.clear();
                if word(b, at, "from") || word(b, at, "import") {
                    f.import(src, at);
                } else if word(b, at, "class") && indent == 0 {
                    f.class(src, &stmts[k..]);
                } else {
                    f.statement(src, file, at, indent);
                }
            }
            f
        }

        fn func(
            &mut self,
            src: &Src,
            file: usize,
            (at, indent): (usize, usize),
            decorators: &[usize],
        ) {
            let (name, j) = dotted(&src.b, at);
            let open = skip_sp(&src.b, j);
            if name.is_empty() || src.b.get(open) != Some(&b'(') {
                return;
            }
            let deps = params(src, file, open);
            for &d in decorators {
                self.route(src, file, d, &name, &deps);
            }
            if indent == 0 {
                self.defs.insert(name, Def::Func(deps));
            }
        }

        fn route(&mut self, src: &Src, file: usize, at: usize, func: &str, own: &[Dep]) {
            let (name, j) = dotted(&src.b, at);
            let Some((router, method)) = name.rsplit_once('.') else {
                return;
            };
            let open = skip_sp(&src.b, j);
            if src.b.get(open) != Some(&b'(') {
                return;
            }
            let parts = src.split(open);
            let methods = match (method, src.kwarg(&parts, "methods")) {
                (m, _) if METHODS.contains(&m) => vec![m.to_ascii_uppercase()],
                ("api_route", None) => vec!["GET".to_string()],
                ("api_route", Some(v)) => match src.strings(v) {
                    Some(list) => list.iter().map(|m| m.to_ascii_uppercase()).collect(),
                    None => return self.unread.push((at, src.text(v).to_string())),
                },
                _ => return,
            };
            let path = src.first(&parts).or_else(|| src.kwarg(&parts, "path"));
            let mut deps = deps_kwarg(src, file, &parts);
            deps.extend(own.iter().map(Dep::copy));
            self.routes.push(Route {
                router: router.to_string(),
                methods,
                path: path.map(|p| val(src, p)),
                deps,
                func: func.to_string(),
                at: at - 1,
            });
        }

        fn import(&mut self, src: &Src, at: usize) {
            let raw = String::from_utf8_lossy(&src.b[at..src.stmt_end(at)]).into_owned();
            let text: String = (raw.chars())
                .map(|c| if "()\\\r\n".contains(c) { ' ' } else { c })
                .collect();
            let alias = |part: &str| {
                let w: Vec<&str> = part.split_whitespace().collect();
                match w.as_slice() {
                    [n, "as", a] => Some((n.to_string(), a.to_string())),
                    [n] if *n != "*" => Some((n.to_string(), n.to_string())),
                    _ => None,
                }
            };
            if let Some((module, names)) = text
                .strip_prefix("from")
                .and_then(|r| r.split_once(" import "))
            {
                for (name, local) in names.split(',').filter_map(alias) {
                    let module = module.trim().to_string();
                    self.imports.insert(
                        local,
                        Import {
                            module,
                            name: Some(name),
                        },
                    );
                }
            } else if let Some(rest) = text.strip_prefix("import") {
                for (module, local) in rest.split(',').filter_map(alias) {
                    let local = if local == module {
                        module.split('.').next().unwrap_or("").into()
                    } else {
                        local
                    };
                    let module = if local == module.split('.').next().unwrap_or("") {
                        local.clone()
                    } else {
                        module
                    };
                    self.imports.insert(local, Import { module, name: None });
                }
            }
        }

        /// `class Name…:` and the literal defaults of its direct body.
        fn class(&mut self, src: &Src, stmts: &[(usize, usize)]) {
            let (name, _) = dotted(&src.b, skip_sp(&src.b, stmts[0].0 + 5));
            let body = stmts.get(1).map_or(0, |s| s.1);
            let mut fields = HashMap::new();
            for &(at, indent) in stmts[1..].iter().take_while(|s| s.1 > 0) {
                if indent != body {
                    continue;
                }
                if let Some((field, rhs)) = assignment(src, at)
                    && let Some(v) = src.literal(rhs.0, rhs.1)
                {
                    fields.insert(field, v);
                }
            }
            self.defs.insert(name, Def::Class(fields));
        }

        fn statement(&mut self, src: &Src, file: usize, at: usize, indent: usize) {
            let (name, j) = dotted(&src.b, at);
            let open = skip_sp(&src.b, j);
            if let Some(parent) = name.strip_suffix(".include_router")
                && src.b.get(open) == Some(&b'(')
            {
                let parts = src.split(open);
                self.includes.push(Include {
                    parent: parent.to_string(),
                    child: src.first(&parts).and_then(|p| src.name(p)),
                    prefix: src.kwarg(&parts, "prefix").map(|p| val(src, p)),
                    deps: deps_kwarg(src, file, &parts),
                    at,
                });
            } else if let Some((name, rhs)) = assignment(src, at)
                && let Some(def) = definition(src, file, rhs, indent)
            {
                self.defs.insert(name, def);
            }
        }
    }

    /// `name = rhs` or `name: T = rhs` at `at`: the name and the rhs range.
    fn assignment(src: &Src, at: usize) -> Option<(String, (usize, usize))> {
        let (name, j) = dotted(&src.b, at);
        let end = src.stmt_end(at);
        let mut i = skip_sp(&src.b, j);
        if name.is_empty() || name.contains('.') || !matches!(src.b.get(i), Some(b'=' | b':')) {
            return None;
        }
        while i < end && src.b[i] != b'=' {
            i = src.close(i).map_or(i + 1, |c| c + 1);
        }
        let eq = i < end && src.b.get(i + 1) != Some(&b'=');
        eq.then(|| (name, src.trim(i + 1, end)))
    }

    /// What an assignment's right-hand side defines, when it matters here.
    fn definition(src: &Src, file: usize, rhs: (usize, usize), indent: usize) -> Option<Def> {
        if rhs.0 >= rhs.1 {
            return None;
        }
        if let Some(v) = src.literal(rhs.0, rhs.1) {
            return (indent == 0).then_some(Def::Str(v));
        }
        let (callee, j) = dotted(&src.b, rhs.0);
        let last = callee.rsplit('.').next().unwrap_or("");
        if last == "Annotated" && src.b.get(skip_ws(&src.b, j)) == Some(&b'[') {
            return Some(Def::Alias(deps_in(src, file, rhs)));
        }
        let (callee, open) = src.call(rhs)?;
        let parts = src.split(open);
        Some(match last {
            "FastAPI" | "APIRouter" => Def::App {
                router: last == "APIRouter",
                prefix: src.kwarg(&parts, "prefix").map(|p| val(src, p)),
                deps: deps_kwarg(src, file, &parts),
                at: rhs.0,
            },
            s if SCHEMES.contains(&s) => Def::Scheme {
                auto_error: src
                    .kwarg(&parts, "auto_error")
                    .is_none_or(|v| src.text(v) != "False"),
            },
            _ if indent == 0 => Def::Instance(callee),
            _ => return None,
        })
    }

    fn val(src: &Src, (a, e): (usize, usize)) -> Val {
        match (src.literal(a, e), src.name((a, e))) {
            (Some(v), _) => Val::Lit(v),
            (None, Some(n)) => Val::Name(n),
            _ => Val::Other(src.text((a, e)).to_string()),
        }
    }

    fn deps_kwarg(src: &Src, file: usize, parts: &[(usize, usize)]) -> Vec<Dep> {
        src.kwarg(parts, "dependencies")
            .map_or_else(Vec::new, |v| deps_in(src, file, v))
    }

    /// A function's parameter dependencies, annotations included.
    fn params(src: &Src, file: usize, open: usize) -> Vec<Dep> {
        let mut deps = src
            .close(open)
            .map_or_else(Vec::new, |c| deps_in(src, file, (open + 1, c)));
        for (a, e) in src.split(open) {
            let mut i = a;
            while i < e && src.b[i] != b':' && src.b[i] != b'=' {
                i = src.close(i).map_or(i + 1, |c| c + 1);
            }
            if src.b.get(i) != Some(&b':') {
                continue;
            }
            let (target, j) = dotted(&src.b, skip_ws(&src.b, i + 1));
            let k = skip_ws(&src.b, j);
            if !target.is_empty() && (k >= e || src.b[k] == b'=') {
                deps.push(Dep::new(file, i + 1, Kind::Annotation, target));
            }
        }
        deps
    }

    /// Every `Depends(…)` and `Security(…)` in `a..e`.
    fn deps_in(src: &Src, file: usize, (a, e): (usize, usize)) -> Vec<Dep> {
        let (mut out, mut i) = (Vec::new(), a);
        while i < e {
            let kind = match () {
                _ if word(&src.b, i, "Depends") => Kind::Depends,
                _ if word(&src.b, i, "Security") => Kind::Security,
                _ => {
                    i += 1;
                    continue;
                }
            };
            let open = skip_ws(&src.b, i + if kind == Kind::Depends { 7 } else { 8 });
            match src.close(open).filter(|_| src.b[open] == b'(') {
                Some(close) => {
                    out.push(dependency(src, file, (i, open), kind));
                    i = close + 1;
                }
                None => i += 1,
            }
        }
        out
    }

    fn dependency(src: &Src, file: usize, (at, open): (usize, usize), kind: Kind) -> Dep {
        let parts = src.split(open);
        let target = src
            .first(&parts)
            .or_else(|| src.kwarg(&parts, "dependency"));
        let mut dep = Dep::new(file, at, kind, String::new());
        if let Some(t) = target {
            match (src.name(t), src.call(t)) {
                (Some(n), _) => dep.target = n,
                (None, Some((n, args))) => {
                    let args = src.split(args);
                    dep.args = args.iter().map(|&(x, y)| src.literal(x, y)).collect();
                    (dep.target, dep.call) = (n, true);
                }
                _ => dep.target = src.text(t).to_string(),
            }
        }
        if let Some(s) = src.kwarg(&parts, "scopes") {
            dep.scopes = src.strings(s);
        }
        dep
    }

    impl Dep {
        fn new(file: usize, at: usize, kind: Kind, target: String) -> Self {
            Self {
                file,
                at,
                kind,
                target,
                call: false,
                args: Some(Vec::new()),
                scopes: Some(Vec::new()),
            }
        }

        fn copy(&self) -> Self {
            Self {
                target: self.target.clone(),
                args: self.args.clone(),
                scopes: self.scopes.clone(),
                ..*self
            }
        }
    }
}

/// The files of one run, and name resolution across them.
mod project {
    use super::*;
    use crate::extract_ctx::Gap;
    use model::Facts;

    pub(super) struct File {
        pub(super) rel: String,
        pub(super) src: py::Src,
        pub(super) facts: Facts,
        pub(super) application: Option<String>,
    }

    /// Module keys are paths without `.py`; a package is its directory.
    pub(super) struct Project {
        pub(super) files: Vec<File>,
        file_of: HashMap<String, usize>,
        keys: HashSet<String>,
        packages: HashSet<String>,
    }

    pub(super) enum Res {
        Module(String),
        Def(usize, String),
        /// `obj.FIELD` with `obj = Cls()`: the file of `obj`, `Cls`, `FIELD`.
        Field(usize, String, String),
    }

    impl Project {
        /// The Python sources below `root`; `None` when none creates a
        /// `FastAPI` app or an `APIRouter`.
        pub(super) fn read(root: &Path, diags: &mut Vec<UiFlowDiagnostic>) -> Option<Project> {
            let walk = match walk(root) {
                Ok(w) => w,
                Err(e) => {
                    gap(
                        diags,
                        "",
                        0,
                        format!("Discovery exclusions not read; no FastAPI app analyzed: {e}"),
                    );
                    return None;
                }
            };
            let (mut paths, mut build_dirs, mut gaps): (Vec<String>, HashSet<String>, Vec<Gap>) =
                Default::default();
            for rel in walk.files {
                let (dir, name) = rel.rsplit_once('/').unwrap_or(("", &rel));
                if BUILD_FILES.contains(&name) {
                    build_dirs.insert(dir.to_string());
                }
                if rel.ends_with(".py") && !test_path(&rel) {
                    paths.push(rel);
                }
            }
            paths.sort();
            if let Some(first) = cap(&mut paths, MAX_FILES) {
                gaps.push((first, format!("FastAPI scan stopped after {MAX_FILES} Python files; later files are unknown.")));
            }
            let mut files = Vec::new();
            for rel in paths {
                match read_text(root, &rel, 1 << 20) {
                    Ok(text) => files.push((rel, text)),
                    Err(e) => {
                        gaps.push((rel, format!("File not read ({e}); its routes are unknown.")))
                    }
                }
            }
            if !files
                .iter()
                .any(|(_, t)| t.contains("FastAPI(") || t.contains("APIRouter("))
            {
                return None;
            }
            for (file, message) in gaps {
                gap(diags, &file, 0, message);
            }
            Some(Self::new(root, files, &build_dirs))
        }

        fn new(root: &Path, sources: Vec<(String, String)>, build_dirs: &HashSet<String>) -> Self {
            let mut p = Project {
                files: Vec::new(),
                file_of: HashMap::new(),
                keys: HashSet::new(),
                packages: HashSet::new(),
            };
            for (i, (rel, text)) in sources.into_iter().enumerate() {
                let key = rel.trim_end_matches(".py");
                let key = key
                    .strip_suffix("/__init__")
                    .or(key.strip_suffix("__init__"))
                    .unwrap_or(key);
                if rel.ends_with("__init__.py") {
                    p.packages.insert(key.to_string());
                }
                p.file_of.insert(key.to_string(), i);
                p.keys.insert(key.to_string());
                p.keys
                    .extend(ancestors(&rel).filter(|d| !d.is_empty()).map(String::from));
                let application = ancestors(&rel)
                    .find(|d| build_dirs.contains(*d))
                    .map(|d| dir_name(root, d));
                let src = py::Src::new(&text);
                let facts = Facts::read(&src, i);
                p.files.push(File {
                    rel,
                    src,
                    facts,
                    application,
                });
            }
            p
        }

        pub(super) fn def(&self, file: usize, name: &str) -> Option<&Def> {
            self.files[file].facts.defs.get(name)
        }

        /// The dotted module of a file (packages by `__init__.py`).
        pub(super) fn module(&self, file: usize) -> String {
            let rel = &self.files[file].rel;
            let mut parts: Vec<&str> = Vec::new();
            let stem = rel
                .rsplit('/')
                .next()
                .unwrap_or(rel)
                .trim_end_matches(".py");
            if stem != "__init__" {
                parts.push(stem);
            }
            for dir in ancestors(rel).take_while(|d| self.packages.contains(*d)) {
                parts.push(dir.rsplit('/').next().unwrap_or(dir));
            }
            parts.reverse();
            parts.join(".")
        }

        /// The module key an import names from `file`.
        fn module_key(&self, file: usize, module: &str) -> Option<String> {
            let rel = &self.files[file].rel;
            let rest = module.trim_start_matches('.');
            let dots = module.len() - rest.len();
            let rest_path = rest.replace('.', "/");
            if dots > 0 {
                let base = ancestors(rel).nth(dots - 1)?;
                let key = [base, rest_path.as_str()]
                    .iter()
                    .filter(|s| !s.is_empty())
                    .copied()
                    .collect::<Vec<_>>()
                    .join("/");
                return self.keys.contains(&key).then_some(key);
            }
            let suffix = format!("/{rest_path}");
            let shared = |k: &str| {
                k.split('/')
                    .zip(rel.split('/'))
                    .take_while(|(a, b)| a == b)
                    .count()
            };
            let mut found: Vec<(usize, &String)> = (self.keys.iter())
                .filter(|k| **k == rest_path || k.ends_with(&suffix))
                .map(|k| (shared(k), k))
                .collect();
            found.sort_by_key(|a| std::cmp::Reverse(a.0));
            match found.as_slice() {
                [(n, k), rest @ ..] if rest.first().is_none_or(|(m, _)| m < n) => {
                    Some((*k).clone())
                }
                _ => None,
            }
        }

        fn lookup(&self, key: &str, name: &str, depth: usize) -> Option<Res> {
            if let Some(&f) = self.file_of.get(key)
                && let Some(r) = self.in_file(f, name, depth)
            {
                return Some(r);
            }
            let sub = if key.is_empty() {
                name.to_string()
            } else {
                format!("{key}/{name}")
            };
            self.keys.contains(&sub).then_some(Res::Module(sub))
        }

        fn in_file(&self, file: usize, name: &str, depth: usize) -> Option<Res> {
            let facts = &self.files[file].facts;
            if facts.defs.contains_key(name) {
                return Some(Res::Def(file, name.to_string()));
            }
            let import = facts.imports.get(name).filter(|_| depth < 16)?;
            let key = self.module_key(file, &import.module)?;
            match &import.name {
                Some(n) => self.lookup(&key, n, depth + 1),
                None => Some(Res::Module(key)),
            }
        }

        /// What dotted `name` in `file` refers to.
        pub(super) fn resolve(&self, file: usize, name: &str) -> Option<Res> {
            let mut segs = name.split('.');
            let mut cur = self.in_file(file, segs.next()?, 0)?;
            for seg in segs {
                cur = match cur {
                    Res::Module(k) => self.lookup(&k, seg, 0)?,
                    Res::Def(f, n) => match self.def(f, &n) {
                        Some(Def::Instance(cls)) => Res::Field(f, cls.clone(), seg.to_string()),
                        _ => return None,
                    },
                    Res::Field(..) => return None,
                };
            }
            Some(cur)
        }

        /// A path or prefix as text; `Err` with the source text when unreadable.
        pub(super) fn value(&self, file: usize, val: &Val) -> Result<String, String> {
            let name = match val {
                Val::Lit(s) => return Ok(s.clone()),
                Val::Name(n) => n,
                Val::Other(t) => return Err(t.clone()),
            };
            let found = match self.resolve(file, name) {
                Some(Res::Def(f, n)) => match self.def(f, &n) {
                    Some(Def::Str(s)) => Some(s.clone()),
                    _ => None,
                },
                Some(Res::Field(f, cls, field)) => match self.resolve(f, &cls) {
                    Some(Res::Def(g, c)) => match self.def(g, &c) {
                        Some(Def::Class(fields)) => fields.get(&field).cloned(),
                        _ => None,
                    },
                    _ => None,
                },
                _ => None,
            };
            found.ok_or_else(|| name.clone())
        }
    }
}

/// The access one dependency, or a list of them, requires.
mod access {
    use super::*;

    #[derive(Clone)]
    pub(super) enum Meaning {
        None,
        Open,
        Cond(Condition),
    }

    pub(super) struct Eval<'a> {
        p: &'a Project,
        access: &'a BTreeMap<String, AccessMeaning>,
        memo: HashMap<(usize, String), Meaning>,
        pub(super) diags: Vec<UiFlowDiagnostic>,
    }

    impl<'a> Eval<'a> {
        pub(super) fn new(p: &'a Project, access: &'a BTreeMap<String, AccessMeaning>) -> Self {
            Self {
                p,
                access,
                memo: HashMap::new(),
                diags: Vec::new(),
            }
        }

        /// All of `deps`: their conditions, else `open` when one is listed open.
        pub(super) fn deps<'d>(&mut self, deps: impl IntoIterator<Item = &'d Dep>) -> Meaning {
            let (mut conds, mut open) = (Vec::new(), false);
            for d in deps {
                match self.dep(d) {
                    Meaning::Cond(c) => conds.push(c),
                    Meaning::Open => open = true,
                    Meaning::None => {}
                }
            }
            match (conds.is_empty(), open) {
                (false, _) => Meaning::Cond(all(conds)),
                (true, true) => Meaning::Open,
                (true, false) => Meaning::None,
            }
        }

        fn dep(&mut self, d: &Dep) -> Meaning {
            let p = self.p;
            if d.kind == Kind::Annotation {
                return match p.resolve(d.file, &d.target) {
                    Some(Res::Def(f, n)) => match p.def(f, &n) {
                        Some(Def::Alias(deps)) => self.deps(deps),
                        _ => Meaning::None,
                    },
                    _ => Meaning::None,
                };
            }
            let name = d.target.rsplit('.').next().unwrap_or("").to_string();
            if d.target.is_empty() {
                return Meaning::None;
            }
            if let Some(m) = self.access.get(&name) {
                return self.listed(d, &name, m);
            }
            let base = self.unlisted(d, &name);
            if d.kind == Kind::Depends {
                return base;
            }
            let mut conds = vec![match base {
                Meaning::Cond(c) => c,
                _ => Condition::Custom(name.clone()),
            }];
            match &d.scopes {
                Some(scopes) => conds.extend(scopes.iter().map(|s| Condition::Role(s.clone()))),
                None => conds.push(self.unknown(d, &name, "its scopes are not a literal list")),
            }
            Meaning::Cond(all(conds))
        }

        fn unlisted(&mut self, d: &Dep, name: &str) -> Meaning {
            let p = self.p;
            if d.call {
                let message = format!(
                    "Dependency factory `{name}(…)` is not followed; whether it checks access is \
                     unknown. Name it in [views.access]."
                );
                self.at(d, message, DiagnosticSeverity::Note);
                return Meaning::None;
            }
            match p.resolve(d.file, &d.target) {
                Some(Res::Def(f, n)) => match p.def(f, &n) {
                    Some(Def::Func(_)) => self.func(f, &n),
                    Some(Def::Scheme { auto_error: true }) => {
                        Meaning::Cond(Condition::Authenticated)
                    }
                    Some(Def::Scheme { .. }) => Meaning::Cond(Condition::Custom(name.into())),
                    _ => Meaning::None,
                },
                Some(_) => Meaning::None,
                None => {
                    let message = format!(
                        "Dependency `{}` not found in the project; whether it checks access is \
                         unknown. Name it in [views.access].",
                        d.target
                    );
                    self.at(d, message, DiagnosticSeverity::Note);
                    Meaning::None
                }
            }
        }

        /// A project function: `custom(name)` plus its own access dependencies.
        fn func(&mut self, file: usize, name: &str) -> Meaning {
            let key = (file, name.to_string());
            if let Some(m) = self.memo.get(&key) {
                return m.clone();
            }
            self.memo.insert(key.clone(), Meaning::None);
            let Some(Def::Func(deps)) = self.p.def(file, name) else {
                return Meaning::None;
            };
            let m = match self.deps(deps) {
                Meaning::Cond(c) => Meaning::Cond(all(vec![c, Condition::Custom(name.into())])),
                _ => Meaning::None,
            };
            self.memo.insert(key, m.clone());
            m
        }

        fn listed(&mut self, d: &Dep, name: &str, meaning: &AccessMeaning) -> Meaning {
            use AccessMeaning as M;
            let args = if d.call { &d.args } else { &d.scopes };
            let has_arguments = args.as_ref().is_none_or(|a| !a.is_empty());
            let atom: fn(String) -> Condition = match meaning.at_use_site(has_arguments) {
                M::Open => return Meaning::Open,
                M::Authenticated => return Meaning::Cond(Condition::Authenticated),
                M::Never => return Meaning::Cond(Condition::Never),
                M::Role(n) => return Meaning::Cond(Condition::Role(n.clone())),
                M::Policy(n) => return Meaning::Cond(Condition::Policy(n.clone())),
                M::RoleFromArguments => Condition::Role,
                M::PolicyFromArguments => Condition::Policy,
                M::FromArgumentsOr(..) => return Meaning::Cond(Condition::Unknown(name.into())),
            };
            Meaning::Cond(match args {
                Some(a) if !a.is_empty() => {
                    let mut any: Vec<Condition> = a.iter().cloned().map(atom).collect();
                    if any.len() == 1 {
                        any.remove(0)
                    } else {
                        Condition::Any(any)
                    }
                }
                _ => self.unknown(d, name, "its arguments are not literals"),
            })
        }

        fn unknown(&mut self, d: &Dep, name: &str, why: &str) -> Condition {
            let message = format!("Access of dependency `{name}` not read: {why}.");
            self.at(d, message, DiagnosticSeverity::Gap);
            Condition::Unknown(name.into())
        }

        fn at(&mut self, d: &Dep, message: String, severity: DiagnosticSeverity) {
            let file = &self.p.files[d.file];
            let (file, line) = (file.rel.clone(), file.src.line(d.at));
            self.diags.push(UiFlowDiagnostic {
                file,
                line,
                message,
                severity,
            });
        }
    }
}

type Key = (usize, String);
/// An include: its file, the call, and the included router when found.
type Edge<'a> = (usize, &'a model::Include, Option<Key>);

/// The routers of a run, walked from every `FastAPI` app down.
struct Tree<'a> {
    p: &'a Project,
    eval: access::Eval<'a>,
    routes: HashMap<Key, Vec<(usize, &'a model::Route)>>,
    includes: HashMap<Key, Vec<Edge<'a>>>,
    children: HashSet<Key>,
    reached: HashSet<Key>,
    endpoints: Vec<EndpointAccess>,
}

impl<'a> Tree<'a> {
    fn new(p: &'a Project, access: &'a BTreeMap<String, AccessMeaning>, flow: &mut UiFlow) -> Self {
        let mut tree = Tree {
            p,
            eval: access::Eval::new(p, access),
            routes: HashMap::new(),
            includes: HashMap::new(),
            children: HashSet::new(),
            reached: HashSet::new(),
            endpoints: Vec::new(),
        };
        for (fi, file) in p.files.iter().enumerate() {
            let rel = &file.rel;
            for (at, text) in &file.facts.unread {
                let message =
                    format!("`methods={text}` is not a literal list; this route is skipped.");
                gap(&mut flow.diagnostics, rel, file.src.line(*at), message);
            }
            for r in &file.facts.routes {
                match tree.app(fi, &r.router) {
                    Some(k) => tree.routes.entry(k).or_default().push((fi, r)),
                    None if p.resolve(fi, &r.router).is_none() => {
                        let message = format!(
                            "`@{}.…` is not traced to a FastAPI app or APIRouter of the project; \
                             this route is skipped.",
                            r.router
                        );
                        note(&mut flow.diagnostics, rel, file.src.line(r.at), message);
                    }
                    None => {}
                }
            }
            for inc in &file.facts.includes {
                let Some(parent) = tree.app(fi, &inc.parent) else {
                    continue;
                };
                let child = inc.child.as_ref().and_then(|c| tree.app(fi, c));
                tree.children.extend(child.clone());
                tree.includes
                    .entry(parent)
                    .or_default()
                    .push((fi, inc, child));
            }
        }
        tree
    }

    /// The `FastAPI`/`APIRouter` that `name` in `file` refers to.
    fn app(&self, file: usize, name: &str) -> Option<Key> {
        match self.p.resolve(file, name)? {
            Res::Def(f, n) if matches!(self.p.def(f, &n), Some(Def::App { .. })) => Some((f, n)),
            _ => None,
        }
    }

    fn apps(&self, router: bool) -> Vec<Key> {
        let mut keys: Vec<Key> = (self.p.files.iter().enumerate())
            .flat_map(|(f, file)| file.facts.defs.iter().map(move |(n, d)| (f, n, d)))
            .filter(|(_, _, d)| matches!(d, Def::App { router: r, .. } if *r == router))
            .map(|(f, n, _)| (f, n.clone()))
            .collect();
        keys.sort();
        keys
    }

    fn emit_all(&mut self) {
        for key in self.apps(false) {
            self.emit(&key, String::new(), Vec::new(), &mut vec![key.clone()]);
        }
        for key in self.apps(true) {
            let used = self.routes.contains_key(&key) || self.includes.contains_key(&key);
            if self.reached.contains(&key) || self.children.contains(&key) || !used {
                continue;
            }
            let message = format!(
                "Router `{}` is not included in a FastAPI app of the project; its paths start at \
                 its own prefix.",
                key.1
            );
            self.gap_at(&key, message);
            self.emit(&key, String::new(), Vec::new(), &mut vec![key.clone()]);
        }
    }

    fn gap_at(&mut self, key: &Key, message: String) {
        let file = &self.p.files[key.0];
        let line = match self.p.def(key.0, &key.1) {
            Some(Def::App { at, .. }) => file.src.line(*at),
            _ => 0,
        };
        gap(&mut self.eval.diags, &file.rel, line, message);
    }

    fn emit(&mut self, key: &Key, prefix: String, mut deps: Vec<&'a Dep>, stack: &mut Vec<Key>) {
        self.reached.insert(key.clone());
        let p = self.p;
        let Some(Def::App {
            prefix: own,
            deps: own_deps,
            ..
        }) = p.def(key.0, &key.1)
        else {
            return;
        };
        let own = match own.as_ref().map(|v| p.value(key.0, v)) {
            None => String::new(),
            Some(Ok(s)) => s,
            Some(Err(t)) => {
                let message = format!(
                    "Prefix `{t}` of `{}` is not a readable literal; its endpoints are skipped.",
                    key.1
                );
                return self.gap_at(key, message);
            }
        };
        let prefix = prefix + &own;
        deps.extend(own_deps);
        for (f, route) in self.routes.get(key).cloned().unwrap_or_default() {
            self.endpoint(f, route, &prefix, &deps);
        }
        for (f, inc, child) in self.includes.get(key).cloned().unwrap_or_default() {
            let file = &p.files[f];
            let line = file.src.line(inc.at);
            let Some(child) = child else {
                let message =
                    "Included router not found in the project; its endpoints are unknown.";
                gap(&mut self.eval.diags, &file.rel, line, message);
                continue;
            };
            let inc_prefix = match inc.prefix.as_ref().map(|v| p.value(f, v)) {
                None => String::new(),
                Some(Ok(s)) => s,
                Some(Err(t)) => {
                    let message = format!(
                        "Include prefix `{t}` is not a readable literal; the included endpoints are skipped."
                    );
                    gap(&mut self.eval.diags, &file.rel, line, message);
                    continue;
                }
            };
            if stack.contains(&child) {
                continue;
            }
            stack.push(child.clone());
            let deps = deps.iter().copied().chain(&inc.deps).collect();
            self.emit(&child, format!("{prefix}{inc_prefix}"), deps, stack);
            stack.pop();
        }
    }

    fn endpoint(&mut self, f: usize, route: &'a model::Route, prefix: &str, deps: &[&'a Dep]) {
        let file = &self.p.files[f];
        let line = file.src.line(route.at);
        let path = match route.path.as_ref().map(|v| self.p.value(f, v)) {
            Some(Ok(path)) => path,
            other => {
                let text = other.and_then(Result::err).unwrap_or_default();
                let message = format!(
                    "Route path `{text}` is not a readable literal; this endpoint is skipped."
                );
                return gap(&mut self.eval.diags, &file.rel, line, message);
            }
        };
        let access = match self.eval.deps(deps.iter().copied().chain(&route.deps)) {
            access::Meaning::Cond(c) => AccessState::Protected(c),
            access::Meaning::Open => AccessState::Open,
            access::Meaning::None => AccessState::Undeclared,
        };
        for method in &route.methods {
            self.endpoints.push(EndpointAccess {
                method: method.clone(),
                path: format!("{prefix}{path}"),
                access: access.clone(),
                application: file.application.clone(),
                file: file.rel.clone(),
                line,
                source: "fastapi".into(),
                handler: Some(format!("{}.{}", self.p.module(f), route.func)),
                sends: Vec::new(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::{AccessState, Condition, Config, DiagnosticSeverity};

    fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, text) in files {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        dir
    }

    fn run(files: &[(&str, &str)], access: &str) -> UiFlow {
        let dir = project(files);
        let config: Config = toml::from_str(&format!("[views.access]\n{access}")).unwrap();
        extract_with(&ExtractContext::new(dir.path(), &config))
    }

    fn eps(flow: &UiFlow) -> Vec<(String, String, AccessState)> {
        (flow.endpoints.iter())
            .map(|e| (e.method.clone(), e.path.clone(), e.access.clone()))
            .collect()
    }

    fn ep(method: &str, path: &str, access: AccessState) -> (String, String, AccessState) {
        (method.into(), path.into(), access)
    }

    fn custom(n: &str) -> Condition {
        Condition::Custom(n.into())
    }

    fn role(n: &str) -> Condition {
        Condition::Role(n.into())
    }

    fn all(items: Vec<Condition>) -> AccessState {
        AccessState::Protected(Condition::All(items))
    }

    fn gaps(flow: &UiFlow) -> Vec<(String, u32, DiagnosticSeverity)> {
        (flow.diagnostics.iter())
            .map(|d| (d.file.clone(), d.line, d.severity))
            .collect()
    }

    /// The shape of the full-stack FastAPI template.
    const TEMPLATE: &[(&str, &str)] = &[
        ("backend/pyproject.toml", ""),
        ("backend/app/__init__.py", ""),
        ("backend/app/core/__init__.py", ""),
        (
            "backend/app/core/config.py",
            "from pydantic_settings import BaseSettings\n\n\
             class Settings(BaseSettings):\n    class Config:\n        API_V1_STR = \"/nested\"\n\
             \x20   API_V1_STR: str = \"/api/v1\"\n\nsettings = Settings()  # type: ignore\n",
        ),
        ("backend/app/api/__init__.py", ""),
        ("backend/app/api/routes/__init__.py", ""),
        (
            "backend/app/api/deps.py",
            "from typing import Annotated\nfrom fastapi import Depends, HTTPException\n\
             from fastapi.security import OAuth2PasswordBearer\nfrom sqlmodel import Session\n\n\
             reusable_oauth2 = OAuth2PasswordBearer(\n    tokenUrl=\"/login\"\n)\n\n\
             def get_db():\n    yield Session()\n\n\
             SessionDep = Annotated[Session, Depends(get_db)]\n\
             TokenDep = Annotated[str, Depends(reusable_oauth2)]\n\n\
             def get_current_user(session: SessionDep, token: TokenDep) -> User:\n\
             \x20   return session.get(User, token)\n\n\
             CurrentUser = Annotated[User, Depends(get_current_user)]\n\n\
             def get_current_active_superuser(current_user: CurrentUser) -> User:\n\
             \x20   if not current_user.is_superuser:\n        raise HTTPException(status_code=403)\n\
             \x20   return current_user\n",
        ),
        (
            "backend/app/api/routes/items.py",
            "from fastapi import APIRouter\nfrom app.api.deps import CurrentUser, SessionDep\n\n\
             router = APIRouter(prefix=\"/items\", tags=[\"items\"])\n\n\
             @router.get(\"/\")\ndef read_items(session: SessionDep, current_user: CurrentUser):\n\
             \x20   \"\"\"Docstring with @router.post(\"/fake\") inside.\"\"\"\n    return []\n\n\
             # @router.delete(\"/commented\")\n@router.get(\n    \"/{id}\",\n)\n\
             async def read_item(id: int, current_user: CurrentUser):\n    return None\n",
        ),
        (
            "backend/app/api/routes/users.py",
            "from fastapi import APIRouter, Depends\n\
             from app.api.deps import SessionDep, get_current_active_superuser\n\n\
             router = APIRouter(prefix=\"/users\")\n\n\
             @router.get(\"/\", dependencies=[Depends(get_current_active_superuser)])\n\
             def read_users(session: SessionDep):\n    return []\n\n\
             @router.post(\"/signup\")\ndef register_user(session: SessionDep):\n    return None\n",
        ),
        (
            "backend/app/api/main.py",
            "from fastapi import APIRouter\nfrom app.api.routes import items, users\n\n\
             api_router = APIRouter()\napi_router.include_router(items.router)\n\
             api_router.include_router(users.router)\n",
        ),
        (
            "backend/app/main.py",
            "from fastapi import FastAPI\nfrom app.api.main import api_router\n\
             from app.core.config import settings\n\napp = FastAPI(title=\"x\")\n\
             app.include_router(api_router, prefix=settings.API_V1_STR)\n",
        ),
        (
            "backend/tests/test_items.py",
            "from fastapi import FastAPI\nt = FastAPI()\n@t.get('/t')\ndef t(): ...\n",
        ),
    ];

    #[test]
    fn template_prefixes_compose_across_imports_and_settings() {
        let flow = run(TEMPLATE, "");
        let user = || all(vec![Condition::Authenticated, custom("get_current_user")]);
        let admin = all(vec![
            Condition::Authenticated,
            custom("get_current_active_superuser"),
            custom("get_current_user"),
        ]);
        assert_eq!(
            eps(&flow),
            vec![
                ep("GET", "/api/v1/items/", user()),
                ep("GET", "/api/v1/items/{id}", user()),
                ep("GET", "/api/v1/users/", admin),
                ep("POST", "/api/v1/users/signup", AccessState::Undeclared),
            ]
        );
        let first = &flow.endpoints[0];
        assert_eq!(
            first.handler.as_deref(),
            Some("app.api.routes.items.read_items")
        );
        assert_eq!(first.application.as_deref(), Some("backend"));
        assert_eq!(
            (first.file.as_str(), first.line),
            ("backend/app/api/routes/items.py", 6)
        );
        assert_eq!(first.source, "fastapi");
        assert_eq!(flow.endpoints[1].line, 12);
        assert!(flow.diagnostics.is_empty(), "{:?}", flow.diagnostics);
        assert!(flow.complete);
    }

    #[test]
    fn views_access_names_the_dependencies() {
        let flow = run(
            TEMPLATE,
            "get_current_user = \"authenticated\"\nget_current_active_superuser = \"role:superuser\"\n",
        );
        let got: Vec<_> = eps(&flow).into_iter().map(|e| e.2).collect();
        let signed_in = AccessState::Protected(Condition::Authenticated);
        let superuser = AccessState::Protected(role("superuser"));
        assert_eq!(
            got,
            vec![
                signed_in.clone(),
                signed_in,
                superuser,
                AccessState::Undeclared
            ]
        );
    }

    const LAYERS: &[(&str, &str)] = &[
        ("svc/__init__.py", ""),
        (
            "svc/auth.py",
            "from fastapi import Depends, Security\nfrom fastapi.security import APIKeyHeader\n\
             api_key = APIKeyHeader(name=\"x\", auto_error=False)\n\
             def verify_key(key: str = Security(api_key)):\n    return key\n\
             def admin(key = Depends(verify_key)):\n    pass\n\
             def open_dep():\n    pass\n",
        ),
        (
            "svc/admin_routes.py",
            "from typing import Annotated\nfrom fastapi import APIRouter, Depends\n\
             from .auth import admin\nrouter = APIRouter(prefix='/admin')\n\
             @router.get('')\ndef adm(user: Annotated[User, Depends(admin)]):\n    pass\n",
        ),
        (
            "svc/main.py",
            "from fastapi import FastAPI, APIRouter, Depends\n\
             from .auth import verify_key, admin, open_dep\nfrom . import admin_routes as ar\n\
             V2 = '/v2'\n\
             app = FastAPI(dependencies=[Depends(verify_key)])\n\
             router = APIRouter(prefix='/r', dependencies=[Depends(admin)])\n\
             @app.api_route('/any', methods=['GET', 'post'])\ndef any_route():\n    pass\n\
             @app.api_route('/default')\ndef default_route():\n    pass\n\
             @router.delete('/x/{id}')\nasync def delete_x(id: int, user=Depends(open_dep)):\n    pass\n\
             app.include_router(router, prefix=V2, dependencies=[Depends(open_dep)])\n\
             app.include_router(ar.router)\n",
        ),
        (
            "svc/public.py",
            "from fastapi import FastAPI, Depends\nfrom svc.auth import open_dep\npublic = FastAPI()\n\
             @public.get('/health', dependencies=[Depends(open_dep)])\ndef health():\n    pass\n\
             @public.get('/none')\ndef none():\n    pass\n",
        ),
    ];

    #[test]
    fn app_router_include_and_endpoint_dependencies_all_hold() {
        let flow = run(LAYERS, "open_dep = \"open\"\n");
        let key = || vec![custom("api_key"), custom("verify_key")];
        let with_admin = || {
            all(vec![
                custom("admin"),
                custom("api_key"),
                custom("verify_key"),
            ])
        };
        assert_eq!(
            eps(&flow),
            vec![
                ep("DELETE", "/v2/r/x/{id}", with_admin()),
                ep("GET", "/admin", with_admin()),
                ep("GET", "/any", all(key())),
                ep("GET", "/default", all(key())),
                ep("GET", "/health", AccessState::Open),
                ep("GET", "/none", AccessState::Undeclared),
                ep("POST", "/any", all(key())),
            ]
        );
        let adm = flow.endpoints.iter().find(|e| e.path == "/admin").unwrap();
        assert_eq!(adm.handler.as_deref(), Some("svc.admin_routes.adm"));
        assert_eq!(adm.application, None);
        assert!(flow.diagnostics.is_empty(), "{:?}", flow.diagnostics);
    }

    const SCOPES: &[(&str, &str)] = &[(
        "main.py",
        "from fastapi import FastAPI, Depends, Security\n\
         from fastapi.security import OAuth2PasswordBearer\nfrom deps import make_dep\n\
         oauth2 = OAuth2PasswordBearer(tokenUrl=\"token\")\n\
         def current_user(token: str = Depends(oauth2)):\n    pass\n\
         def require(*roles):\n    pass\napp = FastAPI()\n\
         @app.get(\"/me\")\ndef me(u = Security(current_user, scopes=[\"me\", \"items\"])):\n    pass\n\
         @app.get(\"/admin\")\ndef adm(u = Depends(require(\"admin\", \"root\"))):\n    pass\n\
         @app.get(\"/factory\")\ndef f(u = Depends(make_dep(\"x\"))):\n    pass\n",
    )];

    #[test]
    fn security_scopes_are_required_roles_and_factories_need_a_listing() {
        let flow = run(SCOPES, "");
        let me = all(vec![
            Condition::Authenticated,
            role("items"),
            role("me"),
            custom("current_user"),
        ]);
        assert_eq!(
            eps(&flow),
            vec![
                ep("GET", "/admin", AccessState::Undeclared),
                ep("GET", "/factory", AccessState::Undeclared),
                ep("GET", "/me", me),
            ]
        );
        let notes = vec![
            ("main.py".to_string(), 14, DiagnosticSeverity::Note),
            ("main.py".to_string(), 17, DiagnosticSeverity::Note),
        ];
        assert_eq!(gaps(&flow), notes);
        assert!(flow.complete);

        let flow = run(
            SCOPES,
            "require = \"role:$arg\"\ncurrent_user = \"role:$arg\"\n",
        );
        let any = |a: &str, b: &str| AccessState::Protected(Condition::Any(vec![role(a), role(b)]));
        let got: Vec<_> = eps(&flow).into_iter().map(|e| e.2).collect();
        assert_eq!(
            got,
            vec![
                any("admin", "root"),
                AccessState::Undeclared,
                any("me", "items")
            ]
        );
    }

    #[test]
    fn unreadable_paths_prefixes_and_routers_are_gaps() {
        let flow = run(
            &[(
                "app.py",
                "from fastapi import FastAPI, APIRouter, Depends\n\
                 from somewhere import PREFIX, external_guard\napp = FastAPI()\n\
                 VERSION = 'v3'\nlonely = APIRouter(prefix='/lonely')\ninner = APIRouter()\n\
                 @lonely.get('/x')\ndef lx():\n    pass\n\
                 @app.get(f'/{VERSION}/f')\ndef fpath():\n    pass\n\
                 @app.get('/guarded', dependencies=[Depends(external_guard)])\ndef g():\n    pass\n\
                 app.include_router(inner, prefix=PREFIX)\n\
                 @inner.get('/i')\ndef inner_i():\n    pass\n\
                 app.include_router(missing_router)\n",
            )],
            "",
        );
        assert_eq!(
            eps(&flow),
            vec![
                ep("GET", "/guarded", AccessState::Undeclared),
                ep("GET", "/lonely/x", AccessState::Undeclared),
            ]
        );
        let at = |line, severity| ("app.py".to_string(), line, severity);
        assert_eq!(
            gaps(&flow),
            vec![
                at(5, DiagnosticSeverity::Gap),
                at(10, DiagnosticSeverity::Gap),
                at(13, DiagnosticSeverity::Note),
                at(16, DiagnosticSeverity::Gap),
                at(20, DiagnosticSeverity::Gap),
            ]
        );
        assert!(!flow.complete);
    }

    #[test]
    fn python_without_fastapi_gives_nothing() {
        let flow = run(&[("manage.py", "import django\nx = APIRouter\n")], "");
        assert!(flow.endpoints.is_empty() && flow.diagnostics.is_empty());
    }

    #[test]
    fn lexer_blanks_comments_and_reads_literals() {
        let text =
            "a = 'x#y' # c'\nb = (r'\\d' \"z\"\n  f'{q}')\nc = \"\"\"t\n'\"\"\"\nd = f'{{k}}'\n";
        let src = py::Src::new(text);
        let blanked = String::from_utf8(src.b.clone()).unwrap();
        assert_eq!(blanked.lines().next().map(str::trim_end), Some("a = '   '"));
        let lit = |needle: &str, len: usize| {
            let a = text.find(needle).unwrap();
            src.literal(a, a + len)
        };
        assert_eq!(lit("'x#y'", 5), Some("x#y".into()));
        assert_eq!(lit("r'\\d'", 9), Some("\\dz".into()));
        assert_eq!(lit("f'{q}'", 6), None);
        assert_eq!(lit("\"\"\"t", 9), Some("t\n'".into()));
        assert_eq!(lit("f'{{k}}'", 8), Some("{k}".into()));
        let starts: Vec<_> = src
            .statements()
            .into_iter()
            .map(|(at, _)| text.as_bytes()[at])
            .collect();
        assert_eq!(starts, b"abcd".to_vec());
        assert_eq!(src.line(text.find("f'{q}'").unwrap()), 3);
    }
}
