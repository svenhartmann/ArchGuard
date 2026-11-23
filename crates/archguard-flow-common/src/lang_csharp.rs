//! C# lexing shared by the Blazor, XAML, backend and domain extractors (ADR
//! 0021, 0022): comments, preprocessor lines and literals, and reading of
//! arguments and plain string literals. Java and Kotlin sources read through
//! the backend extractor use the same rules: their comments, text blocks
//! (`"""`), strings and char literals look the same.
use crate::lang_blank::{blank_regions, find, line_end};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Token {
    /// A `//` or `/* */` comment or a preprocessor line (`#if`, `#region`).
    Comment,
    /// A string or char literal.
    Literal,
}

/// The comment or literal starting at `i`: (kind, blank from, blank to,
/// continue at). For a literal, `from..to` is its content between the quotes.
pub fn token_at(b: &[u8], i: usize) -> Option<(Token, usize, usize, usize)> {
    let next = b.get(i + 1).copied();
    match b[i] {
        b'/' if next == Some(b'/') => Some((Token::Comment, i, line_end(b, i), line_end(b, i))),
        b'/' if next == Some(b'*') => {
            let end = find(b, i + 2, b"*/").map_or(b.len(), |p| p + 2);
            Some((Token::Comment, i, end, end))
        }
        b'#' if at_line_start(b, i) => Some((Token::Comment, i, line_end(b, i), line_end(b, i))),
        b'\'' => char_literal(b, i).map(|(from, to, next)| (Token::Literal, from, to, next)),
        b'"' => {
            let (from, to, next) = string_literal(b, i);
            Some((Token::Literal, from, to, next))
        }
        _ => None,
    }
}

/// End (exclusive) of the string or char literal starting at `i`, if one does.
pub fn literal_end(b: &[u8], i: usize) -> Option<usize> {
    match b[i] {
        b'"' | b'\'' => token_at(b, i).map(|(_, _, _, next)| next.min(b.len())),
        _ => None,
    }
}

fn at_line_start(b: &[u8], i: usize) -> bool {
    b[..i]
        .iter()
        .rev()
        .take_while(|&&c| c != b'\n')
        .all(|c| c.is_ascii_whitespace())
}

/// `'x'`, `'ä'` or an escape such as `'\n'`, `'A'`; an apostrophe that
/// starts none of these is no literal.
fn char_literal(b: &[u8], i: usize) -> Option<(usize, usize, usize)> {
    let end = if b.get(i + 1) == Some(&b'\\') {
        (i + 3..(i + 12).min(b.len()))
            .take_while(|&j| b[j] != b'\n')
            .find(|&j| b[j] == b'\'')?
    } else {
        let width = match *b.get(i + 1)? {
            c if c < 0x80 => 1,
            c if c >= 0xF0 => 4,
            c if c >= 0xE0 => 3,
            _ => 2,
        };
        let end = i + 1 + width;
        (b.get(end) == Some(&b'\'') && !b[i + 1..end].contains(&b'\n')).then_some(end)?
    };
    Some((i + 1, end, end + 1))
}

/// Regular, verbatim (`@"…"`, `$@"…"`, `@$"…"`), interpolated and raw
/// (`"""…"""`) strings: (content from, content to, continue at). A regular
/// string ends at a line break at the latest.
fn string_literal(b: &[u8], i: usize) -> (usize, usize, usize) {
    let quotes = b[i..].iter().take_while(|&&c| c == b'"').count();
    if quotes >= 3 {
        let end = find(b, i + quotes, &b[i..i + quotes]).unwrap_or(b.len());
        return (i + quotes, end, end + quotes);
    }
    let verbatim = b[..i]
        .iter()
        .rev()
        .take_while(|&&c| matches!(c, b'@' | b'$'))
        .any(|&c| c == b'@');
    let mut j = i + 1;
    while j < b.len() {
        match (b[j], verbatim) {
            (b'"', true) if b.get(j + 1) == Some(&b'"') => j += 2,
            (b'"', _) | (b'\n', false) => break,
            (b'\\', false) => j += 2,
            _ => j += 1,
        }
    }
    let j = j.min(b.len());
    (i + 1, j, j + 1)
}

/// `src` with comments, preprocessor lines and the contents of string and
/// char literals blanked (quotes kept), offsets and lines unchanged.
pub fn mask(src: &str) -> String {
    blank_regions(src, |b, i| {
        token_at(b, i).map(|(_, from, to, next)| (from, to, next))
    })
}

/// `src` with comments and preprocessor lines blanked, literals kept.
pub fn code(src: &str) -> String {
    blank_regions(src, |b, i| {
        token_at(b, i).map(|(kind, from, to, next)| match kind {
            Token::Comment => (from, to, next),
            Token::Literal => (next, next, next),
        })
    })
}

/// ([`code`], [`mask`]) of `src`.
pub fn clean(src: &str) -> (String, String) {
    (code(src), mask(src))
}

/// The content of a plain string literal spanning all of `s` (`"x"`,
/// `@"x"`), as written; `None` for anything computed, interpolated or raw.
pub fn literal(s: &str) -> Option<String> {
    let s = s.trim();
    let quote = usize::from(s.starts_with("@\""));
    let b = s.as_bytes();
    let plain = b.len() >= quote + 2 && b[quote] == b'"' && !s[quote..].starts_with("\"\"\"");
    (plain && literal_end(b, quote) == Some(b.len())).then(|| s[quote + 1..s.len() - 1].to_string())
}

/// Prefix of `s` up to the `close` outside literals and nested brackets.
pub fn balanced(s: &str, close: char) -> &str {
    let b = s.as_bytes();
    let mut depth = 0i32;
    let mut i = 0;
    while i < b.len() {
        if let Some(end) = literal_end(b, i) {
            i = end;
            continue;
        }
        match b[i] {
            c if c == close as u8 && depth == 0 => return &s[..i],
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    s
}

/// Top-level comma separated arguments, trimmed.
pub fn split_args(args: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = args;
    while !rest.trim().is_empty() {
        let part = balanced(rest, ',');
        out.push(part.trim().to_string());
        rest = rest.get(part.len() + 1..).unwrap_or("");
    }
    out
}

/// A C# identifier character (letters of any script, digits, `_`).
pub fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Offset where the identifier ending `s` starts.
pub fn ident_start(s: &str) -> usize {
    s.char_indices()
        .rev()
        .take_while(|(_, c)| is_ident(*c))
        .last()
        .map_or(s.len(), |(i, _)| i)
}
