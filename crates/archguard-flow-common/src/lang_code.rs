//! Source-text patterns shared by the C# and Java rule readers (ADR 0022,
//! Verlauf 2026-11-20): the condition guarding a throw, a message argument,
//! `new X(…)` and chained calls. Works on text with comments and literal
//! contents blanked (`text`) and the source as read (`raw`), same offsets;
//! and the service (project folder) of a file.
use crate::lang_text::*;

/// A byte range `(start, end)` of a source text.
pub type Span = (usize, usize);
use std::path::Path;

/// The folder name of the nearest build file above `rel` (a file name in that
/// folder ending in one of `build`, e.g. `.csproj` or `pom.xml`): the service
/// of a publish site. Without one, the first path segment of `rel`.
pub fn project_of(
    root: &Path,
    rel: &str,
    build: &[&str],
    cache: &mut std::collections::BTreeMap<String, String>,
) -> String {
    let start = rel.rsplit_once('/').map_or("", |(d, _)| d);
    if let Some(known) = cache.get(start) {
        return known.clone();
    }
    let has_build = |dir: &str| {
        std::fs::read_dir(root.join(dir)).is_ok_and(|entries| {
            entries.flatten().any(|e| {
                let name = e.file_name();
                let name = name.to_string_lossy();
                build.iter().any(|b| name.ends_with(b))
            })
        })
    };
    let mut dir = start;
    let found = loop {
        if !dir.is_empty() && has_build(dir) {
            break dir.rsplit('/').next().unwrap_or(dir).to_string();
        }
        match dir.rsplit_once('/') {
            Some((up, _)) => dir = up,
            None => {
                break rel
                    .split_once('/')
                    .map_or("", |(first, _)| first)
                    .to_string();
            }
        }
    };
    cache.insert(start.to_string(), found.clone());
    found
}

#[derive(Clone, Copy)]
pub struct Code<'a> {
    pub text: &'a str,
    pub raw: &'a str,
}

/// `new X(…)` starting at or after `from`: (simple `X`, argument span).
pub fn created_at(s: &str, from: usize) -> Option<(String, Span)> {
    let b = s.as_bytes();
    let k = next_nonws(b, from)?;
    if !s[k..].starts_with("new") || b.get(k + 3).is_some_and(|&c| is_id(c)) {
        return None;
    }
    let t = next_nonws(b, k + 3)?;
    let end = (t..b.len())
        .find(|&j| !(is_id(b[j]) || b[j] == b'.'))
        .unwrap_or(b.len());
    let open = next_nonws(b, end).filter(|&o| b[o] == b'(' && end > t)?;
    Some((simple(&s[t..end]), (open + 1, matching(b, open)?)))
}

pub fn raw(c: Code, (a, z): Span) -> String {
    collapse(c.raw.get(a..z).unwrap_or(&c.text[a..z]))
}

/// The call from `start` through the parenthesis opening at `open`, as written.
pub fn call_text(c: Code, start: usize, open: usize) -> Option<String> {
    let close = matching(c.text.as_bytes(), open)?;
    Some(raw(c, (start, close + 1)))
}

/// Arguments split at `,` outside (), [] and {}; unlike `split_top`, a `<`
/// or `>` is a comparison here, not a generic bracket.
pub fn split_args(s: &str) -> Vec<&str> {
    let mut out = vec![];
    let (mut depth, mut start) = (0i32, 0);
    for (k, c) in s.bytes().enumerate() {
        match c {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b',' if depth == 0 => {
                out.push(&s[start..k]);
                start = k + 1;
            }
            _ => {}
        }
    }
    out.push(&s[start..]);
    out
}

/// The first argument: a string literal's content (interpolations kept), else
/// its source text.
pub fn message(c: Code, (a, z): Span) -> Option<String> {
    let first = split_args(&c.text[a..z]).into_iter().next()?;
    let at = a + (first.as_ptr() as usize - c.text[a..].as_ptr() as usize);
    let text = raw(c, (at, at + first.len()));
    // One literal: in the blanked text only its own two quotes are left.
    let blanked = first.trim().trim_start_matches(['$', '@']);
    let literal = blanked.len() >= 2
        && blanked.starts_with('"')
        && blanked.ends_with('"')
        && !blanked[1..blanked.len() - 1].contains('"');
    match (text.is_empty(), literal) {
        (true, _) => None,
        (_, true) => Some(
            text.trim_start_matches(['$', '@'])
                .trim_matches('"')
                .to_string(),
        ),
        _ => Some(text),
    }
}

/// The condition under which the throw (or `new`) at `pos` runs.
pub fn condition(c: Code, body: Span, pos: usize) -> Option<String> {
    let (s, b) = (c.text, c.text.as_bytes());
    let k = prev_nonws(b, pos)?;
    if b[k] == b':' {
        let q = scan_back(b, body.0, k, |j| {
            b[j] == b'?' && b[j - 1] != b'?' && !matches!(b[j + 1], b'?' | b'.' | b'[')
        })?;
        let start = expression_start(s, body.0, q);
        return Some(format!("!({})", raw(c, (start, q))));
    }
    if b[k] == b'?' && k > 0 && b[k - 1] == b'?' {
        let start = expression_start(s, body.0, k - 1);
        return Some(format!("{} == null", raw(c, (start, k - 1))));
    }
    let stmt = statement_start(b, body.0, pos);
    if s[stmt..].starts_with("if") && !b.get(stmt + 2).is_some_and(|&c| is_id(c)) {
        let open = next_nonws(b, stmt + 2).filter(|&o| b[o] == b'(')?;
        return Some(raw(c, (open + 1, matching(b, open)?)));
    }
    let brace = prev_nonws(b, stmt).filter(|&o| b[o] == b'{')?;
    if_condition(c, brace)
}

/// The condition of the `if` (or, negated, of the `if` before the `else`)
/// heading the block that opens at `brace`.
fn if_condition(c: Code, brace: usize) -> Option<String> {
    let (s, b) = (c.text, c.text.as_bytes());
    let k = prev_nonws(b, brace)?;
    if b[k] == b')' {
        let open = matching_back(b, k)?;
        return (prev_word(s, open) == "if").then(|| raw(c, (open + 1, k)));
    }
    if prev_word(s, brace) == "else" {
        let close = prev_nonws(b, k.saturating_sub(3)).filter(|&c| b[c] == b'}')?;
        let inner = if_condition(c, matching_back(b, close)?)?;
        return (!inner.starts_with("!(")).then(|| format!("!({inner})"));
    }
    None
}

/// The nearest `j < to` (not before `from`) at bracket depth 0 where `hit` holds.
fn scan_back(b: &[u8], from: usize, to: usize, hit: impl Fn(usize) -> bool) -> Option<usize> {
    let mut depth = 0i32;
    for j in (from + 1..to).rev() {
        match b[j] {
            b')' | b']' | b'}' => depth += 1,
            b'(' | b'[' | b'{' if depth == 0 => return None,
            b'(' | b'[' | b'{' => depth -= 1,
            b';' if depth == 0 => return None,
            _ if depth == 0 && hit(j) => return Some(j),
            _ => {}
        }
    }
    None
}

/// Start of the expression ending at `end`: after a plain `=`, `return`, an
/// opening bracket, `,` or a statement boundary at depth 0.
fn expression_start(s: &str, from: usize, end: usize) -> usize {
    let b = s.as_bytes();
    let assign = |j: usize| {
        b[j] == b'='
            && !matches!(b[j + 1], b'=' | b'>')
            && !matches!(b[j - 1], b'=' | b'!' | b'<' | b'>')
    };
    let stop = scan_back(b, from, end, |j| assign(j) || b[j] == b',');
    let mut start = stop.map_or_else(
        || {
            let mut depth = 0i32;
            (from..end)
                .rev()
                .find(|&j| match b[j] {
                    b')' | b']' | b'}' => {
                        depth += 1;
                        false
                    }
                    b'(' | b'[' | b'{' if depth == 0 => true,
                    b'(' | b'[' | b'{' => {
                        depth -= 1;
                        false
                    }
                    b';' => depth == 0,
                    _ => false,
                })
                .map_or(from, |j| j + 1)
        },
        |j| j + 1,
    );
    start = next_nonws(b, start).unwrap_or(start);
    if s[start..].starts_with("return") && !b.get(start + 6).is_some_and(|&c| is_id(c)) {
        start = next_nonws(b, start + 6).unwrap_or(start);
    }
    start
}

/// Start of the statement containing `pos`: after the nearest `;`, `{` or `}`
/// at depth 0, crossing the brackets `pos` is inside.
fn statement_start(b: &[u8], from: usize, pos: usize) -> usize {
    let mut depth = 0i32;
    for j in (from..pos).rev() {
        match b[j] {
            b')' | b']' => depth += 1,
            b'(' | b'[' if depth > 0 => depth -= 1,
            b';' | b'{' | b'}' if depth == 0 => return next_nonws(b, j + 1).unwrap_or(pos),
            _ => {}
        }
    }
    next_nonws(b, from).unwrap_or(pos)
}

/// The types of the `new X(…)` at bracket depth 0 of `span`.
pub fn top_news(s: &str, span: Span) -> Vec<String> {
    let b = s.as_bytes();
    let depth = |p: usize| {
        (b[span.0..p].iter()).fold(0i32, |d, c| match c {
            b'(' | b'[' | b'{' => d + 1,
            b')' | b']' | b'}' => d - 1,
            _ => d,
        })
    };
    (idents(s, span.0, span.1).into_iter())
        .filter(|&(p, w)| w == "new" && depth(p) == 0)
        .filter_map(|(p, _)| created_at(s, p).map(|c| c.0))
        .collect()
}

/// The `new` types of every assignment `var = …;` in `before`.
pub fn assigned_news(s: &str, before: Span, var: &str) -> Vec<String> {
    let b = s.as_bytes();
    let mut out = vec![];
    for (p, w) in idents(s, before.0, before.1)
        .into_iter()
        .filter(|x| x.1 == var)
    {
        let Some(eq) = next_nonws(b, p + w.len())
            .filter(|&e| b[e] == b'=' && !matches!(b.get(e + 1), Some(b'=' | b'>')))
        else {
            continue;
        };
        let end = (eq + 1..before.1)
            .scan(0i32, |d, j| {
                *d += match b[j] {
                    b'(' | b'[' | b'{' => 1,
                    b')' | b']' | b'}' => -1,
                    _ => 0,
                };
                Some((j, *d))
            })
            .find(|&(j, d)| (d == 0 && b[j] == b';') || d < 0)
            .map_or(before.1, |(j, _)| j);
        out.extend(top_news(s, (eq + 1, end)));
    }
    out
}
