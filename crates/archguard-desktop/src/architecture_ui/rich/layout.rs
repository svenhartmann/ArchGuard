//! Text layout of the rich-text editor with parley, the engine Slint lays
//! out its own text with, so lines break where a Slint `Text` of the same
//! font would. Gives each block's runs (a stretch of one line in one look,
//! drawn as one `Text`), the caret and selection boxes, and the byte under
//! a point or a line up or down.

use std::collections::BTreeSet;
use std::ops::Range;

use parley::{
    Affinity, Alignment, AlignmentOptions, Cursor, FontContext, FontStyle, FontWeight, Layout,
    LayoutContext, LineHeight, Selection, StyleProperty,
};

use super::doc::{Block, Kind};
use super::inline::Mark;

/// How a run is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Look {
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub link: bool,
}

/// Families and sizes from the window's theme.
#[derive(Debug, Clone, PartialEq)]
pub struct Style {
    pub family: String,
    pub mono: String,
    pub body: f32,
    pub small: f32,
    pub section: f32,
    pub page: f32,
    /// Weights of headings up to level 3 and below.
    pub title: f32,
    pub strong: f32,
    /// The window's scale factor: Slint shapes text in physical pixels,
    /// and fonts with optical sizes run wider or narrower there.
    pub scale: f32,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            family: "sans-serif".into(),
            mono: "monospace".into(),
            body: 14.0,
            small: 12.0,
            section: 17.0,
            page: 21.0,
            title: 700.0,
            strong: 600.0,
            scale: 1.0,
        }
    }
}

/// The font and layout caches; one per window.
#[derive(Default)]
pub struct Fonts {
    fcx: FontContext,
    lcx: LayoutContext<Look>,
}

/// A stretch of one line in one look, for one `Text`.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub text: String,
    /// Left edge and the line's top, in the block.
    pub x: f32,
    pub y: f32,
    pub width: f32,
    /// The line's height and its baseline from `y`.
    pub height: f32,
    pub baseline: f32,
    pub size: f32,
    /// Bold, the block's heading weight, or regular.
    pub weight: f32,
    pub look: Look,
}

/// A box in a block: the caret or a piece of the selection.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// One block laid out at one width.
pub struct Laid {
    layout: Layout<Look>,
    /// The block's text length; an empty block lays out a space.
    len: usize,
    pub size: f32,
    /// A heading's weight.
    strong: Option<f32>,
    looks: Vec<(Range<usize>, Look)>,
    text: String,
    /// Physical pixels per logical one: the layout is in physical pixels.
    scale: f32,
}

/// Font size and heading weight of a block's kind, as the reader's
/// (`architecture_blocks.slint`).
fn size_of(kind: &Kind, style: &Style) -> (f32, Option<f32>) {
    match kind {
        Kind::Heading(level) if *level <= 2 => (style.page, Some(style.title)),
        Kind::Heading(3) => (style.section, Some(style.title)),
        Kind::Heading(_) => (style.body, Some(style.strong)),
        Kind::Code { .. } => (style.small, None),
        _ => (style.body, None),
    }
}

/// The looks of `block` per stretch of its text.
fn looks(block: &Block) -> Vec<(Range<usize>, Look)> {
    let inline = &block.inline;
    let code = matches!(block.kind, Kind::Code { .. });
    let mut cuts: BTreeSet<usize> = [0, inline.text.len()].into();
    for s in &inline.spans {
        cuts.insert(s.range.start);
        cuts.insert(s.range.end);
    }
    let cuts: Vec<usize> = cuts.into_iter().collect();
    cuts.windows(2)
        .filter(|w| w[0] < w[1])
        .map(|w| {
            let marks = inline.marks_at(w[0]);
            let look = Look {
                bold: marks.contains(&Mark::Bold),
                italic: marks.contains(&Mark::Italic),
                code: code || marks.contains(&Mark::Code),
                link: marks.iter().any(|m| matches!(m, Mark::Link(_))),
            };
            (w[0]..w[1], look)
        })
        .collect()
}

impl Fonts {
    /// Lay out `block` to `width`.
    pub fn lay_out(&mut self, block: &Block, width: f32, style: &Style) -> Laid {
        let (size, strong) = size_of(&block.kind, style);
        let len = block.inline.text.len();
        // An empty block still has a line for the caret.
        let text = if len == 0 {
            " "
        } else {
            block.inline.text.as_str()
        };
        let looks = looks(block);
        let code = matches!(block.kind, Kind::Code { .. });
        let scale = style.scale.max(0.25);
        // As Slint's own text (i-slint-core `sharedparley/shaping.rs`).
        let mut b = self.lcx.ranged_builder(&mut self.fcx, text, scale, false);
        let family = if code { &style.mono } else { &style.family };
        // Slint's list: the family, then its fallbacks sans-serif and
        // system-ui (i-slint-common `FALLBACK_FAMILIES`), so a text with
        // no family (Linux) or a glyph the family lacks gets Slint's font.
        let list = match family.as_str() {
            "" => "sans-serif, system-ui".to_string(),
            family => format!("\"{}\", sans-serif, system-ui", family.replace('"', "")),
        };
        b.push_default(StyleProperty::FontFamily(list.as_str().into()));
        b.push_default(StyleProperty::FontSize(size));
        // A Slint `Text`'s line: the font's own line height.
        b.push_default(StyleProperty::LineHeight(LineHeight::MetricsRelative(1.0)));
        if let Some(weight) = strong {
            b.push_default(StyleProperty::FontWeight(FontWeight::new(weight)));
        }
        for (range, look) in &looks {
            if look.bold {
                b.push(StyleProperty::FontWeight(FontWeight::BOLD), range.clone());
            }
            if look.italic {
                b.push(StyleProperty::FontStyle(FontStyle::Italic), range.clone());
            }
            if look.code && !code {
                b.push(
                    StyleProperty::FontFamily(style.mono.as_str().into()),
                    range.clone(),
                );
            }
            b.push(StyleProperty::Brush(*look), range.clone());
        }
        let mut layout = b.build(text);
        layout.break_all_lines(Some(width.max(1.0) * scale));
        layout.align(Alignment::Start, AlignmentOptions::default());
        Laid {
            layout,
            len,
            size,
            strong,
            looks,
            text: block.inline.text.clone(),
            scale,
        }
    }
}

impl Laid {
    /// A layout box in logical pixels.
    fn rect(&self, b: parley::BoundingBox) -> Rect {
        let s = f64::from(self.scale);
        Rect {
            x: (b.x0 / s) as f32,
            y: (b.y0 / s) as f32,
            w: ((b.x1 - b.x0) / s) as f32,
            h: ((b.y1 - b.y0) / s) as f32,
        }
    }

    pub fn height(&self) -> f32 {
        self.layout.height() / self.scale
    }

    fn cursor(&self, at: usize, affinity: Affinity) -> Cursor {
        Cursor::from_byte_index(&self.layout, at.min(self.len), affinity)
    }

    /// The x of byte `at` on the line it starts (or ends with `upstream`).
    fn x(&self, at: usize, upstream: bool) -> f32 {
        let affinity = if upstream {
            Affinity::Upstream
        } else {
            Affinity::Downstream
        };
        self.rect(self.cursor(at, affinity).geometry(&self.layout, 0.0))
            .x
    }

    /// The runs to draw: each line cut at every change of look.
    pub fn runs(&self) -> Vec<Run> {
        let mut out = Vec::new();
        for line in self.layout.lines() {
            let m = line.metrics();
            let s = self.scale;
            let lr = line.text_range();
            for (range, look) in &self.looks {
                let from = range.start.max(lr.start);
                let to = range.end.min(lr.end).min(self.len);
                if from >= to {
                    continue;
                }
                let x = self.x(from, false);
                out.push(Run {
                    text: self.text[from..to].to_string(),
                    x,
                    y: m.block_min_coord / s,
                    width: self.x(to, true) - x,
                    height: m.line_height / s,
                    baseline: (m.baseline - m.block_min_coord) / s,
                    size: self.size,
                    weight: match (look.bold, self.strong) {
                        (true, _) => 700.0,
                        (false, Some(w)) => w,
                        (false, None) => 400.0,
                    },
                    look: *look,
                });
            }
        }
        out
    }

    /// The caret before byte `at`.
    pub fn caret(&self, at: usize) -> Rect {
        self.rect(
            self.cursor(at, Affinity::Downstream)
                .geometry(&self.layout, 1.5 * self.scale),
        )
    }

    /// The boxes of the selected bytes `r`, one per line.
    pub fn selection(&self, r: Range<usize>) -> Vec<Rect> {
        if r.is_empty() || self.len == 0 {
            return Vec::new();
        }
        let s = Selection::new(
            self.cursor(r.start, Affinity::Downstream),
            self.cursor(r.end, Affinity::Upstream),
        );
        s.geometry(&self.layout)
            .into_iter()
            .map(|(b, _)| self.rect(b))
            .collect()
    }

    /// The byte under the point `x`, `y` (block coordinates).
    pub fn hit(&self, x: f32, y: f32) -> usize {
        let s = self.scale;
        Cursor::from_point(&self.layout, x * s, y * s)
            .index()
            .min(self.len)
    }

    /// The word under a point (a double click).
    pub fn word(&self, x: f32, y: f32) -> Range<usize> {
        let s = self.scale;
        let r = Selection::word_from_point(&self.layout, x * s, y * s).text_range();
        r.start.min(self.len)..r.end.min(self.len)
    }

    /// The byte at `x` one line up (`delta` -1) or down (+1) from the line
    /// of `at`; `None` past the first or last line.
    pub fn line_at(&self, at: usize, delta: isize, x: f32) -> Option<usize> {
        let line = self.line_of(at);
        let target = line.checked_add_signed(delta)?;
        let l = self.layout.lines().nth(target)?;
        let m = l.metrics();
        let middle = (m.block_min_coord + m.block_max_coord) / 2.0 / self.scale;
        Some(self.hit(x, middle))
    }

    fn line_of(&self, at: usize) -> usize {
        let y = self.caret(at).y * self.scale;
        self.layout
            .lines()
            .position(|l| l.metrics().block_max_coord > y + 0.5)
            .unwrap_or(0)
    }

    /// Start and end bytes of the line `at` is on (Home, End).
    pub fn line_range(&self, at: usize) -> Range<usize> {
        let r = self
            .layout
            .lines()
            .nth(self.line_of(at))
            .map_or(0..self.len, |l| l.text_range());
        // The end stays before a line's trailing space, as Home/End do.
        let end = if r.end < self.len && self.text[..r.end].ends_with(' ') {
            r.end - 1
        } else {
            r.end
        };
        r.start.min(self.len)..end.min(self.len)
    }

    #[cfg(test)]
    pub fn lines(&self) -> usize {
        self.layout.lines().count()
    }
}

#[cfg(test)]
mod tests {
    use super::super::inline::Inline;
    use super::*;

    fn block(md: &str) -> Block {
        Block {
            kind: Kind::Paragraph,
            inline: Inline::parse(md),
        }
    }

    #[test]
    fn a_long_paragraph_wraps_and_every_caret_hits_its_own_byte() {
        let mut fonts = Fonts::default();
        let b = block(
            "Ein **langer** Absatz mit *mehreren* Wörtern, der bei schmaler Breite umbricht.",
        );
        // Slint lays out in physical pixels: a Retina window at scale 2.
        // 1.25: a common Windows display setting.
        for scale in [1.0, 1.25, 2.0] {
            let style = Style {
                scale,
                ..Style::default()
            };
            let laid = fonts.lay_out(&b, 160.0, &style);
            assert!(laid.lines() > 2, "{} lines", laid.lines());
            let text = &b.inline.text;
            for (at, _) in text.char_indices().skip(1) {
                let c = laid.caret(at);
                assert_eq!(
                    laid.hit(c.x + 0.1, c.y + c.h / 2.0),
                    at,
                    "byte {at} at {scale}"
                );
            }
            for run in laid.runs() {
                assert!(run.x + run.width <= 160.5, "{run:?}");
                assert!(!run.text.is_empty());
                assert!(run.height < 30.0, "logical pixels: {run:?}");
            }
            let joined: String = laid.runs().iter().map(|r| r.text.as_str()).collect();
            assert_eq!(&joined, text, "the runs are the text");
        }
    }

    #[test]
    fn runs_change_with_the_look_and_lines_move_up_and_down() {
        let mut fonts = Fonts::default();
        let b = block("normal **fett** normal");
        let laid = fonts.lay_out(&b, 1000.0, &Style::default());
        let looks: Vec<(String, bool)> = laid
            .runs()
            .into_iter()
            .map(|r| (r.text, r.look.bold))
            .collect();
        assert_eq!(
            looks,
            [
                ("normal ".into(), false),
                ("fett".into(), true),
                (" normal".into(), false)
            ]
        );
        assert_eq!(laid.line_at(3, 1, 0.0), None, "one line only");
        let narrow = fonts.lay_out(&block("aaa bbb ccc ddd eee fff"), 40.0, &Style::default());
        let x = narrow.caret(1).x;
        let down = narrow.line_at(1, 1, x).expect("a second line");
        assert!(narrow.caret(down).y > narrow.caret(1).y);
        let up = narrow.line_at(down, -1, x);
        assert_eq!(up.map(|b| narrow.line_of(b)), Some(0));
    }

    #[test]
    fn an_empty_block_has_a_line_for_the_caret() {
        let mut fonts = Fonts::default();
        let laid = fonts.lay_out(&block(""), 300.0, &Style::default());
        assert!(laid.height() > 10.0 && laid.caret(0).h > 10.0);
        assert_eq!(laid.hit(200.0, 5.0), 0);
        assert!(laid.runs().is_empty() && laid.selection(0..0).is_empty());
    }
}
