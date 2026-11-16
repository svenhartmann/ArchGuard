//! A chapter as the rich-text editor holds it: the Markdown's top-level
//! blocks ([`markdown::chunks`]) as parts of formatted blocks, and between
//! them the text it keeps as written (blank lines, comments) along with
//! the blocks it does not edit: tables, rules, quotes holding more than
//! text, the chapter heading and generated `archguard:arc42` blocks
//! (saving refreshes them). A part nobody edited writes its source back
//! unchanged, so opening and saving a chapter changes nothing.

use std::ops::Range;

use super::super::markdown::{self, Block as Md};
use super::inline::Inline;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Paragraph,
    /// 2..=6: the reader shows `#` as level 2 as well.
    Heading(usize),
    /// A list entry; `number` in an ordered list, `None` for a bullet.
    Item {
        depth: usize,
        number: Option<u64>,
    },
    /// A paragraph inside a quote.
    Quote,
    /// Its text is the code; it has no marks.
    Code {
        lang: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub kind: Kind,
    pub inline: Inline,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    /// The Markdown it was read from.
    source: String,
    blocks: Vec<Block>,
    edited: bool,
}

impl Part {
    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// The blocks to change; from now on the part writes them, not its source.
    pub fn edit(&mut self) -> &mut Vec<Block> {
        self.edited = true;
        &mut self.blocks
    }

    /// A part the chapter did not have: one empty paragraph to type in.
    fn new() -> Part {
        Part {
            source: String::new(),
            blocks: vec![Block {
                kind: Kind::Paragraph,
                inline: Inline::default(),
            }],
            edited: true,
        }
    }

    fn markdown(&self) -> String {
        if self.edited {
            write(&self.blocks)
        } else {
            self.source.clone()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    /// Kept as written.
    Fixed(String),
    Part(Part),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Doc {
    pub pieces: Vec<Piece>,
}

impl Doc {
    /// Read chapter Markdown (LF line ends); the blocks overlapping `fixed`
    /// (byte ranges of `md`) stay as written.
    pub fn parse(md: &str, fixed: &[Range<usize>]) -> Doc {
        let mut pieces = Vec::new();
        let keep = |pieces: &mut Vec<Piece>, text: &str| match pieces.last_mut() {
            _ if text.is_empty() => {}
            Some(Piece::Fixed(t)) => t.push_str(text),
            _ => pieces.push(Piece::Fixed(text.to_string())),
        };
        let mut pos = 0;
        for (range, blocks) in markdown::chunks(md) {
            keep(&mut pieces, &md[pos..range.start]);
            let source = &md[range.clone()];
            let locked = fixed
                .iter()
                .any(|g| g.start < range.end && range.start < g.end);
            match editable(&blocks).filter(|_| !locked) {
                // Parts with only blank lines between are one: Backspace
                // joins paragraphs across them.
                Some(blocks) => match pieces.as_mut_slice() {
                    [.., Piece::Part(last), Piece::Fixed(gap)] if gap.trim().is_empty() => {
                        last.source.push_str(gap);
                        last.source.push_str(source);
                        last.blocks.extend(blocks);
                        pieces.pop();
                    }
                    _ => pieces.push(Piece::Part(Part {
                        source: source.to_string(),
                        blocks,
                        edited: false,
                    })),
                },
                None => keep(&mut pieces, source),
            }
            pos = range.end;
        }
        keep(&mut pieces, &md[pos..]);
        Doc { pieces }
    }

    /// The editable part `piece`, if it is one.
    pub fn part(&self, piece: usize) -> Option<&Part> {
        match self.pieces.get(piece)? {
            Piece::Part(part) => Some(part),
            Piece::Fixed(_) => None,
        }
    }

    pub fn part_mut(&mut self, piece: usize) -> Option<&mut Part> {
        match self.pieces.get_mut(piece)? {
            Piece::Part(part) => Some(part),
            Piece::Fixed(_) => None,
        }
    }

    /// Add an empty part at the end to type in; its piece.
    pub fn append_part(&mut self) -> usize {
        self.pieces.push(Piece::Part(Part::new()));
        self.pieces.len() - 1
    }

    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        for (i, p) in self.pieces.iter().enumerate() {
            match p {
                Piece::Fixed(text) => out.push_str(text),
                Piece::Part(part) => {
                    let text = part.markdown();
                    // A new part starts a block of its own and ends its line.
                    if part.source.is_empty() && !text.is_empty() {
                        while !out.is_empty() && !out.ends_with("\n\n") {
                            out.push('\n');
                        }
                        out.push_str(&text);
                        if i + 1 == self.pieces.len() {
                            out.push('\n');
                        }
                    } else {
                        out.push_str(&text);
                    }
                }
            }
        }
        out
    }
}

/// The blocks the editor edits, or `None` for a chunk it keeps as written.
fn editable(blocks: &[Md]) -> Option<Vec<Block>> {
    let mut out = Vec::new();
    for b in blocks {
        let block = |kind, text: &str| Block {
            kind,
            inline: one_line(text),
        };
        match b {
            // The chapter heading (`## 5. Bausteinsicht`) names the chapter.
            Md::Heading { text, .. } if chapter_number(text) => return None,
            Md::Heading { level, text } => out.push(block(Kind::Heading(*level), text)),
            Md::Paragraph(text) => out.push(block(Kind::Paragraph, text)),
            Md::Item {
                depth,
                marker,
                text,
            } => {
                let number = marker.trim_end_matches('.').parse().ok();
                out.push(block(
                    Kind::Item {
                        depth: *depth,
                        number,
                    },
                    text,
                ));
            }
            Md::Quote(inner) => {
                for q in inner {
                    let Md::Paragraph(text) = q else {
                        return None;
                    };
                    out.push(block(Kind::Quote, text));
                }
            }
            Md::Code { lang, text } => out.push(Block {
                kind: Kind::Code { lang: lang.clone() },
                inline: Inline::plain(text),
            }),
            Md::Table { .. } | Md::Rule => return None,
        }
    }
    Some(out)
}

fn chapter_number(heading: &str) -> bool {
    let digits = heading.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && heading[digits..].starts_with('.')
}

/// Inline Markdown whose source line breaks are spaces, as the reader
/// shows them; an edited block is written on one line.
fn one_line(md: &str) -> Inline {
    let mut inline = Inline::parse(md);
    // Same length: the marks keep their bytes.
    inline.text = inline.text.replace('\n', " ");
    inline
}

/// The Markdown of edited blocks; empty text blocks are dropped.
fn write(blocks: &[Block]) -> String {
    let mut out = String::new();
    // Per list depth: the marker width (for the indent below it) and the
    // last number of an ordered list.
    let mut widths: Vec<usize> = Vec::new();
    let mut numbers: Vec<Option<u64>> = Vec::new();
    let mut prev: Option<&Kind> = None;
    for b in blocks {
        let code = matches!(b.kind, Kind::Code { .. });
        if !code && b.inline.text.trim().is_empty() {
            continue;
        }
        out.push_str(match (prev, &b.kind) {
            (None, _) => "",
            (Some(Kind::Item { .. }), Kind::Item { .. }) => "\n",
            (Some(Kind::Quote), Kind::Quote) => "\n>\n",
            _ => "\n\n",
        });
        if !matches!(b.kind, Kind::Item { .. }) {
            widths.clear();
            numbers.clear();
        }
        let text = b.inline.to_markdown();
        match &b.kind {
            Kind::Paragraph => out.push_str(&text),
            Kind::Heading(level) => {
                out.push_str(&"#".repeat((*level).clamp(2, 6)));
                out.push(' ');
                out.push_str(&text);
            }
            Kind::Item { depth, number } => {
                let depth = (*depth).min(widths.len());
                widths.truncate(depth);
                numbers.truncate(depth + 1);
                // An ordered entry after one of its list counts on from it.
                let number = match (number, numbers.get(depth)) {
                    (Some(_), Some(Some(last))) => Some(last + 1),
                    (n, _) => *n,
                };
                numbers.resize(depth, None);
                numbers.push(number);
                let marker = number.map_or("-".to_string(), |n| format!("{n}."));
                out.push_str(&" ".repeat(widths.iter().sum()));
                out.push_str(&marker);
                out.push(' ');
                out.push_str(&text);
                widths.push(marker.len() + 1);
            }
            Kind::Quote => {
                out.push_str("> ");
                out.push_str(&text);
            }
            Kind::Code { lang } => {
                // shortcut: a line of three backticks inside the code ends it
                // early; write a longer fence once the reader knows them.
                out.push_str("```");
                out.push_str(lang);
                out.push('\n');
                out.push_str(&b.inline.text);
                out.push_str("\n```");
            }
        }
        prev = Some(&b.kind);
    }
    out
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::super::inline::{Mark, Span};
    use super::*;

    const CHAPTER: &str = "\
## 5. Bausteinsicht

<!-- archguard:arc42:05:start -->
Quelle: vollständiger Quellgraph.

| Baustein | Dateien |
| --- | ---: |
| app | 3 |
<!-- archguard:arc42:05:end -->

<!-- Eigene Erläuterungen zu Kapitel 5 hier ergänzen. -->
Ein Absatz mit **fett** und
einer zweiten Zeile.

* Punkt eins
    * eingerückt
3) drei
4) vier

> Zitat *kursiv*

```rust
fn main() {}
```

---

Ende.
";

    fn generated(md: &str) -> Vec<Range<usize>> {
        let start = md.find("<!-- archguard:arc42:05:start").unwrap();
        let end = md.find("<!-- archguard:arc42:05:end").unwrap();
        vec![start..end]
    }

    fn parts(doc: &mut Doc) -> Vec<&mut Part> {
        doc.pieces
            .iter_mut()
            .filter_map(|p| match p {
                Piece::Part(part) => Some(part),
                Piece::Fixed(_) => None,
            })
            .collect()
    }

    #[test]
    fn an_untouched_chapter_writes_back_unchanged() {
        let doc = Doc::parse(CHAPTER, &generated(CHAPTER));
        assert_eq!(doc.to_markdown(), CHAPTER);
    }

    /// Every chapter the exporter writes for two projects writes back byte
    /// for byte. A fresh chapter is all generated blocks and a comment
    /// asking for own text: the editor offers an empty paragraph there.
    #[test]
    fn exported_chapters_write_back_unchanged() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for project in ["fixtures/py-layers", "examples/service-shop"] {
            let dir = root.join(project).canonicalize().expect("project");
            let loaded = archguard_engine::load_config(None, &dir).expect("config");
            let run =
                archguard_engine::analyze_run_quiet(&dir, &loaded, archguard_engine::Level::Module)
                    .expect("analysis");
            let doc = archguard_engine::arc42_document(&dir, &run.config, &run.graph, &run.report)
                .expect("arc42");
            for n in 1..=12 {
                let md = crate::arc42_file::generated(&doc, n);
                let parsed = Doc::parse(&md, &super::super::super::edit::generated(&md));
                assert_eq!(parsed.to_markdown(), md, "{project} chapter {n}");
            }
        }
    }

    #[test]
    fn heading_generated_blocks_tables_and_rules_stay_fixed() {
        let mut doc = Doc::parse(CHAPTER, &generated(CHAPTER));
        let kinds: Vec<Vec<Kind>> = parts(&mut doc)
            .iter()
            .map(|p| p.blocks().iter().map(|b| b.kind.clone()).collect())
            .collect();
        let item = |depth, number| Kind::Item { depth, number };
        assert_eq!(
            kinds,
            // One part up to the rule: only blank lines lie between.
            [
                vec![
                    Kind::Paragraph,
                    item(0, None),
                    item(1, None),
                    item(0, Some(3)),
                    item(0, Some(4)),
                    Kind::Quote,
                    Kind::Code {
                        lang: "rust".into()
                    },
                ],
                vec![Kind::Paragraph],
            ]
        );
        let fixed: String = doc
            .pieces
            .iter()
            .filter_map(|p| match p {
                Piece::Fixed(t) => Some(t.as_str()),
                Piece::Part(_) => None,
            })
            .collect();
        for kept in [
            "## 5. Bausteinsicht",
            "| app | 3 |",
            "Quellgraph.",
            "\n---\n",
        ] {
            assert!(fixed.contains(kept), "{kept}");
        }
    }

    #[test]
    fn a_paragraph_reads_its_line_breaks_as_spaces() {
        let mut doc = Doc::parse(CHAPTER, &generated(CHAPTER));
        let all = parts(&mut doc);
        let p = &all[0].blocks()[0].inline;
        assert_eq!(p.text, "Ein Absatz mit fett und einer zweiten Zeile.");
        assert_eq!(
            p.spans,
            [Span {
                range: 15..19,
                mark: Mark::Bold
            }]
        );
    }

    #[test]
    fn edited_parts_write_markdown_that_reads_back_the_same() {
        let mut doc = Doc::parse(CHAPTER, &generated(CHAPTER));
        for part in parts(&mut doc) {
            part.edit();
        }
        let md = doc.to_markdown();
        let mut again = Doc::parse(&md, &generated(&md));
        let blocks = |d: &mut Doc| -> Vec<Vec<Block>> {
            parts(d).iter().map(|p| p.blocks().to_vec()).collect()
        };
        assert_eq!(blocks(&mut again), blocks(&mut doc), "{md}");
        // The edited list in its written form.
        assert!(
            md.contains("- Punkt eins\n  - eingerückt\n3. drei\n4. vier\n"),
            "{md}"
        );
        assert!(md.contains("> Zitat *kursiv*"), "{md}");
    }

    #[test]
    fn parts_with_only_blank_lines_between_are_one() {
        let md = "a\n\nb\n\n<!-- c -->\nd\n";
        let mut doc = Doc::parse(md, &[]);
        assert_eq!(
            parts(&mut doc)
                .iter()
                .map(|p| p.blocks().len())
                .collect::<Vec<_>>(),
            [2, 1]
        );
        assert_eq!(doc.to_markdown(), md);
    }

    #[test]
    fn a_new_part_is_its_own_block_and_writes_nothing_while_empty() {
        let md = "<!-- Eigene Erläuterungen hier ergänzen. -->\n";
        let mut doc = Doc::parse(md, &[]);
        let piece = doc.append_part();
        assert_eq!(doc.to_markdown(), md, "empty");
        doc.part_mut(piece).unwrap().edit()[0].inline = Inline::plain("Neu.");
        assert_eq!(
            doc.to_markdown(),
            "<!-- Eigene Erläuterungen hier ergänzen. -->\n\nNeu.\n"
        );
    }

    #[test]
    fn new_list_entries_count_on_and_empty_blocks_go() {
        let md = "1. a\n2. b\n";
        let mut doc = Doc::parse(md, &[]);
        let mut all = parts(&mut doc);
        let blocks = all.remove(0).edit();
        let mut new = blocks[0].clone();
        new.inline = Inline::plain("neu");
        blocks.insert(1, new);
        blocks.push(Block {
            kind: Kind::Paragraph,
            inline: Inline::plain("  "),
        });
        assert_eq!(doc.to_markdown(), "1. a\n2. neu\n3. b\n");
    }
}
