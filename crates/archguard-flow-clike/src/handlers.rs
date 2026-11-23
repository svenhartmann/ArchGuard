//! The handler of an endpoint and the requests it sends (ADR 0022, decision 4).
//!
//! In the handler body (method or lambda, parameters included):
//! - `x.Send(arg)`, `x.SendAsync(arg)`, `x.send(arg)`, `x.sendAndWait(arg)`
//!   where the receiver name contains `mediator`, `sender`, `gateway` or `bus`
//!   (MediatR, `services.Mediator`, Axon `commandGateway`). `arg` is
//!   `new T(…)`, or a variable assigned `new T(…)` or declared with type `T`
//!   (local, parameter or field). A generic `new W<A, B>(x, …)` also yields the
//!   argument types that are one of `A`, `B`: the wrapped request.
//! - `x.m(…)` where `x` is declared with a type named `*UseCase`,
//!   `*CommandHandler` or `*QueryHandler`: that class, called directly.
//!
//! Anything else (a computed argument, a factory call, Kotlin constructor
//! calls without `new`) adds nothing; nothing is guessed.
use crate::lex::{ident_before, is_ident, skip_ws, skip_ws_back, split_top, words};
use crate::source::{Source, statement_end};

/// Handler method id, where it is declared and the types it sends.
pub type Link = (Option<String>, Option<(String, u32)>, Vec<String>);

const SEND: [&str; 4] = ["Send", "SendAsync", "send", "sendAndWait"];
const RECEIVERS: [&str; 4] = ["mediator", "sender", "gateway", "bus"];
const USE_CASES: [&str; 3] = ["UseCase", "CommandHandler", "QueryHandler"];
const NOT_TYPES: [&str; 12] = [
    "return", "new", "await", "in", "out", "ref", "is", "as", "case", "else", "throw", "yield",
];
/// Bound for `a = b; b = new T()` hops.
const MAX_DEPTH: usize = 3;

/// The method of `class` whose parameter list opens at `open`.
pub fn of_method(src: &Source, class: &str, name: &str, open: usize) -> Link {
    let sends = method_span(src, open).map_or_else(Vec::new, |span| sends(src, span));
    let at = (src.rel.clone(), src.line(open));
    (Some(format!("{class}.{name}")), Some(at), sends)
}

/// A minimal-API delegate argument: a method group (searched in `src`, then
/// the project) or a lambda.
pub fn of_delegate(src: &Source, sources: &[Source], arg: &str) -> Link {
    let at = arg.as_ptr() as usize - src.text.as_ptr() as usize;
    if arg.contains("=>") {
        return (None, None, sends(src, (at, at + arg.len())));
    }
    let name = arg.rsplit('.').next().unwrap_or(arg).trim();
    if name.is_empty() || !name.bytes().all(is_ident) {
        return (None, None, vec![]);
    }
    let found = std::iter::once(src)
        .chain(sources)
        .find_map(|s| definition(s, name).map(|open| (s, open)));
    match found {
        Some((s, open)) => {
            let class = enclosing_class(s, open).unwrap_or_default();
            of_method(s, &class, name, open)
        }
        None => (None, None, vec![]),
    }
}

/// From the parameter list at `open` to the end of the body; none without one.
fn method_span(src: &Source, open: usize) -> Option<(usize, usize)> {
    let b = src.bytes();
    let mut j = src.pair(open)? + 1;
    while j < b.len() {
        match b[j] {
            b'=' if b.get(j + 1) == Some(&b'>') => return Some((open, statement_end(src, j))),
            b'{' => return Some((open, src.pair(j)?)),
            b';' | b'}' => return None,
            _ => j += 1,
        }
    }
    None
}

/// Whether `src` declares a method named `name` with a body.
pub fn declares(src: &Source, name: &str) -> bool {
    definition(src, name).is_some()
}

/// The parameter list of a method named `name` declared in `src`.
fn definition(src: &Source, name: &str) -> Option<usize> {
    definitions(src, name).next()
}

/// The parameter lists of all methods named `name` declared in `src`.
pub fn definitions<'a>(src: &'a Source, name: &'a str) -> impl Iterator<Item = usize> + 'a {
    let b = src.bytes();
    words(&src.text, name).filter_map(move |p| {
        let prev = skip_ws_back(b, p);
        let typed =
            prev > 0 && (is_ident(b[prev - 1]) || matches!(b[prev - 1], b'>' | b']' | b'?'));
        let open = skip_ws(b, p + name.len());
        (typed && b.get(open) == Some(&b'(') && method_span(src, open).is_some()).then_some(open)
    })
}

/// The innermost class, record or struct whose body contains `at`.
pub fn enclosing_class(src: &Source, at: usize) -> Option<String> {
    let b = src.bytes();
    let mut found = None;
    for kw in ["class", "record", "struct"] {
        for p in words(&src.text, kw).filter(|p| *p < at) {
            let s = skip_ws(b, p + kw.len());
            let e = s + b[s..].iter().take_while(|c| is_ident(**c)).count();
            let Some(open) = src.text[e..at].find('{').map(|o| o + e) else {
                continue;
            };
            if src.text[e..open].contains(';') || src.pair(open).is_none_or(|c| c < at) {
                continue;
            }
            if found.as_ref().is_none_or(|(o, _)| *o < open) {
                found = Some((open, src.text[s..e].to_string()));
            }
        }
    }
    found.map(|(_, name)| name)
}

/// [`sends`] for other extractors (message handlers, ADR 0022): `text` is
/// a C#, Java or Kotlin file, `span` a byte range of it.
pub fn sends_in(text: &str, span: (usize, usize)) -> Vec<String> {
    sends(&Source::new(String::new(), text), span)
}

/// Sent request types and directly called use cases in `span`, sorted.
fn sends(src: &Source, span: (usize, usize)) -> Vec<String> {
    let b = src.bytes();
    let mut found = Vec::new();
    for dot in (span.0..span.1).filter(|&i| b[i] == b'.') {
        let (s, e) = ident_before(b, dot);
        let m = skip_ws(b, dot + 1);
        let m_end = m + b[m..].iter().take_while(|c| is_ident(**c)).count();
        let open = skip_ws(b, m_end);
        let close = (b.get(open) == Some(&b'('))
            .then(|| src.pair(open))
            .flatten();
        let (Some(close), true) = (close, s < e) else {
            continue;
        };
        let receiver = &src.text[s..e];
        let lower = receiver.to_ascii_lowercase();
        if SEND.contains(&&src.text[m..m_end]) && RECEIVERS.iter().any(|r| lower.contains(r)) {
            if let Some(arg) = arguments(&src.text[open + 1..close]).first() {
                found.extend(resolve(src, span, arg, 0));
            }
        } else if let Some(t) = declared(src, span, receiver).filter(|t| use_case(t)) {
            found.push(t);
        }
    }
    found.sort();
    found.dedup();
    found
}

/// `split_top` that keeps `W<A, B>` in one argument.
fn arguments(s: &str) -> Vec<&str> {
    let angles = |p: &str| {
        let b = p.as_bytes();
        let opened = b.iter().filter(|c| **c == b'<').count();
        let closed = (b.iter().enumerate())
            .filter(|(i, c)| **c == b'>' && (*i == 0 || b[i - 1] != b'='))
            .count();
        opened > closed
    };
    let mut out: Vec<&str> = Vec::new();
    for part in split_top(s) {
        match out.last_mut() {
            Some(last) if angles(last) => {
                let start = last.as_ptr() as usize - s.as_ptr() as usize;
                let end = part.as_ptr() as usize - s.as_ptr() as usize + part.len();
                *last = &s[start..end];
            }
            _ => out.push(part),
        }
    }
    out
}

/// `IdentifiedCommand` of `Ns.IdentifiedCommand<A, B>`.
pub fn simple_type(t: &str) -> &str {
    let outer = t.split('<').next().unwrap_or(t).trim();
    outer.rsplit('.').next().unwrap_or(outer)
}

fn use_case(type_text: &str) -> bool {
    USE_CASES
        .iter()
        .any(|x| simple_type(type_text).ends_with(x))
}

/// Types the expression `expr` (a slice of `src.text`) stands for.
fn resolve(src: &Source, span: (usize, usize), expr: &str, depth: usize) -> Vec<String> {
    let expr = expr.trim();
    if depth > MAX_DEPTH {
        return vec![];
    }
    if let Some(rest) = expr.strip_prefix("new ") {
        return constructed(src, span, rest.trim_start(), depth);
    }
    if !expr.is_empty() && expr.bytes().all(is_ident) {
        return variable(src, span, expr, depth);
    }
    vec![]
}

/// `T(…)` or `W<A, B>(x, …)` after `new`: `T`, or `W<A, B>` and the
/// arguments whose type is one of `A`, `B`.
fn constructed(src: &Source, span: (usize, usize), text: &str, depth: usize) -> Vec<String> {
    let end = text.find(['(', '{']).unwrap_or(text.len());
    let ty = text[..end].trim();
    if ty.is_empty() {
        return vec![];
    }
    let mut found = vec![ty.to_string()];
    let generics: Vec<&str> = ty
        .split_once('<')
        .map(|(_, g)| g.trim_end_matches('>'))
        .map_or_else(Vec::new, |g| g.split(',').map(simple_type).collect());
    let at = text.as_ptr() as usize - src.text.as_ptr() as usize + end;
    if let (false, Some(close)) = (generics.is_empty(), src.pair(at)) {
        for arg in arguments(&src.text[at + 1..close]) {
            let inner = resolve(src, span, arg, depth + 1);
            found.extend(
                inner
                    .into_iter()
                    .filter(|t| generics.contains(&simple_type(t))),
            );
        }
    }
    found
}

/// The variable's initializer if it is `new …`, else its declared type: first
/// in `span`, then anywhere in the file (fields).
fn variable(src: &Source, span: (usize, usize), var: &str, depth: usize) -> Vec<String> {
    for scope in [&src.text[span.0..span.1], &src.text[..]] {
        let base = scope.as_ptr() as usize - src.text.as_ptr() as usize;
        for p in words(scope, var).map(|p| p + base) {
            if let Some(found) = declaration(src, span, p, var, depth) {
                return found;
            }
        }
    }
    vec![]
}

/// `T var`, `T var = init` or `var var = init` at `p`.
fn declaration(
    src: &Source,
    span: (usize, usize),
    p: usize,
    var: &str,
    depth: usize,
) -> Option<Vec<String>> {
    let b = src.bytes();
    let ty = type_before(src, p)?;
    let eq = skip_ws(b, p + var.len());
    if b.get(eq) == Some(&b'=') && !matches!(b.get(eq + 1), Some(b'=' | b'>')) {
        let init = &src.text[eq + 1..statement_end(src, eq + 1)];
        let found = resolve(src, span, init, depth + 1);
        if !found.is_empty() {
            return Some(found);
        }
    }
    (!matches!(ty, "var" | "val")).then(|| vec![ty.to_string()])
}

/// The declared type right before the variable at `p`, if one is written.
fn type_before(src: &Source, p: usize) -> Option<&str> {
    let b = src.bytes();
    let end = skip_ws_back(b, p);
    let (s, e) = ident_before(b, end);
    let before = skip_ws_back(b, s);
    let member = before > 0 && b[before - 1] == b'.';
    let ty = &src.text[s..end];
    (s < e && !member && !NOT_TYPES.contains(&&src.text[s..e])).then_some(ty)
}

/// The type text of the variable `var` in `span` or the file.
fn declared(src: &Source, span: (usize, usize), var: &str) -> Option<String> {
    for scope in [&src.text[span.0..span.1], &src.text[..]] {
        let base = scope.as_ptr() as usize - src.text.as_ptr() as usize;
        let found = words(scope, var)
            .map(|p| p + base)
            .find_map(|p| type_before(src, p).filter(|t| !matches!(*t, "var" | "val")));
        if let Some(t) = found {
            return Some(t.to_string());
        }
    }
    None
}
