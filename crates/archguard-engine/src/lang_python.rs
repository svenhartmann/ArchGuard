//! Python lexing shared by the Python extractors of ADR 0021 (Django; the
//! FastAPI extractor moves here): masking, string literals, call arguments,
//! logical lines, `def`/`class` blocks by indentation, and imports resolved
//! within the scanned files. Structure is read from the masked text, literals
//! from the source at the same offsets.
use crate::lang_blank::{blank_regions, find, line_end};
use crate::lang_text::{is_id, line, matching, unparen};
use std::collections::BTreeSet;

/// `src` with comments and string literal contents blanked (quotes and
/// prefixes kept), offsets and lines unchanged. Triple-quoted strings span
/// lines; others end at a line break at the latest. In raw strings `\` still
/// keeps the next quote from ending the literal, as in Python.
pub(crate) fn mask(src: &str) -> String {
    blank_regions(src, |b, i| match b[i] {
        b'#' => Some((i, line_end(b, i), line_end(b, i))),
        b'\'' | b'"' => Some(literal_at(b, i)),
        _ => None,
    })
}

/// The literal whose first quote is at `i`: (content from, content to, continue at).
fn literal_at(b: &[u8], i: usize) -> (usize, usize, usize) {
    let q = b[i];
    let triple = b[i..].starts_with(&[q, q, q]);
    let open = if triple { 3 } else { 1 };
    let mut j = i + open;
    while j < b.len() {
        match b[j] {
            b'\\' => j += 2,
            b'\n' if !triple => return (i + 1, j, j),
            c if c == q && (!triple || b[j..].starts_with(&[q, q, q])) => {
                return (i + open, j, j + open);
            }
            _ => j += 1,
        }
    }
    (i + open, b.len(), b.len() + 1)
}

/// The value of `expr` when it is one string literal or adjacent literals,
/// optionally in parentheses. Prefixes `r`, `b`, `u`, `f` in any case; an
/// f-string with a `{…}` hole is no literal, `{{`/`}}` are braces.
pub(crate) fn string_value(expr: &str) -> Option<String> {
    let expr = unparen(expr);
    let b = expr.as_bytes();
    let (mut out, mut i, mut any) = (String::new(), 0, false);
    while i < b.len() {
        if b[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let prefix = b[i..]
            .iter()
            .take_while(|c| c.is_ascii_alphabetic())
            .count();
        let p = expr[i..i + prefix].to_ascii_lowercase();
        let quote = b.get(i + prefix).copied();
        if prefix > 2
            || !p.chars().all(|c| "rbuf".contains(c))
            || !matches!(quote, Some(b'\'' | b'"'))
        {
            return None;
        }
        let (from, to, next) = literal_at(b, i + prefix);
        if next > b.len() || to == next {
            return None;
        }
        let text = &expr[from..to];
        let text = if p.contains('f') {
            f_text(text)?
        } else {
            text.to_string()
        };
        out.push_str(&if p.contains('r') {
            text
        } else {
            unescape(&text)
        });
        (i, any) = (next, true);
    }
    any.then_some(out)
}

/// An f-string's text without holes, braces undoubled; none with a hole.
fn f_text(text: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if matches!(c, '{' | '}') && chars.next() != Some(c) {
            return None;
        }
        out.push(c);
    }
    Some(out)
}

/// The common escapes; an unknown one keeps its backslash, as in Python.
fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some(e @ ('\\' | '\'' | '"')) => out.push(e),
            Some('\n') => {}
            Some(e) => out.extend(['\\', e]),
            None => out.push('\\'),
        }
    }
    out
}

/// One argument of a call: keyword name (`None` when positional, also for
/// `*a` and `**kw`), value as written and trimmed, offset of the argument.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Arg<'a> {
    pub(crate) name: Option<&'a str>,
    pub(crate) value: &'a str,
    pub(crate) at: usize,
}

/// The arguments of the call whose `(` is at `open` in `masked` (same offsets
/// as `src`), and the offset of its `)`; none when it is not closed.
pub(crate) fn call_args<'a>(
    src: &'a str,
    masked: &str,
    open: usize,
) -> Option<(Vec<Arg<'a>>, usize)> {
    let close = matching(masked.as_bytes(), open)?;
    let args = (split_commas(masked, open + 1, close).into_iter())
        .filter_map(|(from, to)| {
            let raw = &src[from..to];
            let at = from + (raw.len() - raw.trim_start().len());
            let text = raw.trim();
            if text.is_empty() {
                return None;
            }
            let m = &masked[at..at + text.len()];
            let name_len = m.bytes().take_while(|&c| is_id(c)).count();
            let rest = m[name_len..].trim_start();
            if name_len > 0 && rest.starts_with('=') && !rest.starts_with("==") {
                let value = text[text.len() - rest.len() + 1..].trim();
                return Some(Arg {
                    name: Some(&text[..name_len]),
                    value,
                    at,
                });
            }
            Some(Arg {
                name: None,
                value: text,
                at,
            })
        })
        .collect();
    Some((args, close))
}

/// Ranges of `masked[from..to]` between commas outside brackets.
pub(crate) fn split_commas(masked: &str, from: usize, to: usize) -> Vec<(usize, usize)> {
    let b = masked.as_bytes();
    let (mut out, mut depth, mut start) = (vec![], 0i32, from);
    for (k, &c) in b.iter().enumerate().take(to).skip(from) {
        match c {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b',' if depth == 0 => {
                out.push((start, k));
                start = k + 1;
            }
            _ => {}
        }
    }
    out.push((start, to));
    out
}

/// One logical line: from its first non-blank byte to its end (brackets and
/// `\` continuations joined, trailing blanks and the line break excluded).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Stmt {
    pub(crate) at: usize,
    pub(crate) end: usize,
    /// Columns before `at`.
    pub(crate) indent: usize,
}

/// Every non-blank logical line of `masked` (comments are blank there). A
/// `;` does not split statements.
pub(crate) fn statements(masked: &str) -> Vec<Stmt> {
    let b = masked.as_bytes();
    let (mut out, mut i) = (vec![], 0);
    while i < b.len() {
        let line_start = i;
        while i < b.len() && matches!(b[i], b' ' | b'\t' | b'\r') {
            i += 1;
        }
        if i >= b.len() || b[i] == b'\n' {
            i += 1;
            continue;
        }
        let at = i;
        let mut depth = 0i32;
        while i < b.len() {
            match b[i] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                b'\\' if b.get(i + 1) == Some(&b'\n') => i += 1,
                b'\n' if depth <= 0 => break,
                _ => {}
            }
            i += 1;
        }
        let end = at + masked[at..i.min(b.len())].trim_end().len();
        out.push(Stmt {
            at,
            end,
            indent: at - line_start,
        });
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum DefKind {
    Class,
    Function,
}

/// A `class` or `def` (also `async def`) block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Def {
    pub(crate) kind: DefKind,
    pub(crate) name: String,
    /// Line of the `class`/`def` keyword.
    pub(crate) line: u32,
    pub(crate) indent: usize,
    /// Inside the parentheses after the name: bases or parameters.
    pub(crate) head: Option<(usize, usize)>,
    /// Each decorator's expression after `@`, outermost first.
    pub(crate) decorators: Vec<(usize, usize)>,
    /// After the header's `:` to the end of the block's last statement.
    pub(crate) body: (usize, usize),
    /// Index of the enclosing block in the returned list.
    pub(crate) parent: Option<usize>,
}

/// Every `class` and `def` of `masked`, in text order.
pub(crate) fn defs(masked: &str) -> Vec<Def> {
    let stmts = statements(masked);
    let mut out: Vec<Def> = vec![];
    for (n, s) in stmts.iter().enumerate() {
        let text = &masked[s.at..s.end];
        let text = text.strip_prefix("async ").map_or(text, str::trim_start);
        let (kind, rest) = match text.split_once(char::is_whitespace) {
            Some(("class", r)) => (DefKind::Class, r.trim_start()),
            Some(("def", r)) => (DefKind::Function, r.trim_start()),
            _ => continue,
        };
        let name_len = rest.bytes().take_while(|&c| is_id(c)).count();
        if name_len == 0 {
            continue;
        }
        let after = s.end - rest.len() + name_len;
        let tail = masked[after..s.end].trim_start();
        let head = (tail.starts_with('('))
            .then(|| s.end - tail.len())
            .and_then(|open| matching(masked.as_bytes(), open).map(|close| (open + 1, close)));
        let header_end = head.map_or(after, |h| h.1);
        let Some(colon) = find(masked.as_bytes(), header_end, b":").filter(|&c| c < s.end) else {
            continue;
        };
        let last = (stmts[n + 1..].iter())
            .take_while(|t| t.indent > s.indent)
            .last()
            .map_or(s.end, |t| t.end);
        let mut decorators: Vec<(usize, usize)> = (stmts[..n].iter().rev())
            .take_while(|d| d.indent == s.indent && masked[d.at..].starts_with('@'))
            .map(|d| (d.at + 1, d.end))
            .collect();
        decorators.reverse();
        let parent = (0..out.len())
            .rev()
            .find(|&p| out[p].body.0 <= s.at && s.at < out[p].body.1);
        out.push(Def {
            kind,
            name: rest[..name_len].to_string(),
            line: line(masked, s.at),
            indent: s.indent,
            head,
            decorators,
            body: (colon + 1, last.max(colon + 1)),
            parent,
        });
    }
    out
}

/// `target = value` or `target += value`; a type annotation on the target is dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Assign<'a> {
    pub(crate) target: &'a str,
    pub(crate) augmented: bool,
    pub(crate) value: &'a str,
    pub(crate) value_at: usize,
}

/// The assignment `stmt` makes, if it assigns one dotted name.
pub(crate) fn assignment<'a>(src: &'a str, masked: &str, stmt: &Stmt) -> Option<Assign<'a>> {
    let m = &masked[stmt.at..stmt.end];
    let b = m.as_bytes();
    let target_len = b.iter().take_while(|&&c| is_id(c) || c == b'.').count();
    if target_len == 0 {
        return None;
    }
    let mut k = target_len + (m[target_len..].len() - m[target_len..].trim_start().len());
    if b.get(k) == Some(&b':') {
        k = (k..b.len()).find(|&j| b[j] == b'=' && b.get(j + 1) != Some(&b'='))?;
    }
    let augmented = b.get(k) == Some(&b'+') && b.get(k + 1) == Some(&b'=');
    let eq = k + usize::from(augmented);
    if b.get(eq) != Some(&b'=') || b.get(eq + 1) == Some(&b'=') {
        return None;
    }
    let raw = &src[stmt.at + eq + 1..stmt.end];
    Some(Assign {
        target: &src[stmt.at..stmt.at + target_len],
        augmented,
        value: raw.trim(),
        value_at: stmt.at + eq + 1 + (raw.len() - raw.trim_start().len()),
    })
}

/// One imported name: `local` is the name bound in the importing module
/// (`*` for a star import), `module` as written (leading dots relative),
/// `name` the imported member of `from … import`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Import {
    pub(crate) local: String,
    pub(crate) module: String,
    pub(crate) name: Option<String>,
    pub(crate) line: u32,
}

/// Every `import` and `from … import` statement of `masked`, also inside
/// blocks. `import a.b` binds `a` to module `a`; `import a.b as c` binds `c`
/// to `a.b`.
pub(crate) fn imports(masked: &str) -> Vec<Import> {
    let mut out = vec![];
    for s in statements(masked) {
        let text = &masked[s.at..s.end];
        if text.starts_with("import ") {
            for (at, module, alias) in import_items(masked, s.at + "import ".len(), s.end) {
                let local = match alias == module {
                    true => module.split('.').next().unwrap_or("").to_string(),
                    false => alias,
                };
                let line = line(masked, at);
                out.push(Import {
                    local,
                    module,
                    name: None,
                    line,
                });
            }
        } else if let Some(rest) = text.strip_prefix("from ") {
            let Some(split) = rest.find(" import") else {
                continue;
            };
            let module = rest[..split].trim().to_string();
            let list_at = s.at + "from ".len() + split + " import".len();
            for (at, name, local) in import_items(masked, list_at, s.end) {
                out.push(Import {
                    local,
                    module: module.clone(),
                    name: Some(name),
                    line: line(masked, at),
                });
            }
        }
    }
    out
}

/// (offset, name, alias) of each comma-separated item of an import list at
/// `masked[from..end]`, parenthesized or not.
fn import_items(masked: &str, from: usize, end: usize) -> Vec<(usize, String, String)> {
    let list = masked[from..end].trim_start();
    let (from, to) = match list.starts_with('(') {
        true => (end - list.len() + 1, end - 1),
        false => (from, end),
    };
    (split_commas(masked, from, to).into_iter())
        .filter_map(|(a, b)| {
            let raw = &masked[a..b];
            let item = raw.trim_matches(|c: char| c.is_whitespace() || "()\\".contains(c));
            let at = a + raw.find(item).unwrap_or(0);
            let mut words = item.split_whitespace();
            let name = words.next()?.to_string();
            let alias = match (words.next(), words.next()) {
                (Some("as"), Some(alias)) => alias.to_string(),
                _ => name.clone(),
            };
            Some((at, name, alias))
        })
        .collect()
}

/// The first folder above `file` without `__init__.py` (`""` for the root):
/// where absolute imports of `file` start.
pub(crate) fn source_root<'a>(file: &'a str, files: &BTreeSet<String>) -> &'a str {
    let mut dir = parent(file);
    while !dir.is_empty() && files.contains(&format!("{dir}/__init__.py")) {
        dir = parent(dir);
    }
    dir
}

fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

fn join(dir: &str, rel: &str) -> String {
    match (dir.is_empty(), rel.is_empty()) {
        (true, _) => rel.to_string(),
        (_, true) => dir.to_string(),
        _ => format!("{dir}/{rel}"),
    }
}

/// The dotted module name of `file` below its source root (`a/b/__init__.py` is `a.b`).
pub(crate) fn module_name(file: &str, files: &BTreeSet<String>) -> String {
    let root = source_root(file, files);
    let rel = file[root.len()..].trim_start_matches('/');
    let rel = (rel.strip_suffix("/__init__.py"))
        .or_else(|| rel.strip_suffix(".py"))
        .unwrap_or(rel);
    rel.replace('/', ".")
}

/// The file of `module` as imported from `from_file`: leading dots count
/// packages up from `from_file`'s own; an absolute name starts at the source
/// root of `from_file`. `a/b.py` before `a/b/__init__.py`; none for a module
/// outside the scanned files (the standard library, installed packages).
pub(crate) fn resolve_module(
    from_file: &str,
    module: &str,
    files: &BTreeSet<String>,
) -> Option<String> {
    let dots = module.bytes().take_while(|&c| c == b'.').count();
    let mut dir = match dots {
        0 => source_root(from_file, files),
        _ => parent(from_file),
    };
    for _ in 1..dots {
        dir = parent(dir);
    }
    let rel = module[dots..].replace('.', "/");
    let base = join(dir, &rel);
    let candidates = match rel.is_empty() {
        true => vec![join(&base, "__init__.py")],
        false => vec![format!("{base}.py"), join(&base, "__init__.py")],
    };
    candidates.into_iter().find(|f| files.contains(f))
}

/// Where import `imp` of `from_file` points: the module file, and the symbol
/// in it (none when the import binds a module, e.g. `from . import views`
/// with a `views.py`).
pub(crate) fn resolve_import(
    from_file: &str,
    imp: &Import,
    files: &BTreeSet<String>,
) -> Option<(String, Option<String>)> {
    let Some(name) = &imp.name else {
        return resolve_module(from_file, &imp.module, files).map(|f| (f, None));
    };
    let sep = if imp.module.ends_with('.') { "" } else { "." };
    let sub = format!("{}{sep}{name}", imp.module);
    (resolve_module(from_file, &sub, files).map(|f| (f, None)))
        .or_else(|| resolve_module(from_file, &imp.module, files).map(|f| (f, Some(name.clone()))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_blanks_comments_and_every_kind_of_string() {
        let src = "x = 'a#b'  # c 'd'\ny = r\"\\\"q\" + f'{z}' + b\"\"\ns = '''one\n\"two\"\n''' # e\nt = \"it\\\"s\"\n";
        let m = mask(src);
        assert_eq!(m.len(), src.len());
        assert_eq!(m.lines().count(), src.lines().count());
        assert!(m.starts_with("x = '   '         \n"), "{m:?}");
        assert!(m.contains("y = r\"   \" + f'   ' + b\"\"\n"), "{m:?}");
        assert!(m.contains("s = '''   \n     \n'''    \n"), "{m:?}");
        assert!(m.ends_with("t = \"     \"\n"), "{m:?}");
    }

    #[test]
    fn string_values_read_literals_only() {
        assert_eq!(string_value(" 'a/b' ").as_deref(), Some("a/b"));
        assert_eq!(string_value("\"a\" 'b'").as_deref(), Some("ab"));
        assert_eq!(string_value("('a'\n 'b')").as_deref(), Some("ab"));
        assert_eq!(string_value(r"r'^\d+$'").as_deref(), Some(r"^\d+$"));
        assert_eq!(string_value(r"'a\'b\n'").as_deref(), Some("a'b\n"));
        assert_eq!(string_value("'''x\ny'''").as_deref(), Some("x\ny"));
        assert_eq!(string_value("f'x{{y}}'").as_deref(), Some("x{y}"));
        assert_eq!(string_value("f'x{y}'"), None);
        assert_eq!(string_value("'a' + b"), None);
        assert_eq!(string_value("name"), None);
        assert_eq!(string_value(""), None);
    }

    #[test]
    fn call_args_split_at_top_level_commas() {
        let src = "path('a, (b)', include(('x', 'y')), name='n', **kw)";
        let m = mask(src);
        let (args, close) = call_args(src, &m, 4).unwrap();
        assert_eq!(close, src.len() - 1);
        let got: Vec<_> = args.iter().map(|a| (a.name, a.value)).collect();
        assert_eq!(
            got,
            [
                (None, "'a, (b)'"),
                (None, "include(('x', 'y'))"),
                (Some("name"), "'n'"),
                (None, "**kw"),
            ]
        );
        assert_eq!(args[2].at, src.find("name=").unwrap());
        assert_eq!(call_args("f()", "f()", 1).unwrap().0, vec![]);
        assert_eq!(
            call_args("f(a == b)", "f(a == b)", 1).unwrap().0[0].name,
            None
        );
    }

    #[test]
    fn statements_join_brackets_and_continuations() {
        let src = "a = [\n  1,\n]\n\n  # only a comment\nb = 1 + \\\n  2\nif x:\n    c = 3\n";
        let m = mask(src);
        let s = statements(&m);
        let texts: Vec<_> = s.iter().map(|s| (&src[s.at..s.end], s.indent)).collect();
        assert_eq!(
            texts,
            [
                ("a = [\n  1,\n]", 0),
                ("b = 1 + \\\n  2", 0),
                ("if x:", 0),
                ("c = 3", 4),
            ]
        );
        let assign = |i: usize| assignment(src, &m, &s[i]);
        let a = assign(0).unwrap();
        assert_eq!((a.target, a.augmented, a.value), ("a", false, "[\n  1,\n]"));
        assert_eq!(a.value_at, src.find('[').unwrap());
        assert!(assign(2).is_none());
        let src = "x: list = []\nx += y\nx == 1\nf(a=1)\n";
        let m = mask(src);
        let got: Vec<_> = (statements(&m).iter())
            .map(|s| assignment(src, &m, s).map(|a| (a.target, a.augmented, a.value)))
            .collect();
        assert_eq!(
            got,
            [Some(("x", false, "[]")), Some(("x", true, "y")), None, None]
        );
    }

    #[test]
    fn defs_follow_indentation_decorators_and_nesting() {
        let src = "@decorate(\n  'x')\nclass A(B, m.C):\n    attr = 1\n\n    @action(detail=True)\n    async def go(self,\n                 pk):\n        return 1\n\n    def other(self): pass\n\ndef top():\n    pass\n";
        let m = mask(src);
        let d = defs(&m);
        let names: Vec<_> = d
            .iter()
            .map(|d| (d.kind, d.name.as_str(), d.line, d.parent))
            .collect();
        assert_eq!(
            names,
            [
                (DefKind::Class, "A", 3, None),
                (DefKind::Function, "go", 7, Some(0)),
                (DefKind::Function, "other", 11, Some(0)),
                (DefKind::Function, "top", 13, None),
            ]
        );
        let head = d[0].head.unwrap();
        assert_eq!(&src[head.0..head.1], "B, m.C");
        let dec: Vec<_> = d[0].decorators.iter().map(|&(a, b)| &src[a..b]).collect();
        assert_eq!(dec, ["decorate(\n  'x')"]);
        let dec: Vec<_> = d[1].decorators.iter().map(|&(a, b)| &src[a..b]).collect();
        assert_eq!(dec, ["action(detail=True)"]);
        let body = &src[d[0].body.0..d[0].body.1];
        assert!(
            body.contains("attr = 1") && body.contains("def other") && !body.contains("top"),
            "{body:?}"
        );
        assert!(src[d[1].body.0..d[1].body.1].contains("return 1"));
        assert_eq!(src[d[2].body.0..d[2].body.1].trim(), "pass");
        assert_eq!(d[1].indent, 4);
    }

    #[test]
    fn imports_read_every_form() {
        let src = "import os, a.b as c\nfrom . import views\nfrom ..x.y import (\n  P as Q,\n  R,\n)\nfrom m import *\n";
        let got: Vec<_> = (imports(&mask(src)).into_iter())
            .map(|i| (i.local, i.module, i.name, i.line))
            .collect();
        let s = |x: &str| x.to_string();
        assert_eq!(
            got,
            [
                (s("os"), s("os"), None, 1),
                (s("c"), s("a.b"), None, 1),
                (s("views"), s("."), Some(s("views")), 2),
                (s("Q"), s("..x.y"), Some(s("P")), 4),
                (s("R"), s("..x.y"), Some(s("R")), 5),
                (s("*"), s("m"), Some(s("*")), 7),
            ]
        );
    }

    #[test]
    fn modules_resolve_within_the_source_root() {
        let files: BTreeSet<String> = [
            "site/manage.py",
            "site/shop/__init__.py",
            "site/shop/urls.py",
            "site/orders/__init__.py",
            "site/orders/views.py",
            "site/orders/api/__init__.py",
            "site/orders/api/views.py",
        ]
        .map(String::from)
        .into();
        let from = "site/orders/api/views.py";
        assert_eq!(source_root(from, &files), "site");
        assert_eq!(module_name(from, &files), "orders.api.views");
        assert_eq!(
            module_name("site/orders/api/__init__.py", &files),
            "orders.api"
        );
        let r = |m: &str| resolve_module(from, m, &files);
        assert_eq!(r("orders.views").as_deref(), Some("site/orders/views.py"));
        assert_eq!(
            r("orders.api").as_deref(),
            Some("site/orders/api/__init__.py")
        );
        assert_eq!(r(".").as_deref(), Some("site/orders/api/__init__.py"));
        assert_eq!(r("..views").as_deref(), Some("site/orders/views.py"));
        assert_eq!(r("django.urls"), None);
        let imp = |module: &str, name: &str| Import {
            local: name.into(),
            module: module.into(),
            name: Some(name.into()),
            line: 1,
        };
        assert_eq!(
            resolve_import("site/shop/urls.py", &imp("orders", "views"), &files),
            Some(("site/orders/views.py".into(), None))
        );
        assert_eq!(
            resolve_import(from, &imp("..views", "home"), &files),
            Some(("site/orders/views.py".into(), Some("home".into())))
        );
        let plain = Import {
            local: "orders".into(),
            module: "orders".into(),
            name: None,
            line: 1,
        };
        assert_eq!(
            resolve_import(from, &plain, &files),
            Some(("site/orders/__init__.py".into(), None))
        );
    }
}
