//! A chapter as the rich-text editor holds it: the Markdown's top-level
//! blocks ([`markdown::chunks`]) as parts of formatted blocks, and between
//! them the text it keeps as written (blank lines, comments) along with
//! the blocks it does not edit: rules, quotes holding more than text, text
//! it would not write back as read (reference links, footnotes), the
//! chapter heading and generated `archguard:arc42` blocks (saving
//! refreshes them). A table is one cell block per cell. A part nobody edited writes its source back
//! unchanged, so opening and saving a chapter changes nothing.

use std::ops::Range;

use super::super::markdown::{self, Align, Block as Md};
use super::inline::Inline;
pub use crate::arc42_file::{diagram_block, diagram_ranges};

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
    /// A diagram of the analysis kept in the text (`archguard:diagram`
    /// markers): drawn as a picture, refreshed on save; `source` is its
    /// Markdown with the markers. It has no text.
    Diagram {
        id: String,
        source: String,
    },
    /// A table cell; the cells of a table follow each other row by row,
    /// row 0 the header.
    Cell {
        row: usize,
        col: usize,
        cols: usize,
        align: Align,
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
        let locked = |r: &Range<usize>| fixed.iter().any(|g| g.start < r.end && r.start < g.end);
        // Diagram blocks are one block each; a chunk reaching into one (an
        // unclosed fence before it) stays as written.
        let diagrams = diagram_ranges(md);
        let mut units: Vec<(Range<usize>, Option<Vec<Block>>)> = markdown::chunks(md)
            .into_iter()
            .filter(|(r, _)| {
                !diagrams
                    .iter()
                    .any(|(d, _)| d.start < r.end && r.start < d.end)
            })
            .map(|(r, blocks)| {
                // A quote without text gives no block: a part needs one
                // for the caret.
                let editable = editable(&blocks)
                    .filter(|b| !b.is_empty() && !locked(&r) && !as_written(&md[r.clone()], &blocks));
                (r, editable)
            })
            .collect();
        for (r, id) in diagrams {
            let block = Block {
                kind: Kind::Diagram {
                    id,
                    source: md[r.clone()].to_string(),
                },
                inline: Inline::default(),
            };
            let editable = (!locked(&r)).then(|| vec![block]);
            units.push((r, editable));
        }
        units.sort_by_key(|(r, _)| r.start);
        let mut pieces = Vec::new();
        let keep = |pieces: &mut Vec<Piece>, text: &str| match pieces.last_mut() {
            _ if text.is_empty() => {}
            Some(Piece::Fixed(t)) => t.push_str(text),
            _ => pieces.push(Piece::Fixed(text.to_string())),
        };
        let mut pos = 0;
        for (range, editable) in units {
            keep(&mut pieces, &md[pos..range.start]);
            let source = &md[range.clone()];
            match editable {
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

    /// The Markdown piece `i` writes.
    pub fn piece_markdown(&self, i: usize) -> String {
        match &self.pieces[i] {
            Piece::Fixed(text) => text.clone(),
            Piece::Part(part) => part.markdown(),
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

/// The bytes of `text` inside generated `archguard:arc42` blocks; a block
/// without its end marker runs to the end.
pub fn generated(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut open = None;
    for (i, _) in text.match_indices(crate::arc42_file::MARKER) {
        let tag = text[i..].split_once("-->").map_or("", |(tag, _)| tag);
        if tag.trim_end().ends_with(":start") {
            open.get_or_insert(i);
        } else if let Some(start) = open.take() {
            out.push(start..i);
        }
    }
    out.extend(open.map(|start| start..text.len()));
    out
}

/// The editable blocks of Markdown `md` (a detached diagram's section).
pub fn blocks_of(md: &str) -> Vec<Block> {
    Doc::parse(md, &[])
        .pieces
        .into_iter()
        .filter_map(|p| match p {
            Piece::Part(part) => Some(part.blocks),
            Piece::Fixed(_) => None,
        })
        .flatten()
        .collect()
}

/// The blocks the editor edits, or `None` for a chunk it keeps as written.
fn editable(blocks: &[Md]) -> Option<Vec<Block>> {
    let mut out = Vec::new();
    let mut faithful = true;
    for b in blocks {
        let mut block = |kind, text: &str| {
            let block = Block {
                kind,
                inline: one_line(text),
            };
            faithful &= writes_back(&block, text);
            block
        };
        match b {
            // The chapter heading (`## 5. Bausteinsicht`) names the chapter.
            Md::Heading { level: 2, text } if chapter_heading(text) => return None,
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
            Md::Table {
                columns,
                align,
                rows,
            } => {
                let cols = columns.len();
                for (row, cells) in std::iter::once(columns).chain(rows).enumerate() {
                    for (col, text) in cells.iter().enumerate() {
                        let align = align.get(col).copied().unwrap_or(Align::Left);
                        let kind = Kind::Cell {
                            row,
                            col,
                            cols,
                            align,
                        };
                        out.push(block(kind, text));
                    }
                }
            }
            Md::Rule => return None,
        }
    }
    faithful.then_some(out)
}

/// Whether chunk `source` holds Markdown its `blocks` lose: a comment
/// outside fenced code (the reader drops it), or code, a setext heading
/// or HTML the reader takes for a paragraph or list (an edit would write
/// it on one line).
fn as_written(source: &str, blocks: &[Md]) -> bool {
    let code = matches!(blocks, [Md::Code { .. }]);
    let indented = |l: &str| l.starts_with("    ") || l.starts_with('\t');
    let not_prose = |l: &str| {
        let t = l.trim();
        indented(l)
            || t.starts_with("~~~")
            || t.starts_with('<')
            || !t.is_empty() && (t.chars().all(|c| c == '=') || t.chars().all(|c| c == '-'))
    };
    !code && (source.contains("<!--") || source.contains("-->"))
        || indented(source)
        || matches!(blocks, [Md::Paragraph(_)]) && source.lines().any(not_prose)
        || wider_than_header(source, blocks)
}

/// Whether a table row of chunk `source` has more cells than its header:
/// the reader drops the extra cells.
fn wider_than_header(source: &str, blocks: &[Md]) -> bool {
    let [Md::Table { columns, .. }] = blocks else {
        return false;
    };
    source
        .lines()
        .skip(2)
        .any(|row| markdown::cells(row).len() > columns.len())
}

/// Whether `block`, read from `md`, writes it back (line breaks as
/// spaces). Else `md` holds Markdown the editor does not model (reference
/// links and definitions, footnotes, link titles, hard breaks, entities)
/// and its chunk stays as written.
fn writes_back(block: &Block, md: &str) -> bool {
    let mut again = block.inline.to_markdown();
    if matches!(block.kind, Kind::Cell { .. }) {
        again = again.replace('|', "\\|");
    }
    let flat = md.replace('\n', " ");
    let same = again == flat
        // After `### `, `- ` or `| ` the text starts no block: `1. x`
        // needs no `1\. x`. A paragraph does need it.
        || matches!(block.kind, Kind::Heading(_) | Kind::Item { .. } | Kind::Cell { .. }) && {
            let digits = again.chars().take_while(char::is_ascii_digit).count();
            again[digits..].starts_with('\\') && {
                again.remove(digits);
                again == flat
            }
        };
    same && !md.contains("  \n")
}

/// Whether a level-2 heading with this text names the chapter.
pub fn chapter_heading(text: &str) -> bool {
    crate::arc42_file::heading_number(&format!("## {text}")).is_some()
}

/// Inline Markdown whose source line breaks are spaces, as the reader
/// shows them; an edited block is written on one line.
fn one_line(md: &str) -> Inline {
    let mut inline = Inline::parse(md);
    // Same length: the marks keep their bytes.
    inline.text = inline.text.replace('\n', " ");
    inline
}

/// Whether a block is written: empty text blocks are dropped (an empty
/// code block stays).
pub fn written(b: &Block) -> bool {
    matches!(
        b.kind,
        Kind::Code { .. } | Kind::Cell { .. } | Kind::Diagram { .. }
    ) || !b.inline.text.trim().is_empty()
}

/// Each list entry's depth as written (at most one deeper than the entry
/// before) and its number: an ordered entry after one of its list counts
/// on from it. Other blocks end the list; blocks not written do not.
pub fn list_entries(blocks: &[Block]) -> Vec<Option<(usize, Option<u64>)>> {
    let mut numbers: Vec<Option<u64>> = Vec::new();
    blocks
        .iter()
        .map(|b| {
            let Kind::Item { depth, number } = b.kind else {
                if written(b) {
                    numbers.clear();
                }
                return None;
            };
            let depth = depth.min(numbers.len());
            numbers.truncate(depth + 1);
            let number = match (number, numbers.get(depth)) {
                (Some(_), Some(Some(last))) => Some(last + 1),
                (n, _) => n,
            };
            numbers.resize(depth, None);
            numbers.push(number);
            Some((depth, number))
        })
        .collect()
}

/// The blocks of the table holding cell `b`: the cells around it up to
/// the next table's first header cell.
pub fn table_range(blocks: &[Block], b: usize) -> Range<usize> {
    let cell = |i: usize| matches!(blocks[i].kind, Kind::Cell { .. });
    let first = |i: usize| matches!(blocks[i].kind, Kind::Cell { row: 0, col: 0, .. });
    let mut start = b;
    while start > 0 && !first(start) && cell(start - 1) {
        start -= 1;
    }
    let mut end = b + 1;
    while end < blocks.len() && cell(end) && !first(end) {
        end += 1;
    }
    start..end
}

/// The `| --- | ---: |` line under a header row of `header` cells.
fn separator(header: &[Block]) -> String {
    let mut out = String::from("|");
    for b in header {
        out.push_str(match b.kind {
            Kind::Cell {
                align: Align::Center,
                ..
            } => " :---: |",
            Kind::Cell {
                align: Align::Right,
                ..
            } => " ---: |",
            _ => " --- |",
        });
    }
    out
}

/// What `write` puts between a block of kind `prev` and one of `kind`.
fn between(prev: Option<&Kind>, kind: &Kind) -> &'static str {
    match (prev, kind) {
        (None, _) => "",
        (Some(Kind::Item { .. }), Kind::Item { .. }) => "\n",
        (Some(Kind::Quote), Kind::Quote) => "\n>\n",
        // A table's rows; a cell after another in its row. A header cell
        // after a cell starts the next table.
        (Some(Kind::Cell { .. }), Kind::Cell { row: 0, col: 0, .. }) => "\n\n",
        (Some(Kind::Cell { .. }), Kind::Cell { col: 0, .. }) => "\n",
        (Some(Kind::Cell { .. }), Kind::Cell { .. }) => "",
        _ => "\n\n",
    }
}

/// The Markdown of edited blocks; empty text blocks are dropped.
fn write(blocks: &[Block]) -> String {
    let mut out = String::new();
    // Per list depth: the marker width, for the indent below it.
    let mut widths: Vec<usize> = Vec::new();
    let mut prev: Option<&Kind> = None;
    for (i, (b, entry)) in blocks.iter().zip(list_entries(blocks)).enumerate() {
        if !written(b) {
            continue;
        }
        out.push_str(between(prev, &b.kind));
        let text = b.inline.to_markdown();
        match &b.kind {
            Kind::Paragraph => out.push_str(&text),
            Kind::Heading(level) => {
                out.push_str(&"#".repeat((*level).clamp(2, 6)));
                out.push(' ');
                out.push_str(&text);
            }
            Kind::Item { .. } => {
                let (depth, number) = entry.expect("an item has an entry");
                widths.truncate(depth);
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
            Kind::Cell { row, col, cols, .. } => {
                if *col == 0 {
                    out.push('|');
                }
                // shortcut: a `|` inside inline code is written as `\|` and
                // reads back with the backslash; split the code span if it matters.
                out.push(' ');
                out.push_str(&text.replace('|', "\\|"));
                out.push_str(" |");
                if *row == 0 && col + 1 == *cols {
                    out.push('\n');
                    out.push_str(&separator(&blocks[i + 1 - cols..=i]));
                }
            }
            Kind::Diagram { source, .. } => out.push_str(source),
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

    fn chapter_five(md: &str) -> Vec<Range<usize>> {
        let start = md.find("<!-- archguard:arc42:05:start").unwrap();
        let end = md.find("<!-- archguard:arc42:05:end").unwrap();
        std::iter::once(start..end).collect()
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
        let doc = Doc::parse(CHAPTER, &chapter_five(CHAPTER));
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
                let parsed = Doc::parse(&md, &generated(&md));
                assert_eq!(parsed.to_markdown(), md, "{project} chapter {n}");
            }
        }
    }

    /// An unclosed code fence before a diagram block reaches into the
    /// block; the text stays as written instead of crashing the view.
    #[test]
    fn an_unclosed_fence_before_a_diagram_block_writes_back_unchanged() {
        let block = diagram_block("calls", "### Aufrufe\n\n```mermaid\nflowchart LR\n```");
        let md = format!("Text.\n\n```\noffen\n\n{block}\n\nDanach.\n");
        assert_eq!(Doc::parse(&md, &[]).to_markdown(), md);
    }

    #[test]
    fn heading_generated_blocks_tables_and_rules_stay_fixed() {
        let mut doc = Doc::parse(CHAPTER, &chapter_five(CHAPTER));
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
        let mut doc = Doc::parse(CHAPTER, &chapter_five(CHAPTER));
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
        let mut doc = Doc::parse(CHAPTER, &chapter_five(CHAPTER));
        for part in parts(&mut doc) {
            part.edit();
        }
        let md = doc.to_markdown();
        let mut again = Doc::parse(&md, &chapter_five(&md));
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

    /// Markdown the editor does not model (reference links and their
    /// definitions, footnotes, link titles, hard breaks) is not written
    /// again when another paragraph is edited.
    #[test]
    fn text_the_editor_does_not_model_survives_an_edit_elsewhere() {
        let kept = [
            "Siehe [ADR][adr] und die Fußnote[^1].",
            "[adr]: https://example.org/adr \"ADR\"",
            "[^1]: Eine Fußnote.",
            "Ein [Link](https://example.org \"Titel\").",
            "Zeile eins  \nZeile zwei",
        ];
        let md = format!("Erster Absatz.\n\n{}\n", kept.join("\n\n"));
        let mut doc = Doc::parse(&md, &[]);
        assert_eq!(doc.to_markdown(), md, "untouched");
        parts(&mut doc)[0].edit()[0].inline = Inline::plain("Neu.");
        let written = doc.to_markdown();
        assert!(written.starts_with("Neu.\n\n"), "{written}");
        assert!(
            written.ends_with(&format!("{}\n", kept.join("\n\n"))),
            "{written}"
        );
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
    fn a_diagram_block_is_one_block_and_writes_back_as_written() {
        let block = diagram_block(
            "runtime-calls",
            "### Aufrufe\n\nQuelle.\n\n```mermaid\nflowchart LR\n```\n\n",
        );
        let md = format!("Text davor.\n\n{block}\n\nText danach.\n");
        let mut doc = Doc::parse(&md, &[]);
        assert_eq!(doc.to_markdown(), md, "untouched");
        let mut all = parts(&mut doc);
        let blocks = all[0].edit();
        assert_eq!(blocks.len(), 3, "{blocks:?}");
        assert!(matches!(&blocks[1].kind, Kind::Diagram { id, .. } if id == "runtime-calls"));
        assert_eq!(
            blocks[1].inline.text, "",
            "a diagram has no text to type in"
        );
        blocks.swap(0, 1);
        assert_eq!(
            doc.to_markdown(),
            format!("{block}\n\nText davor.\n\nText danach.\n"),
            "moved as a whole"
        );
        let inner = blocks_of("### Aufrufe\n\nQuelle.\n\n```mermaid\nflowchart LR\n```\n");
        assert_eq!(inner.len(), 3);
        assert!(matches!(&inner[2].kind, Kind::Code { lang } if lang == "mermaid"));
        assert!(diagram_ranges("<!-- archguard:diagram:x:start -->\nohne Ende").is_empty());
    }

    #[test]
    fn tables_are_cells_and_write_back_as_tables() {
        let md = "| Begriff | Bedeutung |\n| --- | ---: |\n| SCC | Gruppe **gegenseitig** erreichbarer Knoten |\n| A\\|B | |\n";
        let mut doc = Doc::parse(md, &[]);
        assert_eq!(doc.to_markdown(), md, "untouched");
        let mut all = parts(&mut doc);
        let cells = all[0].edit();
        assert_eq!(cells.len(), 6);
        assert_eq!(
            cells[3].kind,
            Kind::Cell {
                row: 1,
                col: 1,
                cols: 2,
                align: Align::Right
            }
        );
        assert_eq!(cells[4].inline.text, "A|B");
        cells[5].inline = Inline::plain("neu");
        let written = doc.to_markdown();
        assert_eq!(
            written,
            "| Begriff | Bedeutung |\n| --- | ---: |\n| SCC | Gruppe **gegenseitig** erreichbarer Knoten |\n| A\\|B | neu |\n"
        );
        assert_eq!(Doc::parse(&written, &[]).to_markdown(), written);
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

    /// Only `## N.` names the chapter: numbered sub-headings are own text
    /// to edit, with the text below them.
    #[test]
    fn numbered_sub_headings_stay_editable() {
        let md = "## 5. Bausteinsicht\n\n### 5.1 Whitebox Gesamtsystem\nText eins.\n\n### 1. Schritt\n\nText zwei.\n";
        let mut doc = Doc::parse(md, &[]);
        let blocks: Vec<(Kind, String)> = parts(&mut doc)
            .iter()
            .flat_map(|p| p.blocks().iter())
            .map(|b| (b.kind.clone(), b.inline.text.clone()))
            .collect();
        assert_eq!(
            blocks,
            [
                (Kind::Heading(3), "5.1 Whitebox Gesamtsystem".into()),
                (Kind::Paragraph, "Text eins.".into()),
                (Kind::Heading(3), "1. Schritt".into()),
                (Kind::Paragraph, "Text zwei.".into()),
            ]
        );
        assert_eq!(doc.to_markdown(), md);
        for part in parts(&mut doc) {
            part.edit();
        }
        let written = doc.to_markdown();
        let mut again = Doc::parse(&written, &[]);
        let texts = |d: &mut Doc| -> Vec<Vec<Block>> {
            parts(d).iter().map(|p| p.blocks().to_vec()).collect()
        };
        assert_eq!(texts(&mut again), texts(&mut doc), "{written}");
    }

    /// `## 5.1` is a section of the chapter, not its heading: own text.
    #[test]
    fn a_level_two_numbered_section_is_editable() {
        let md = "## 5. Bausteinsicht\n\n## 5.1 Whitebox\n";
        let mut doc = Doc::parse(md, &[]);
        let all = parts(&mut doc);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].blocks()[0].kind, Kind::Heading(2));
        assert_eq!(all[0].blocks()[0].inline.text, "5.1 Whitebox");
    }

    /// List entries and cells may start with `1.` as written: the editor
    /// writes `1\.` but the block reads the same. A paragraph needs the
    /// escape (else it is a list) and is written with it.
    #[test]
    fn numbered_entries_and_cells_stay_editable() {
        let own = "- 1. Schritt\n* 2. Punkt\n\n| Nr | Was |\n| --- | --- |\n| 1. a | b |\n\n1990\\. war ein Jahr.\n";
        let md = format!("Vorne.\n\n---\n\n{own}");
        let mut doc = Doc::parse(&md, &[]);
        assert_eq!(doc.to_markdown(), md);
        let mut all = parts(&mut doc);
        assert_eq!(all.len(), 2);
        // Two entries, four cells, the paragraph.
        assert_eq!(all[1].blocks().len(), 7, "{:?}", all[1].blocks());
        all[0].edit()[0].inline = Inline::plain("Neu.");
        assert_eq!(doc.to_markdown(), format!("Neu.\n\n---\n\n{own}"));
        parts(&mut doc)[1].edit();
        let written = doc.to_markdown();
        assert!(written.contains("- 1\\. Schritt\n- 2\\. Punkt"), "{written}");
        assert!(written.contains("\n\n1990\\. war ein Jahr."), "{written}");
        let mut again = Doc::parse(&written, &[]);
        let blocks = |d: &mut Doc| -> Vec<Block> {
            parts(d).iter().flat_map(|p| p.blocks().to_vec()).collect()
        };
        assert_eq!(blocks(&mut again), blocks(&mut doc), "{written}");
    }

    /// A quote without text (comments are no text) is no part without
    /// blocks: the editor could not put its caret there.
    #[test]
    fn a_quote_without_text_stays_as_written() {
        for md in ["---\n\n>\n\n---\n", "> <!-- Hinweis -->\n", "Text.\n\n>\n"] {
            let mut doc = Doc::parse(md, &[]);
            assert!(
                parts(&mut doc).iter().all(|p| !p.blocks().is_empty()),
                "{md}"
            );
            assert_eq!(doc.to_markdown(), md);
        }
    }

    /// Edit the first part's first block; every other byte stays.
    fn edit_first(md: &str) -> String {
        let mut doc = Doc::parse(md, &[]);
        assert_eq!(doc.to_markdown(), md, "untouched");
        parts(&mut doc)[0].edit()[0].inline = Inline::plain("Neu.");
        doc.to_markdown()
    }

    /// The reader drops comments; a block with a comment beside its text
    /// stays as written, else an edit elsewhere in its part deletes it.
    #[test]
    fn comments_beside_text_survive_an_edit_elsewhere() {
        let kept = [
            "Ein Satz. <!-- TODO Quelle -->",
            "- a <!-- x -->",
            "### Titel <!-- {#a} -->",
        ];
        let md = format!("Erster Absatz.\n\n{}\n\nLetzter.\n", kept.join("\n\n"));
        assert_eq!(edit_first(&md), md.replacen("Erster Absatz.", "Neu.", 1));
        // The end of a comment begun on the line before stays too.
        let md = "<!-- a\nb --> Text\n\nLetzter.\n";
        let mut doc = Doc::parse(md, &[]);
        let mut all = parts(&mut doc);
        let last = all.last_mut().unwrap().edit().last_mut().unwrap();
        last.inline = Inline::plain("Neu.");
        assert_eq!(doc.to_markdown(), md.replace("Letzter.", "Neu."));
    }

    /// Code, setext headings and HTML the reader takes for a paragraph
    /// (or a list) stay as written, not flattened onto one line.
    #[test]
    fn code_and_html_read_as_paragraphs_survive_an_edit_elsewhere() {
        let kept = [
            "~~~\nfn main() {}\n~~~",
            "    eingerückter\n    Code",
            "\tmit Tab",
            "Titel\n=====",
            "Titel\n--",
            "<pre>\nZeile\n</pre>",
            "dienste:\n    - web\n    - db",
        ];
        let md = format!("Erster Absatz.\n\n{}\n", kept.join("\n\n"));
        assert_eq!(edit_first(&md), md.replacen("Erster Absatz.", "Neu.", 1));
    }

    /// Two tables with a blank line between are one part; edited, they
    /// are still written as two tables.
    #[test]
    fn two_tables_in_a_part_stay_two() {
        let md = "| a | b |\n| --- | --- |\n| 1 | 2 |\n\n| c | d |\n| --- | --- |\n| 3 | 4 |\n";
        assert_eq!(edit_first(md), md.replacen("| a |", "| Neu. |", 1));
    }

    /// The reader drops cells beyond the header's; such a table stays as
    /// written.
    #[test]
    fn a_row_wider_than_its_header_survives_an_edit_elsewhere() {
        let md = "Erster Absatz.\n\n| a | b |\n| --- | --- |\n| 1 | 2 | 3 |\n";
        assert_eq!(edit_first(md), md.replacen("Erster Absatz.", "Neu.", 1));
    }
}
