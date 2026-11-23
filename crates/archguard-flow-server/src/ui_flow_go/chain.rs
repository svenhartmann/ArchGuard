//! Registration calls of a router chain.
use super::model::*;
use super::text::*;
use archguard_core::ANY_METHOD;

/// One call of a chain on a router.
struct Call<'s> {
    src: usize,
    id: usize,
    at: usize,
    kind: Kind,
    name: &'s str,
    args: Vec<(usize, usize)>,
}

/// State carried along one chain.
#[derive(Default)]
struct St {
    cursor: Option<usize>,
    /// The routes the last registration made (for `.Methods(..)`).
    routes: Vec<usize>,
    /// A gorilla `PathPrefix(..)`/`Path(..)` waiting for its handler.
    pending: Option<String>,
    methods: Option<Vec<String>>,
}

fn any_or(ms: Vec<String>) -> Vec<String> {
    if ms.is_empty() {
        vec![ANY_METHOD.into()]
    } else {
        ms
    }
}

impl Call<'_> {
    fn is_verb(&self) -> bool {
        let upper = self.name.to_ascii_uppercase();
        VERBS.contains(&upper.as_str()) && !matches!(self.kind, Kind::Http | Kind::Gorilla)
    }
}

impl<'a> Interp<'a> {
    pub(super) fn walk_chain(
        &mut self,
        fr: &mut Frame,
        src: usize,
        segs: &[Seg],
        from: usize,
        cursor: Option<usize>,
    ) -> Option<usize> {
        let srcs = self.srcs;
        let s = &srcs[src];
        let mut st = St {
            cursor,
            ..Default::default()
        };
        for sg in &segs[from..] {
            let Some(id) = st.cursor else { break };
            let Some(args) = sg.args else { continue };
            let c = Call {
                src,
                id,
                at: sg.at,
                kind: self.routers[id].kind,
                name: &sg.name,
                args: split_top(&s.code, args.0, args.1, b','),
            };
            self.seg(fr, &c, &mut st);
        }
        st.cursor
    }

    fn seg(&mut self, fr: &mut Frame, c: &Call, st: &mut St) {
        match c.name {
            "Use" | "Pre" => self.seg_use(fr, c),
            "Group" | "Route" | "Mount" | "With" => self.seg_scope(fr, c, st),
            "PathPrefix" | "Path" | "Subrouter" | "Methods" | "Handler" | "HandlerFunc" => {
                self.seg_gorilla(fr, c, st)
            }
            "Handle" | "HandleFunc" | "Method" | "MethodFunc" | "Add" | "Any" => {
                self.seg_handle(fr, c, st)
            }
            _ if c.is_verb() && !c.args.is_empty() => {
                st.routes = self.verb_route(fr, c, &c.name.to_ascii_uppercase());
            }
            _ => {}
        }
    }

    fn seg_use(&mut self, fr: &mut Frame, c: &Call) {
        let root = self.routers[c.id].up.is_empty();
        let echo_late = c.kind == Kind::Echo && (c.name == "Pre" || root);
        let late = c.kind == Kind::Gorilla || echo_late;
        for r in &c.args {
            let Some(cx) = self.middleware(fr, c.src, *r) else {
                continue;
            };
            let router = &mut self.routers[c.id];
            if late {
                router.late.push(cx)
            } else {
                router.chain.push(cx)
            }
        }
    }

    fn mws(&mut self, fr: &mut Frame, c: &Call, from: usize) -> Vec<Cx> {
        let rest = c.args.get(from..).unwrap_or(&[]);
        rest.iter()
            .filter_map(|r| self.middleware(fr, c.src, *r))
            .collect()
    }

    fn seg_scope(&mut self, fr: &mut Frame, c: &Call, st: &mut St) {
        match c.name {
            "Group" => self.group(fr, c, st),
            "Route" | "Mount" if c.args.len() == 2 => {
                let path = self.path_arg(c.src, c.args.first().copied(), c.at).0;
                if c.name == "Route" {
                    let child = self.derive(c.id, &path, vec![]);
                    self.closure(fr, c.src, c.args[1], child);
                } else {
                    self.mount(fr, c, &path);
                }
            }
            "With" => {
                let extra = self.mws(fr, c, 0);
                st.cursor = Some(self.derive(c.id, "", extra));
            }
            _ => {}
        }
    }

    fn group(&mut self, fr: &mut Frame, c: &Call, st: &mut St) {
        let s = &self.srcs[c.src];
        let closure = c
            .args
            .first()
            .is_some_and(|f| s.code[f.0..f.1].starts_with("func"));
        if closure {
            let child = self.derive(c.id, "", vec![]);
            self.closure(fr, c.src, c.args[0], child);
        } else if c.kind == Kind::Chi {
            if !c.args.is_empty() {
                let extra = self.mws(fr, c, 0);
                st.cursor = Some(self.derive(c.id, "", extra));
            }
        } else {
            let path = self.path_arg(c.src, c.args.first().copied(), c.at).0;
            let extra = self.mws(fr, c, 1);
            st.cursor = Some(self.derive(c.id, &path, extra));
        }
    }

    /// `Mount(path, sub)`: the sub-router takes the prefix and chain.
    fn mount(&mut self, fr: &mut Frame, c: &Call, path: &str) {
        match self.eval(fr, c.src, c.args[1]) {
            Some(child) => {
                let parent = self.derive(c.id, path, vec![]);
                self.rebase(child, parent);
            }
            None => {
                let s = &self.srcs[c.src];
                let t = collapse(&s.raw[c.args[1].0..c.args[1].1]);
                let m = format!(
                    "Mounted handler {t} is not a router that was read; its routes are unknown."
                );
                self.gap(c.src, c.at, m);
            }
        }
    }

    fn seg_gorilla(&mut self, fr: &mut Frame, c: &Call, st: &mut St) {
        match c.name {
            "PathPrefix" | "Path" if c.kind == Kind::Gorilla => {
                st.pending = Some(self.path_arg(c.src, c.args.first().copied(), c.at).0);
            }
            "Subrouter" if st.pending.is_some() => {
                let prefix = st.pending.take().unwrap_or_default();
                st.cursor = Some(self.derive(c.id, &prefix, vec![]));
            }
            "Methods" => self.methods(c, st),
            "Handler" | "HandlerFunc" if st.pending.is_some() && !c.args.is_empty() => {
                let path = st.pending.take().unwrap_or_default();
                let (mws, handler) = self.handler_arg(fr, c.src, c.args[0]);
                let methods = any_or(st.methods.take().unwrap_or_default());
                st.routes = self.reg(
                    c,
                    Reg {
                        methods,
                        path,
                        mws,
                        handler,
                    },
                );
            }
            _ => {}
        }
    }

    fn methods(&mut self, c: &Call, st: &mut St) {
        let ms: Vec<String> = c
            .args
            .iter()
            .filter_map(|r| self.method_arg(c.src, *r))
            .collect();
        if ms.len() != c.args.len() {
            self.gap(
                c.src,
                c.at,
                "HTTP method of Methods(..) is not a literal; ANY assumed.",
            );
        }
        if st.routes.is_empty() {
            st.methods = Some(ms);
        } else if !ms.is_empty() {
            self.set_methods(&st.routes, &ms);
        }
    }

    fn reg(&mut self, c: &Call, reg: Reg) -> Vec<usize> {
        self.register(c.src, c.id, c.at, reg)
    }

    fn seg_handle(&mut self, fr: &mut Frame, c: &Call, st: &mut St) {
        let named = matches!(c.name, "Handle" | "HandleFunc");
        match c.kind {
            Kind::Http if named => self.handle_http(fr, c, st),
            Kind::Gorilla | Kind::Chi if named => self.handle_path(fr, c, st),
            _ if c.name != "Any" && c.args.len() >= 3 => self.handle_method(fr, c, st),
            _ if c.name == "Any" && c.args.len() >= 2 => {
                st.routes = self.verb_route(fr, c, ANY_METHOD);
            }
            _ => {}
        }
    }

    /// `mux.Handle("GET /p/{id}", h)`.
    fn handle_http(&mut self, fr: &mut Frame, c: &Call, st: &mut St) {
        let (Some(p), Some(h)) = (c.args.first(), c.args.get(1)) else {
            return;
        };
        let (mws, handler) = self.handler_arg(fr, c.src, *h);
        let pat = self.path_arg(c.src, Some(*p), c.at).0;
        let (method, path) = match pat.split_once(' ') {
            Some((m, rest)) if VERBS.contains(&m) => (m.to_string(), rest.trim().to_string()),
            _ => (ANY_METHOD.to_string(), pat),
        };
        let path = path
            .find('/')
            .map_or(path.clone(), |i| path[i..].to_string());
        let path = path.replace("{$}", "");
        st.routes = self.reg(
            c,
            Reg {
                methods: vec![method],
                path,
                mws,
                handler,
            },
        );
    }

    /// gorilla and chi `HandleFunc(path, h)`.
    fn handle_path(&mut self, fr: &mut Frame, c: &Call, st: &mut St) {
        let (Some(p), Some(h)) = (c.args.first(), c.args.get(1)) else {
            return;
        };
        let (mws, handler) = self.handler_arg(fr, c.src, *h);
        let path = self.path_arg(c.src, Some(*p), c.at).0;
        let methods = any_or(st.methods.take().unwrap_or_default());
        st.routes = self.reg(
            c,
            Reg {
                methods,
                path,
                mws,
                handler,
            },
        );
    }

    /// `Method("GET", path, h)`, gin `Handle("GET", path, h, mw..)`, echo `Add(..)`.
    fn handle_method(&mut self, fr: &mut Frame, c: &Call, st: &mut St) {
        let method = self.method_arg(c.src, c.args[0]);
        let path = self.path_arg(c.src, Some(c.args[1]), c.at).0;
        let (mut mws, handler) = self.handler_arg(fr, c.src, c.args[2]);
        if matches!(c.kind, Kind::Gin | Kind::Echo) {
            mws = self.mws(fr, c, 3);
        }
        let methods = vec![method.unwrap_or_else(|| ANY_METHOD.into())];
        st.routes = self.reg(
            c,
            Reg {
                methods,
                path,
                mws,
                handler,
            },
        );
    }

    /// `r.GET(path, mw.., h)` (gin), `e.GET(path, h, mw..)` (echo), `r.Get(path, h)`.
    fn verb_route(&mut self, fr: &mut Frame, c: &Call, method: &str) -> Vec<usize> {
        let path = self.path_arg(c.src, c.args.first().copied(), c.at).0;
        let n = c.args.len();
        let (mws, handler) = match c.kind {
            Kind::Gin if n >= 2 => {
                let mws = c.args[1..n - 1].iter();
                let mws = mws.filter_map(|r| self.middleware(fr, c.src, *r)).collect();
                (mws, self.handler_name(c.src, c.args[n - 1]))
            }
            Kind::Echo if n >= 2 => (self.mws(fr, c, 2), self.handler_name(c.src, c.args[1])),
            _ if n >= 2 => self.handler_arg(fr, c.src, c.args[n - 1]),
            _ => (vec![], None),
        };
        let methods = vec![method.to_string()];
        self.reg(
            c,
            Reg {
                methods,
                path,
                mws,
                handler,
            },
        )
    }
}
