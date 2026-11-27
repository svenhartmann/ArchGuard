//! Text guards shared by the C#, Java/Kotlin and TypeScript domain readers
//! (ADR 0021/0022): open blocks, single-statement `if`, exits and call
//! ownership, over blanked source text (`lang_text`). Language rules (`Head`,
//! `statement_start`, literals, operators) stay in each reader.

use crate::lang_text::{idents, is_id, matching, next_nonws, prev_nonws};
use std::collections::{BTreeMap, BTreeSet};

type Span = (usize, usize);

/// Offsets of the `{` still open at `to`, scanning from `from`.
pub fn open_blocks(b: &[u8], from: usize, to: usize) -> Vec<usize> {
    let mut stack = vec![];
    for (k, &c) in b.iter().enumerate().take(to).skip(from) {
        match c {
            b'{' => stack.push(k),
            b'}' => {
                stack.pop();
            }
            _ => {}
        }
    }
    stack
}

/// The innermost bracket still open at `to`, scanning from `from`.
pub fn innermost_open(b: &[u8], from: usize, to: usize) -> Option<usize> {
    let mut stack = vec![];
    for (k, &c) in b.iter().enumerate().take(to).skip(from) {
        match c {
            b'(' | b'[' | b'{' => stack.push(k),
            b')' | b']' | b'}' => {
                stack.pop();
            }
            _ => {}
        }
    }
    stack.pop()
}

/// `;` or a line break at depth 0 from `from`. C# has no line-break rule and
/// does not use this.
pub fn statement_end(b: &[u8], from: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (k, &c) in b.iter().enumerate().skip(from) {
        match c {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b';' | b'\n' if depth <= 0 => return Some(k),
            _ => {}
        }
    }
    None
}

/// An `if` keyword at `k`: (condition, block or single statement ended by
/// [`statement_end`]).
pub fn if_at(s: &str, k: usize) -> Option<(Span, Span)> {
    let b = s.as_bytes();
    let keyword = b[k..].starts_with(b"if")
        && (k == 0 || !is_id(b[k - 1]))
        && !b.get(k + 2).is_some_and(|&c| is_id(c));
    if !keyword {
        return None;
    }
    let open = next_nonws(b, k + 2).filter(|&o| b[o] == b'(')?;
    let close = matching(b, open)?;
    let first = next_nonws(b, close + 1)?;
    let end = if b[first] == b'{' {
        matching(b, first)?
    } else {
        statement_end(b, first)?
    };
    Some(((open + 1, close), (first, end)))
}

/// True when `range` throws, returns or calls a method that only throws.
pub fn exits(s: &str, range: Span, helpers: &BTreeSet<&str>) -> bool {
    idents(s, range.0, range.1)
        .iter()
        .any(|(_, w)| matches!(*w, "throw" | "return") || helpers.contains(w))
}

pub fn capitalize(x: &str) -> String {
    let mut c = x.chars();
    c.next()
        .map_or_else(String::new, |f| f.to_uppercase().chain(c).collect())
}

/// `Entity.m` for each `.m(` in `body` whose `m` one entity owns, plus
/// (offset, message) for names several entities own.
pub fn calls_in(
    s: &str,
    body: Span,
    owners: &BTreeMap<&str, BTreeSet<&str>>,
) -> (Vec<String>, Vec<(usize, String)>) {
    let b = s.as_bytes();
    let mut calls = BTreeSet::new();
    let mut ambiguous = vec![];
    for (pos, w) in idents(s, body.0, body.1) {
        let dotted = prev_nonws(b, pos).is_some_and(|k| b[k] == b'.');
        let call = next_nonws(b, pos + w.len()).is_some_and(|k| b[k] == b'(');
        let Some(set) = owners.get(w).filter(|_| dotted && call) else {
            continue;
        };
        let list: Vec<&str> = set.iter().copied().collect();
        match list.as_slice() {
            [one] => {
                calls.insert(format!("{one}.{w}"));
            }
            _ => ambiguous.push((
                pos,
                format!(
                    "call .{w}( matches methods of several entities ({}); not recorded.",
                    list.join(", ")
                ),
            )),
        }
    }
    (calls.into_iter().collect(), ambiguous)
}

/// The type created for a raise argument: `new X(…)` per `new_type` (language
/// rule), or a variable assigned one in `before` (the method text up to the call).
pub fn created(
    s: &str,
    before: Span,
    arg: &str,
    new_type: fn(&str) -> Option<String>,
) -> Option<String> {
    if let Some(t) = new_type(arg) {
        return Some(t);
    }
    if arg.is_empty() || !arg.bytes().all(is_id) {
        return None;
    }
    let b = s.as_bytes();
    idents(s, before.0, before.1)
        .into_iter()
        .rev()
        .filter(|x| x.1 == arg)
        .find_map(|(p, w)| {
            let eq = next_nonws(b, p + w.len())
                .filter(|&k| b[k] == b'=' && b.get(k + 1) != Some(&b'='))?;
            new_type(&s[eq + 1..before.1])
        })
}
