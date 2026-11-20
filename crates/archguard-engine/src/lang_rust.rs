//! Rust lexing for the Rust UI extractor (ADR 0021): comments (doc comments,
//! nested block comments), string literals (escapes, raw `r#"…"#`, byte and
//! C strings), char literals versus lifetimes and labels (`'a'` / `'a`), and
//! navigation in the blanked text: attributes before an item, macro bodies
//! (`rsx! { }`, `view! { }`, `html! { }`), `fn` items and their bodies.
//! Text level, no parser: a file that does not lex keeps its offsets anyway.
use crate::lang_blank::{blank_regions, find, line_end};
use crate::lang_text::{is_id, matching, matching_back, next_nonws, prev_nonws, prev_word};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Token {
    Comment,
    /// A string, byte string or char literal.
    Literal,
}

/// The comment or literal starting at `i`: (kind, blank from, blank to,
/// continue at). For a literal, `from..to` is its content between the quotes.
pub(crate) fn token_at(b: &[u8], i: usize) -> Option<(Token, usize, usize, usize)> {
    match (b[i], b.get(i + 1)) {
        (b'/', Some(b'/')) => Some((Token::Comment, i, line_end(b, i), line_end(b, i))),
        (b'/', Some(b'*')) => {
            let end = block_comment_end(b, i);
            Some((Token::Comment, i, end, end))
        }
        _ if i > 0 && is_id(b[i - 1]) => None,
        (b'"', _) => Some(literal(i + 1, quoted_end(b, i + 1))),
        (b'\'', _) => char_end(b, i).map(|end| literal(i + 1, end)),
        (b'r' | b'b' | b'c', _) => prefixed(b, i),
        _ => None,
    }
}

fn literal(from: usize, to: usize) -> (Token, usize, usize, usize) {
    (Token::Literal, from, to, to + 1)
}

/// End (exclusive) of `/* … */` at `i`, nested comments counted.
fn block_comment_end(b: &[u8], i: usize) -> usize {
    let (mut depth, mut j) = (0usize, i);
    while j + 1 < b.len() {
        match (b[j], b[j + 1]) {
            (b'/', b'*') => (depth, j) = (depth + 1, j + 2),
            (b'*', b'/') if depth == 1 => return j + 2,
            (b'*', b'/') => (depth, j) = (depth - 1, j + 2),
            _ => j += 1,
        }
    }
    b.len()
}

/// The closing quote of a string whose content starts at `from`.
fn quoted_end(b: &[u8], from: usize) -> usize {
    let mut j = from;
    while j < b.len() && b[j] != b'"' {
        j += if b[j] == b'\\' { 2 } else { 1 };
    }
    j.min(b.len())
}

/// The closing quote of the char literal at `i`; none for a lifetime or label.
fn char_end(b: &[u8], i: usize) -> Option<usize> {
    if b.get(i + 1) == Some(&b'\\') {
        return (i + 3..(i + 14).min(b.len()))
            .take_while(|&j| b[j] != b'\n')
            .find(|&j| b[j] == b'\'');
    }
    let width = match *b.get(i + 1)? {
        b'\n' | b'\'' => return None,
        c if c < 0x80 => 1,
        c if c >= 0xF0 => 4,
        c if c >= 0xE0 => 3,
        _ => 2,
    };
    (b.get(i + 1 + width) == Some(&b'\'')).then_some(i + 1 + width)
}

/// `b"…"`, `b'x'`, `c"…"`, `r"…"`, `r#"…"#`, `br#"…"#`, `cr"…"`.
fn prefixed(b: &[u8], i: usize) -> Option<(Token, usize, usize, usize)> {
    let mut j = i + 1;
    if matches!(b[i], b'b' | b'c') && b.get(j) == Some(&b'r') {
        j += 1;
    }
    let raw = b[i] == b'r' || j == i + 2;
    if !raw {
        return match b.get(j)? {
            b'"' => Some(literal(j + 1, quoted_end(b, j + 1))),
            b'\'' if b[i] == b'b' => char_end(b, j).map(|end| literal(j + 1, end)),
            _ => None,
        };
    }
    let hashes = b[j..].iter().take_while(|&&c| c == b'#').count();
    if b.get(j + hashes) != Some(&b'"') {
        return None;
    }
    let from = j + hashes + 1;
    let mut close = vec![b'"'];
    close.resize(hashes + 1, b'#');
    let to = find(b, from, &close).unwrap_or(b.len());
    Some((Token::Literal, from, to, to + close.len()))
}

/// `src` with comments and the contents of string and char literals blanked
/// (quotes and prefixes kept), offsets and lines unchanged.
pub(crate) fn mask(src: &str) -> String {
    blank_regions(src, |b, i| {
        token_at(b, i).map(|(_, from, to, next)| (from, to, next))
    })
}

/// `src` with comments blanked, literals kept.
pub(crate) fn code(src: &str) -> String {
    blank_regions(src, |b, i| {
        token_at(b, i).map(|(kind, from, to, next)| match kind {
            Token::Comment => (from, to, next),
            Token::Literal => (next, next, next),
        })
    })
}

/// The string literal starting at `at` in `code` (whitespace skipped):
/// its value and the offset after it. Plain strings have the escapes `\"`,
/// `\\`, `\n`, `\t` and `\'` read; raw strings are taken as written; byte,
/// C strings and other escapes are no plain value.
pub(crate) fn string_at(code: &str, at: usize) -> Option<(String, usize)> {
    let b = code.as_bytes();
    let at = next_nonws(b, at)?;
    if !matches!(b[at], b'"' | b'r') {
        return None;
    }
    let (Token::Literal, from, to, next) = token_at(b, at)? else {
        return None;
    };
    let text = code.get(from..to)?;
    if b[at] == b'r' {
        return Some((text.to_string(), next));
    }
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        out.push(match c {
            '\\' => match chars.next()? {
                'n' => '\n',
                't' => '\t',
                c @ ('"' | '\\' | '\'') => c,
                _ => return None,
            },
            c => c,
        });
    }
    Some((out, next))
}

/// The value of a string literal spanning all of `s` (`"x"`, `r#"x"#`).
pub(crate) fn literal_value(s: &str) -> Option<String> {
    let s = s.trim();
    string_at(s, 0)
        .filter(|(_, end)| *end == s.len())
        .map(|(v, _)| v)
}

/// One attribute `#[path(args)]`: offsets in the masked text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Attr {
    /// The `#`.
    pub(crate) at: usize,
    /// The closing `]`.
    pub(crate) end: usize,
    /// The brackets around the arguments, if any.
    pub(crate) args: Option<(usize, usize)>,
}

impl Attr {
    /// The path, e.g. `tauri::command`, `route`, `derive`.
    pub(crate) fn path<'a>(&self, mask: &'a str) -> &'a str {
        let open = mask[self.at..self.end]
            .find('[')
            .map_or(self.at, |o| self.at + o);
        let stop = self.args.map_or(self.end, |(o, _)| o);
        mask[open + 1..stop].trim()
    }

    /// The last segment of the path.
    pub(crate) fn name<'a>(&self, mask: &'a str) -> &'a str {
        let path = self.path(mask);
        path.rsplit("::").next().unwrap_or(path).trim()
    }
}

/// The attribute whose `#` is at `at` (`#[…]` or `#![…]`).
pub(crate) fn attr_at(mask: &str, at: usize) -> Option<Attr> {
    let b = mask.as_bytes();
    let mut open = next_nonws(b, at + 1)?;
    if b[open] == b'!' {
        open = next_nonws(b, open + 1)?;
    }
    if b[at] != b'#' || b[open] != b'[' {
        return None;
    }
    let end = matching(b, open)?;
    let args = (open + 1..end)
        .find(|&k| matches!(b[k], b'(' | b'[' | b'{' | b'='))
        .filter(|&k| b[k] != b'=')
        .and_then(|k| Some((k, matching(b, k)?)));
    Some(Attr { at, end, args })
}

/// The attributes right before the item keyword at `at` (`fn`, `enum`,
/// `struct`), in source order; visibility and `async`, `const`, `unsafe`
/// between them and the keyword are skipped.
pub(crate) fn attrs_before(mask: &str, at: usize) -> Vec<Attr> {
    let b = mask.as_bytes();
    let mut k = at;
    let mut out = vec![];
    while let Some(p) = prev_nonws(b, k) {
        let word = prev_word(mask, k);
        if matches!(word, "pub" | "async" | "const" | "unsafe" | "default") {
            k = p + 1 - word.len();
        } else if b[p] == b')' && prev_word(mask, matching_back(b, p).unwrap_or(0)) == "pub" {
            k = matching_back(b, p).unwrap_or(0);
        } else if b[p] == b']' {
            let Some(attr) = matching_back(b, p)
                .and_then(|o| prev_nonws(b, o))
                .and_then(|h| attr_at(mask, h))
            else {
                break;
            };
            out.push(attr);
            k = attr.at;
        } else {
            break;
        }
    }
    out.reverse();
    out
}

/// Offsets of `word` with no identifier byte on either side.
pub(crate) fn words(text: &str, word: &str) -> Vec<usize> {
    let b = text.as_bytes();
    (text.match_indices(word))
        .map(|(at, _)| at)
        .filter(|&at| at == 0 || !is_id(b[at - 1]))
        .filter(|&at| b.get(at + word.len()).is_none_or(|&c| !is_id(c)))
        .collect()
}

/// Bodies of `name! { … }`, `name!( … )` and `name![ … ]`: the offsets of
/// their opening and closing brackets.
pub(crate) fn macro_bodies(mask: &str, name: &str) -> Vec<(usize, usize)> {
    let b = mask.as_bytes();
    let body = |at: usize| {
        let bang = next_nonws(b, at + name.len()).filter(|&k| b[k] == b'!')?;
        let open = next_nonws(b, bang + 1).filter(|&k| matches!(b[k], b'{' | b'(' | b'['))?;
        Some((open, matching(b, open)?))
    };
    words(mask, name).into_iter().filter_map(body).collect()
}

/// A `fn` item with a body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FnItem {
    pub(crate) name: String,
    /// The `fn` keyword.
    pub(crate) at: usize,
    /// The braces of the body.
    pub(crate) body: (usize, usize),
}

/// Every `fn` with a body, in source order (nested ones included).
pub(crate) fn fn_items(mask: &str) -> Vec<FnItem> {
    let b = mask.as_bytes();
    let item = |at: usize| {
        let start = next_nonws(b, at + 2)?;
        let end = crate::lang_text::ident_end(b, start);
        let name = mask[start..end].to_string();
        let mut depth = 0i32;
        let open = (end..b.len()).find(|&k| {
            match b[k] {
                b'(' | b'[' => depth += 1,
                b')' | b']' => depth -= 1,
                _ => {}
            }
            depth == 0 && matches!(b[k], b'{' | b';')
        })?;
        let body = (b[open] == b'{').then(|| matching(b, open)).flatten()?;
        (!name.is_empty()).then_some(FnItem {
            name,
            at,
            body: (open, body),
        })
    };
    words(mask, "fn").into_iter().filter_map(item).collect()
}

/// The innermost `fn` whose body holds `at`.
pub(crate) fn fn_at(fns: &[FnItem], at: usize) -> Option<&FnItem> {
    (fns.iter())
        .filter(|f| f.body.0 < at && at < f.body.1)
        .max_by_key(|f| f.body.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_blanks_comments_and_literal_contents_keeping_offsets() {
        let src = "let a = \"x // y\"; // c\n/* a /* nested */ still */ let b = 'q';";
        let m = mask(src);
        assert_eq!(m.len(), src.len());
        assert_eq!(&m[..16], "let a = \"      \"");
        assert!(!m.contains("still") && !m.contains("nested") && !m.contains('q'));
        assert!(m.contains("let b = ' ';"), "{m}");
        assert_eq!(m.matches('\n').count(), 1);
    }

    #[test]
    fn raw_byte_and_c_strings_are_literals() {
        let src =
            r####"let a = r#"he said "hi" // no"#; let b = br"x"; let c = c"y"; let d = b'"';"####;
        let m = mask(src);
        assert!(!m.contains("hi") && !m.contains("no"), "{m}");
        assert!(
            m.contains("let b = br\" \";") && m.contains("let c = c\" \";"),
            "{m}"
        );
        assert!(m.ends_with("let d = b' ';"), "{m}");
        let (value, _) = string_at(src, src.find("r#").unwrap()).unwrap();
        assert_eq!(value, "he said \"hi\" // no");
    }

    #[test]
    fn lifetimes_and_labels_are_no_char_literals() {
        let src = "fn f<'a>(x: &'a str) -> char { 'outer: loop { break 'outer; } '\\'' }";
        let m = mask(src);
        assert!(m.contains("<'a>") && m.contains("&'a str") && m.contains("'outer: loop"));
        assert!(m.ends_with("'  ' }"), "{m}");
        let src = "let c = '\\u{1F600}'; let d = 'ä'; let r#type = 1;";
        let m = mask(src);
        assert!(m.contains("let r#type = 1;") && !m.contains("1F600") && !m.contains('ä'));
    }

    #[test]
    fn string_values_read_escapes_and_refuse_others() {
        assert_eq!(literal_value(r#""a\"b\\c""#).as_deref(), Some("a\"b\\c"));
        assert_eq!(literal_value(r##"r#"/x"#"##).as_deref(), Some("/x"));
        assert_eq!(literal_value(r#""\x41""#), None);
        assert_eq!(literal_value(r#""a" + b"#), None);
        assert_eq!(literal_value("b\"x\""), None);
        assert_eq!(code("a /* \"b\" */ \"// c\""), "a           \"// c\"");
    }

    #[test]
    fn attributes_before_an_item_in_source_order() {
        let src = "#[derive(Routable, Clone)]\n#[rustfmt::skip]\npub(crate) enum Route {}\n\
                   #[tauri::command]\npub async fn greet() {}";
        let m = mask(src);
        let attrs = attrs_before(&m, m.find("enum").unwrap());
        let paths: Vec<&str> = attrs.iter().map(|a| a.path(&m)).collect();
        assert_eq!(paths, ["derive", "rustfmt::skip"]);
        let (o, c) = attrs[0].args.unwrap();
        assert_eq!(&m[o..=c], "(Routable, Clone)");
        let fns = attrs_before(&m, m.find("fn greet").unwrap());
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].name(&m), "command");
        assert!(attrs_before(&m, m.find("Route").unwrap()).is_empty());
    }

    #[test]
    fn macro_bodies_and_fn_items() {
        let src = "fn App() -> Element { rsx! { div { \"}\" } } }\n\
                   fn g<T: Fn() -> u8>(x: [u8; 2]) where T: Copy { html!(<a/>) }\nfn decl();";
        let m = mask(src);
        let bodies = macro_bodies(&m, "rsx");
        assert_eq!(bodies.len(), 1);
        assert_eq!(&m[bodies[0].0..=bodies[0].1], "{ div { \" \" } }");
        assert_eq!(macro_bodies(&m, "html").len(), 1);
        let fns = fn_items(&m);
        let names: Vec<&str> = fns.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["App", "g"]);
        let inside = m.find("html").unwrap();
        assert_eq!(fn_at(&fns, inside).unwrap().name, "g");
        assert_eq!(words("a fn fnx _fn fn", "fn"), [2, 13]);
    }
}
