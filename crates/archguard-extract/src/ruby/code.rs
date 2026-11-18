//! The token stream of a Ruby file and its block structure: which keyword
//! or brace opens what, and where each block ends. Shared by the
//! declaration scan ([`super::structure`]) and the call sites
//! ([`super::sites`]).
use std::collections::BTreeMap;

use crate::lex::{Flavor, Token};

/// Reserved words: operators for Halstead, never method calls.
pub(super) const KEYWORDS: [&str; 37] = [
    "alias",
    "and",
    "begin",
    "break",
    "case",
    "class",
    "def",
    "defined?",
    "do",
    "else",
    "elsif",
    "end",
    "ensure",
    "for",
    "if",
    "in",
    "module",
    "next",
    "not",
    "or",
    "redo",
    "rescue",
    "retry",
    "return",
    "then",
    "undef",
    "unless",
    "until",
    "when",
    "while",
    "yield",
    "__method__",
    "BEGIN",
    "END",
    "__FILE__",
    "__LINE__",
    "__dir__",
];

/// What an opening keyword or brace starts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Kind {
    Class,
    Module,
    /// `class << self`.
    Singleton,
    Def,
    /// `if` / `unless` with a body.
    Cond,
    /// `while` / `until` / `for` with a body.
    Loop,
    Case,
    Begin,
    /// `do … end` or a `{ … }` block.
    Block,
    /// A hash literal.
    Hash,
}

impl Kind {
    pub(super) fn nests(self) -> bool {
        matches!(self, Kind::Cond | Kind::Loop | Kind::Case | Kind::Block)
    }
}

/// The tokens of a file (newlines included) and its block structure.
pub(super) struct Code<'a> {
    pub source: &'a str,
    pub toks: Vec<Token>,
    /// Opening token → its `end` or `}`.
    pub pairs: BTreeMap<usize, usize>,
    pub kinds: BTreeMap<usize, Kind>,
    line_starts: Vec<usize>,
}

impl<'a> Code<'a> {
    /// `None` when `end`s and braces do not balance.
    pub(super) fn new(source: &'a str) -> Option<Self> {
        let mut code = Code {
            source,
            toks: crate::lex::tokens(source, Flavor::Ruby),
            pairs: BTreeMap::new(),
            kinds: BTreeMap::new(),
            line_starts: std::iter::once(0)
                .chain(source.match_indices('\n').map(|(at, _)| at + 1))
                .collect(),
        };
        code.blocks()?;
        Some(code)
    }

    /// The code text at `i`; empty for literals and past the end.
    pub(super) fn text(&self, i: usize) -> &str {
        self.toks
            .get(i)
            .filter(|t| !t.literal)
            .map_or("", |t| t.text.as_str())
    }

    /// The token before `i` is a method-call dot: `x.class` is no keyword.
    pub(super) fn after_dot(&self, i: usize) -> bool {
        i > 0 && matches!(self.text(i - 1), "." | "&." | "::")
    }

    pub(super) fn column(&self, i: usize) -> u32 {
        let token = &self.toks[i];
        (token.start - self.line_starts[token.line as usize - 1] + 1) as u32
    }

    /// A keyword at `i` begins a statement, so `if`/`while` there are not
    /// modifiers.
    pub(super) fn statement_start(&self, i: usize) -> bool {
        let Some(previous) = i.checked_sub(1).map(|k| &self.toks[k]) else {
            return true;
        };
        if previous.literal {
            return false;
        }
        let text = previous.text.as_str();
        matches!(
            text,
            "\n" | ";"
                | "("
                | "["
                | "{"
                | ","
                | "|"
                | "||"
                | "&&"
                | "and"
                | "or"
                | "not"
                | "!"
                | "return"
                | "then"
                | "do"
                | "else"
                | "begin"
                | "=>"
                | "?"
                | ":"
                | "<<"
        ) || (text.ends_with('=') && !matches!(text, "==" | "!=" | ">=" | "<=" | "==="))
    }

    /// `{` opens a block after a name, `)` or `->`; otherwise a hash.
    pub(super) fn brace_kind(&self, i: usize) -> Kind {
        let previous = i.checked_sub(1).map(|k| &self.toks[k]);
        let block = previous.is_some_and(|t| {
            !t.literal
                && (matches!(t.text.as_str(), ")" | "->")
                    || (t.text.starts_with(|c: char| c.is_alphabetic() || c == '_')
                        && !t.text.ends_with(':')
                        && !KEYWORDS.contains(&t.text.as_str())))
        });
        if block { Kind::Block } else { Kind::Hash }
    }

    /// Pair every opener with its `end` or `}`.
    pub(super) fn blocks(&mut self) -> Option<()> {
        let mut stack: Vec<usize> = Vec::new();
        // The line of a `while`/`until`/`for` whose optional `do` is still due.
        let mut loop_do: Option<u32> = None;
        for i in 0..self.toks.len() {
            let word = self.text(i);
            if matches!(word, "\n" | ";") {
                loop_do = None;
            }
            if self.after_dot(i) || self.toks[i].literal {
                continue;
            }
            if matches!(word, "end" | "}") {
                let open = stack.pop()?;
                if (self.text(open) == "{") != (word == "}") {
                    return None;
                }
                self.pairs.insert(open, i);
            } else if let Some(kind) = self.opener(i, &mut loop_do) {
                self.kinds.insert(i, kind);
                stack.push(i);
            }
        }
        stack.is_empty().then_some(())
    }

    /// What the keyword or brace at `i` opens, if anything.
    fn opener(&self, i: usize, loop_do: &mut Option<u32>) -> Option<Kind> {
        match self.text(i) {
            "class" => Some(if self.text(i + 1) == "<<" {
                Kind::Singleton
            } else {
                Kind::Class
            }),
            "module" => Some(Kind::Module),
            "def" => self.def_header(i).filter(|h| !h.endless).map(|_| Kind::Def),
            "if" | "unless" if self.statement_start(i) => Some(Kind::Cond),
            "while" | "until" | "for" if self.statement_start(i) => {
                *loop_do = Some(self.toks[i].line);
                Some(Kind::Loop)
            }
            "case" => Some(Kind::Case),
            "begin" => Some(Kind::Begin),
            // The optional `do` of a loop opens no block.
            "do" => loop_do.take().is_none().then_some(Kind::Block),
            "{" => Some(self.brace_kind(i)),
            _ => None,
        }
    }

    /// The index after the bracket group that opens at `open`.
    pub(super) fn after_group(&self, open: usize) -> usize {
        let mut depth = 0usize;
        for k in open..self.toks.len() {
            match self.text(k) {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" => {
                    depth -= 1;
                    if depth == 0 {
                        return k + 1;
                    }
                }
                _ => {}
            }
        }
        self.toks.len()
    }

    /// The first newline or `;` at bracket depth 0 from `from`.
    pub(super) fn line_end(&self, from: usize) -> usize {
        let mut k = from;
        while k < self.toks.len() && !matches!(self.text(k), "\n" | ";") {
            k = if matches!(self.text(k), "(" | "[" | "{") {
                self.after_group(k)
            } else {
                k + 1
            };
        }
        k
    }

    /// The method name starting at `*j` (`x`, `x?`, `x=`, `==`, `[]=`);
    /// moves `*j` past it.
    fn def_name(&self, j: &mut usize) -> String {
        let mut name = String::new();
        while *j < self.toks.len() && !matches!(self.text(*j), "(" | "\n" | ";" | "") {
            // `def x = 1` is endless; `def x=(v)` is a setter.
            let touching = self.toks[*j].start == self.toks[*j - 1].end;
            if self.text(*j) == "=" && !name.is_empty() && !(touching && self.text(*j + 1) == "(") {
                break;
            }
            name.push_str(self.text(*j));
            *j += 1;
            if name.starts_with(|c: char| c.is_alphabetic() || c == '_') && self.text(*j) != "=" {
                break;
            }
        }
        name
    }

    /// The header of the `def` at `i`.
    pub(super) fn def_header(&self, i: usize) -> Option<Header> {
        let mut j = i + 1;
        let receiver = (matches!(self.text(j + 1), "." | "&.")
            && self
                .text(j)
                .starts_with(|c: char| c.is_alphabetic() || c == '_'))
        .then(|| {
            j += 2;
            self.text(j - 2).to_string()
        });
        let name = self.def_name(&mut j);
        if name.is_empty() {
            return None;
        }
        let params = if self.text(j) == "(" {
            let close = self.after_group(j);
            let range = (j + 1, close - 1);
            j = close;
            range
        } else if self.text(j) == "=" {
            (j, j)
        } else {
            let end = self.line_end(j);
            let range = (j, end);
            j = end;
            range
        };
        let endless = self.text(j) == "=";
        let (body, end) = if endless {
            let end = self.line_end(j + 1);
            ((j + 1, end), end)
        } else {
            let close = *self.pairs.get(&i).unwrap_or(&self.toks.len());
            ((j, close), close)
        };
        Some(Header {
            name,
            singleton: receiver.is_some(),
            params,
            body,
            end,
            endless,
            signature_end: j.min(self.toks.len()).saturating_sub(1),
        })
    }
}

/// The parts of a `def`.
pub(super) struct Header {
    pub name: String,
    /// `def self.x` (or any other receiver).
    pub singleton: bool,
    /// Token range of the parameters.
    pub params: (usize, usize),
    /// Token range of the body, exclusive of `end`.
    pub body: (usize, usize),
    /// The `end` token, or the line end of an endless method.
    pub end: usize,
    pub endless: bool,
    /// The last token of the signature.
    pub signature_end: usize,
}

/// The index after a nested `def`, `class`, `module` or `class << x` at `i`
/// inside a method body, which is not measured with it.
pub(super) fn nested_scope_end(code: &Code, i: usize) -> Option<usize> {
    match code.kinds.get(&i)? {
        Kind::Def | Kind::Class | Kind::Module | Kind::Singleton => Some(code.pairs[&i] + 1),
        _ => None,
    }
}
