//! TypeScript and JavaScript reading shared by the Angular, React and Vue
//! extractors (ADR 0021): a literal parser, lexical helpers and `import`
//! clauses. Text level, no TypeScript parser.
use std::collections::BTreeSet;

/// Placeholder for a route or URL segment that is not a literal.
pub const PARAM: &str = ":param";
/// Nesting bound for parsed literals.
pub const MAX_DEPTH: usize = 32;

/// A JavaScript literal as far as this scanner reads it; `at..end` is its source.
pub struct Item {
    pub at: usize,
    pub end: usize,
    pub value: Value,
}

impl Item {
    pub fn source(&self, text: &str) -> String {
        text.get(self.at..self.end).unwrap_or("").trim().to_string()
    }
}

pub enum Value {
    Str(String),
    /// Template literal with `${…}` holes, raw.
    Tpl(String),
    Arr(Vec<Item>),
    Obj(Vec<(String, Item)>),
    /// Any other expression (also JSX), as source text.
    Raw(String),
}

impl Value {
    /// A string literal, or a template literal that starts literal, holes as `:param`.
    pub fn literal(&self) -> Option<String> {
        match self {
            Value::Str(s) => Some(s.clone()),
            Value::Tpl(t) => fill(t, |_| None),
            _ => None,
        }
    }

    /// A literal, or a link array joined: the first part must be readable, later
    /// unread parts become `:param`, objects (matrix parameters) are dropped. A name
    /// and a template hole are read through `constant`.
    pub fn link(&self, constant: &dyn Fn(&str) -> Option<String>) -> Option<String> {
        let part = |v: &Value| match v {
            Value::Str(s) => Some(s.clone()),
            Value::Tpl(t) => fill(t, constant),
            Value::Raw(r) if is_name(r) => constant(r),
            _ => None,
        };
        let Value::Arr(items) = self else {
            return part(self);
        };
        let (first, rest) = items.split_first()?;
        let mut out = vec![part(&first.value)?];
        for item in rest {
            match (&item.value, part(&item.value)) {
                (_, Some(s)) => out.push(s),
                (Value::Obj(_), None) => {}
                _ => out.push(PARAM.into()),
            }
        }
        Some(out.join("/"))
    }
}

pub struct Parser<'a> {
    pub s: &'a str,
    pub i: usize,
}

impl Parser<'_> {
    pub fn peek(&self) -> u8 {
        self.s.as_bytes().get(self.i).copied().unwrap_or(0)
    }

    /// Skips whitespace and comments.
    pub fn ws(&mut self) {
        let b = self.s.as_bytes();
        loop {
            self.i = skip_ws(b, self.i);
            let rest = b.get(self.i..).unwrap_or_default();
            self.i = if rest.starts_with(b"//") {
                self.s[self.i..].find('\n').map_or(b.len(), |e| self.i + e)
            } else if rest.starts_with(b"/*") {
                self.s[self.i + 2..]
                    .find("*/")
                    .map_or(b.len(), |e| self.i + 2 + e + 2)
            } else {
                return;
            };
        }
    }

    /// Offset after the string literal at the cursor; the end for an open one.
    pub fn string_end(&self) -> usize {
        let b = self.s.as_bytes();
        let quote = b[self.i];
        let mut i = self.i + 1;
        while i < b.len() {
            match b[i] {
                b'\\' => i += 2,
                c if c == quote => return i + 1,
                _ => i += 1,
            }
        }
        b.len()
    }

    /// Content of the string literal at the cursor; moves past it.
    pub fn string(&mut self) -> String {
        let at = self.i;
        self.i = self.string_end();
        self.s.get(at + 1..self.i - 1).unwrap_or("").to_string()
    }

    /// One value; anything followed by more than a separator is `Raw`.
    pub fn value(&mut self, depth: usize) -> Item {
        self.ws();
        let at = self.i;
        let value = match self.peek() {
            b'[' if depth < MAX_DEPTH => Value::Arr(self.array(depth + 1)),
            b'{' if depth < MAX_DEPTH => Value::Obj(self.object(depth + 1)),
            q @ (b'\'' | b'"' | b'`') => {
                let inner = self.string();
                if q == b'`' && inner.contains("${") {
                    Value::Tpl(inner)
                } else {
                    Value::Str(inner)
                }
            }
            _ => return self.raw(at),
        };
        let end = self.i;
        self.ws();
        if matches!(self.peek(), b',' | b']' | b'}' | b')' | b';' | 0) {
            Item { at, end, value }
        } else {
            self.raw(at)
        }
    }

    /// Source text up to the next separator outside brackets and strings.
    pub fn raw(&mut self, at: usize) -> Item {
        let b = self.s.as_bytes();
        self.i = at;
        let mut depth = 0usize;
        while self.i < b.len() {
            match b[self.i] {
                b'\'' | b'"' | b'`' => {
                    self.i = self.string_end();
                    continue;
                }
                b'/' if matches!(b.get(self.i + 1), Some(b'/' | b'*')) => {
                    self.ws();
                    continue;
                }
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' if depth == 0 => break,
                b')' | b']' | b'}' => depth -= 1,
                b',' | b';' if depth == 0 => break,
                _ => {}
            }
            self.i += 1;
        }
        let text = self.s[at..self.i].trim_end();
        Item {
            at,
            end: at + text.len(),
            value: Value::Raw(text.into()),
        }
    }

    /// Items of the array whose `[` is at the cursor.
    pub fn array(&mut self, depth: usize) -> Vec<Item> {
        self.i += 1;
        let mut items = Vec::new();
        loop {
            self.ws();
            match self.peek() {
                b']' => {
                    self.i += 1;
                    break;
                }
                0 => break,
                _ => {}
            }
            let item = self.value(depth);
            if item.end > item.at {
                items.push(item);
            }
            self.ws();
            match self.peek() {
                b',' => self.i += 1,
                b']' => {}
                _ => break,
            }
        }
        items
    }

    /// Fields of the object whose `{` is at the cursor; shorthand and methods are `Raw`.
    pub fn object(&mut self, depth: usize) -> Vec<(String, Item)> {
        self.i += 1;
        let mut fields = Vec::new();
        loop {
            self.ws();
            match self.peek() {
                b'}' => {
                    self.i += 1;
                    break;
                }
                0 => break,
                _ => {}
            }
            let at = self.i;
            let key = if matches!(self.peek(), b'\'' | b'"') {
                self.string()
            } else {
                let key = ident_at(self.s, at).to_string();
                self.i += key.len();
                key
            };
            self.ws();
            let item = match self.peek() {
                b':' if !key.is_empty() => {
                    self.i += 1;
                    self.value(depth)
                }
                b',' | b'}' if !key.is_empty() => Item {
                    at,
                    end: at + key.len(),
                    value: Value::Raw(key.clone()),
                },
                _ => self.raw(at),
            };
            fields.push((key, item));
            self.ws();
            match self.peek() {
                b',' => self.i += 1,
                b'}' => {}
                _ => break,
            }
        }
        fields
    }
}

/// Path of a URL: scheme and host, query and fragment dropped, leading `/`.
pub fn url_path(url: &str) -> String {
    let url = url.split(['?', '#']).next().unwrap_or("");
    let url = match url.split_once("://") {
        Some((_, rest)) => rest.find('/').map_or("", |i| &rest[i..]),
        None => url,
    };
    format!("/{}", url.trim_start_matches('/'))
}

/// `dir` and `rel` joined with `.` and `..` applied; `..` stops at the top.
pub fn join(dir: &str, rel: &str) -> String {
    let mut parts: Vec<&str> = dir.split('/').filter(|s| !s.is_empty()).collect();
    for s in rel.split('/') {
        match s {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

pub fn dir(file: &str) -> &str {
    file.rsplit_once('/').map_or("", |(d, _)| d)
}

pub fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
}

/// A plain identifier.
pub fn is_ident(s: &str) -> bool {
    !s.is_empty() && !s.starts_with(|c: char| c.is_ascii_digit()) && s.chars().all(is_ident_char)
}

/// A plain identifier or dotted name, e.g. `authGuard` or `Guards.admin`.
pub fn is_name(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with(|c: char| c.is_ascii_digit())
        && s.chars().all(|c| is_ident_char(c) || c == '.')
}

pub fn ident_at(text: &str, at: usize) -> &str {
    let Some(rest) = text.get(at..) else {
        return "";
    };
    &rest[..rest.find(|c: char| !is_ident_char(c)).unwrap_or(rest.len())]
}

pub fn trailing_ident(text: &str) -> Option<String> {
    let start = text.rfind(|c: char| !is_ident_char(c)).map_or(0, |i| i + 1);
    let name = &text[start..];
    (!name.is_empty()).then(|| name.to_string())
}

/// Offsets of `word` with no identifier character on either side.
pub fn words(text: &str, word: &str) -> Vec<usize> {
    let b = text.as_bytes();
    let edge = |i: Option<&u8>| i.is_none_or(|c| !is_ident_char(*c as char));
    text.match_indices(word)
        .map(|(i, _)| i)
        .filter(|&i| (i == 0 || edge(b.get(i - 1))) && edge(b.get(i + word.len())))
        .collect()
}

pub fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while b.get(i).is_some_and(u8::is_ascii_whitespace) {
        i += 1;
    }
    i
}

/// Past a `<…>` type argument list; stops early at `(`, `;` or after 200 bytes.
pub fn skip_generic(b: &[u8], start: usize) -> usize {
    let mut depth = 0;
    for (i, c) in b.iter().enumerate().skip(start).take(200) {
        match c {
            b'<' => depth += 1,
            b'>' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            b'(' | b';' => return i,
            _ => {}
        }
    }
    start
}

pub fn line_of(text: &str, at: usize) -> u32 {
    let at = at.min(text.len());
    text.as_bytes()[..at]
        .iter()
        .filter(|&&c| c == b'\n')
        .count() as u32
        + 1
}

pub fn idents(text: &str) -> BTreeSet<&str> {
    text.split(|c: char| !is_ident_char(c))
        .filter(|w| !w.is_empty())
        .collect()
}

/// The top-level `+` operands of an expression; one part when there is none.
pub fn split_plus(raw: &str) -> Vec<&str> {
    let b = raw.as_bytes();
    let mut parts = Vec::new();
    let (mut depth, mut start, mut i) = (0usize, 0, 0);
    while i < b.len() {
        match b[i] {
            b'\'' | b'"' | b'`' => {
                i = Parser { s: raw, i }.string_end();
                continue;
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b'+' if depth == 0 => {
                if matches!(b.get(i + 1), Some(b'+' | b'=')) {
                    return vec![raw];
                }
                parts.push(raw[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    parts.push(raw[start..].trim());
    if parts.iter().any(|p| p.is_empty()) {
        return vec![raw];
    }
    parts
}

/// A template literal's text with each `${…}` hole replaced by `value(inner)`, or
/// `:param` when that is `None`; `None` when the literal starts with an unread hole.
pub fn fill(t: &str, mut value: impl FnMut(&str) -> Option<String>) -> Option<String> {
    let mut out = String::new();
    let mut rest = t;
    let mut first = true;
    while let Some(i) = rest.find("${") {
        out.push_str(&rest[..i]);
        let mut depth = 0;
        let mut end = rest.len();
        for (j, c) in rest[i + 1..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i + 1 + j + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        let inner = rest.get(i + 2..end.saturating_sub(1)).unwrap_or("");
        match value(inner) {
            Some(v) => out.push_str(&v),
            None if first && out.is_empty() => return None,
            None => out.push_str(PARAM),
        }
        first = false;
        rest = &rest[end..];
    }
    out.push_str(rest);
    Some(out)
}

/// [`fill`], but `None` when any hole stays unread.
pub fn fill_all(t: &str, value: impl Fn(&str) -> Option<String>) -> Option<String> {
    let mut all = true;
    let filled = fill(t, |hole| {
        let v = value(hole);
        all &= v.is_some();
        v
    });
    filled.filter(|_| all)
}

/// `name(args)`: the called name and its arguments.
pub fn call_parts(text: &str) -> Option<(&str, Vec<Item>)> {
    let open = text.find('(')?;
    let name = text[..open].trim();
    if !is_name(name) || !text.trim_end().ends_with(')') {
        return None;
    }
    Some((name, Parser { s: text, i: open }.array(0)))
}

/// Calls `on(offset, byte, depth before)` for every `open`/`close` byte outside strings
/// and comments from `from` on, until it returns `true`.
pub fn scan_brackets(
    text: &str,
    from: usize,
    (open, close): (u8, u8),
    mut on: impl FnMut(usize, u8, usize) -> bool,
) {
    let b = text.as_bytes();
    let mut i = from;
    let mut depth = 0usize;
    while i < b.len() {
        match b[i] {
            b'\'' | b'"' | b'`' => {
                i = Parser { s: text, i }.string_end();
                continue;
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                i = text[i..].find('\n').map_or(b.len(), |e| i + e);
                continue;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i = text[i + 2..].find("*/").map_or(b.len(), |e| i + 2 + e + 2);
                continue;
            }
            c if c == open => {
                if on(i, c, depth) {
                    return;
                }
                depth += 1;
            }
            c if c == close => {
                if on(i, c, depth) {
                    return;
                }
                depth = depth.saturating_sub(1);
            }
            _ => {}
        }
        i += 1;
    }
}

/// Offset after the bracket that closes the one at `at`; the end when unclosed.
pub fn block_end(text: &str, at: usize, open: u8, close: u8) -> usize {
    let mut end = text.len();
    scan_brackets(text, at, (open, close), |i, c, depth| {
        let done = c == close && depth == 1;
        if done {
            end = i + 1;
        }
        done
    });
    end
}

/// Offset after the bracket that closes the one at `at`, skipping strings and
/// comments; brackets of any kind count together.
pub fn close(text: &str, at: usize) -> usize {
    let b = text.as_bytes();
    let (mut i, mut depth) = (at, 0usize);
    while i < b.len() {
        match b[i] {
            b'\'' | b'"' | b'`' => {
                i = Parser { s: text, i }.string_end();
                continue;
            }
            b'/' if matches!(b.get(i + 1), Some(b'/' | b'*')) => {
                let mut p = Parser { s: text, i };
                p.ws();
                i = p.i;
                continue;
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    b.len()
}
