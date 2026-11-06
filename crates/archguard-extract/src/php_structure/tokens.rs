//! The token layer of the PHP scanner: lexer tokens with merged operators,
//! bracket pairs and the keyword tables.

use std::collections::BTreeMap;

use crate::lex::{Flavor, tokens};

/// Multi-character operators, longest first; the lexer emits single characters.
pub(super) const OPERATORS: [&str; 36] = [
    "<=>", "**=", "...", "<<=", ">>=", "??=", "?->", "===", "!==", "->", "=>", "::", "&&", "||",
    "??", "==", "!=", "<>", "<=", ">=", "++", "--", "+=", "-=", "*=", "/=", ".=", "%=", "&=", "|=",
    "^=", "**", "<<", ">>", "?:", "#[",
];
pub(super) const KEYWORDS: [&str; 71] = [
    "abstract",
    "and",
    "array",
    "as",
    "break",
    "callable",
    "case",
    "catch",
    "class",
    "clone",
    "const",
    "continue",
    "declare",
    "default",
    "do",
    "echo",
    "else",
    "elseif",
    "empty",
    "enddeclare",
    "endfor",
    "endforeach",
    "endif",
    "endswitch",
    "endwhile",
    "enum",
    "eval",
    "exit",
    "extends",
    "final",
    "finally",
    "fn",
    "for",
    "foreach",
    "function",
    "global",
    "goto",
    "if",
    "implements",
    "include",
    "include_once",
    "instanceof",
    "insteadof",
    "interface",
    "isset",
    "list",
    "match",
    "namespace",
    "new",
    "or",
    "print",
    "private",
    "protected",
    "public",
    "readonly",
    "require",
    "require_once",
    "return",
    "static",
    "switch",
    "throw",
    "trait",
    "try",
    "unset",
    "use",
    "var",
    "while",
    "xor",
    "yield",
    "self",
    "parent",
];
pub(super) struct Tok {
    pub(super) text: String,
    pub(super) literal: bool,
    pub(super) line: u32,
    pub(super) start: usize,
    pub(super) end: usize,
}

impl Tok {
    pub(super) fn is(&self, word: &str) -> bool {
        !self.literal && self.text.eq_ignore_ascii_case(word)
    }
    pub(super) fn is_name(&self) -> bool {
        !self.literal
            && self
                .text
                .starts_with(|c: char| c.is_alphabetic() || c == '_' || c == '\\')
    }
}

/// Lexer tokens without line breaks, with adjacent operator characters merged.
pub(super) fn merged(source: &str) -> Vec<Tok> {
    let raw: Vec<_> = tokens(source, Flavor::Php)
        .into_iter()
        .filter(|token| token.text != "\n")
        .collect();
    let mut out: Vec<Tok> = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let token = &raw[i];
        let width = OPERATORS
            .iter()
            .find(|op| {
                let n = op.chars().count();
                i + n <= raw.len()
                    && raw[i..i + n].windows(2).all(|w| w[0].end == w[1].start)
                    && raw[i..i + n].iter().all(|t| !t.literal)
                    && raw[i..i + n]
                        .iter()
                        .map(|t| t.text.as_str())
                        .collect::<String>()
                        == **op
            })
            .map_or(1, |op| op.chars().count());
        out.push(Tok {
            text: raw[i..i + width].iter().map(|t| t.text.as_str()).collect(),
            literal: token.literal,
            line: token.line,
            start: token.start,
            end: raw[i + width - 1].end,
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
            "(" | "[" | "{" | "#[" => open.push(index),
            ")" | "]" | "}" => {
                out.insert(open.pop()?, index);
            }
            _ => {}
        }
    }
    open.is_empty().then_some(out)
}

pub(super) fn class_kind(token: &Tok) -> Option<&'static str> {
    ["class", "interface", "trait", "enum"]
        .into_iter()
        .find(|kind| token.is(kind))
}

/// Keywords and language constructs that look like calls.
pub(super) fn is_keyword(word: &str) -> bool {
    let lower = word.to_ascii_lowercase();
    KEYWORDS.contains(&lower.as_str()) || lower == "die" || lower == "__halt_compiler"
}

/// A name segment (not `\`, not a `$variable`).
fn is_word(token: &Tok) -> bool {
    !token.literal
        && token
            .text
            .starts_with(|c: char| c.is_alphabetic() || c == '_')
}

/// The balanced tokens of one file with their bracket pairs, read-only.
pub(super) struct View<'a> {
    pub(super) toks: &'a [Tok],
    pub(super) pairs: &'a BTreeMap<usize, usize>,
    /// The opening bracket of every closing one.
    pub(super) openers: BTreeMap<usize, usize>,
    /// Tokens inside `#[…]` attributes.
    pub(super) in_attr: Vec<bool>,
    pub(super) source: &'a str,
}

impl<'a> View<'a> {
    pub(super) fn new(source: &'a str, toks: &'a [Tok], pairs: &'a BTreeMap<usize, usize>) -> Self {
        let mut in_attr = vec![false; toks.len()];
        for (&open, &close) in pairs.iter().filter(|(open, _)| toks[**open].text == "#[") {
            in_attr[open..=close].fill(true);
        }
        View {
            toks,
            pairs,
            openers: pairs.iter().map(|(&open, &close)| (close, open)).collect(),
            in_attr,
            source,
        }
    }
    /// The code text at `i`; empty for literals and past the end.
    pub(super) fn text(&self, i: usize) -> &str {
        self.toks
            .get(i)
            .filter(|t| !t.literal)
            .map_or("", |t| t.text.as_str())
    }
    pub(super) fn word(&self, i: usize) -> Option<&Tok> {
        self.toks.get(i).filter(|t| is_word(t))
    }
    pub(super) fn is(&self, i: usize, word: &str) -> bool {
        self.toks.get(i).is_some_and(|t| t.is(word))
    }
    pub(super) fn adjacent(&self, a: usize, b: usize) -> bool {
        self.toks[a].end == self.toks[b].start
    }
    /// The first non-literal `want` token at or after `from`.
    pub(super) fn find(&self, from: usize, want: &[&str]) -> Option<usize> {
        (from..self.toks.len()).find(|&i| want.contains(&self.text(i)))
    }

    /// A possibly qualified name (`A\B`, `\A\B`, `namespace\A`) starting at
    /// `i`, and the index after it.
    pub(super) fn qname(&self, i: usize) -> Option<(String, usize)> {
        let mut out = String::new();
        let mut j = i;
        if self.text(j) == "\\" {
            if self.word(j + 1).is_none() || !self.adjacent(j, j + 1) {
                return None;
            }
            out.push('\\');
            j += 1;
        }
        self.word(j)?;
        loop {
            out.push_str(&self.toks[j].text);
            j += 1;
            let more = self.text(j) == "\\"
                && self.adjacent(j - 1, j)
                && self.word(j + 1).is_some()
                && self.adjacent(j, j + 1);
            if !more {
                return Some((out, j));
            }
            out.push('\\');
            j += 1;
        }
    }
    /// True unless `i` continues a qualified name.
    pub(super) fn starts_name(&self, i: usize) -> bool {
        let Some(p) = i.checked_sub(1) else {
            return true;
        };
        let joined = self.text(p) == "\\" || self.word(p).is_some();
        !(joined && self.adjacent(p, i))
    }

    /// The index after an anonymous class body that starts at `new`.
    pub(super) fn anonymous_end(&self, new: usize) -> usize {
        let mut at = new + 2;
        if self.text(at) == "(" {
            at = self.pairs[&at] + 1;
        }
        self.find(at, &["{"])
            .map_or(self.toks.len(), |open| self.pairs[&open] + 1)
    }

    /// The end of the arrow function body that starts at `from`.
    pub(super) fn expression_end(&self, from: usize, to: usize) -> usize {
        let mut j = from;
        while j < to {
            match self.text(j) {
                "(" | "[" | "{" | "#[" => j = self.pairs[&j] + 1,
                ";" | "," | ")" | "]" | "}" => return j,
                _ => j += 1,
            }
        }
        to
    }
}
