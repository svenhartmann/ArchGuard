//! Middleware classification and handler/server wrappers.
use super::model::*;
use super::text::*;
use crate::extract_ctx::access_condition;
use crate::lang_ts::{Item, Value};
use archguard_core::{AccessState, Condition};

/// Packages whose middleware never decides access.
const PASSIVE: [&str; 14] = [
    "gin-contrib/cors",
    "gin-contrib/gzip",
    "gin-contrib/logger",
    "gin-contrib/requestid",
    "gin-contrib/secure",
    "gin-contrib/sessions",
    "gin-contrib/static",
    "gin-contrib/pprof",
    "gin-contrib/zap",
    "rs/cors",
    "gorilla/handlers",
    "go.opentelemetry.io/contrib",
    "prometheus/client_golang",
    "unrolled/secure",
];
const GIN_PASSIVE: [&str; 8] = [
    "Logger",
    "LoggerWithConfig",
    "LoggerWithFormatter",
    "LoggerWithWriter",
    "Recovery",
    "RecoveryWithWriter",
    "CustomRecovery",
    "ErrorLogger",
];
const ECHO_AUTH: [&str; 6] = [
    "JWT",
    "JWTWithConfig",
    "BasicAuth",
    "BasicAuthWithConfig",
    "KeyAuth",
    "KeyAuthWithConfig",
];
const ECHO_PASSIVE: [&str; 15] = [
    "Logger",
    "LoggerWithConfig",
    "Recover",
    "RecoverWithConfig",
    "CORS",
    "CORSWithConfig",
    "Gzip",
    "GzipWithConfig",
    "RequestID",
    "Secure",
    "BodyLimit",
    "Decompress",
    "RemoveTrailingSlash",
    "AddTrailingSlash",
    "Static",
];
const ECHO_PASSIVE_MORE: [&str; 4] = ["Timeout", "RateLimiter", "MethodOverride", "Rewrite"];
const CASBIN_FNS: [&str; 5] = [
    "NewAuthorizer",
    "Authorizer",
    "Middleware",
    "MiddlewareWithConfig",
    "CasbinAuth",
];
const HTTP_PASSIVE: [&str; 7] = [
    "StripPrefix",
    "TimeoutHandler",
    "MaxBytesHandler",
    "HandlerFunc",
    "FileServer",
    "Handler",
    "NotFoundHandler",
];

/// What a recognised library middleware does for access.
enum Known {
    Auth,
    Passive,
    Verifier,
    EchoAuth,
    Casbin,
    Other,
}

fn known_gin_chi(path: &str, last: &str) -> Option<Known> {
    let chi_mw = path.starts_with("github.com/go-chi/chi") && path.ends_with("middleware");
    if path == "github.com/gin-gonic/gin" {
        return Some(match last {
            "BasicAuth" | "BasicAuthForRealm" => Known::Auth,
            l if GIN_PASSIVE.contains(&l) => Known::Passive,
            _ => Known::Other,
        });
    }
    if path.contains("go-chi/jwtauth") {
        return Some(match last {
            "Authenticator" => Known::Auth,
            "Verifier" | "Verify" => Known::Verifier,
            _ => Known::Other,
        });
    }
    chi_mw.then(|| {
        if last == "BasicAuth" {
            Known::Auth
        } else {
            Known::Passive
        }
    })
}

fn known_echo(path: &str, last: &str) -> Option<Known> {
    let echo_mw = path.starts_with("github.com/labstack/echo") && path.ends_with("middleware");
    if path.starts_with("github.com/labstack/echo-jwt") || (echo_mw && ECHO_AUTH.contains(&last)) {
        return Some(Known::EchoAuth);
    }
    let passive = ECHO_PASSIVE.contains(&last) || ECHO_PASSIVE_MORE.contains(&last);
    (echo_mw && passive).then_some(Known::Passive)
}

fn known(path: &str, last: &str) -> Known {
    if let Some(k) = known_gin_chi(path, last).or_else(|| known_echo(path, last)) {
        return k;
    }
    if (path.contains("casbin") || path.contains("authz")) && CASBIN_FNS.contains(&last) {
        return Known::Casbin;
    }
    let passive = (path == "net/http" && HTTP_PASSIVE.contains(&last))
        || PASSIVE.iter().any(|x| path.contains(x));
    if passive {
        Known::Passive
    } else {
        Known::Other
    }
}

fn cx(state: AccessState) -> Cx {
    Cx {
        state,
        skip: vec![],
        casbin: None,
    }
}

fn unknown_state(text: &str) -> AccessState {
    AccessState::Protected(Condition::Unknown(text.into()))
}

/// The called name (`a.b.c`), the arguments of its first call and the text.
struct Callee {
    name: String,
    args: Option<(usize, usize)>,
}

/// The callee of a middleware expression: a call chain or a composite `T{..}`.
fn callee_of(s: &Src, r: (usize, usize)) -> Option<Callee> {
    let code = s.code[r.0..r.1].trim_start().trim_start_matches('&');
    let start = r.1 - code.len();
    let Some(segs) = chain(&s.code, start, r.1) else {
        let t = code.split('{').next().unwrap_or("").trim();
        let named = code.contains('{') && !t.is_empty() && t.split('.').all(is_ident);
        return named.then(|| Callee {
            name: t.to_string(),
            args: None,
        });
    };
    let n = (segs.iter())
        .take_while(|g| g.args.is_none() && !g.name.is_empty())
        .count();
    let upto = if n < segs.len() && !segs[n].name.is_empty() {
        n + 1
    } else {
        n
    };
    let name = segs[..upto]
        .iter()
        .map(|g| g.name.as_str())
        .collect::<Vec<_>>();
    let args = (upto > 0).then(|| segs[upto - 1].args).flatten();
    Some(Callee {
        name: name.join("."),
        args,
    })
}

/// A call chain whose last call wraps a handler.
struct Wrap {
    /// The callee text range (without the arguments of the last call).
    callee: (usize, usize),
    args: Vec<(usize, usize)>,
    /// `http.HandlerFunc(x)`: a conversion, no wrapper.
    conv: bool,
}

fn wrap_from(s: &Src, start: usize, segs: &[Seg]) -> Option<Wrap> {
    let ci = segs.iter().rposition(|g| g.args.is_some())?;
    let sg = &segs[ci];
    let args = sg.args?;
    let end = if sg.name.is_empty() {
        sg.at
    } else {
        sg.at + sg.name.len()
    };
    Some(Wrap {
        callee: (start, end),
        args: split_top(&s.code, args.0, args.1, b','),
        conv: matches!(sg.name.as_str(), "HandlerFunc" | "Handler"),
    })
}

/// The argument that is the wrapped handler: not a literal.
fn inner_index(s: &Src, args: &[(usize, usize)]) -> Option<usize> {
    args.iter().position(|p| {
        s.lit(*p).is_none()
            && !s.code[p.0..p.1]
                .trim()
                .starts_with(|c: char| c.is_ascii_digit())
    })
}

/// What `handler_arg` found at one step.
enum Peel {
    Done(Option<String>),
    Inner((usize, usize)),
}

impl<'a> Interp<'a> {
    pub(super) fn middleware(
        &mut self,
        fr: &mut Frame,
        src: usize,
        r: (usize, usize),
    ) -> Option<Cx> {
        let found = self.classify(src, r)?;
        if !fr.cond {
            return Some(found);
        }
        let t = collapse(&self.srcs[src].raw[r.0..r.1]);
        let m = format!("Middleware {t} is registered conditionally; its effect is unknown.");
        self.gap(src, r.0, m);
        Some(cx(unknown_state(&format!("conditional middleware {t}"))))
    }

    fn unknown_cx(
        &mut self,
        src: usize,
        r: (usize, usize),
        display: &str,
        why: &str,
    ) -> Option<Cx> {
        let m = format!(
            "Middleware {display} is not in [views.access] ({why}); treated as an unknown condition."
        );
        self.note(src, r.0, m);
        Some(cx(unknown_state(display)))
    }

    /// The `[views.access]` entry of a middleware, when it has one.
    fn listed(
        &mut self,
        src: usize,
        r: (usize, usize),
        callee: &Callee,
        display: &str,
    ) -> Option<Cx> {
        let srcs = self.srcs;
        let s = &srcs[src];
        let last = callee.name.rsplit('.').next().unwrap_or(&callee.name);
        let access = &self.ctx.config.views.access;
        let key = [callee.name.as_str(), last]
            .into_iter()
            .find(|k| access.contains_key(*k))?;
        let items: Vec<Item> = (callee.args.into_iter())
            .flat_map(|a| split_top(&s.code, a.0, a.1, b','))
            .map(|p| Item {
                at: p.0,
                end: p.1,
                value: (s.lit(p)).map_or_else(|| Value::Raw(s.raw[p.0..p.1].into()), Value::Str),
            })
            .collect();
        let state = access_condition(access, key, &items).unwrap_or_else(|_| {
            let m = format!(
                "Argument of middleware {display} is not a literal; its condition is unknown."
            );
            self.gap(src, r.0, m);
            unknown_state(display)
        });
        Some(cx(state))
    }

    /// The layer of one middleware expression; none when it adds nothing.
    fn classify(&mut self, src: usize, r: (usize, usize)) -> Option<Cx> {
        let srcs = self.srcs;
        let s = &srcs[src];
        let display = collapse(&s.raw[r.0..r.1]);
        if s.code[r.0..r.1].trim_start().starts_with("func") {
            return self.unknown_cx(src, r, &display, "inline function");
        }
        let Some(callee) = callee_of(s, r) else {
            return self.unknown_cx(src, r, &display, "expression not read");
        };
        if let Some(found) = self.listed(src, r, &callee, &display) {
            return Some(found);
        }
        let pkg = callee.name.split('.').next().unwrap_or("");
        let path = (callee.name.contains('.'))
            .then(|| s.imports.get(pkg))
            .flatten();
        let Some(path) = path else {
            return self.unknown_cx(src, r, &display, "no known package");
        };
        let last = callee.name.rsplit('.').next().unwrap_or(&callee.name);
        match known(path, last) {
            Known::Auth => Some(cx(AccessState::Protected(Condition::Authenticated))),
            Known::Passive => None,
            Known::Verifier => {
                let m = "jwtauth.Verifier only parses the token and never rejects; it adds no condition.";
                self.note(src, r.0, m);
                None
            }
            Known::EchoAuth => self.echo_auth(src, r, last, callee.args),
            Known::Casbin => self.casbin_cx(src, r, callee.args, &display),
            Known::Other => self.unknown_cx(src, r, &display, "package not known to decide access"),
        }
    }

    fn casbin_cx(
        &mut self,
        src: usize,
        r: (usize, usize),
        args: Option<(usize, usize)>,
        display: &str,
    ) -> Option<Cx> {
        let s = &self.srcs[src];
        let first = args.and_then(|a| split_top(&s.code, a.0, a.1, b',').first().copied());
        let name = first.map(|x| s.code[x.0..x.1].trim().to_string());
        if let Some(e) = name.and_then(|n| self.enf_vars.get(&n).copied()) {
            return Some(Cx {
                casbin: Some(e),
                ..cx(AccessState::Undeclared)
            });
        }
        self.gap(
            src,
            r.0,
            format!("Casbin middleware {display}: the enforcer is not read."),
        );
        Some(cx(unknown_state("casbin enforcer not read")))
    }

    /// `Skipper` of an auth config: the literal paths it skips, or the unknown layer.
    fn echo_skipper(
        &mut self,
        src: usize,
        r: (usize, usize),
        cfg: (usize, usize),
    ) -> Result<Vec<String>, Cx> {
        let srcs = self.srcs;
        let s = &srcs[src];
        let Some(v) = field(&s.code, cfg, "Skipper") else {
            return Ok(vec![]);
        };
        if s.code[v.0..v.1].trim() == "nil" {
            return Ok(vec![]);
        }
        skipper_paths(s, v).ok_or_else(|| {
            let m = "Skipper of the auth middleware is not a literal path comparison; its condition is unknown.";
            self.gap(src, r.0, m);
            cx(unknown_state("Skipper not readable"))
        })
    }

    /// `ContinueOnIgnoredError`: `Ok(true)` when the authentication is optional.
    fn echo_optional(
        &mut self,
        src: usize,
        r: (usize, usize),
        cfg: (usize, usize),
    ) -> Result<bool, Cx> {
        let s = &self.srcs[src];
        let Some(v) = field(&s.code, cfg, "ContinueOnIgnoredError") else {
            return Ok(false);
        };
        match s.code[v.0..v.1].trim() {
            "false" => Ok(false),
            "true" => {
                let m =
                    "ContinueOnIgnoredError: true makes the authentication optional; no condition.";
                self.note(src, r.0, m);
                Ok(true)
            }
            _ => {
                let m = "ContinueOnIgnoredError is not a literal; its condition is unknown.";
                self.gap(src, r.0, m);
                Err(cx(unknown_state("ContinueOnIgnoredError not readable")))
            }
        }
    }

    fn echo_auth(
        &mut self,
        src: usize,
        r: (usize, usize),
        last: &str,
        args: Option<(usize, usize)>,
    ) -> Option<Cx> {
        let authed = AccessState::Protected(Condition::Authenticated);
        let cfg = match args {
            Some(a) if last.ends_with("Config") => a,
            _ => return Some(cx(authed)),
        };
        let skip = match self.echo_skipper(src, r, cfg) {
            Ok(skip) => skip,
            Err(unknown) => return Some(unknown),
        };
        match self.echo_optional(src, r, cfg) {
            Ok(false) => Some(Cx { skip, ..cx(authed) }),
            Ok(true) => None,
            Err(unknown) => Some(unknown),
        }
    }

    /// Wrappers around a handler argument (outermost first) and its name.
    pub(super) fn handler_arg(
        &mut self,
        fr: &mut Frame,
        src: usize,
        r: (usize, usize),
    ) -> (Vec<Cx>, Option<String>) {
        let (mut cur, mut out) = (r, vec![]);
        for _ in 0..16 {
            match self.peel(fr, src, cur, &mut out) {
                Peel::Done(name) => return (out, name),
                Peel::Inner(next) => cur = next,
            }
        }
        (out, None)
    }

    fn peel(&mut self, fr: &mut Frame, src: usize, cur: (usize, usize), out: &mut Vec<Cx>) -> Peel {
        let srcs = self.srcs;
        let s = &srcs[src];
        let t = s.code[cur.0..cur.1].trim_start();
        let (start, cur) = (cur.1 - t.len(), (cur.1 - t.len(), cur.1));
        if t.starts_with("func") {
            return Peel::Done(None);
        }
        let Some(segs) = chain(&s.code, start, cur.1) else {
            return Peel::Done(None);
        };
        let Some(w) = wrap_from(s, start, &segs) else {
            return Peel::Done(self.handler_name(src, cur));
        };
        let Some(k) = inner_index(s, &w.args) else {
            return Peel::Done(self.handler_name(src, cur));
        };
        out.extend(self.wrapper(fr, src, &w, k));
        Peel::Inner(w.args[k])
    }

    /// The layer of the wrapper of `w` whose handler is argument `k`.
    fn wrapper(&mut self, fr: &mut Frame, src: usize, w: &Wrap, k: usize) -> Option<Cx> {
        if w.conv {
            return None;
        }
        let s = &self.srcs[src];
        let rest: Vec<String> = (w.args.iter().enumerate())
            .filter(|(i, _)| *i != k)
            .map(|(_, p)| collapse(&s.raw[p.0..p.1]))
            .collect();
        let name = s.raw[w.callee.0..w.callee.1].trim();
        let display = if rest.is_empty() {
            name.to_string()
        } else {
            format!("{name}({})", rest.join(", "))
        };
        self.middleware_text(fr, src, w.callee, &display)
    }

    /// [`Self::middleware`] for a callee whose arguments were cut out.
    fn middleware_text(
        &mut self,
        fr: &mut Frame,
        src: usize,
        callee: (usize, usize),
        display: &str,
    ) -> Option<Cx> {
        let mut cx = self.middleware(fr, src, callee)?;
        if let AccessState::Protected(Condition::Unknown(t)) = &mut cx.state
            && !t.starts_with("conditional")
        {
            *t = display.to_string();
        }
        Some(cx)
    }

    /// The router a served handler wraps, with the wrappers (outermost first).
    pub(super) fn unwrap_server(
        &mut self,
        fr: &mut Frame,
        src: usize,
        r: (usize, usize),
    ) -> Option<(usize, Vec<Cx>)> {
        let srcs = self.srcs;
        let s = &srcs[src];
        let t = s.code[r.0..r.1].trim_start().trim_start_matches('&');
        let start = r.1 - t.len();
        if let Some(&id) = fr.vars.get(t.trim()) {
            return Some((id, vec![]));
        }
        let brace = t
            .find('{')
            .filter(|&b| !t.contains('(') || Some(b) < t.find('('));
        match brace {
            Some(open) => self.unwrap_composite(fr, src, start, open),
            None => self.unwrap_call(fr, src, start, r.1),
        }
    }

    /// `T{next: h}`: `T` wraps `h`.
    fn unwrap_composite(
        &mut self,
        fr: &mut Frame,
        src: usize,
        start: usize,
        open: usize,
    ) -> Option<(usize, Vec<Cx>)> {
        let srcs = self.srcs;
        let s = &srcs[src];
        let close = close_of(&s.code, start + open)?;
        for f in split_top(&s.code, start + open + 1, close, b',') {
            let Some(p) = s.code[f.0..f.1].find(':') else {
                continue;
            };
            if let Some((id, mut w)) = self.unwrap_server(fr, src, (f.0 + p + 1, f.1)) {
                let name = s.raw[start..start + open].trim().to_string();
                w.insert(
                    0,
                    self.middleware_text(fr, src, (start, start + open), &name)?,
                );
                return Some((id, w));
            }
        }
        None
    }

    /// `f(.., h, ..)`: `f` wraps the argument that is a served router.
    fn unwrap_call(
        &mut self,
        fr: &mut Frame,
        src: usize,
        start: usize,
        end: usize,
    ) -> Option<(usize, Vec<Cx>)> {
        let srcs = self.srcs;
        let s = &srcs[src];
        let w = wrap_from(s, start, &chain(&s.code, start, end)?)?;
        for (k, p) in w.args.iter().enumerate() {
            if let Some((id, mut layers)) = self.unwrap_server(fr, src, *p) {
                layers.splice(0..0, self.wrapper(fr, src, &w, k));
                return Some((id, layers));
            }
        }
        None
    }

    pub(super) fn server_handler(
        &mut self,
        fr: &mut Frame,
        src: usize,
        r: (usize, usize),
    ) -> Option<(usize, Vec<Cx>)> {
        let code = &self.srcs[src].code;
        let open = code[r.0..r.1].find('{')? + r.0;
        let close = close_of(code, open)?;
        let v = field(code, (open + 1, close), "Handler")?;
        self.unwrap_server(fr, src, v)
    }
}
