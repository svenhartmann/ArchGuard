//! Text-level C# patterns of the view conventions: comments and strings
//! blanked, then state assignments, domain-event types and their creation.
use std::collections::BTreeSet;

/// `src` with comments and literal contents blanked, quotes kept, so
/// `new X("…")` still has its shape.
pub(super) use archguard_flow_common::lang_csharp::mask as blank;
pub(super) use archguard_flow_common::lang_text::line;

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
