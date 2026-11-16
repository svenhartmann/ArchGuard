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
use crate::ui::{RichItem, RichRect, RichRun, RichTools};

thread_local! {
    /// Fonts and layout caches of the window's thread.
    static FONTS: RefCell<Fonts> = RefCell::new(Fonts::default());
}

/// Space a code block's background reaches below its text (as in the view).
const CODE_PAD: f32 = 8.0;

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
}

/// Space above a block and the left edge of its text.
fn geometry(blocks: &[Block], b: usize, entries: &[Option<(usize, Option<u64>)>]) -> (f32, f32) {
    let prev = b.checked_sub(1).map(|p| &blocks[p].kind);
    let top = match (&blocks[b].kind, prev) {
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
        }
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

    /// Block `b` of `piece` laid out at the editor's width (cached).
    fn laid(&mut self, piece: usize, b: usize) -> &Laid {
        let block = self.blocks(piece)[b].clone();
        let (_, indent) = self.place(piece, b);
        let pad = if matches!(block.kind, Kind::Code { .. }) {
            12.0
        } else {
            0.0
        };
        let width = (self.width - indent - pad).max(40.0);
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
    /// diagram it cannot draw.
    pub fn items(&mut self, note: &str) -> Vec<RichItem> {
        let selection = self.editor.selection();
        let focus = self.editor.focus;
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
                    // The chapter heading shows above the editor.
                    let shown: Vec<markdown::Block> = markdown::parse(&text)
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
                            ..Default::default()
                        });
                    }
                    continue;
                }
                Ok(blocks) => blocks,
            };
            let entries = doc::list_entries(&blocks);
            for (b, block) in blocks.iter().enumerate() {
                let (top, indent) = geometry(&blocks, b, &entries);
                let laid = self.laid(p, b);
                let runs: Vec<RichRun> = laid
                    .runs()
                    .into_iter()
                    .map(|r| RichRun {
                        text: r.text.into(),
                        x: r.x,
                        y: r.y,
                        w: r.width,
                        h: r.height,
                        size: r.size,
                        weight: r.weight,
                        italic: r.look.italic,
                        code: r.look.code,
                        link: r.look.link,
                    })
                    .collect();
                let selected: Vec<RichRect> = match selection {
                    Some((s, e)) if s.piece == p && (s.block..=e.block).contains(&b) => {
                        let from = if b == s.block { s.at } else { 0 };
                        let to = if b == e.block {
                            e.at
                        } else {
                            block.inline.text.len()
                        };
                        let mut rects: Vec<RichRect> = laid
                            .selection(from..to)
                            .into_iter()
                            .map(rich_rect)
                            .collect();
                        // A selected line break shows as a narrow box.
                        if b != e.block {
                            let end = laid.caret(block.inline.text.len());
                            rects.push(RichRect {
                                x: end.x,
                                y: end.y,
                                w: 6.0,
                                h: end.h,
                            });
                        }
                        rects
                    }
                    _ => Vec::new(),
                };
                let first = laid.caret(0);
                let marker = match entries[b] {
                    Some((_, Some(n))) => format!("{n}."),
                    Some((depth, None)) if depth % 2 == 1 => "◦".into(),
                    Some(_) => "•".into(),
                    None => String::new(),
                };
                let here = focus.piece == p && focus.block == b;
                out.push(RichItem {
                    fixed: model(Vec::new()),
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
                });
            }
        }
        out
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

    /// One line up (-1) or down (+1), keeping the x the moves started at.
    fn vertical(&mut self, delta: isize, extend: bool) {
        let c = self.editor.focus;
        let (_, indent) = self.place(c.piece, c.block);
        let x = match self.goal {
            Some(x) => x,
            None => self.laid(c.piece, c.block).caret(c.at).x + indent,
        };
        self.goal = Some(x);
        let laid = self.laid(c.piece, c.block);
        let to = match laid.line_at(c.at, delta, x - indent) {
            Some(at) => Caret { at, ..c },
            None => {
                let next = if delta < 0 {
                    self.editor.prev_block(c)
                } else {
                    self.editor.next_block(c)
                };
                match next {
                    Some(n) => {
                        let (_, indent) = self.place(n.piece, n.block);
                        let laid = self.laid(n.piece, n.block);
                        let y = if delta < 0 { laid.height() - 1.0 } else { 1.0 };
                        Caret {
                            at: laid.hit(x - indent, y),
                            ..n
                        }
                    }
                    // Past the first or last line: to the block's start or end.
                    None if delta < 0 => Caret { at: 0, ..c },
                    None => Caret {
                        at: self.blocks(c.piece)[c.block].inline.text.len(),
                        ..c
                    },
                }
            }
        };
        self.editor.place(to, extend);
    }

    /// The caret at `x`, `y` of block `b`'s text; a point above or below it
    /// (a drag) lands in the blocks of the same part there.
    fn locate(&mut self, piece: usize, mut b: usize, mut x: f32, mut y: f32) -> Caret {
        let len = self.blocks(piece).len();
        if len == 0 {
            return self.editor.focus;
        }
        b = b.min(len - 1);
        loop {
            let (top, indent) = self.place(piece, b);
            if y < 0.0 && b > 0 {
                let (_, prev_indent) = self.place(piece, b - 1);
                let prev = self.laid(piece, b - 1).height() + self.pad(piece, b - 1);
                y += top + prev;
                x += indent - prev_indent;
                b -= 1;
                continue;
            }
            let height = self.laid(piece, b).height();
            if y > height + self.pad(piece, b) && b + 1 < len {
                let (next_top, next_indent) = self.place(piece, b + 1);
                y -= height + self.pad(piece, b) + next_top;
                x += indent - next_indent;
                b += 1;
                continue;
            }
            let laid = self.laid(piece, b);
            let at = match () {
                () if y < 0.0 => 0,
                () if y > laid.height() => self.blocks(piece)[b].inline.text.len(),
                () => laid.hit(x, y),
            };
            return Caret {
                piece,
                block: b,
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

    pub fn press(&mut self, piece: usize, block: usize, x: f32, y: f32, extend: bool) {
        self.goal = None;
        let c = self.locate(piece, block, x, y);
        self.editor.place(c, extend);
    }

    pub fn drag(&mut self, piece: usize, block: usize, x: f32, y: f32) {
        let c = self.locate(piece, block, x, y);
        self.editor.place(c, true);
    }

    /// Select the word at `x`, `y` (a double click).
    pub fn word(&mut self, piece: usize, block: usize, x: f32, y: f32) {
        let c = self.locate(piece, block, x, y);
        let r = self.laid(c.piece, c.block).word(x, y);
        self.editor.place(Caret { at: r.start, ..c }, false);
        self.editor.place(Caret { at: r.end, ..c }, true);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn rich(md: &str) -> Rich {
        let mut r = Rich::new(md);
        r.set_width(400.0);
        r
    }

    #[test]
    fn rows_show_fixed_pieces_and_editable_blocks_with_the_caret() {
        let md = "## 5. Bausteine\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n\nText **fett**.\n\n- eins\n- zwei\n";
        let mut r = rich(md);
        let items = r.items("");
        let shape: Vec<(i32, bool, String)> = items
            .iter()
            .map(|i| (i.kind, i.fixed.row_count() > 0, i.marker.to_string()))
            .collect();
        use slint::Model;
        assert_eq!(
            shape,
            [
                (0, true, String::new()),
                (0, false, String::new()),
                (2, false, "•".into()),
                (2, false, "•".into())
            ],
            "the table as the reader shows it, the heading above the editor"
        );
        assert!(
            items[1].caret && !items[2].caret,
            "the caret in the first editable block"
        );
        let texts: Vec<String> = (0..items[1].runs.row_count())
            .map(|i| items[1].runs.row_data(i).unwrap().text.to_string())
            .collect();
        assert_eq!(texts, ["Text ", "fett", "."]);
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
        r.items("");
        r.press(r.editor.focus.piece, 0, 0.0, 5.0, false);
        let below = r.laid(r.editor.focus.piece, 0).height() + 15.0;
        r.drag(r.editor.focus.piece, 0, 200.0, below);
        assert_eq!(r.editor.selected_text(), "eins\nzwei");
        r.word(r.editor.focus.piece, 1, 5.0, 5.0);
        assert_eq!(r.editor.selected_text(), "zwei");
    }
}
