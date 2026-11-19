//! A character grid for the terminal drawings: lines are stored as
//! direction bits per cell, so crossings and junctions get the right
//! box-drawing character; text and arrowheads overlay them.

/// How the drawing is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    /// Box-drawing characters; `false`: plain ASCII.
    pub unicode: bool,
    /// ANSI colours (only for a terminal with `NO_COLOR` unset).
    pub color: bool,
    /// The widest line wanted, in characters.
    pub width: usize,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            unicode: true,
            color: false,
            width: 100,
        }
    }
}

pub const N: u8 = 1;
pub const E: u8 = 2;
pub const S: u8 = 4;
pub const W: u8 = 8;

/// ANSI colour of a cell; 0: none.
pub const RED: u8 = 31;
pub const BLUE: u8 = 34;
pub const MAGENTA: u8 = 35;
pub const YELLOW: u8 = 33;
pub const DIM: u8 = 90;

#[derive(Debug, Clone, Copy, Default)]
struct Cell {
    lines: u8,
    ch: Option<char>,
    color: u8,
}

pub struct Canvas {
    rows: Vec<Vec<Cell>>,
    pub style: Style,
}

impl Canvas {
    pub fn new(style: Style) -> Self {
        Canvas {
            rows: Vec::new(),
            style,
        }
    }

    fn cell(&mut self, x: usize, y: usize) -> &mut Cell {
        if self.rows.len() <= y {
            self.rows.resize(y + 1, Vec::new());
        }
        let row = &mut self.rows[y];
        if row.len() <= x {
            row.resize(x + 1, Cell::default());
        }
        &mut row[x]
    }

    /// Whether nothing is drawn at `(x, y)`.
    pub fn free(&self, x: usize, y: usize) -> bool {
        self.rows
            .get(y)
            .and_then(|r| r.get(x))
            .is_none_or(|c| c.lines == 0 && c.ch.is_none())
    }

    /// Whether `(x, y)` holds exactly the lines `bits` and no text.
    pub fn only(&self, x: usize, y: usize, bits: u8) -> bool {
        self.rows
            .get(y)
            .and_then(|r| r.get(x))
            .is_some_and(|c| c.lines == bits && c.ch.is_none())
    }

    /// Adds line directions to a cell; a colour, if any, wins over none.
    pub fn line(&mut self, x: usize, y: usize, bits: u8, color: u8) {
        let c = self.cell(x, y);
        c.lines |= bits;
        if color != 0 {
            c.color = color;
        }
    }

    /// A vertical line from `y0` to `y1` (both included).
    pub fn vline(&mut self, x: usize, y0: usize, y1: usize, color: u8) {
        let (a, b) = (y0.min(y1), y0.max(y1));
        for y in a..=b {
            let up = if y > a { N } else { 0 };
            let down = if y < b { S } else { 0 };
            self.line(x, y, up | down, color);
        }
    }

    /// A horizontal line from `x0` to `x1` (both included).
    pub fn hline(&mut self, y: usize, x0: usize, x1: usize, color: u8) {
        let (a, b) = (x0.min(x1), x0.max(x1));
        for x in a..=b {
            let left = if x > a { W } else { 0 };
            let right = if x < b { E } else { 0 };
            self.line(x, y, left | right, color);
        }
    }

    pub fn put(&mut self, x: usize, y: usize, ch: char, color: u8) {
        let c = self.cell(x, y);
        c.ch = Some(ch);
        c.color = color;
    }

    pub fn text(&mut self, x: usize, y: usize, text: &str, color: u8) {
        for (i, ch) in text.chars().enumerate() {
            self.put(x + i, y, ch, color);
        }
    }

    /// A box with its top-left corner at `(x, y)`.
    pub fn frame(&mut self, x: usize, y: usize, w: usize, h: usize, color: u8) {
        let (r, b) = (x + w - 1, y + h - 1);
        self.hline(y, x, r, color);
        self.hline(b, x, r, color);
        self.vline(x, y, b, color);
        self.vline(r, y, b, color);
    }

    /// The picture, trailing spaces trimmed, one `\n` per row.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for row in &self.rows {
            let mut line = String::new();
            let mut open = 0;
            for c in row {
                let ch = c.ch.unwrap_or_else(|| glyph(c.lines, self.style.unicode));
                let color = if self.style.color && ch != ' ' {
                    c.color
                } else {
                    open
                };
                if self.style.color && color != open {
                    line.push_str(&ansi(color));
                    open = color;
                }
                line.push(ch);
            }
            if open != 0 {
                line.push_str(&ansi(0));
            }
            out.push_str(line.trim_end());
            out.push('\n');
        }
        out
    }
}

fn ansi(color: u8) -> String {
    format!("\x1b[{color}m")
}

/// The character for a set of line directions.
fn glyph(lines: u8, unicode: bool) -> char {
    if !unicode {
        return match lines {
            0 => ' ',
            l if l & (E | W) == 0 => '|',
            l if l & (N | S) == 0 => '-',
            _ => '+',
        };
    }
    const GLYPHS: [char; 16] = [
        ' ', '│', '─', '└', '│', '│', '┌', '├', '─', '┘', '─', '┴', '┐', '┤', '┬', '┼',
    ];
    GLYPHS[lines as usize]
}

/// `text` cut to `max` characters, the cut marked.
pub fn fit(text: &str, max: usize, unicode: bool) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let keep: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{keep}{}", if unicode { '…' } else { '~' })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crossing_lines_join_and_ascii_uses_plus() {
        let mut c = Canvas::new(Style::default());
        c.hline(1, 0, 4, 0);
        c.vline(2, 0, 2, 0);
        c.frame(5, 0, 3, 3, 0);
        assert_eq!(c.render(), "  │  ┌─┐\n──┼──│ │\n  │  └─┘\n");
        c.style.unicode = false;
        assert_eq!(c.render(), "  |  +-+\n--+--| |\n  |  +-+\n");
    }

    #[test]
    fn colour_is_written_only_when_asked() {
        let mut c = Canvas::new(Style {
            color: true,
            ..Style::default()
        });
        c.text(0, 0, "ab", RED);
        assert_eq!(c.render(), "\x1b[31mab\x1b[0m\n");
        assert_eq!(fit("abcdef", 4, false), "abc~");
    }
}
