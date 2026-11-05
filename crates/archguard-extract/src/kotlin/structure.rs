//! Token-based Kotlin structure and measurements: classes, objects and
//! functions with per-function complexity, cognitive and Halstead counts.
//! No Kotlin compiler is needed; calls and types stay unresolved.
//!
//! Rules mirror the Rust, Python and PHP analyzers (`ast-decisions-v1`,
//! `source-maintainability/v1`):
//! - cyclomatic: `if`, `for`, `while`, `do`, `catch`, each `when` branch
//!   except `else`, `&&`, `||` and the elvis operator `?:`. Safe calls `?.`
//!   are not decisions, as in the TypeScript analyzer.
//! - cognitive: `if`, loops, `when` and `catch` add 1 plus nesting; `else if`
//!   and `else` add 1; a run of logical operators adds 1 plus 1 per change of
//!   operator (a line break ends a run unless an operator continues it);
//!   `break@label` and `continue@label` add 1. `try`, `finally` and `?:` add
//!   nothing.
//! - nesting follows braced control bodies, `when` bodies, lambdas,
//!   anonymous and local functions; braced `when` branch bodies, `try` and
//!   `finally` do not nest. Local functions count toward their enclosing
//!   function; object expressions and local classes are not measured.
//! - Halstead: keywords and operators are operators; names and literals are
//!   operands, a string with its `${…}` templates is one operand; brackets,
//!   `;` and `,` are neither.
//!
//! Recorded functions have a block or expression body; abstract and interface
//! functions without one, property accessors, constructors and `init` blocks
//! are not recorded.

use std::collections::{BTreeMap, BTreeSet};

use archguard_core::{CallClass, CallMethod, MethodComplexity, SourceMethodMetrics};

use crate::lex::{Flavor, tokens};

/// Multi-character operators, longest first; the lexer emits single characters.
const OPERATORS: [&str; 25] = [
    "!==", "===", "..<", "as?", "!is", "!in", "?.", "?:", "!!", "::", "->", "==", "!=", "<=", ">=",
    "&&", "||", "++", "--", "+=", "-=", "*=", "/=", "%=", "..",
];
const KEYWORDS: [&str; 30] = [
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
const DECLARES: [&str; 11] = [
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
const MODIFIERS: [&str; 25] = [
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

struct Tok {
    text: String,
    literal: bool,
    line: u32,
    end_line: u32,
    start: usize,
    end: usize,
}

impl Tok {
    /// A name; backtick-quoted names count although the lexer marks them literal.
    fn is_name(&self) -> bool {
        (!self.literal
            && self
                .text
                .starts_with(|c: char| c.is_alphabetic() || c == '_'))
            || self.text.starts_with('`')
    }
}

/// Lexer tokens without line breaks, with adjacent operator characters merged.
fn merged(source: &str) -> Vec<Tok> {
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
fn pairs(toks: &[Tok]) -> Option<BTreeMap<usize, usize>> {
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
enum Start {
    /// A `{` body at this index.
    Block(usize),
    /// An `=` expression body at this index.
    Expr(usize),
    /// No body; the next declaration starts at this index.
    None(usize),
}

/// Tokens with their bracket pairs, and the lookups both passes share.
struct Code<'a> {
    toks: &'a [Tok],
    pairs: BTreeMap<usize, usize>,
}

impl<'a> Code<'a> {
    /// The code text at `i`; empty for literals and past the end.
    fn text(&self, i: usize) -> &'a str {
        self.toks
            .get(i)
            .filter(|t| !t.literal)
            .map_or("", |t| t.text.as_str())
    }
    fn is_name(&self, i: usize) -> bool {
        self.toks.get(i).is_some_and(Tok::is_name)
    }
    /// `i` is a member name such as `Foo::class` or `x.object`.
    fn after_member(&self, i: usize) -> bool {
        i > 0 && matches!(self.text(i - 1), "." | "?." | "::")
    }
    /// The token at `i` opens a new source line.
    fn new_line(&self, i: usize) -> bool {
        i > 0 && i < self.toks.len() && self.toks[i].line > self.toks[i - 1].end_line
    }
    /// A declaration starts at `i`: a declaring keyword, an annotation, or
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
    /// The body after a declaration header that continues at `from`.
    fn body_start(&self, from: usize) -> Start {
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
    fn expression_end(&self, from: usize) -> usize {
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
    fn params(&self, keyword: usize) -> Option<usize> {
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
    /// The dotted name `a.b.C` that starts at `i`.
    fn dotted(&self, i: usize) -> String {
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
    fn modifiers(&self, at: usize) -> BTreeSet<&'a str> {
        (0..at)
            .rev()
            .map(|k| self.text(k))
            .take_while(|word| MODIFIERS.contains(word))
            .collect()
    }
    /// The kind and name of a class-like declaration at `i`.
    fn class_at(&self, i: usize) -> Option<(&'static str, String)> {
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
    fn supertypes(&self, from: usize, to: usize) -> Vec<String> {
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

/// Classes, functions and raw measurements of one Kotlin file.
#[derive(Debug, Default)]
pub struct KotlinStructure {
    pub classes: Vec<CallClass>,
    pub methods: Vec<CallMethod>,
    pub metrics: Vec<SourceMethodMetrics>,
    pub diagnostics: Vec<String>,
}

/// Scan one Kotlin source. `file` is the source path, `graph_node` the name
/// of its node in the dependency graph.
pub fn scan_kotlin_structure(file: &str, graph_node: &str, source: &str) -> KotlinStructure {
    let toks = merged(source);
    let mut out = KotlinStructure::default();
    let Some(pairs) = pairs(&toks) else {
        out.diagnostics
            .push(format!("{file}: unbalanced brackets; not measured"));
        return out;
    };
    let mut scan = Scan {
        file,
        graph_node,
        source,
        code: Code { toks: &toks, pairs },
        package: file.rsplit_once('/').map_or("", |(dir, _)| dir).to_string(),
        out,
        seen: BTreeSet::new(),
    };
    scan.walk();
    scan.out
}

/// An open class body: its id and whether it is an `object`.
struct Owner {
    id: String,
    object: bool,
}

/// A function to record: its keyword, name and the measured token range.
struct Span {
    keyword: usize,
    name: usize,
    /// The `{` or `=` after the header.
    head_end: usize,
    from: usize,
    to: usize,
    end_line: u32,
}

struct Scan<'a> {
    file: &'a str,
    graph_node: &'a str,
    source: &'a str,
    code: Code<'a>,
    package: String,
    out: KotlinStructure,
    seen: BTreeSet<String>,
}

impl Scan<'_> {
    /// Declarations outside function bodies; bodies are measured and skipped.
    fn walk(&mut self) {
        let mut owners = Vec::new();
        let mut i = 0;
        while i < self.code.toks.len() {
            i = self.declaration(i, &mut owners);
        }
    }

    /// Handle the token at `i` outside any function body; returns the next index.
    fn declaration(&mut self, i: usize, owners: &mut Vec<Owner>) -> usize {
        if self.code.toks[i].literal || self.code.after_member(i) {
            return i + 1;
        }
        match self.code.text(i) {
            "package" if self.code.is_name(i + 1) => {
                self.package = self.code.dotted(i + 1);
                i + 1
            }
            "fun" if self.code.text(i + 1) != "interface" => self.function(i, owners.last()),
            // Initializers, accessors, `init` blocks and enum entry bodies.
            "{" | "(" | "[" => self.code.pairs[&i] + 1,
            "}" => {
                owners.pop();
                i + 1
            }
            _ => match self.code.class_at(i) {
                Some((kind, name)) => self.class(i, kind, &name, owners),
                None => i + 1,
            },
        }
    }

    /// Record the class-like declaration at `keyword`; returns where to go on.
    fn class(&mut self, keyword: usize, kind: &str, name: &str, owners: &mut Vec<Owner>) -> usize {
        let header = keyword + 1 + usize::from(self.code.is_name(keyword + 1));
        let (open, end) = match self.code.body_start(header) {
            Start::Block(open) => (Some(open), open + 1),
            Start::Expr(end) | Start::None(end) => (None, end),
        };
        let modifiers = self.code.modifiers(keyword);
        let is_abstract =
            kind == "interface" || modifiers.contains("abstract") || modifiers.contains("sealed");
        let id = match owners.last() {
            Some(owner) => format!("{}.{name}", owner.id),
            None => format!("{}::{name}", self.file),
        };
        let class = CallClass {
            language: Some("kotlin".into()),
            id: id.clone(),
            name: name.into(),
            package: self.package.clone(),
            file: self.file.into(),
            graph_node: self.graph_node.into(),
            line: self.code.toks[keyword].line,
            kind: kind.into(),
            is_abstract: Some(is_abstract),
            visibility: None,
            supertypes: self.code.supertypes(header, open.unwrap_or(end)),
            supertype_sites: None,
            declaration_files: None,
        };
        self.klass(class);
        if open.is_some() {
            owners.push(Owner {
                id,
                object: kind == "object",
            });
        }
        end
    }

    fn klass(&mut self, class: CallClass) {
        if self.seen.insert(class.id.clone()) {
            self.out.classes.push(class);
        }
    }

    /// The pseudo-class of top-level functions.
    fn module(&mut self) -> String {
        let id = format!("{}::(module)", self.file);
        self.klass(CallClass {
            language: Some("kotlin".into()),
            id: id.clone(),
            name: "(module)".into(),
            package: self.package.clone(),
            file: self.file.into(),
            graph_node: self.graph_node.into(),
            line: 1,
            kind: "module".into(),
            is_abstract: Some(false),
            visibility: None,
            supertypes: Vec::new(),
            supertype_sites: None,
            declaration_files: None,
        });
        id
    }

    /// Record and measure the function declared at `keyword`; returns the
    /// index after its body, or after its header when it has none.
    fn function(&mut self, keyword: usize, owner: Option<&Owner>) -> usize {
        let Some(open) = self.code.params(keyword) else {
            return keyword + 1;
        };
        let (span, next) = match self.code.body_start(self.code.pairs[&open] + 1) {
            Start::Block(brace) => {
                let close = self.code.pairs[&brace];
                let line = self.code.toks[close].line;
                (
                    span(keyword, open, brace, brace + 1, close, line),
                    close + 1,
                )
            }
            Start::Expr(eq) => {
                let end = self.code.expression_end(eq + 1);
                let line = self.code.toks[end - 1].end_line;
                (span(keyword, open, eq, eq + 1, end, line), end)
            }
            Start::None(end) => return end,
        };
        let owner = owner.map(|o| (o.id.clone(), o.object));
        self.record(&span, owner);
        next
    }

    fn record(&mut self, span: &Span, owner: Option<(String, bool)>) {
        let toks = self.code.toks;
        let is_static = owner.as_ref().is_none_or(|(_, object)| *object);
        let class_id = owner.map_or_else(|| self.module(), |(id, _)| id);
        let name = toks[span.name].text.trim_matches('`').to_string();
        let line = toks[span.keyword].line;
        let id = format!("{class_id}#{name}@{line}");
        let signature = self.source[toks[span.keyword].start..toks[span.head_end - 1].end]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let measure = Measure::of(&self.code, span.from, span.to);
        self.out.methods.push(CallMethod {
            id: id.clone(),
            class_id,
            name,
            signature,
            file: self.file.into(),
            line,
            end_line: Some(span.end_line),
            is_static,
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
    }
}

/// A function span; the name is the token before the parameter list.
fn span(
    keyword: usize,
    open: usize,
    head_end: usize,
    from: usize,
    to: usize,
    end_line: u32,
) -> Span {
    Span {
        keyword,
        name: open - 1,
        head_end,
        from,
        to,
        end_line,
    }
}

/// How a brace inside a measured body affects nesting.
#[derive(Clone, Copy, PartialEq)]
enum Block {
    Plain,
    Nest,
    Do,
    When,
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
    /// Measure the body tokens `from..to`.
    fn of(code: &Code, from: usize, to: usize) -> Self {
        let mut marks = Marks::default();
        for i in from..to {
            marks.mark(code, i);
        }
        Walk {
            code,
            from,
            marks,
            m: Measure {
                cyclomatic: 1,
                ..Default::default()
            },
            stack: Vec::new(),
            level: 0,
            logic: None,
            closed_do: None,
        }
        .run(to)
    }

    /// A decision that counts once and adds cognitive load: with nesting
    /// (`nested`) for a new construct, flat for `else if`.
    fn decision(&mut self, nested: bool) {
        self.cyclomatic += 1;
        self.cognitive += if nested { 1 + self.depth } else { 1 };
        self.max_nesting = self.max_nesting.max(self.depth + 1);
    }

    /// `when`: cognitive load with nesting, but no decision of its own (its
    /// branches count).
    fn branch(&mut self) {
        self.cognitive += 1 + self.depth;
        self.max_nesting = self.max_nesting.max(self.depth + 1);
    }

    fn nest(&mut self) {
        self.depth += 1;
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
        let operand = token.literal
            || token.text.starts_with(|c: char| c.is_ascii_digit())
            || (token.is_name() && !KEYWORDS.contains(&token.text.as_str()));
        if operand {
            self.operands.insert(token.text.clone());
            self.operand_count += 1;
        } else {
            self.operators.insert(token.text.clone());
            self.operator_count += 1;
        }
        self.lines.insert(token.line);
    }
}

/// What a pre-pass over a body finds: braces that do not nest like a
/// lambda, nested scopes to skip (start index to the index after them) and
/// the `->` of every counted `when` branch.
#[derive(Default)]
struct Marks {
    blocks: BTreeMap<usize, Block>,
    skip: BTreeMap<usize, usize>,
    arms: BTreeSet<usize>,
}

impl Marks {
    fn mark(&mut self, code: &Code, i: usize) {
        if code.after_member(i) {
            return;
        }
        let braced = code.text(i + 1) == "{";
        match code.text(i) {
            "do" if braced => {
                self.blocks.insert(i + 1, Block::Do);
            }
            "try" | "finally" if braced => {
                self.blocks.insert(i + 1, Block::Plain);
            }
            "when" => self.when(code, i),
            "object" => self.skip_body(code, i, i + 1),
            "class" if code.is_name(i + 1) => self.skip_body(code, i, i + 2),
            _ => {}
        }
    }

    /// Mark a `when` body and the `->` of its branches at its own level.
    fn when(&mut self, code: &Code, i: usize) {
        let open = if code.text(i + 1) == "(" {
            code.pairs[&(i + 1)] + 1
        } else {
            i + 1
        };
        if code.text(open) != "{" {
            return;
        }
        self.blocks.insert(open, Block::When);
        let mut k = open + 1;
        while k < code.pairs[&open] {
            match code.text(k) {
                "->" => {
                    if code.text(k - 1) != "else" {
                        self.arms.insert(k);
                    }
                    if code.text(k + 1) == "{" {
                        self.blocks.insert(k + 1, Block::Plain);
                    }
                }
                "(" | "[" | "{" => k = code.pairs[&k],
                _ => {}
            }
            k += 1;
        }
    }

    /// Skip from `i` past the braced body of the declaration whose header
    /// continues at `at`.
    fn skip_body(&mut self, code: &Code, i: usize, at: usize) {
        if let Start::Block(open) = code.body_start(at) {
            self.skip.insert(i, code.pairs[&open] + 1);
        }
    }
}

/// The measuring pass over a body, token by token.
struct Walk<'a> {
    code: &'a Code<'a>,
    from: usize,
    marks: Marks,
    m: Measure,
    stack: Vec<Block>,
    level: usize,
    /// The current run of logical operators: its level and whether it is `&&`.
    logic: Option<(usize, bool)>,
    closed_do: Option<usize>,
}

impl Walk<'_> {
    fn run(mut self, to: usize) -> Measure {
        let mut i = self.from;
        while i < to {
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
        let token = &self.code.toks[i];
        let word = self.code.text(i);
        let logical = |w: &str| matches!(w, "&&" | "||");
        if i > self.from
            && self.code.new_line(i)
            && !logical(word)
            && !logical(self.code.text(i - 1))
        {
            self.logic = None;
        }
        self.m.halstead(token);
        if !self.bracket(i, word) && !self.keyword(i, word) {
            self.operator(i, word);
        }
    }

    /// Brackets and separators; false for any other word.
    fn bracket(&mut self, i: usize, word: &str) -> bool {
        match word {
            "(" | "[" => self.level += 1,
            ")" | "]" => {
                self.level = self.level.saturating_sub(1);
                self.logic = self.logic.filter(|(lv, _)| *lv <= self.level);
            }
            "{" => {
                // Unmarked braces are lambdas, function and control bodies.
                let block = self.marks.blocks.get(&i).copied().unwrap_or(Block::Nest);
                if block != Block::Plain {
                    self.m.nest();
                }
                self.stack.push(block);
                self.logic = None;
            }
            "}" => {
                let block = self.stack.pop().unwrap_or(Block::Plain);
                if block != Block::Plain {
                    self.m.depth -= 1;
                }
                self.closed_do = (block == Block::Do).then_some(i);
                self.logic = None;
            }
            ";" | "," => self.logic = None,
            _ => return false,
        }
        true
    }

    /// Control keywords; false for any other word.
    fn keyword(&mut self, i: usize, word: &str) -> bool {
        let code = self.code;
        match word {
            "if" => self
                .m
                .decision(!(i > self.from && code.text(i - 1) == "else")),
            "else" if !matches!(code.text(i + 1), "if" | "->") => self.m.cognitive += 1,
            "while" if self.closed_do == Some(i.wrapping_sub(1)) => {}
            "for" | "while" | "do" | "catch" => self.m.decision(true),
            "when" => self.m.branch(),
            "break" | "continue" if self.labeled(i) => self.m.cognitive += 1,
            _ => return false,
        }
        true
    }

    /// `break@label` / `continue@label`.
    fn labeled(&self, i: usize) -> bool {
        let toks = self.code.toks;
        self.code.text(i + 1) == "@" && toks[i].end == toks[i + 1].start
    }

    /// Elvis, logical operators and `when` branches.
    fn operator(&mut self, i: usize, word: &str) {
        match word {
            "?:" => self.m.cyclomatic += 1,
            "&&" => self.logical(true),
            "||" => self.logical(false),
            "->" if self.marks.arms.contains(&i) => self.m.cyclomatic += 1,
            _ => {}
        }
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

    fn scan(source: &str) -> KotlinStructure {
        scan_kotlin_structure("src/Shop.kt", "com/shop/Shop.kt", source)
    }
    fn method<'a>(found: &'a KotlinStructure, name: &str) -> &'a CallMethod {
        found
            .methods
            .iter()
            .find(|m| m.name == name)
            .unwrap_or_else(|| panic!("no function {name}: {:?}", found.diagnostics))
    }
    fn metrics<'a>(found: &'a KotlinStructure, name: &str) -> &'a SourceMethodMetrics {
        let id = &method(found, name).id;
        found.metrics.iter().find(|m| &m.method_id == id).unwrap()
    }
    /// (cyclomatic, max nesting, cognitive) of one function.
    fn complexity(found: &KotlinStructure, name: &str) -> (u32, u32, u32) {
        let measured = method(found, name).complexity.as_ref().unwrap();
        let cognitive = metrics(found, name).cognitive;
        (measured.cyclomatic, measured.max_nesting, cognitive)
    }

    #[test]
    fn counts_follow_the_shared_rules() {
        let found = scan(
            r#"package com.shop

fun plain() = 1
fun block(): Int { return 1 }
fun branches(a: Boolean, b: Boolean): Int {
    if (a) {
        if (b) { return 1 } else { return 2 }
    } else if (b) {
        return 3
    } else {
        return 4
    }
}
fun logic(a: Boolean, b: Boolean, c: Boolean) = a && b || c
fun statements(a: Boolean, b: Boolean): Boolean {
    val x = a && b
    val y = a &&
        b
    return x && y
}
fun arms(x: Int): String = when (x) {
    0, 1 -> "low"
    2 -> { "mid" }
    else -> "high"
}
fun subjectless(a: Int) {
    when {
        a > 0 && a < 10 -> println("small")
        a < 0 -> { if (a < -10) println("very") }
        else -> println("zero")
    }
}
fun guarded(items: List<Int?>): Int {
    try {
        for (item in items) {
            while (item != null) { break }
        }
    } catch (e: IllegalStateException) {
        return 0
    } catch (e: Exception) {
        return 1
    } finally {
        println("done")
    }
    return 2
}
fun nulls(a: String?): Int = a?.length ?: 0
fun lambdas(xs: List<Int>): List<Int> {
    val f = { y: Int -> if (y > 0) y else 0 }
    return xs.map { x -> f(x) }.filter(fun(v: Int): Boolean { return v > 1 && v < 9 })
}
fun looped(start: Int): Int { var n = start; do { n-- } while (n > 0); return n }
fun braceless(a: Boolean): Int { if (a) return 1; return 2 }
fun labeled(rows: List<List<Int>>) {
    outer@ for (row in rows) {
        for (cell in row) {
            if (cell < 0) break@outer
            if (cell == 0) continue
        }
    }
}
fun local(a: Int): Int {
    fun helper(b: Int): Int { if (b > 0) { return b }; return 0 }
    val o = object : Runnable { override fun run() { if (a > 0) println(a) } }
    o.run()
    return helper(a)
}
"#,
        );
        assert_eq!(complexity(&found, "plain"), (1, 0, 0));
        assert_eq!(complexity(&found, "block"), (1, 0, 0));
        // Same shape and numbers as the Rust, Python and PHP `branches` cases.
        assert_eq!(complexity(&found, "branches"), (4, 2, 6));
        assert_eq!(complexity(&found, "logic"), (3, 0, 2));
        // A line break ends a run of logical operators unless an operator
        // continues it: three runs of `&&`, three cognitive points.
        assert_eq!(complexity(&found, "statements"), (4, 0, 3));
        // Each `when` branch except `else`; `0, 1 ->` is one branch and a
        // braced branch body does not nest.
        assert_eq!(complexity(&found, "arms"), (3, 1, 1));
        assert_eq!(complexity(&found, "subjectless"), (5, 2, 4));
        // `try` and `finally` neither count nor nest, as in Python and PHP.
        assert_eq!(complexity(&found, "guarded"), (5, 2, 5));
        // `?:` is a decision without cognitive load; `?.` is not counted.
        assert_eq!(complexity(&found, "nulls"), (2, 0, 0));
        // Lambdas and anonymous functions are one level deeper; `->` in a
        // lambda is not a branch.
        assert_eq!(complexity(&found, "lambdas"), (3, 2, 4));
        // `do … while` is one loop, not two.
        assert_eq!(complexity(&found, "looped"), (2, 1, 1));
        assert_eq!(complexity(&found, "braceless"), (2, 1, 1));
        // A jump to a label adds cognitive load; a plain `continue` does not.
        assert_eq!(complexity(&found, "labeled"), (5, 3, 10));
        // A local function nests like a lambda and counts toward its
        // enclosing function; object expressions are their own scope.
        assert_eq!(complexity(&found, "local"), (2, 2, 2));
        assert!(
            found
                .methods
                .iter()
                .all(|m| m.name != "helper" && m.name != "run")
        );
        // `1`: one operand and no operator; `return 1`: one of each.
        let plain = metrics(&found, "plain");
        assert_eq!(
            (plain.operators, plain.operands, plain.source_lines),
            (0, 1, 1)
        );
        let block = metrics(&found, "block");
        assert_eq!(
            (block.operators, block.operands, block.source_lines),
            (1, 1, 1)
        );
        assert_eq!(method(&found, "arms").end_line, Some(25));
        assert!(found.diagnostics.is_empty(), "{:?}", found.diagnostics);
    }

    #[test]
    fn strings_templates_and_comments_hide_their_contents() {
        let found = scan(
            r#"fun templates(name: String, n: Int): String {
    val raw = """if (x) { "${'$'}" } && || when"""
    // if (a && b) { }
    /* when { /* nested */ else -> } */
    return "Hello ${if (n > 1) "all" else name} ${name.length} {"
}
fun path() = """C:\"""
fun after(a: Boolean) = if (a) 1 else 2
"#,
        );
        // Template expressions stay inside the string literal: one operand,
        // no decisions.
        assert_eq!(complexity(&found, "templates"), (1, 0, 0));
        let templates = metrics(&found, "templates");
        assert_eq!(
            (
                templates.operators,
                templates.operands,
                templates.source_lines
            ),
            (3, 3, 2)
        );
        // A backslash in a raw string is no escape: `after` is still found.
        assert_eq!(complexity(&found, "path"), (1, 0, 0));
        assert_eq!(complexity(&found, "after"), (2, 1, 2));
        assert!(found.diagnostics.is_empty(), "{:?}", found.diagnostics);
    }

    #[test]
    fn classes_functions_and_abstractness_come_from_declarations() {
        let found = scan(
            r#"package com.shop

import com.shop.base.Base

interface Port { fun send(msg: String) }

sealed class Shape {
    abstract fun area(): Double
    data class Circle(val r: Double) : Shape() {
        override fun area() = 3.14 * r * r
    }
    object Empty : Shape() { override fun area() = 0.0 }
}

abstract class Repo<T : Any>(val name: String) : Base(name), Port, com.x.Countable<T> {
    override fun send(msg: String) { println(msg) }
    companion object {
        fun create(): Repo<Int>? = null
    }
    inner class Cursor { fun next(): Int = 1 }
    val size: Int get() = 0
    init { val f = fun() = 1 }
}

enum class Suit { HEARTS { override fun toString() = "h" }, SPADES; fun short() = name.take(1) }

fun interface Action { fun run() }

annotation class Marker

data class Point(val x: Int, val y: Int)
fun String.shout(): String = uppercase() + "!"

private fun <T> List<T>.second(): T = this[1]

object Registry {
    fun register(name: String) {
        val anon = object : Runnable { override fun run() { if (name.isEmpty()) return } }
        anon.run()
    }
}
fun `backticked name`() = Repo::class
"#,
        );
        let class = |name: &str| {
            found
                .classes
                .iter()
                .find(|c| c.name == name)
                .unwrap_or_else(|| panic!("no class {name}: {:?}", found.classes))
        };
        let kinds: Vec<_> = [
            "Port",
            "Shape",
            "Circle",
            "Empty",
            "Repo",
            "Companion",
            "Cursor",
            "Suit",
            "Action",
            "Marker",
            "Point",
            "Registry",
            "(module)",
        ]
        .iter()
        .map(|name| (class(name).kind.as_str(), class(name).is_abstract))
        .collect();
        assert_eq!(
            kinds,
            [
                ("interface", Some(true)),
                ("class", Some(true)),
                ("class", Some(false)),
                ("object", Some(false)),
                ("class", Some(true)),
                ("object", Some(false)),
                ("class", Some(false)),
                ("enum", Some(false)),
                ("interface", Some(true)),
                ("annotation", Some(false)),
                ("class", Some(false)),
                ("object", Some(false)),
                ("module", Some(false)),
            ]
        );
        assert_eq!(found.classes.len(), 13, "{:?}", found.classes);
        let repo = class("Repo");
        assert_eq!(repo.supertypes, ["Base", "Port", "com.x.Countable"]);
        assert_eq!(repo.package, "com.shop");
        assert_eq!(repo.id, "src/Shop.kt::Repo");
        assert_eq!(repo.file, "src/Shop.kt");
        assert_eq!(repo.graph_node, "com/shop/Shop.kt");
        assert_eq!(repo.language.as_deref(), Some("kotlin"));
        assert_eq!(repo.line, 15);
        assert_eq!(class("Circle").id, "src/Shop.kt::Shape.Circle");
        assert_eq!(class("Circle").supertypes, ["Shape"]);
        assert_eq!(class("Companion").id, "src/Shop.kt::Repo.Companion");
        // Bodiless interface and abstract functions have nothing to measure.
        assert!(
            found
                .methods
                .iter()
                .all(|m| m.name != "send" || m.class_id == repo.id)
        );
        assert_eq!(found.methods.iter().filter(|m| m.name == "area").count(), 2);
        let send = method(&found, "send");
        assert_eq!(send.signature, "fun send(msg: String)");
        assert!(!send.is_static);
        // `object` and `companion object` members are Kotlin's statics.
        let create = method(&found, "create");
        assert_eq!(create.class_id, class("Companion").id);
        assert!(create.is_static);
        assert_eq!(create.signature, "fun create(): Repo<Int>?");
        assert_eq!(method(&found, "next").class_id, class("Cursor").id);
        assert_eq!(method(&found, "short").class_id, class("Suit").id);
        let register = method(&found, "register");
        assert!(register.is_static);
        assert_eq!(complexity(&found, "register"), (1, 0, 0));
        // Extension and top-level functions belong to the file module.
        let shout = method(&found, "shout");
        assert_eq!(shout.class_id, "src/Shop.kt::(module)");
        assert_eq!(shout.signature, "fun String.shout(): String");
        assert!(shout.is_static);
        assert_eq!(
            method(&found, "second").signature,
            "fun <T> List<T>.second(): T"
        );
        assert_eq!(method(&found, "backticked name").end_line, Some(42));
        // Accessors, `init` blocks, enum entry bodies and object expressions
        // are not recorded as functions.
        let names: Vec<_> = found.methods.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "area",
                "area",
                "send",
                "create",
                "next",
                "short",
                "shout",
                "second",
                "register",
                "backticked name",
            ]
        );
    }

    #[test]
    fn unbalanced_sources_are_reported_not_measured() {
        let found = scan("fun broken() { if (\n");
        assert!(found.methods.is_empty());
        assert_eq!(
            found.diagnostics,
            ["src/Shop.kt: unbalanced brackets; not measured"]
        );
    }
}
