//! Formatted text of one block: plain text with marks (bold, italic, code,
//! link) over byte ranges, read from and written back to the inline
//! Markdown the arc42 chapters use (`**b**`, `*i*`, `` `c` ``, `[l](u)`).
//!
//! `*` emphasis is read as CommonMark does (delimiter runs, flanking, the
//! rule of three), so what the editor writes reads the same in Slint's
//! `StyledText`, on GitHub and in the web viewer's renderer. Code spans
//! are single backticks and links `[label](target)`, as the exporter
//! writes them. The writer ([`Inline::to_markdown`]) only produces what the
//! reader reads back as the same marks.

use std::collections::BTreeSet;
use std::ops::Range;

use super::super::markdown;

/// The kinds of marks.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Mark {
    Link(String),
    Bold,
    Italic,
    Code,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    /// Bytes of [`Inline::text`].
    pub range: Range<usize>,
    pub mark: Mark,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inline {
    pub text: String,
    /// Sorted by start, merged, never empty ([`Inline::tidy`]).
    pub spans: Vec<Span>,
}

impl Inline {
    pub fn plain(text: &str) -> Self {
        Inline {
            text: text.to_string(),
            spans: Vec::new(),
        }
    }

    /// Read inline Markdown.
    pub fn parse(md: &str) -> Self {
        let chars: Vec<char> = md.chars().collect();
        let mut out = scope(&chars);
        out.normalize();
        out
    }

    /// The marks on the character that starts at byte `at`.
    pub fn marks_at(&self, at: usize) -> BTreeSet<&Mark> {
        self.spans
            .iter()
            .filter(|s| s.range.contains(&at))
            .map(|s| &s.mark)
            .collect()
    }

    /// The one form of a formatted text: spans clipped to the text and
    /// without spaces at their ends (`** a**` is no emphasis; a space
    /// could not open a code span after it), no backticks inside code
    /// (they cannot be written there), links that do not overlap (a later
    /// link replaces earlier ones over its range), spans of one mark merged
    /// where they touch or only spaces lie between (a code span split
    /// around a link, two links to one target), and no empty spans.
    pub fn normalize(&mut self) {
        self.clean(true);
    }

    /// The form while editing: as [`Inline::normalize`], but spaces at the
    /// ends of a mark stay marked (typing on after `**bold **` stays bold)
    /// and only touching spans merge.
    pub fn tidy(&mut self) {
        self.clean(false);
    }

    fn clean(&mut self, trim: bool) {
        let text = &self.text;
        let mut given: Vec<Span> = Vec::new();
        for s in std::mem::take(&mut self.spans) {
            if matches!(s.mark, Mark::Link(_)) {
                given = given
                    .into_iter()
                    .flat_map(|l| match l.mark {
                        Mark::Link(_) => cut(l, &s.range),
                        _ => vec![l],
                    })
                    .collect();
            }
            given.push(s);
        }
        let mut spans: Vec<Span> = Vec::new();
        for s in given {
            let mut r = s.range.start.min(text.len())..s.range.end.min(text.len());
            if trim {
                let inner = &text[r.clone()];
                let start = r.start + (inner.len() - inner.trim_start().len());
                // Only spaces: empty, dropped below.
                r = start..(r.start + inner.trim_end().len()).max(start);
            }
            if s.mark == Mark::Code {
                let mut from = r.start;
                for (i, _) in text[r.clone()].match_indices('`') {
                    spans.push(Span {
                        range: from..r.start + i,
                        mark: Mark::Code,
                    });
                    from = r.start + i + 1;
                }
                spans.push(Span {
                    range: from..r.end,
                    mark: Mark::Code,
                });
            } else {
                spans.push(Span {
                    range: r,
                    mark: s.mark,
                });
            }
        }
        spans.retain(|s| !s.range.is_empty());
        spans.sort_by(|a, b| (&a.mark, a.range.start).cmp(&(&b.mark, b.range.start)));
        let mut merged: Vec<Span> = Vec::new();
        for s in spans {
            match merged.last_mut() {
                Some(last)
                    if last.mark == s.mark
                        && (s.range.start <= last.range.end
                            || (trim && text[last.range.end..s.range.start].trim().is_empty())) =>
                {
                    last.range.end = last.range.end.max(s.range.end);
                }
                _ => merged.push(s),
            }
        }
        merged.sort_by(|a, b| (a.range.start, &a.mark).cmp(&(b.range.start, &b.mark)));
        self.spans = merged;
    }

    /// Insert `s` at byte `at`; the new text carries exactly `marks`.
    pub fn insert(&mut self, at: usize, s: &str, marks: &BTreeSet<Mark>) {
        let n = s.len();
        self.text.insert_str(at, s);
        for span in &mut self.spans {
            if span.range.start >= at {
                span.range.start += n;
            }
            if span.range.end > at {
                span.range.end += n;
            }
        }
        let new = at..at + n;
        self.spans = std::mem::take(&mut self.spans)
            .into_iter()
            .flat_map(|span| match marks.contains(&span.mark) {
                true => vec![span],
                false => cut(span, &new),
            })
            .collect();
        self.spans.extend(marks.iter().map(|m| Span {
            range: new.clone(),
            mark: m.clone(),
        }));
        self.tidy();
    }

    /// Remove the bytes `r`.
    pub fn delete(&mut self, r: Range<usize>) {
        let n = r.len();
        self.text.replace_range(r.clone(), "");
        let map = |p: usize| match p {
            p if p <= r.start => p,
            p if p >= r.end => p - n,
            _ => r.start,
        };
        for span in &mut self.spans {
            span.range = map(span.range.start)..map(span.range.end);
        }
        self.tidy();
    }

    /// Whether every character in `r` but spaces has `mark` (any link for
    /// a link); false without such characters.
    pub fn has(&self, r: Range<usize>, mark: &Mark) -> bool {
        let mut chars = self.text[r.clone()]
            .char_indices()
            .filter(|(_, c)| !c.is_whitespace())
            .peekable();
        chars.peek().is_some()
            && chars.all(|(i, _)| {
                self.spans
                    .iter()
                    .any(|s| s.range.contains(&(r.start + i)) && same(&s.mark, mark))
            })
    }

    /// Mark the bytes `r`; a link replaces links there.
    pub fn add(&mut self, r: Range<usize>, mark: Mark) {
        self.spans.push(Span { range: r, mark });
        self.tidy();
    }

    /// Take `mark` (any link for a link) off the bytes `r`.
    pub fn remove(&mut self, r: Range<usize>, mark: &Mark) {
        self.spans = std::mem::take(&mut self.spans)
            .into_iter()
            .flat_map(|s| match same(&s.mark, mark) {
                true => cut(s, &r),
                false => vec![s],
            })
            .collect();
        self.tidy();
    }

    /// The marks typing at `at` carries on: those of the character before
    /// (at the start: after), never a link.
    pub fn typing(&self, at: usize) -> BTreeSet<Mark> {
        let before = self.text[..at]
            .chars()
            .next_back()
            .map(|c| at - c.len_utf8());
        self.marks_at(before.unwrap_or(0))
            .into_iter()
            .filter(|m| !matches!(m, Mark::Link(_)))
            .cloned()
            .collect()
    }

    /// Split at byte `at`: this keeps the text before, the rest is returned.
    pub fn split_off(&mut self, at: usize) -> Inline {
        let mut rest = Inline {
            text: self.text.split_off(at),
            spans: Vec::new(),
        };
        for s in std::mem::take(&mut self.spans) {
            if s.range.start < at {
                self.spans.push(Span {
                    range: s.range.start..s.range.end.min(at),
                    mark: s.mark.clone(),
                });
            }
            if s.range.end > at {
                rest.spans.push(Span {
                    range: s.range.start.max(at) - at..s.range.end - at,
                    mark: s.mark,
                });
            }
        }
        rest.tidy();
        self.tidy();
        rest
    }

    /// Append `other` (its marks with it).
    pub fn append(&mut self, other: Inline) {
        append(self, other);
        self.tidy();
    }

    /// Where the span of `mark` that covers byte `at` ends.
    fn end_of(&self, mark: &Mark, at: usize) -> usize {
        self.spans
            .iter()
            .find(|s| &s.mark == mark && s.range.contains(&at))
            .map_or(at, |s| s.range.end)
    }

    /// Write as inline Markdown that [`Inline::parse`] reads back the same
    /// (in its [`Inline::normalize`]d form).
    pub fn to_markdown(&self) -> String {
        let mut canonical = self.clone();
        canonical.normalize();
        canonical.write()
    }

    fn write(&self) -> String {
        let mut cuts: BTreeSet<usize> = [0, self.text.len()].into();
        for s in &self.spans {
            cuts.insert(s.range.start);
            cuts.insert(s.range.end);
        }
        let cuts: Vec<usize> = cuts.into_iter().collect();
        let mut out = String::new();
        let mut stack: Vec<&Mark> = Vec::new();
        // The last segment's trailing spaces: a closing `*` must touch the
        // text it marks, so they come after what closes.
        let mut held = "";
        for w in cuts.windows(2) {
            let (from, to) = (w[0], w[1]);
            let active = self.marks_at(from);
            // Keep what stays open; nothing opens inside code.
            let mut keep = stack.iter().take_while(|m| active.contains(*m)).count();
            let opens = active.iter().any(|m| !stack[..keep].contains(m));
            if opens && let Some(code) = stack[..keep].iter().position(|m| **m == Mark::Code) {
                keep = code;
            }
            while stack.len() > keep {
                close(&mut out, stack.pop().expect("longer than keep"));
            }
            out.push_str(held);
            let mut text = &self.text[from..to];
            let body = text.trim_end().len();
            held = &text[body..];
            text = &text[..body];
            let mut new: Vec<&Mark> = active.into_iter().filter(|m| !stack.contains(m)).collect();
            if !new.is_empty() {
                // An opening `*` must touch the text it marks.
                let lead = text.len() - text.trim_start().len();
                write_text(&mut out, &text[..lead], stack.last() == Some(&&Mark::Code));
                text = &text[lead..];
                // Only spaces: they join what follows (`normalize`), nothing opens.
                if text.is_empty() {
                    continue;
                }
            }
            // What lasts longer opens outside; code always inside.
            new.sort_by_key(|m| (**m == Mark::Code, std::cmp::Reverse(self.end_of(m, from))));
            for m in new {
                out.push_str(open(m));
                stack.push(m);
            }
            write_text(&mut out, text, stack.last() == Some(&&Mark::Code));
        }
        while let Some(m) = stack.pop() {
            close(&mut out, m);
        }
        out.push_str(held);
        out
    }
}

/// `l` without `range`: up to two pieces.
fn cut(l: Span, range: &Range<usize>) -> Vec<Span> {
    if l.range.end <= range.start || range.end <= l.range.start {
        return vec![l];
    }
    [l.range.start..range.start, range.end..l.range.end]
        .into_iter()
        .filter(|r| r.start < r.end)
        .map(|range| Span {
            range,
            mark: l.mark.clone(),
        })
        .collect()
}

fn open(m: &Mark) -> &'static str {
    match m {
        Mark::Link(_) => "[",
        Mark::Bold => "**",
        Mark::Italic => "*",
        Mark::Code => "`",
    }
}

fn close(out: &mut String, m: &Mark) {
    match m {
        Mark::Link(url) => {
            out.push_str("](");
            // The reader ends a target at whitespace or `)`.
            for c in url.chars() {
                match c {
                    ')' => out.push_str("%29"),
                    c if c.is_whitespace() => out.push_str("%20"),
                    c => out.push(c),
                }
            }
            out.push(')');
        }
        m => out.push_str(open(m)),
    }
}

const ENTITIES: [(&str, char); 5] = [
    ("amp;", '&'),
    ("lt;", '<'),
    ("gt;", '>'),
    ("quot;", '"'),
    ("apos;", '\''),
];

/// Text escaped so it stays text; at a line start also what would start
/// a block (`# x`, `- x`, `1. x`, `> x`, `---`).
fn write_text(out: &mut String, text: &str, code: bool) {
    if code {
        out.push_str(text);
        return;
    }
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        let line_start = out.is_empty() || out.ends_with('\n');
        if line_start && starts_block(rest) {
            let digits = rest.chars().take_while(char::is_ascii_digit).count();
            if digits > 0 {
                // `1\. x`: the number stays, the list marker does not.
                out.push_str(&rest[..digits]);
                out.push('\\');
                out.push_str(&rest[digits..digits + 1]);
                rest = &rest[digits + 1..];
                continue;
            }
            out.push('\\');
        } else if matches!(c, '\\' | '*' | '`' | '[' | ']')
            || (c == '&' && ENTITIES.iter().any(|(n, _)| rest[1..].starts_with(n)))
        {
            out.push('\\');
        }
        out.push(c);
        rest = &rest[c.len_utf8()..];
    }
}

fn starts_block(line: &str) -> bool {
    let line = line.split('\n').next().unwrap_or("");
    markdown::heading(line).is_some()
        || markdown::list_item(line).is_some()
        || markdown::quote_line(line).is_some()
        || markdown::rule(line)
        || markdown::fence(line).is_some()
}

/// A `*` run that may open or close emphasis.
struct Delim {
    /// Its piece in the scope.
    piece: usize,
    /// Stars not yet used.
    left: usize,
    len: usize,
    open: bool,
    close: bool,
}

enum Piece {
    Text(Inline),
    Stars(usize),
}

fn punct(c: char) -> bool {
    c.is_ascii_punctuation() || (!c.is_ascii() && !c.is_alphanumeric() && !c.is_whitespace())
}

/// One scope of inline Markdown (the whole text or a link label): code,
/// links, escapes and entities first, then emphasis over what is left.
fn scope(c: &[char]) -> Inline {
    let mut pieces: Vec<Piece> = Vec::new();
    let mut delims: Vec<Delim> = Vec::new();
    let mut text = Inline::default();
    let flush = |text: &mut Inline, pieces: &mut Vec<Piece>| {
        if !text.text.is_empty() {
            pieces.push(Piece::Text(std::mem::take(text)));
        }
    };
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            '\\' if c.get(i + 1).is_some_and(|n| n.is_ascii_punctuation()) => {
                text.text.push(c[i + 1]);
                i += 2;
                continue;
            }
            '`' => {
                if let Some(end) = (i + 1..c.len())
                    .find(|&k| c[k] == '`')
                    .filter(|&e| e > i + 1)
                {
                    let start = text.text.len();
                    text.text.extend(&c[i + 1..end]);
                    text.spans.push(Span {
                        range: start..text.text.len(),
                        mark: Mark::Code,
                    });
                    i = end + 1;
                    continue;
                }
            }
            '[' => {
                if let Some((label, url, end)) = link(c, i) {
                    let start = text.text.len();
                    append(&mut text, scope(label));
                    text.spans.push(Span {
                        range: start..text.text.len(),
                        mark: Mark::Link(url),
                    });
                    i = end;
                    continue;
                }
            }
            '&' => {
                if let Some((name, ch)) = ENTITIES
                    .iter()
                    .find(|(name, _)| c[i + 1..].starts_with(&name.chars().collect::<Vec<_>>()))
                {
                    text.text.push(*ch);
                    i += 1 + name.len();
                    continue;
                }
            }
            '*' => {
                let len = c[i..].iter().take_while(|&&x| x == '*').count();
                let before = i.checked_sub(1).map(|k| c[k]);
                let after = c.get(i + len).copied();
                let space = |x: Option<char>| x.is_none_or(char::is_whitespace);
                let left = !space(after)
                    && (!after.is_some_and(punct) || space(before) || before.is_some_and(punct));
                let right = !space(before)
                    && (!before.is_some_and(punct) || space(after) || after.is_some_and(punct));
                flush(&mut text, &mut pieces);
                delims.push(Delim {
                    piece: pieces.len(),
                    left: len,
                    len,
                    open: left,
                    close: right,
                });
                pieces.push(Piece::Stars(len));
                i += len;
                continue;
            }
            _ => {}
        }
        text.text.push(c[i]);
        i += 1;
    }
    flush(&mut text, &mut pieces);
    let matches = emphasis(&mut delims);
    for d in &delims {
        pieces[d.piece] = Piece::Stars(d.left);
    }
    // Offsets of each piece in the output.
    let mut out = Inline::default();
    let mut bounds = Vec::with_capacity(pieces.len());
    for p in pieces {
        let start = out.text.len();
        match p {
            Piece::Text(t) => append(&mut out, t),
            Piece::Stars(n) => out.text.extend(std::iter::repeat_n('*', n)),
        }
        bounds.push(start..out.text.len());
    }
    for (opener, closer, mark) in matches {
        out.spans.push(Span {
            range: bounds[opener].end..bounds[closer].start,
            mark,
        });
    }
    out
}

/// The same mark; any two links count as one kind.
fn same(a: &Mark, b: &Mark) -> bool {
    a == b || matches!((a, b), (Mark::Link(_), Mark::Link(_)))
}

fn append(to: &mut Inline, from: Inline) {
    let shift = to.text.len();
    to.text.push_str(&from.text);
    to.spans.extend(from.spans.into_iter().map(|s| Span {
        range: s.range.start + shift..s.range.end + shift,
        mark: s.mark,
    }));
}

/// CommonMark's "process emphasis" for `*`: each closer takes the nearest
/// opener; two stars each make bold, else one makes italic; runs between
/// them stay text. Returns (opener piece, closer piece, mark); an opener's
/// used stars are its last, a closer's its first.
fn emphasis(delims: &mut [Delim]) -> Vec<(usize, usize, Mark)> {
    let mut list: Vec<usize> = (0..delims.len()).collect();
    let mut found = Vec::new();
    let mut ci = 0;
    while ci < list.len() {
        let d = list[ci];
        if !delims[d].close || delims[d].left == 0 {
            ci += 1;
            continue;
        }
        let opener = (0..ci).rev().find(|&oi| {
            let o = &delims[list[oi]];
            let both = (o.open && o.close) || (delims[d].open && delims[d].close);
            let three = (o.len + delims[d].len).is_multiple_of(3)
                && !(o.len.is_multiple_of(3) && delims[d].len.is_multiple_of(3));
            o.open && o.left > 0 && !(both && three)
        });
        let Some(oi) = opener else {
            if delims[d].open {
                ci += 1;
            } else {
                list.remove(ci);
            }
            continue;
        };
        let o = list[oi];
        let strong = delims[o].left >= 2 && delims[d].left >= 2;
        let used = if strong { 2 } else { 1 };
        delims[o].left -= used;
        delims[d].left -= used;
        let mark = if strong { Mark::Bold } else { Mark::Italic };
        found.push((delims[o].piece, delims[d].piece, mark));
        list.drain(oi + 1..ci);
        ci = oi + 1;
        if delims[o].left == 0 {
            list.remove(oi);
            ci -= 1;
        }
        if delims[d].left == 0 {
            list.remove(ci);
        }
    }
    // Stars an opener did not use come before its emphasis, a closer's
    // after it: the spans start after the opener's piece and end before
    // the closer's, which now hold only the unused stars.
    found
}

/// `[label](target)`: the label, the target and where the link ends.
fn link(c: &[char], i: usize) -> Option<(&[char], String, usize)> {
    let mut mid = i + 1;
    while mid < c.len() && c[mid] != ']' && c[mid] != '\n' {
        mid += if c[mid] == '\\' { 2 } else { 1 };
    }
    if c.get(mid) != Some(&']') || mid == i + 1 || c.get(mid + 1) != Some(&'(') {
        return None;
    }
    let end = (mid + 2..c.len()).find(|&k| c[k] == ')' || c[k].is_whitespace())?;
    if c[end] != ')' || end == mid + 2 {
        return None;
    }
    let url: String = c[mid + 2..end].iter().collect();
    Some((
        &c[i + 1..mid],
        url.replace("%20", " ").replace("%29", ")"),
        end + 1,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(range: Range<usize>, mark: Mark) -> Span {
        Span { range, mark }
    }

    fn styled(text: &str, spans: Vec<Span>) -> Inline {
        let mut i = Inline {
            text: text.into(),
            spans,
        };
        i.normalize();
        i
    }

    #[test]
    fn reads_marks_nested_and_side_by_side() {
        assert_eq!(
            Inline::parse("a **b *c* d** e"),
            styled(
                "a b c d e",
                vec![span(2..7, Mark::Bold), span(4..5, Mark::Italic)]
            )
        );
        assert_eq!(
            Inline::parse("**a***b*"),
            styled("ab", vec![span(0..1, Mark::Bold), span(1..2, Mark::Italic)])
        );
        assert_eq!(
            Inline::parse("***x***"),
            styled("x", vec![span(0..1, Mark::Bold), span(0..1, Mark::Italic)])
        );
        assert_eq!(
            Inline::parse("*a **b***"),
            styled(
                "a b",
                vec![span(0..3, Mark::Italic), span(2..3, Mark::Bold)]
            )
        );
        assert_eq!(
            Inline::parse("***a** b*"),
            styled(
                "a b",
                vec![span(0..3, Mark::Italic), span(0..1, Mark::Bold)]
            )
        );
        assert_eq!(
            Inline::parse("see [the **docs**](https://x.org/a%20b) and `a*b`"),
            styled(
                "see the docs and a*b",
                vec![
                    span(4..12, Mark::Link("https://x.org/a b".into())),
                    span(8..12, Mark::Bold),
                    span(17..20, Mark::Code),
                ]
            )
        );
    }

    #[test]
    fn stars_without_a_partner_stay_text() {
        assert_eq!(Inline::parse("2 * 3 = 6"), Inline::plain("2 * 3 = 6"));
        assert_eq!(Inline::parse("**open"), Inline::plain("**open"));
        assert_eq!(
            Inline::parse("a \\*b\\* &amp; c"),
            Inline::plain("a *b* & c")
        );
        assert_eq!(Inline::parse("** a**"), Inline::plain("** a**"));
    }

    #[test]
    fn writes_what_it_reads() {
        for md in [
            "plain",
            "a **b *c* d** e",
            "**a***b*",
            "*a **b***",
            "***a** b*",
            "[l **b**](u) `c`",
            "x \\*y\\* \\[z\\] \\` \\\\ \\&amp; & done",
        ] {
            assert_eq!(Inline::parse(md).to_markdown(), md, "{md}");
        }
    }

    #[test]
    fn line_starts_that_would_open_a_block_are_escaped() {
        for text in [
            "# not a heading",
            "- not an item",
            "12. not a list",
            "> no quote",
            "---",
        ] {
            let md = Inline::plain(text).to_markdown();
            assert_eq!(Inline::parse(&md), Inline::plain(text), "{md}");
            assert!(starts_block(text) && !starts_block(&md), "{md}");
        }
    }

    #[test]
    fn spaces_stay_outside_emphasis_and_backticks_outside_code() {
        let i = styled("a b c", vec![span(1..4, Mark::Bold)]);
        assert_eq!(i.spans, [span(2..3, Mark::Bold)]);
        assert_eq!(i.to_markdown(), "a **b** c");
        let i = styled("x`y", vec![span(0..3, Mark::Code)]);
        assert_eq!(i.to_markdown(), "`x`\\``y`");
        assert_eq!(Inline::parse(&i.to_markdown()), i);
    }

    #[test]
    fn typing_inserting_and_deleting_keep_the_marks_in_place() {
        let mut i = Inline::parse("a **bold** c");
        let typing = i.typing(6);
        assert_eq!(typing, [Mark::Bold].into());
        i.insert(6, "er ", &typing);
        assert_eq!(i.text, "a bolder  c");
        assert_eq!(
            i.spans,
            [span(2..9, Mark::Bold)],
            "a typed space stays bold"
        );
        assert_eq!(
            i.typing(9),
            [Mark::Bold].into(),
            "so typing goes on in bold"
        );
        assert_eq!(i.to_markdown(), "a **bolder**  c", "written without it");
        i.delete(8..9);
        i.insert(0, "x", &BTreeSet::new());
        assert_eq!(i.to_markdown(), "xa **bolder** c");
        i.insert(5, "!", &BTreeSet::new());
        assert_eq!(i.to_markdown(), "xa **bo**!**lder** c");
        i.delete(4..7);
        assert_eq!(i.to_markdown(), "xa **bder** c");
        i.delete(0..i.text.len());
        assert_eq!(i, Inline::default());
    }

    #[test]
    fn marks_go_on_and_off_a_range() {
        let mut i = Inline::plain("one two three");
        i.add(4..7, Mark::Italic);
        assert!(i.has(4..7, &Mark::Italic) && !i.has(3..7, &Mark::Bold));
        assert!(i.has(3..8, &Mark::Italic), "spaces do not count");
        i.add(0..13, Mark::Link("a".into()));
        i.add(4..7, Mark::Link("b".into()));
        assert_eq!(i.to_markdown(), "[one](a) [*two*](b) [three](a)");
        i.remove(0..13, &Mark::Link(String::new()));
        i.remove(5..6, &Mark::Italic);
        assert_eq!(i.to_markdown(), "one *t*w*o* three");
        assert_eq!(i.typing(0), BTreeSet::new());
    }

    #[test]
    fn a_text_splits_and_joins_with_its_marks() {
        let mut i = Inline::parse("a **bc** d");
        let rest = i.split_off(3);
        assert_eq!(
            (i.to_markdown(), rest.to_markdown()),
            ("a **b**".into(), "**c** d".into())
        );
        i.append(rest);
        assert_eq!(i, Inline::parse("a **bc** d"));
    }

    /// Any marks whose ends lie at word ends write Markdown that reads back
    /// as the same formatted text.
    #[test]
    fn any_marks_read_back_the_same() {
        let text = "ab cd ef gh ij";
        let marks = [
            Mark::Bold,
            Mark::Italic,
            Mark::Code,
            Mark::Link("u".into()),
            Mark::Link("v w".into()),
        ];
        // Word starts and ends; a mark inside a word next to a link or
        // code would need punctuation rules no writer can satisfy.
        let bounds = [0, 2, 3, 5, 6, 8, 9, 11, 12, 14];
        let mut seed: u64 = 7;
        let mut next = |n: usize| {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (seed >> 33) as usize % n
        };
        for _ in 0..20000 {
            let spans = (0..1 + next(5))
                .map(|_| {
                    let (a, b) = (bounds[next(bounds.len())], bounds[next(bounds.len())]);
                    span(a.min(b)..a.max(b), marks[next(marks.len())].clone())
                })
                .collect();
            let want = styled(text, spans);
            let md = want.to_markdown();
            assert_eq!(Inline::parse(&md), want, "{md}");
        }
    }
}
