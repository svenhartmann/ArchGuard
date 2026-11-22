//! Expression evaluation and calls of module functions.
use super::model::*;
use super::text::*;

fn dir_of(rel: &str) -> &str {
    rel.rsplit_once('/').map_or("", |x| x.0)
}

/// The longest dotted variable at the start of a chain: the router and the
/// number of segments it uses.
fn var_head(fr: &Frame, segs: &[Seg]) -> Option<(usize, usize)> {
    let mut key = segs[0].name.clone();
    let mut best = fr.vars.get(&key).map(|&id| (id, 1));
    for (j, sg) in segs.iter().enumerate().skip(1) {
        if segs[j - 1].args.is_some() || sg.name.is_empty() {
            break;
        }
        key = format!("{key}.{}", sg.name);
        if let Some(&id) = fr.vars.get(&key) {
            best = Some((id, j + 1));
        }
    }
    best
}

type Head = Option<(Option<usize>, usize)>;

impl<'a> Interp<'a> {
    /// Runs the registrations of a call chain; the router it ends in.
    pub(super) fn eval(
        &mut self,
        fr: &mut Frame,
        src: usize,
        (s, e): (usize, usize),
    ) -> Option<usize> {
        let srcs = self.srcs;
        let code = &srcs[src].code;
        let s = e - code[s..e].trim_start().len();
        let segs = chain(code, s, e)?;
        let (cursor, i) = match var_head(fr, &segs) {
            Some((id, n)) => (Some(id), n),
            None => self.head(fr, src, &segs)?,
        };
        self.walk_chain(fr, src, &segs, i, cursor)
    }

    /// The first segments of a chain that is not a known variable: a
    /// constructor, a call of a function of the module, the default mux.
    pub(super) fn head(&mut self, fr: &mut Frame, src: usize, segs: &[Seg]) -> Head {
        let pkg = if segs[0].args.is_none() && segs.len() > 1 {
            self.head_pkg(fr, src, segs)
        } else {
            None
        };
        let found = match pkg {
            Some(f) => Some(f),
            None if segs[0].args.is_some() => self.head_local(fr, src, segs),
            None => self.head_method(fr, src, segs),
        };
        if found.is_some() {
            return found;
        }
        // unresolved call: its arguments may still hold registrations
        let code = &self.srcs[src].code;
        for (a, b) in segs.iter().filter_map(|sg| sg.args) {
            for r in split_top(code, a, b, b',') {
                self.eval(fr, src, r);
            }
        }
        None
    }

    fn find_func(&self, name: &str, dir: &str) -> Option<Func> {
        let srcs = self.srcs;
        let same = |f: &&Func| f.name == name && dir_of(&srcs[f.src].rel) == dir;
        self.funcs.iter().find(same).cloned()
    }

    /// `pkg.F(..)`: a constructor, `http.Handle`, `http.ListenAndServe`, a
    /// function of another package of the module.
    fn head_pkg(&mut self, fr: &mut Frame, src: usize, segs: &[Seg]) -> Head {
        let srcs = self.srcs;
        let path = srcs[src].imports.get(&segs[0].name)?;
        let call = &segs[1];
        let args = call.args?;
        if let Some(kind) = constructor(path, &call.name) {
            return Some((Some(self.new_router(kind)), 2));
        }
        if path == "net/http"
            && let Some(found) = self.http_head(fr, src, &call.name, args)
        {
            return Some(found);
        }
        let f = self.find_func(&call.name, &self.import_dir(path)?)?;
        let r = self.call(fr, src, &f, args);
        Some((r, 2))
    }

    fn http_head(&mut self, fr: &mut Frame, src: usize, name: &str, args: (usize, usize)) -> Head {
        if matches!(name, "Handle" | "HandleFunc") {
            return Some((Some(self.default_mux_id()), 1));
        }
        if name.starts_with("ListenAndServe") || name == "Serve" {
            let a = split_top(&self.srcs[src].code, args.0, args.1, b',');
            if let Some(&h) = a.get(1)
                && let Some(h) = self.unwrap_server(fr, src, h)
            {
                self.apply_server(h);
            }
            return Some((None, 2));
        }
        None
    }

    /// `F(..)`: a function of the same package.
    fn head_local(&mut self, fr: &mut Frame, src: usize, segs: &[Seg]) -> Head {
        let dir = dir_of(&self.srcs[src].rel);
        let f = self.find_func(&segs[0].name, dir)?;
        let r = self.call(fr, src, &f, segs[0].args?);
        Some((r, 1))
    }

    /// `value.Method(..)` where exactly one method of that name exists.
    fn head_method(&mut self, fr: &mut Frame, src: usize, segs: &[Seg]) -> Head {
        let srcs = self.srcs;
        let call = segs
            .get(1)
            .filter(|c| c.args.is_some() && !c.name.is_empty())?;
        let is_method = |f: &&Func| {
            let code = &srcs[f.src].code[..f.body.0];
            code.rfind("func")
                .is_some_and(|p| srcs[f.src].code[p + 4..].trim_start().starts_with('('))
        };
        let named: Vec<&Func> = self
            .funcs
            .iter()
            .filter(|f| f.name == call.name)
            .filter(is_method)
            .collect();
        if named.len() != 1 || srcs[src].imports.contains_key(&segs[0].name) {
            return None;
        }
        let f = named[0].clone();
        let r = self.call(fr, src, &f, call.args?);
        Some((r, 2))
    }
    pub(super) fn import_dir(&self, path: &str) -> Option<String> {
        let rest = path
            .strip_prefix(&self.module.path)?
            .trim_start_matches('/');
        Some(match (self.module.dir.as_str(), rest) {
            ("", r) => r.to_string(),
            (d, "") => d.to_string(),
            (d, r) => format!("{d}/{r}"),
        })
    }

    /// Inlines a call of a module function; the router it returns.
    pub(super) fn call(
        &mut self,
        fr: &mut Frame,
        src: usize,
        f: &Func,
        args: (usize, usize),
    ) -> Option<usize> {
        if self.depth >= MAX_DEPTH {
            self.gap(
                src,
                args.0,
                format!("Call of {} not followed: nesting too deep.", f.name),
            );
            return None;
        }
        let srcs = self.srcs;
        let given = split_top(&srcs[src].code, args.0, args.1, b',');
        let pieces = split_top(&srcs[f.src].code, f.params.0, f.params.1, b',');
        let mut inner = Frame {
            cond: fr.cond,
            ..Default::default()
        };
        for (k, a) in given.iter().enumerate() {
            let bound = self.eval(fr, src, *a);
            let Some(id) = bound else { continue };
            let Some(p) = pieces.get(k) else { continue };
            let words: Vec<&str> = srcs[f.src].code[p.0..p.1].split_whitespace().collect();
            let later_named = pieces[k + 1..]
                .iter()
                .any(|q| srcs[f.src].code[q.0..q.1].split_whitespace().count() > 1);
            if words.len() > 1 || later_named {
                inner.vars.insert(words[0].to_string(), id);
            }
        }
        self.depth += 1;
        self.block(&mut inner, f.src, f.body);
        self.depth -= 1;
        inner.ret
    }

    pub(super) fn closure(&mut self, fr: &mut Frame, src: usize, r: (usize, usize), router: usize) {
        let srcs = self.srcs;
        let code = &srcs[src].code;
        let Some(po) = code[r.0..r.1].find('(').map(|p| r.0 + p) else {
            return;
        };
        let Some(pc) = close_of(code, po) else { return };
        let Some(bo) = code[pc..r.1].find('{').map(|p| pc + p) else {
            return;
        };
        let Some(bc) = close_of(code, bo) else { return };
        let name = code[po + 1..pc]
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_string();
        let mut inner = fr.clone();
        if is_ident(&name) {
            inner.vars.insert(name, router);
        }
        self.block(&mut inner, src, (bo + 1, bc));
    }

    /// A route path: literals, constants of the package, `+` joins. The text
    /// holds `{name}` for a part that is not a literal (a gap).
    pub(super) fn path_arg(
        &mut self,
        src: usize,
        r: Option<(usize, usize)>,
        at: usize,
    ) -> (String, bool) {
        let Some(r) = r else {
            return (String::new(), true);
        };
        let srcs = self.srcs;
        let s = &srcs[src];
        let mut out = String::new();
        let mut ok = true;
        for part in split_top(&s.code, r.0, r.1, b'+') {
            if let Some(l) = s.lit(part) {
                out.push_str(&l);
            } else if let Some(c) = self.consts.get(s.code[part.0..part.1].trim()) {
                out.push_str(c);
            } else {
                ok = false;
                out.push_str(&format!("{{{}}}", collapse(&s.raw[part.0..part.1])));
            }
        }
        if !ok {
            self.gap(
                src,
                at,
                format!(
                    "Route path {} is not a literal; the endpoint path is a placeholder.",
                    collapse(&s.raw[r.0..r.1])
                ),
            );
        }
        (out, ok)
    }
}
