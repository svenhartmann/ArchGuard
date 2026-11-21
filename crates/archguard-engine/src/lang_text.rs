//! Navigation in source text whose comments and literal contents are blanked
//! (`lang_blank`, same offsets as the source), shared by the C#, Java/Kotlin
//! and TypeScript readers of ADR 0021/0022. Identifiers include `$` (Java,
//! TypeScript); in C# text a `$` is only left as a string prefix.

pub(crate) fn is_id(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c >= 0x80
}

pub(crate) fn ident_end(b: &[u8], from: usize) -> usize {
    (from..b.len()).find(|&k| !is_id(b[k])).unwrap_or(b.len())
}

/// Identifiers in `s[from..to]` with their offsets.
pub(crate) fn idents(s: &str, from: usize, to: usize) -> Vec<(usize, &str)> {
    let b = s.as_bytes();
    let mut out = vec![];
    let mut k = from;
    while k < to {
        if !is_id(b[k]) {
            k += 1;
            continue;
        }
        let start = k;
        k = ident_end(b, k).min(to);
        if !b[start].is_ascii_digit() {
            out.push((start, &s[start..k]));
        }
    }
    out
}

/// Offset of the sub-slice `part` in `s`.
pub(crate) fn offset(s: &str, part: &str) -> usize {
    part.as_ptr() as usize - s.as_ptr() as usize
}

/// Only whitespace between token `a` and offset `b`.
pub(crate) fn gap(s: &str, a: (usize, &str), b: usize) -> bool {
    s[a.0 + a.1.len()..b].trim().is_empty()
}

pub(crate) fn next_nonws(b: &[u8], from: usize) -> Option<usize> {
    (from..b.len()).find(|&k| !b[k].is_ascii_whitespace())
}

pub(crate) fn prev_nonws(b: &[u8], before: usize) -> Option<usize> {
    (0..before).rev().find(|&k| !b[k].is_ascii_whitespace())
}

/// The identifier ending right before `before` (whitespace skipped).
pub(crate) fn prev_word(s: &str, before: usize) -> &str {
    let b = s.as_bytes();
    let Some(end) = prev_nonws(b, before).map(|k| k + 1) else {
        return "";
    };
    let start = (0..end).rev().find(|&k| !is_id(b[k])).map_or(0, |k| k + 1);
    &s[start..end]
}

fn pair(c: u8) -> Option<(u8, u8)> {
    [(b'(', b')'), (b'{', b'}'), (b'[', b']'), (b'<', b'>')]
        .into_iter()
        .find(|&(o, z)| c == o || c == z)
}

/// The bracket closing the one at `open`.
pub(crate) fn matching(b: &[u8], open: usize) -> Option<usize> {
    let (o, z) = pair(b[open]).filter(|p| p.0 == b[open])?;
    let mut depth = 0;
    for (k, &c) in b.iter().enumerate().skip(open) {
        depth += i32::from(c == o) - i32::from(c == z);
        if depth == 0 {
            return Some(k);
        }
    }
    None
}

/// The bracket opening the one at `close`.
pub(crate) fn matching_back(b: &[u8], close: usize) -> Option<usize> {
    let (o, z) = pair(b[close]).filter(|p| p.1 == b[close])?;
    let mut depth = 0;
    for k in (0..=close).rev() {
        depth += i32::from(b[k] == z) - i32::from(b[k] == o);
        if depth == 0 {
            return Some(k);
        }
    }
    None
}

/// Splits at `sep` outside (), <>, [] and {}.
pub(crate) fn split_top<'a>(s: &'a str, sep: &str) -> Vec<&'a str> {
    let b = s.as_bytes();
    let mut out = vec![];
    let (mut depth, mut start, mut k) = (0i32, 0, 0);
    while k < b.len() {
        match b[k] {
            b'(' | b'<' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b'>' if k == 0 || !matches!(b[k - 1], b'=' | b'-') => depth -= 1,
            _ if depth == 0 && b[k..].starts_with(sep.as_bytes()) => {
                out.push(&s[start..k]);
                k += sep.len();
                start = k;
                continue;
            }
            _ => {}
        }
        k += 1;
    }
    out.push(&s[start..]);
    out
}

/// First `pat` outside parentheses and brackets.
pub(crate) fn find_top(s: &str, pat: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut depth = 0i32;
    for k in 0..b.len() {
        if depth == 0 && b[k..].starts_with(pat.as_bytes()) {
            return Some(k);
        }
        match b[k] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// First plain `=` (not `==`, `!=`, `<=`, `>=`, `=>`) outside parentheses.
pub(crate) fn first_assign(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut depth = 0i32;
    for k in 0..b.len() {
        match b[k] {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b'=' if depth == 0
                && !matches!(b.get(k + 1), Some(b'=' | b'>'))
                && !(k > 0 && matches!(b[k - 1], b'=' | b'!' | b'<' | b'>')) =>
            {
                return Some(k);
            }
            _ => {}
        }
    }
    None
}

pub(crate) fn unparen(s: &str) -> &str {
    let mut s = s.trim();
    while s.starts_with('(') && matching(s.as_bytes(), 0) == Some(s.len() - 1) {
        s = s[1..s.len() - 1].trim();
    }
    s
}

/// `a.b.Type<T>?` / `() => Base` / `Base()` / `T[]` / `T...` → `Type` / `Base` / `T`.
pub(crate) fn simple(t: &str) -> String {
    let t = t.trim();
    let t = t.rsplit("=>").next().unwrap_or(t).trim();
    // A leading `(` is a C# tuple type, kept whole.
    let t = &t[..t.find(['<', '(']).filter(|&p| p > 0).unwrap_or(t.len())];
    let t = (t.trim_end_matches('?').trim_end_matches("[]")).trim_end_matches("...");
    t.rsplit('.').next().unwrap_or(t).trim().to_string()
}

pub(crate) fn mentions(cond: &str, field: &str) -> bool {
    idents(cond, 0, cond.len()).iter().any(|x| x.1 == field)
}

pub(crate) fn line(s: &str, pos: usize) -> u32 {
    let newlines = s.as_bytes()[..pos.min(s.len())]
        .iter()
        .filter(|&&c| c == b'\n')
        .count();
    u32::try_from(newlines + 1).unwrap_or(u32::MAX)
}

pub(crate) fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}
