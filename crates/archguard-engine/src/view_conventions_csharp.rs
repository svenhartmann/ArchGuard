//! Text-level C# patterns of the view conventions: comments and strings
//! blanked, then state assignments, domain-event types and their creation.
use std::collections::BTreeSet;

/// The raise function of the C# domain extractor (`domain_csharp.rs`).
pub(super) const RAISE_FUNCTION: &str = "AddDomainEvent";

/// `(offset of the field, enum value)` of each `.field = EnumType.Value`.
pub(super) fn member_assignments<'a>(
    s: &'a str,
    field: &str,
    enum_type: &str,
) -> Vec<(usize, &'a str)> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    for (pos, _) in s.match_indices(field) {
        let end = pos + field.len();
        if pos == 0 || b[pos - 1] != b'.' || b.get(end).is_some_and(|c| is_id(*c)) {
            continue;
        }
        let rest = s[end..].trim_start();
        let Some(rest) = rest
            .strip_prefix('=')
            .filter(|r| !r.starts_with(['=', '>']))
        else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix(enum_type) else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix('.') else {
            continue;
        };
        let value = ident(rest.trim_start());
        if !value.is_empty() {
            out.push((pos, value));
        }
    }
    out
}

/// Names of classes, records and structs named `*DomainEvent` or listing
/// `INotification` as a base.
pub(super) fn event_types(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    for kw in ["class ", "record ", "struct "] {
        for (pos, _) in s.match_indices(kw) {
            if pos > 0 && is_id(s.as_bytes()[pos - 1]) {
                continue;
            }
            let rest = s[pos + kw.len()..].trim_start();
            let name = ident(rest);
            let header = &rest[..rest.find(['{', ';']).unwrap_or(rest.len())];
            let notification = header
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .any(|w| w == "INotification");
            if !name.is_empty() && (name.ends_with("DomainEvent") || notification) {
                out.push(name);
            }
        }
    }
    out
}

/// `(offset of new, type name)` of each `new T(` or `new T {`; a qualified
/// name counts by its last segment.
pub(super) fn constructions(s: &str) -> Vec<(usize, &str)> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    for (pos, _) in s.match_indices("new") {
        if (pos > 0 && is_id(b[pos - 1]))
            || !b.get(pos + 3).is_some_and(|c| c.is_ascii_whitespace())
        {
            continue;
        }
        let rest = s[pos + 3..].trim_start();
        let qualified_len = rest.bytes().take_while(|c| is_id(*c) || *c == b'.').count();
        let name = rest[..qualified_len].rsplit('.').next().unwrap_or("");
        if !name.is_empty() && rest[qualified_len..].trim_start().starts_with(['(', '{']) {
            out.push((pos, name));
        }
    }
    out
}

/// Trimmed text up to the first `)` of every `AddDomainEvent(` call.
pub(super) fn raise_arguments(s: &str) -> BTreeSet<&str> {
    let call = format!("{RAISE_FUNCTION}(");
    s.match_indices(&call)
        .filter_map(|(pos, _)| {
            let rest = &s[pos + call.len()..];
            rest.find(')').map(|end| rest[..end].trim())
        })
        .collect()
}

/// `new` at `pos` is a raise argument, or assigned to a variable that is one.
pub(super) fn passed_to_raise(s: &str, pos: usize, raised: &BTreeSet<&str>) -> bool {
    let before = s[..pos].trim_end();
    if let Some(call) = before.strip_suffix('(') {
        return call.trim_end().ends_with(RAISE_FUNCTION);
    }
    let Some(target) = before.strip_suffix('=') else {
        return false;
    };
    if target.ends_with(['=', '!', '<', '>']) {
        return false;
    }
    let target = target.trim_end();
    let start = target.len() - target.bytes().rev().take_while(|c| is_id(*c)).count();
    raised.contains(&target[start..])
}

fn is_id(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

fn ident(s: &str) -> &str {
    &s[..s.bytes().take_while(|c| is_id(*c)).count()]
}

pub(super) fn line(s: &str, pos: usize) -> u32 {
    1 + s[..pos].matches('\n').count() as u32
}

/// `src` with comments and the contents of string and char literals replaced
/// by spaces (newlines kept), so offsets and lines stay those of the source.
/// shortcut: raw (`"""`) and interpolated strings are blanked like plain ones,
/// expressions inside `{…}` included; good enough to hide text, not to read it.
pub(super) fn blank(src: &str) -> String {
    let mut b = src.as_bytes().to_vec();
    let mut i = 0;
    let wipe = |b: &mut [u8], from: usize, to: usize| {
        // An unterminated string ends before its own content starts.
        let to = to.min(b.len()).max(from);
        b[from..to]
            .iter_mut()
            .filter(|c| **c != b'\n')
            .for_each(|c| *c = b' ');
    };
    while i < b.len() {
        let end = match (b[i], b.get(i + 1).copied()) {
            (b'/', Some(b'/')) => b[i..]
                .iter()
                .position(|c| *c == b'\n')
                .map_or(b.len(), |n| i + n),
            (b'/', Some(b'*')) => find(&b, i + 2, b"*/").map_or(b.len(), |n| n + 2),
            (b'"', _) => {
                let verbatim = i > 0 && (b[i - 1] == b'@' || (i > 1 && b[i - 2] == b'@'));
                string_end(&b, i + 1, verbatim)
            }
            (b'\'', _) => {
                let close = if b.get(i + 1) == Some(&b'\\') {
                    i + 3
                } else {
                    i + 2
                };
                b.get(close..)
                    .unwrap_or_default()
                    .iter()
                    .take(8)
                    .position(|c| *c == b'\'')
                    .map_or(i + 1, |n| close + n + 1)
            }
            _ => {
                i += 1;
                continue;
            }
        };
        // Keep the quotes so `new X("…")` still has its shape.
        let quoted = b[i] == b'"';
        wipe(
            &mut b,
            if quoted { i + 1 } else { i },
            if quoted { end.saturating_sub(1) } else { end },
        );
        i = end.max(i + 1);
    }
    String::from_utf8_lossy(&b).into_owned()
}

fn find(b: &[u8], from: usize, pat: &[u8]) -> Option<usize> {
    b.get(from..)?
        .windows(pat.len())
        .position(|w| w == pat)
        .map(|n| from + n)
}

/// Offset just past the closing quote of a string whose content starts at `i`.
fn string_end(b: &[u8], mut i: usize, verbatim: bool) -> usize {
    while i < b.len() {
        match b[i] {
            b'\\' if !verbatim => i += 2,
            b'"' if verbatim && b.get(i + 1) == Some(&b'"') => i += 2,
            b'"' => return i + 1,
            b'\n' if !verbatim => return i,
            _ => i += 1,
        }
    }
    b.len()
}
