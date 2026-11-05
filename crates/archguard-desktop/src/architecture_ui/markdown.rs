//! Block parser for the Markdown the arc42 exporter writes; a port of the
//! web viewer's `arc42Markdown` and `arc42Inline` (viewer_architecture.js).
//! Inline markup stays Markdown source: the window renders it with Slint's
//! `StyledText`; [`plain`] gives the text without markup.

/// Column alignment of a table cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// Level 2..=6 (the web renders `#` as `h2` too).
    Heading {
        level: usize,
        text: String,
    },
    Paragraph(String),
    /// One list entry; `depth` 0 is the outer list.
    Item {
        depth: usize,
        marker: String,
        text: String,
    },
    Table {
        columns: Vec<String>,
        align: Vec<Align>,
        rows: Vec<Vec<String>>,
    },
    Code {
        lang: String,
        text: String,
    },
    Quote(Vec<Block>),
    Rule,
}

/// Parse a Markdown document into blocks.
pub fn parse(markdown: &str) -> Vec<Block> {
    let text = strip_comments(&markdown.replace("\r\n", "\n").replace('\r', "\n"));
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        i = parse_block(&lines, i, &mut out);
    }
    out
}

/// Parse the block that starts at line `i`; returns the next line.
fn parse_block(lines: &[&str], i: usize, out: &mut Vec<Block>) -> usize {
    let line = lines[i];
    if line.trim().is_empty() {
        return i + 1;
    }
    if let Some(lang) = fence(line) {
        return code(lines, i, lang, out);
    }
    if let Some((level, text)) = heading(line) {
        out.push(Block::Heading {
            level: level.max(2),
            text: text.to_string(),
        });
        return i + 1;
    }
    if lines.get(i + 1).is_some_and(|l| divider(l)) {
        return table(lines, i, out);
    }
    if list_item(line).is_some() {
        return list(lines, i, out);
    }
    if quote_line(line).is_some() {
        let mut quote = Vec::new();
        let mut j = i;
        while let Some(rest) = lines.get(j).and_then(|l| quote_line(l)) {
            quote.push(rest);
            j += 1;
        }
        out.push(Block::Quote(parse(&quote.join("\n"))));
        return j;
    }
    if rule(line) {
        out.push(Block::Rule);
        return i + 1;
    }
    let mut paragraph = vec![line];
    let mut j = i + 1;
    while j < lines.len() && !starts_block(lines, j) {
        paragraph.push(lines[j]);
        j += 1;
    }
    out.push(Block::Paragraph(paragraph.join("\n")));
    j
}

fn code(lines: &[&str], i: usize, lang: &str, out: &mut Vec<Block>) -> usize {
    let mut j = i + 1;
    let mut body = Vec::new();
    while j < lines.len()
        && !(lines[j].trim_start().starts_with("```") && fence(lines[j]) == Some(""))
    {
        body.push(lines[j]);
        j += 1;
    }
    out.push(Block::Code {
        lang: lang.to_string(),
        text: body.join("\n"),
    });
    j + 1
}

fn table(lines: &[&str], i: usize, out: &mut Vec<Block>) -> usize {
    let columns = cells(lines[i]);
    let align = cells(lines[i + 1])
        .iter()
        .map(|a| match (a.starts_with(':'), a.ends_with(':')) {
            (true, true) => Align::Center,
            (false, true) => Align::Right,
            _ => Align::Left,
        })
        .collect::<Vec<_>>();
    let mut rows = Vec::new();
    let mut j = i + 2;
    while j < lines.len() && !lines[j].trim().is_empty() && lines[j].trim().contains('|') {
        let row = cells(lines[j]);
        rows.push(
            (0..columns.len())
                .map(|c| row.get(c).cloned().unwrap_or_default())
                .collect(),
        );
        j += 1;
    }
    let align = (0..columns.len())
        .map(|c| align.get(c).copied().unwrap_or(Align::Left))
        .collect();
    out.push(Block::Table {
        columns,
        align,
        rows,
    });
    j
}

/// Nested lists flatten to items with a depth, as the web's list stack.
fn list(lines: &[&str], i: usize, out: &mut Vec<Block>) -> usize {
    // (indent, ordered, next number)
    let mut stack: Vec<(usize, bool, u64)> = Vec::new();
    let mut j = i;
    while let Some((indent, marker, text)) = lines.get(j).and_then(|l| list_item(l)) {
        j += 1;
        let ordered = marker.starts_with(|c: char| c.is_ascii_digit());
        while stack
            .last()
            .is_some_and(|&(d, o, _)| indent < d || (indent == d && ordered != o))
        {
            stack.pop();
        }
        if stack.last().is_none_or(|&(d, _, _)| indent > d) {
            let start = marker
                .trim_end_matches(['.', ')'])
                .parse::<u64>()
                .unwrap_or(1);
            stack.push((indent, ordered, start));
        }
        let depth = stack.len() - 1;
        let top = stack.last_mut().expect("pushed above");
        let marker = if top.1 {
            top.2 += 1;
            format!("{}.", top.2 - 1)
        } else {
            "•".to_string()
        };
        out.push(Block::Item {
            depth,
            marker,
            text: text.to_string(),
        });
    }
    j
}

fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        match rest[start + 4..].find("-->") {
            Some(end) => rest = &rest[start + 4 + end + 3..],
            None => {
                rest = &rest[start..];
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// `^\s*```(\w*)\s*$`
fn fence(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix("```")?;
    let lang = rest.trim_end();
    lang.chars()
        .all(|c| c.is_alphanumeric() || c == '_')
        .then_some(lang)
}

/// `^(#{1,6})\s+(.+)$`
fn heading(line: &str) -> Option<(usize, &str)> {
    let level = line.chars().take_while(|&c| c == '#').count();
    let rest = &line[level..];
    if !(1..=6).contains(&level) || !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let text = rest.trim_start();
    (!text.is_empty()).then_some((level, text))
}

/// `^(\s*)([-+*]|\d+[.)])\s+(.+)$`
fn list_item(line: &str) -> Option<(usize, &str, &str)> {
    let body = line.trim_start();
    let indent = line.len() - body.len();
    let marker_len = if body.starts_with(['-', '+', '*']) {
        1
    } else {
        let digits = body.chars().take_while(char::is_ascii_digit).count();
        if digits == 0 || !body[digits..].starts_with(['.', ')']) {
            return None;
        }
        digits + 1
    };
    let rest = &body[marker_len..];
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let text = rest.trim_start();
    (!text.is_empty()).then_some((indent, &body[..marker_len], text))
}

/// `^\s*> ?(.*)`
fn quote_line(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix('>')?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

/// `^\s*([-*_])\1{2,}\s*$`
fn rule(line: &str) -> bool {
    let t = line.trim();
    let Some(c) = t.chars().next() else {
        return false;
    };
    matches!(c, '-' | '*' | '_') && t.len() >= 3 && t.chars().all(|x| x == c)
}

/// `^\|?\s*:?-{3,}:?\s*(\|\s*:?-{3,}:?\s*)+\|?$`: at least two columns.
fn divider(line: &str) -> bool {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    let parts: Vec<&str> = t.split('|').collect();
    parts.len() >= 2
        && parts.iter().all(|p| {
            let p = p.trim();
            let p = p.strip_prefix(':').unwrap_or(p);
            let p = p.strip_suffix(':').unwrap_or(p);
            p.len() >= 3 && p.chars().all(|c| c == '-')
        })
}

fn starts_block(lines: &[&str], i: usize) -> bool {
    let line = lines[i];
    line.trim().is_empty()
        || heading(line).is_some()
        || fence(line).is_some()
        || quote_line(line).is_some()
        || rule(line)
        || list_item(line).is_some()
        || lines.get(i + 1).is_some_and(|l| divider(l))
}

/// Table cells; `\|` does not split.
fn cells(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    let mut out = Vec::new();
    let mut cell = String::new();
    let mut escaped = false;
    for ch in t.chars() {
        if ch == '|' && !escaped {
            out.push(cell.trim().to_string());
            cell.clear();
        } else {
            cell.push(ch);
        }
        escaped = ch == '\\' && !escaped;
    }
    out.push(cell.trim().to_string());
    out
}

/// The text of inline Markdown without markup (escapes, code, emphasis,
/// links and the five entities resolved).
pub fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        i = plain_token(&chars, i, &mut out);
    }
    out
}

fn plain_token(c: &[char], i: usize, out: &mut String) -> usize {
    let close = |from: usize, end: &str| -> Option<usize> {
        let end: Vec<char> = end.chars().collect();
        (from..c.len()).find(|&k| c[k..].starts_with(&end))
    };
    match c[i] {
        '\\' if c.get(i + 1).is_some_and(|n| n.is_ascii_punctuation()) => {
            out.push(c[i + 1]);
            return i + 2;
        }
        '`' => {
            if let Some(end) = close(i + 1, "`").filter(|&e| e > i + 1) {
                out.extend(&c[i + 1..end]);
                return end + 1;
            }
        }
        '*' if c.get(i + 1) == Some(&'*') => {
            if let Some(end) = close(i + 2, "**").filter(|&e| e > i + 2) {
                out.push_str(&plain(&c[i + 2..end].iter().collect::<String>()));
                return end + 2;
            }
        }
        '*' => {
            if let Some(end) = close(i + 1, "*").filter(|&e| e > i + 1) {
                out.push_str(&plain(&c[i + 1..end].iter().collect::<String>()));
                return end + 1;
            }
        }
        '[' => {
            if let Some(n) = link(c, i, out) {
                return n;
            }
        }
        '&' => {
            for (name, ch) in [
                ("amp;", '&'),
                ("lt;", '<'),
                ("gt;", '>'),
                ("quot;", '"'),
                ("apos;", '\''),
            ] {
                if c[i + 1..].starts_with(&name.chars().collect::<Vec<_>>()) {
                    out.push(ch);
                    return i + 1 + name.len();
                }
            }
        }
        _ => {}
    }
    out.push(c[i]);
    i + 1
}

/// `[label](target)` → label.
fn link(c: &[char], i: usize, out: &mut String) -> Option<usize> {
    let mid = (i + 1..c.len()).find(|&k| c[k] == ']' || c[k] == '\n')?;
    if c[mid] != ']' || mid == i + 1 || c.get(mid + 1) != Some(&'(') {
        return None;
    }
    let end = (mid + 2..c.len()).find(|&k| c[k] == ')' || c[k].is_whitespace())?;
    if c[end] != ')' || end == mid + 2 {
        return None;
    }
    out.push_str(&plain(&c[i + 1..mid].iter().collect::<String>()));
    Some(end + 1)
}

/// Inline Markdown for `StyledText`: block syntax at a line start is
/// escaped, so a cell "1. x" or "- x" stays text, not a list.
pub fn inline_source(text: &str) -> String {
    text.split('\n')
        .map(|line| {
            let body = line.trim_start();
            let starts_block = heading(body).is_some()
                || list_item(body).is_some()
                || body.starts_with('>')
                || rule(body);
            if !starts_block {
                return body.to_string();
            }
            let digits = body.chars().take_while(char::is_ascii_digit).count();
            if digits > 0 {
                format!("{}\\{}", &body[..digits], &body[digits..])
            } else {
                format!("\\{body}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn para(s: &str) -> Block {
        Block::Paragraph(s.into())
    }

    #[test]
    fn headings_paragraphs_and_comments() {
        let md = "# Top\n\n<!-- hidden\nstill -->\n### Sub\nline one\nline two\n\n#### Deep";
        assert_eq!(
            parse(md),
            vec![
                Block::Heading {
                    level: 2,
                    text: "Top".into()
                },
                Block::Heading {
                    level: 3,
                    text: "Sub".into()
                },
                para("line one\nline two"),
                Block::Heading {
                    level: 4,
                    text: "Deep".into()
                },
            ]
        );
    }

    #[test]
    fn tables_with_alignment_and_escaped_pipes() {
        let md =
            "| Paket | Knoten | Mitte |\n| --- | ---: | :---: |\n| a\\|b | 72 | x |\n| c |\n\nnext";
        assert_eq!(
            parse(md),
            vec![
                Block::Table {
                    columns: vec!["Paket".into(), "Knoten".into(), "Mitte".into()],
                    align: vec![Align::Left, Align::Right, Align::Center],
                    rows: vec![
                        vec!["a\\|b".into(), "72".into(), "x".into()],
                        vec!["c".into(), String::new(), String::new()],
                    ],
                },
                para("next"),
            ]
        );
    }

    #[test]
    fn nested_and_ordered_lists() {
        let md = "- a\n  - b\n- c\n\n3. x\n4. y";
        let items: Vec<(usize, String, String)> = parse(md)
            .into_iter()
            .map(|b| match b {
                Block::Item {
                    depth,
                    marker,
                    text,
                } => (depth, marker, text),
                other => panic!("unexpected {other:?}"),
            })
            .collect();
        let want = [
            (0, "•", "a"),
            (1, "•", "b"),
            (0, "•", "c"),
            (0, "3.", "x"),
            (0, "4.", "y"),
        ];
        assert_eq!(
            items,
            want.map(|(d, m, t)| (d, m.to_string(), t.to_string()))
        );
    }

    #[test]
    fn code_quote_and_rule() {
        let md = "```mermaid\nflowchart LR\n  a --> b\n```\n> quoted\n> more\n\n---\ntext";
        assert_eq!(
            parse(md),
            vec![
                Block::Code {
                    lang: "mermaid".into(),
                    text: "flowchart LR\n  a --> b".into()
                },
                Block::Quote(vec![para("quoted\nmore")]),
                Block::Rule,
                para("text"),
            ]
        );
    }

    #[test]
    fn a_paragraph_ends_before_a_list_or_table() {
        let md = "intro\n- item\nhead | b\n--- | ---\n1 | 2";
        let blocks = parse(md);
        assert_eq!(blocks[0], para("intro"));
        assert!(matches!(blocks[1], Block::Item { .. }));
        assert!(matches!(blocks[2], Block::Table { .. }));
    }

    #[test]
    fn plain_text_drops_inline_markup() {
        assert_eq!(
            plain(
                "Quelle: `architecture.goals` **fett** *kursiv* [arc42](https://arc42.org/) a\\_b &amp; c"
            ),
            "Quelle: architecture.goals fett kursiv arc42 a_b & c"
        );
        assert_eq!(plain("2 * 3 and `"), "2 * 3 and `");
    }

    #[test]
    fn inline_source_escapes_block_starts() {
        assert_eq!(inline_source("1. Ziel"), "1\\. Ziel");
        assert_eq!(inline_source("- dash"), "\\- dash");
        assert_eq!(inline_source("# x"), "\\# x");
        assert_eq!(inline_source("**bold** text"), "**bold** text");
    }
}
