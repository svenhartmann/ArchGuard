//! Markup reading shared by the Blazor (Razor) and XAML extractors (ADR
//! 0021): tags with their attributes, element blocks, comment blanking and
//! the text of an element.

/// One XAML tag; `attrs` is the text after the name.
pub(crate) struct Tag<'a> {
    pub(crate) at: usize,
    pub(crate) name: &'a str,
    pub(crate) attrs: &'a str,
    pub(crate) self_closing: bool,
    pub(crate) closing: bool,
}

/// Every tag in text order, closing tags included; `<?` and `<!` skipped.
pub(crate) fn all_tags(text: &str) -> Vec<Tag<'_>> {
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(i) = text[from..].find('<') {
        let at = from + i;
        let rest = &text[at + 1..];
        let Some(end) = tag_end(rest) else { break };
        from = at + 1 + end;
        if rest.starts_with(['?', '!']) {
            continue;
        }
        let closing = rest.starts_with('/');
        let body = &rest[usize::from(closing)..end];
        let len = body
            .find(|c: char| c.is_whitespace() || c == '/')
            .unwrap_or(body.len());
        if len == 0 {
            continue;
        }
        found.push(Tag {
            at,
            name: &body[..len],
            attrs: &body[len..],
            self_closing: body.trim_end().ends_with('/'),
            closing,
        });
    }
    found
}

/// Opening and self-closing tags.
pub(crate) fn tags(text: &str) -> impl Iterator<Item = Tag<'_>> {
    all_tags(text).into_iter().filter(|t| !t.closing)
}

/// (open offset, close offset, opening tag) of every non-self-closing
/// element whose local name is one of `names`.
pub(crate) fn blocks<'a>(text: &'a str, names: &[&str]) -> Vec<(usize, usize, Tag<'a>)> {
    let mut stack: Vec<Tag> = Vec::new();
    let mut found = Vec::new();
    for tag in all_tags(text) {
        if !names.contains(&tag.name.rsplit(':').next().unwrap_or(tag.name)) {
            continue;
        }
        if tag.closing {
            if let Some(open) = stack.pop() {
                found.push((open.at, tag.at, open));
            }
        } else if !tag.self_closing {
            stack.push(tag);
        }
    }
    found
}

/// Offset of the `>` closing a tag, skipping quoted attribute values.
pub(crate) fn tag_end(s: &str) -> Option<usize> {
    let mut quote = None;
    for (i, c) in s.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), _) if q == c => quote = None,
            (None, '>') => return Some(i),
            _ => {}
        }
    }
    None
}

/// `name="value"` pairs of a tag's attribute text.
pub(crate) fn attributes(attrs: &str) -> Vec<(&str, &str)> {
    let mut found = Vec::new();
    let mut rest = attrs;
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_whitespace() || c == '/');
        let Some(eq) = rest.find('=') else { break };
        let after = rest[eq + 1..].trim_start();
        let Some(q) = after.chars().next().filter(|c| matches!(c, '"' | '\'')) else {
            break;
        };
        let Some(end) = after[1..].find(q) else { break };
        found.push((rest[..eq].trim(), &after[1..1 + end]));
        rest = &after[end + 2..];
    }
    found
}

pub(crate) fn attr<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    attributes(attrs)
        .into_iter()
        .find(|(n, _)| *n == name)
        .map(|(_, v)| v)
}

/// Replaces `open`...`close` regions (`<!--`…`-->`, `@*`…`*@`) with spaces,
/// keeping line breaks and byte offsets; an unclosed region runs to the end.
pub(crate) fn blank(text: &str, open: &str, close: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(open) {
        out.push_str(&rest[..start]);
        let end = rest[start..]
            .find(close)
            .map_or(rest.len(), |e| start + e + close.len());
        out.extend(
            rest[start..end].chars().flat_map(|c| {
                std::iter::repeat_n(if c == '\n' { '\n' } else { ' ' }, c.len_utf8())
            }),
        );
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// The text outside tags (`<…>`).
pub(crate) fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in s.chars() {
        match c {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}
