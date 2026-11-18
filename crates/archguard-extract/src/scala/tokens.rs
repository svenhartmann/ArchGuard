//! Scala tokens shared by the structure walk and the call scanner: lexer
//! tokens with operators merged, bracket pairs, line indentation and the
//! region rule that makes Scala 3 indentation syntax and brace-less Scala 2
//! bodies end in the same place.

use std::collections::BTreeMap;

use crate::lex::{Flavor, tokens};

/// Multi-character operators, longest first; the lexer emits single characters.
const OPERATORS: [&str; 21] = [
    "?=>", ":::", "=:=", "=>", "<-", "->", "==", "!=", "<=", ">=", "&&", "||", "++", "+=", "-=",
    "*=", "/=", "%=", "::", "<:", ">:",
];
/// Keywords; Halstead counts them as operators. Soft keywords included.
pub(super) const KEYWORDS: [&str; 44] = [
    "abstract",
    "case",
    "catch",
    "class",
    "def",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "extension",
    "final",
    "finally",
    "for",
    "given",
    "if",
    "implicit",
    "import",
    "lazy",
    "match",
    "new",
    "object",
    "override",
    "package",
    "private",
    "protected",
    "return",
    "sealed",
    "super",
    "then",
    "this",
    "throw",
    "trait",
    "try",
    "type",
    "using",
    "val",
    "var",
    "while",
    "with",
    "yield",
    "derives",
    "inline",
    "opaque",
];
const MODIFIERS: [&str; 14] = [
    "abstract",
    "sealed",
    "final",
    "case",
    "implicit",
    "private",
    "protected",
    "override",
    "lazy",
    "open",
    "inline",
    "transparent",
    "opaque",
    "infix",
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

/// Lexer tokens without line breaks, adjacent operator characters merged
/// and an interpolator (`s"…"`) joined to its string literal.
pub(super) fn merged(source: &str) -> Vec<Tok> {
    let raw: Vec<_> = tokens(source, Flavor::Kotlin)
        .into_iter()
        .filter(|token| token.text != "\n")
        .collect();
    let mut out: Vec<Tok> = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let width = width(&raw[i..]);
        let (first, last) = (&raw[i], &raw[i + width - 1]);
        out.push(Tok {
            text: raw[i..i + width].iter().map(|t| t.text.as_str()).collect(),
            literal: last.literal,
            line: first.line,
            end_line: last.end_line,
            start: first.start,
            end: last.end,
        });
        i += width;
    }
    out
}

/// How many lexer tokens at the start of `raw` form one token: an
/// interpolated string or a multi-character operator.
fn width(raw: &[crate::lex::Token]) -> usize {
    let touching = |n: usize| {
        raw.get(..n)
            .filter(|part| part.windows(2).all(|w| w[0].end == w[1].start))
    };
    let interpolated = touching(2).is_some_and(|part| {
        !part[0].literal
            && part[0].text.starts_with(char::is_alphabetic)
            && part[1].literal
            && part[1].text.starts_with('"')
    });
    if interpolated {
        return 2;
    }
    let operator = |part: &[crate::lex::Token]| {
        let text: String = part.iter().map(|t| t.text.as_str()).collect();
        part.iter().all(|t| !t.literal) && OPERATORS.contains(&text.as_str())
    };
    OPERATORS
        .iter()
        .map(|op| op.chars().count())
        .find(|&n| touching(n).is_some_and(operator))
        .unwrap_or(1)
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

/// Tokens with their bracket pairs and line indentation.
pub(super) struct Code<'a> {
    pub(super) toks: &'a [Tok],
    pub(super) pairs: BTreeMap<usize, usize>,
    /// Leading whitespace characters of every source line.
    indents: Vec<u32>,
}

impl<'a> Code<'a> {
    pub(super) fn new(toks: &'a [Tok], pairs: BTreeMap<usize, usize>, source: &str) -> Self {
        let indents = source
            .lines()
            .map(|line| line.chars().take_while(|c| c.is_whitespace()).count() as u32)
            .collect();
        Code {
            toks,
            pairs,
            indents,
        }
    }
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
    /// The name at `i` without backticks.
    pub(super) fn name(&self, i: usize) -> String {
        self.toks[i].text.trim_matches('`').to_string()
    }
    /// `i` is a member name such as `x.match` or `x.new`.
    pub(super) fn after_member(&self, i: usize) -> bool {
        i > 0 && self.text(i - 1) == "."
    }
    /// The token at `i` opens a new source line.
    pub(super) fn new_line(&self, i: usize) -> bool {
        i == 0 || (i < self.toks.len() && self.toks[i].line > self.toks[i - 1].end_line)
    }
    /// The token at `i` is the last one of its line.
    pub(super) fn line_ends(&self, i: usize) -> bool {
        self.new_line(i + 1) || i + 1 >= self.toks.len()
    }
    /// The indentation of the line the token at `i` starts on.
    pub(super) fn indent(&self, i: usize) -> u32 {
        self.toks
            .get(i)
            .and_then(|t| self.indents.get(t.line as usize - 1))
            .copied()
            .unwrap_or(0)
    }
    /// The enclosing region ends at `i`: past the end, a closing bracket or
    /// `;` of the enclosing scope, or a line indented no deeper than `base`.
    /// A token at `from` itself never ends the region by its indentation.
    pub(super) fn closes(&self, i: usize, from: usize, base: u32) -> bool {
        i >= self.toks.len()
            || matches!(self.text(i), ")" | "]" | "}" | ";")
            || (i > from && self.new_line(i) && self.indent(i) <= base)
    }
    /// The index after the region that starts at `from` inside a construct
    /// whose line is indented by `base`. Bracketed groups are skipped whole,
    /// so a brace-closed body ends at its `}` in either syntax.
    pub(super) fn region_end(&self, from: usize, base: u32) -> usize {
        let mut j = from;
        while !self.closes(j, from, base) {
            if matches!(self.text(j), "(" | "[" | "{") {
                j = self.pairs[&j];
            }
            j += 1;
        }
        j
    }
    /// The dotted name `a.b.C` that starts at `i`, and the index after it.
    pub(super) fn dotted(&self, i: usize) -> (Vec<String>, usize) {
        let mut names = vec![self.name(i)];
        let mut j = i + 1;
        while self.text(j) == "." && self.is_name(j + 1) {
            names.push(self.name(j + 1));
            j += 2;
        }
        (names, j)
    }
    /// Modifiers directly before the declaration keyword at `at`.
    pub(super) fn modifiers(&self, at: usize) -> Vec<String> {
        let mut out: Vec<String> = (0..at)
            .rev()
            .map(|k| self.text(k))
            .take_while(|word| MODIFIERS.contains(word))
            .map(str::to_string)
            .collect();
        out.sort();
        out
    }
    /// Names after `extends`, `with` and `,` up to `to`, sorted; constructor
    /// arguments and type arguments are not names of supertypes.
    pub(super) fn supertypes(&self, from: usize, to: usize) -> Vec<String> {
        let mut out = Vec::new();
        let (mut seen, mut want) = (false, false);
        let mut j = from;
        while j < to && self.text(j) != "derives" {
            match self.text(j) {
                "(" | "[" | "{" => j = self.pairs[&j],
                "extends" => (seen, want) = (true, true),
                "with" | "," if seen => want = true,
                _ if want && self.is_name(j) => {
                    let (names, end) = self.dotted(j);
                    out.push(names.join("."));
                    want = false;
                    j = end - 1;
                }
                _ => {}
            }
            j += 1;
        }
        out.sort();
        out
    }
}
