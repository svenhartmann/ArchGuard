//! The editing state of the rich-text editor: a caret and a selection in
//! the chapter's editable blocks, the edits keys and toolbar buttons make,
//! and undo. Positions are bytes of a block's text; what lies on which
//! screen line is the layout's business (moving up and down, Home, End).

use std::collections::BTreeSet;

use super::super::markdown::Align;
use super::doc::{Block, Doc, Kind};
use super::inline::{Inline, Mark};

/// A position: the part's piece in the chapter, the block in the part and
/// the byte in the block's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Caret {
    pub piece: usize,
    pub block: usize,
    pub at: usize,
}

#[derive(Debug, Clone)]
struct State {
    doc: Doc,
    anchor: Caret,
    focus: Caret,
}

#[derive(Debug, Clone)]
pub struct Editor {
    doc: Doc,
    /// Where the selection started; the caret is `focus`.
    pub anchor: Caret,
    pub focus: Caret,
    /// Marks the next typed text gets (a toggle without a selection);
    /// `None`: those of the text before the caret.
    pending: Option<BTreeSet<Mark>>,
    undo: Vec<State>,
    redo: Vec<State>,
    /// The last edit was typing: more typing joins its undo step.
    typing: bool,
}

/// Steps undo keeps.
const UNDO: usize = 200;

impl Editor {
    /// Edit `doc`. A chapter that ends in text it keeps as written (a
    /// generated block, a table) gets an empty part after it to type in;
    /// it writes nothing while empty.
    pub fn new(doc: Doc) -> Editor {
        let doc = with_trailing_part(doc);
        let piece = (0..doc.pieces.len())
            .find(|&p| doc.part(p).is_some())
            .expect("a part exists");
        let caret = Caret {
            piece,
            block: 0,
            at: 0,
        };
        Editor {
            doc,
            anchor: caret,
            focus: caret,
            pending: None,
            undo: Vec::new(),
            redo: Vec::new(),
            typing: false,
        }
    }

    pub fn doc(&self) -> &Doc {
        &self.doc
    }

    pub fn markdown(&self) -> String {
        self.doc.to_markdown()
    }

    pub fn block(&self, c: Caret) -> &Block {
        &self
            .doc
            .part(c.piece)
            .expect("a caret is in a part")
            .blocks()[c.block]
    }

    fn blocks(&self, piece: usize) -> &[Block] {
        self.doc.part(piece).expect("a caret is in a part").blocks()
    }

    fn blocks_mut(&mut self, piece: usize) -> &mut Vec<Block> {
        self.doc
            .part_mut(piece)
            .expect("a caret is in a part")
            .edit()
    }

    /// The selection in reading order; `None` when it is empty.
    pub fn selection(&self) -> Option<(Caret, Caret)> {
        (self.anchor != self.focus)
            .then(|| (self.anchor.min(self.focus), self.anchor.max(self.focus)))
    }

    /// Put the caret at `c` (a click), extending the selection with `extend`.
    /// A selection stays inside one part.
    pub fn place(&mut self, c: Caret, extend: bool) {
        let mut c = self.clamp(c);
        if extend && c.piece != self.anchor.piece {
            let blocks = self.blocks(self.anchor.piece);
            c = if c.piece > self.anchor.piece {
                let block = blocks.len() - 1;
                Caret {
                    piece: self.anchor.piece,
                    block,
                    at: blocks[block].inline.text.len(),
                }
            } else {
                Caret {
                    piece: self.anchor.piece,
                    block: 0,
                    at: 0,
                }
            };
        }
        self.focus = c;
        if !extend {
            self.anchor = c;
        }
        self.pending = None;
        self.typing = false;
    }

    /// `c` inside its block's text, on a character boundary.
    fn clamp(&self, mut c: Caret) -> Caret {
        let blocks = self.blocks(c.piece);
        c.block = c.block.min(blocks.len() - 1);
        let text = &blocks[c.block].inline.text;
        c.at = c.at.min(text.len());
        while !text.is_char_boundary(c.at) {
            c.at -= 1;
        }
        c
    }

    fn checkpoint(&mut self, typing: bool) {
        if !(typing && self.typing) {
            self.undo.push(State {
                doc: self.doc.clone(),
                anchor: self.anchor,
                focus: self.focus,
            });
            if self.undo.len() > UNDO {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
        self.typing = typing;
    }

    pub fn undo(&mut self) {
        self.swap(true);
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// The link target at the selection's start or the caret, if any.
    pub fn link_target(&self) -> Option<String> {
        let c = self.selection().map_or(self.focus, |(start, _)| start);
        let inline = &self.block(c).inline;
        inline.spans.iter().find_map(|s| match &s.mark {
            Mark::Link(t) if s.range.start <= c.at && c.at <= s.range.end => Some(t.clone()),
            _ => None,
        })
    }

    pub fn redo(&mut self) {
        self.swap(false);
    }

    fn swap(&mut self, back: bool) {
        let (from, to) = if back {
            (&mut self.undo, &mut self.redo)
        } else {
            (&mut self.redo, &mut self.undo)
        };
        let Some(state) = from.pop() else { return };
        to.push(State {
            doc: std::mem::replace(&mut self.doc, state.doc),
            anchor: self.anchor,
            focus: self.focus,
        });
        (self.anchor, self.focus) = (state.anchor, state.focus);
        self.typing = false;
        self.pending = None;
    }

    fn collapse(&mut self, c: Caret) {
        self.anchor = c;
        self.focus = c;
    }

    /// Remove the selected text; the caret where it started.
    fn delete_selection(&mut self) -> bool {
        let Some((start, end)) = self.selection() else {
            return false;
        };
        let cells = (start.block..=end.block).any(|b| self.is_cell(start.piece, b));
        if cells && start.block != end.block {
            // Table cells keep their places: only their text goes.
            for (b, r) in self.ranges(start, end) {
                self.blocks_mut(start.piece)[b].inline.delete(r);
            }
            self.collapse(start);
            return true;
        }
        let blocks = self.blocks_mut(start.piece);
        if start.block == end.block {
            blocks[start.block].inline.delete(start.at..end.at);
        } else {
            let mut last = blocks[end.block].inline.clone();
            let tail = last.split_off(end.at);
            let first = &mut blocks[start.block].inline;
            first.split_off(start.at);
            first.append(tail);
            blocks.drain(start.block + 1..=end.block);
        }
        self.collapse(start);
        true
    }

    /// Type `text`: a new line in a code block, else a new block per line.
    pub fn type_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.checkpoint(!text.contains('\n') && self.selection().is_none());
        self.delete_selection();
        let code = matches!(self.block(self.focus).kind, Kind::Code { .. });
        // A cell holds one line.
        let text = if self.is_cell(self.focus.piece, self.focus.block) {
            text.replace('\n', " ")
        } else {
            text.to_string()
        };
        for (i, line) in text.split('\n').enumerate() {
            if i > 0 && code {
                self.insert("\n");
            } else if i > 0 {
                self.split();
            }
            self.insert(line);
        }
    }

    fn insert(&mut self, s: &str) {
        let c = self.focus;
        let code = matches!(self.block(c).kind, Kind::Code { .. });
        let marks = match self.pending.clone() {
            _ if code => BTreeSet::new(),
            Some(marks) => marks,
            None => self.block(c).inline.typing(c.at),
        };
        self.blocks_mut(c.piece)[c.block]
            .inline
            .insert(c.at, s, &marks);
        self.collapse(Caret {
            at: c.at + s.len(),
            ..c
        });
    }

    /// Enter: a new block after the caret's text; in code a new line; an
    /// empty list entry or quote line becomes a paragraph.
    pub fn enter(&mut self) {
        self.checkpoint(false);
        self.delete_selection();
        let c = self.focus;
        let block = self.block(c);
        match block.kind {
            Kind::Code { .. } => self.insert("\n"),
            Kind::Cell { .. } => self.add_row(),
            Kind::Item { .. } | Kind::Quote if block.inline.text.is_empty() => {
                self.blocks_mut(c.piece)[c.block].kind = Kind::Paragraph;
            }
            _ => self.split(),
        }
        self.pending = None;
    }

    fn split(&mut self) {
        let c = self.focus;
        let blocks = self.blocks_mut(c.piece);
        let rest = blocks[c.block].inline.split_off(c.at);
        let kind = match &blocks[c.block].kind {
            Kind::Heading(_) => Kind::Paragraph,
            Kind::Item { depth, number } => Kind::Item {
                depth: *depth,
                number: number.map(|n| n + 1),
            },
            kind => kind.clone(),
        };
        blocks.insert(c.block + 1, Block { kind, inline: rest });
        self.collapse(Caret {
            block: c.block + 1,
            at: 0,
            ..c
        });
    }

    /// Backspace: the selection, else the character before; at a block's
    /// start a list entry, heading or quote becomes a paragraph first, a
    /// paragraph joins the block before.
    pub fn backspace(&mut self) {
        self.checkpoint(self.selection().is_none() && self.focus.at > 0);
        if self.delete_selection() {
            return;
        }
        let c = self.focus;
        if c.at > 0 {
            let text = &self.block(c).inline.text;
            // shortcut: removes one char, not a grapheme cluster (é as e + ́ takes two).
            let prev = text[..c.at].chars().next_back().map_or(0, char::len_utf8);
            self.blocks_mut(c.piece)[c.block]
                .inline
                .delete(c.at - prev..c.at);
            self.collapse(Caret {
                at: c.at - prev,
                ..c
            });
        } else if self.is_cell(c.piece, c.block) {
            // Cells do not merge; Backspace at a cell's start goes to the one before.
            if c.block > 0 && self.is_cell(c.piece, c.block - 1) {
                let at = self.blocks(c.piece)[c.block - 1].inline.text.len();
                self.collapse(Caret {
                    block: c.block - 1,
                    at,
                    ..c
                });
            }
        } else if !matches!(self.block(c).kind, Kind::Paragraph | Kind::Code { .. }) {
            self.blocks_mut(c.piece)[c.block].kind = Kind::Paragraph;
        } else if c.block > 0 && !self.is_cell(c.piece, c.block - 1) {
            self.join(c.piece, c.block - 1);
        }
    }

    /// Delete: the selection, else the character after; at a block's end
    /// the next block joins this one.
    pub fn delete_forward(&mut self) {
        self.checkpoint(false);
        if self.delete_selection() {
            return;
        }
        let c = self.focus;
        let len = self.block(c).inline.text.len();
        if c.at < len {
            let text = &self.block(c).inline.text;
            let next = text[c.at..].chars().next().map_or(0, char::len_utf8);
            self.blocks_mut(c.piece)[c.block]
                .inline
                .delete(c.at..c.at + next);
        } else if c.block + 1 < self.blocks(c.piece).len()
            && !self.is_cell(c.piece, c.block)
            && !self.is_cell(c.piece, c.block + 1)
        {
            self.join(c.piece, c.block);
        }
    }

    fn is_cell(&self, piece: usize, block: usize) -> bool {
        matches!(
            self.blocks(piece).get(block).map(|b| &b.kind),
            Some(Kind::Cell { .. })
        )
    }

    /// The blocks of the table the caret is in, as rows of blocks.
    fn table(&self) -> Option<(std::ops::Range<usize>, Vec<Vec<Block>>)> {
        let c = self.focus;
        if !self.is_cell(c.piece, c.block) {
            return None;
        }
        let blocks = self.blocks(c.piece);
        let start = (0..=c.block)
            .rev()
            .take_while(|&b| self.is_cell(c.piece, b))
            .last()?;
        let end = (c.block..blocks.len())
            .take_while(|&b| self.is_cell(c.piece, b))
            .last()?
            + 1;
        let mut rows: Vec<Vec<Block>> = Vec::new();
        for b in &blocks[start..end] {
            if let Kind::Cell { col: 0, .. } = b.kind {
                rows.push(Vec::new());
            }
            rows.last_mut()?.push(b.clone());
        }
        Some((start..end, rows))
    }

    /// Put `rows` in place of the table at `range`, numbering the cells
    /// anew (alignment by column from the header); the caret to `row`, `col`.
    fn set_table(
        &mut self,
        range: std::ops::Range<usize>,
        rows: Vec<Vec<Block>>,
        row: usize,
        col: usize,
    ) {
        let piece = self.focus.piece;
        let cols = rows.first().map_or(0, Vec::len);
        let aligns: Vec<Align> = rows
            .first()
            .map(|h| {
                h.iter()
                    .map(|b| match b.kind {
                        Kind::Cell { align, .. } => align,
                        _ => Align::Left,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let start = range.start;
        let cells: Vec<Block> = rows
            .into_iter()
            .enumerate()
            .flat_map(|(r, cells)| {
                let aligns = aligns.clone();
                cells.into_iter().enumerate().map(move |(c, b)| Block {
                    kind: Kind::Cell {
                        row: r,
                        col: c,
                        cols,
                        align: aligns.get(c).copied().unwrap_or(Align::Left),
                    },
                    inline: b.inline,
                })
            })
            .collect();
        let count = cells.len();
        self.blocks_mut(piece).splice(range, cells);
        let block = (start + row * cols + col.min(cols.saturating_sub(1)))
            .min(start + count.saturating_sub(1));
        self.collapse(Caret {
            piece,
            block,
            at: 0,
        });
    }

    fn empty_cell() -> Block {
        Block {
            kind: Kind::Paragraph,
            inline: Inline::default(),
        }
    }

    fn cell_at(&self) -> (usize, usize) {
        match self.block(self.focus).kind {
            Kind::Cell { row, col, .. } => (row, col),
            _ => (0, 0),
        }
    }

    /// A new empty row below the caret's (Enter in a cell).
    pub fn add_row(&mut self) {
        let Some((range, mut rows)) = self.table() else {
            return;
        };
        self.checkpoint(false);
        let (row, _) = self.cell_at();
        let cols = rows[0].len();
        rows.insert(row + 1, vec![Self::empty_cell(); cols]);
        self.set_table(range, rows, row + 1, 0);
    }

    /// A new empty column right of the caret's.
    pub fn add_column(&mut self) {
        let Some((range, mut rows)) = self.table() else {
            return;
        };
        self.checkpoint(false);
        let (row, col) = self.cell_at();
        for cells in &mut rows {
            cells.insert(col + 1, Self::empty_cell());
        }
        self.set_table(range, rows, row, col + 1);
    }

    /// Remove the caret's row; the header stays.
    pub fn delete_row(&mut self) {
        let Some((range, mut rows)) = self.table() else {
            return;
        };
        let (row, col) = self.cell_at();
        if row == 0 {
            return;
        }
        self.checkpoint(false);
        rows.remove(row);
        let row = row.min(rows.len() - 1);
        self.set_table(range, rows, row, col);
    }

    /// Remove the caret's column; the last one stays.
    pub fn delete_column(&mut self) {
        let Some((range, mut rows)) = self.table() else {
            return;
        };
        let (row, col) = self.cell_at();
        if rows[0].len() < 2 {
            return;
        }
        self.checkpoint(false);
        for cells in &mut rows {
            cells.remove(col);
        }
        self.set_table(range, rows, row, col.saturating_sub(1));
    }

    /// A two-column table with a header and one row after the caret's block.
    pub fn insert_table(&mut self) {
        let c = self.focus;
        if self.is_cell(c.piece, c.block) {
            return;
        }
        self.checkpoint(false);
        let at = c.block + 1;
        self.blocks_mut(c.piece).insert(at, Self::empty_cell());
        let rows = vec![vec![Self::empty_cell(); 2]; 2];
        self.set_table(at..at + 1, rows, 0, 0);
    }

    /// Tab in a table: the next cell, a new row after the last; Shift+Tab
    /// the cell before. False outside a table.
    pub fn next_cell(&mut self, back: bool) -> bool {
        let c = self.focus;
        if !self.is_cell(c.piece, c.block) {
            return false;
        }
        if back {
            if c.block > 0 && self.is_cell(c.piece, c.block - 1) {
                self.place(
                    Caret {
                        block: c.block - 1,
                        at: 0,
                        ..c
                    },
                    false,
                );
            }
        } else if self.is_cell(c.piece, c.block + 1) {
            self.place(
                Caret {
                    block: c.block + 1,
                    at: 0,
                    ..c
                },
                false,
            );
        } else {
            self.add_row();
        }
        true
    }

    /// Append block `block + 1` to `block`; the caret at the join.
    fn join(&mut self, piece: usize, block: usize) {
        let blocks = self.blocks_mut(piece);
        let next = blocks.remove(block + 1);
        let into = &mut blocks[block];
        let at = into.inline.text.len();
        if matches!(into.kind, Kind::Code { .. }) {
            into.inline.text.push_str(&next.inline.text);
        } else {
            into.inline.append(next.inline);
        }
        self.collapse(Caret { piece, block, at });
    }

    /// Bold, italic or code over the selection, or for the next typed
    /// text: off where the whole selection has it, else on.
    pub fn toggle(&mut self, mark: Mark) {
        let Some((start, end)) = self.selection() else {
            let mut marks = self
                .pending
                .take()
                .unwrap_or_else(|| self.block(self.focus).inline.typing(self.focus.at));
            if !marks.remove(&mark) {
                marks.insert(mark);
            }
            self.pending = Some(marks);
            return;
        };
        self.checkpoint(false);
        let on = !self.ranges(start, end).iter().all(|(b, r)| {
            let block = &self.blocks(start.piece)[*b];
            r.is_empty() || block.inline.has(r.clone(), &mark)
        });
        for (b, r) in self.ranges(start, end) {
            let inline = &mut self.blocks_mut(start.piece)[b].inline;
            if on {
                inline.add(r, mark.clone());
            } else {
                inline.remove(r, &mark);
            }
        }
    }

    /// Link the selection to `target`, or unlink it with `None`.
    pub fn link(&mut self, target: Option<String>) {
        let Some((start, end)) = self.selection() else {
            return;
        };
        self.checkpoint(false);
        for (b, r) in self.ranges(start, end) {
            let inline = &mut self.blocks_mut(start.piece)[b].inline;
            match &target {
                Some(t) => inline.add(r, Mark::Link(t.clone())),
                None => inline.remove(r, &Mark::Link(String::new())),
            }
        }
    }

    /// The selected bytes per block of one part.
    fn ranges(&self, start: Caret, end: Caret) -> Vec<(usize, std::ops::Range<usize>)> {
        (start.block..=end.block)
            .map(|b| {
                let len = self.blocks(start.piece)[b].inline.text.len();
                let from = if b == start.block { start.at } else { 0 };
                let to = if b == end.block { end.at } else { len };
                (b, from..to)
            })
            .collect()
    }

    /// The blocks the selection or caret touches.
    fn touched(&self) -> (usize, std::ops::RangeInclusive<usize>) {
        let (start, end) = self.selection().unwrap_or((self.focus, self.focus));
        (start.piece, start.block..=end.block)
    }

    /// Make the touched blocks `kind`; if they all are, paragraphs again.
    /// Code drops the marks; a list entry keeps its depth.
    pub fn set_kind(&mut self, kind: Kind) {
        self.checkpoint(false);
        let (piece, range) = self.touched();
        let same = |a: &Kind, b: &Kind| match (a, b) {
            (Kind::Item { number: x, .. }, Kind::Item { number: y, .. }) => {
                x.is_some() == y.is_some()
            }
            (Kind::Code { .. }, Kind::Code { .. }) => true,
            (a, b) => a == b,
        };
        let all = range
            .clone()
            .all(|b| same(&self.blocks(piece)[b].kind, &kind));
        for b in range {
            let block = &mut self.blocks_mut(piece)[b];
            if matches!(block.kind, Kind::Cell { .. }) {
                continue;
            }
            block.kind = match (&kind, &block.kind) {
                _ if all => Kind::Paragraph,
                (Kind::Item { number, .. }, Kind::Item { depth, .. }) => Kind::Item {
                    depth: *depth,
                    number: *number,
                },
                (kind, _) => kind.clone(),
            };
            if matches!(block.kind, Kind::Code { .. }) {
                block.inline = Inline::plain(&block.inline.text);
            }
        }
    }

    /// Tab and Shift+Tab: list entries one level deeper or out.
    pub fn indent(&mut self, deeper: bool) {
        self.checkpoint(false);
        let (piece, range) = self.touched();
        for b in range {
            if let Kind::Item { depth, .. } = &mut self.blocks_mut(piece)[b].kind {
                *depth = if deeper {
                    *depth + 1
                } else {
                    depth.saturating_sub(1)
                };
            }
        }
    }

    /// The touched block's kind (for the toolbar).
    pub fn kind(&self) -> &Kind {
        &self.block(self.focus).kind
    }

    /// The marks the toolbar shows as on: the selection's (where all of it
    /// has them), else those the next typed text gets.
    pub fn marks(&self) -> BTreeSet<Mark> {
        let Some((start, end)) = self.selection() else {
            return self
                .pending
                .clone()
                .unwrap_or_else(|| self.block(self.focus).inline.typing(self.focus.at));
        };
        [Mark::Bold, Mark::Italic, Mark::Code]
            .into_iter()
            .filter(|m| {
                self.ranges(start, end).iter().all(|(b, r)| {
                    r.is_empty() || self.blocks(start.piece)[*b].inline.has(r.clone(), m)
                })
            })
            .collect()
    }

    /// One character (or word) left, across blocks and parts.
    pub fn left(&mut self, word: bool, extend: bool) {
        let c = self.focus;
        if !extend && let Some((start, _)) = self.selection() {
            return self.place(start, false);
        }
        let text = &self.block(c).inline.text;
        let to = if c.at > 0 {
            Caret {
                at: if word {
                    word_start(text, c.at)
                } else {
                    c.at - text[..c.at].chars().next_back().map_or(0, char::len_utf8)
                },
                ..c
            }
        } else if let Some(prev) = self.prev_block(c) {
            prev
        } else {
            c
        };
        self.place(to, extend);
    }

    /// One character (or word) right, across blocks and parts.
    pub fn right(&mut self, word: bool, extend: bool) {
        let c = self.focus;
        if !extend && let Some((_, end)) = self.selection() {
            return self.place(end, false);
        }
        let text = &self.block(c).inline.text;
        let to = if c.at < text.len() {
            Caret {
                at: if word {
                    word_end(text, c.at)
                } else {
                    c.at + text[c.at..].chars().next().map_or(0, char::len_utf8)
                },
                ..c
            }
        } else if let Some(next) = self.next_block(c) {
            next
        } else {
            c
        };
        self.place(to, extend);
    }

    /// The end of the block before `c` (in this part or an earlier one).
    pub fn prev_block(&self, c: Caret) -> Option<Caret> {
        if c.block > 0 {
            let block = c.block - 1;
            let at = self.blocks(c.piece)[block].inline.text.len();
            return Some(Caret { block, at, ..c });
        }
        let piece = (0..c.piece).rev().find(|&p| self.doc.part(p).is_some())?;
        let block = self.blocks(piece).len() - 1;
        let at = self.blocks(piece)[block].inline.text.len();
        Some(Caret { piece, block, at })
    }

    /// The start of the block after `c` (in this part or a later one).
    pub fn next_block(&self, c: Caret) -> Option<Caret> {
        if c.block + 1 < self.blocks(c.piece).len() {
            return Some(Caret {
                block: c.block + 1,
                at: 0,
                ..c
            });
        }
        let piece = (c.piece + 1..self.doc.pieces.len()).find(|&p| self.doc.part(p).is_some())?;
        Some(Caret {
            piece,
            block: 0,
            at: 0,
        })
    }

    /// Make the generated blocks in fixed piece `piece` own text: their
    /// `archguard:arc42` markers go, so the analysis no longer refreshes
    /// them and they can be edited. False if the piece has none.
    pub fn take_over(&mut self, piece: usize) -> bool {
        let md = self.doc.to_markdown();
        let start = piece_start(&self.doc, piece);
        let end = start + self.doc.piece_markdown(piece).len();
        let blocks: Vec<std::ops::Range<usize>> = super::doc::generated(&md)
            .into_iter()
            .filter(|g| g.start < end && start < g.end.max(g.start + 1))
            .collect();
        if blocks.is_empty() {
            return false;
        }
        self.checkpoint(false);
        // The marker lines, from the last on, so earlier offsets hold.
        let line = |at: usize| at..md[at..].find('\n').map_or(md.len(), |n| at + n + 1);
        let mut cuts: Vec<std::ops::Range<usize>> = Vec::new();
        for g in blocks {
            cuts.push(line(g.start));
            if md[g.end..].starts_with(crate::arc42_file::MARKER) {
                cuts.push(line(g.end));
            }
        }
        cuts.sort_by_key(|r| std::cmp::Reverse(r.start));
        let mut new = md;
        for r in cuts {
            new.replace_range(r, "");
        }
        self.doc = with_trailing_part(Doc::parse(&new, &super::doc::generated(&new)));
        // The caret in the first part from where the block was.
        let piece = (0..self.doc.pieces.len())
            .find(|&p| self.doc.part(p).is_some() && piece_start(&self.doc, p) + 1 >= start)
            .or_else(|| (0..self.doc.pieces.len()).find(|&p| self.doc.part(p).is_some()))
            .expect("a part exists");
        self.collapse(Caret {
            piece,
            block: 0,
            at: 0,
        });
        true
    }

    /// A click on fixed piece `piece`: the caret to the next part, or the
    /// end of the part before.
    pub fn jump_from_fixed(&mut self, piece: usize) {
        let is_part = |p: &usize| self.doc.part(*p).is_some();
        let target = if let Some(p) = (piece + 1..self.doc.pieces.len()).find(is_part) {
            Caret {
                piece: p,
                block: 0,
                at: 0,
            }
        } else if let Some(p) = (0..piece).rev().find(is_part) {
            let block = self.blocks(p).len() - 1;
            let at = self.blocks(p)[block].inline.text.len();
            Caret {
                piece: p,
                block,
                at,
            }
        } else {
            return;
        };
        self.place(target, false);
    }

    /// Whether the caret is in a table.
    pub fn in_table(&self) -> bool {
        self.is_cell(self.focus.piece, self.focus.block)
    }

    /// Select the word at `c` (a double click): the text between spaces.
    pub fn select_word(&mut self, c: Caret) {
        let c = self.clamp(c);
        let text = &self.block(c).inline.text;
        let start = text[..c.at]
            .char_indices()
            .rev()
            .find(|(_, ch)| ch.is_whitespace())
            .map_or(0, |(i, ch)| i + ch.len_utf8());
        let end = text[c.at..]
            .find(char::is_whitespace)
            .map_or(text.len(), |i| c.at + i);
        self.place(Caret { at: start, ..c }, false);
        self.place(Caret { at: end, ..c }, true);
    }

    /// Select the whole part the caret is in.
    pub fn select_all(&mut self) {
        let piece = self.focus.piece;
        let blocks = self.blocks(piece);
        let block = blocks.len() - 1;
        let at = blocks[block].inline.text.len();
        self.anchor = Caret {
            piece,
            block: 0,
            at: 0,
        };
        self.focus = Caret { piece, block, at };
    }

    /// The selected text, lines per block (for the clipboard).
    pub fn selected_text(&self) -> String {
        let Some((start, end)) = self.selection() else {
            return String::new();
        };
        self.ranges(start, end)
            .into_iter()
            .map(|(b, r)| self.blocks(start.piece)[b].inline.text[r].to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// `doc` with an empty part at its end if it ends in fixed text.
fn with_trailing_part(mut doc: Doc) -> Doc {
    let last = doc.pieces.iter().rposition(|p| match p {
        super::doc::Piece::Fixed(text) => !text.trim().is_empty(),
        super::doc::Piece::Part(_) => true,
    });
    if last.is_none_or(|i| doc.part(i).is_none()) {
        doc.append_part();
    }
    doc
}

/// The byte where piece `i` starts in the Markdown `doc` writes.
fn piece_start(doc: &Doc, i: usize) -> usize {
    (0..i).map(|p| doc.piece_markdown(p).len()).sum()
}

fn word_start(text: &str, at: usize) -> usize {
    let before = &text[..at];
    let trimmed = before.trim_end();
    trimmed
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map_or(0, |(i, c)| i + c.len_utf8())
}

fn word_end(text: &str, at: usize) -> usize {
    let after = &text[at..];
    let skip = after.len() - after.trim_start().len();
    after[skip..]
        .char_indices()
        .find(|(_, c)| c.is_whitespace())
        .map_or(text.len(), |(i, _)| at + skip + i)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor(md: &str) -> Editor {
        Editor::new(Doc::parse(md, &[]))
    }

    fn at(e: &Editor, block: usize, at: usize) -> Caret {
        Caret {
            piece: e.focus.piece,
            block,
            at,
        }
    }

    #[test]
    fn typing_goes_on_in_the_marks_before_the_caret() {
        let mut e = editor("ein **fetter** Satz\n");
        e.place(at(&e, 0, 10), false);
        e.type_text("er");
        assert_eq!(e.markdown(), "ein **fetterer** Satz\n");
        e.toggle(Mark::Bold);
        e.type_text(" normal");
        assert_eq!(e.markdown(), "ein **fetterer** normal Satz\n");
        e.toggle(Mark::Italic);
        assert_eq!(e.marks(), [Mark::Italic].into());
        e.type_text("!");
        assert_eq!(e.markdown(), "ein **fetterer** normal*!* Satz\n");
    }

    #[test]
    fn enter_splits_and_backspace_joins() {
        let mut e = editor("# Titel\n\nAbsatz eins\n");
        e.place(at(&e, 1, 6), false);
        e.enter();
        assert_eq!(e.markdown(), "## Titel\n\nAbsatz\n\n eins\n");
        e.backspace();
        assert_eq!(e.markdown(), "## Titel\n\nAbsatz eins\n");
        e.place(at(&e, 0, 5), false);
        e.enter();
        e.type_text("Text");
        assert_eq!(
            e.markdown(),
            "## Titel\n\nText\n\nAbsatz eins\n",
            "a heading is followed by text"
        );
    }

    #[test]
    fn lists_continue_on_enter_and_end_on_an_empty_entry() {
        let mut e = editor("- eins\n");
        e.place(at(&e, 0, 4), false);
        e.enter();
        e.type_text("zwei");
        assert_eq!(e.markdown(), "- eins\n- zwei\n");
        e.indent(true);
        assert_eq!(e.markdown(), "- eins\n  - zwei\n");
        e.enter();
        e.enter();
        e.type_text("danach");
        assert_eq!(e.markdown(), "- eins\n  - zwei\n\ndanach\n");
        e.place(at(&e, 0, 0), false);
        e.backspace();
        assert_eq!(
            e.markdown(),
            "eins\n\n- zwei\n\ndanach\n",
            "Backspace at the start leaves the list"
        );
    }

    #[test]
    fn a_selection_over_blocks_is_deleted_marked_and_converted() {
        let mut e = editor("eins zwei\n\ndrei vier\n");
        e.place(at(&e, 0, 5), false);
        e.place(at(&e, 1, 4), true);
        assert_eq!(e.selected_text(), "zwei\ndrei");
        e.toggle(Mark::Bold);
        assert_eq!(e.markdown(), "eins **zwei**\n\n**drei** vier\n");
        assert_eq!(e.marks(), [Mark::Bold].into());
        e.toggle(Mark::Bold);
        assert_eq!(e.markdown(), "eins zwei\n\ndrei vier\n");
        e.set_kind(Kind::Item {
            depth: 0,
            number: Some(1),
        });
        assert_eq!(e.markdown(), "1. eins zwei\n2. drei vier\n");
        e.type_text("X");
        assert_eq!(e.markdown(), "1. eins X vier\n");
    }

    #[test]
    fn links_and_code_blocks() {
        let mut e = editor("siehe Doku\n");
        e.place(at(&e, 0, 6), false);
        e.right(true, true);
        e.link(Some("https://x.org/a b".into()));
        assert_eq!(e.markdown(), "siehe [Doku](https://x.org/a%20b)\n");
        e.link(None);
        e.set_kind(Kind::Code {
            lang: String::new(),
        });
        e.right(false, false);
        e.type_text("\nmehr");
        assert_eq!(e.markdown(), "```\nsiehe Doku\nmehr\n```\n");
    }

    #[test]
    fn undo_takes_back_typing_at_once_and_redo_brings_it_back() {
        let mut e = editor("a\n");
        e.place(at(&e, 0, 1), false);
        for c in ["b", "c", "d"] {
            e.type_text(c);
        }
        e.enter();
        e.type_text("e");
        assert_eq!(e.markdown(), "abcd\n\ne\n");
        e.undo();
        assert_eq!(
            e.markdown(),
            "abcd\n",
            "the empty new paragraph is not written"
        );
        e.undo();
        assert_eq!(e.markdown(), "abcd\n");
        e.undo();
        assert_eq!(
            e.markdown(),
            "a\n",
            "the untouched part writes its source again"
        );
        e.redo();
        assert_eq!(e.markdown(), "abcd\n");
    }

    #[test]
    fn an_empty_chapter_gets_a_paragraph_and_the_caret_skips_fixed_text() {
        let md = "## 3. Kontext\n\n<!-- hier ergänzen -->\n";
        let mut e = editor(md);
        assert_eq!(e.markdown(), md);
        e.type_text("Eigener Text.");
        assert_eq!(e.markdown(), format!("{md}\nEigener Text.\n"));
        let mut e = editor("eins\n\n---\n\nzwei\n");
        e.place(at(&e, 0, 4), false);
        e.right(false, false);
        assert_eq!(e.block(e.focus).inline.text, "zwei", "over the rule");
        e.left(false, false);
        assert_eq!(
            (e.block(e.focus).inline.text.as_str(), e.focus.at),
            ("eins", 4)
        );
        e.place(
            Caret {
                piece: 2,
                block: 0,
                at: 2,
            },
            true,
        );
        assert_eq!(
            e.focus.piece, e.anchor.piece,
            "a selection stays in its part"
        );
    }

    #[test]
    fn tables_grow_and_shrink_and_cells_never_merge() {
        let mut e = editor("| a | b |\n| --- | --- |\n| 1 | 2 |\n");
        e.place(at(&e, 2, 0), false);
        e.backspace();
        assert_eq!(
            (e.focus.block, e.focus.at),
            (1, 1),
            "to the end of the cell before"
        );
        e.place(at(&e, 3, 1), false);
        e.enter();
        e.type_text("x\ny");
        assert_eq!(
            e.markdown(),
            "| a | b |\n| --- | --- |\n| 1 | 2 |\n| x y |  |\n"
        );
        e.next_cell(false);
        e.type_text("z");
        e.next_cell(false);
        assert_eq!(
            e.markdown(),
            "| a | b |\n| --- | --- |\n| 1 | 2 |\n| x y | z |\n|  |  |\n",
            "Tab after the last cell"
        );
        e.delete_row();
        e.place(at(&e, 0, 0), false);
        e.add_column();
        e.type_text("c");
        assert_eq!(
            e.markdown(),
            "| a | c | b |\n| --- | --- | --- |\n| 1 |  | 2 |\n| x y |  | z |\n"
        );
        e.place(at(&e, 0, 0), false);
        e.place(at(&e, 4, 1), true);
        e.backspace();
        assert_eq!(
            e.markdown(),
            "|  |  |  |\n| --- | --- | --- |\n|  |  | 2 |\n| x y |  | z |\n",
            "only text goes"
        );
        e.delete_column();
        assert!(
            e.markdown().starts_with("|  |  |\n| --- | --- |\n"),
            "{}",
            e.markdown()
        );
        e.undo();
        e.undo();
        assert_eq!(
            e.markdown(),
            "| a | c | b |\n| --- | --- | --- |\n| 1 |  | 2 |\n| x y |  | z |\n"
        );
    }

    #[test]
    fn a_table_goes_in_after_a_paragraph() {
        let mut e = editor("Text\n");
        e.insert_table();
        e.type_text("Kopf");
        assert_eq!(
            e.markdown(),
            "Text\n\n| Kopf |  |\n| --- | --- |\n|  |  |\n"
        );
    }

    #[test]
    fn a_generated_block_taken_over_is_own_text_and_undo_brings_it_back() {
        let md = "## 12. Glossar\n\n<!-- archguard:arc42:12:start -->\n| Begriff | Bedeutung |\n| --- | --- |\n| SCC | Gruppe |\n<!-- archguard:arc42:12:end -->\n\n<!-- hier ergänzen -->\n";
        let fixed = super::super::doc::generated(md);
        let mut e = Editor::new(Doc::parse(md, &fixed));
        let piece = (0..e.doc().pieces.len())
            .find(|&p| e.doc().part(p).is_none())
            .unwrap();
        e.jump_from_fixed(piece);
        assert_eq!(e.block(e.focus).inline.text, "", "the empty part after it");
        assert!(e.take_over(piece));
        assert!(e.in_table(), "the caret in the taken table");
        e.type_text("Neu");
        assert_eq!(
            e.markdown(),
            "## 12. Glossar\n\n| NeuBegriff | Bedeutung |\n| --- | --- |\n| SCC | Gruppe |\n\n<!-- hier ergänzen -->\n"
        );
        e.undo();
        e.undo();
        assert_eq!(e.markdown(), md);
        let mut plain = editor("## 5. Bausteine\n\nText\n");
        assert!(
            !plain.take_over(0),
            "a chapter heading is no generated block"
        );
    }

    #[test]
    fn word_moves_skip_spaces() {
        assert_eq!(word_start("ab cd  ef", 7), 3);
        assert_eq!(word_start("ab cd", 2), 0);
        assert_eq!(word_end("ab cd  ef", 2), 5);
        assert_eq!(word_end("ab", 0), 2);
    }
}
