//! What a TypeScript or JavaScript file declares and imports, for the Angular,
//! React, Vue and domain extractors (ADR 0021, 0022), and its blanked view.
use crate::lang_ts::{Parser, ident_at, is_name, skip_ws, trailing_ident, words};

/// `src` with comments and template-literal contents blanked, and string
/// contents too unless `keep_strings`; offsets and lines unchanged (the
/// domain extractor's view of a file, ADR 0022).
// shortcut: regular-expression literals are not recognized; a quote or
// bracket inside one can shift the blanking. Upgrade when a project needs it.
pub(crate) fn mask(src: &str, keep_strings: bool) -> String {
    use crate::lang_blank::{blank_regions, find, line_end};
    // The closing quote of the literal opened at `i`, with `\` escapes;
    // `one_line` stops at a line break.
    let closing = |b: &[u8], i: usize, one_line: bool| {
        let q = b[i];
        let mut j = i + 1;
        while j < b.len() && b[j] != q && !(one_line && b[j] == b'\n') {
            j += if b[j] == b'\\' { 2 } else { 1 };
        }
        j.min(b.len())
    };
    blank_regions(src, |b, i| {
        let next = b.get(i + 1).copied();
        match b[i] {
            b'/' if next == Some(b'/') => Some((i, line_end(b, i), line_end(b, i))),
            b'/' if next == Some(b'*') => {
                let end = find(b, i + 2, b"*/").map_or(b.len(), |p| p + 2);
                Some((i, end, end))
            }
            b'`' => {
                let end = closing(b, i, false);
                Some((i + 1, end, end + 1))
            }
            b'\'' | b'"' => {
                let end = closing(b, i, true);
                Some(if keep_strings {
                    (end, end, end + 1)
                } else {
                    (i + 1, end, end + 1)
                })
            }
            _ => None,
        }
    })
}

/// Offset of the `[` that `const|let|var name … = [` binds.
pub(crate) fn local_array(text: &str, name: &str) -> Option<usize> {
    let b = text.as_bytes();
    for kw in ["const", "let", "var"] {
        for pos in words(text, kw) {
            let j = skip_ws(b, pos + kw.len());
            if ident_at(text, j) != name {
                continue;
            }
            let k = j + name.len();
            let Some(eq) = text[k..].find(['=', ';']).map(|e| k + e) else {
                continue;
            };
            let after = skip_ws(b, eq + 1);
            if b[eq] == b'=' && b.get(after) == Some(&b'[') {
                return Some(after);
            }
        }
    }
    None
}

/// Whether `text` declares `name` as function, class or variable.
pub(crate) fn declares(text: &str, name: &str) -> bool {
    ["function", "class", "const", "let", "var"]
        .iter()
        .any(|kw| {
            words(text, kw)
                .into_iter()
                .any(|pos| ident_at(text, skip_ws(text.as_bytes(), pos + kw.len())) == name)
        })
}

/// `"name"` as a key in `package.json` text.
pub(crate) fn has_dep(text: &str, name: &str) -> bool {
    let key = format!("\"{name}\"");
    text.match_indices(&key)
        .any(|(i, _)| text[i + key.len()..].trim_start().starts_with(':'))
}

/// One `import … from '…'`: `imported` is `None` for the default import and
/// `Some("*")` for a namespace import.
pub(crate) struct Import {
    pub(crate) local: String,
    pub(crate) imported: Option<String>,
    pub(crate) module: String,
}

pub(crate) fn imports(text: &str) -> Vec<Import> {
    let mut out = Vec::new();
    for pos in words(text, "import") {
        if let Some((clause, module)) = import_at(&text[pos + "import".len()..]) {
            import_names(clause, &module, &mut out);
        }
    }
    out
}

/// The clause and module of the import whose text follows `import`.
pub(crate) fn import_at(rest: &str) -> Option<(&str, String)> {
    let from = *words(rest, "from").first()?;
    let clause = rest[..from].trim();
    if clause.contains(';') || clause.starts_with('(') || clause.starts_with("type ") {
        return None;
    }
    // The module string alone: imports without `;` run into the next line.
    let mut p = Parser {
        s: rest,
        i: skip_ws(rest.as_bytes(), from + "from".len()),
    };
    matches!(p.peek(), b'\'' | b'"').then(|| (clause, p.string()))
}

/// The names an import clause (`A, { b, c as d }`, `* as X`) binds.
fn import_names(clause: &str, module: &str, out: &mut Vec<Import>) {
    let (head, braces) = match (clause.find('{'), clause.rfind('}')) {
        (Some(open), Some(close)) if open < close => (&clause[..open], &clause[open + 1..close]),
        _ => (clause, ""),
    };
    let mut push = |local: &str, imported: Option<&str>| {
        if is_name(local) {
            out.push(Import {
                local: local.into(),
                imported: imported.map(str::to_string),
                module: module.into(),
            });
        }
    };
    for part in head.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        match part.strip_prefix("* as ") {
            Some(local) => push(local.trim(), Some("*")),
            None => push(part, None),
        }
    }
    for part in braces.split(',') {
        let part = part.trim().trim_start_matches("type ").trim();
        match part.split_once(" as ") {
            Some((name, local)) => push(local.trim(), Some(name.trim())),
            None => push(part, Some(part)),
        }
    }
}

/// What `… = hook()` binds: a name, or destructured `(key, local)` pairs.
pub(crate) enum Binding {
    Name(String),
    Destructure(Vec<(String, String)>),
}

pub(crate) fn bindings(text: &str, hook: &str) -> Vec<Binding> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    for pos in words(text, hook) {
        let open = skip_ws(b, pos + hook.len());
        if b.get(open) != Some(&b'(') || b.get(skip_ws(b, open + 1)) != Some(&b')') {
            continue;
        }
        let Some(lhs) = text[..pos].trim_end().strip_suffix('=') else {
            continue;
        };
        if lhs.ends_with(['=', '!', '<', '>']) {
            continue;
        }
        let lhs = lhs.trim_end();
        if let Some(body) = lhs.strip_suffix('}') {
            let Some(open) = body.rfind('{') else {
                continue;
            };
            let pairs = body[open + 1..]
                .split(',')
                .filter_map(|p| {
                    let (key, local) = p.split_once(':').unwrap_or((p, p));
                    let (key, local) = (key.trim(), local.trim());
                    (is_name(key) && is_name(local)).then(|| (key.into(), local.into()))
                })
                .collect();
            out.push(Binding::Destructure(pairs));
        } else if let Some(name) = trailing_ident(lhs) {
            out.push(Binding::Name(name));
        }
    }
    out
}
