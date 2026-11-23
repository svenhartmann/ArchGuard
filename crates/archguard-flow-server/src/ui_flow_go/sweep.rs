//! Route calls on receivers the interpreter did not follow (struct fields,
//! package variables, closures, interface registrars): a gap each, and an
//! unknown route when the path is a literal (ADR 0021: never dropped silently).
use super::model::*;
use super::text::*;
use archguard_core::{ANY_METHOD, AccessState, Condition};

const CHI_VERBS: [&str; 9] = [
    "Get", "Post", "Put", "Patch", "Delete", "Options", "Head", "Connect", "Trace",
];

fn is_route_name(name: &str) -> bool {
    VERBS.contains(&name)
        || CHI_VERBS.contains(&name)
        || matches!(name, "Handle" | "HandleFunc" | "Any")
}

/// The router kind of the file's imports, if it imports a router package.
fn kind_of(s: &Src) -> Option<Kind> {
    let has = |p: &str| s.imports.values().any(|v| v.contains(p));
    [
        ("gin-gonic/gin", Kind::Gin),
        ("go-chi/chi", Kind::Chi),
        ("labstack/echo", Kind::Echo),
        ("gorilla/mux", Kind::Gorilla),
    ]
    .into_iter()
    .find(|(p, _)| has(p))
    .map(|(_, k)| k)
}

/// `(name offset, name, first argument)` of each route-style call `x.NAME(..)`.
type Site = (usize, String, Option<(usize, usize)>);

fn call_sites(s: &Src) -> Vec<Site> {
    let code = &s.code;
    let mut out = vec![];
    for (i, _) in code.match_indices('.') {
        let j = skip_ws(code, i + 1);
        let len = code[j..]
            .bytes()
            .take_while(|c| c.is_ascii_alphanumeric())
            .count();
        let name = &code[j..j + len];
        let p = skip_ws(code, j + len);
        if !is_route_name(name) || code.as_bytes().get(p) != Some(&b'(') {
            continue;
        }
        let Some(c) = close_of(code, p) else { continue };
        let before = code[..i].trim_end();
        let recv = before
            .rsplit(|c: char| !(c.is_alphanumeric() || c == '_'))
            .next();
        if recv.is_some_and(|r| s.imports.contains_key(r)) {
            continue;
        }
        out.push((
            j,
            name.to_string(),
            split_top(code, p + 1, c, b',').first().copied(),
        ));
    }
    out
}

/// `(method, path)` of a literal first argument, none when it is no route path.
fn literal_route(name: &str, lit: &str) -> Option<(String, String)> {
    if matches!(name, "Handle" | "HandleFunc") {
        if let Some((m, rest)) = lit.split_once(' ')
            && VERBS.contains(&m)
        {
            return Some((m.to_string(), rest.trim().to_string()));
        }
        return lit
            .starts_with('/')
            .then(|| (ANY_METHOD.to_string(), lit.to_string()));
    }
    if !(lit.is_empty() || lit.starts_with('/')) {
        return None;
    }
    let upper = name.to_ascii_uppercase();
    let method = if VERBS.contains(&upper.as_str()) {
        upper
    } else {
        ANY_METHOD.into()
    };
    Some((method, lit.to_string()))
}

impl<'a> Interp<'a> {
    pub(super) fn sweep(&mut self) {
        let srcs = self.srcs;
        for &i in self.mine {
            let s = &srcs[i];
            let http = s.imports.values().any(|v| v == "net/http");
            let kind = kind_of(s);
            if kind.is_none() && !http {
                continue;
            }
            for (at, name, first) in call_sites(s) {
                if self.seen.contains(&(i, at)) {
                    continue;
                }
                let handle = matches!(name.as_str(), "Handle" | "HandleFunc");
                if kind.is_none() && !handle {
                    continue;
                }
                self.sweep_site(i, kind.unwrap_or(Kind::Http), at, &name, first);
            }
        }
    }

    fn sweep_site(
        &mut self,
        src: usize,
        kind: Kind,
        at: usize,
        name: &str,
        first: Option<(usize, usize)>,
    ) {
        let s = &self.srcs[src];
        let lit = first.and_then(|r| s.lit(r));
        let route = match &lit {
            Some(l) => literal_route(name, l),
            None => None,
        };
        let loud = VERBS.contains(&name) || matches!(name, "Handle" | "HandleFunc" | "Any");
        if route.is_none() && !(lit.is_none() && first.is_some() && loud) {
            return;
        }
        let m = format!(
            "Route call .{name}(..) is on a router that was not followed (struct field, package variable, closure or interface); its access is unknown."
        );
        self.gap(src, at, m);
        let Some((method, path)) = route else { return };
        let id = self.new_router(kind);
        let unknown = AccessState::Protected(Condition::Unknown("router not followed".into()));
        let cx = Cx {
            state: unknown,
            skip: vec![],
            casbin: None,
        };
        let reg = Reg {
            methods: vec![method],
            path,
            mws: vec![cx],
            handler: None,
        };
        self.register(src, id, at, reg);
    }
}
