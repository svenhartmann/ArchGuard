//! Where the formatted editor's caret is in the chapter's Markdown, and
//! back: the Markdown field opens at the caret and the formatted editor
//! comes back at the field's cursor.

use super::rich::doc::{Block, Doc, Kind, Piece};
use super::rich::editor::Caret;

/// A private-use character: in no chapter, never escaped.
const MARK: char = '\u{E000}';

/// The byte of `md`, the Markdown of `doc`, where caret `c` is.
pub(super) fn offset(doc: &Doc, md: &str, c: Caret) -> usize {
    let starts = starts(doc, md);
    let here = |x: &Caret| (x.piece, x.block);
    match starts.iter().find(|(x, ..)| here(x) == here(&c)) {
        Some((_, start, _, raw)) => start + within(block(doc, c), c.at, *raw),
        // shortcut: a block not found as written (a quote over several
        // lines) gives the end of the block before; find it by its lines if it matters.
        None => starts
            .iter()
            .rev()
            .find(|(x, ..)| here(x) < here(&c))
            .map_or(0, |(_, start, len, _)| start + len),
    }
}

/// Where byte `at` of block `b`'s text is in its Markdown: as the writer
/// puts it, or as is in a kept source found by its plain text (`raw`).
fn within(b: &Block, at: usize, raw: bool) -> usize {
    if raw { at } else { written(b, at).1 }
}

/// The caret at byte `offset` of `md`, the Markdown of `doc`: in the
/// block whose text holds it, else in the last block before it.
pub(super) fn caret(doc: &Doc, md: &str, offset: usize) -> Option<Caret> {
    let starts = starts(doc, md);
    let (c, start, _, raw) = starts
        .iter()
        .find(|(_, s, len, _)| (*s..=s + len).contains(&offset))
        .or_else(|| starts.iter().rev().find(|(_, s, ..)| *s <= offset))
        .or(starts.first())?;
    let text = &block(doc, *c).inline.text;
    let at = (0..=text.len())
        .filter(|&i| text.is_char_boundary(i))
        .take_while(|&i| start + within(block(doc, *c), i, *raw) <= offset)
        .last()
        .unwrap_or(0);
    Some(Caret { at, ..*c })
}

fn block(doc: &Doc, c: Caret) -> &Block {
    &doc.part(c.piece).expect("a caret is in a part").blocks()[c.block]
}

/// Where block `b` starts in `rest`, its length, and whether it was found
/// by its plain text rather than as the writer puts it.
fn find(rest: &str, b: &Block) -> Option<(usize, usize, bool)> {
    let forms = [
        (written(b, 0).0, false),
        (b.inline.text.replace('\n', " "), true),
    ];
    forms
        .into_iter()
        .find_map(|(text, raw)| rest.find(&text).map(|i| (i, text.len(), raw)))
}

/// Each block found in `md`, the Markdown of `doc`, in order: its caret
/// at 0, where its written text starts, its length, and whether it was
/// found by its plain text (an untouched block keeps a source the writer
/// would escape differently, e.g. `_id`).
fn starts(doc: &Doc, md: &str) -> Vec<(Caret, usize, usize, bool)> {
    // Line breaks as spaces: a block read from several lines writes one.
    let flat = md.replace('\n', " ");
    let (mut out, mut pos, mut piece_start) = (Vec::new(), 0, 0);
    for (piece, p) in doc.pieces.iter().enumerate() {
        if let Piece::Part(part) = p {
            pos = pos.max(piece_start);
            for (block, b) in part.blocks().iter().enumerate() {
                let rest = flat.get(pos..).unwrap_or_default();
                let Some((i, len, raw)) = find(rest, b) else {
                    continue;
                };
                out.push((
                    Caret {
                        piece,
                        block,
                        at: 0,
                    },
                    pos + i,
                    len,
                    raw,
                ));
                pos += i + len;
            }
        }
        // A new part's line breaks before it are not counted: the search goes on.
        piece_start += doc.piece_markdown(piece).len();
    }
    out
}

/// Block `b`'s text as the Markdown writes it, line breaks as spaces,
/// and where byte `at` of its text is in it.
fn written(b: &Block, at: usize) -> (String, usize) {
    let write = |at: Option<usize>| {
        let mut inline = b.inline.clone();
        if let Some(at) = at {
            let marks = inline.typing(at);
            inline.insert(at, &MARK.to_string(), &marks);
        }
        let md = match &b.kind {
            Kind::Code { .. } => inline.text,
            Kind::Diagram { source, .. } => source.clone(),
            Kind::Cell { .. } => inline.to_markdown().replace('|', "\\|"),
            _ => inline.to_markdown(),
        };
        md.replace('\n', " ")
    };
    let text = write(None);
    // A mark may change how the text around it is escaped: at most the end.
    let k = write(Some(at)).find(MARK).unwrap_or(0).min(text.len());
    (text, k)
}

#[cfg(test)]
mod tests {
    use super::super::rich::doc;
    use super::*;

    const MD: &str = "\
## 5. Title

<!-- archguard:arc42:05:start -->
Generiert.
<!-- archguard:arc42:05:end -->

Eins **fett** zwei
und weiter.

- a
- b *c*

| K | V |
| --- | --- |
| x | y\\|z |

```rust
let a = 1;
```
";

    fn parse(md: &str) -> Doc {
        Doc::parse(md, &doc::generated(md))
    }

    fn at(piece: usize, block: usize, at: usize) -> Caret {
        Caret { piece, block, at }
    }

    #[test]
    fn a_caret_is_at_its_text_in_the_markdown() {
        let d = parse(MD);
        let part = (0..d.pieces.len()).find(|&p| d.part(p).is_some()).unwrap();
        let before = |c| &MD[..offset(&d, MD, c)];
        assert!(before(at(part, 0, 0)).ends_with("end -->\n\n"));
        // After "fett", bold: inside its marks; a line break of the source.
        assert!(before(at(part, 0, 9)).ends_with("Eins **fett"));
        assert!(before(at(part, 0, 16)).ends_with("zwei\nu"));
        assert!(before(at(part, 2, 3)).ends_with("- b *c"));
        assert!(before(at(part, 6, 1)).ends_with("| y"));
        assert!(before(at(part, 7, 4)).ends_with("```rust\nlet "));
    }

    #[test]
    fn an_offset_is_in_the_block_that_holds_it() {
        let d = parse(MD);
        let part = (0..d.pieces.len()).find(|&p| d.part(p).is_some()).unwrap();
        let of = |s: &str| MD.find(s).unwrap();
        assert_eq!(caret(&d, MD, of("ett**")), Some(at(part, 0, 6)));
        // Between blocks: the end of the one before; in fixed text and
        // before every block: the first.
        assert_eq!(caret(&d, MD, of("\n\n- a") + 1), Some(at(part, 0, 26)));
        assert_eq!(caret(&d, MD, of("Generiert")), Some(at(part, 0, 0)));
        assert_eq!(caret(&d, MD, MD.len()), Some(at(part, 7, 10)));
    }

    /// An untouched block keeps its source, which the writer would escape
    /// (`_id` as `\_id`): the caret still lands in it, not before it.
    #[test]
    fn a_kept_block_the_writer_would_escape_is_found() {
        let md = "Erster Absatz.\n\nDie Spalte _id ist neu.\n";
        let d = parse(md);
        let part = (0..d.pieces.len()).find(|&p| d.part(p).is_some()).unwrap();
        let c = at(part, 1, 12);
        assert!(md[..offset(&d, md, c)].ends_with("Die Spalte _"), "{md}");
        assert_eq!(caret(&d, md, offset(&d, md, c)), Some(c));
    }

    #[test]
    fn every_caret_comes_back_from_its_offset() {
        let read = parse(MD);
        let part = (0..read.pieces.len())
            .find(|&p| read.part(p).is_some())
            .unwrap();
        // An edited part writes its blocks, not its source.
        let mut edited = read.clone();
        edited.part_mut(part).unwrap().edit();
        for doc in [read, edited] {
            let md = doc.to_markdown();
            for (block, b) in doc.part(part).unwrap().blocks().iter().enumerate() {
                let text = &b.inline.text;
                for i in (0..=text.len()).filter(|&i| text.is_char_boundary(i)) {
                    let c = at(part, block, i);
                    assert_eq!(caret(&doc, &md, offset(&doc, &md, c)), Some(c), "{md}");
                }
            }
        }
    }
}
