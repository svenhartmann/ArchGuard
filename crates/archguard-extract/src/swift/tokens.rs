//! Swift tokens for the structure walk: lexer tokens with operators merged,
//! bracket pairs and declaration lookups.

use std::collections::BTreeMap;

use crate::lex::{Flavor, tokens};

/// Multi-character operators, longest first; the lexer emits single characters.
const OPERATORS: [&str; 20] = [
    "===", "!==", "...", "..<", "??", "?.", "->", "==", "!=", "<=", ">=", "&&", "||", "+=", "-=",
    "*=", "/=", "%=", "<<", ">>",
];
pub(super) const KEYWORDS: [&str; 45] = [
    "as",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "continue",
    "default",
    "defer",
    "deinit",
    "do",
    "else",
    "enum",
    "extension",
    "fallthrough",
    "for",
    "func",
    "guard",
    "if",
    "import",
    "in",
    "init",
    "inout",
    "is",
    "let",
    "nil",
    "protocol",
    "repeat",
    "rethrows",
    "return",
    "self",
    "Self",
    "some",
    "any",
    "static",
    "struct",
    "subscript",
    "super",
    "switch",
    "throw",
    "throws",
    "try",
    "var",
    "while",
];
/// Keywords that start a declaration wherever they stand.
const DECLARES: [&str; 14] = [
    "func",
    "var",
    "let",
    "class",
    "struct",
    "enum",
    "protocol",
    "actor",
    "extension",
    "init",
    "deinit",
    "subscript",
    "typealias",
    "import",
];
pub(super) const MODIFIERS: [&str; 22] = [
    "public",
    "private",
    "fileprivate",
    "internal",
    "package",
    "open",
    "final",
    "static",
    "class",
    "override",
    "mutating",
    "nonmutating",
    "convenience",
    "required",
    "lazy",
    "weak",
    "unowned",
    "dynamic",
    "optional",
    "indirect",
    "nonisolated",
    "distributed",
];

pub(super) struct Tok {
    pub(super) text: String,
    pub(super) literal: bool,
    pub(super) line: u32,
    pub(super) end_line: u32,
    pub(super) start: usize,
    pub(super) end: usize,
}

impl Tok {
    /// A name; backtick-quoted names count although the lexer marks them literal.
    pub(super) fn is_name(&self) -> bool {
        (!self.literal
            && self
                .text
                .starts_with(|c: char| c.is_alphabetic() || c == '_'))
            || self.text.starts_with('`')
    }
}

/// Lexer tokens without line breaks, with adjacent operator characters merged.
pub(super) fn merged(source: &str) -> Vec<Tok> {
    let raw: Vec<_> = tokens(source, Flavor::Swift)
        .into_iter()
        .filter(|token| token.text != "\n")
        .collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let width = OPERATORS
            .iter()
            .map(|op| op.len())
            .find(|&n| {
                let part = &raw[i..(i + n).min(raw.len())];
                part.len() == n
                    && part.windows(2).all(|w| w[0].end == w[1].start)
                    && part.iter().all(|t| !t.literal)
                    && OPERATORS.contains(
                        &part
                            .iter()
                            .map(|t| t.text.as_str())
                            .collect::<String>()
                            .as_str(),
                    )
            })
            .unwrap_or(1);
        let (first, last) = (&raw[i], &raw[i + width - 1]);
        out.push(Tok {
            text: raw[i..i + width].iter().map(|t| t.text.as_str()).collect(),
            literal: first.literal,
            line: first.line,
            end_line: last.end_line,
            start: first.start,
            end: last.end,
        });
        i += width;
    }
    out
}

/// The index of the closing bracket for every opening one; `None` when unbalanced.
pub(super) fn pairs(toks: &[Tok]) -> Option<BTreeMap<usize, usize>> {
    let mut open = Vec::new();
    let mut out = BTreeMap::new();
    for (index, token) in toks.iter().enumerate().filter(|(_, t)| !t.literal) {
        match token.text.as_str() {
            "(" | "[" | "{" => open.push(index),
            ")" | "]" | "}" => {
                out.insert(open.pop()?, index);
            }
            _ => {}
        }
    }
    open.is_empty().then_some(out)
}

/// Tokens with their bracket pairs, and the lookups the walk needs.
pub(super) struct Code<'a> {
    pub(super) toks: &'a [Tok],
    pub(super) pairs: BTreeMap<usize, usize>,
}

impl<'a> Code<'a> {
    /// The code text at `i`; empty for literals and past the end.
    pub(super) fn text(&self, i: usize) -> &'a str {
        self.toks
            .get(i)
            .filter(|t| !t.literal)
            .map_or("", |t| t.text.as_str())
    }
    pub(super) fn is_name(&self, i: usize) -> bool {
        self.toks.get(i).is_some_and(Tok::is_name)
    }
    /// A name that is no keyword.
    pub(super) fn is_identifier(&self, i: usize) -> bool {
        self.is_name(i) && !KEYWORDS.contains(&self.text(i))
    }
    pub(super) fn name(&self, i: usize) -> String {
        self.toks[i].text.trim_matches('`').to_string()
    }
    /// `i` is a member name such as `x.init` or `.class`.
    pub(super) fn after_dot(&self, i: usize) -> bool {
        i > 0 && matches!(self.text(i - 1), "." | "?.")
    }
    /// The token at `i` opens a new source line.
    pub(super) fn new_line(&self, i: usize) -> bool {
        i > 0 && i < self.toks.len() && self.toks[i].line > self.toks[i - 1].end_line
    }
    /// The tokens at `a` and `b` touch.
    pub(super) fn adjacent(&self, a: usize, b: usize) -> bool {
        self.toks[a].end == self.toks[b].start
    }
    /// A declaration starts at `i`: a declaring keyword, an attribute, or
    /// a modifier that a declaring keyword or another modifier follows.
    fn declares(&self, i: usize) -> bool {
        let word = self.text(i);
        DECLARES.contains(&word)
            || word == "@"
            || (MODIFIERS.contains(&word)
                && (DECLARES.contains(&self.text(i + 1)) || MODIFIERS.contains(&self.text(i + 1))))
    }
    /// The enclosing scope or statement ends, or a new declaration starts, at `i`.
    fn ends(&self, i: usize) -> bool {
        i >= self.toks.len()
            || matches!(self.text(i), "}" | ")" | "]" | ";")
            || (self.new_line(i) && self.declares(i))
    }
    /// The `{` of the body after a declaration header that continues at
    /// `from`, or `Err` with the index where the next declaration starts.
    pub(super) fn body(&self, from: usize) -> Result<usize, usize> {
        let mut j = from;
        while !self.ends(j) {
            match self.text(j) {
                "(" | "[" => j = self.pairs[&j],
                "{" => return Ok(j),
                // A property with an initializer: no body of its own.
                "=" => return Err(j),
                _ => {}
            }
            j += 1;
        }
        Err(j)
    }
    /// The dotted name `a.b.C` that starts at `i`.
    pub(super) fn dotted(&self, i: usize) -> String {
        let mut name = self.name(i);
        let mut j = i + 1;
        while self.text(j) == "." && self.is_name(j + 1) {
            name.push('.');
            name.push_str(&self.name(j + 1));
            j += 2;
        }
        name
    }
    /// Modifiers directly before the declaration keyword at `at`;
    /// attributes such as `@objc` or `@available(…)` in between are skipped.
    pub(super) fn modifiers(&self, at: usize) -> Vec<&'a str> {
        let mut out = Vec::new();
        let mut k = at;
        while k > 0 {
            let word = self.text(k - 1);
            if MODIFIERS.contains(&word) {
                out.push(word);
                k -= 1;
            } else if k >= 2 && self.text(k - 2) == "@" && self.is_name(k - 1) {
                k -= 2;
            } else if word == ")" {
                match self.pairs.iter().find(|(_, close)| **close == k - 1) {
                    Some((&open, _)) if open >= 2 && self.text(open - 2) == "@" => k = open - 2,
                    _ => break,
                }
            } else {
                break;
            }
        }
        out
    }
    /// Names after the header's `:` up to `to`, sorted; generic arguments
    /// and `where` clauses are not supertypes.
    pub(super) fn supertypes(&self, from: usize, to: usize) -> Vec<String> {
        let mut out = Vec::new();
        let (mut angle, mut want) = (0usize, false);
        let mut j = from;
        while j < to && self.text(j) != "where" {
            match self.text(j) {
                "(" | "[" => j = self.pairs[&j],
                "<" => angle += 1,
                ">" => angle = angle.saturating_sub(1),
                ":" | "," if angle == 0 => want = true,
                _ if want && angle == 0 && self.is_name(j) => {
                    out.push(self.dotted(j));
                    want = false;
                }
                _ => {}
            }
            j += 1;
        }
        out.sort();
        out.dedup();
        out
    }
}
