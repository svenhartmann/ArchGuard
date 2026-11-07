//! Kotlin tokens shared by the structure walk and the call scanner: lexer
//! tokens with operators merged, bracket pairs and declaration lookups.

use std::collections::{BTreeMap, BTreeSet};

use crate::lex::{Flavor, tokens};

/// Multi-character operators, longest first; the lexer emits single characters.
pub(super) const OPERATORS: [&str; 25] = [
    "!==", "===", "..<", "as?", "!is", "!in", "?.", "?:", "!!", "::", "->", "==", "!=", "<=", ">=",
    "&&", "||", "++", "--", "+=", "-=", "*=", "/=", "%=", "..",
];
pub(super) const KEYWORDS: [&str; 30] = [
    "as",
    "as?",
    "break",
    "by",
    "catch",
    "class",
    "continue",
    "do",
    "else",
    "finally",
    "for",
    "fun",
    "if",
    "in",
    "!in",
    "interface",
    "is",
    "!is",
    "object",
    "return",
    "super",
    "this",
    "throw",
    "try",
    "typealias",
    "val",
    "var",
    "when",
    "where",
    "while",
];
/// Keywords that start a declaration wherever they stand.
pub(super) const DECLARES: [&str; 11] = [
    "fun",
    "val",
    "var",
    "class",
    "interface",
    "object",
    "typealias",
    "init",
    "constructor",
    "import",
    "package",
];
pub(super) const MODIFIERS: [&str; 25] = [
    "public",
    "protected",
    "private",
    "internal",
    "abstract",
    "open",
    "final",
    "sealed",
    "data",
    "enum",
    "annotation",
    "inner",
    "value",
    "inline",
    "companion",
    "override",
    "suspend",
    "operator",
    "infix",
    "tailrec",
    "external",
    "const",
    "lateinit",
    "expect",
    "actual",
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
    let raw: Vec<_> = tokens(source, Flavor::Kotlin)
        .into_iter()
        .filter(|token| token.text != "\n")
        .collect();
    let mut out: Vec<Tok> = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let width = OPERATORS
            .iter()
            .map(|op| op.chars().count())
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

/// Where a declaration header ends.
pub(super) enum Start {
    /// A `{` body at this index.
    Block(usize),
    /// An `=` expression body at this index.
    Expr(usize),
    /// No body; the next declaration starts at this index.
    None(usize),
}

/// Tokens with their bracket pairs, and the lookups both passes share.
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
    /// `i` is a member name such as `Foo::class` or `x.object`.
    pub(super) fn after_member(&self, i: usize) -> bool {
        i > 0 && matches!(self.text(i - 1), "." | "?." | "::")
    }
    /// The token at `i` opens a new source line.
    pub(super) fn new_line(&self, i: usize) -> bool {
        i > 0 && i < self.toks.len() && self.toks[i].line > self.toks[i - 1].end_line
    }
    /// A declaration starts at `i`: a declaring keyword, an annotation, or
    /// a modifier that a declaring keyword or another modifier follows.
    pub(super) fn declares(&self, i: usize) -> bool {
        let word = self.text(i);
        DECLARES.contains(&word)
            || word == "@"
            || (MODIFIERS.contains(&word)
                && (DECLARES.contains(&self.text(i + 1)) || MODIFIERS.contains(&self.text(i + 1))))
    }
    /// The enclosing scope or statement ends, or a new declaration starts, at `i`.
    pub(super) fn ends(&self, i: usize) -> bool {
        i >= self.toks.len()
            || matches!(self.text(i), "}" | ")" | "]" | ";")
            || (self.new_line(i) && self.declares(i))
    }
    /// The body after a declaration header that continues at `from`.
    pub(super) fn body_start(&self, from: usize) -> Start {
        let mut angle = 0usize;
        let mut j = from;
        while !self.ends(j) {
            match self.text(j) {
                "(" | "[" => j = self.pairs[&j],
                "<" => angle += 1,
                ">" => angle = angle.saturating_sub(1),
                "{" => return Start::Block(j),
                "=" if angle == 0 => return Start::Expr(j),
                _ => {}
            }
            j += 1;
        }
        Start::None(j)
    }
    /// The index after an expression that starts at `from`.
    pub(super) fn expression_end(&self, from: usize) -> usize {
        let mut j = from;
        while j < self.toks.len() && (j == from || !self.ends(j)) {
            if matches!(self.text(j), "(" | "[" | "{") {
                j = self.pairs[&j];
            }
            j += 1;
        }
        j
    }
    /// The `(` of the parameter list of the named function at `keyword`.
    pub(super) fn params(&self, keyword: usize) -> Option<usize> {
        let mut angle = 0usize;
        let mut j = keyword + 1;
        while !self.ends(j) && !matches!(self.text(j), "{" | "=") {
            match self.text(j) {
                "(" if angle == 0 => return (j > keyword + 1 && self.is_name(j - 1)).then_some(j),
                "(" | "[" => j = self.pairs[&j],
                "<" => angle += 1,
                ">" => angle = angle.saturating_sub(1),
                _ => {}
            }
            j += 1;
        }
        None
    }
    /// The function header from `keyword` to its parameter list at `open`
    /// names a receiver type: `fun T.name(…)`.
    pub(super) fn is_extension(&self, keyword: usize, open: usize) -> bool {
        let mut angle = 0usize;
        (keyword + 1..open).any(|j| {
            match self.text(j) {
                "<" => angle += 1,
                ">" => angle = angle.saturating_sub(1),
                _ => {}
            }
            angle == 0 && self.text(j) == "."
        })
    }
    /// The dotted name `a.b.C` that starts at `i`.
    pub(super) fn dotted(&self, i: usize) -> String {
        let mut name = self.toks[i].text.clone();
        let mut j = i + 1;
        while self.text(j) == "." && self.is_name(j + 1) {
            name.push('.');
            name.push_str(&self.toks[j + 1].text);
            j += 2;
        }
        name
    }
    /// Modifiers directly before the declaration keyword at `at`.
    pub(super) fn modifiers(&self, at: usize) -> BTreeSet<&'a str> {
        (0..at)
            .rev()
            .map(|k| self.text(k))
            .take_while(|word| MODIFIERS.contains(word))
            .collect()
    }
    /// The kind and name of a class-like declaration at `i`.
    pub(super) fn class_at(&self, i: usize) -> Option<(&'static str, String)> {
        let named = self
            .is_name(i + 1)
            .then(|| self.toks[i + 1].text.trim_matches('`').to_string());
        match self.text(i) {
            "class" => {
                let modifiers = self.modifiers(i);
                let kind = ["enum", "annotation"]
                    .into_iter()
                    .find(|m| modifiers.contains(m))
                    .unwrap_or("class");
                Some((kind, named?))
            }
            "interface" => Some(("interface", named?)),
            "object" if named.is_some() => Some(("object", named?)),
            "object" if self.text(i.wrapping_sub(1)) == "companion" => {
                Some(("object", "Companion".into()))
            }
            _ => None,
        }
    }
    /// Names after the header's `:`, sorted; constructor arguments,
    /// type arguments and `by` delegates are not names of supertypes.
    pub(super) fn supertypes(&self, from: usize, to: usize) -> Vec<String> {
        let mut out = Vec::new();
        let (mut angle, mut want) = (0usize, false);
        let mut j = from;
        while j < to && self.text(j) != "where" {
            match self.text(j) {
                "(" | "[" => j = self.pairs[&j],
                "<" => angle += 1,
                ">" => angle = angle.saturating_sub(1),
                // Outside type parameters and parentheses, `:` opens the
                // supertype list and `,` separates its entries.
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
        out
    }
}
