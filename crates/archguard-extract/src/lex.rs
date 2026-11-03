//! Lexical shielding for lightweight import scanners. This is not a compiler parser.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Flavor {
    C,
    Rust,
    Js,
    Python,
    Php,
}
#[derive(Debug)]
pub(crate) struct Token {
    pub text: String,
    pub literal: bool,
    pub line: u32,
    pub end_line: u32,
    pub start: usize,
    pub end: usize,
}
pub(crate) fn tokens(source: &str, flavor: Flavor) -> Vec<Token> {
    let chars: Vec<_> = source.char_indices().collect();
    let mut out: Vec<Token> = Vec::new();
    let mut i = 0;
    let mut line = 1;
    let mut control_parens = Vec::new();
    let mut after_control = false;
    let mut php = flavor != Flavor::Php || !source.contains("<?");
    while i < chars.len() {
        let start = i;
        let start_line = line;
        let c = chars[i].1;
        let at = |n: usize| chars.get(n).map(|p| p.1);
        if flavor == Flavor::Php {
            let tail = &source[chars[i].0..];
            if tail.starts_with("<?php") {
                php = true;
                i += 5;
                continue;
            }
            if tail.starts_with("<?") {
                php = true;
                i += 2;
                continue;
            }
            if tail.starts_with("?>") {
                php = false;
                i += 2;
                continue;
            }
            if !php {
                if c == '\n' {
                    line += 1;
                }
                i += 1;
                continue;
            }
        }
        if c == '\n' {
            out.push(Token {
                text: "\n".into(),
                literal: false,
                line,
                end_line: line,
                start: chars[i].0,
                end: chars[i].0 + 1,
            });
            line += 1;
            i += 1;
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        let hash =
            matches!(flavor, Flavor::Python | Flavor::Php) && c == '#' && at(i + 1) != Some('[');
        if hash || (flavor != Flavor::Python && c == '/' && at(i + 1) == Some('/')) {
            while i < chars.len() && at(i) != Some('\n') {
                i += 1;
            }
            continue;
        }
        if flavor != Flavor::Python && c == '/' && at(i + 1) == Some('*') {
            i += 2;
            let mut depth = 1usize;
            while i < chars.len() {
                if flavor == Flavor::Rust && at(i) == Some('/') && at(i + 1) == Some('*') {
                    depth += 1;
                    i += 2;
                    continue;
                }
                if at(i) == Some('*') && at(i + 1) == Some('/') {
                    i += 2;
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    continue;
                }
                if at(i) == Some('\n') {
                    line += 1;
                }
                i += 1;
            }
            continue;
        }
        let mut literal = false;
        let raw_start = if flavor == Flavor::Rust {
            let mut q = i;
            if matches!(at(q), Some('b' | 'c')) {
                q += 1;
            }
            if at(q) == Some('r') {
                q += 1;
                let hash_start = q;
                while at(q) == Some('#') {
                    q += 1;
                }
                (at(q) == Some('"')).then_some((q, q - hash_start))
            } else {
                None
            }
        } else {
            None
        };

        if let Some((quote, hashes)) = raw_start {
            literal = true;
            i = quote + 1;
            while i < chars.len() {
                if at(i) == Some('"') && (0..hashes).all(|n| at(i + 1 + n) == Some('#')) {
                    i += 1 + hashes;
                    break;
                }
                if at(i) == Some('\n') {
                    line += 1;
                }
                i += 1;
            }
        } else if flavor == Flavor::Php
            && c == '<'
            && at(i + 1) == Some('<')
            && at(i + 2) == Some('<')
        {
            literal = true;
            let tail = &source[chars[i].0 + 3..];
            let label = tail
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .trim_matches(['\'', '"']);
            let mut consumed = 0;
            for (index, part) in tail.split_inclusive('\n').enumerate() {
                consumed += part.len();
                if index > 0
                    && part.trim_start().strip_prefix(label).is_some_and(|s| {
                        s.trim().is_empty() || s.trim_start().starts_with([';', ',', ')'])
                    })
                {
                    break;
                }
            }
            let end = chars[i].0 + 3 + consumed;
            while i < chars.len() && chars[i].0 < end {
                if at(i) == Some('\n') {
                    line += 1;
                }
                i += 1;
            }
        } else if matches!(c, '\'' | '"' | '`')
            && !(flavor == Flavor::Rust
                && c == '\''
                && at(i + 1).is_some_and(|c| c.is_alphabetic() || c == '_')
                && at(i + 2) != Some('\''))
        {
            literal = true;
            let run = chars[i..].iter().take_while(|p| p.1 == c).count();
            let width = if c != '`' && run >= 3 {
                if flavor == Flavor::C { run } else { 3 }
            } else {
                1
            };
            i += width;
            while i < chars.len() {
                if at(i) == Some('\\') {
                    i += 1;
                    if at(i) == Some('\n') {
                        line += 1;
                    }
                    i = (i + 1).min(chars.len());
                    continue;
                }
                if at(i) == Some('\n') {
                    line += 1;
                }
                if chars[i..].iter().take(width).filter(|p| p.1 == c).count() == width {
                    i += width;
                    break;
                }
                i += 1;
            }
        } else if flavor == Flavor::Js
            && c == '/'
            && (after_control
                || out.iter().rev().find(|t| t.text != "\n").is_none_or(|t| {
                    matches!(
                        t.text.as_str(),
                        "=" | "(" | "[" | ":" | "," | ";" | "!" | "return" | ">" | "{"
                    )
                }))
        {
            literal = true;
            i += 1;
            let mut class = false;
            while i < chars.len() && at(i) != Some('\n') {
                match at(i) {
                    Some('\\') => {
                        i = (i + 2).min(chars.len());
                        continue;
                    }
                    Some('[') => class = true,
                    Some(']') => class = false,
                    Some('/') if !class => {
                        i += 1;
                        break;
                    }
                    _ => {}
                }
                i += 1;
            }
        } else if c.is_alphanumeric() || c == '_' || c == '$' {
            i += 1;
            while i < chars.len()
                && (chars[i].1.is_alphanumeric() || matches!(chars[i].1, '_' | '$'))
            {
                i += 1;
            }
        } else {
            i += 1;
        }
        let end = chars.get(i).map_or(source.len(), |p| p.0);
        let text = &source[chars[start].0..end];
        if flavor == Flavor::Js {
            if !literal && text == "(" {
                control_parens.push(out.iter().rev().find(|t| t.text != "\n").is_some_and(|t| {
                    matches!(
                        t.text.as_str(),
                        "if" | "while" | "for" | "with" | "catch" | "switch"
                    )
                }));
                after_control = false;
            } else if !literal && text == ")" {
                after_control = control_parens.pop().unwrap_or(false);
            } else {
                after_control = false;
            }
        }

        out.push(Token {
            text: text.into(),
            start: chars[start].0,
            end,
            literal,
            line: start_line,
            end_line: line,
        });
        if flavor == Flavor::Php && text.starts_with("<<<") && text.trim_end().ends_with(';') {
            out.push(Token {
                text: ";".into(),
                literal: false,
                line: line.saturating_sub(1),
                end_line: line.saturating_sub(1),
                start: end,
                end,
            });
        }
    }
    out
}

pub(crate) struct Statement {
    pub code: String,
    pub line: u32,
    pub raw: String,
    pub exact: String,
}
/// Split statement boundaries outside literals and Python continuations. Keep PHP group-use braces.
pub(crate) fn statements(source: &str, flavor: Flavor) -> Vec<Statement> {
    let ts = tokens(source, flavor);
    let lines: Vec<_> = source.lines().collect();
    let mut out = Vec::new();
    let mut current: Vec<&Token> = Vec::new();
    let mut depth = 0usize;
    let mut group = 0usize;
    for token in &ts {
        if !token.literal
            && matches!(token.text.as_str(), "using" | "namespace")
            && current
                .first()
                .is_some_and(|t| t.text == "namespace" && t.line < token.line)
            && depth == 0
        {
            current.clear();
        }
        if token.text == "\n" {
            if flavor == Flavor::Python
                && depth == 0
                && !current.last().is_some_and(|t| t.text == "\\")
            {
                flush(&mut current, &lines, source, &mut out);
            } else if flavor == Flavor::Python && current.last().is_some_and(|t| t.text == "\\") {
                current.pop();
            }
            continue;
        }
        if !token.literal {
            if matches!(token.text.as_str(), "(" | "[") {
                depth += 1;
            }
            if matches!(token.text.as_str(), ")" | "]") {
                depth = depth.saturating_sub(1);
            }
            if flavor == Flavor::Php
                && token.text == "{"
                && current.first().is_some_and(|t| t.text == "use")
            {
                group += 1;
            } else if flavor == Flavor::Php && token.text == "}" && group > 0 {
                group -= 1;
                current.push(token);
                continue;
            }
        }
        current.push(token);
        if !token.literal
            && depth == 0
            && group == 0
            && (token.text == ";"
                || (flavor != Flavor::Python && matches!(token.text.as_str(), "{" | "}")))
        {
            flush(&mut current, &lines, source, &mut out);
        }
    }
    if matches!(flavor, Flavor::Python | Flavor::Php) {
        flush(&mut current, &lines, source, &mut out);
    }
    out
}
fn flush(current: &mut Vec<&Token>, lines: &[&str], source: &str, out: &mut Vec<Statement>) {
    if current.is_empty() {
        return;
    }
    let start = current[0].line;
    let end = current.last().unwrap().end_line;
    let mut code = String::new();
    let mut previous = "";
    for token in current.iter() {
        let text = token.text.as_str();
        let adjacent = matches!(
            text,
            "." | "\\" | ":" | ";" | "," | "(" | ")" | "{" | "}" | "[" | "]"
        ) || matches!(previous, "." | "\\" | ":" | "(" | "[" | "@");
        if !code.is_empty()
            && (!adjacent
                || matches!(
                    previous,
                    "from"
                        | "import"
                        | "use"
                        | "require"
                        | "require_once"
                        | "include"
                        | "include_once"
                )
                || text == "import")
        {
            code.push(' ');
        }
        code.push_str(text);
        previous = text;
    }
    out.push(Statement {
        exact: source[current[0].start..current.last().unwrap().end].to_owned(),
        code,
        line: start,
        raw: lines[(start - 1) as usize..(end as usize).min(lines.len())].join("\n"),
    });
    current.clear();
}

/// Preserve source positions while hiding comments and literal contents from visibility scans.
pub(crate) fn code_only(source: &str, flavor: Flavor) -> String {
    let mut bytes: Vec<_> = source
        .bytes()
        .map(|b| if b == b'\n' || b == b'\r' { b } else { b' ' })
        .collect();
    for token in tokens(source, flavor).iter().filter(|t| !t.literal) {
        bytes[token.start..token.end].copy_from_slice(&source.as_bytes()[token.start..token.end]);
    }
    String::from_utf8(bytes).expect("tokens retain complete UTF-8 codepoints")
}
