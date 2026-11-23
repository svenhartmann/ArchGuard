//! Literals, brackets, identifiers and argument lists on raw text.
pub use archguard_flow_common::lang_csharp::{code as strip_comments, literal, literal_end};

pub fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

pub fn skip_ws(b: &[u8], i: usize) -> usize {
    i + b[i.min(b.len())..]
        .iter()
        .take_while(|c| c.is_ascii_whitespace())
        .count()
}

pub fn skip_ws_back(b: &[u8], mut i: usize) -> usize {
    while i > 0 && b[i - 1].is_ascii_whitespace() {
        i -= 1;
    }
    i
}

/// The identifier right before `end` (exclusive), skipping blanks and one `<…>`.
pub fn ident_before(b: &[u8], end: usize) -> (usize, usize) {
    let mut k = skip_ws_back(b, end);
    if k > 0 && b[k - 1] == b'>' {
        let mut depth = 0;
        while k > 0 {
            k -= 1;
            depth += match b[k] {
                b'>' => 1,
                b'<' => -1,
                _ => 0,
            };
            if depth == 0 {
                break;
            }
        }
        k = skip_ws_back(b, k);
    }
    let start = k - b[..k].iter().rev().take_while(|c| is_ident(**c)).count();
    (start, k)
}

/// Whole-word occurrences of `word`.
pub fn words<'a>(text: &'a str, word: &'a str) -> impl Iterator<Item = usize> + 'a {
    let b = text.as_bytes();
    text.match_indices(word).map(|(i, _)| i).filter(move |&i| {
        (i == 0 || !is_ident(b[i - 1])) && b.get(i + word.len()).is_none_or(|c| !is_ident(*c))
    })
}

/// Splits on commas outside brackets and literals.
pub fn split_top(s: &str) -> Vec<&str> {
    split_by(s, b',')
}

/// Splits on `sep` outside brackets and literals; empty parts are dropped.
pub fn split_by(s: &str, sep: u8) -> Vec<&str> {
    let b = s.as_bytes();
    let (mut parts, mut depth, mut start, mut i) = (Vec::new(), 0i32, 0, 0);
    while i < b.len() {
        if let Some(end) = literal_end(b, i) {
            i = end;
            continue;
        }
        match b[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            c if c == sep && depth == 0 => {
                parts.push(s[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    parts.push(s[start..].trim());
    parts.retain(|p| !p.is_empty());
    parts
}

/// `name = value` (Java/Kotlin/C# attribute) or `name: value` (C# argument).
pub fn named_part(part: &str) -> Option<(&str, &str)> {
    let b = part.as_bytes();
    let end = b.iter().take_while(|c| is_ident(**c)).count();
    let k = skip_ws(b, end);
    let pair = (*b.get(k)?, b.get(k + 1).copied());
    let named = matches!(pair, (b'=', next) if next != Some(b'='))
        || matches!(pair, (b':', next) if next != Some(b':'));
    (end > 0 && named).then(|| (&part[..end], part[k + 1..].trim()))
}

pub fn named<'a>(args: &'a str, key: &str) -> Option<&'a str> {
    split_top(args)
        .into_iter()
        .find_map(|p| named_part(p).filter(|(k, _)| *k == key).map(|(_, v)| v))
}

pub fn positional(args: &str) -> Option<&str> {
    split_top(args)
        .into_iter()
        .find(|p| named_part(p).is_none())
}

/// A literal or an array (`{…}`, `[…]`, `arrayOf(…)`) of literals.
pub fn literals(s: &str) -> Option<Vec<String>> {
    let s = s.trim();
    if let Some(one) = literal(s) {
        return Some(vec![one]);
    }
    let inner = [("arrayOf(", ')'), ("{", '}'), ("[", ']')]
        .iter()
        .find_map(|(open, close)| s.strip_prefix(open)?.strip_suffix(*close))?;
    split_top(inner).into_iter().map(literal).collect()
}
