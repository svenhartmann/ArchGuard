//! Small text helpers of the TypeScript readers (`domain_typescript` and its
//! schema reader), on text whose comments are blanked and strings kept.

use crate::lang_text::{matching, next_nonws};

/// Whether `shown` has an `import … from 'm'` with `module(m)` that names one
/// of `names` (any name when empty).
pub(crate) fn imports(shown: &str, module: impl Fn(&str) -> bool, names: &[&str]) -> bool {
    let mut from = 0;
    while let Some(i) = shown[from..].find("from") {
        let at = from + i;
        from = at + 4;
        let rest = shown[from..].trim_start();
        let Some(q) = rest.chars().next().filter(|c| matches!(c, '\'' | '"')) else {
            continue;
        };
        let Some(end) = rest[1..].find(q) else {
            continue;
        };
        if !module(&rest[1..1 + end]) {
            continue;
        }
        let Some(start) = shown[..at].rfind("import") else {
            continue;
        };
        let clause = &shown[start + 6..at];
        if names.is_empty()
            || (clause.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$')))
                .any(|w| names.contains(&w))
        {
            return true;
        }
    }
    false
}

/// `'x'`, `"x"` or `` `x` `` (in `shown` text) → `x`.
pub(crate) fn string_literal(x: &str) -> Option<String> {
    let x = x.trim();
    let q = x.chars().next().filter(|c| matches!(c, '\'' | '"' | '`'))?;
    let inner = x.strip_prefix(q)?.strip_suffix(q)?;
    (!inner.contains(q)).then(|| inner.to_string())
}

/// Past a `<…>` type-argument list at `i`, if one follows.
pub(crate) fn skip_generics(b: &[u8], i: usize) -> usize {
    match next_nonws(b, i).filter(|&k| b[k] == b'<') {
        Some(k) => matching(b, k).map_or(i, |e| e + 1),
        None => i,
    }
}
