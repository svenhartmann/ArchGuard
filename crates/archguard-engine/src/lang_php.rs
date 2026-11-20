//! PHP lexing shared by the PHP readers of ADR 0021 (Laravel): comments
//! (`//`, `#`, `/* */`; `#[` opens an attribute, no comment), single- and
//! double-quoted strings over several lines, heredoc and nowdoc, and reading
//! of literals, arguments, arrays, `use` imports, the namespace and class and
//! method blocks over the masked text (same offsets as the source).
use crate::lang_blank::{blank_regions, find, line_end};
use crate::lang_text::{ident_end, is_id, matching, next_nonws, prev_word};
use std::collections::BTreeMap;
use std::ops::Range;

/// The comment (`true`) or literal (`false`) at `i`: (comment, blank from,
/// blank to, continue at); for a literal `from..to` is its content.
fn token_at(b: &[u8], i: usize) -> Option<(bool, usize, usize, usize)> {
    let next = b.get(i + 1).copied();
    match b[i] {
        b'/' if next == Some(b'/') => Some((true, i, line_end(b, i), line_end(b, i))),
        b'#' if next != Some(b'[') => Some((true, i, line_end(b, i), line_end(b, i))),
        b'/' if next == Some(b'*') => {
            let end = find(b, i + 2, b"*/").map_or(b.len(), |p| p + 2);
            Some((true, i, end, end))
        }
        b'\'' | b'"' => {
            let mut j = i + 1;
            while j < b.len() && b[j] != b[i] {
                j += if b[j] == b'\\' { 2 } else { 1 };
            }
            let j = j.min(b.len());
            Some((false, i + 1, j, j + 1))
        }
        b'<' if b[i..].starts_with(b"<<<") => heredoc(b, i).map(|(f, t, n)| (false, f, t, n)),
        _ => None,
    }
}

/// `<<<ID`, `<<<"ID"` or `<<<'ID'` up to the line whose first word is `ID`
/// (PHP 7.3 allows it indented): (content from, content to, continue at).
fn heredoc(b: &[u8], i: usize) -> Option<(usize, usize, usize)> {
    let mut k = i + 3;
    while matches!(b.get(k), Some(b' ' | b'\t')) {
        k += 1;
    }
    let quoted = matches!(b.get(k), Some(b'\'' | b'"'));
    let start = k + usize::from(quoted);
    let end = ident_end(b, start);
    if end == start || b[start].is_ascii_digit() {
        return None;
    }
    let id = &b[start..end];
    let from = line_end(b, end) + 1;
    let mut line = from;
    while line < b.len() {
        let word = (line..b.len()).find(|&j| !matches!(b[j], b' ' | b'\t'))?;
        if b[word..].starts_with(id) && !b.get(word + id.len()).is_some_and(|&c| is_id(c)) {
            return Some((from, line, word + id.len()));
        }
        line = line_end(b, line) + 1;
    }
    Some((from.min(b.len()), b.len(), b.len()))
}

/// `src` with comments and the contents of strings, heredocs and nowdocs
/// blanked (quotes and heredoc markers kept), offsets and lines unchanged.
pub(crate) fn mask(src: &str) -> String {
    blank_regions(src, |b, i| {
        token_at(b, i).map(|(_, from, to, next)| (from, to, next))
    })
}

/// `src` with comments blanked, literals kept.
pub(crate) fn code(src: &str) -> String {
    blank_regions(src, |b, i| {
        token_at(b, i).map(|(comment, from, to, next)| match comment {
            true => (from, to, next),
            false => (next, next, next),
        })
    })
}

/// The value of a plain string literal spanning all of `s` (`'a\'b'`,
/// `"a"`); `None` for interpolation (`"$x"`), heredocs and anything computed.
pub(crate) fn literal(s: &str) -> Option<String> {
    let s = s.trim();
    let b = s.as_bytes();
    let q = *b.first()?;
    if !matches!(q, b'\'' | b'"') || token_at(b, 0).map(|t| t.3) != Some(b.len()) {
        return None;
    }
    let (mut out, mut chars) = (String::new(), s[1..s.len() - 1].chars());
    while let Some(c) = chars.next() {
        match (c, q) {
            ('$', b'"') => return None,
            ('\\', _) => match (chars.next(), q) {
                (Some(e @ ('\\' | '\'')), b'\'') => out.push(e),
                (Some(e @ ('\\' | '"' | '$')), b'"') => out.push(e),
                (Some('n'), b'"') => out.push('\n'),
                (Some('t'), b'"') => out.push('\t'),
                (Some(e), _) => out.extend(['\\', e]),
                (None, _) => out.push('\\'),
            },
            _ => out.push(c),
        }
    }
    Some(out)
}

/// The bracket closing the one at `open` in raw PHP text, skipping strings
/// and comments; for expressions cut out of templates (Blade directives).
pub(crate) fn close(b: &[u8], open: usize) -> Option<usize> {
    let (o, z) = match b.get(open)? {
        b'(' => (b'(', b')'),
        b'[' => (b'[', b']'),
        b'{' => (b'{', b'}'),
        _ => return None,
    };
    let (mut depth, mut k) = (0i32, open);
    while k < b.len() {
        if let Some((_, _, _, next)) = token_at(b, k) {
            k = next.max(k + 1);
            continue;
        }
        depth += i32::from(b[k] == o) - i32::from(b[k] == z);
        if depth == 0 {
            return Some(k);
        }
        k += 1;
    }
    None
}

/// The trimmed top-level items of the `(…)` or `[…]` at `open` in masked
/// text (a trailing comma adds none), and the offset of the closing bracket.
pub(crate) fn args(mask: &str, open: usize) -> Option<(Vec<Range<usize>>, usize)> {
    let b = mask.as_bytes();
    let end = matching(b, open)?;
    let (mut out, mut depth, mut start) = (Vec::new(), 0i32, open + 1);
    for (k, &c) in b.iter().enumerate().take(end + 1).skip(open + 1) {
        match c {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' if depth > 0 => depth -= 1,
            b',' | b')' | b']' | b'}' if depth == 0 => {
                let r = trim(mask, start..k);
                if !r.is_empty() {
                    out.push(r);
                }
                start = k + 1;
            }
            _ => {}
        }
    }
    Some((out, end))
}

/// `range` without surrounding whitespace.
pub(crate) fn trim(s: &str, range: Range<usize>) -> Range<usize> {
    let part = &s[range.clone()];
    let start = range.start + (part.len() - part.trim_start().len());
    start..start.max(range.start + part.trim_end().len())
}

/// The items of the array at `open` (`[` or `array(` at its `(`): the key
/// range for `k => v`, and the value range.
pub(crate) fn array_items(mask: &str, open: usize) -> Vec<(Option<Range<usize>>, Range<usize>)> {
    let Some((items, _)) = args(mask, open) else {
        return vec![];
    };
    (items.into_iter())
        .map(|r| match top_level(mask, r.clone(), "=>") {
            Some(k) => (Some(trim(mask, r.start..k)), trim(mask, k + 2..r.end)),
            None => (None, r),
        })
        .collect()
}

/// First `pat` in `mask[range]` outside brackets.
pub(crate) fn top_level(mask: &str, range: Range<usize>, pat: &str) -> Option<usize> {
    let b = mask.as_bytes();
    let mut depth = 0i32;
    for k in range.clone() {
        match b[k] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            _ if depth == 0 && b[k..range.end].starts_with(pat.as_bytes()) => return Some(k),
            _ => {}
        }
    }
    None
}

/// The `;` ending the statement that starts at `from` (outside brackets),
/// or the first unmatched closing bracket, or the end.
pub(crate) fn statement_end(mask: &str, from: usize) -> usize {
    let b = mask.as_bytes();
    let mut depth = 0i32;
    for (k, &c) in b.iter().enumerate().skip(from) {
        match c {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' if depth == 0 => return k,
            b')' | b']' | b'}' => depth -= 1,
            b';' if depth == 0 => return k,
            _ => {}
        }
    }
    b.len()
}

/// Whether the word at `at` stands alone: no identifier character, `$`,
/// `->` or `::` right before it.
pub(crate) fn word_at(b: &[u8], at: usize, word: &str) -> bool {
    b[at..].starts_with(word.as_bytes())
        && !b.get(at + word.len()).is_some_and(|&c| is_id(c))
        && (at == 0 || !(is_id(b[at - 1]) || b[..at].ends_with(b"->") || b[..at].ends_with(b"::")))
}

/// Offsets of `word` standing alone ([`word_at`]) in `mask[range]`.
pub(crate) fn words(mask: &str, range: Range<usize>, word: &str) -> Vec<usize> {
    let b = mask.as_bytes();
    let mut out = vec![];
    let mut from = range.start;
    while let Some(k) = find(&b[..range.end], from, word.as_bytes()) {
        if word_at(b, k, word) {
            out.push(k);
        }
        from = k + 1;
    }
    out
}

/// The namespace the file declares (`namespace A\B;`), `""` for none.
pub(crate) fn namespace(mask: &str) -> String {
    let at = words(mask, 0..mask.len(), "namespace").into_iter().next();
    at.map_or(String::new(), |k| {
        let end = statement_end(mask, k);
        let name = mask[k + 9..end].trim();
        let name = name.trim_end_matches('{').trim();
        name.trim_start_matches('\\').to_string()
    })
}

/// Class imports of the file (`use A\B;`, `use A\B as C;`, `use A\{B, C as
/// D};`) as alias to fully qualified name; `use function`/`use const` and
/// trait uses inside classes are left out.
pub(crate) fn uses(mask: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for k in words(mask, 0..mask.len(), "use") {
        if depth_at(mask.as_bytes(), k) != 0 {
            continue;
        }
        let end = statement_end(mask, k);
        let text = mask[k + 3..end].trim();
        if text.starts_with("function ") || text.starts_with("const ") {
            continue;
        }
        let (base, items) = match (text.find('{'), text.rfind('}')) {
            (Some(o), Some(z)) if o < z => (&text[..o], &text[o + 1..z]),
            _ => ("", text),
        };
        for item in items.split(',').map(str::trim).filter(|i| !i.is_empty()) {
            let (path, alias) = match item.split_once(" as ") {
                Some((p, a)) => (p.trim(), a.trim()),
                None => (item, item.rsplit('\\').next().unwrap_or(item)),
            };
            let full = format!("{base}{path}").trim_start_matches('\\').to_string();
            out.insert(alias.to_string(), full);
        }
    }
    out
}

/// Brace depth at `at` (masked text).
fn depth_at(b: &[u8], at: usize) -> i32 {
    b[..at]
        .iter()
        .map(|&c| i32::from(c == b'{') - i32::from(c == b'}'))
        .sum()
}

/// A `class`, `enum`, `trait` or `interface` block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Class {
    pub kind: String,
    pub name: String,
    /// Offset of the name.
    pub at: usize,
    /// From after the name to the opening brace (`extends`, `: string`).
    pub header: Range<usize>,
    /// Inside the braces.
    pub body: Range<usize>,
    pub methods: Vec<Method>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Method {
    pub name: String,
    /// Offset of the name.
    pub at: usize,
    /// Inside the parentheses.
    pub params: Range<usize>,
    /// Inside the braces; none for an abstract or interface method.
    pub body: Option<Range<usize>>,
}

/// The named classes, enums, traits and interfaces of masked text with their
/// methods (functions directly in the body); anonymous classes are skipped.
pub(crate) fn classes(mask: &str) -> Vec<Class> {
    let b = mask.as_bytes();
    let mut out = vec![];
    for kind in ["class", "enum", "trait", "interface"] {
        for k in words(mask, 0..mask.len(), kind) {
            let Some(at) = next_nonws(b, k + kind.len()) else {
                continue;
            };
            let end = ident_end(b, at);
            if end == at || prev_word(mask, k) == "new" {
                continue;
            }
            let Some(open) = (end..b.len()).find(|&j| matches!(b[j], b'{' | b';')) else {
                continue;
            };
            let Some(close) = (b[open] == b'{').then(|| matching(b, open)).flatten() else {
                continue;
            };
            let body = open + 1..close;
            out.push(Class {
                kind: kind.into(),
                name: mask[at..end].to_string(),
                at,
                header: end..open,
                methods: methods(mask, body.clone()),
                body,
            });
        }
    }
    out.sort_by_key(|c| c.at);
    out
}

fn methods(mask: &str, body: Range<usize>) -> Vec<Method> {
    let b = mask.as_bytes();
    let mut out = vec![];
    let mut k = body.start;
    while k < body.end {
        match b[k] {
            b'{' => k = matching(b, k).unwrap_or(body.end) + 1,
            _ if word_at(b, k, "function") => {
                let Some(at) = next_nonws(b, k + 8) else {
                    break;
                };
                let end = ident_end(b, at);
                let open = next_nonws(b, end).filter(|&o| b[o] == b'(');
                let Some((open, close)) = open.and_then(|o| Some((o, matching(b, o)?))) else {
                    k += 8;
                    continue;
                };
                let stop = (close..body.end).find(|&j| matches!(b[j], b'{' | b';'));
                let block = stop.filter(|&j| b[j] == b'{');
                let body_end = block.and_then(|o| matching(b, o));
                out.push(Method {
                    name: mask[at..end].to_string(),
                    at,
                    params: open + 1..close,
                    body: block.zip(body_end).map(|(o, z)| o + 1..z),
                });
                k = body_end.or(stop).map_or(body.end, |j| j + 1);
            }
            _ => k += 1,
        }
    }
    out
}

/// The fully qualified name `name` (`Alias\Rest`, `\A\B`, `B`) means in a
/// file with `uses` and `namespace`.
pub(crate) fn qualify(name: &str, uses: &BTreeMap<String, String>, namespace: &str) -> String {
    if let Some(abs) = name.strip_prefix('\\') {
        return abs.to_string();
    }
    let (first, rest) = name.split_once('\\').unwrap_or((name, ""));
    match (uses.get(first), rest) {
        (Some(full), "") => full.clone(),
        (Some(full), rest) => format!("{full}\\{rest}"),
        (None, _) if namespace.is_empty() => name.to_string(),
        (None, _) => format!("{namespace}\\{name}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_blanks_comments_strings_and_heredocs_but_keeps_attributes() {
        let src = "<?php\n# c\n#[Attr('x')]\n$a = 'it\\'s'; // d\n$b = \"q\\\"\n$x\";\n\
                   $h = <<<EOT\n  {$a} 'x'\n  EOT;\n$n = <<<'N'\nraw\nN;\n/* e */ f();\n";
        let m = mask(src);
        assert_eq!(m.len(), src.len());
        assert_eq!(m.lines().count(), src.lines().count());
        assert!(m.contains("#[Attr(' ')]"), "{m:?}");
        assert!(m.contains("$a = '     ';"), "{m:?}");
        assert!(m.contains("$b = \"   \n  \";"), "{m:?}");
        assert!(m.contains("$h = <<<EOT\n          \n  EOT;"), "{m:?}");
        assert!(m.contains("$n = <<<'N'\n   \nN;"), "{m:?}");
        assert!(!m.contains("# c") && !m.contains("// d") && !m.contains("/* e */"));
        assert!(m.contains("f();"));
        let c = code("f('a // b'); // gone\n# gone too\n");
        assert_eq!(c, "f('a // b');        \n          \n");
    }

    #[test]
    fn literal_reads_plain_strings_only() {
        assert_eq!(literal(" 'a\\'b\\\\c' ").as_deref(), Some("a'b\\c"));
        assert_eq!(literal("\"x\\\"y\\n\"").as_deref(), Some("x\"y\n"));
        assert_eq!(literal("'$x'").as_deref(), Some("$x"));
        assert_eq!(literal("\"$x\""), None);
        assert_eq!(literal("'a' . 'b'"), None);
        assert_eq!(literal("X::Y"), None);
    }

    #[test]
    fn args_and_arrays_split_at_top_level_commas() {
        let src = "f('a,b', [1, 2], g(3, 4), fn($x) => $x,)";
        let m = mask(src);
        let (items, end) = args(&m, 1).unwrap();
        assert_eq!(end, src.len() - 1);
        let parts: Vec<&str> = items.iter().map(|r| &src[r.clone()]).collect();
        assert_eq!(parts, ["'a,b'", "[1, 2]", "g(3, 4)", "fn($x) => $x"]);
        let src = "['only' => ['a', 'b'], 'x', 'k' => fn() => 1]";
        let m = mask(src);
        let items = array_items(&m, 0);
        let text = |r: &Range<usize>| &src[r.clone()];
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].0.as_ref().map(text), Some("'only'"));
        assert_eq!(text(&items[0].1), "['a', 'b']");
        assert_eq!((items[1].0.as_ref(), text(&items[1].1)), (None, "'x'"));
        assert_eq!(text(&items[2].1), "fn() => 1");
        let b = "@if(f(')') && g(\"(\"))".as_bytes();
        assert_eq!(close(b, 3), Some(b.len() - 1));
        assert_eq!(statement_end("a(b; c); d", 0), 7);
    }

    #[test]
    fn uses_namespace_and_qualify() {
        let src = "<?php\nnamespace App\\Http;\nuse A\\B;\nuse C\\D as E, \\F\\G;\n\
                   use H\\{I, J\\K as L};\nuse function h;\nclass X { use T; }\n";
        let m = mask(src);
        assert_eq!(namespace(&m), "App\\Http");
        let u = uses(&m);
        let pairs: Vec<(&str, &str)> = u.iter().map(|(a, f)| (a.as_str(), f.as_str())).collect();
        assert_eq!(
            pairs,
            [
                ("B", "A\\B"),
                ("E", "C\\D"),
                ("G", "F\\G"),
                ("I", "H\\I"),
                ("L", "H\\J\\K")
            ]
        );
        assert_eq!(qualify("E\\X", &u, "N"), "C\\D\\X");
        assert_eq!(qualify("\\Q\\R", &u, "N"), "Q\\R");
        assert_eq!(qualify("R", &u, "N"), "N\\R");
        assert_eq!(qualify("B", &u, ""), "A\\B");
    }

    #[test]
    fn classes_and_methods_with_bodies() {
        let src = "<?php\nabstract class A extends B implements C {\n  const X = 'x';\n\
                   public function __construct(private $s) { $f = function () { return 1; }; }\n\
                   abstract protected function m(): void;\n  public function show(int $id): View\n  {\n\
                   return view('a', ['k' => fn() => 2]);\n  }\n}\n\
                   enum P: string { case A = 'a'; public function mw(): string { return 'can:' . $this->value; } }\n\
                   $o = new class { function z() {} };\n$n = X::class;\n";
        let m = mask(src);
        let cs = classes(&m);
        let names: Vec<(&str, &str)> = cs
            .iter()
            .map(|c| (c.kind.as_str(), c.name.as_str()))
            .collect();
        assert_eq!(names, [("class", "A"), ("enum", "P")]);
        let a = &cs[0];
        assert!(src[a.header.clone()].contains("extends B"));
        let ms: Vec<(&str, bool)> = a
            .methods
            .iter()
            .map(|x| (x.name.as_str(), x.body.is_some()))
            .collect();
        assert_eq!(ms, [("__construct", true), ("m", false), ("show", true)]);
        let show = &a.methods[2];
        assert_eq!(&src[show.params.clone()], "int $id");
        assert!(
            src[show.body.clone().unwrap()]
                .trim()
                .starts_with("return view('a'")
        );
        assert_eq!(cs[1].methods[0].name, "mw");
        assert_eq!(
            words(&m, 0..m.len(), "class").len(),
            2,
            "X::class is no keyword"
        );
    }
}
