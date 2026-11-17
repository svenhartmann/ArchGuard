//! The rich-text editor in the window: lays the chapter's blocks out at
//! the editor's width, turns them into the rows `architecture_rich.slint`
//! draws, and turns keys, clicks and toolbar buttons into edits.

use std::cell::RefCell;
use std::collections::HashMap;

use super::super::{blocks, markdown};
use super::doc::{self, Block, Doc, Kind, Piece};
use super::editor::{Caret, Editor};
use super::inline::Mark;
use super::layout::{Fonts, Laid, Rect, Style};
use crate::tab_support::model;
use crate::ui::{ArcFigure, RichCell, RichItem, RichRect, RichRun, RichTools};

thread_local! {
    /// Fonts and layout caches of the window's thread.
    static FONTS: RefCell<Fonts> = RefCell::new(Fonts::default());
}

/// Space a code block's background reaches below its text (as in the view).
const CODE_PAD: f32 = 8.0;
/// Space around a table cell's text.
const CELL_X: f32 = 8.0;
const CELL_Y: f32 = 6.0;
/// A diagram block's height for drags and arrow keys, which only need
/// its order: the view sizes it to its drawing.
const DIAGRAM_H: f32 = 360.0;

/// What a key did.
#[derive(Debug, PartialEq, Eq)]
pub enum Key {
    Done,
    /// Open the link row (Cmd+K).
    Link,
    /// Not the editor's key: the window handles it.
    Ignored,
}

/// The editor with its layout.
pub struct Rich {
    pub editor: Editor,
    width: f32,
    pub style: Style,
    /// Each block's layout with the block and width it was made for.
    laid: HashMap<(usize, usize), (Block, f32, Laid)>,
    /// The x the caret keeps over moves up and down.
    goal: Option<f32>,
    /// The drawings of the diagram blocks, by diagram id.
    figures: HashMap<String, ArcFigure>,
    /// The diagram sections of fixed text last drawn (`items`): taken
    /// over, they become diagram blocks.
    drawn: Vec<(String, String)>,
    /// Read only: no caret or selection, every own diagram drawn.
    pub reading: bool,
}

/// Space above a block and the left edge of its text.
fn geometry(blocks: &[Block], b: usize, entries: &[Option<(usize, Option<u64>)>]) -> (f32, f32) {
    let prev = b.checked_sub(1).map(|p| &blocks[p].kind);
    let top = match (&blocks[b].kind, prev) {
        // A table's rows follow each other; space above its header.
        (Kind::Cell { row: 1.., .. }, _) => 0.0,
        (Kind::Cell { .. }, Some(Kind::Cell { .. })) => 0.0,
        (Kind::Cell { .. }, None) => 8.0,
        (Kind::Cell { .. }, _) => 14.0,
        (_, None) => 8.0,
        (Kind::Heading(_), _) => 18.0,
        (Kind::Item { .. }, Some(Kind::Item { .. })) => 4.0,
        (Kind::Quote, Some(Kind::Quote)) => 6.0,
        (Kind::Code { .. }, _) => 18.0,
        _ => 10.0,
    };
    let indent = match (&blocks[b].kind, entries[b]) {
        (Kind::Item { .. }, Some((depth, _))) => 30.0 + 22.0 * depth as f32,
        (Kind::Quote, _) => 14.0,
        (Kind::Code { .. }, _) => 12.0,
        _ => 0.0,
    };
    (top, indent)
}

fn kind_number(kind: &Kind) -> i32 {
    match kind {
        Kind::Paragraph => 0,
        Kind::Heading(_) => 1,
        Kind::Item { .. } => 2,
        Kind::Quote => 3,
        Kind::Code { .. } => 4,
        Kind::Cell { .. } => 5,
        Kind::Diagram { .. } => 6,
    }
}

fn rich_rect(r: Rect) -> RichRect {
    RichRect {
        x: r.x,
        y: r.y,
        w: r.w,
        h: r.h,
    }
}

/// `laid`'s runs moved by `dx`, `dy` (a cell's place in its row).
fn runs(laid: &Laid, dx: f32, dy: f32) -> Vec<RichRun> {
    laid.runs()
        .into_iter()
        .map(|r| RichRun {
            text: r.text.into(),
            x: r.x + dx,
            y: r.y + dy,
            w: r.width,
            h: r.height,
            size: r.size,
            weight: r.weight,
            italic: r.look.italic,
            code: r.look.code,
            link: r.look.link,
        })
        .collect()
}

fn moved(r: Rect, dx: f32, dy: f32) -> RichRect {
    rich_rect(Rect {
        x: r.x + dx,
        y: r.y + dy,
        ..r
    })
}

/// The selected boxes of block `b` (`len` bytes long) of piece `p`.
fn selected(
    laid: &Laid,
    selection: Option<(Caret, Caret)>,
    p: usize,
    b: usize,
    len: usize,
    dx: f32,
    dy: f32,
) -> Vec<RichRect> {
    let Some((s, e)) = selection.filter(|(s, e)| s.piece == p && (s.block..=e.block).contains(&b))
    else {
        return Vec::new();
    };
    let from = if b == s.block { s.at } else { 0 };
    let to = if b == e.block { e.at } else { len };
    let mut rects: Vec<RichRect> = laid
        .selection(from..to)
        .into_iter()
        .map(|r| moved(r, dx, dy))
        .collect();
    // A selected line break shows as a narrow box.
    if b != e.block {
        let end = laid.caret(len);
        rects.push(moved(Rect { w: 6.0, ..end }, dx, dy));
    }
    rects
}

impl Rich {
    /// Edit chapter Markdown (LF line ends).
    pub fn new(md: &str) -> Rich {
        let fixed = doc::generated(md);
        Rich {
            editor: Editor::new(Doc::parse(md, &fixed)),
            width: 600.0,
            style: Style::default(),
            laid: HashMap::new(),
            goal: None,
            figures: HashMap::new(),
            drawn: Vec::new(),
            reading: false,
        }
    }

    /// Insert the diagram `id` with Markdown `section` at the caret, as a
    /// block the analysis refreshes on save.
    pub fn insert_diagram(&mut self, id: &str, section: &str) {
        self.goal = None;
        self.editor
            .insert_diagram(id, &doc::diagram_block(id, section));
    }

    pub fn markdown(&self) -> String {
        self.editor.markdown()
    }

    /// The window's fonts; a change lays every block out again.
    pub fn set_style(&mut self, style: Style) {
        if style != self.style {
            self.style = style;
            self.laid.clear();
        }
    }

    /// The editor's width changed; true if the rows must be made again.
    pub fn set_width(&mut self, width: f32) -> bool {
        let changed = (width - self.width).abs() >= 0.5 && width > 0.0;
        if changed {
            self.width = width;
        }
        changed
    }

    fn blocks(&self, piece: usize) -> &[Block] {
        self.editor.doc().part(piece).map_or(&[], |p| p.blocks())
    }

    /// Space above block `b` of `piece` and its text's left edge.
    fn place(&self, piece: usize, b: usize) -> (f32, f32) {
        let blocks = self.blocks(piece);
        geometry(blocks, b, &doc::list_entries(blocks))
    }

    fn is_cell(&self, piece: usize, b: usize) -> bool {
        matches!(
            self.blocks(piece).get(b).map(|x| &x.kind),
            Some(Kind::Cell { .. })
        )
    }

    /// The first block of the table around cell `b` and its column widths
    /// as shares of the editor's width (as the reader's).
    fn table_of(&self, piece: usize, b: usize) -> (usize, Vec<f32>) {
        let blocks = self.blocks(piece);
        let Kind::Cell { cols, .. } = blocks[b].kind else {
            return (b, vec![1.0]);
        };
        let start = (0..=b)
            .rev()
            .take_while(|&i| self.is_cell(piece, i))
            .last()
            .unwrap_or(b);
        let end = (b..blocks.len())
            .take_while(|&i| self.is_cell(piece, i))
            .last()
            .unwrap_or(b)
            + 1;
        let texts: Vec<String> = blocks[start..end]
            .iter()
            .map(|c| c.inline.text.clone())
            .collect();
        let head = cols.min(texts.len());
        let rows: Vec<Vec<String>> = texts[head..]
            .chunks(cols.max(1))
            .map(<[String]>::to_vec)
            .collect();
        (start, blocks::column_widths(&texts[..head], &rows))
    }

    /// Left edge and width of cell `b` in its row.
    fn cell_box(&self, piece: usize, b: usize) -> (f32, f32) {
        let Kind::Cell { col, .. } = self.blocks(piece)[b].kind else {
            return (0.0, self.width);
        };
        let (_, shares) = self.table_of(piece, b);
        let x = shares[..col.min(shares.len())].iter().sum::<f32>() * self.width;
        (x, shares.get(col).copied().unwrap_or(1.0) * self.width)
    }

    /// The first cell of the row of cell `b`, and the row's column count.
    fn row_of(&self, piece: usize, b: usize) -> (usize, usize) {
        match self.blocks(piece)[b].kind {
            Kind::Cell { col, cols, .. } => (b - col, cols),
            _ => (b, 1),
        }
    }

    /// A table row's height: its tallest cell and the space around it.
    fn row_height(&mut self, piece: usize, first: usize) -> f32 {
        let (_, cols) = self.row_of(piece, first);
        let end = (first + cols).min(self.blocks(piece).len());
        let tallest = (first..end)
            .map(|c| self.laid(piece, c).height())
            .fold(0.0, f32::max);
        tallest + 2.0 * CELL_Y
    }

    /// The height of the row block `b` is in: a table row, or the block.
    fn unit_height(&mut self, piece: usize, b: usize) -> f32 {
        if matches!(self.blocks(piece)[b].kind, Kind::Diagram { .. }) {
            return DIAGRAM_H;
        }
        if self.is_cell(piece, b) {
            let (first, _) = self.row_of(piece, b);
            self.row_height(piece, first)
        } else {
            self.laid(piece, b).height() + self.pad(piece, b)
        }
    }

    /// Block `b` of `piece` laid out at the editor's width (cached).
    fn laid(&mut self, piece: usize, b: usize) -> &Laid {
        let block = self.blocks(piece)[b].clone();
        let (_, indent) = self.place(piece, b);
        let width = match block.kind {
            Kind::Code { .. } => self.width - indent - 12.0,
            Kind::Cell { .. } => self.cell_box(piece, b).1 - 2.0 * CELL_X,
            _ => self.width - indent,
        }
        .max(40.0);
        let style = &self.style;
        let stale = self
            .laid
            .get(&(piece, b))
            .is_none_or(|(old, w, _)| *old != block || *w != width);
        if stale {
            let laid = FONTS.with(|f| f.borrow_mut().lay_out(&block, width, style));
            self.laid.insert((piece, b), (block, width, laid));
        }
        &self.laid[&(piece, b)].2
    }

    /// The rows the view draws; `note` is the reader's text under a
    /// diagram it cannot draw, `figure` draws a diagram block's diagram,
    /// `own` the Mermaid source of an own code block, and `drawn` are the
    /// diagram sections (id, Markdown) fixed text shows as their drawing,
    /// in their place.
    pub fn items(
        &mut self,
        note: &str,
        figure: &dyn Fn(&str) -> Option<ArcFigure>,
        own: &dyn Fn(&str) -> Option<ArcFigure>,
        drawn: &[(String, String)],
    ) -> Vec<RichItem> {
        self.drawn = drawn.to_vec();
        // Reading: a caret in no block.
        let reading = self.reading;
        let (selection, focus) = if reading {
            let nowhere = Caret {
                piece: usize::MAX,
                block: 0,
                at: 0,
            };
            (None, nowhere)
        } else {
            (self.editor.selection(), self.editor.focus)
        };
        let pieces: Vec<Result<Vec<Block>, String>> = self
            .editor
            .doc()
            .pieces
            .iter()
            .map(|p| match p {
                Piece::Fixed(text) => Err(text.clone()),
                Piece::Part(part) => Ok(part.blocks().to_vec()),
            })
            .collect();
        let mut out = Vec::new();
        for (p, piece) in pieces.into_iter().enumerate() {
            let blocks = match piece {
                Err(text) => {
                    let generated = !doc::generated(&text).is_empty();
                    for part in split_drawn(&text, drawn) {
                        let segment = match part {
                            Ok(segment) => segment,
                            Err(id) => {
                                if !self.figures.contains_key(&id)
                                    && let Some(f) = figure(&id)
                                {
                                    self.figures.insert(id.clone(), f);
                                }
                                out.push(RichItem {
                                    figure: self.figures.get(&id).cloned().unwrap_or_default(),
                                    piece: p as i32,
                                    block: -1,
                                    kind: 7,
                                    top: 14.0,
                                    generated,
                                    ..Default::default()
                                });
                                continue;
                            }
                        };
                        // The chapter heading shows above the editor.
                        let shown: Vec<markdown::Block> = markdown::parse(&segment)
                            .into_iter()
                            .filter(|b| {
                                !matches!(b, markdown::Block::Heading { text, .. } if doc::chapter_number(text))
                            })
                            .collect();
                        if !shown.is_empty() {
                            out.push(RichItem {
                                fixed: model(blocks::ui_blocks(&shown, note)),
                                piece: p as i32,
                                block: -1,
                                generated,
                                ..Default::default()
                            });
                        }
                    }
                    continue;
                }
                Ok(blocks) => blocks,
            };
            let entries = doc::list_entries(&blocks);
            for (b, block) in blocks.iter().enumerate() {
                let (top, indent) = geometry(&blocks, b, &entries);
                if let Kind::Cell { row, col, cols, .. } = block.kind {
                    if col == 0 {
                        out.push(self.table_row(p, b, row, cols, top, selection, focus));
                    }
                    continue;
                }
                if let Kind::Diagram { id, .. } = &block.kind {
                    if !self.figures.contains_key(id)
                        && let Some(f) = figure(id)
                    {
                        self.figures.insert(id.clone(), f);
                    }
                    let here = focus.piece == p && focus.block == b;
                    out.push(RichItem {
                        fixed: model(Vec::new()),
                        cells: model(Vec::new()),
                        runs: model(Vec::new()),
                        selection: model(Vec::new()),
                        figure: self.figures.get(id).cloned().unwrap_or_default(),
                        piece: p as i32,
                        block: b as i32,
                        kind: 6,
                        top: 14.0,
                        caret: here,
                        ..Default::default()
                    });
                    continue;
                }
                let here = focus.piece == p && focus.block == b;
                // Own Mermaid is drawn until the caret goes in to edit it.
                if !here
                    && matches!(&block.kind, Kind::Code { lang } if lang == "mermaid")
                    && let Some(f) = self.own_figure(&block.inline.text, own)
                {
                    out.push(RichItem {
                        figure: f,
                        piece: p as i32,
                        block: b as i32,
                        kind: 8,
                        top,
                        ..Default::default()
                    });
                    continue;
                }
                let laid = self.laid(p, b);
                let len = block.inline.text.len();
                let runs = runs(laid, 0.0, 0.0);
                let selected = selected(laid, selection, p, b, len, 0.0, 0.0);
                let first = laid.caret(0);
                let marker = match entries[b] {
                    Some((_, Some(n))) => format!("{n}."),
                    Some((depth, None)) if depth % 2 == 1 => "◦".into(),
                    Some(_) => "•".into(),
                    None => String::new(),
                };
                out.push(RichItem {
                    fixed: model(Vec::new()),
                    cells: model(Vec::new()),
                    // The hint to write shows only to edit.
                    empty: len == 0 && block.kind == Kind::Paragraph && !reading,
                    generated: false,
                    piece: p as i32,
                    block: b as i32,
                    kind: kind_number(&block.kind),
                    top,
                    height: laid.height(),
                    indent,
                    marker: marker.into(),
                    marker_y: first.y,
                    marker_h: first.h,
                    runs: model(runs),
                    selection: model(selected),
                    caret: here,
                    caret_rect: if here {
                        rich_rect(laid.caret(focus.at))
                    } else {
                        RichRect::default()
                    },
                    figure: Default::default(),
                });
            }
        }
        out
    }

    /// The drawing of own Mermaid `source`, kept until the source changes.
    fn own_figure(
        &mut self,
        source: &str,
        own: &dyn Fn(&str) -> Option<ArcFigure>,
    ) -> Option<ArcFigure> {
        // Ids have no line break, so a source never meets one.
        let key = format!("\n{source}");
        if !self.figures.contains_key(&key) {
            self.figures.insert(key.clone(), own(source)?);
        }
        self.figures.get(&key).cloned()
    }

    /// One table row as one row of the view: its cells' runs, selection
    /// and caret in row coordinates, and the cells' places for the grid.
    #[allow(clippy::too_many_arguments)]
    fn table_row(
        &mut self,
        p: usize,
        first: usize,
        row: usize,
        cols: usize,
        top: f32,
        selection: Option<(Caret, Caret)>,
        focus: Caret,
    ) -> RichItem {
        let end = (first + cols).min(self.blocks(p).len());
        let (mut all_runs, mut sel, mut cells) = (Vec::new(), Vec::new(), Vec::new());
        let mut caret = None;
        for c in first..end {
            let (x, w) = self.cell_box(p, c);
            let (dx, dy) = (x + CELL_X, CELL_Y);
            let len = self.blocks(p)[c].inline.text.len();
            let laid = self.laid(p, c);
            all_runs.extend(runs(laid, dx, dy));
            sel.extend(selected(laid, selection, p, c, len, dx, dy));
            if focus.piece == p && focus.block == c {
                caret = Some(moved(laid.caret(focus.at), dx, dy));
            }
            cells.push(RichCell {
                x,
                w,
                header: row == 0,
            });
        }
        RichItem {
            fixed: model(Vec::new()),
            cells: model(cells),
            empty: false,
            generated: false,
            piece: p as i32,
            block: first as i32,
            kind: 5,
            top,
            height: self.row_height(p, first),
            indent: 0.0,
            marker: Default::default(),
            marker_y: 0.0,
            marker_h: 0.0,
            runs: model(all_runs),
            selection: model(sel),
            caret: caret.is_some(),
            caret_rect: caret.unwrap_or_default(),
            figure: Default::default(),
        }
    }

    /// What the toolbar shows.
    pub fn tools(&self) -> RichTools {
        let e = &self.editor;
        let marks = e.marks();
        let (kind, level, ordered) = match e.kind() {
            Kind::Heading(l) => (1, *l as i32, false),
            Kind::Item { number, .. } => (2, 0, number.is_some()),
            k => (kind_number(k), 0, false),
        };
        let target = e.link_target();
        RichTools {
            bold: marks.contains(&Mark::Bold),
            italic: marks.contains(&Mark::Italic),
            code: marks.contains(&Mark::Code),
            link: target.is_some(),
            kind,
            level,
            ordered,
            undo: e.can_undo(),
            redo: e.can_redo(),
            table: e.in_table(),
            target: target.unwrap_or_default().into(),
        }
    }

    /// A toolbar button.
    pub fn tool(&mut self, name: &str) {
        self.goal = None;
        let e = &mut self.editor;
        match name {
            "bold" => e.toggle(Mark::Bold),
            "italic" => e.toggle(Mark::Italic),
            "code" => e.toggle(Mark::Code),
            "unlink" => e.link(None),
            "h2" => e.set_kind(Kind::Heading(2)),
            "h3" => e.set_kind(Kind::Heading(3)),
            "h4" => e.set_kind(Kind::Heading(4)),
            "bullet" => e.set_kind(Kind::Item {
                depth: 0,
                number: None,
            }),
            "numbered" => e.set_kind(Kind::Item {
                depth: 0,
                number: Some(1),
            }),
            "quote" => e.set_kind(Kind::Quote),
            "codeblock" => e.set_kind(Kind::Code {
                lang: String::new(),
            }),
            "paragraph" => e.set_kind(Kind::Paragraph),
            "undo" => e.undo(),
            "redo" => e.redo(),
            "table" => e.insert_table(),
            "row_add" => e.add_row(),
            "col_add" => e.add_column(),
            "row_del" => e.delete_row(),
            "col_del" => e.delete_column(),
            "detach" => e.detach(),
            "remove_diagram" => e.remove_diagram(),
            _ => {}
        }
    }

    pub fn link(&mut self, target: &str) {
        let target = target.trim();
        self.editor
            .link((!target.is_empty()).then(|| target.to_string()));
    }

    pub fn type_text(&mut self, text: &str) {
        self.goal = None;
        self.editor.type_text(&text.replace("\r\n", "\n"));
    }

    /// A named key or a shortcut letter with Shift, Alt (Option) and
    /// Control (Command on macOS).
    pub fn key(&mut self, name: &str, shift: bool, alt: bool, control: bool) -> Key {
        let mac = cfg!(target_os = "macos");
        // Words: Option on macOS, Control elsewhere; line ends: Command.
        let word = if mac { alt } else { control };
        let line = mac && control;
        if !matches!(name, "up" | "down") {
            self.goal = None;
        }
        let e = &mut self.editor;
        match name {
            "left" if line => self.line_end(false, shift),
            "right" if line => self.line_end(true, shift),
            "left" => e.left(word, shift),
            "right" => e.right(word, shift),
            "up" => self.vertical(-1, shift),
            "down" => self.vertical(1, shift),
            "home" => self.line_end(false, shift),
            "end" => self.line_end(true, shift),
            "backspace" => {
                if word && e.selection().is_none() {
                    e.left(true, true);
                }
                e.backspace();
            }
            "delete" => {
                if word && e.selection().is_none() {
                    e.right(true, true);
                }
                e.delete_forward();
            }
            "enter" => e.enter(),
            "tab" | "backtab" if e.next_cell(name == "backtab" || shift) => {}
            "tab" | "backtab" => {
                if matches!(e.kind(), Kind::Code { .. }) && name == "tab" {
                    e.type_text("    ");
                } else {
                    e.indent(name == "tab" && !shift);
                }
            }
            _ if control => return self.shortcut(&name.to_lowercase(), shift),
            _ => return Key::Ignored,
        }
        Key::Done
    }

    fn shortcut(&mut self, letter: &str, shift: bool) -> Key {
        let e = &mut self.editor;
        match letter {
            "b" => e.toggle(Mark::Bold),
            "i" => e.toggle(Mark::Italic),
            "e" => e.toggle(Mark::Code),
            "k" => return Key::Link,
            "z" if shift => e.redo(),
            "z" => e.undo(),
            "y" => e.redo(),
            "a" => e.select_all(),
            "c" => clipboard_set(&e.selected_text()),
            "x" => {
                clipboard_set(&e.selected_text());
                if e.selection().is_some() {
                    e.backspace();
                }
            }
            "v" => {
                if let Some(text) = clipboard_get() {
                    e.type_text(&text.replace("\r\n", "\n"));
                }
            }
            _ => return Key::Ignored,
        }
        Key::Done
    }

    /// Home or End of the caret's line.
    fn line_end(&mut self, end: bool, extend: bool) {
        let c = self.editor.focus;
        let r = self.laid(c.piece, c.block).line_range(c.at);
        let at = if end { r.end } else { r.start };
        self.editor.place(Caret { at, ..c }, extend);
    }

    /// The left edge of block `b`'s text in its row (a cell's in its table row).
    fn text_left(&self, piece: usize, b: usize) -> f32 {
        if self.is_cell(piece, b) {
            self.cell_box(piece, b).0 + CELL_X
        } else {
            self.place(piece, b).1
        }
    }

    /// One line up (-1) or down (+1), keeping the x the moves started at.
    fn vertical(&mut self, delta: isize, extend: bool) {
        let c = self.editor.focus;
        let left = self.text_left(c.piece, c.block);
        let x = match self.goal {
            Some(x) => x,
            None => self.laid(c.piece, c.block).caret(c.at).x + left,
        };
        self.goal = Some(x);
        if let Some(at) = self.laid(c.piece, c.block).line_at(c.at, delta, x - left) {
            return self.editor.place(Caret { at, ..c }, extend);
        }
        // Just above or below this row: the block or table row there.
        let (first, cols) = self.row_of(c.piece, c.block);
        let len = self.blocks(c.piece).len();
        let indent = self.place(c.piece, first).1;
        let to = if delta < 0 && first > 0 {
            // Above the space over this row: the last line of the row before.
            let top = self.place(c.piece, first).0;
            Some(self.locate(c.piece, first, x - indent, -top - 1.0))
        } else if delta > 0 && first + cols < len {
            let below = self.unit_height(c.piece, first) + self.place(c.piece, first + cols).0;
            Some(self.locate(c.piece, first, x - indent, below + 0.5))
        } else if delta < 0 {
            // The last row of the part before.
            self.editor
                .prev_block(Caret {
                    block: first,
                    at: 0,
                    ..c
                })
                .map(|n| {
                    let (u, _) = self.row_of(n.piece, n.block);
                    let h = self.unit_height(n.piece, u);
                    let indent = self.place(n.piece, u).1;
                    self.locate(n.piece, u, x - indent, h - CODE_PAD.min(h) - 0.5)
                })
        } else {
            let last = Caret {
                block: (first + cols).min(len) - 1,
                ..c
            };
            self.editor.next_block(last).map(|n| {
                let indent = self.place(n.piece, n.block).1;
                self.locate(n.piece, n.block, x - indent, 0.5)
            })
        };
        let to = to.unwrap_or_else(|| match delta < 0 {
            // Past the first or last line: to the block's start or end.
            true => Caret { at: 0, ..c },
            false => Caret {
                at: self.blocks(c.piece)[c.block].inline.text.len(),
                ..c
            },
        });
        self.editor.place(to, extend);
    }

    /// The caret at `x`, `y` of the row of block `b` (its text, or a table
    /// row); a point above or below it (a drag) lands in the rows of the
    /// same part there.
    fn locate(&mut self, piece: usize, b: usize, mut x: f32, mut y: f32) -> Caret {
        let len = self.blocks(piece).len();
        if len == 0 {
            return self.editor.focus;
        }
        let (mut u, _) = self.row_of(piece, b.min(len - 1));
        loop {
            let (top, indent) = self.place(piece, u);
            let height = self.unit_height(piece, u);
            if y < 0.0 && u > 0 {
                let (prev, _) = self.row_of(piece, u - 1);
                let prev_indent = self.place(piece, prev).1;
                y += top + self.unit_height(piece, prev);
                x += indent - prev_indent;
                u = prev;
                continue;
            }
            let next = u + self.row_of(piece, u).1;
            // The space above a row belongs to the row before (as `y < 0`
            // above), so a point in it does not go back and forth.
            let next_top = if next < len {
                self.place(piece, next).0
            } else {
                0.0
            };
            if y > height + next_top && next < len {
                let next_indent = self.place(piece, next).1;
                y -= height + next_top;
                x += indent - next_indent;
                u = next;
                continue;
            }
            if matches!(self.blocks(piece)[u].kind, Kind::Diagram { .. }) {
                return Caret {
                    piece,
                    block: u,
                    at: 0,
                };
            }
            if self.is_cell(piece, u) {
                let (first, cols) = self.row_of(piece, u);
                let end = (first + cols).min(len);
                let c = (first..end)
                    .find(|&c| {
                        let (cx, cw) = self.cell_box(piece, c);
                        x < cx + cw
                    })
                    .unwrap_or(end - 1);
                let left = self.text_left(piece, c);
                let laid = self.laid(piece, c);
                let local = (y - CELL_Y).clamp(0.0, (laid.height() - 0.5).max(0.0));
                let at = laid.hit(x - left, local);
                return Caret {
                    piece,
                    block: c,
                    at,
                };
            }
            let laid = self.laid(piece, u);
            let at = match () {
                () if y < 0.0 => 0,
                () if y > laid.height() => self.blocks(piece)[u].inline.text.len(),
                () => laid.hit(x, y),
            };
            return Caret {
                piece,
                block: u,
                at,
            };
        }
    }

    fn pad(&self, piece: usize, b: usize) -> f32 {
        if matches!(self.blocks(piece)[b].kind, Kind::Code { .. }) {
            CODE_PAD
        } else {
            0.0
        }
    }

    /// A press at `x`, `y` of the row of `block` (`None`: fixed text,
    /// which sends the caret to the nearest text to edit).
    pub fn press(&mut self, piece: usize, block: Option<usize>, x: f32, y: f32, extend: bool) {
        self.goal = None;
        let Some(block) = block else {
            return self.editor.jump_from_fixed(piece);
        };
        let c = self.locate(piece, block, x, y);
        self.editor.place(c, extend);
    }

    /// "Edit as own text" on a generated block.
    pub fn take_over(&mut self, piece: usize) {
        self.goal = None;
        self.editor.take_over(piece, &self.drawn);
    }

    pub fn drag(&mut self, piece: usize, block: usize, x: f32, y: f32) {
        let c = self.locate(piece, block, x, y);
        self.editor.place(c, true);
    }

    /// Select the word at `x`, `y` (a double click).
    pub fn word(&mut self, piece: usize, block: usize, x: f32, y: f32) {
        let c = self.locate(piece, block, x, y);
        self.editor.select_word(c);
    }
}

/// The system clipboard; silently nothing where there is none (headless).
fn clipboard_set(text: &str) {
    if text.is_empty() {
        return;
    }
    if let Ok(mut c) = arboard::Clipboard::new() {
        let _ = c.set_text(text);
    }
}

fn clipboard_get() -> Option<String> {
    arboard::Clipboard::new().ok()?.get_text().ok()
}

/// `text` cut at the diagram sections of `drawn` (id, Markdown): the text
/// between them, and each section's id in its place.
fn split_drawn(text: &str, drawn: &[(String, String)]) -> Vec<Result<String, String>> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some((at, id, len)) = drawn
        .iter()
        .filter(|(_, section)| !section.is_empty())
        .filter_map(|(id, section)| {
            rest.find(section.as_str())
                .map(|at| (at, id, section.len()))
        })
        .min_by_key(|(at, ..)| *at)
    {
        out.push(Ok(rest[..at].to_string()));
        out.push(Err(id.clone()));
        rest = &rest[at + len..];
    }
    out.push(Ok(rest.to_string()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rich(md: &str) -> Rich {
        let mut r = Rich::new(md);
        r.set_width(400.0);
        r
    }

    #[test]
    fn rows_show_tables_as_rows_and_editable_blocks_with_the_caret() {
        use slint::Model;
        let md = "## 5. Bausteine\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n\nText **fett**.\n\n- eins\n- zwei\n\n---\n";
        let mut r = rich(md);
        let items = r.items("", &|_| None, &|_| None, &[]);
        let shape: Vec<(i32, bool, usize, String)> = items
            .iter()
            .map(|i| {
                (
                    i.kind,
                    i.fixed.row_count() > 0,
                    i.cells.row_count(),
                    i.marker.to_string(),
                )
            })
            .collect();
        assert_eq!(
            shape,
            [
                (5, false, 2, String::new()),
                (5, false, 2, String::new()),
                (0, false, 0, String::new()),
                (2, false, 0, "•".into()),
                (2, false, 0, "•".into()),
                (0, true, 0, String::new()),
                (0, false, 0, String::new()),
            ],
            "table rows, text, the rule as the reader shows it and the empty part after it; the heading shows above the editor"
        );
        assert!(
            items[0].cells.row_data(0).unwrap().header
                && !items[1].cells.row_data(0).unwrap().header
        );
        assert!(items[0].caret, "the caret in the first cell");
        assert!(items[6].empty, "the empty part shows where to type");
        let texts: Vec<String> = (0..items[2].runs.row_count())
            .map(|i| items[2].runs.row_data(i).unwrap().text.to_string())
            .collect();
        assert_eq!(texts, ["Text ", "fett", "."]);
    }

    /// Every line of a code block is drawn with its text and width; the
    /// line break is not part of the line (it made all but the last line
    /// zero wide, so only a sliver of each showed).
    #[test]
    fn each_line_of_a_code_block_is_drawn_whole() {
        use slint::Model;
        let md = "Text.\n\n```mermaid\nflowchart LR\n  A[Portal] --> B[APIM]\n  K -. Metadaten .-> B\n```\n";
        let mut r = rich(md);
        let items = r.items("", &|_| None, &|_| None, &[]);
        let code = items.iter().find(|i| i.kind == 4).expect("the code block");
        let runs: Vec<_> = code.runs.iter().collect();
        let texts: Vec<&str> = runs.iter().map(|r| r.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "flowchart LR",
                "  A[Portal] --> B[APIM]",
                "  K -. Metadaten .-> B"
            ]
        );
        assert!(runs.iter().all(|r| r.w > 50.0), "every line has its width");
        assert!(
            runs[0].y < runs[1].y && runs[1].y < runs[2].y,
            "one below the other"
        );
    }

    /// Own Mermaid is drawn while the caret is elsewhere and shows its
    /// source to edit once the caret is in it; other code stays code.
    #[test]
    fn own_mermaid_is_drawn_until_the_caret_is_in_it() {
        let md = "Text.\n\n```mermaid\nflowchart LR\n  A --> B\n```\n\n```rust\nfn x() {}\n```\n";
        let mut r = rich(md);
        let own = |source: &str| {
            Some(ArcFigure {
                id: source.into(),
                ..Default::default()
            })
        };
        let kinds = |r: &mut Rich| -> Vec<(i32, String)> {
            r.items("", &|_| None, &own, &[])
                .iter()
                .map(|i| (i.kind, i.figure.id.to_string()))
                .collect()
        };
        assert_eq!(
            kinds(&mut r),
            [
                (0, String::new()),
                (8, "flowchart LR\n  A --> B".into()),
                (4, String::new()),
            ]
        );
        r.key("down", false, false, false);
        assert_eq!(
            kinds(&mut r),
            [(0, String::new()), (4, String::new()), (4, String::new())],
            "the caret in it shows the source"
        );
    }

    /// Reading draws every own diagram and shows no caret or selection.
    #[test]
    fn reading_draws_every_diagram_and_shows_no_caret() {
        let mut r = rich("Text.\n\n```mermaid\nflowchart LR\n  A --> B\n```\n");
        r.key("down", false, false, false);
        r.reading = true;
        let own = |source: &str| {
            Some(ArcFigure {
                id: source.into(),
                ..Default::default()
            })
        };
        let items = r.items("", &|_| None, &own, &[]);
        assert_eq!(items.iter().map(|i| i.kind).collect::<Vec<_>>(), [0, 8]);
        assert!(items.iter().all(|i| !i.caret));
    }

    /// A generated block's diagram section is drawn in its place: the
    /// text before it, the drawing, the text after it.
    #[test]
    fn generated_diagrams_are_drawn_where_their_section_is() {
        use slint::Model;
        let section = "### Pakete\n\n```mermaid\nflowchart LR\n```\n";
        let md = format!(
            "## 5. Bausteine\n\n<!-- archguard:arc42:05:start -->\nVorher.\n\n{section}\nNachher.\n<!-- archguard:arc42:05:end -->\n\nEigener Text.\n"
        );
        let mut r = rich(&md);
        let figure = |id: &str| {
            Some(ArcFigure {
                id: id.into(),
                ..Default::default()
            })
        };
        let drawn = [("packages".to_string(), section.to_string())];
        let items = r.items("", &figure, &|_| None, &drawn);
        let shape: Vec<(i32, usize, String)> = items
            .iter()
            .map(|i| (i.kind, i.fixed.row_count(), i.figure.id.to_string()))
            .collect();
        assert_eq!(
            shape,
            [
                (0, 1, String::new()),
                (7, 0, "packages".into()),
                (0, 1, String::new()),
                (0, 0, String::new()),
            ],
            "Vorher, the drawing, Nachher, the own text"
        );
        assert!(items[1].generated);
        assert_eq!(
            split_drawn("abc", &[("x".into(), "zz".into())]),
            [Ok("abc".to_string())]
        );
    }

    #[test]
    fn clicks_and_arrows_find_table_cells_and_the_space_between_rows() {
        let mut r = rich("| a | b |\n| --- | --- |\n| 1 | 2 |\n\nText\n");
        let piece = r.editor.focus.piece;
        // A click in the right half of the header row: its second cell.
        r.press(piece, Some(0), 300.0, 8.0, false);
        assert_eq!(r.editor.focus.block, 1);
        r.key("down", false, false, false);
        assert_eq!(r.editor.focus.block, 3, "the same column one row down");
        r.key("down", false, false, false);
        assert_eq!(
            r.editor.block(r.editor.focus).inline.text,
            "Text",
            "out of the table"
        );
        r.key("up", false, false, false);
        assert!(r.editor.in_table(), "back into the last row");
        // A drag into the space between table and text ends in the table,
        // below that space in the text.
        let height = r.row_height(piece, 2);
        r.drag(piece, 2, 10.0, height + 5.0);
        assert!(r.editor.in_table(), "the space belongs to the row above");
        r.drag(piece, 2, 10.0, height + 15.0);
        assert_eq!(r.editor.focus.block, 4, "the paragraph below the space");
        r.drag(piece, 4, 10.0, -3.0);
        assert!(
            r.editor.in_table(),
            "the space above the paragraph belongs to the table"
        );
    }

    #[test]
    fn keys_move_edit_and_select_across_blocks() {
        let mut r = rich("eins\n\nzwei\n");
        assert_eq!(r.key("end", false, false, false), Key::Done);
        assert_eq!(r.editor.focus.at, 4);
        r.key("down", false, false, false);
        // "zwei" is wider: the end of "eins" lies inside it.
        assert_eq!(
            (r.editor.focus.block, r.editor.focus.at),
            (1, 3),
            "down keeps the column"
        );
        r.key("end", false, false, false);
        r.key("up", true, false, false);
        assert_eq!(r.editor.selected_text(), "\nzwei");
        r.type_text("!");
        assert_eq!(r.markdown(), "eins!\n");
        assert_eq!(r.key("b", false, false, true), Key::Done);
        r.type_text("x");
        assert_eq!(r.markdown(), "eins!**x**\n");
        assert_eq!(r.key("k", false, false, true), Key::Link);
        assert_eq!(r.key("q", false, false, false), Key::Ignored);
        r.key("z", false, false, true);
        assert_eq!(r.markdown(), "eins!\n");
    }

    #[test]
    fn a_drag_past_a_block_selects_into_the_next() {
        let mut r = rich("eins\n\nzwei\n");
        r.items("", &|_| None, &|_| None, &[]);
        r.press(r.editor.focus.piece, Some(0), 0.0, 5.0, false);
        let below = r.laid(r.editor.focus.piece, 0).height() + 15.0;
        r.drag(r.editor.focus.piece, 0, 200.0, below);
        assert_eq!(r.editor.selected_text(), "eins\nzwei");
        r.word(r.editor.focus.piece, 1, 5.0, 5.0);
        assert_eq!(r.editor.selected_text(), "zwei");
    }
}
