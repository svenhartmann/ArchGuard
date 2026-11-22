//! Routers, routes, the interpreter state and the route access fold.
use super::text::*;
use crate::extract_ctx::{ExtractContext, gap};
use archguard_core::{AccessState, DiagnosticSeverity, EndpointAccess, UiFlow, UiFlowDiagnostic};
use std::collections::HashMap;

pub(super) const MAX_DEPTH: usize = 12;
pub(super) const VERBS: [&str; 9] = [
    "GET", "POST", "PUT", "PATCH", "DELETE", "OPTIONS", "HEAD", "CONNECT", "TRACE",
];

pub(super) struct Module {
    pub(super) dir: String,
    pub(super) path: String,
    pub(super) name: String,
}

pub(super) enum Policy {
    Csv(String),
    Db,
    Unreadable(String),
}

pub(super) struct Enforcer {
    pub(super) model: Option<String>,
    pub(super) policy: Policy,
    pub(super) at: (String, u32),
}

// ---- routers and routes -------------------------------------------------

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Kind {
    Gin,
    Chi,
    Echo,
    Gorilla,
    Http,
}

impl Kind {
    pub(super) fn label(self) -> &'static str {
        match self {
            Kind::Gin => "gin route",
            Kind::Chi => "chi route",
            Kind::Echo => "echo route",
            Kind::Gorilla => "gorilla/mux route",
            Kind::Http => "net/http route",
        }
    }
}

pub(super) fn constructor(path: &str, name: &str) -> Option<Kind> {
    match (path, name) {
        ("github.com/gin-gonic/gin", "New" | "Default") => Some(Kind::Gin),
        (p, "NewRouter" | "NewMux") if p.starts_with("github.com/go-chi/chi") => Some(Kind::Chi),
        (p, "New") if p.starts_with("github.com/labstack/echo") && !p.ends_with("middleware") => {
            Some(Kind::Echo)
        }
        ("github.com/gorilla/mux", "NewRouter") => Some(Kind::Gorilla),
        ("net/http", "NewServeMux") => Some(Kind::Http),
        _ => None,
    }
}

/// One middleware layer: its state, the literal paths it skips and a casbin
/// enforcer to evaluate per route.
#[derive(Clone, Debug)]
pub(super) struct Cx {
    pub(super) state: AccessState,
    pub(super) skip: Vec<String>,
    pub(super) casbin: Option<usize>,
}

pub(super) struct Router {
    pub(super) kind: Kind,
    pub(super) prefix: String,
    pub(super) chain: Vec<Cx>,
    /// Position-independent layers (gorilla `Use`, echo root `Use`/`Pre`).
    pub(super) late: Vec<Cx>,
    /// Server wrappers around the handler, outermost first.
    pub(super) wrap: Vec<Cx>,
    /// Routers whose `late`/`wrap` apply too, outermost first.
    pub(super) up: Vec<usize>,
}

#[derive(Clone)]
pub(super) struct Route {
    pub(super) router: usize,
    pub(super) method: String,
    pub(super) path: String,
    pub(super) chain: Vec<Cx>,
    pub(super) file: String,
    pub(super) line: u32,
    pub(super) handler: Option<String>,
}

/// A route to register on a router.
pub(super) struct Reg {
    pub(super) methods: Vec<String>,
    pub(super) path: String,
    pub(super) mws: Vec<Cx>,
    pub(super) handler: Option<String>,
}

#[derive(Clone, Default)]
pub(super) struct Frame {
    pub(super) vars: HashMap<String, usize>,
    pub(super) cond: bool,
    pub(super) ret: Option<usize>,
}

pub(super) struct Interp<'a> {
    pub(super) ctx: &'a ExtractContext<'a>,
    pub(super) srcs: &'a [Src],
    pub(super) mine: &'a [usize],
    pub(super) module: &'a Module,
    pub(super) funcs: Vec<Func>,
    pub(super) consts: HashMap<String, String>,
    pub(super) routers: Vec<Router>,
    pub(super) routes: Vec<Route>,
    pub(super) enforcers: Vec<Enforcer>,
    pub(super) enf_vars: HashMap<String, usize>,
    pub(super) default_mux: Option<usize>,
    pub(super) diags: Vec<UiFlowDiagnostic>,
    pub(super) depth: usize,
}

pub(super) fn join_path(prefix: &str, path: &str) -> String {
    let mut full = format!("{prefix}/{path}");
    while full.contains("//") {
        full = full.replace("//", "/");
    }
    if full.len() > 1 && full.ends_with('/') && !path.ends_with('/') && !path.is_empty() {
        full.pop();
    }
    if !full.starts_with('/') {
        full.insert(0, '/');
    }
    if full.len() > 1 && path.is_empty() {
        full = full.trim_end_matches('/').to_string();
        if full.is_empty() {
            full = "/".into();
        }
    }
    full
}

/// `const name = "literal"` or `var name = "literal"` on one line.
pub(super) fn const_of(s: &Src, line: &str) -> Option<(String, String)> {
    let t = line.trim_start();
    let t = t
        .strip_prefix("const ")
        .or_else(|| t.strip_prefix("var "))
        .unwrap_or(t);
    let (name, rest) = t.split_once('=')?;
    let name = name.split_whitespace().next().filter(|n| is_ident(n))?;
    let rest = rest.trim();
    if rest.len() < 2 || !rest.starts_with('"') || !rest.ends_with('"') {
        return None;
    }
    let off = rest.as_ptr() as usize - s.code.as_ptr() as usize;
    Some((name.to_string(), s.lit((off, off + rest.len()))?))
}

/// Whether `name(` is called somewhere in `all` (not its own declaration).
pub(super) fn is_called(all: &str, name: &str) -> bool {
    all.match_indices(&format!("{name}(")).any(|(p, _)| {
        let before = &all[..p];
        let line = &before[before.rfind('\n').map_or(0, |x| x + 1)..];
        !before.ends_with(|c: char| c.is_alphanumeric() || c == '_')
            && !(line.trim_start().starts_with("func ") && !line.contains('{'))
    })
}

pub(super) fn takes_router(params: &str) -> bool {
    const TYPES: [&str; 8] = [
        "Router",
        "RouterGroup",
        "Echo",
        "Group",
        "ServeMux",
        "Engine",
        "IRoutes",
        "IRouter",
    ];
    TYPES.iter().any(|t| params.contains(t))
}

impl<'a> Interp<'a> {
    pub(super) fn new(
        ctx: &'a ExtractContext<'a>,
        srcs: &'a [Src],
        mine: &'a [usize],
        module: &'a Module,
    ) -> Self {
        let mut funcs = Vec::new();
        let mut consts = HashMap::new();
        for &i in mine {
            funcs.extend(funcs_of(i, &srcs[i]));
            consts.extend(srcs[i].code.lines().filter_map(|l| const_of(&srcs[i], l)));
        }
        Interp {
            ctx,
            srcs,
            mine,
            module,
            funcs,
            consts,
            routers: vec![],
            routes: vec![],
            enforcers: vec![],
            enf_vars: HashMap::new(),
            default_mux: None,
            diags: vec![],
            depth: 0,
        }
    }

    pub(super) fn note(&mut self, src: usize, off: usize, m: impl Into<String>) {
        let s = &self.srcs[src];
        self.diags.push(UiFlowDiagnostic {
            file: s.rel.clone(),
            line: lines_before(&s.raw, off),
            message: m.into(),
            severity: DiagnosticSeverity::Note,
        });
    }

    pub(super) fn gap(&mut self, src: usize, off: usize, m: impl Into<String>) {
        let s = &self.srcs[src];
        gap(
            &mut self.diags,
            &s.rel.clone(),
            lines_before(&s.raw, off),
            m,
        );
    }

    pub(super) fn at(&self, src: usize, off: usize) -> (String, u32) {
        let s = &self.srcs[src];
        (s.rel.clone(), lines_before(&s.raw, off))
    }

    /// Functions never called by name are the roots (`main`, `init`).
    pub(super) fn run(&mut self) {
        let all: String = self
            .mine
            .iter()
            .map(|&i| format!("{}\n", self.srcs[i].code))
            .collect();
        for f in self.funcs.clone() {
            if is_called(&all, &f.name) && f.name != "main" && f.name != "init" {
                continue;
            }
            if takes_router(&self.srcs[f.src].code[f.params.0..f.params.1]) {
                let m = format!(
                    "Function {} takes a router but is not called; its routes are not read.",
                    f.name
                );
                self.gap(f.src, f.body.0, m);
                continue;
            }
            self.block(&mut Frame::default(), f.src, f.body);
        }
    }

    pub(super) fn default_mux_id(&mut self) -> usize {
        if let Some(id) = self.default_mux {
            return id;
        }
        let id = self.new_router(Kind::Http);
        self.default_mux = Some(id);
        id
    }

    pub(super) fn new_router(&mut self, kind: Kind) -> usize {
        let (prefix, chain, late, wrap, up) = Default::default();
        self.routers.push(Router {
            kind,
            prefix,
            chain,
            late,
            wrap,
            up,
        });
        self.routers.len() - 1
    }

    pub(super) fn derive(&mut self, parent: usize, path: &str, extra: Vec<Cx>) -> usize {
        let p = &self.routers[parent];
        let mut up = p.up.clone();
        up.push(parent);
        let mut chain = p.chain.clone();
        chain.extend(extra);
        let prefix = join_path(&p.prefix, path);
        let r = Router {
            kind: p.kind,
            prefix,
            chain,
            late: vec![],
            wrap: vec![],
            up,
        };
        self.routers.push(r);
        self.routers.len() - 1
    }

    pub(super) fn handler_name(&self, src: usize, r: (usize, usize)) -> Option<String> {
        let t = self.srcs[src].code[r.0..r.1].trim();
        t.split('.').all(is_ident).then(|| t.to_string())
    }

    pub(super) fn method_arg(&self, src: usize, r: (usize, usize)) -> Option<String> {
        let s = &self.srcs[src];
        if let Some(l) = s.lit(r) {
            return Some(l.to_ascii_uppercase());
        }
        let m = s.code[r.0..r.1]
            .trim()
            .strip_prefix("http.Method")?
            .to_ascii_uppercase();
        VERBS.contains(&m.as_str()).then_some(m)
    }

    pub(super) fn set_methods(&mut self, routes: &[usize], ms: &[String]) {
        let Some(&first) = routes.first() else { return };
        self.routes[first].method = ms[0].clone();
        for m in &ms[1..] {
            let dup = Route {
                method: m.clone(),
                ..self.routes[first].clone()
            };
            self.routes.push(dup);
        }
    }

    pub(super) fn register(&mut self, src: usize, id: usize, at: usize, reg: Reg) -> Vec<usize> {
        let r = &self.routers[id];
        let mut chain = r.chain.clone();
        chain.extend(reg.mws);
        let path = join_path(&r.prefix, &reg.path);
        let (file, line) = self.at(src, at);
        let mut out = vec![];
        for method in reg.methods {
            let (chain, path, file) = (chain.clone(), path.clone(), file.clone());
            let handler = reg.handler.clone();
            self.routes.push(Route {
                router: id,
                method,
                path,
                chain,
                file,
                line,
                handler,
            });
            out.push(self.routes.len() - 1);
        }
        out
    }

    /// A mounted router takes the parent's prefix and chain.
    pub(super) fn rebase(&mut self, child: usize, parent: usize) {
        let (pp, pc, mut pu) = {
            let p = &self.routers[parent];
            (p.prefix.clone(), p.chain.clone(), p.up.clone())
        };
        pu.push(parent);
        let members: Vec<usize> = (0..self.routers.len())
            .filter(|&i| i == child || self.routers[i].up.contains(&child))
            .collect();
        for &i in &members {
            let r = &mut self.routers[i];
            r.prefix = join_path(&pp, &r.prefix);
            r.chain.splice(0..0, pc.clone());
            r.up.splice(0..0, pu.clone());
        }
        for rt in self
            .routes
            .iter_mut()
            .filter(|rt| members.contains(&rt.router))
        {
            rt.path = join_path(&pp, &rt.path);
            rt.chain.splice(0..0, pc.clone());
        }
    }

    /// The layers a route runs through: server wrappers, position-independent
    /// layers, then its own chain.
    pub(super) fn layers(&self, rt: &Route) -> Vec<Cx> {
        let ups = self.routers[rt.router]
            .up
            .iter()
            .copied()
            .chain([rt.router]);
        let ups: Vec<usize> = ups.collect();
        let wraps = ups.iter().flat_map(|&u| self.routers[u].wrap.iter());
        let lates = ups.iter().flat_map(|&u| self.routers[u].late.iter());
        wraps.chain(lates).chain(rt.chain.iter()).cloned().collect()
    }

    /// The access of a route: all layers must hold; a layer that skips the
    /// route's literal path is left out and, alone, makes it open.
    pub(super) fn route_state(&mut self, rt: &Route) -> AccessState {
        let (mut state, mut skipped) = (AccessState::Undeclared, false);
        for cx in self.layers(rt) {
            if cx.skip.contains(&rt.path) {
                skipped = true;
                continue;
            }
            let st = match cx.casbin {
                Some(e) => self.casbin_state(e, &rt.method, &rt.path),
                None => cx.state,
            };
            state = state.and(st);
        }
        if skipped && state == AccessState::Undeclared {
            return AccessState::Open;
        }
        state
    }

    pub(super) fn finish_into(mut self, flow: &mut UiFlow) {
        for rt in std::mem::take(&mut self.routes) {
            let access = self.route_state(&rt);
            let source = self.routers[rt.router].kind.label().into();
            flow.endpoints.push(EndpointAccess {
                method: rt.method,
                path: rt.path,
                access,
                application: Some(self.module.name.clone()),
                file: rt.file,
                line: rt.line,
                source,
                handler: rt.handler,
                sends: vec![],
            });
        }
        flow.diagnostics.append(&mut self.diags);
    }
}
