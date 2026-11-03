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
        let mut blocks: BTreeMap<usize, Block> = BTreeMap::new();
        let mut skip: BTreeMap<usize, usize> = BTreeMap::new();
        let mut arrows: BTreeSet<usize> = BTreeSet::new();
        let is = |i: usize, word: &str| toks.get(i).is_some_and(|t| t.is(word));
        let text = |i: usize| {
            toks.get(i)
                .filter(|t| !t.literal)
                .map_or("", |t| t.text.as_str())
        };
        let brace_after = |i: usize| (i..to).find(|&k| text(k) == "{" || text(k) == ";");
        for i in from..to {
            let conditioned = [
                "if", "elseif", "for", "foreach", "while", "switch", "catch", "match",
            ];
            if conditioned.iter().any(|w| is(i, w)) && text(i + 1) == "(" {
                let close = pairs[&(i + 1)];
                if text(close + 1) == "{" {
                    let kind = if is(i, "match") {
                        Block::Match
                    } else {
                        Block::Nest
                    };
                    blocks.insert(close + 1, kind);
                }
            } else if (is(i, "else") || is(i, "do")) && text(i + 1) == "{" {
                blocks.insert(i + 1, if is(i, "do") { Block::Do } else { Block::Nest });
            } else if is(i, "function") {
                let paren = if text(i + 1) == "&" { i + 2 } else { i + 1 };
                if text(paren) == "(" {
                    // Closure: optional `use (…)` and return type before the body.
                    let mut at = pairs[&paren] + 1;
                    if is(at, "use") && text(at + 1) == "(" {
                        at = pairs[&(at + 1)] + 1;
                    }
                    if let Some(open) = brace_after(at).filter(|&k| text(k) == "{") {
                        blocks.insert(open, Block::Nest);
                    }
                } else if text(paren + 1) == "(" {
                    // Named function declared inside a body: its own scope.
                    let end = brace_after(pairs[&(paren + 1)] + 1)
                        .map_or(to, |k| if text(k) == "{" { pairs[&k] + 1 } else { k + 1 });
                    skip.insert(i, end);
                }
            } else if is(i, "fn") && text(i + 1) == "(" {
                if let Some(arrow) = (pairs[&(i + 1)] + 1..to).find(|&k| text(k) == "=>") {
                    arrows.insert(arrow);
                }
            } else if is(i, "new") && is(i + 1, "class") {
                let mut at = i + 2;
                if text(at) == "(" {
                    at = pairs[&at] + 1;
                }
                if let Some(open) = (at..to).find(|&k| text(k) == "{") {
                    skip.insert(i, pairs[&open] + 1);
                }
            } else if class_kind(&toks[i]).is_some()
                && toks.get(i + 1).is_some_and(Tok::is_name)
                && !matches!(text(i.wrapping_sub(1)), "::" | "->" | "?->")
                && let Some(open) = (i..to).find(|&k| text(k) == "{")
            {
                skip.insert(i, pairs[&open] + 1);
            }
        }

        let mut m = Measure {
            cyclomatic: 1,
            ..Default::default()
        };
        let mut stack: Vec<Block> = Vec::new();
        let mut arms: Vec<Arm> = Vec::new();
        let mut open_arrows: Vec<(usize, usize)> = Vec::new();
        let mut level = 0usize;
        let mut logic: Option<(usize, bool)> = None;
        let mut closed_do = None;
        let mut i = from;
        while i < to {
            if let Some(&end) = skip.get(&i) {
                i = end;
                continue;
            }
            let token = &toks[i];
            let word = if token.literal {
                ""
            } else {
                token.text.as_str()
            };
            while let Some(&(lv, depth)) = open_arrows.last() {
                let ends = (matches!(word, ";" | ",") && level == lv && stack.len() == depth)
                    || (matches!(word, ")" | "]") && level == lv)
                    || (word == "}" && stack.len() == depth);
                if !ends {
                    break;
                }
                open_arrows.pop();
                m.depth -= 1;
            }
            if let Some(arm) = arms.last_mut()
                && arm.level == level
                && stack.last() == Some(&Block::Match)
                && arm.expect
                && !matches!(word, "{" | "}")
            {
                arm.default = token.is("default");
                arm.expect = false;
            }
            m.halstead(token);
            let lower = word.to_ascii_lowercase();
            match lower.as_str() {
                "(" | "[" | "#[" => level += 1,
                ")" | "]" => {
                    level = level.saturating_sub(1);
                    logic = logic.filter(|(lv, _)| *lv <= level);
                }
                "{" => {
                    let block = blocks.get(&i).copied().unwrap_or(Block::Plain);
                    if block != Block::Plain {
                        m.nest(1);
                    }
                    if block == Block::Match {
                        arms.push(Arm {
                            level,
                            expect: true,
                            default: false,
                        });
                    }
                    stack.push(block);
                    logic = None;
                }
                "}" => {
                    let block = stack.pop().unwrap_or(Block::Plain);
                    if block != Block::Plain {
                        m.depth -= 1;
                    }
                    if block == Block::Match {
                        arms.pop();
                    }
                    closed_do = (block == Block::Do).then_some(i);
                    logic = None;
                }
                ";" => logic = None,
                "," => {
                    logic = None;
                    if let Some(arm) = arms.last_mut()
                        && arm.level == level
                        && stack.last() == Some(&Block::Match)
                    {
                        arm.expect = true;
                    }
                }
                "if" => {
                    let ladder = i > from && toks[i - 1].is("else");
                    m.decision(!ladder);
                }
                "elseif" => m.decision(false),
                "else" if !is(i + 1, "if") => m.cognitive += 1,
                "while" if closed_do == Some(i.wrapping_sub(1)) => {}
                "for" | "foreach" | "while" | "do" | "catch" => m.decision(true),
                "switch" | "match" if text(i + 1) == "(" => {
                    m.cognitive += 1 + m.depth;
                    m.max_nesting = m.max_nesting.max(m.depth + 1);
                }
                "case" => m.cyclomatic += 1,
                "?" if !matches!(text(i.wrapping_sub(1)), "(" | "," | ":" | "|") => {
                    m.decision(true);
                    logic = None;
                }
                "?:" => {
                    m.decision(true);
                    logic = None;
                }
                "??" | "??=" | "?->" => m.cyclomatic += 1,
                "&&" | "||" | "and" | "or" => {
                    m.cyclomatic += 1;
                    let and = matches!(lower.as_str(), "&&" | "and");
                    m.cognitive += match logic {
                        Some((lv, previous)) if lv == level => u32::from(previous != and),
                        _ => 1,
                    };
                    logic = Some((level, and));
                }
                "break" | "continue"
                    if toks
                        .get(i + 1)
                        .is_some_and(|t| t.text.parse::<u32>().is_ok_and(|n| n > 1)) =>
                {
                    m.cognitive += 1;
                }
                "=>" if arrows.contains(&i) => {
                    m.nest(1);
                    open_arrows.push((level, stack.len()));
                }
                "=>" => {
                    if let Some(arm) = arms.last()
                        && arm.level == level
                        && stack.last() == Some(&Block::Match)
                        && !arm.default
                    {
                        m.cyclomatic += 1;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        m
    }

    /// A decision that counts once and adds cognitive load: with nesting
    /// (`nested`) for a new construct, flat for `elseif` / `else if`.
    fn decision(&mut self, nested: bool) {
        self.cyclomatic += 1;
        self.cognitive += if nested { 1 + self.depth } else { 1 };
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
