//! Painless (Elasticsearch scripting) complexity counter, rule `ast-decisions-v1`.
//! ADR 0023 names the rule; this is the one place that lists it.
//!
//! - cyclomatic: base 1, plus 1 for each `if` (so `else if` counts once), `for`
//!   (both forms), `while`, `do`, `case`, `catch`, `&&`, `||`, ternary `? :`,
//!   Elvis `?:` and null-safe `?.`. `else`, `try`, `switch` and `finally` add nothing.
//!   Exception: the `while` that follows a closed `do { … }` block (comments in
//!   between are skipped) belongs to that `do` and does not count again.
//! - max_nesting: deepest stack of control blocks (`if`, `else`, `for`, `while`,
//!   `do`, `switch`, `try`, `catch`, `finally`) opened with `{`. Lambda and map
//!   braces do not count.
//! - Strings (`'…'`, `"…"`), comments and regex literals (`/…/`) are not scanned.
//! - Unbalanced `()[]{}` or an unterminated string, comment or regex: not measured (`None`).
//!
//! Own tiny lexer instead of `lex.rs`: Painless needs regex literals and
//! multi-character `'…'` strings, which no `lex` flavor knows.
// shortcut: a brace-less control body (`if (x) y;`) adds to cyclomatic but not to
// nesting; upgrade if brace-less bodies turn out to be common.

/// Result of [`measure`].
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct PainlessMetrics {
    pub cyclomatic: u32,
    pub max_nesting: u32,
}

const CONTROL: [&str; 9] = [
    "if", "else", "for", "while", "do", "switch", "try", "catch", "finally",
];
const DECISION: [&str; 6] = ["if", "for", "while", "do", "case", "catch"];

/// Measure one Painless source; `None` when it is not measurable.
pub fn measure(src: &str) -> Option<PainlessMetrics> {
    let mut s = Scan {
        c: src.chars().collect(),
        i: 0,
        cyc: 1,
        max_nest: 0,
        depth: 0,
        stack: Vec::new(),
        pending: false,
        operand: false,
        do_pending: false,
        closed_do: false,
        after_do: false,
    };
    while s.i < s.c.len() {
        s.step()?;
    }
    s.stack.is_empty().then_some(PainlessMetrics {
        cyclomatic: s.cyc,
        max_nesting: s.max_nest,
    })
}

struct Scan {
    c: Vec<char>,
    i: usize,
    cyc: u32,
    max_nest: u32,
    depth: u32,
    /// Open `(`, `[`, `{`; the flags mark a control-block brace and a `do` block.
    stack: Vec<(char, bool, bool)>,
    /// A control keyword was seen and its `{` has not opened yet.
    pending: bool,
    /// The previous token ends an operand, so `/` divides instead of opening a regex.
    operand: bool,
    /// `do` was seen and its `{` has not opened yet.
    do_pending: bool,
    /// The last token closed a `do { … }` block; a `while` right after it is not a new loop.
    closed_do: bool,
    /// The token before this one closed a `do { … }` block.
    after_do: bool,
}

impl Scan {
    fn at(&self, o: usize) -> Option<char> {
        self.c.get(self.i + o).copied()
    }

    fn step(&mut self) -> Option<()> {
        let ch = self.c[self.i];
        if ch.is_whitespace() {
            self.i += 1;
            return Some(());
        }
        self.after_do = std::mem::take(&mut self.closed_do);
        if ch == '/' && matches!(self.at(1), Some('/' | '*')) {
            self.closed_do = self.after_do; // a comment between `}` and `while` keeps the do-while
            self.comment()?;
        } else if ch == '"' || ch == '\'' || (ch == '/' && !self.operand) {
            self.literal(ch)?;
        } else if ch.is_alphanumeric() || matches!(ch, '_' | '$') {
            self.word();
        } else {
            self.punct(ch)?;
        }
        Some(())
    }

    fn comment(&mut self) -> Option<()> {
        if self.at(1) == Some('/') {
            while self.at(0).is_some_and(|c| c != '\n') {
                self.i += 1;
            }
            return Some(());
        }
        self.i += 2;
        while !(self.at(0) == Some('*') && self.at(1) == Some('/')) {
            self.at(0)?;
            self.i += 1;
        }
        self.i += 2;
        Some(())
    }

    /// String or regex literal delimited by `q`; backslash escapes.
    fn literal(&mut self, q: char) -> Option<()> {
        self.i += 1;
        loop {
            match self.at(0)? {
                '\\' => self.i += 2,
                '\n' if q == '/' => return None,
                c if c == q => break,
                _ => self.i += 1,
            }
        }
        self.i += 1;
        self.operand = true;
        Some(())
    }

    fn word(&mut self) {
        let start = self.i;
        while self
            .at(0)
            .is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '$'))
        {
            self.i += 1;
        }
        let word: String = self.c[start..self.i].iter().collect();
        let loop_end = word == "while" && self.after_do;
        self.cyc += u32::from(DECISION.contains(&word.as_str()) && !loop_end);
        self.do_pending |= word == "do";
        self.pending |= CONTROL.contains(&word.as_str());
        // `return /x/` and friends: a keyword is not an operand.
        self.operand = !matches!(word.as_str(), "return" | "case" | "else" | "in" | "do");
    }

    fn punct(&mut self, ch: char) -> Option<()> {
        let next = self.at(1);
        self.i += 1;
        let before = self.operand;
        self.operand = matches!(ch, ')' | ']' | '}');
        match ch {
            '+' | '-' if next == Some(ch) => {
                self.i += 1;
                self.operand = before;
            }
            '&' | '|' if next == Some(ch) => {
                self.cyc += 1;
                self.i += 1;
                self.operand = false;
            }
            '?' => {
                self.cyc += 1; // ternary, Elvis `?:` and null-safe `?.` each count once
                self.i += usize::from(matches!(next, Some('.' | ':')));
            }
            '(' | '[' => self.stack.push((ch, false, false)),
            '{' => self.open_brace(),
            ')' | ']' | '}' => return self.close(ch),
            ';' if self.stack.iter().all(|s| s.0 == '{') => {
                self.pending = false;
                self.do_pending = false;
            }
            _ => {}
        }
        Some(())
    }

    fn open_brace(&mut self) {
        let ctl = self.pending && self.stack.iter().all(|s| s.0 == '{');
        self.pending = false;
        let is_do = std::mem::take(&mut self.do_pending);
        self.stack.push(('{', ctl, is_do));
        if ctl {
            self.depth += 1;
            self.max_nest = self.max_nest.max(self.depth);
        }
    }

    fn close(&mut self, ch: char) -> Option<()> {
        let want = match ch {
            ')' => '(',
            ']' => '[',
            _ => '{',
        };
        match self.stack.pop() {
            Some((o, ctl, is_do)) if o == want => {
                self.depth -= u32::from(ctl);
                self.closed_do = is_do;
                Some(())
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cyc(s: &str) -> u32 {
        measure(s).unwrap().cyclomatic
    }

    #[test]
    fn empty_body_is_one() {
        assert_eq!(
            measure(""),
            Some(PainlessMetrics {
                cyclomatic: 1,
                max_nesting: 0
            })
        );
    }

    #[test]
    fn each_rule_counts_one() {
        for (src, want) in [
            ("if (a) { b }", 2),
            ("if (a) { b } else if (c) { d } else { e }", 3),
            ("for (x in xs) { y }", 2),
            ("for (;;) { y }", 2),
            ("while (a) { b }", 2),
            ("do { a } while (b)", 2), // the `while` of a do-while is not counted again
            ("do { a } while (b) && c", 3),
            ("do { a } // c\n while (b)", 2),
            ("do { a } /* c */ while (b)", 2),
            ("do { if (a) { b } } while (c)", 3),
            ("switch (a) { case 1: b; case 2: c; }", 3),
            ("try { a } catch (Exception e) { b }", 2),
            ("a && b", 2),
            ("a || b", 2),
            ("a ? b : c", 2),
            ("a ?: b", 2),
            ("a?.b", 2),
        ] {
            assert_eq!(cyc(src), want, "{src}");
        }
    }

    #[test]
    fn slash_after_postfix_increment_divides() {
        for (src, want) in [
            ("i++ / 2 ? a : b", 2),
            ("i-- / 2 / 3", 1),
            ("++i / 2 + 1", 1),
        ] {
            assert_eq!(cyc(src), want, "{src}");
        }
    }

    #[test]
    fn strings_comments_and_regex_do_not_count() {
        for src in [
            "'a ? b' + \"c && d\"",
            "// if ? && \n 1",
            "/* while (x) ? */ 1",
            "def m = x =~ /a?b|c&&d/",
            "'it\\'s ?'",
        ] {
            assert_eq!(cyc(src), 1, "{src}");
        }
        assert_eq!(cyc("a / b ? c : d / e"), 2); // division, not regex
    }

    #[test]
    fn nesting_is_deepest_control_block() {
        let m =
            measure("if (a) { for (x in xs) { while (b) { c } } } else { d } def f = x -> { 1 }")
                .unwrap();
        assert_eq!(m.max_nesting, 3);
        assert_eq!(measure("def f = x -> { 1 }").unwrap().max_nesting, 0);
        assert_eq!(measure("if (a) { b } if (c) { d }").unwrap().max_nesting, 1);
    }

    #[test]
    fn unbalanced_is_not_measured() {
        for src in [
            "if (a) { b",
            "a }",
            "f(a",
            "(]",
            "'open",
            "/* open",
            "x =~ /open",
        ] {
            assert_eq!(measure(src), None, "{src}");
        }
    }
}
