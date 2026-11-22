//! Phoenix routes and their access for ui-flow/v1 (ADR 0021, LOGIC.md 3.19).
//!
//! A line and statement level scan of the `.ex` files below the root (no
//! Elixir parser); only for a router module that does `use Phoenix.Router`
//! or `use …, :router`. It fills `endpoints`, `screens` (LiveViews) and
//! `diagnostics`.
//!
//! Routes: `get/post/put/patch/delete/head/options/match`, `resources`
//! (`only:`/`except:`/`singleton:`, nested `do` blocks), `forward`, `live`.
//! `scope` prefixes and module aliases accumulate; `pipe_through` covers only
//! the routes AFTER it in its scope (and nested scopes entered after it).
//!
//! Access = AND of: the endpoint plugs before the router plug, the
//! `pipe_through` pipelines (a name that is no pipeline is a function plug),
//! the controller's `plug … when action in [..]` and a Bodyguard permit as the
//! first statement of the action. A plug is read by (1) `[views.access]`,
//! (2) the libraries Guardian (`EnsureAuthenticated`, `EnsureNotAuthenticated`,
//! loaders), `Bodyguard.Plug.Authorize`, `Plug.BasicAuth`, (3) its BODY in the
//! project: it only counts when it halts and reads the request's identity
//! (`current_scope`, `current_user`, `logged_in?`, `authorization`); an
//! `if <identity> … else … halt` is `Authenticated`, the mirror
//! `Not(Authenticated)`, any other reading `Custom(name)`. A plug that does
//! not halt is a loader; one that halts without reading an identity (rate
//! limit, read-only mode) is not access. A plug nothing explains is `Unknown`
//! plus a gap, never open. `Open` only from a `[views.access]` `open` and only
//! when nothing else guards.
//!
//! LiveViews: `on_mount` hooks of the `live_session`, of the module and of
//! `def live_view` in the web macro module (read from their
//! `def on_mount(:name, …)` clauses: `{:halt` after an identity check is a
//! gate). A gate in the hooks is the screen's access; a pipeline gate alone
//! inside a `live_session` is `weak` (live navigation skips the pipeline),
//! with a gap when sessions of the same name differ in their pipelines.
//! Not read: channel `connect/3`, `handle_event` checks, Ash policies, a
//! Bodyguard detail beyond the literal action, hosts (a `host:` scope is
//! noted), routes from macros.
use crate::extract_ctx::{ExtractContext, ancestors, cap, dir_name, finish, gap, read_text, walk};
use crate::services::support::test_path;
use archguard_core::{
    AccessMeaning, AccessState, Condition, DiagnosticSeverity, EndpointAccess, Screen, ScreenKind,
    UiFlow, UiFlowDiagnostic,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};

const MAX_FILES: usize = 20_000;
const SOURCE: &str = "phoenix";
/// What a plug body reads to be an access check; `logged_in?` joins them only
/// when the project defines it as a plain identity check.
const IDENTITY: [&str; 3] = ["current_scope", "current_user", "authorization"];
/// Libraries whose plugs only load, log, parse or limit; none is an access check.
const NON_ACCESS_LIBS: [&str; 9] = [
    "Guardian.Plug.",
    "Plug.",
    "Phoenix.",
    "Sentry.",
    "Corsica",
    "Tidewave",
    "Ueberauth",
    "PlugAttack",
    "Hammer",
];
const PASSIVE: [&str; 12] = [
    "accepts",
    "fetch_session",
    "fetch_flash",
    "fetch_live_flash",
    "protect_from_forgery",
    "put_secure_browser_headers",
    "put_root_layout",
    "put_layout",
    "put_flash",
    "put_format",
    "put_view",
    "put_new_layout",
];
const VERBS: [&str; 8] = [
    "get", "post", "put", "patch", "delete", "head", "options", "connect",
];

/// [`crate::ui_flow::extract_all`]'s entry: routes of every Phoenix router below the root.
pub fn extract_with(ctx: &ExtractContext) -> UiFlow {
    let mut flow = UiFlow::default();
    let walked = match walk(ctx.root) {
        Ok(w) => w,
        Err(e) => {
            let message = format!("Discovery exclusions not read; no Phoenix app analyzed: {e}");
            gap(&mut flow.diagnostics, "", 0, message);
            finish(&mut flow);
            return flow;
        }
    };
    let (mut paths, mut build_dirs) = (Vec::new(), BTreeSet::new());
    for rel in walked.files {
        let (dir, name) = rel.rsplit_once('/').unwrap_or(("", &rel));
        if name == "mix.exs" {
            build_dirs.insert(dir.to_string());
        }
        if rel.ends_with(".ex") && !test_path(&rel) {
            paths.push(rel);
        }
    }
    paths.sort();
    let cut = cap(&mut paths, MAX_FILES);
    let (mut files, mut unread) = (Vec::new(), Vec::new());
    for rel in paths {
        match read_text(ctx.root, &rel, 1 << 20) {
            Ok(text) => {
                let app = (ancestors(&rel))
                    .find(|d| build_dirs.contains(*d))
                    .map(|d| dir_name(ctx.root, d));
                files.push(File { rel, text, app });
            }
            Err(e) => unread.push((rel, format!("File not read ({e}); its routes are unknown."))),
        }
    }
    if !files.iter().any(|f| is_router(&f.text)) {
        return flow;
    }
    if let Some(first) = cut {
        let message =
            format!("Phoenix scan stopped after {MAX_FILES} files; later ones are unknown.");
        gap(&mut flow.diagnostics, &first, 0, message);
    }
    for (rel, message) in unread {
        gap(&mut flow.diagnostics, &rel, 0, message);
    }
    if let Some(e) = &ctx.config_error {
        let message = format!("Configuration not read; [views.access] not applied: {e}");
        note(&mut flow.diagnostics, ".", 0, message);
    }
    let project = Project::new(files, &ctx.config.views.access);
    project.emit(&mut flow);
    finish(&mut flow);
    flow
}

fn is_router(text: &str) -> bool {
    text.contains("use Phoenix.Router") || text.contains(", :router")
}

fn note(out: &mut Vec<UiFlowDiagnostic>, file: &str, line: u32, message: impl Into<String>) {
    out.push(UiFlowDiagnostic {
        file: file.into(),
        line,
        message: message.into(),
        severity: DiagnosticSeverity::Note,
    });
}

struct File {
    rel: String,
    text: String,
    app: Option<String>,
}

/// A `def`/`defp` with its head and body text.
#[derive(Clone)]
struct FnDef {
    name: String,
    head: String,
    body: String,
}

/// What one plug, hook or check does to access.
#[derive(Clone, PartialEq, Debug)]
enum Eff {
    Pass,
    Cond(Condition),
    Open,
}

/// The AND of layers: a condition (also an unknown one) wins over `Open`.
fn fold(effs: Vec<Eff>) -> AccessState {
    let conds: Vec<Condition> = (effs.iter())
        .filter_map(|e| match e {
            Eff::Cond(c) => Some(c.clone()),
            _ => None,
        })
        .collect();
    if !conds.is_empty() {
        AccessState::Protected(crate::ui_flow_fastapi::all(conds))
    } else if effs.contains(&Eff::Open) {
        AccessState::Open
    } else {
        AccessState::Undeclared
    }
}

struct Project<'a> {
    files: Vec<File>,
    /// Module name to file index.
    mods: HashMap<String, usize>,
    /// Every function by name.
    fns: HashMap<String, Vec<(usize, FnDef)>>,
    access: &'a BTreeMap<String, AccessMeaning>,
    /// The names a body reads to ask for an identity.
    ids: Vec<String>,
    diags: std::cell::RefCell<Vec<UiFlowDiagnostic>>,
}

struct Route {
    method: String,
    path: String,
    line: u32,
    /// Controller or LiveView module as resolved (scope alias applied).
    module: String,
    action: Option<String>,
    pipes: Vec<(String, u32)>,
    /// A LiveView route, with its `live_session` if it is in one.
    live: Option<Option<Session>>,
}

#[derive(Clone)]
struct Session {
    name: String,
    hooks: Vec<String>,
}

enum Frame {
    Scope {
        prefix: String,
        module: String,
        pipes: Vec<(String, u32)>,
    },
    Session(Session),
    Res(String),
    Pipeline(String),
    Other,
}

/// The state of one router scan.
#[derive(Default)]
struct Acc {
    stack: Vec<Frame>,
    pipelines: HashMap<String, Vec<(u32, String)>>,
    routes: Vec<Route>,
}

impl Acc {
    /// The statements that only touch the scan state; none for any other.
    fn simple(&mut self, word: &str, line: u32, rest: &str, a: &[String]) -> Option<bool> {
        Some(match word {
            "pipeline" => self.pipeline(a),
            "plug" => self.plug(line, rest),
            "pipe_through" => self.pipe_through(line, rest),
            "live_session" => self.live_session(a),
            "forward" => self.forward(line, a),
            _ => return None,
        })
    }

    fn pipeline(&mut self, a: &[String]) -> bool {
        let name = a
            .first()
            .map_or("", |n| n.trim_start_matches(':'))
            .to_string();
        self.pipelines.entry(name.clone()).or_default();
        self.stack.push(Frame::Pipeline(name));
        true
    }

    fn plug(&mut self, line: u32, rest: &str) -> bool {
        if let Some(Frame::Pipeline(n)) = self.stack.last() {
            self.pipelines
                .entry(n.clone())
                .or_default()
                .push((line, rest.into()));
        }
        false
    }

    fn pipe_through(&mut self, line: u32, rest: &str) -> bool {
        let names =
            (split_list(rest).into_iter()).map(|n| (n.trim_start_matches(':').to_string(), line));
        let scope = (self.stack.iter_mut()).rev().find_map(|f| match f {
            Frame::Scope { pipes, .. } => Some(pipes),
            _ => None,
        });
        if let Some(pipes) = scope {
            pipes.extend(names);
        }
        false
    }

    fn live_session(&mut self, a: &[String]) -> bool {
        let name = a
            .first()
            .map_or("", |n| n.trim_start_matches(':'))
            .to_string();
        let hooks = opt(a, "on_mount")
            .map(|v| split_list(&v))
            .unwrap_or_default();
        self.stack.push(Frame::Session(Session { name, hooks }));
        true
    }

    fn forward(&mut self, line: u32, a: &[String]) -> bool {
        if let Some(path) = a.first().and_then(|p| literal(p)) {
            let module = join_mod(&current_module(&self.stack), a.get(1).map_or("", |m| m));
            let path = format!("{path}/*");
            self.routes.push(route(
                line,
                &self.stack,
                ("ANY", &path),
                &module,
                None,
                false,
            ));
        }
        false
    }
}

impl<'a> Project<'a> {
    fn new(files: Vec<File>, access: &'a BTreeMap<String, AccessMeaning>) -> Self {
        let (mut mods, mut fns) = (
            HashMap::new(),
            HashMap::<String, Vec<(usize, FnDef)>>::new(),
        );
        for (i, f) in files.iter().enumerate() {
            for l in f.text.lines() {
                if let Some(rest) = l.trim().strip_prefix("defmodule ") {
                    mods.insert(rest.trim_end_matches(" do").trim().to_string(), i);
                }
            }
            for d in fn_defs(&f.text) {
                fns.entry(d.name.clone()).or_default().push((i, d));
            }
        }
        let mut ids: Vec<String> = IDENTITY.iter().map(|t| t.to_string()).collect();
        let defs = fns.get("logged_in?").map(Vec::as_slice).unwrap_or(&[]);
        if !defs.is_empty() && defs.iter().all(|(_, d)| plain_login_body(&d.body)) {
            ids.push("logged_in?".into());
        }
        Self {
            files,
            mods,
            fns,
            ids,
            access,
            diags: Default::default(),
        }
    }

    fn gap(&self, file: usize, line: u32, message: String) {
        let rel = &self.files[file].rel;
        gap(&mut self.diags.borrow_mut(), rel, line, message);
    }

    fn emit(&self, flow: &mut UiFlow) {
        let endpoint = self.endpoint_plugs();
        for (i, f) in self.files.iter().enumerate() {
            if is_router(&f.text) {
                self.router(i, &endpoint, flow);
            }
        }
        flow.diagnostics.extend(self.diags.borrow().iter().cloned());
    }

    /// The plugs of an endpoint module before its `plug …Router`.
    fn endpoint_plugs(&self) -> Vec<Eff> {
        let mut out = Vec::new();
        for (i, f) in self.files.iter().enumerate() {
            if !f.text.contains("use Phoenix.Endpoint") {
                continue;
            }
            for (line, s) in stmts(&f.text) {
                let Some(rest) = s.strip_prefix("plug ") else {
                    continue;
                };
                let name = split_args(rest).remove(0);
                if name.ends_with(".Router") {
                    break;
                }
                out.push(self.plug(&name, rest, (i, line), 0));
            }
        }
        out
    }

    fn router(&self, file: usize, endpoint: &[Eff], flow: &mut UiFlow) {
        let mut acc = Acc::default();
        for (line, s) in stmts(&self.files[file].text) {
            if s == "end" {
                acc.stack.pop();
                continue;
            }
            let (word, rest) = s.split_once(' ').unwrap_or((s.as_str(), ""));
            let opens = s.ends_with(" do");
            let rest = rest.strip_suffix(" do").unwrap_or(rest);
            let at = (file, line);
            let a = split_args(rest);
            let nested = match acc.simple(word, line, rest, &a) {
                Some(n) => n,
                None => match word {
                    "scope" => self.scope(at, &a, &mut acc),
                    "resources" => self.resources(line, &a, opens, &mut acc),
                    "live" => self.live(at, &a, &mut acc),
                    "match" => self.matched(at, &a, &mut acc),
                    w if VERBS.contains(&w) => {
                        self.plain(at, &w.to_uppercase(), &a, &mut acc);
                        false
                    }
                    _ => false,
                },
            };
            if opens && !nested {
                acc.stack.push(Frame::Other);
            }
        }
        self.output(file, acc.routes, &acc.pipelines, endpoint, flow);
    }

    /// `match :get, "/p", C, :a` (`:*` is any method).
    fn matched(&self, at: (usize, u32), a: &[String], acc: &mut Acc) -> bool {
        let m = a.first().map(|m| m.trim_start_matches(':').to_uppercase());
        let m = m.unwrap_or_default();
        let method = if m == "*" { "ANY" } else { &m };
        self.plain(at, method, a.get(1..).unwrap_or(&[]), acc);
        false
    }

    /// `scope`: pushes its frame; true as it always opens one.
    fn scope(&self, at: (usize, u32), a: &[String], acc: &mut Acc) -> bool {
        let path = a.first().and_then(|p| literal(p));
        if path.is_none() {
            let m = "A scope path is not a string literal; its routes are unknown.";
            self.gap(at.0, at.1, m.into());
        }
        let upper = |x: &&String| x.starts_with(|c: char| c.is_ascii_uppercase());
        let alias = (a.iter().skip(1)).find(upper).cloned();
        let alias = alias.or_else(|| opt(a, "alias")).unwrap_or_default();
        if opt(a, "host").is_some() {
            let m = "A host: scope is read like any scope; the host itself is not.";
            note(&mut self.diags.borrow_mut(), &self.files[at.0].rel, at.1, m);
        }
        acc.stack.push(Frame::Scope {
            prefix: path.unwrap_or_default(),
            module: join_mod(&current_module(&acc.stack), &alias),
            pipes: Vec::new(),
        });
        true
    }

    /// `resources`: its routes; true when a `do` block frame was pushed.
    fn resources(&self, line: u32, a: &[String], opens: bool, acc: &mut Acc) -> bool {
        let path = a.first().and_then(|p| literal(p)).unwrap_or_default();
        let module = join_mod(&current_module(&acc.stack), a.get(1).map_or("", |m| m));
        for (method, sub, action) in resource_routes(a) {
            let full = format!("{path}{sub}");
            let r = route(
                line,
                &acc.stack,
                (&method, &full),
                &module,
                Some(action),
                false,
            );
            acc.routes.push(r);
        }
        if opens {
            let seg = path.rsplit('/').next().unwrap_or("").trim_end_matches('s');
            acc.stack.push(Frame::Res(format!("{path}/:{seg}_id")));
        }
        opens
    }

    fn live(&self, at: (usize, u32), a: &[String], acc: &mut Acc) -> bool {
        let Some(path) = a.first().and_then(|p| literal(p)) else {
            let m = "A live route path is not a string literal; the screen is unknown.";
            self.gap(at.0, at.1, m.into());
            return false;
        };
        let module = join_mod(&current_module(&acc.stack), a.get(1).map_or("", |m| m));
        let r = route(
            at.1,
            &acc.stack,
            ("GET", &path),
            &module,
            atom(a.get(2)),
            true,
        );
        acc.routes.push(r);
        false
    }

    fn plain(&self, at: (usize, u32), method: &str, a: &[String], acc: &mut Acc) {
        let Some(path) = a.first().and_then(|p| literal(p)) else {
            let m = "A route path is not a string literal; the route is unknown.";
            self.gap(at.0, at.1, m.into());
            return;
        };
        let module = join_mod(&current_module(&acc.stack), a.get(1).map_or("", |m| m));
        let r = route(
            at.1,
            &acc.stack,
            (method, &path),
            &module,
            atom(a.get(2)),
            false,
        );
        acc.routes.push(r);
    }

    fn output(
        &self,
        file: usize,
        routes: Vec<Route>,
        pipelines: &HashMap<String, Vec<(u32, String)>>,
        endpoint: &[Eff],
        flow: &mut UiFlow,
    ) {
        let f = &self.files[file];
        let pipe_effs = |r: &Route| -> Vec<Eff> {
            let mut out = Vec::new();
            for (name, at) in &r.pipes {
                match pipelines.get(name) {
                    Some(plugs) => {
                        for (l, p) in plugs {
                            let n = split_args(p).remove(0);
                            out.push(self.plug(&n, p, (file, *l), 0));
                        }
                    }
                    None => {
                        let n = format!(":{name}");
                        out.push(self.plug(&n, &n, (file, *at), 0));
                    }
                }
            }
            out
        };
        // The pipeline states seen under each live_session name, to tell a bypass.
        let mut sessions: HashMap<String, BTreeSet<AccessState>> = HashMap::new();
        for r in &routes {
            if let Some(Some(s)) = &r.live {
                let state = fold(pipe_effs(r));
                sessions.entry(s.name.clone()).or_default().insert(state);
            }
        }
        for r in routes {
            let mut layers = endpoint.to_vec();
            layers.extend(pipe_effs(&r));
            let module_file = self.module_file(&r.module);
            if r.live.is_some() {
                Views(self).screen(&r, (file, module_file), layers, &sessions, flow);
                continue;
            }
            if let (Some(c), Some(action)) = (module_file, &r.action) {
                layers.extend(Controllers(self).layers(c, action));
            }
            let handler = format!("{}.{}", r.module, r.action.clone().unwrap_or_default());
            flow.endpoints.push(EndpointAccess {
                method: r.method,
                path: r.path,
                access: fold(layers),
                application: f.app.clone(),
                file: f.rel.clone(),
                line: r.line,
                source: SOURCE.into(),
                handler: r.action.map(|_| handler),
                sends: Vec::new(),
            });
        }
    }

    fn unknown(&self, name: &str, at: (usize, u32), why: &str) -> Eff {
        let message = format!(
            "`{name}` is not readable ({why}) and not listed in [views.access]; \
             its effect on access is unknown."
        );
        self.gap(at.0, at.1, message);
        Eff::Cond(Condition::Unknown(name.into()))
    }

    fn meaning(&self, m: &AccessMeaning, name: &str, at: (usize, u32)) -> Eff {
        match m.at_use_site(false) {
            AccessMeaning::Open => Eff::Open,
            AccessMeaning::Authenticated => Eff::Cond(Condition::Authenticated),
            AccessMeaning::Never => Eff::Cond(Condition::Never),
            AccessMeaning::Role(n) => Eff::Cond(Condition::Role(n.clone())),
            AccessMeaning::Policy(n) => Eff::Cond(Condition::Policy(n.clone())),
            _ => self.unknown(name, at, "its [views.access] meaning needs arguments"),
        }
    }

    /// `[views.access]` entry for the name as written or its last segment.
    fn listed(&self, name: &str) -> Option<&AccessMeaning> {
        let name = name.trim_start_matches(':');
        let last = name.rsplit('.').next().unwrap_or(name);
        self.access.get(name).or_else(|| self.access.get(last))
    }

    fn module_file(&self, name: &str) -> Option<usize> {
        let name = name.trim();
        if let Some(i) = self.mods.get(name) {
            return Some(*i);
        }
        let suffix = format!(".{name}");
        let hits: BTreeSet<usize> = (self.mods.iter())
            .filter(|(m, _)| m.ends_with(&suffix))
            .map(|(_, i)| *i)
            .collect();
        (hits.len() == 1).then(|| *hits.iter().next().unwrap())
    }

    /// One `plug NAME, …`: `name` is `:fun` or a module, `full` the statement after `plug `;
    /// functions are looked up in the file `at.0` first.
    fn plug(&self, name: &str, full: &str, at: (usize, u32), depth: u8) -> Eff {
        let name = name.trim();
        let short = name.trim_start_matches(':');
        if let Some(m) = self.listed(short) {
            return self.meaning(m, short, at);
        }
        if let Some(eff) = self.library(short, full, at) {
            return eff;
        }
        match name.starts_with(':') {
            true => self.plug_fn(short, at),
            false => self.plug_module(short, at, depth),
        }
    }

    /// The libraries whose plugs are read by name.
    fn library(&self, short: &str, full: &str, at: (usize, u32)) -> Option<Eff> {
        Some(match short {
            "Guardian.Plug.EnsureAuthenticated" => Eff::Cond(Condition::Authenticated),
            "Guardian.Plug.EnsureNotAuthenticated" => {
                Eff::Cond(Condition::Not(Box::new(Condition::Authenticated)))
            }
            "Plug.BasicAuth" => Eff::Cond(Condition::Custom("Plug.BasicAuth".into())),
            "Bodyguard.Plug.Authorize" => Controllers(self).bodyguard_plug(full, at),
            "Guardian.Permissions" | "Guardian.Permissions.Bitwise" => {
                self.unknown(short, at, "Guardian permissions are not mapped")
            }
            _ if NON_ACCESS_LIBS.iter().any(|p| short.starts_with(p)) => Eff::Pass,
            _ => return None,
        })
    }

    fn plug_fn(&self, short: &str, at: (usize, u32)) -> Eff {
        let cands = self.fns.get(short).map(Vec::as_slice).unwrap_or(&[]);
        let local: Vec<&FnDef> = (cands.iter())
            .filter(|(i, _)| *i == at.0)
            .map(|(_, d)| d)
            .collect();
        let chosen: Vec<&FnDef> = match local.is_empty() {
            true => cands.iter().map(|(_, d)| d).collect(),
            false => local,
        };
        if chosen.is_empty() {
            let known = PASSIVE.contains(&short);
            return match known {
                true => Eff::Pass,
                false => self.unknown(short, at, "no such function in the project"),
            };
        }
        let effs: BTreeSet<String> = (chosen.iter())
            .map(|d| format!("{:?}", gate(&d.body, "halt", short, &self.ids)))
            .collect();
        match effs.len() {
            1 => self.read(&chosen[0].body, "halt", short, at),
            _ => self.unknown(short, at, "several functions of that name read differently"),
        }
    }

    /// [`gate`] of a body; a halt that reads no identity is a gap, never "not access".
    fn read(&self, body: &str, halt: &str, name: &str, at: (usize, u32)) -> Eff {
        let eff = gate(body, halt, name, &self.ids);
        if matches!(&eff, Eff::Cond(Condition::Unknown(_))) {
            return self.unknown(name, at, "it halts without reading an identity");
        }
        eff
    }

    /// A module plug: its `call`, or the plugs of a `use Plug.Builder` module.
    fn plug_module(&self, short: &str, at: (usize, u32), depth: u8) -> Eff {
        let Some(m) = self.module_file(short) else {
            return self.unknown(short, at, "plug module not found in the project");
        };
        let text = &self.files[m].text;
        if let Some(c) = fn_defs(text).into_iter().find(|d| d.name == "call") {
            return self.read(&c.body, "halt", short, at);
        }
        if text.contains("use PlugAttack") {
            return Eff::Pass;
        }
        let inner: Vec<Eff> = (stmts(text).into_iter())
            .filter(|_| depth < 3)
            .filter_map(|(l, s)| {
                let rest = s.strip_prefix("plug ")?.to_string();
                let n = split_args(&rest).remove(0);
                Some(self.plug(&n, &rest, (m, l), depth + 1))
            })
            .collect();
        match (inner.is_empty(), fold(inner)) {
            (false, AccessState::Protected(c)) => Eff::Cond(c),
            (false, _) => Eff::Pass,
            (true, _) => self.unknown(short, at, "plug module has no call/2"),
        }
    }
}

/// LiveView screens: `on_mount` hooks and the pipeline.
struct Views<'p, 'a>(&'p Project<'a>);

impl Views<'_, '_> {
    fn screen(
        &self,
        r: &Route,
        (file, module_file): (usize, Option<usize>),
        http: Vec<Eff>,
        sessions: &HashMap<String, BTreeSet<AccessState>>,
        flow: &mut UiFlow,
    ) {
        let p = self.0;
        let f = &p.files[file];
        let session = r.live.clone().flatten();
        let mut hooks: Vec<Eff> = Vec::new();
        for h in session.iter().flat_map(|s| &s.hooks) {
            hooks.push(self.hook(h, (file, r.line)));
        }
        if let Some(m) = module_file {
            hooks.extend(self.module_hooks(m, &p.files[m].text));
        }
        if let Some((w, text)) = web_live_view(&p.files) {
            hooks.extend(self.module_hooks(w, &text));
        }
        let (live, http) = (fold(hooks), fold(http));
        let (access, weak) = match (&live, &http, &session) {
            (AccessState::Undeclared, AccessState::Protected(_), Some(s)) => {
                let message = format!(
                    "Protected by the pipeline only: live navigation inside live_session :{} \
                     is not guarded by it (no on_mount gate).",
                    s.name
                );
                if sessions.get(&s.name).is_some_and(|x| x.len() > 1) {
                    p.gap(file, r.line, message);
                } else {
                    note(&mut p.diags.borrow_mut(), &f.rel, r.line, message);
                }
                (http, true)
            }
            (AccessState::Undeclared, ..) => (http, false),
            _ => (fold(vec![as_eff(live), as_eff(http)]), false),
        };
        let app = f.app.clone().unwrap_or_default();
        flow.screens.push(Screen {
            id: format!("{app}:{}", r.path),
            name: r.module.rsplit('.').next().unwrap_or(&r.module).into(),
            kind: ScreenKind::Page,
            route: Some(r.path.clone()),
            application: f.app.clone(),
            file: f.rel.clone(),
            line: r.line,
            access,
            weak,
            source: SOURCE.into(),
        });
    }

    /// `on_mount` lines of a LiveView module (or of the web macro's `live_view` block).
    fn module_hooks(&self, file: usize, text: &str) -> Vec<Eff> {
        (stmts(text).into_iter())
            .filter_map(|(line, s)| Some(self.hook(s.strip_prefix("on_mount ")?, (file, line))))
            .collect()
    }

    /// One `on_mount` entry: `Mod`, `{Mod, :name}`.
    fn hook(&self, spec: &str, at: (usize, u32)) -> Eff {
        let p = self.0;
        let spec = spec.trim();
        let (module, name) = match spec.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
            Some(inner) => {
                let a = split_args(inner);
                (a.first().cloned().unwrap_or_default(), atom(a.get(1)))
            }
            None => (spec.to_string(), None),
        };
        let name = name.unwrap_or_else(|| "default".into());
        let label = format!("{module}:{name}");
        if let Some(m) = p.listed(&label).or_else(|| p.listed(&module)) {
            return p.meaning(m, &label, at);
        }
        let Some(m) = p.module_file(&module) else {
            return p.unknown(&label, at, "on_mount hook module not found in the project");
        };
        let of_module = || (p.fns.get("on_mount").into_iter().flatten()).filter(|(i, _)| *i == m);
        let mut clauses: Vec<&FnDef> = (of_module())
            .filter(|(_, d)| d.head.contains(&format!(":{name}")))
            .map(|(_, d)| d)
            .collect();
        if clauses.is_empty() && name == "default" {
            clauses = of_module().map(|(_, d)| d).collect();
        }
        if clauses.is_empty() {
            return p.unknown(&label, at, "no matching on_mount clause");
        }
        (clauses.iter())
            .map(|d| p.read(&d.body, "{:halt", &label, at))
            .find(|e| *e != Eff::Pass)
            .unwrap_or(Eff::Pass)
    }
}

fn as_eff(a: AccessState) -> Eff {
    match a {
        AccessState::Protected(c) => Eff::Cond(c),
        AccessState::Open => Eff::Open,
        AccessState::Undeclared => Eff::Pass,
    }
}

/// The `def live_view` block of the web macro module: from its head to the next `def`.
fn web_live_view(files: &[File]) -> Option<(usize, String)> {
    let i = (files.iter()).position(|f| {
        f.text.contains("def live_view") && f.text.contains("use Phoenix.LiveView")
    })?;
    let t = &files[i].text;
    let rest = &t[t.find("def live_view")?..];
    let end = rest[1..].find("\n  def ").map_or(rest.len(), |e| e + 1);
    Some((i, rest[..end].to_string()))
}

/// Controller plugs and Bodyguard permits.
struct Controllers<'p, 'a>(&'p Project<'a>);

impl Controllers<'_, '_> {
    fn bodyguard_plug(&self, full: &str, at: (usize, u32)) -> Eff {
        let a = split_args(full);
        let action = opt(&a, "action").filter(|x| x.starts_with(':'));
        match (opt(&a, "policy"), action) {
            (Some(p), Some(act)) => Eff::Cond(Condition::Policy(format!(
                "{p}.{}",
                act.trim_start_matches(':')
            ))),
            _ => self.0.unknown(
                "Bodyguard.Plug.Authorize",
                at,
                "policy or action is not a literal",
            ),
        }
    }

    /// Controller `plug … when action in [..]` and a Bodyguard permit first in the action.
    fn layers(&self, file: usize, action: &str) -> Vec<Eff> {
        let mut out = self.plugs(file, action);
        out.extend(self.permit(file, action));
        out
    }

    fn plugs(&self, file: usize, action: &str) -> Vec<Eff> {
        let p = self.0;
        let mut out = Vec::new();
        for (line, s) in stmts(&p.files[file].text) {
            let Some(rest) = s.strip_prefix("plug ") else {
                continue;
            };
            let (head, guard) = match rest.split_once(" when ") {
                Some((h, g)) => (h.trim(), Some(g.trim())),
                None => (rest.trim(), None),
            };
            let name = split_args(head).remove(0);
            let eff = p.plug(&name, head, (file, line), 0);
            match (applies(guard, action), eff) {
                (Some(true), eff) => out.push(eff),
                (Some(false), _) | (None, Eff::Pass) => {}
                (None, _) => {
                    let why = "its `when` guard is not `action in [...]`";
                    out.push(p.unknown(&name, (file, line), why));
                }
            }
        }
        out
    }

    /// `Bodyguard.permit(Ctx, :action, …)` as the first statement of the action.
    fn permit(&self, file: usize, action: &str) -> Option<Eff> {
        let d = fn_defs(&self.0.files[file].text)
            .into_iter()
            .find(|d| d.name == action)?;
        let first = d.body.lines().map(str::trim).find(|l| !l.is_empty())?;
        let call = &first[first.find("Bodyguard.permit")?..];
        let inner = call.split_once('(').map(|(_, r)| r).unwrap_or("");
        let args = split_args(inner.trim_end_matches(" do").trim_end_matches(')'));
        Some(match (args.first(), args.get(1)) {
            (Some(c), Some(a)) if a.starts_with(':') => Eff::Cond(Condition::Policy(format!(
                "{c}.{}",
                a.trim_start_matches(':')
            ))),
            _ => self.0.unknown(
                "Bodyguard.permit",
                (file, 0),
                "context or action is not a literal",
            ),
        })
    }
}

/// Whether a `when` guard `action [not] in [..]` / `==` / `!=` covers `action`;
/// none when it is not that form.
fn applies(guard: Option<&str>, action: &str) -> Option<bool> {
    let Some(g) = guard else {
        return Some(true);
    };
    ["not in", "in", "!=", "=="].iter().find_map(|op| {
        let (a, list) = g.split_once(&format!(" {op} "))?;
        (a.trim() == "action").then(|| {
            let listed = split_list(list)
                .iter()
                .any(|x| x.trim_start_matches(':') == action);
            listed != matches!(*op, "not in" | "!=")
        })
    })
}

fn route(
    line: u32,
    stack: &[Frame],
    (method, path): (&str, &str),
    module: &str,
    action: Option<String>,
    live: bool,
) -> Route {
    let (mut full, mut pipes, mut session) = (String::new(), Vec::new(), None);
    for fr in stack {
        match fr {
            Frame::Scope {
                prefix, pipes: p, ..
            } => {
                full.push_str(prefix);
                pipes.extend(p.iter().cloned());
            }
            Frame::Res(p) => full.push_str(p),
            Frame::Session(s) => session = Some(s.clone()),
            _ => {}
        }
    }
    full.push_str(path);
    Route {
        method: method.into(),
        path: normalise(&full),
        line,
        module: module.into(),
        action,
        pipes,
        live: live.then_some(session),
    }
}

/// What a plug/hook body does to access: see the module docs.
fn gate(body: &str, halt: &str, name: &str, ids: &[String]) -> Eff {
    if !body.contains(halt) {
        return Eff::Pass;
    }
    if !ids.iter().any(|t| body.contains(t.as_str())) {
        return Eff::Cond(Condition::Unknown(name.into()));
    }
    let custom = || Eff::Cond(Condition::Custom(name.into()));
    let lines: Vec<&str> = body.lines().collect();
    let Some(at) = lines.iter().position(|l| {
        let t = l.trim();
        t.starts_with("if ") || t.starts_with("unless ")
    }) else {
        return custom();
    };
    let head = lines[at].trim();
    let unless = head.starts_with("unless ");
    let cond = head.trim_start_matches("unless ").trim_start_matches("if ");
    let cond = cond.trim_end_matches(" do").trim();
    let negated = cond.starts_with("not ") || cond.starts_with('!');
    if !plain_identity(cond, ids) {
        return custom();
    }
    let rest = &lines[at + 1..];
    let else_at = rest.iter().position(|l| l.trim() == "else");
    let halt_at = rest.iter().position(|l| l.contains(halt));
    let halt_in_else = else_at.is_some_and(|e| halt_at.is_some_and(|h| h > e));
    let present_runs_do = !(unless ^ negated);
    if present_runs_do == halt_in_else {
        Eff::Cond(Condition::Authenticated)
    } else {
        Eff::Cond(Condition::Not(Box::new(Condition::Authenticated)))
    }
}

/// A condition that only asks whether an identity is there: every `&&`/`||`
/// part is `current_scope`, `current_scope.user`, `current_user` or `logged_in?(..)`.
fn plain_identity(cond: &str, ids: &[String]) -> bool {
    let cond = cond.trim_start_matches("not ").trim_start_matches('!');
    cond.split("&&").flat_map(|p| p.split("||")).all(|p| {
        let p = p.trim().trim_start_matches("not ").trim_start_matches('!');
        let p = p
            .replace("conn.assigns", "")
            .replace("socket.assigns", "")
            .replace("assigns", "");
        let p = p.trim_start_matches('.').replace("[:", "").replace(']', "");
        matches!(
            p.as_str(),
            "current_scope" | "current_scope.user" | "current_user"
        ) || (p.starts_with("logged_in?(") && ids.iter().any(|i| i == "logged_in?"))
    })
}

/// The body of a project `logged_in?`: only `!!x`, `not is_nil(x)` or `x != nil`
/// of a plain identity read.
fn plain_login_body(body: &str) -> bool {
    let b = body.trim();
    let inner = (b.strip_prefix("!!"))
        .or_else(|| {
            b.strip_prefix("not is_nil(")
                .and_then(|x| x.strip_suffix(')'))
        })
        .or_else(|| b.strip_suffix(" != nil"));
    inner.is_some_and(|i| plain_identity(i, &[]))
}

fn atom(s: Option<&String>) -> Option<String> {
    s.filter(|x| x.starts_with(':'))
        .map(|x| x.trim_start_matches(':').to_string())
}

fn current_module(stack: &[Frame]) -> String {
    (stack.iter())
        .rev()
        .find_map(|f| match f {
            Frame::Scope { module, .. } => Some(module.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

use ex::*;

/// Elixir text helpers: statements, arguments, literals, function bodies.
mod ex {
    pub(super) fn join_mod(outer: &str, inner: &str) -> String {
        match (outer.is_empty(), inner.is_empty()) {
            (true, _) => inner.into(),
            (_, true) => outer.into(),
            _ => format!("{outer}.{inner}"),
        }
    }

    pub(super) fn normalise(path: &str) -> String {
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        format!("/{}", parts.join("/"))
    }

    pub(super) fn literal(s: &str) -> Option<String> {
        let inner = s.trim().strip_prefix('"')?.strip_suffix('"')?;
        (!inner.contains("#{")).then(|| inner.to_string())
    }

    /// The value of keyword option `key:` among already split arguments.
    pub(super) fn opt(args: &[String], key: &str) -> Option<String> {
        let k = format!("{key}:");
        args.iter()
            .find_map(|a| a.strip_prefix(&k))
            .map(|v| v.trim().to_string())
    }

    /// `[:a, :b]` as its items; any other form as its one top-level item.
    pub(super) fn split_list(s: &str) -> Vec<String> {
        let t = s.trim();
        match t.strip_prefix('[').and_then(|x| x.strip_suffix(']')) {
            Some(inner) => split_args(inner),
            None => split_args(t).into_iter().take(1).collect(),
        }
    }

    pub(super) fn resource_routes(a: &[String]) -> Vec<(String, String, String)> {
        let singleton = opt(a, "singleton").is_some_and(|v| v == "true");
        let all = [
            ("GET", "", "index"),
            ("GET", "/new", "new"),
            ("POST", "", "create"),
            ("GET", "/:id", "show"),
            ("GET", "/:id/edit", "edit"),
            ("PATCH", "/:id", "update"),
            ("PUT", "/:id", "update"),
            ("DELETE", "/:id", "delete"),
        ];
        let list = |k: &str| {
            opt(a, k).map(|v| {
                split_list(&v)
                    .iter()
                    .map(|x| x.trim_start_matches(':').to_string())
                    .collect::<Vec<_>>()
            })
        };
        let (only, except) = (list("only"), list("except"));
        (all.iter())
            .filter(|(_, _, act)| !(singleton && *act == "index"))
            .filter(|(_, _, act)| only.as_ref().is_none_or(|o| o.iter().any(|x| x == act)))
            .filter(|(_, _, act)| except.as_ref().is_none_or(|o| !o.iter().any(|x| x == act)))
            .map(|(m, p, act)| {
                let p = if singleton {
                    p.replace("/:id", "")
                } else {
                    p.to_string()
                };
                (m.to_string(), p, act.to_string())
            })
            .collect()
    }

    /// Splits at top-level commas (outside brackets and strings).
    pub(super) fn split_args(s: &str) -> Vec<String> {
        let (mut out, mut cur, mut depth, mut quote) = (Vec::new(), String::new(), 0i32, false);
        for c in s.chars() {
            match c {
                '"' => quote = !quote,
                '(' | '[' | '{' if !quote => depth += 1,
                ')' | ']' | '}' if !quote => depth -= 1,
                ',' if !quote && depth == 0 => {
                    out.push(cur.trim().to_string());
                    cur.clear();
                    continue;
                }
                _ => {}
            }
            cur.push(c);
        }
        if !cur.trim().is_empty() {
            out.push(cur.trim().to_string());
        }
        out
    }

    /// Statements of Elixir source: comments dropped, continued lines joined.
    pub(super) fn stmts(text: &str) -> Vec<(u32, String)> {
        let (mut out, mut cur, mut start, mut depth) = (Vec::new(), String::new(), 0u32, 0i32);
        for (i, raw) in text.lines().enumerate() {
            let (line, delta) = strip_line(raw);
            depth += delta;
            let t = line.trim();
            if t.is_empty() {
                continue;
            }
            if cur.is_empty() {
                start = i as u32 + 1;
            } else {
                cur.push(' ');
            }
            cur.push_str(t);
            if depth <= 0 && !t.ends_with([',', '\\']) && !t.ends_with("|>") {
                out.push((start, std::mem::take(&mut cur)));
                depth = 0;
            }
        }
        if !cur.is_empty() {
            out.push((start, cur));
        }
        out
    }

    /// A line without its `#` comment, and its bracket balance (strings skipped).
    pub(super) fn strip_line(raw: &str) -> (String, i32) {
        let (mut line, mut quote, mut depth) = (String::new(), false, 0);
        for c in raw.chars() {
            match (c, quote) {
                ('"', _) => quote = !quote,
                ('#', false) => break,
                ('(' | '[' | '{', false) => depth += 1,
                (')' | ']' | '}', false) => depth -= 1,
                _ => {}
            }
            line.push(c);
        }
        (line, depth)
    }

    /// `def`/`defp` functions; a block body ends at the `end` with the head's indent.
    pub(super) fn fn_defs(text: &str) -> Vec<super::FnDef> {
        let lines: Vec<&str> = text.lines().collect();
        let mut out = Vec::new();
        for (i, l) in lines.iter().enumerate() {
            let t = l.trim_start();
            let Some(rest) = t.strip_prefix("defp ").or_else(|| t.strip_prefix("def ")) else {
                continue;
            };
            let name: String = (rest.chars())
                .take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '?' | '!'))
                .collect();
            let indent = l.len() - t.len();
            let body = if let Some((_, one)) = t.split_once(", do:") {
                one.trim().to_string()
            } else if t.ends_with(" do") {
                let end = (lines[i + 1..].iter())
                    .position(|n| n.trim() == "end" && n.len() - n.trim_start().len() == indent)
                    .map_or(lines.len(), |e| i + 1 + e);
                lines[i + 1..end].join("\n")
            } else {
                continue;
            };
            out.push(super::FnDef {
                name,
                head: t.to_string(),
                body,
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use archguard_core::Config;

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

    const AUTH: &str = "defmodule W.UserAuth do
  def fetch_current_scope_for_user(conn, _opts) do
    assign(conn, :current_scope, 1)
  end

  def require_authenticated_user(conn, _opts) do
    if conn.assigns.current_scope && conn.assigns.current_scope.user do
      conn
    else
      conn
      |> redirect(to: \"/login\")
      |> halt()
    end
  end

  def redirect_if_user_is_authenticated(conn, _opts) do
    if conn.assigns.current_scope do
      conn
      |> redirect(to: \"/\")
      |> halt()
    else
      conn
    end
  end

  def require_admin(conn, _opts) do
    if conn.assigns.current_scope.user.role == :admin do
      conn
    else
      conn |> halt()
    end
  end

  def on_mount(:mount_current_scope, _params, _session, socket) do
    {:cont, socket}
  end

  def on_mount(:require_authenticated, _params, _session, socket) do
    if socket.assigns.current_scope && socket.assigns.current_scope.user do
      {:cont, socket}
    else
      {:halt, Phoenix.LiveView.redirect(socket, to: \"/login\")}
    end
  end
end
";

    fn router(body: &str) -> String {
        format!(
            "defmodule W.Router do
  use W, :router
  pipeline :browser do
    plug :accepts, [\"html\"]
    plug :fetch_session
  end
  pipeline :fetched do
    plug :fetch_current_scope_for_user
  end
  pipeline :auth do
    plug :require_authenticated_user
  end
  pipeline :admin do
    plug :require_admin
  end
{body}end
"
        )
    }

    fn go(body: &str, extra: &[(&str, &str)]) -> UiFlow {
        let r = router(body);
        let mut files = vec![
            ("mix.exs", ""),
            ("lib/w_web/router.ex", r.as_str()),
            ("lib/w_web/user_auth.ex", AUTH),
        ];
        files.extend_from_slice(extra);
        run(&files, "")
    }

    fn acc(flow: &UiFlow, path: &str) -> AccessState {
        (flow.endpoints.iter())
            .find(|e| e.path == path)
            .unwrap_or_else(|| panic!("no {path}: {:?}", flow.endpoints))
            .access
            .clone()
    }

    fn authed() -> AccessState {
        AccessState::Protected(Condition::Authenticated)
    }

    #[test]
    fn pf1_passive_pipeline_is_undeclared() {
        let f = go(
            "  scope \"/\", W do\n    pipe_through :browser\n    get \"/\", PageController, :home\n  end\n",
            &[],
        );
        assert_eq!(acc(&f, "/"), AccessState::Undeclared);
        assert!(f.complete, "{:?}", f.diagnostics);
    }

    #[test]
    fn pf2_pipe_through_covers_only_later_routes() {
        let f = go(
            "  scope \"/\" do\n    get \"/a\", C, :a\n    pipe_through [:browser, :auth]\n    get \"/b\", C, :b\n  end\n",
            &[],
        );
        assert_eq!(acc(&f, "/a"), AccessState::Undeclared);
        assert_eq!(acc(&f, "/b"), authed());
    }

    #[test]
    fn pf3_nested_scopes_accumulate() {
        let f = go(
            "  scope \"/\" do\n    pipe_through :auth\n    scope \"/admin\" do\n      pipe_through :admin\n      get \"/x\", C, :x\n    end\n    get \"/y\", C, :y\n  end\n",
            &[],
        );
        let both = vec![
            Condition::Authenticated,
            Condition::Custom("require_admin".into()),
        ];
        assert_eq!(
            acc(&f, "/admin/x"),
            AccessState::Protected(Condition::All(both))
        );
        assert_eq!(acc(&f, "/y"), authed());
    }

    #[test]
    fn pf4_guest_only_plug() {
        let f = go(
            "  scope \"/\" do\n    pipe_through [:browser, :redirect_if_user_is_authenticated]\n    get \"/users/log-in\", C, :new\n  end\n",
            &[],
        );
        let guest = Condition::Not(Box::new(Condition::Authenticated));
        assert_eq!(acc(&f, "/users/log-in"), AccessState::Protected(guest));
    }

    #[test]
    fn pf5_loader_only_is_undeclared() {
        let f = go(
            "  scope \"/\" do\n    pipe_through :fetched\n    get \"/\", C, :a\n  end\n",
            &[],
        );
        assert_eq!(acc(&f, "/"), AccessState::Undeclared);
    }

    #[test]
    fn pf6_live_session_hook_protects_the_screen() {
        let f = go(
            "  scope \"/\" do\n    pipe_through [:browser, :auth]\n    live_session :app, on_mount: [{W.UserAuth, :require_authenticated}] do\n      live \"/dash\", DashLive\n    end\n  end\n",
            &[],
        );
        let s = &f.screens[0];
        assert_eq!(
            (s.route.as_deref(), &s.access, s.weak),
            (Some("/dash"), &authed(), false)
        );
    }

    #[test]
    fn pf7_pipeline_only_live_screen_is_weak_and_a_gap_when_sessions_differ() {
        let body = "  scope \"/\" do\n    pipe_through :auth\n    live_session :default do\n      live \"/dash\", DashLive\n    end\n  end\n";
        let f = go(body, &[]);
        assert_eq!((&f.screens[0].access, f.screens[0].weak), (&authed(), true));
        assert!(f.complete, "{:?}", f.diagnostics);
        let both = format!(
            "{body}  scope \"/\" do\n    live_session :default do\n      live \"/pub\", PubLive\n    end\n  end\n"
        );
        let f = go(&both, &[]);
        assert!(!f.complete);
        let public = f
            .screens
            .iter()
            .find(|s| s.route.as_deref() == Some("/pub"))
            .unwrap();
        assert_eq!(public.access, AccessState::Undeclared);
    }

    #[test]
    fn pf8_loader_hook_is_no_condition() {
        let f = go(
            "  scope \"/\" do\n    live_session :a, on_mount: [{W.UserAuth, :mount_current_scope}] do\n      live \"/p\", PLive\n    end\n  end\n",
            &[],
        );
        assert_eq!(f.screens[0].access, AccessState::Undeclared);
        assert!(f.complete, "{:?}", f.diagnostics);
    }

    #[test]
    fn pf9_controller_plug_with_action_guard() {
        let ctl = "defmodule W.PostController do
  plug :authorize when action in [:edit, :update]
  def edit(c, _), do: c
  defp authorize(conn, _) do
    if conn.assigns.current_user.id == 1 do
      conn
    else
      halt(conn)
    end
  end
end
";
        let body = "  scope \"/\", W do\n    get \"/posts\", PostController, :index\n    get \"/posts/:id/edit\", PostController, :edit\n  end\n";
        let f = go(body, &[("lib/w_web/post_controller.ex", ctl)]);
        assert_eq!(acc(&f, "/posts"), AccessState::Undeclared);
        let custom = Condition::Custom("authorize".into());
        assert_eq!(acc(&f, "/posts/:id/edit"), AccessState::Protected(custom));
    }

    #[test]
    fn pf10_resources_only() {
        let f = go(
            "  scope \"/\" do\n    resources \"/posts\", PostController, only: [:index, :show]\n  end\n",
            &[],
        );
        assert_eq!(f.endpoints.len(), 2);
        let f = go(
            "  scope \"/\" do\n    resources \"/posts\", PostController\n  end\n",
            &[],
        );
        assert_eq!(f.endpoints.len(), 8);
    }

    #[test]
    fn pf11_bodyguard_permit_first_statement() {
        let ctl = "defmodule W.PostController do
  def delete(conn, params) do
    with :ok <- Bodyguard.permit(Blog, :delete_post, user, post) do
      conn
    end
  end
end
";
        let body =
            "  scope \"/\", W do\n    delete \"/posts/:id\", PostController, :delete\n  end\n";
        let f = go(body, &[("lib/w_web/post_controller.ex", ctl)]);
        let policy = Condition::Policy("Blog.delete_post".into());
        assert_eq!(acc(&f, "/posts/:id"), AccessState::Protected(policy));
    }

    #[test]
    fn pf12_guardian_verify_is_not_protection_ensure_is() {
        let r = "defmodule W.Router do
  use W, :router
  pipeline :v do
    plug Guardian.Plug.VerifyHeader
  end
  pipeline :e do
    plug Guardian.Plug.VerifyHeader
    plug Guardian.Plug.EnsureAuthenticated
  end
  scope \"/a\" do
    pipe_through :v
    get \"/x\", C, :x
  end
  scope \"/b\" do
    pipe_through :e
    get \"/y\", C, :y
  end
end
";
        let f = run(&[("mix.exs", ""), ("lib/r.ex", r)], "");
        assert_eq!(acc(&f, "/a/x"), AccessState::Undeclared);
        assert_eq!(acc(&f, "/b/y"), authed());
    }

    #[test]
    fn pf13_endpoint_plug_before_the_router_applies_to_everything() {
        let ep = "defmodule W.Endpoint do
  use Phoenix.Endpoint, otp_app: :w
  plug BasicAuthPlug
  plug W.Router
end
";
        let basic = "defmodule BasicAuthPlug do
  def call(conn, _) do
    case get_req_header(conn, \"authorization\") do
      [] -> halt(conn)
      _ -> conn
    end
  end
end
";
        let f = go(
            "  scope \"/\" do\n    get \"/\", C, :a\n  end\n",
            &[("lib/w_web/endpoint.ex", ep), ("lib/basic.ex", basic)],
        );
        let custom = Condition::Custom("BasicAuthPlug".into());
        assert_eq!(acc(&f, "/"), AccessState::Protected(custom));
    }

    #[test]
    fn a_halting_plug_without_identity_is_unknown_and_a_gap() {
        let limit = "defmodule RateLimit do\n  def call(conn, _), do: halt(conn)\nend\n";
        let r = "defmodule W.Router do\n  use W, :router\n  pipeline :p do\n    plug RateLimit\n  end\n  scope \"/\" do\n    pipe_through :p\n    get \"/\", C, :a\n  end\nend\n";
        let f = run(&[("mix.exs", ""), ("lib/r.ex", r), ("lib/l.ex", limit)], "");
        let unknown = Condition::Unknown("RateLimit".into());
        assert_eq!(acc(&f, "/"), AccessState::Protected(unknown));
        assert!(!f.complete);
    }

    #[test]
    fn a_halting_function_plug_and_hook_without_identity_are_unknown() {
        let auth = "defmodule W.Gate do
  def only_admin(conn, _opts) do
    if conn.assigns.admin? do
      conn
    else
      halt(conn)
    end
  end

  def on_mount(:admin, _p, _s, socket) do
    if socket.assigns.admin? do
      {:cont, socket}
    else
      {:halt, socket}
    end
  end
end
";
        let r = "defmodule W.Router do
  use W, :router
  scope \"/\" do
    pipe_through :only_admin
    get \"/a\", C, :a
    live_session :s, on_mount: [{W.Gate, :admin}] do
      live \"/l\", L
    end
  end
end
";
        let f = run(&[("mix.exs", ""), ("lib/r.ex", r), ("lib/g.ex", auth)], "");
        let plug = Condition::Unknown("only_admin".into());
        assert_eq!(acc(&f, "/a"), AccessState::Protected(plug.clone()));
        let both = Condition::All(vec![Condition::Unknown("W.Gate:admin".into()), plug]);
        assert_eq!(f.screens[0].access, AccessState::Protected(both));
        assert!(!f.complete);
    }

    #[test]
    fn a_plug_builder_module_reads_its_inner_plugs() {
        let guarded =
            "defmodule W.Guarded do\n  use Plug.Builder\n  plug :require_authenticated_user\nend\n";
        let body = "  pipeline :g do\n    plug W.Guarded\n  end\n  scope \"/\" do\n    pipe_through :g\n    get \"/\", C, :a\n  end\n";
        let f = go(body, &[("lib/w_web/guarded.ex", guarded)]);
        assert_eq!(acc(&f, "/"), authed());
    }

    fn login_helper(def: &str) -> UiFlow {
        let helper = format!(
            "defmodule W.Helpers do\n{def}\n  def must_login(conn, _) do\n    if logged_in?(conn) do\n      conn\n    else\n      halt(conn)\n    end\n  end\nend\n"
        );
        let r = "defmodule W.Router do\n  use W, :router\n  scope \"/\" do\n    pipe_through :must_login\n    get \"/\", C, :a\n  end\nend\n";
        run(
            &[("mix.exs", ""), ("lib/r.ex", r), ("lib/h.ex", &helper)],
            "",
        )
    }

    #[test]
    fn logged_in_is_read_from_its_project_definition_only() {
        let plain =
            login_helper("  def logged_in?(conn) do\n    !!conn.assigns[:current_user]\n  end\n");
        assert_eq!(acc(&plain, "/"), authed());
        let unknown = AccessState::Protected(Condition::Unknown("must_login".into()));
        let other =
            login_helper("  def logged_in?(conn) do\n    conn.assigns.role == :staff\n  end\n");
        assert_eq!(acc(&other, "/"), unknown);
        assert!(!other.complete);
        let missing = login_helper("");
        assert_eq!(acc(&missing, "/"), unknown);
        assert!(!missing.complete);
    }

    #[test]
    fn an_unexplained_plug_is_unknown_and_a_gap_and_beats_open() {
        let r = "defmodule W.Router do
  use W, :router
  pipeline :p do
    plug :public_marker
    plug :mystery
  end
  scope \"/\" do
    pipe_through :p
    get \"/\", C, :a
  end
end
";
        let f = run(
            &[("mix.exs", ""), ("lib/r.ex", r)],
            "public_marker = \"open\"\n",
        );
        let unknown = Condition::Unknown("mystery".into());
        assert_eq!(acc(&f, "/"), AccessState::Protected(unknown));
        assert!(!f.complete);
        let only_open = r.replace("    plug :mystery\n", "");
        let f = run(
            &[("mix.exs", ""), ("lib/r.ex", &only_open)],
            "public_marker = \"open\"\n",
        );
        assert_eq!(acc(&f, "/"), AccessState::Open);
    }
}
