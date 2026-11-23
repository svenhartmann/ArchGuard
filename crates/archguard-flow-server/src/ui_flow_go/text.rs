//! Go source text helpers: masking, bracket matching, statements, call chains.
use std::collections::BTreeMap;

/// One element of `a.b(x).c`: its name and the byte range inside the
/// parentheses (`at` is the offset of the name or of the `(`).
#[derive(Debug, Clone)]
pub(super) struct Seg {
    pub(super) name: String,
    pub(super) at: usize,
    pub(super) args: Option<(usize, usize)>,
}

pub(super) struct Src {
    pub(super) rel: String,
    pub(super) raw: String,
    pub(super) code: String,
    pub(super) imports: BTreeMap<String, String>,
}

#[derive(Clone)]
pub(super) struct Func {
    pub(super) src: usize,
    pub(super) name: String,
    pub(super) params: (usize, usize),
    pub(super) body: (usize, usize),
}

/// `raw` with comments and the contents of string and rune literals blanked
/// (same length, newlines kept).
pub(super) fn mask(raw: &str) -> String {
    let b = raw.as_bytes();
    let mut out = b.to_vec();
    let blank = |out: &mut Vec<u8>, from: usize, to: usize| {
        for c in &mut out[from..to] {
            if *c != b'\n' {
                *c = b' ';
            }
        }
    };
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                let end = b[i..]
                    .iter()
                    .position(|&c| c == b'\n')
                    .map_or(b.len(), |p| i + p);
                blank(&mut out, i, end);
                i = end;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let end = raw[i + 2..].find("*/").map_or(b.len(), |p| i + 2 + p + 2);
                blank(&mut out, i, end);
                i = end;
            }
            q @ (b'"' | b'\'') => {
                let mut j = i + 1;
                while j < b.len() && b[j] != q && b[j] != b'\n' {
                    j += if b[j] == b'\\' { 2 } else { 1 };
                }
                let j = j.min(b.len());
                blank(&mut out, i + 1, j);
                i = j + 1;
            }
            b'`' => {
                let end = b[i + 1..]
                    .iter()
                    .position(|&c| c == b'`')
                    .map_or(b.len(), |p| i + 1 + p);
                blank(&mut out, i + 1, end);
                i = end + 1;
            }
            _ => i += 1,
        }
    }
    String::from_utf8(out).unwrap_or_default()
}

pub(super) fn close_of(code: &str, open: usize) -> Option<usize> {
    let b = code.as_bytes();
    let mut depth = 0usize;
    for (i, &c) in b.iter().enumerate().skip(open) {
        match c {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// `[start, end)` split at depth-0 `sep`, each part trimmed; empty parts dropped.
pub(super) fn split_top(code: &str, start: usize, end: usize, sep: u8) -> Vec<(usize, usize)> {
    let b = code.as_bytes();
    let (mut parts, mut depth, mut s) = (Vec::new(), 0i32, start);
    let mut push = |s: usize, e: usize| {
        let t = &code[s..e];
        let lead = t.len() - t.trim_start().len();
        let t = t.trim();
        if !t.is_empty() {
            parts.push((s + lead, s + lead + t.len()));
        }
    };
    for (i, &c) in b.iter().enumerate().take(end).skip(start) {
        match c {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            c if c == sep && depth == 0 => {
                push(s, i);
                s = i + 1;
            }
            _ => {}
        }
    }
    push(s, end);
    parts
}

/// Statements of a block: split at depth-0 `;` and at newlines after a token
/// that can end a statement.
pub(super) fn stmts(code: &str, start: usize, end: usize) -> Vec<(usize, usize)> {
    let b = code.as_bytes();
    let (mut out, mut depth, mut s) = (Vec::new(), 0i32, start);
    let push = |s: usize, e: usize, out: &mut Vec<(usize, usize)>| {
        let t = code[s..e].trim();
        if !t.is_empty() {
            let lead = code[s..e].len() - code[s..e].trim_start().len();
            out.push((s + lead, s + lead + t.len()));
        }
    };
    for i in start..end {
        match b[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b';' if depth == 0 => {
                push(s, i, &mut out);
                s = i + 1;
            }
            b'\n' if depth == 0 => {
                let last = code[s..i].trim_end().bytes().last();
                if last.is_some_and(|c| c.is_ascii_alphanumeric() || b"_)]}\"`'".contains(&c)) {
                    push(s, i, &mut out);
                    s = i + 1;
                }
            }
            _ => {}
        }
    }
    push(s, end, &mut out);
    out
}

pub(super) fn is_ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| c.is_alphanumeric() || c == '_')
        && !s.starts_with(|c: char| c.is_ascii_digit())
}

fn ident_end(b: &[u8], mut j: usize, end: usize) -> usize {
    while j < end && (b[j].is_ascii_alphanumeric() || b[j] == b'_' || b[j] >= 0x80) {
        j += 1;
    }
    j
}

/// A call `(..)` after the last segment (its arguments) or a call of a call.
fn push_call(code: &str, segs: &mut Vec<Seg>, open: usize, end: usize) -> Option<usize> {
    let c = close_of(code, open).filter(|&c| c < end)?;
    let args = Some((open + 1, c));
    match segs.last_mut() {
        Some(l) if l.args.is_none() && l.at + l.name.len() == open => l.args = args,
        _ => segs.push(Seg {
            name: String::new(),
            at: open,
            args,
        }),
    }
    Some(c + 1)
}

pub(super) fn chain(code: &str, start: usize, end: usize) -> Option<Vec<Seg>> {
    let b = code.as_bytes();
    let e = ident_end(b, start, end);
    if e == start {
        return None;
    }
    let mut segs = vec![Seg {
        name: code[start..e].into(),
        at: start,
        args: None,
    }];
    let mut i = e;
    while i < end {
        match b[i] {
            b'(' => i = push_call(code, &mut segs, i, end)?,
            b'.' => {
                let j = skip_ws(code, i + 1);
                let e = ident_end(b, j, end);
                if e == j {
                    return None;
                }
                segs.push(Seg {
                    name: code[j..e].into(),
                    at: j,
                    args: None,
                });
                i = e;
            }
            c if c.is_ascii_whitespace() => i += 1,
            _ => return None,
        }
    }
    Some(segs)
}

pub(super) fn lines_before(raw: &str, off: usize) -> u32 {
    (raw.as_bytes()[..off.min(raw.len())]
        .iter()
        .filter(|&&c| c == b'\n')
        .count()
        + 1) as u32
}

pub(super) fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The package name an import path gets without an alias.
pub(super) fn default_name(path: &str) -> String {
    let mut parts: Vec<&str> = path.split('/').collect();
    while parts.len() > 1
        && parts.last().is_some_and(|p| {
            p.len() > 1 && p.starts_with('v') && p[1..].bytes().all(|c| c.is_ascii_digit())
        })
    {
        parts.pop();
    }
    let last = parts.last().copied().unwrap_or("");
    let last = last.split(".v").next().unwrap_or(last);
    last.replace(['-', '.'], "")
}

pub(super) fn skip_ws(code: &str, mut j: usize) -> usize {
    let b = code.as_bytes();
    while j < b.len() && b[j].is_ascii_whitespace() {
        j += 1;
    }
    j
}

/// `(name, parameter parentheses offset)` of a declaration at `at` (`func`),
/// receiver skipped.
fn decl(code: &str, at: usize) -> Option<(String, usize)> {
    let b = code.as_bytes();
    let mut j = skip_ws(code, at + 5);
    if b.get(j) == Some(&b'(') {
        j = skip_ws(code, close_of(code, j)? + 1);
    }
    let ne = (j..b.len())
        .find(|&k| !(b[k].is_ascii_alphanumeric() || b[k] == b'_'))
        .unwrap_or(b.len());
    let po = skip_ws(code, ne);
    (ne > j && b.get(po) == Some(&b'(')).then(|| (code[j..ne].to_string(), po))
}

/// The `{` of a body on the line of the closing parenthesis; `interface{}`
/// and `struct{}` in the result types are skipped.
fn body_open(code: &str, mut k: usize) -> Option<usize> {
    let line_end = code[k..].find('\n').map_or(code.len(), |p| k + p);
    while let Some(p) = code[k..line_end].find('{') {
        let at = k + p;
        let before = code[..at].trim_end();
        if !(before.ends_with("interface") || before.ends_with("struct")) {
            return Some(at);
        }
        k = close_of(code, at)? + 1;
        if k >= line_end {
            return None;
        }
    }
    None
}

pub(super) fn funcs_of(i: usize, s: &Src) -> Vec<Func> {
    let code = &s.code;
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(p) = code[from..].find("func ") {
        let at = from + p;
        from = at + 5;
        if at > 0 && code.as_bytes()[at - 1] != b'\n' {
            continue;
        }
        let Some((name, po)) = decl(code, at) else {
            continue;
        };
        let Some(pc) = close_of(code, po) else {
            continue;
        };
        let Some(bo) = body_open(code, pc + 1) else {
            continue;
        };
        if let Some(bc) = close_of(code, bo) {
            let (params, body) = ((po + 1, pc), (bo + 1, bc));
            out.push(Func {
                src: i,
                name,
                params,
                body,
            });
        }
    }
    out
}

/// The text of `key: value` inside `[a.0, a.1)` (value up to the next comma).
pub(super) fn field(code: &str, a: (usize, usize), key: &str) -> Option<(usize, usize)> {
    let mut from = a.0;
    while let Some(p) = code[from..a.1].find(key) {
        let at = from + p;
        from = at + key.len();
        let ok_before = at == 0
            || !code.as_bytes()[at - 1].is_ascii_alphanumeric() && code.as_bytes()[at - 1] != b'_';
        let rest = code[from..a.1].trim_start();
        if ok_before && rest.starts_with(':') {
            let vs = a.1 - rest.len() + 1;
            let end = split_top(code, vs, a.1, b',').first().map_or(vs, |v| v.1);
            let t = code[vs..end].trim_start();
            return Some((end - t.len(), end));
        }
    }
    None
}

/// The literal paths a `func(c) bool { return c.Path() == "/x" || .. }` skips.
pub(super) fn skipper_paths(s: &Src, v: (usize, usize)) -> Option<Vec<String>> {
    let code = &s.code;
    let t = &code[v.0..v.1];
    if !t.starts_with("func") {
        return None;
    }
    let bo = v.0 + t.find('{')?;
    let bc = close_of(code, bo)?;
    let body = code[bo + 1..bc].trim();
    body.strip_prefix("return ")?;
    let off = bo + 1 + code[bo + 1..bc].find("return ")? + 7;
    let mut out = vec![];
    for term in split_or(code, off, bc) {
        let tt = &code[term.0..term.1];
        let (l, _) = tt.split_once("==")?;
        if !l.contains("Path") {
            return None;
        }
        let eq = term.0 + tt.find("==")? + 2;
        out.push(s.lit((eq, term.1))?);
    }
    Some(out)
}

pub(super) fn split_or(code: &str, s: usize, e: usize) -> Vec<(usize, usize)> {
    let mut out = vec![];
    let mut from = s;
    loop {
        let rest = &code[from..e];
        match rest.find("||") {
            Some(p) => {
                out.push((from, from + p));
                from += p + 2;
            }
            None => {
                out.push((from, e));
                break;
            }
        }
    }
    out.into_iter()
        .map(|(a, b)| {
            (
                a + code[a..b].len() - code[a..b].trim_start().len(),
                a + code[a..b].trim_end().len(),
            )
        })
        .collect()
}

impl Src {
    pub(super) fn new(rel: String, raw: String) -> Self {
        let code = mask(&raw);
        let mut imports = BTreeMap::new();
        let mut block = false;
        for line in raw.lines() {
            let t = line.trim();
            let spec = if block && t == ")" {
                block = false;
                continue;
            } else if block {
                t
            } else if t.starts_with("import (") {
                block = true;
                continue;
            } else if let Some(r) = t.strip_prefix("import ") {
                r.trim()
            } else {
                continue;
            };
            if let (Some(a), Some(b)) = (spec.find('"'), spec.rfind('"')) {
                let (a, b) = if a < b { (a, b) } else { continue };
                let alias = spec[..a].trim();
                let path = &spec[a + 1..b];
                let name = if alias.is_empty() {
                    default_name(path)
                } else {
                    alias.to_string()
                };
                imports.insert(name, path.to_string());
            }
        }
        Src {
            rel,
            raw,
            code,
            imports,
        }
    }

    pub(super) fn text(&self, r: (usize, usize)) -> &str {
        &self.raw[r.0..r.1]
    }

    /// A string literal at `r` (a whole `"..."` expression without escapes).
    pub(super) fn lit(&self, r: (usize, usize)) -> Option<String> {
        let t = self.text(r).trim();
        if let Some(raw) = t.strip_prefix('`').and_then(|x| x.strip_suffix('`')) {
            return (!raw.contains('`')).then(|| raw.to_string());
        }
        let inner = t.strip_prefix('"')?.strip_suffix('"')?;
        (!inner.contains('"') && !inner.contains('\\')).then(|| inner.to_string())
    }
}
