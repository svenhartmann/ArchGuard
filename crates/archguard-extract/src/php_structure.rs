//! Token-based PHP structure and measurements: classes, functions and methods
//! with per-function complexity, cognitive and Halstead counts. Nothing is
//! executed and no PHP installation is needed; calls and types stay unresolved.

//!
//! Rules mirror the Rust and Python analyzers (`ast-decisions-v1`,
//! `source-maintainability/v1`):
//! - cyclomatic: `if`/`elseif`/`else if`, `for`, `foreach`, `while`, `do`,
//!   each `case`, each `match` arm except `default`, each `catch`, ternaries
//!   (`?`, `?:`), `&&`, `||`, `and`, `or`, `??`, `??=` and `?->`.
//! - cognitive: `if`, loops, `switch`, `match`, `catch` and ternaries add 1 plus
//!   nesting; `elseif` and `else` add 1; a run of logical operators adds 1 plus
//!   1 per change of operator; `break n`/`continue n` add 1. `try` adds nothing.
//! - nesting follows braced control bodies, closures and arrow functions;
//!   nested named functions, classes and anonymous classes are not measured.
//! - Halstead: keywords and operators are operators; variables, names and
//!   literals are operands; brackets, `;` and `,` are neither.

use std::collections::{BTreeMap, BTreeSet};

use archguard_core::{CallClass, CallMethod, MethodComplexity, SourceMethodMetrics};

use crate::lex::{Flavor, tokens};

/// Multi-character operators, longest first; the lexer emits single characters.
const OPERATORS: [&str; 36] = [
    "<=>", "**=", "...", "<<=", ">>=", "??=", "?->", "===", "!==", "->", "=>", "::", "&&", "||",
    "??", "==", "!=", "<>", "<=", ">=", "++", "--", "+=", "-=", "*=", "/=", ".=", "%=", "&=", "|=",
    "^=", "**", "<<", ">>", "?:", "#[",
];
const KEYWORDS: [&str; 71] = [
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
const MODIFIERS: [&str; 7] = [
    "public",
    "protected",
    "private",
    "static",
    "abstract",
    "final",
    "readonly",
];

struct Tok {
    text: String,
    literal: bool,
    line: u32,
    start: usize,
    end: usize,
}

impl Tok {
    fn is(&self, word: &str) -> bool {
        !self.literal && self.text.eq_ignore_ascii_case(word)
    }
    fn is_name(&self) -> bool {
        !self.literal
            && self
                .text
                .starts_with(|c: char| c.is_alphabetic() || c == '_' || c == '\\')
    }
}

/// Lexer tokens without line breaks, with adjacent operator characters merged.
fn merged(source: &str) -> Vec<Tok> {
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
fn pairs(toks: &[Tok]) -> Option<BTreeMap<usize, usize>> {
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

/// Classes, methods and raw measurements of one PHP file.
#[derive(Debug, Default)]
pub struct PhpStructure {
    pub classes: Vec<CallClass>,
    pub methods: Vec<CallMethod>,
    pub metrics: Vec<SourceMethodMetrics>,
    pub diagnostics: Vec<String>,
}

/// Scan one PHP source. `file` is the source path, `graph_node` the name of
/// its node in the dependency graph.
pub fn scan_php_structure(file: &str, graph_node: &str, source: &str) -> PhpStructure {
    let toks = merged(source);
    let mut scan = Scan {
        file,
        graph_node,
        source,
        toks: &toks,
        pairs: BTreeMap::new(),
        package: file.rsplit_once('/').map_or("", |(dir, _)| dir).to_string(),
        out: PhpStructure::default(),
        seen: BTreeSet::new(),
    };
    match pairs(&toks) {
        Some(found) => {
            scan.pairs = found;
            scan.walk();
        }
        None => scan
            .out
            .diagnostics
            .push(format!("{file}: unbalanced brackets; not measured")),
    }
    scan.out
}

struct Scan<'a> {
    file: &'a str,
    graph_node: &'a str,
    source: &'a str,
    toks: &'a [Tok],
    pairs: BTreeMap<usize, usize>,
    package: String,
    out: PhpStructure,
    seen: BTreeSet<String>,
}

impl Scan<'_> {
    fn tok(&self, index: usize) -> Option<&Tok> {
        self.toks.get(index)
    }
    fn is(&self, index: usize, word: &str) -> bool {
        self.tok(index).is_some_and(|t| t.is(word))
    }
    /// The first non-literal `want` token at or after `from`.
    fn find(&self, from: usize, want: &[&str]) -> Option<usize> {
        (from..self.toks.len())
            .find(|&i| !self.toks[i].literal && want.contains(&self.toks[i].text.as_str()))
    }
    fn klass(
        &mut self,
        name: &str,
        kind: &str,
        line: u32,
        is_abstract: bool,
        supertypes: Vec<String>,
    ) -> String {
        let id = format!("{}::{name}", self.file);
        if self.seen.insert(id.clone()) {
            self.out.classes.push(CallClass {
                language: Some("php".into()),
                id: id.clone(),
                name: name.into(),
                package: self.package.clone(),
                file: self.file.into(),
                graph_node: self.graph_node.into(),
                line,
                kind: kind.into(),
                is_abstract: Some(is_abstract),
                visibility: None,
                supertypes,
                supertype_sites: None,
                declaration_files: None,
            });
        }
        id
    }

    /// Declarations outside function bodies; bodies are measured and skipped.
    fn walk(&mut self) {
        // One entry per open brace: the class it opens, if any.
        let mut braces: Vec<Option<String>> = Vec::new();
        let mut i = 0;
        while i < self.toks.len() {
            let token = &self.toks[i];
            let after_scope =
                i > 0 && matches!(self.toks[i - 1].text.as_str(), "::" | "->" | "?->");
            if token.literal || after_scope {
                i += 1;
                continue;
            }
            if token.is("namespace") && self.tok(i + 1).is_some_and(Tok::is_name) {
                let end = self.find(i + 1, &[";", "{"]).unwrap_or(self.toks.len());
                self.package = self.toks[i + 1..end]
                    .iter()
                    .map(|t| t.text.as_str())
                    .collect();
                i = end;
                continue;
            }
            if token.is("new") && self.is(i + 1, "class") {
                i = self.anonymous_end(i);
                continue;
            }
            if let Some(kind) = class_kind(token)
                && self.tok(i + 1).is_some_and(Tok::is_name)
                && let Some(open) = self.find(i + 1, &["{"])
            {
                let modifiers = self.modifiers(i);
                let supertypes = supertypes(&self.toks[i + 2..open]);
                let is_abstract = kind == "interface" || modifiers.contains("abstract");
                let name = self.toks[i + 1].text.clone();
                let id = self.klass(&name, kind, token.line, is_abstract, supertypes);
                braces.push(Some(id));
                i = open + 1;
                continue;
            }
            if token.is("function") {
                let named = if self.tok(i + 1).is_some_and(|t| t.text == "&") {
                    i + 2
                } else {
                    i + 1
                };
                if self.tok(named).is_some_and(Tok::is_name)
                    && self.tok(named + 1).is_some_and(|t| t.text == "(")
                {
                    let owner = braces.last().cloned().flatten();
                    i = self.function(i, named, owner);
                    continue;
                }
            }
            match token.text.as_str() {
                "{" => braces.push(None),
                "}" => {
                    braces.pop();
                }
                _ => {}
            }
            i += 1;
        }
    }

    /// The index after an anonymous class body that starts at `new`.
    fn anonymous_end(&self, new: usize) -> usize {
        let mut at = new + 2;
        if self.tok(at).is_some_and(|t| t.text == "(") {
            at = self.pairs[&at] + 1;
        }
        self.find(at, &["{"])
            .map_or(self.toks.len(), |open| self.pairs[&open] + 1)
    }

    /// Lowercase modifiers directly before the declaration keyword at `at`.
    fn modifiers(&self, at: usize) -> BTreeSet<String> {
        self.toks[..at]
            .iter()
            .rev()
            .take_while(|t| MODIFIERS.iter().any(|m| t.is(m)))
            .map(|t| t.text.to_ascii_lowercase())
            .collect()
    }

    /// Record and measure the function declared at `keyword`; returns the
    /// index after its body (or after `;` for a declaration without one).
    fn function(&mut self, keyword: usize, named: usize, owner: Option<String>) -> usize {
        let close = self.pairs[&(named + 1)];
        let Some(end) = self.find(close + 1, &["{", ";"]) else {
            return self.toks.len();
        };
        if self.toks[end].text == ";" {
            return end + 1;
        }
        let body_end = self.pairs[&end];
        let in_class = owner.is_some();
        let class_id =
            owner.unwrap_or_else(|| self.klass("(module)", "module", 1, false, Vec::new()));
        let name = self.toks[named].text.clone();
        let line = self.toks[keyword].line;
        let id = format!("{class_id}#{name}@{line}");
        let signature = self.source[self.toks[keyword].start..self.toks[end - 1].end]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let measure = Measure::of(self.toks, &self.pairs, end + 1, body_end);
        self.out.methods.push(CallMethod {
            id: id.clone(),
            class_id,
            name,
            signature,
            file: self.file.into(),
            line,
            end_line: Some(self.toks[body_end].line),
            is_static: !in_class || self.modifiers(keyword).contains("static"),
            complexity: Some(MethodComplexity {
                cyclomatic: measure.cyclomatic,
                max_nesting: measure.max_nesting,
                definition: "ast-decisions-v1".into(),
            }),
        });
        self.out.metrics.push(SourceMethodMetrics {
            method_id: id,
            definition: "source-maintainability/v1".into(),
            cognitive: measure.cognitive,
            distinct_operators: measure.operators.len() as u32,
            distinct_operands: measure.operands.len() as u32,
            operators: measure.operator_count,
            operands: measure.operand_count,
            source_lines: measure.lines.len() as u32,
        });
        body_end + 1
    }
}

fn class_kind(token: &Tok) -> Option<&'static str> {
    ["class", "interface", "trait", "enum"]
        .into_iter()
        .find(|kind| token.is(kind))
}

/// Names after `extends`/`implements`, sorted.
fn supertypes(header: &[Tok]) -> Vec<String> {
    let mut out = Vec::new();
    let mut collecting = false;
    let mut current = String::new();
    for token in header {
        if token.is("extends") || token.is("implements") || token.text == "," {
            out.extend((!current.is_empty()).then(|| std::mem::take(&mut current)));
            collecting |= token.text != ",";
        } else if collecting && token.is_name() {
            current.push_str(&token.text);
        }
    }
    out.extend((!current.is_empty()).then_some(current));
    out.sort();
    out
}

/// How a brace inside a measured body affects nesting.
#[derive(Clone, Copy, PartialEq)]
enum Block {
    Plain,
    Nest,
    Do,
    Match,
}

/// An open `match` body: its bracket level and the state of the current arm.
struct Arm {
    level: usize,
    expect: bool,
    default: bool,
}

#[derive(Default)]
struct Measure {
    cyclomatic: u32,
    cognitive: u32,
    depth: u32,
    max_nesting: u32,
    operators: BTreeSet<String>,
    operands: BTreeSet<String>,
    operator_count: u32,
    operand_count: u32,
    lines: BTreeSet<u32>,
}

impl Measure {
    /// Measure the body tokens `from..to` (exclusive of its braces).
    fn of(toks: &[Tok], pairs: &BTreeMap<usize, usize>, from: usize, to: usize) -> Self {
        let body = Body {
            toks,
            pairs,
            from,
            to,
        };
        let marks = Marks::of(&body);
        Walk {
            body,
            marks,
            m: Measure {
                cyclomatic: 1,
                ..Default::default()
            },
            stack: Vec::new(),
            arms: Vec::new(),
            open_arrows: Vec::new(),
            level: 0,
            logic: None,
            closed_do: None,
        }
        .run()
    }

    /// A decision that counts once and adds cognitive load: with nesting
    /// (`nested`) for a new construct, flat for `elseif` / `else if`.
    fn decision(&mut self, nested: bool) {
        self.cyclomatic += 1;
        self.cognitive += if nested { 1 + self.depth } else { 1 };
        self.max_nesting = self.max_nesting.max(self.depth + 1);
    }

    /// `switch` / `match`: cognitive load with nesting, but no decision of its
    /// own (its cases and arms count).
    fn branch(&mut self) {
        self.cognitive += 1 + self.depth;
        self.max_nesting = self.max_nesting.max(self.depth + 1);
    }

    fn nest(&mut self, by: u32) {
        self.depth += by;
        self.max_nesting = self.max_nesting.max(self.depth);
    }

    fn halstead(&mut self, token: &Tok) {
        if !token.literal
            && matches!(
                token.text.as_str(),
                "(" | ")" | "[" | "]" | "{" | "}" | ";" | ","
            )
        {
            return;
        }
        let lower = token.text.to_ascii_lowercase();
        let operand = token.literal
            || token.text.starts_with('$')
            || token.text.starts_with(|c: char| c.is_ascii_digit())
            || (token.is_name() && !KEYWORDS.contains(&lower.as_str()));
        if operand {
            self.operands.insert(token.text.clone());
            self.operand_count += 1;
        } else {
            self.operators.insert(lower);
            self.operator_count += 1;
        }
        self.lines.insert(token.line);
    }
}

/// Control keywords whose `( … )` condition may be followed by a braced body.
const CONDITIONED: [&str; 8] = [
    "if", "elseif", "for", "foreach", "while", "switch", "catch", "match",
];

/// A measured body: the tokens `from..to` and the bracket pairs of the file.
struct Body<'a> {
    toks: &'a [Tok],
    pairs: &'a BTreeMap<usize, usize>,
    from: usize,
    to: usize,
}

impl Body<'_> {
    fn is(&self, i: usize, word: &str) -> bool {
        self.toks.get(i).is_some_and(|t| t.is(word))
    }
    /// The code text at `i`; empty for literals and past the end.
    fn text(&self, i: usize) -> &str {
        self.toks
            .get(i)
            .filter(|t| !t.literal)
            .map_or("", |t| t.text.as_str())
    }
    /// The first `want` token in `at..to`.
    fn find(&self, at: usize, want: &str) -> Option<usize> {
        (at..self.to).find(|&k| self.text(k) == want)
    }
    /// The first `{` or `;` in `at..to`.
    fn brace_after(&self, at: usize) -> Option<usize> {
        (at..self.to).find(|&k| matches!(self.text(k), "{" | ";"))
    }
    /// A named class-like declaration at `i` (not `Foo::class` or `$o->enum`).
    fn names_class(&self, i: usize) -> bool {
        class_kind(&self.toks[i]).is_some()
            && self.toks.get(i + 1).is_some_and(Tok::is_name)
            && !matches!(self.text(i.wrapping_sub(1)), "::" | "->" | "?->")
    }
}

/// What a pre-pass over a body finds: how each brace nests, nested scopes
/// to skip (start index to the index after them) and arrow-function `=>`s.
#[derive(Default)]
struct Marks {
    blocks: BTreeMap<usize, Block>,
    skip: BTreeMap<usize, usize>,
    arrows: BTreeSet<usize>,
}

impl Marks {
    fn of(body: &Body) -> Self {
        let mut marks = Marks::default();
        for i in body.from..body.to {
            marks.mark(body, i);
        }
        marks
    }

    fn mark(&mut self, b: &Body, i: usize) {
        if CONDITIONED.iter().any(|w| b.is(i, w)) && b.text(i + 1) == "(" {
            let kind = if b.is(i, "match") {
                Block::Match
            } else {
                Block::Nest
            };
            self.block(b, b.pairs[&(i + 1)] + 1, kind);
        } else if (b.is(i, "else") || b.is(i, "do")) && b.text(i + 1) == "{" {
            let kind = if b.is(i, "do") {
                Block::Do
            } else {
                Block::Nest
            };
            self.blocks.insert(i + 1, kind);
        } else if b.is(i, "function") {
            self.function(b, i);
        } else if b.is(i, "fn") && b.text(i + 1) == "(" {
            self.arrows.extend(b.find(b.pairs[&(i + 1)] + 1, "=>"));
        } else if b.is(i, "new") && b.is(i + 1, "class") {
            self.anonymous(b, i);
        } else if b.names_class(i) {
            self.skip_body(b, i, i);
        }
    }

    /// Mark `open` as a `kind` block if it is a brace.
    fn block(&mut self, b: &Body, open: usize, kind: Block) {
        if b.text(open) == "{" {
            self.blocks.insert(open, kind);
        }
    }

    /// Skip an anonymous class, from `new` past its body.
    fn anonymous(&mut self, b: &Body, new: usize) {
        let mut at = new + 2;
        if b.text(at) == "(" {
            at = b.pairs[&at] + 1;
        }
        self.skip_body(b, new, at);
    }

    /// Skip from `i` past the braced body that starts at or after `at`.
    fn skip_body(&mut self, b: &Body, i: usize, at: usize) {
        if let Some(open) = b.find(at, "{") {
            self.skip.insert(i, b.pairs[&open] + 1);
        }
    }

    /// A closure nests its body; a named function inside a body is skipped.
    fn function(&mut self, b: &Body, i: usize) {
        let paren = if b.text(i + 1) == "&" { i + 2 } else { i + 1 };
        if b.text(paren) == "(" {
            // Closure: optional `use (…)` and return type before the body.
            let mut at = b.pairs[&paren] + 1;
            if b.is(at, "use") && b.text(at + 1) == "(" {
                at = b.pairs[&(at + 1)] + 1;
            }
            if let Some(open) = b.brace_after(at).filter(|&k| b.text(k) == "{") {
                self.blocks.insert(open, Block::Nest);
            }
        } else if b.text(paren + 1) == "(" {
            let end = b.brace_after(b.pairs[&(paren + 1)] + 1).map_or(b.to, |k| {
                if b.text(k) == "{" {
                    b.pairs[&k] + 1
                } else {
                    k + 1
                }
            });
            self.skip.insert(i, end);
        }
    }
}

/// The measuring pass over a body, token by token.
struct Walk<'a> {
    body: Body<'a>,
    marks: Marks,
    m: Measure,
    stack: Vec<Block>,
    arms: Vec<Arm>,
    /// Open arrow functions: bracket level and brace depth at their `=>`.
    open_arrows: Vec<(usize, usize)>,
    level: usize,
    /// The current run of logical operators: its level and whether it is `&&`.
    logic: Option<(usize, bool)>,
    closed_do: Option<usize>,
}

impl Walk<'_> {
    fn run(mut self) -> Measure {
        let mut i = self.body.from;
        while i < self.body.to {
            if let Some(&end) = self.marks.skip.get(&i) {
                i = end;
                continue;
            }
            self.step(i);
            i += 1;
        }
        self.m
    }

    fn step(&mut self, i: usize) {
        let token = &self.body.toks[i];
        let word = if token.literal {
            ""
        } else {
            token.text.as_str()
        };
        self.close_arrows(word);
        if let Some(arm) = self.arm()
            && arm.expect
            && !matches!(word, "{" | "}")
        {
            arm.default = token.is("default");
            arm.expect = false;
        }
        self.m.halstead(token);
        let lower = word.to_ascii_lowercase();
        if !self.bracket(i, &lower) && !self.keyword(i, &lower) {
            self.operator(i, &lower);
        }
    }

    /// End the arrow functions that `word` closes.
    fn close_arrows(&mut self, word: &str) {
        while let Some(&(level, depth)) = self.open_arrows.last() {
            let here = level == self.level;
            let same_block = self.stack.len() == depth;
            let ends = (matches!(word, ";" | ",") && here && same_block)
                || (matches!(word, ")" | "]") && here)
                || (word == "}" && same_block);
            if !ends {
                break;
            }
            self.open_arrows.pop();
            self.m.depth -= 1;
        }
    }

    /// The innermost `match` arm, if the walk is directly in its body.
    fn arm(&mut self) -> Option<&mut Arm> {
        let in_match = self.stack.last() == Some(&Block::Match);
        let level = self.level;
        self.arms
            .last_mut()
            .filter(|arm| in_match && arm.level == level)
    }

    /// Brackets and separators; false for any other word.
    fn bracket(&mut self, i: usize, word: &str) -> bool {
        match word {
            "(" | "[" | "#[" => self.level += 1,
            ")" | "]" => {
                self.level = self.level.saturating_sub(1);
                self.logic = self.logic.filter(|(lv, _)| *lv <= self.level);
            }
            "{" => self.open_brace(i),
            "}" => self.close_brace(i),
            ";" => self.logic = None,
            "," => {
                self.logic = None;
                if let Some(arm) = self.arm() {
                    arm.expect = true;
                }
            }
            _ => return false,
        }
        true
    }

    fn open_brace(&mut self, i: usize) {
        let block = self.marks.blocks.get(&i).copied().unwrap_or(Block::Plain);
        if block != Block::Plain {
            self.m.nest(1);
        }
        if block == Block::Match {
            self.arms.push(Arm {
                level: self.level,
                expect: true,
                default: false,
            });
        }
        self.stack.push(block);
        self.logic = None;
    }

    fn close_brace(&mut self, i: usize) {
        let block = self.stack.pop().unwrap_or(Block::Plain);
        if block != Block::Plain {
            self.m.depth -= 1;
        }
        if block == Block::Match {
            self.arms.pop();
        }
        self.closed_do = (block == Block::Do).then_some(i);
        self.logic = None;
    }

    /// Control keywords; false for any other word.
    fn keyword(&mut self, i: usize, word: &str) -> bool {
        let b = &self.body;
        match word {
            "if" => {
                let ladder = i > b.from && b.toks[i - 1].is("else");
                self.m.decision(!ladder);
            }
            "elseif" => self.m.decision(false),
            "else" if !b.is(i + 1, "if") => self.m.cognitive += 1,
            "while" if self.closed_do == Some(i.wrapping_sub(1)) => {}
            "for" | "foreach" | "while" | "do" | "catch" => self.m.decision(true),
            "switch" | "match" if b.text(i + 1) == "(" => self.m.branch(),
            "case" => self.m.cyclomatic += 1,
            "break" | "continue" if self.jumps_out(i) => self.m.cognitive += 1,
            _ => return false,
        }
        true
    }

    /// `break n` / `continue n` with `n > 1`.
    fn jumps_out(&self, i: usize) -> bool {
        self.body
            .toks
            .get(i + 1)
            .is_some_and(|t| t.text.parse::<u32>().is_ok_and(|n| n > 1))
    }

    /// Ternaries, null-safe and logical operators and `=>`.
    fn operator(&mut self, i: usize, word: &str) {
        match word {
            "?" if !matches!(self.body.text(i.wrapping_sub(1)), "(" | "," | ":" | "|") => {
                self.ternary();
            }
            "?:" => self.ternary(),
            "??" | "??=" | "?->" => self.m.cyclomatic += 1,
            "&&" | "and" => self.logical(true),
            "||" | "or" => self.logical(false),
            "=>" if self.marks.arrows.contains(&i) => {
                self.m.nest(1);
                self.open_arrows.push((self.level, self.stack.len()));
            }
            "=>" if self.arm().is_some_and(|arm| !arm.default) => self.m.cyclomatic += 1,
            _ => {}
        }
    }

    fn ternary(&mut self) {
        self.m.decision(true);
        self.logic = None;
    }

    /// One logical operator: a decision; cognitive per run and change of operator.
    fn logical(&mut self, and: bool) {
        self.m.cyclomatic += 1;
        self.m.cognitive += match self.logic {
            Some((level, previous)) if level == self.level => u32::from(previous != and),
            _ => 1,
        };
        self.logic = Some((self.level, and));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(source: &str) -> PhpStructure {
        scan_php_structure("src/Shop.php", "App/Shop/Shop.php", source)
    }
    fn method<'a>(found: &'a PhpStructure, name: &str) -> &'a CallMethod {
        found
            .methods
            .iter()
            .find(|m| m.name == name)
            .unwrap_or_else(|| panic!("no method {name}: {:?}", found.diagnostics))
    }
    /// (cyclomatic, max nesting, cognitive) of one function.
    fn complexity(found: &PhpStructure, name: &str) -> (u32, u32, u32) {
        let method = method(found, name);
        let measured = method.complexity.as_ref().unwrap();
        let cognitive = found
            .metrics
            .iter()
            .find(|m| m.method_id == method.id)
            .unwrap()
            .cognitive;
        (measured.cyclomatic, measured.max_nesting, cognitive)
    }

    #[test]
    fn counts_follow_the_shared_rules() {
        let found = scan(
            r#"<?php
function plain() { return 1; }
function branches($a, $b) {
    if ($a) {
        if ($b) { return 1; } else { return 2; }
    } elseif ($b) {
        return 3;
    } else {
        return 4;
    }
}
function logic($a, $b, $c) { return $a && $b || $c; }
function arms($x) {
    switch ($x) {
        case 0: return 1;
        case 1: return 2;
        default: return 3;
    }
}
function matched($x) {
    return match($x) { 0, 1 => 'low', 2 => ['k' => 'mid'], default => 'high' };
}
function guarded($items) {
    try {
        foreach ($items as $item) {
            while ($item) { $item--; }
        }
    } catch (ValueError $e) {
        return 0;
    } catch (TypeError $e) {
        return 1;
    }
    return 2;
}
function nulls($a) { return $a?->b ?? 'x'; }
function ternary($a) {
    $f = function (?int $y) use ($a) { return $a ? $y : 0; };
    return $f(1);
}
function arrow($a) { $f = fn($y) => $a ? $y : 0; return $f(1); }
function looped($n) { do { $n--; } while ($n > 0); return $n; }
function braceless($a) { if ($a) return 1; return 2; }
function scoped() {
    function inner($a) { if ($a) { return 1; } }
    $o = new class { public function hidden($a) { if ($a) { return 1; } } };
    return 0;
}
"#,
        );
        assert_eq!(complexity(&found, "plain"), (1, 0, 0));
        // Same shape and numbers as the Rust and Python `branches` cases.
        assert_eq!(complexity(&found, "branches"), (4, 2, 6));
        assert_eq!(complexity(&found, "logic"), (3, 0, 2));
        // Each `case` counts, `default` does not; the switch nests once.
        assert_eq!(complexity(&found, "arms"), (3, 1, 1));
        // Each match arm except `default`; `0, 1 =>` is one arm and the
        // array `=>` inside an arm is not an arm.
        assert_eq!(complexity(&found, "matched"), (3, 1, 1));
        // `try` neither counts nor nests, as in Python.
        assert_eq!(complexity(&found, "guarded"), (5, 2, 5));
        // `?->` and `??` are decisions without cognitive load.
        assert_eq!(complexity(&found, "nulls"), (3, 0, 0));
        // Closures and arrow functions are one level deeper; `?int` is a type.
        assert_eq!(complexity(&found, "ternary"), (2, 2, 2));
        assert_eq!(complexity(&found, "arrow"), (2, 2, 2));
        // `do … while` is one loop, not two.
        assert_eq!(complexity(&found, "looped"), (2, 1, 1));
        assert_eq!(complexity(&found, "braceless"), (2, 1, 1));
        // Nested named functions and anonymous classes are their own scope.
        assert_eq!(complexity(&found, "scoped"), (1, 0, 0));
        assert!(
            found
                .methods
                .iter()
                .all(|m| m.name != "inner" && m.name != "hidden")
        );
        let plain = found
            .metrics
            .iter()
            .find(|m| m.method_id == method(&found, "plain").id)
            .unwrap();
        // `return 1;`: one operator, one operand, one line.
        assert_eq!(
            (plain.operators, plain.operands, plain.source_lines),
            (1, 1, 1)
        );
    }

    #[test]
    fn classes_methods_and_abstractness_come_from_declarations() {
        let found = scan(
            r#"<?php
namespace App\Shop;

interface Port { public function send($msg); }

abstract class Repo extends Base implements Port, \Countable {
    abstract protected function load(int $id): ?array;

    public function send($msg) { return $msg; }
}

trait Logs { public function log() {} }

#[Entity]
final class Order {
    public static function create(): self { return new self(); }
}

enum Suit: string { case Hearts = 'H'; }

if (!function_exists('helper')) {
    function helper() { return Order::class; }
}
"#,
        );
        let class = |name: &str| {
            found
                .classes
                .iter()
                .find(|c| c.name == name)
                .unwrap_or_else(|| panic!("no class {name}: {:?}", found.classes))
        };
        let kinds: Vec<_> = ["Port", "Repo", "Logs", "Order", "Suit", "(module)"]
            .iter()
            .map(|name| (class(name).kind.as_str(), class(name).is_abstract))
            .collect();
        assert_eq!(
            kinds,
            [
                ("interface", Some(true)),
                ("class", Some(true)),
                ("trait", Some(false)),
                ("class", Some(false)),
                ("enum", Some(false)),
                ("module", Some(false)),
            ]
        );
        let repo = class("Repo");
        assert_eq!(repo.supertypes, ["Base", "Port", "\\Countable"]);
        assert_eq!(repo.package, "App\\Shop");
        assert_eq!(repo.id, "src/Shop.php::Repo");
        assert_eq!(repo.file, "src/Shop.php");
        assert_eq!(repo.graph_node, "App/Shop/Shop.php");
        assert_eq!(repo.language.as_deref(), Some("php"));
        // Bodiless interface and abstract methods have nothing to measure.
        assert!(found.methods.iter().all(|m| m.name != "load"));
        assert_eq!(found.methods.iter().filter(|m| m.name == "send").count(), 1);
        let send = method(&found, "send");
        assert_eq!(send.class_id, repo.id);
        assert_eq!(send.signature, "function send($msg)");
        assert!(!send.is_static);
        let create = method(&found, "create");
        assert!(create.is_static);
        assert_eq!(create.signature, "function create(): self");
        assert_eq!(method(&found, "log").class_id, "src/Shop.php::Logs");
        // Conditional top-level declarations still belong to the module.
        assert_eq!(method(&found, "helper").class_id, "src/Shop.php::(module)");
        assert_eq!(method(&found, "helper").end_line, Some(22));
    }
}
