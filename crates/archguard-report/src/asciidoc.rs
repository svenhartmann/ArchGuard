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
    /// Open fenced block: its closing delimiter.
    code: Option<&'static str>,
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
        let text = literal(&std::mem::take(&mut self.text));
        let text = if self.cell {
            text.replace('|', "\\|")
        } else {
            text
        };
        self.top().push_str(&text);
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

    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => self.top().push_str(&t),
            Event::Code(c) => {
                let code = if c.contains('+') {
                    format!("`pass:c[{}]`", c.replace(']', "\\]"))
                } else {
                    format!("`+{c}+`")
                };
                let code = if self.cell {
                    code.replace('|', "\\|")
                } else {
                    code
                };
                self.top().push_str(&code);
            }
            Event::Html(h) => self.html(&h),
            Event::InlineHtml(h) if !h.starts_with("<!--") => {
                let text = literal(&h);
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
            Tag::Paragraph if self.lists.is_empty() && !self.cell => self.block_start(),
            Tag::Heading { level, .. } => {
                self.block_start();
                let marker = "=".repeat(level as usize) + " ";
                self.top().push_str(&marker);
            }
            Tag::BlockQuote(_) => {
                self.block_start();
                self.top().push_str("____\n");
            }
            Tag::CodeBlock(kind) => self.code_start(kind),
            Tag::List(start) => {
                if self.lists.is_empty() {
                    self.block_start();
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
            }
            Tag::Table(aligns) => self.table_start(&aligns),
            Tag::TableCell => {
                self.cell = true;
                self.out.push(String::new());
            }
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
        self.block_start();
        let lang = match kind {
            CodeBlockKind::Fenced(l) => l.split_whitespace().next().unwrap_or("").to_owned(),
            CodeBlockKind::Indented => String::new(),
        };
        let (head, delim) = match lang.as_str() {
            "mermaid" | "plantuml" => (format!("[{lang}]\n"), "...."),
            "" => (String::new(), "----"),
            l => (format!("[source,{l}]\n"), "----"),
        };
        self.top().push_str(&format!("{head}{delim}\n"));
        self.code = Some(delim);
    }

    fn table_start(&mut self, aligns: &[Alignment]) {
        self.block_start();
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
                self.line_start();
                self.top().push_str("____\n");
            }
            TagEnd::CodeBlock => {
                self.line_start();
                let delim = self.code.take().unwrap_or("----");
                self.top().push_str(delim);
                self.top().push('\n');
            }
            TagEnd::List(_) => {
                self.lists.pop();
                if self.lists.is_empty() {
                    self.top().push('\n');
                }
            }
            TagEnd::Table => self.top().push_str("|===\n"),
            TagEnd::TableCell => {
                self.cell = false;
                let text = self.out.pop().unwrap_or_default();
                self.top().push_str(&format!("| {} ", text.trim()));
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
                let link = format!("{target}[{}]", text.replace(']', "\\]"));
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
                self.top().push_str(&format!("////\n{inner}\n////\n"));
            } else {
                self.top().push_str(&format!("// {inner}\n"));
            }
        } else {
            self.block_start();
            self.top().push_str(&format!("++++\n{t}\n++++\n"));
        }
    }
}

/// Text that contains AsciiDoc markup characters, wrapped as literal.
fn literal(t: &str) -> String {
    const MARKUP: &[char] = &[
        '*', '_', '`', '#', '^', '~', '{', '}', '[', ']', '<', '>', '+', '\\', '\'', '"', '&',
    ];
    if t.contains(MARKUP) && !t.contains("++") {
        format!("++{t}++")
    } else {
        t.to_owned()
    }
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
            "= Title\n\nSome **bold**, __it__ and `+code+`.\n\n== 1. Chapter\n\n=== Sub\n\ntext\n"
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
        assert!(a.contains("// archguard:arc42:01:end\n"), "{a}");
    }

    #[test]
    fn tables_keep_alignment_and_escape_pipes() {
        let a = adoc("| Source | Files |\n| --- | ---: |\n| a\\|b | 3 |\n| `x` | 4 |\n");
        assert_eq!(
            a,
            "[%header,cols=\"<1,>1\"]\n|===\n| Source | Files \n| a\\|b | 3 \n| `+x+` | 4 \n|===\n"
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
}
