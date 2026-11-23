//! Statements in source order.
use super::model::*;
use super::text::*;

/// The first `:=` or lone `=` at depth 0: `(start of operator, end of operator)`.
fn assign_split(code: &str, s: usize, e: usize) -> Option<(usize, usize)> {
    let b = code.as_bytes();
    let mut depth = 0i32;
    for i in s..e {
        match b[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b':' if depth == 0 && b.get(i + 1) == Some(&b'=') => return Some((i, i + 2)),
            b'=' if depth == 0 && b.get(i + 1) != Some(&b'=') => {
                let prev = b[i.saturating_sub(1)];
                if !b"=!<>+-*/".contains(&prev) {
                    return Some((i, i + 1));
                }
            }
            _ => {}
        }
    }
    None
}

/// The variable a statement's left side assigns (the first name).
fn assigned_name(lhs: &str) -> String {
    let lhs = lhs.trim();
    let lhs = lhs.strip_prefix("var ").unwrap_or(lhs);
    let first = lhs.split(',').next().unwrap_or("");
    first.split_whitespace().next().unwrap_or("").to_string()
}

impl<'a> Interp<'a> {
    pub(super) fn block(&mut self, fr: &mut Frame, src: usize, range: (usize, usize)) {
        let code = &self.srcs[src].code;
        for (s, e) in stmts(code, range.0, range.1) {
            self.stmt(fr, src, (s, e));
        }
    }

    pub(super) fn stmt(&mut self, fr: &mut Frame, src: usize, (mut s, e): (usize, usize)) {
        let srcs = self.srcs;
        let code = &srcs[src].code;
        let mut t = &code[s..e];
        if (t.starts_with("case ") || t.starts_with("default:"))
            && let Some(p) = t.find(':')
        {
            s += p + 1;
            t = code[s..e].trim_start();
            s = e - t.len();
        }
        let word = t.split(|c: char| !c.is_alphanumeric()).next().unwrap_or("");
        match word {
            "return" => self.return_stmt(fr, src, (s + 6, e)),
            "if" | "for" | "switch" | "select" => self.compound(fr, src, (s, e), true),
            "go" | "defer" => self.compound(fr, src, (s, e), false),
            _ => match assign_split(code, s, e) {
                Some(split) => self.assign(fr, src, (s, e), split),
                None => {
                    self.eval(fr, src, (s, e));
                }
            },
        }
    }

    fn return_stmt(&mut self, fr: &mut Frame, src: usize, rest: (usize, usize)) {
        let code = &self.srcs[src].code;
        if let Some(&first) = split_top(code, rest.0, rest.1, b',').first()
            && let Some(id) = self.eval(fr, src, first)
        {
            fr.ret = Some(id);
        }
    }

    /// The brace blocks of an `if`/`for`/`switch`/`go` statement; what they
    /// register is conditional for the first three.
    fn compound(&mut self, fr: &mut Frame, src: usize, (s, e): (usize, usize), cond: bool) {
        let code = &self.srcs[src].code;
        let mut i = s;
        while i < e {
            if code.as_bytes()[i] == b'{'
                && let Some(c) = close_of(code, i).filter(|&c| c < e)
            {
                let mut inner = fr.clone();
                inner.cond = fr.cond || cond;
                self.block(&mut inner, src, (i + 1, c));
                i = c;
            }
            i += 1;
        }
    }

    fn assign(
        &mut self,
        fr: &mut Frame,
        src: usize,
        (s, e): (usize, usize),
        (l_end, r_start): (usize, usize),
    ) {
        let srcs = self.srcs;
        let code = &srcs[src].code;
        let name = assigned_name(&code[s..l_end]);
        let rt = code[r_start..e].trim_start();
        let rhs = (e - rt.len(), e);
        if name.is_empty() || name == "_" {
            self.eval(fr, src, rhs);
            return;
        }
        if self.assign_special(fr, src, &name, rhs) {
            return;
        }
        if let Some(id) = self.eval(fr, src, rhs) {
            fr.vars.insert(name, id);
        } else if let Some(h) = self.unwrap_server(fr, src, rhs) {
            let bound = fr.vars.get(&name).copied();
            if bound.is_none() || bound == Some(h.0) {
                fr.vars.insert(name, h.0);
            }
            self.apply_server(h);
        }
    }

    /// `e := casbin.NewEnforcer(..)` and `srv := &http.Server{Handler: ..}`.
    fn assign_special(
        &mut self,
        fr: &mut Frame,
        src: usize,
        name: &str,
        rhs: (usize, usize),
    ) -> bool {
        let code = &self.srcs[src].code;
        let rt = &code[rhs.0..rhs.1];
        if let Some(p) = rt.find("NewEnforcer(") {
            let at = rhs.0 + p + 11;
            if let Some(c) = close_of(code, at) {
                self.enforcer(src, name.to_string(), (at + 1, c), rhs.0);
                return true;
            }
        }
        if rt.contains("Server{") {
            if let Some(h) = self.server_handler(fr, src, rhs) {
                self.apply_server(h);
            }
            return true;
        }
        false
    }

    pub(super) fn apply_server(&mut self, (id, wrappers): (usize, Vec<Cx>)) {
        self.routers[id].wrap.splice(0..0, wrappers);
    }
}
