//! Markdown → AsciiDoc (ADR 0013) for the arc42 document: headings,
//! paragraphs, emphasis, links, lists, tables, code and HTML comments.
//! Mermaid and PlantUML blocks become Asciidoctor Diagram blocks.
//! Plain text that could trigger AsciiDoc markup is passed through
//! literally, so evidence such as `SHIPPING_API` or `{id}` stays as written.

use pulldown_cmark::{Alignment, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};

/// The AsciiDoc of the CommonMark+tables document `md`.
#[tracing::instrument(skip_all)]
pub fn markdown_to_asciidoc(md: &str) -> String {
    let mut w = Writer {
        out: vec![String::new()],
        lists: Vec::new(),
        list_end: None,
        quotes: Vec::new(),
        item: 0,
        code: None,
        cell: false,
        links: Vec::new(),
        text: String::new(),
    };
    for event in Parser::new_ext(md, Options::ENABLE_TABLES) {
        // The parser splits text at escapes; markup is decided per run.
        if let Event::Text(t) = &event
            && w.code.is_none()
        {
            w.text.push_str(t);
            continue;
        }
        w.flush();
        w.event(event);
    }
    w.flush();
    let text = w.out.pop().unwrap_or_default();
    text.trim_end().to_owned() + "\n"
}

struct Writer {
    /// Output stack: links, headings and table cells collect their text first.
    out: Vec<String>,
    /// Open lists: `true` for ordered.
    lists: Vec<bool>,
    /// Output length right after the last top-level list.
    list_end: Option<usize>,
    /// Open block quotes: the list depth each one started at.
    quotes: Vec<usize>,
    /// Output length right after the marker of the open list item.
    item: usize,
    /// Open fenced block: its attribute line and delimiter character.
    code: Option<(String, char)>,
    cell: bool,
    /// Targets of the open links and images: `link:` or `image:` plus URL.
    links: Vec<String>,
    /// Plain text not yet written.
    text: String,
}

impl Writer {
    fn flush(&mut self) {
        if self.text.is_empty() {
            return;
        }
        let raw = std::mem::take(&mut self.text);
        let extra = self.extra_markup();
        // `API:: def` would start a description list, even inside `++…++`.
        let mut text = String::new();
        let mut rest = raw.as_str();
        while let Some(i) = dlist_marker(rest) {
            text += &literal(&rest[..=i], extra);
            text += "{empty}";
            rest = &rest[i + 1..];
        }
        text += &literal(rest, extra);
        let top = self.top();
        let line_start = top.is_empty() || top.ends_with('\n');
        let wrapped = text.starts_with("++") || text.starts_with("pass:c[");
        if self.out.len() == 1 && line_start && !wrapped && block_syntax(&raw) {
            text.insert_str(0, "{empty}");
        }
        let text = if self.cell {
            text.replace('|', "\\|")
        } else {
            text
        };
        self.top().push_str(&text);
    }

    /// In link text a visible `=` turns the text into an attribute list;
    /// in image text a `,` too.
    fn extra_markup(&self) -> &'static [char] {
        match self.links.last() {
            Some(l) if l.starts_with("image:") => &['=', ','],
            Some(_) => &['='],
            None => &[],
        }
    }

    fn top(&mut self) -> &mut String {
        self.out.last_mut().expect("output stack")
    }

    fn line_start(&mut self) {
        let top = self.top();
        if !top.is_empty() && !top.ends_with('\n') {
            top.push('\n');
        }
    }

    fn block_start(&mut self) {
        self.line_start();
        let top = self.top();
        if !top.is_empty() && !top.ends_with("\n\n") {
            top.push('\n');
        }
    }

    /// Starts a block: after a blank line, or inside a list item as a list
    /// continuation (`+`). A paragraph that opens its item needs neither,
    /// nor does a block inside a quote of the item.
    fn block(&mut self, paragraph: bool) {
        if self.lists.is_empty() || self.quotes.last() == Some(&self.lists.len()) {
            return self.block_start();
        }
        let empty = self.top().len() == self.item;
        if empty && paragraph {
            return;
        }
        if empty {
            self.top().push_str("{empty}");
        }
        self.line_start();
        self.top().push_str("+\n");
    }

    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => self.top().push_str(&t),
            Event::Code(c) => {
                // Unconstrained, so a letter or `*` next to it does not matter.
                let code = if c.contains('+') {
                    format!("``{}``", pass(&c))
                } else {
                    format!("``+{c}+``")
                };
                let code = if self.cell {
                    code.replace('|', "\\|")
                } else {
                    code
                };
                self.top().push_str(&code);
            }
            // Lines of an HTML block, collected until its end.
            Event::Html(h) => self.top().push_str(&h),
            Event::InlineHtml(h) if !h.starts_with("<!--") => {
                let text = literal(&h, self.extra_markup());
                self.top().push_str(&text);
            }
            Event::InlineHtml(h) => self.html(&h),
            Event::SoftBreak => {
                let sep = if self.cell { " " } else { "\n" };
                self.top().push_str(sep);
            }
            Event::HardBreak => self.top().push_str(" +\n"),
            Event::Rule => {
                self.block_start();
                self.top().push_str("'''\n");
            }
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph if !self.cell => self.block(true),
            Tag::Heading { level, .. } => {
                self.block_start();
                let marker = "=".repeat(level as usize) + " ";
                self.top().push_str(&marker);
            }
            Tag::BlockQuote(_) => {
                self.block(false);
                self.top().push_str("____\n");
                self.quotes.push(self.lists.len());
            }
            Tag::CodeBlock(kind) => self.code_start(kind),
            Tag::List(start) => {
                if self.lists.is_empty() {
                    let len = self.top().len();
                    let adjacent = self.list_end == Some(len);
                    self.block_start();
                    if adjacent {
                        // Two lists in a row would merge or nest.
                        self.top().push_str("//\n\n");
                    }
                }
                self.lists.push(start.is_some());
            }
            Tag::Item => {
                self.line_start();
                let ch = if self.lists.last() == Some(&true) {
                    "."
                } else {
                    "*"
                };
                let marker = ch.repeat(self.lists.len()) + " ";
                self.top().push_str(&marker);
                self.item = self.top().len();
            }
            Tag::Table(aligns) => self.table_start(&aligns),
            Tag::TableCell => {
                self.cell = true;
                self.out.push(String::new());
            }
            Tag::HtmlBlock => self.out.push(String::new()),
            tag => self.start_inline(tag),
        }
    }

    fn start_inline(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Emphasis => self.top().push_str("__"),
            Tag::Strong => self.top().push_str("**"),
            Tag::Link { dest_url, .. } => {
                self.links.push(format!("link:{dest_url}"));
                self.out.push(String::new());
            }
            Tag::Image { dest_url, .. } => {
                self.links.push(format!("image:{dest_url}"));
                self.out.push(String::new());
            }
            _ => {}
        }
    }

    /// Mermaid and PlantUML as Asciidoctor Diagram literal blocks.
    fn code_start(&mut self, kind: CodeBlockKind<'_>) {
        self.block(false);
        let lang = match kind {
            CodeBlockKind::Fenced(l) => l.split_whitespace().next().unwrap_or("").to_owned(),
            CodeBlockKind::Indented => String::new(),
        };
        let code = match lang.as_str() {
            "mermaid" | "plantuml" => (format!("[{lang}]\n"), '.'),
            "" => (String::new(), '-'),
            l => (format!("[source,{l}]\n"), '-'),
        };
        self.code = Some(code);
        // The content decides the delimiter length, so it is collected first.
        self.out.push(String::new());
    }

    fn table_start(&mut self, aligns: &[Alignment]) {
        self.block(false);
        let cols: Vec<&str> = aligns
            .iter()
            .map(|a| match a {
                Alignment::Right => ">1",
                Alignment::Center => "^1",
                _ => "<1",
            })
            .collect();
        self.top()
            .push_str(&format!("[%header,cols=\"{}\"]\n|===\n", cols.join(",")));
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph if self.lists.is_empty() && !self.cell => self.top().push('\n'),
            TagEnd::Heading(_) | TagEnd::TableHead | TagEnd::TableRow => self.top().push('\n'),
            TagEnd::BlockQuote(_) => {
                self.quotes.pop();
                self.line_start();
                self.top().push_str("____\n");
            }
            TagEnd::CodeBlock => {
                let content = self.out.pop().unwrap_or_default();
                let (head, ch) = self.code.take().unwrap_or_default();
                let block = head + &delimited(ch, &content);
                self.top().push_str(&block);
            }
            TagEnd::List(_) => {
                self.lists.pop();
                if self.lists.is_empty() {
                    self.top().push('\n');
                    self.list_end = Some(self.top().len());
                }
            }
            TagEnd::Table => self.top().push_str("|===\n"),
            TagEnd::TableCell => {
                self.cell = false;
                let text = self.out.pop().unwrap_or_default();
                self.top().push_str(&format!("| {} ", text.trim()));
            }
            TagEnd::HtmlBlock => {
                let html = self.out.pop().unwrap_or_default();
                self.block_start();
                self.html(&html);
            }
            tag => self.end_inline(tag),
        }
    }

    fn end_inline(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Emphasis => self.top().push_str("__"),
            TagEnd::Strong => self.top().push_str("**"),
            TagEnd::Link | TagEnd::Image => {
                let text = self.out.pop().unwrap_or_default();
                let target = self.links.pop().unwrap_or_default();
                // Every `]` of the text is inside a passthrough already.
                let link = format!("{target}[{text}]");
                self.top().push_str(&link);
            }
            _ => {}
        }
    }

    /// HTML comments become AsciiDoc comments; other HTML passes through.
    fn html(&mut self, h: &str) {
        let t = h.trim();
        if let Some(inner) = t.strip_prefix("<!--").and_then(|r| r.strip_suffix("-->")) {
            self.line_start();
            let inner = inner.trim();
            if inner.contains('\n') {
                self.top().push_str(&delimited('/', inner));
            } else {
                self.top().push_str(&format!("// {inner}\n"));
            }
        } else {
            self.block_start();
            self.top().push_str(&delimited('+', t));
        }
    }
}

/// `content` as a delimited block of `ch`: the delimiter is longer than any
/// content line made of `ch` alone, so none closes the block early, and a
/// line-start preprocessor directive is escaped, as Asciidoctor would run
/// it (e.g. `include::/etc/passwd[]`) in any block; the `\` does not render.
fn delimited(ch: char, content: &str) -> String {
    const DIRECTIVES: [&str; 5] = ["include::", "ifdef::", "ifndef::", "ifeval::", "endif::"];
    let longest = content
        .lines()
        .filter(|l| !l.is_empty() && l.chars().all(|c| c == ch))
        .map(str::len)
        .max()
        .unwrap_or(0);
    let delim = ch.to_string().repeat(longest.max(3) + 1);
    let mut out = format!("{delim}\n");
    for line in content.lines() {
        if DIRECTIVES.iter().any(|d| line.starts_with(d)) {
            out.push('\\');
        }
        out += line;
        out.push('\n');
    }
    out + &delim + "\n"
}

/// Text that contains AsciiDoc markup characters or one of `extra`,
/// wrapped as literal.
fn literal(t: &str, extra: &[char]) -> String {
    const MARKUP: &[char] = &[
        '*', '_', '`', '#', '^', '~', '{', '}', '[', ']', '<', '>', '+', '\\', '\'', '"', '&',
    ];
    if t.contains("++") {
        // `++…++` cannot hold `++`.
        pass(t)
    } else if t.contains(MARKUP) || t.contains(extra) {
        format!("++{t}++")
    } else {
        t.to_owned()
    }
}

/// `t` as a `pass:c[…]` macro; trailing backslashes follow it as
/// `{backslash}`: one before `]` would escape it, a bare one the next markup.
fn pass(t: &str) -> String {
    let body = t.trim_end_matches('\\');
    let tail = "{backslash}".repeat(t.len() - body.len());
    format!("pass:c[{}]{tail}", body.replace(']', "\\]"))
}

/// Index of the first character of a description list marker (`::` or
/// `;;` before a space or the end) in `t`.
fn dlist_marker(t: &str) -> Option<usize> {
    let b = t.as_bytes();
    (1..b.len())
        .find(|&i| {
            matches!(&b[i - 1..=i], b"::" | b";;")
                && b.get(i + 1).is_none_or(u8::is_ascii_whitespace)
        })
        .map(|i| i - 1)
}

/// Whether a line starting with `t` could be read as a block title,
/// comment, attribute entry, admonition, list item or delimiter; a
/// leading `{empty}` keeps it text and renders as nothing.
fn block_syntax(t: &str) -> bool {
    let word = t.split_whitespace().next().unwrap_or("");
    !t.starts_with(char::is_alphanumeric) || word.ends_with(['.', ')', ':', ';'])
}

#[cfg(test)]
mod tests {
    use super::markdown_to_asciidoc as adoc;

    #[test]
    fn headings_paragraphs_and_inline_markup() {
        let a = adoc(
            "# Title\n\nSome **bold**, *it* and `code`.\n\n## 1. Chapter\n\n### Sub\n\ntext\n",
        );
        assert_eq!(
            a,
            "= Title\n\nSome **bold**, __it__ and ``+code+``.\n\n== 1. Chapter\n\n=== Sub\n\ntext\n"
        );
    }

    #[test]
    fn links_lists_and_comments() {
        let a = adoc(
            "See [arc42](https://arc42.org/overview/).\n\n<!-- archguard:arc42:01:start -->\n- one\n- two\n  1. nested\n\n<!-- archguard:arc42:01:end -->\n",
        );
        assert!(
            a.contains("See link:https://arc42.org/overview/[arc42]."),
            "{a}"
        );
        assert!(a.contains("// archguard:arc42:01:start\n"), "{a}");
        assert!(a.contains("* one\n* two\n.. nested\n"), "{a}");
        assert!(
            a.contains("two\n.. nested\n\n// archguard:arc42:01:end\n"),
            "{a}"
        );
        let multi = adoc("text\n\n<!-- first\nsecond -->\n\nmore\n");
        assert_eq!(multi, "text\n\n////\nfirst\nsecond\n////\n\nmore\n");
    }

    #[test]
    fn tables_keep_alignment_and_escape_pipes() {
        let a = adoc("| Source | Files |\n| --- | ---: |\n| a\\|b | 3 |\n| `x` | 4 |\n");
        assert_eq!(
            a,
            "[%header,cols=\"<1,>1\"]\n|===\n| Source | Files \n| a\\|b | 3 \n| ``+x+`` | 4 \n|===\n"
        );
    }

    #[test]
    fn diagrams_become_asciidoctor_diagram_blocks() {
        let a = adoc(
            "```mermaid\nflowchart LR\n  e0[\"a\"]\n```\n\n```plantuml\n@startuml\n@enduml\n```\n\n```rust\nfn x() {}\n```\n",
        );
        assert!(
            a.contains("[mermaid]\n....\nflowchart LR\n  e0[\"a\"]\n....\n"),
            "{a}"
        );
        assert!(
            a.contains("[plantuml]\n....\n@startuml\n@enduml\n....\n"),
            "{a}"
        );
        assert!(a.contains("[source,rust]\n----\nfn x() {}\n----\n"), "{a}");
    }

    #[test]
    fn markup_characters_in_text_stay_literal() {
        let a = adoc("env SHIPPING\\_API and {id} *x*\n");
        assert_eq!(a, "++env SHIPPING_API and {id} ++__x__\n");
    }

    #[test]
    fn line_starts_with_block_syntax_stay_text() {
        assert_eq!(adoc(".NET 8 runs here.\n"), "{empty}.NET 8 runs here.\n");
        assert_eq!(adoc("// not a comment\n"), "{empty}// not a comment\n");
        assert_eq!(adoc(":attr: value\n"), "{empty}:attr: value\n");
        for label in ["NOTE", "TIP", "IMPORTANT", "WARNING", "CAUTION"] {
            assert_eq!(
                adoc(&format!("{label}: read this\n")),
                format!("{{empty}}{label}: read this\n")
            );
        }
        assert_eq!(adoc("a\n.NET\n"), "a\n{empty}.NET\n");
        assert_eq!(adoc("Use .NET 8.\n"), "Use .NET 8.\n");
    }

    #[test]
    fn text_with_double_plus_is_passed_through() {
        assert_eq!(
            adoc("C++ and {product-name} and #tag#\n"),
            "pass:c[C++ and {product-name} and #tag#]\n"
        );
        assert_eq!(adoc("a ++ b\\]\n"), "pass:c[a ++ b\\]]\n");
    }

    #[test]
    fn link_text_keeps_commas_and_equals_signs() {
        assert_eq!(
            adoc("[docs, version=2](https://a.org)\n"),
            "link:https://a.org[++docs, version=2++]\n"
        );
        assert_eq!(
            adoc("[a\\]b](https://a.org)\n"),
            "link:https://a.org[++a]b++]\n"
        );
    }

    #[test]
    fn code_spans_are_unconstrained() {
        assert_eq!(
            adoc("a `x`s and *`y`* and `a+b`\n"),
            "a ``+x+``s and __``+y+``__ and ``pass:c[a+b]``\n"
        );
    }

    #[test]
    fn blocks_inside_list_items_are_continued() {
        assert_eq!(
            adoc("- first point\n\n  second paragraph\n- next\n"),
            "* first point\n+\nsecond paragraph\n* next\n"
        );
        let code = "\n   ```\n   code\n   ```\n2. next\n";
        for start in ["1. step\n", "1. step\n\n"] {
            assert_eq!(
                adoc(&format!("{start}{code}")),
                ". step\n+\n----\ncode\n----\n. next\n",
                "{start:?}"
            );
        }
    }

    #[test]
    fn trailing_backslashes_stay_outside_passthrough_macros() {
        assert_eq!(
            adoc("`c+\\` and *b*\n"),
            "``pass:c[c+]{backslash}`` and __b__\n"
        );
        assert_eq!(adoc("a ++ b\\\\*x*\n"), "pass:c[a ++ b]{backslash}__x__\n");
    }

    #[test]
    fn description_list_markers_are_broken() {
        // The leading `{empty}` comes from `block_syntax` (a word ending in `:`).
        assert_eq!(adoc("API:: def\n"), "{empty}API:{empty}: def\n");
        assert_eq!(adoc("Hint;; x\n"), "{empty}Hint;{empty}; x\n");
        assert_eq!(adoc("Foo bar::\n"), "Foo bar:{empty}:\n");
        assert_eq!(adoc("a::: b\n"), "{empty}a::{empty}: b\n");
        assert_eq!(adoc("API:: {x}\n"), "{empty}API:{empty}++: {x}++\n");
        assert_eq!(
            adoc("C++ and Rust:: ok\n"),
            "pass:c[C++ and Rust:]{empty}: ok\n"
        );
        assert_eq!(adoc("a::b and c ::\n"), "a::b and c :{empty}:\n");
        // A code span or link before the marker: the marker is in the text after it.
        assert_eq!(adoc("`api`:: def\n"), "``+api+``:{empty}: def\n");
        assert_eq!(adoc("[api](u):: def\n"), "link:u[api]:{empty}: def\n");
    }

    #[test]
    fn block_quotes_in_list_items_have_no_inner_continuation() {
        assert_eq!(
            adoc("1. step\n\n   > quoted in item\n   >\n   > second\n2. after\n"),
            ". step\n+\n____\n\nquoted in item\n\nsecond\n____\n. after\n"
        );
        assert_eq!(adoc("> - a\n>\n> - b\n"), "____\n\n* a\n* b\n____\n");
    }

    #[test]
    fn image_text_with_commas_stays_alt_text() {
        assert_eq!(
            adoc("![Context, overview](ctx.png)\n"),
            "image:ctx.png[++Context, overview++]\n"
        );
    }

    #[test]
    fn adjacent_lists_stay_apart() {
        assert_eq!(adoc("- a\n\n1. b\n"), "* a\n\n//\n\n. b\n");
        assert_eq!(adoc("- a\n\ntext\n\n1. b\n"), "* a\n\ntext\n\n. b\n");
    }

    /// Repository Markdown in verbatim blocks never becomes a live
    /// preprocessor directive, and no content line closes its block.
    #[test]
    fn verbatim_blocks_cannot_include_files_or_close_early() {
        let code = adoc("```\ninclude::/etc/passwd[]\n----\nifdef::x[]\n```\n");
        assert_eq!(
            code,
            "-----\n\\include::/etc/passwd[]\n----\n\\ifdef::x[]\n-----\n"
        );
        let diagram = adoc("```mermaid\n....\n......\nendif::[]\n```\n");
        assert_eq!(
            diagram,
            "[mermaid]\n.......\n....\n......\n\\endif::[]\n.......\n"
        );
        let html = adoc("<div>\n++++\ninclude::/etc/passwd[]\n</div>\n");
        assert_eq!(
            html,
            "+++++\n<div>\n++++\n\\include::/etc/passwd[]\n</div>\n+++++\n"
        );
        let comment = adoc("<!-- a\n////\nifeval::[1 == 1]\nifndef::y[] -->\n");
        assert_eq!(
            comment,
            "/////\na\n////\n\\ifeval::[1 == 1]\n\\ifndef::y[]\n/////\n"
        );
    }
}
