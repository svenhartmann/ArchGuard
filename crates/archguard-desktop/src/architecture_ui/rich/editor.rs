//! The editing state of the rich-text editor: a caret and a selection in
//! the chapter's editable blocks, the edits keys and toolbar buttons make,
//! and undo. Positions are bytes of a block's text; what lies on which
//! screen line is the layout's business (moving up and down, Home, End).

use std::collections::BTreeSet;

use unicode_segmentation::UnicodeSegmentation;

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
            keep(
                &mut self.undo,
                State {
                    doc: self.doc.clone(),
                    anchor: self.anchor,
                    focus: self.focus,
                },
            );
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
        let (mut start, mut end) = (start, end);
        // A diagram at an end of the selection goes with it: as an empty
        // paragraph it merges with the text beside it.
        for c in [&mut start, &mut end] {
            if self.is_diagram(c.piece, c.block) {
                self.blocks_mut(c.piece)[c.block] = Self::empty_cell();
                c.at = 0;
            }
        }
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
        self.leave_diagram();
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

    /// On a diagram: a new empty paragraph after it, the caret there.
    fn leave_diagram(&mut self) {
        let c = self.focus;
        if self.is_diagram(c.piece, c.block) {
            self.blocks_mut(c.piece)
                .insert(c.block + 1, Self::empty_cell());
            self.collapse(Caret {
                block: c.block + 1,
                at: 0,
                ..c
            });
        }
    }

    /// Enter: a new block after the caret's text; in code a new line; an
    /// empty list entry or quote line becomes a paragraph.
    pub fn enter(&mut self) {
        self.checkpoint(false);
        self.delete_selection();
        if self.is_diagram(self.focus.piece, self.focus.block) {
            return self.leave_diagram();
        }
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
        if c.at == 0 && !blocks[c.block].inline.text.is_empty() {
            // At its start: an empty paragraph above it, the block keeps its
            // kind (a heading stays one, a list entry its number).
            blocks.insert(c.block, Self::empty_cell());
            return self.collapse(Caret {
                block: c.block + 1,
                ..c
            });
        }
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
        step(
            self,
            self.selection().is_none() && self.focus.at > 0,
            erase_back,
        );
    }

    /// Delete: the selection, else the character after; at a block's end
    /// the next block joins this one.
    pub fn delete_forward(&mut self) {
        step(self, false, erase_forward);
    }

    fn is_diagram(&self, piece: usize, block: usize) -> bool {
        matches!(
            self.blocks(piece).get(block).map(|b| &b.kind),
            Some(Kind::Diagram { .. })
        )
    }

    /// Remove block `c.block` (a diagram); a part keeps one block to type in.
    fn remove_block(&mut self, c: Caret) {
        let blocks = self.blocks_mut(c.piece);
        blocks.remove(c.block);
        if blocks.is_empty() {
            blocks.push(Self::empty_cell());
        }
        let block = c.block.min(blocks.len() - 1);
        let at = if block < c.block {
            blocks[block].inline.text.len()
        } else {
            0
        };
        self.collapse(Caret { block, at, ..c });
    }

    /// A diagram block at the caret: in place of an empty paragraph, else
    /// before the caret's block (caret at its start) or after it; never
    /// inside a table. An empty paragraph follows a diagram at the end.
    pub fn insert_diagram(&mut self, id: &str, source: &str) {
        self.checkpoint(false);
        let mut c = self.focus;
        if let Some((range, _)) = self.table() {
            c = Caret {
                block: range.end - 1,
                at: usize::MAX,
                ..c
            };
        }
        let block = Block {
            kind: Kind::Diagram {
                id: id.to_string(),
                source: source.to_string(),
            },
            inline: Inline::default(),
        };
        let blocks = self.blocks_mut(c.piece);
        let here = &blocks[c.block];
        let at = if here.kind == Kind::Paragraph && here.inline.text.is_empty() {
            blocks[c.block] = block;
            c.block
        } else if c.at == 0 && !matches!(here.kind, Kind::Diagram { .. }) {
            blocks.insert(c.block, block);
            c.block
        } else {
            blocks.insert(c.block + 1, block);
            c.block + 1
        };
        if at + 1 == blocks.len() {
            blocks.push(Self::empty_cell());
        }
        self.collapse(Caret {
            block: at,
            at: 0,
            ..c
        });
    }

    /// The diagram at the caret as editable text (its title, note and
    /// Mermaid or PlantUML source); the analysis no longer updates it.
    pub fn detach(&mut self) {
        let c = self.focus;
        let Kind::Diagram { source, .. } = &self.block(c).kind else {
            return;
        };
        // Without its marker lines.
        let inner: Vec<&str> = source.lines().collect();
        if inner.len() < 2 {
            return;
        }
        let inner = inner[1..inner.len() - 1].join("\n");
        let blocks = super::doc::blocks_of(&inner);
        if blocks.is_empty() {
            return;
        }
        self.checkpoint(false);
        self.blocks_mut(c.piece).splice(c.block..=c.block, blocks);
        self.collapse(Caret { at: 0, ..c });
    }

    /// Remove the diagram at the caret.
    pub fn remove_diagram(&mut self) {
        let c = self.focus;
        if self.is_diagram(c.piece, c.block) {
            self.checkpoint(false);
            self.remove_block(c);
        }
    }

    /// Whether the caret is on a diagram block.
    #[cfg(test)]
    pub fn on_diagram(&self) -> bool {
        self.is_diagram(self.focus.piece, self.focus.block)
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
        let std::ops::Range { start, end } = super::doc::table_range(blocks, c.block);
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

    /// Append block `block + 1` to `block`; the caret at the join. Into
    /// text that is no code goes only the first line of code; the other
    /// lines stay code.
    fn join(&mut self, piece: usize, block: usize) {
        let blocks = self.blocks_mut(piece);
        let mut next = blocks.remove(block + 1);
        let at = blocks[block].inline.text.len();
        if matches!(blocks[block].kind, Kind::Code { .. }) {
            blocks[block].inline.text.push_str(&next.inline.text);
        } else {
            if let Some(n) = next.inline.text.find('\n') {
                let rest = next.inline.split_off(n + 1);
                next.inline.delete(n..n + 1);
                let kind = next.kind.clone();
                blocks.insert(block + 1, Block { kind, inline: rest });
            }
            blocks[block].inline.append(next.inline);
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
    /// Code drops the marks; a list entry keeps its depth; code left is a
    /// block per line.
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
        // From the last on: a split block moves only the blocks after it.
        for b in range.rev() {
            let block = &mut self.blocks_mut(piece)[b];
            if matches!(block.kind, Kind::Cell { .. } | Kind::Diagram { .. }) {
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
            } else if block.inline.text.contains('\n') {
                let blocks = self.doc.part_mut(piece).expect("a caret is in a part");
                split_lines(blocks.edit(), b, [&mut self.anchor, &mut self.focus]);
            }
        }
    }

    /// Tab and Shift+Tab: list entries one level deeper or out.
    pub fn indent(&mut self, deeper: bool) {
        step(self, false, |e| {
            let (piece, range) = e.touched();
            for b in range {
                // Only list entries: `blocks_mut` marks the part edited.
                if !matches!(e.blocks(piece)[b].kind, Kind::Item { .. }) {
                    continue;
                }
                if let Kind::Item { depth, .. } = &mut e.blocks_mut(piece)[b].kind {
                    *depth = if deeper {
                        *depth + 1
                    } else {
                        depth.saturating_sub(1)
                    };
                }
            }
        });
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
                    c.at - prev_grapheme(text, c.at)
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
                    c.at + next_grapheme(text, c.at)
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
    pub fn take_over(&mut self, piece: usize, drawn: &[(String, String)]) -> bool {
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
        let mut end = end;
        for r in cuts {
            end -= r.len();
            new.replace_range(r, "");
        }
        // Its diagrams stay diagrams: each section a diagram block.
        for (id, section) in drawn.iter().filter(|(_, s)| !s.is_empty()) {
            if let Some(at) = new[start..end].find(section.as_str()) {
                let at = start + at;
                let tail = if section.ends_with('\n') { "\n" } else { "" };
                let block = super::doc::diagram_block(id, section) + tail;
                new.replace_range(at..at + section.len(), &block);
                end += block.len() - section.len();
            }
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

/// Run `edit` on `e` as one undo step; an edit that changes nothing
/// leaves undo and redo as they were.
fn step(e: &mut Editor, typing: bool, edit: impl FnOnce(&mut Editor)) {
    let before = State {
        doc: e.doc.clone(),
        anchor: e.anchor,
        focus: e.focus,
    };
    let joins = typing && e.typing;
    edit(e);
    if e.doc == before.doc {
        e.typing = false;
        return;
    }
    if !joins {
        keep(&mut e.undo, before);
    }
    e.redo.clear();
    e.typing = typing;
}

fn keep(undo: &mut Vec<State>, state: State) {
    undo.push(state);
    if undo.len() > UNDO {
        undo.remove(0);
    }
}

fn erase_back(e: &mut Editor) {
    if e.delete_selection() {
        return;
    }
    let c = e.focus;
    if e.is_diagram(c.piece, c.block) {
        return e.remove_block(c);
    }
    if c.at == 0 && c.block > 0 && e.is_diagram(c.piece, c.block - 1) {
        return e.remove_block(Caret {
            block: c.block - 1,
            ..c
        });
    }
    if c.at > 0 {
        let text = &e.block(c).inline.text;
        let prev = prev_grapheme(text, c.at);
        e.blocks_mut(c.piece)[c.block]
            .inline
            .delete(c.at - prev..c.at);
        e.collapse(Caret {
            at: c.at - prev,
            ..c
        });
    } else if e.is_cell(c.piece, c.block) {
        // Cells do not merge; Backspace at a cell's start goes to the one before.
        if c.block > 0 && e.is_cell(c.piece, c.block - 1) {
            let at = e.blocks(c.piece)[c.block - 1].inline.text.len();
            e.collapse(Caret {
                block: c.block - 1,
                at,
                ..c
            });
        }
    } else if !matches!(e.block(c).kind, Kind::Paragraph | Kind::Code { .. }) {
        e.blocks_mut(c.piece)[c.block].kind = Kind::Paragraph;
    } else if c.block > 0 && !e.is_cell(c.piece, c.block - 1) {
        e.join(c.piece, c.block - 1);
    }
}

fn erase_forward(e: &mut Editor) {
    if e.delete_selection() {
        return;
    }
    let c = e.focus;
    if e.is_diagram(c.piece, c.block) {
        return e.remove_block(c);
    }
    let len = e.block(c).inline.text.len();
    if c.at == len && e.is_diagram(c.piece, c.block + 1) {
        e.remove_block(Caret {
            block: c.block + 1,
            at: 0,
            ..c
        });
        return e.collapse(c);
    }
    if c.at < len {
        let text = &e.block(c).inline.text;
        let next = next_grapheme(text, c.at);
        e.blocks_mut(c.piece)[c.block]
            .inline
            .delete(c.at..c.at + next);
    } else if c.block + 1 < e.blocks(c.piece).len()
        && !e.is_cell(c.piece, c.block)
        && !e.is_cell(c.piece, c.block + 1)
    {
        e.join(c.piece, c.block);
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

/// Block `b` as a block of its kind per line of its text, the carets on
/// their lines: only code holds line breaks (others are written as one line).
fn split_lines(blocks: &mut Vec<Block>, b: usize, carets: [&mut Caret; 2]) {
    let text = blocks[b].inline.text.clone();
    let kind = blocks[b].kind.clone();
    let lines: Vec<Block> = text
        .split('\n')
        .map(|line| Block {
            kind: kind.clone(),
            inline: Inline::plain(line),
        })
        .collect();
    let more = lines.len() - 1;
    blocks.splice(b..=b, lines);
    for c in carets {
        if c.block > b {
            c.block += more;
        } else if c.block == b {
            let before = &text[..c.at];
            c.block += before.matches('\n').count();
            c.at -= before.rfind('\n').map_or(0, |n| n + 1);
        }
    }
}

/// The bytes of the grapheme cluster before `at` (an emoji with its skin
/// tone, a letter with its accent): one step for the caret.
fn prev_grapheme(text: &str, at: usize) -> usize {
    text[..at].graphemes(true).next_back().map_or(0, str::len)
}

/// The bytes of the grapheme cluster after `at`.
fn next_grapheme(text: &str, at: usize) -> usize {
    text[at..].graphemes(true).next().map_or(0, str::len)
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

    /// A quote without text between rules is kept as written: the caret
    /// goes to the empty part after it, not into a part without blocks.
    #[test]
    fn an_empty_quote_takes_no_caret() {
        let mut e = editor("---\n\n>\n\n---\n");
        e.place(at(&e, 0, 0), false);
        assert_eq!(e.kind(), &Kind::Paragraph);
        assert_eq!(e.markdown(), "---\n\n>\n\n---\n");
    }

    /// Of two tables in one part, the caret's table is its own rows only.
    #[test]
    fn the_table_at_the_caret_ends_at_the_next_table() {
        let mut e =
            editor("| a | b |\n| --- | --- |\n| 1 | 2 |\n\n| c | d |\n| --- | --- |\n| 3 | 4 |\n");
        e.place(at(&e, 5, 0), false);
        let (range, rows) = e.table().expect("in a table");
        assert_eq!(range, 4..8);
        assert_eq!(rows.len(), 2);
        e.place(at(&e, 3, 0), false);
        assert_eq!(e.table().expect("in a table").0, 0..4);
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
        // A `#` heading stays as written (doc.rs); `##` is the editable one.
        let mut e = editor("## Titel\n\nAbsatz eins\n");
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

    /// Tab in a paragraph, Backspace at the start and Delete at the end
    /// change nothing: no undo step, and redo stays.
    #[test]
    fn keys_that_change_nothing_leave_undo_and_redo_alone() {
        let mut e = editor("eins\n");
        e.place(at(&e, 0, 4), false);
        e.type_text("x");
        e.undo();
        e.place(at(&e, 0, 0), false);
        e.indent(true);
        e.indent(false);
        e.backspace();
        e.place(at(&e, 0, 4), false);
        e.delete_forward();
        assert!(!e.can_undo(), "nothing to take back");
        assert!(e.can_redo(), "the undone typing can come back");
        e.redo();
        assert_eq!(e.markdown(), "einsx\n");
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
        assert!(e.take_over(piece, &[]));
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
            !plain.take_over(0, &[]),
            "a chapter heading is no generated block"
        );
    }

    /// Taken over, a generated block's diagrams stay diagrams: each of
    /// its diagram sections becomes a diagram block.
    #[test]
    fn diagrams_of_a_taken_over_block_become_diagram_blocks() {
        let section = "### Pakete\n\n```mermaid\nflowchart LR\n```\n";
        let md = format!(
            "## 5. Bausteine\n\n<!-- archguard:arc42:05:start -->\nVorher.\n\n{section}\nNachher.\n<!-- archguard:arc42:05:end -->\n"
        );
        let fixed = super::super::doc::generated(&md);
        let mut e = Editor::new(Doc::parse(&md, &fixed));
        let piece = (0..e.doc().pieces.len())
            .find(|&p| e.doc().part(p).is_none())
            .unwrap();
        let drawn = [("packages".to_string(), section.to_string())];
        assert!(e.take_over(piece, &drawn));
        let block = super::super::doc::diagram_block("packages", section);
        assert_eq!(
            e.markdown(),
            format!("## 5. Bausteine\n\nVorher.\n\n{block}\n\nNachher.\n")
        );
        assert!(
            (0..e.doc().pieces.len()).any(|p| e.doc().part(p).is_some_and(|part| part
                .blocks()
                .iter()
                .any(|b| matches!(b.kind, Kind::Diagram { .. })))),
            "a diagram block to detach or remove"
        );
    }

    /// Each section of a taken-over block becomes a diagram block, also
    /// when an earlier one made the text longer (umlauts near the end).
    #[test]
    fn every_diagram_of_a_taken_over_block_becomes_a_diagram_block() {
        let first = "### Pakete\n\n```mermaid\nflowchart LR\n```\n";
        let second = "### Zyklen\n\n```mermaid\nflowchart TB\n```\n";
        let tail = "Größe: äöü äöü äöü äöü äöü äöü äöü äöü äöü äöü äöü äöü äöü äöü äöü.\n";
        let md = format!(
            "## 5. Bausteine\n\n<!-- archguard:arc42:05:start -->\n{first}\n{second}\n{tail}<!-- archguard:arc42:05:end -->\n"
        );
        let fixed = super::super::doc::generated(&md);
        let mut e = Editor::new(Doc::parse(&md, &fixed));
        let piece = (0..e.doc().pieces.len())
            .find(|&p| e.doc().part(p).is_none())
            .unwrap();
        let drawn = [
            ("packages".to_string(), first.to_string()),
            ("cycles".to_string(), second.to_string()),
        ];
        assert!(e.take_over(piece, &drawn));
        let one = super::super::doc::diagram_block("packages", first);
        let two = super::super::doc::diagram_block("cycles", second);
        assert_eq!(
            e.markdown(),
            format!("## 5. Bausteine\n\n{one}\n\n{two}\n\n{tail}")
        );
    }

    /// A block with both markers on one line has no inside to keep.
    #[test]
    fn detaching_a_one_line_diagram_block_keeps_it() {
        let block = "<!-- archguard:diagram:calls:start --> <!-- archguard:diagram:calls:end -->";
        let mut e = editor(&format!("eins\n\n{block}\n"));
        e.place(at(&e, 1, 0), false);
        assert!(e.on_diagram());
        e.detach();
        assert!(e.markdown().contains(block));
    }

    #[test]
    fn diagrams_go_in_at_the_caret_and_out_with_backspace_or_detach() {
        let block = super::super::doc::diagram_block(
            "calls",
            "### Aufrufe\n\nQuelle.\n\n```mermaid\nflowchart LR\n```",
        );
        let mut e = editor("eins\n\nzwei\n");
        e.place(at(&e, 0, 4), false);
        e.insert_diagram("calls", &block);
        assert!(e.on_diagram());
        assert_eq!(
            e.markdown(),
            format!("eins\n\n{block}\n\nzwei\n"),
            "after the caret's paragraph"
        );
        e.type_text("x");
        assert_eq!(
            e.markdown(),
            format!("eins\n\n{block}\n\nx\n\nzwei\n"),
            "typing on it opens a paragraph after it"
        );
        e.place(at(&e, 2, 0), false);
        e.backspace();
        assert_eq!(
            e.markdown(),
            "eins\n\nx\n\nzwei\n",
            "Backspace after it removes it as a whole"
        );
        e.undo();
        e.place(at(&e, 1, 0), false);
        e.detach();
        assert_eq!(
            e.markdown(),
            "eins\n\n### Aufrufe\n\nQuelle.\n\n```mermaid\nflowchart LR\n```\n\nx\n\nzwei\n"
        );
        let mut last = editor("nur Text\n");
        last.place(at(&last, 0, 8), false);
        last.insert_diagram("calls", &block);
        last.type_text("weiter");
        assert_eq!(
            last.markdown(),
            format!("nur Text\n\n{block}\n\nweiter\n"),
            "a paragraph follows a diagram at the end"
        );
    }

    /// Text that is no code holds no line break: written, it would be one
    /// line after reopening (a heading only its first).
    fn no_break_outside_code(e: &Editor) {
        for b in e.blocks(e.focus.piece) {
            if !matches!(b.kind, Kind::Code { .. }) {
                assert!(!b.inline.text.contains('\n'), "{b:?}");
            }
        }
    }

    /// Leaving code by the toolbar makes one block per line, the caret
    /// on its line.
    #[test]
    fn a_code_block_made_text_is_one_block_per_line() {
        let md = "```mermaid\nflowchart LR\n  A --> B\n```\n";
        let mut e = editor(md);
        e.place(at(&e, 0, 15), false);
        e.set_kind(Kind::Paragraph);
        no_break_outside_code(&e);
        assert_eq!(e.markdown(), "flowchart LR\n\n  A --> B\n");
        assert_eq!((e.focus.block, e.focus.at), (1, 2));
        let mut h = editor(md);
        h.set_kind(Kind::Heading(3));
        assert_eq!(h.markdown(), "### flowchart LR\n\n###   A --> B\n");
    }

    /// Backspace at a code block's start and Delete before it take its
    /// first line into the text before; the other lines stay code.
    #[test]
    fn joining_a_code_block_into_text_takes_its_first_line() {
        let md = "Text\n\n```\neins\nzwei\n```\n";
        let mut e = editor(md);
        e.place(at(&e, 1, 0), false);
        e.backspace();
        no_break_outside_code(&e);
        assert_eq!(e.markdown(), "Texteins\n\n```\nzwei\n```\n");
        assert_eq!((e.focus.block, e.focus.at), (0, 4));
        let mut d = editor(md);
        d.place(at(&d, 0, 4), false);
        d.delete_forward();
        assert_eq!(d.markdown(), "Texteins\n\n```\nzwei\n```\n");
        let mut one = editor("Text\n\n```\neins\n```\n");
        one.place(at(&one, 0, 4), false);
        one.delete_forward();
        assert_eq!(one.markdown(), "Texteins\n");
    }

    /// Enter at the start of a block opens an empty line above it; the
    /// block keeps its kind and the caret its start.
    #[test]
    fn enter_at_a_blocks_start_keeps_its_kind() {
        for (md, want) in [
            ("### Titel\n", "### Titel\n"),
            ("1. eins\n2. zwei\n", "1. eins\n2. zwei\n"),
            ("> Zitat\n", "> Zitat\n"),
        ] {
            let mut e = editor(md);
            let last = e.blocks(e.focus.piece).len() - 1;
            e.place(at(&e, last, 0), false);
            e.enter();
            assert_eq!(e.markdown(), want, "{md}");
            assert_eq!((e.focus.block, e.focus.at), (last + 1, 0), "{md}");
        }
    }

    /// Left, Right, Delete and Backspace take an emoji with its skin tone,
    /// a family of joined emoji and a letter with its accent as one.
    #[test]
    fn keys_step_over_whole_grapheme_clusters() {
        for g in ["👍🏽", "👨\u{200d}👩\u{200d}👧", "e\u{301}"] {
            let text = format!("a{g}b");
            let mut e = editor(&format!("{text}\n"));
            e.place(at(&e, 0, 1), false);
            e.right(false, false);
            assert_eq!(e.focus.at, 1 + g.len(), "right over {g:?}");
            e.left(false, false);
            assert_eq!(e.focus.at, 1, "left over {g:?}");
            e.delete_forward();
            assert_eq!(e.markdown(), "ab\n", "delete {g:?}");
            e.undo();
            e.place(at(&e, 0, 1 + g.len()), false);
            e.backspace();
            assert_eq!(e.markdown(), "ab\n", "backspace {g:?}");
        }
    }

    #[test]
    fn word_moves_skip_spaces() {
        assert_eq!(word_start("ab cd  ef", 7), 3);
        assert_eq!(word_start("ab cd", 2), 0);
        assert_eq!(word_end("ab cd  ef", 2), 5);
        assert_eq!(word_end("ab", 0), 2);
    }
}
