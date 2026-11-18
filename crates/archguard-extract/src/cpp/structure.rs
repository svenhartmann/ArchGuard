//! Token-based C/C++ structure and measurements (ADR 0017): classes,
//! structs, unions, namespaces and functions with per-function complexity,
//! cognitive and Halstead counts. No compiler or preprocessor runs.
//!
//! Preprocessor directives are dropped line by line (with `\` continuations);
//! the code between `#if`/`#else`/`#endif` is measured as written, every
//! branch at once. When the branches leave brackets unbalanced, the file is
//! reported and not measured. Macro bodies are never measured, and macro
//! calls look like function calls.
//!
//! Rules mirror the other token scanners (`ast-decisions-v1`,
//! `source-maintainability/v1`):
//! - cyclomatic: `if`, `for`, `while`, `do`, `catch`, each `case`, `?`,
//!   `&&`/`and` and `||`/`or`; `switch` and `default` add nothing.
//! - cognitive: `if`, loops, `switch`, `catch` and `?` add 1 plus nesting;
//!   `else if` and `else` add 1; a run of logical operators adds 1 plus 1
//!   per change of operator; `goto` adds 1. `try` adds nothing.
//! - nesting follows braces after `)` (control bodies, lambdas with
//!   parameters), `else`, `do`, `]` and `mutable` (lambdas); plain blocks,
//!   `try` and initializer braces do not nest.
//! - Halstead: keywords and operators are operators; names, numbers and
//!   literals are operands; brackets, `;` and `,` are neither.
//!
//! Recorded functions have a body: free functions belong to the file's
//! `(module)` pseudo-class, functions defined in a class body to that class,
//! and out-of-class definitions `A::f` to the class id `<file>::<namespace>::A`
//! that the engine binds to the declaring class. A class is abstract when it
//! declares a pure virtual function (`= 0`).

use std::collections::{BTreeMap, BTreeSet};

use archguard_core::{CallClass, CallMethod, MethodComplexity, SourceMethodMetrics};

use crate::lex::{Flavor, Token, tokens};

/// Multi-character operators, longest first; the lexer emits single characters.
const OPERATORS: [&str; 26] = [
    "<=>", "<<=", ">>=", "->*", "...", "::", "->", "&&", "||", "==", "!=", "<=", ">=", "++", "--",
    "+=", "-=", "*=", "/=", "%=", "&=", "|=", "^=", "<<", ">>", ".*",
];
const KEYWORDS: [&str; 88] = [
    "alignas",
    "alignof",
    "and",
    "and_eq",
    "asm",
    "auto",
    "bitand",
    "bitor",
    "bool",
    "break",
    "case",
    "catch",
    "char",
    "char8_t",
    "char16_t",
    "char32_t",
    "class",
    "co_await",
    "co_return",
    "co_yield",
    "compl",
    "concept",
    "const",
    "consteval",
    "constexpr",
    "constinit",
    "const_cast",
    "continue",
    "decltype",
    "default",
    "delete",
    "do",
    "double",
    "dynamic_cast",
    "else",
    "enum",
    "explicit",
    "export",
    "extern",
    "float",
    "for",
    "friend",
    "goto",
    "if",
    "inline",
    "int",
    "long",
    "mutable",
    "namespace",
    "new",
    "noexcept",
    "not",
    "not_eq",
    "operator",
    "or",
    "or_eq",
    "private",
    "protected",
    "public",
    "register",
    "reinterpret_cast",
    "requires",
    "return",
    "short",
    "signed",
    "sizeof",
    "static",
    "static_assert",
    "static_cast",
    "struct",
    "switch",
    "template",
    "this",
    "thread_local",
    "throw",
    "try",
    "typedef",
    "typeid",
    "typename",
    "union",
    "unsigned",
    "using",
    "virtual",
    "void",
    "volatile",
    "wchar_t",
    "while",
    "xor",
];
/// Words allowed between a parameter list and the function body.
const TRAILERS: [&str; 8] = [
    "const", "volatile", "noexcept", "override", "final", "throw", "&", "&&",
];

/// Classes, functions and raw measurements of one C/C++ file.
#[derive(Debug, Default)]
pub struct CppStructure {
    pub classes: Vec<CallClass>,
    pub methods: Vec<CallMethod>,
    pub metrics: Vec<SourceMethodMetrics>,
    pub diagnostics: Vec<String>,
}

/// Scan one C/C++ source. `file` is the source path, `graph_node` the name
/// of its node in the dependency graph.
pub fn scan_cpp_structure(file: &str, graph_node: &str, source: &str) -> CppStructure {
    let toks = code_tokens(source);
    let mut out = CppStructure::default();
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
        out,
        scopes: Vec::new(),
        seen: BTreeMap::new(),
    };
    let mut i = 0;
    while i < toks.len() {
        i = scan.step(i);
    }
    scan.out
}

/// Lexer tokens without line breaks and preprocessor directives, with
/// adjacent operator characters merged.
fn code_tokens(source: &str) -> Vec<Token> {
    let mut code: Vec<Token> = Vec::new();
    let (mut line_start, mut directive, mut backslash) = (true, false, false);
    for token in tokens(source, Flavor::C) {
        if token.text == "\n" {
            // A trailing backslash continues the directive.
            directive &= backslash;
            line_start = true;
            backslash = false;
            continue;
        }
        directive |= line_start && !token.literal && token.text == "#";
        line_start = false;
        backslash = !token.literal && token.text == "\\";
        if !directive {
            code.push(token);
        }
    }
    merge_operators(code)
}

fn merge_operators(raw: Vec<Token>) -> Vec<Token> {
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
        out.push(Token {
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
fn pairs(toks: &[Token]) -> Option<BTreeMap<usize, usize>> {
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

struct Code<'a> {
    toks: &'a [Token],
    pairs: BTreeMap<usize, usize>,
}

impl<'a> Code<'a> {
    /// The code text at `i`; empty for literals and out of range.
    fn text(&self, i: usize) -> &'a str {
        self.toks
            .get(i)
            .filter(|t| !t.literal)
            .map_or("", |t| t.text.as_str())
    }
    /// A name that is no keyword.
    fn is_name(&self, i: usize) -> bool {
        is_identifier(self.text(i)) && !KEYWORDS.contains(&self.text(i))
    }
    fn close(&self, open: usize) -> usize {
        self.pairs[&open]
    }
    /// The name of a class header that starts at `k` and where the header
    /// goes on: the last name wins (`class EXPORT_API Name final`);
    /// attributes and `alignas(…)` are skipped.
    fn class_name(&self, mut k: usize) -> (Option<usize>, usize) {
        let mut name = None;
        loop {
            let word = self.text(k);
            if word == "[" && self.text(k + 1) == "[" {
                k = self.close(k) + 1;
            } else if word == "final" && name.is_some() {
                return (name, k + 1);
            } else if self.is_name(k) || word == "alignas" {
                name = (word != "alignas").then_some(k);
                k += 1;
                if self.text(k) == "(" {
                    name = None;
                    k = self.close(k) + 1;
                }
            } else {
                return (name, k);
            }
        }
    }
    /// The index after a qualifier between parameters and body at `k`:
    /// `const`, `noexcept(…)`, `__attribute__((…))`, `[[…]]`.
    fn trailer_end(&self, k: usize) -> Option<usize> {
        let word = self.text(k);
        let next = match word {
            "[" => k,
            _ if TRAILERS.contains(&word) || word.starts_with("__") => k + 1,
            _ => return None,
        };
        Some(match self.text(next) {
            "(" | "[" => self.close(next) + 1,
            _ => next,
        })
    }
    /// The index after a trailing return type or requires clause at `k`.
    fn clause_end(&self, mut k: usize) -> usize {
        while k < self.toks.len() && !matches!(self.text(k), "{" | ";" | "=") {
            if self.text(k) == "(" {
                k = self.close(k);
            }
            k += 1;
        }
        k
    }
}

fn is_identifier(word: &str) -> bool {
    word.starts_with(|c: char| c.is_alphabetic() || c == '_')
}

/// An open brace at declaration level.
enum Scope {
    /// A namespace and its qualified name (an anonymous one adds nothing).
    Namespace(String),
    /// A class body: index into the classes and its qualified name.
    Class(usize, String),
    /// `extern "C" { … }` and other transparent braces.
    Other,
}

/// What follows a function's parameter list.
enum Tail {
    /// A body between these braces.
    Body(usize, usize),
    /// `= 0;` — a pure virtual function; the scan goes on at the index.
    Pure(usize),
    /// A declaration or something else; the scan goes on at the index.
    Skip(usize),
}

struct Scan<'a> {
    file: &'a str,
    graph_node: &'a str,
    source: &'a str,
    code: Code<'a>,
    out: CppStructure,
    scopes: Vec<Scope>,
    /// Recorded class ids and their index in `out.classes`.
    seen: BTreeMap<String, usize>,
}

impl Scan<'_> {
    /// Handle the token at `i` outside any function body; returns the next index.
    fn step(&mut self, i: usize) -> usize {
        let code = &self.code;
        if code.toks[i].literal {
            return i + 1;
        }
        match code.text(i) {
            "namespace" => self.namespace(i),
            "enum" => self.skip_declaration(i),
            "class" | "struct" | "union" => self.class(i),
            "extern"
                if code.toks.get(i + 1).is_some_and(|t| t.literal) && code.text(i + 2) == "{" =>
            {
                self.scopes.push(Scope::Other);
                i + 3
            }
            // Initializers and anything else in braces.
            "{" => code.close(i) + 1,
            "}" => {
                self.scopes.pop();
                i + 1
            }
            "(" => self.function(i),
            _ => i + 1,
        }
    }

    /// `namespace a::b {` opens a scope; an alias or a malformed header does not.
    fn namespace(&mut self, i: usize) -> usize {
        let mut names = Vec::new();
        let mut k = i + 1;
        loop {
            match self.code.text(k) {
                "inline" | "::" => {}
                "{" => {
                    let path = join(&self.scope_path(), &names.join("::"));
                    self.scopes.push(Scope::Namespace(path));
                    return k + 1;
                }
                word if self.code.is_name(k) => names.push(word),
                _ => return i + 1,
            }
            k += 1;
        }
    }

    /// Skip an `enum` with its body.
    fn skip_declaration(&self, i: usize) -> usize {
        let mut k = i + 1;
        while k < self.code.toks.len() {
            match self.code.text(k) {
                "{" => return self.code.close(k) + 1,
                ";" | "}" => return k,
                "(" | "[" => k = self.code.close(k),
                _ => {}
            }
            k += 1;
        }
        k
    }

    /// The qualified name of the current scope, `ns::Outer`.
    fn scope_path(&self) -> String {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| match scope {
                Scope::Namespace(path) | Scope::Class(_, path) => Some(path.clone()),
                Scope::Other => None,
            })
            .unwrap_or_default()
    }

    /// A class, struct or union definition at keyword `i`; elaborated type
    /// names, forward declarations and template parameters are skipped.
    fn class(&mut self, i: usize) -> usize {
        let code = &self.code;
        let (name, k) = code.class_name(i + 1);
        let open = match code.text(k) {
            "{" => k,
            ":" => match (k..code.toks.len()).find(|&j| matches!(code.text(j), "{" | ";")) {
                Some(open) if code.text(open) == "{" => open,
                _ => return i + 1,
            },
            _ => return i + 1,
        };
        let Some(name) = name else {
            // An anonymous struct or union: nothing to name.
            return code.close(open) + 1;
        };
        let name = code.text(name).to_string();
        let qualified = join(&self.scope_path(), &name);
        let id = format!("{}::{qualified}", self.file);
        let class = CallClass {
            name,
            line: code.toks[i].line,
            kind: code.text(i).into(),
            supertypes: supertypes(code, k, open),
            ..self.class_row(&id)
        };
        let index = self.add_class(class);
        self.scopes.push(Scope::Class(index, qualified));
        open + 1
    }

    /// A concrete class row of this file; the caller fills in the rest.
    fn class_row(&self, id: &str) -> CallClass {
        CallClass {
            language: Some("cpp".into()),
            id: id.into(),
            name: String::new(),
            package: package(self.file),
            file: self.file.into(),
            graph_node: self.graph_node.into(),
            line: 1,
            kind: String::new(),
            is_abstract: Some(false),
            visibility: None,
            supertypes: Vec::new(),
            supertype_sites: None,
            declaration_files: None,
        }
    }

    /// Record a class once per id; returns its index.
    fn add_class(&mut self, class: CallClass) -> usize {
        if let Some(&index) = self.seen.get(&class.id) {
            return index;
        }
        self.seen.insert(class.id.clone(), self.out.classes.len());
        self.out.classes.push(class);
        self.out.classes.len() - 1
    }

    /// A parameter list at `open`: record and measure the function when a
    /// body follows; returns where the scan goes on.
    fn function(&mut self, open: usize) -> usize {
        let close = self.code.close(open);
        let Some((name, name_start)) = self.function_name(open) else {
            return close + 1;
        };
        let (body, next) = match self.tail(close + 1) {
            Tail::Body(from, to) => ((from, to), to + 1),
            Tail::Pure(next) => {
                if let Some(Scope::Class(index, _)) = self.scopes.last() {
                    self.out.classes[*index].is_abstract = Some(true);
                }
                return next;
            }
            Tail::Skip(next) => return next,
        };
        let (qualifier, qualified_start) = self.qualifier(name_start);
        let start = self.header_start(qualified_start);
        let (class_id, is_static) = match self.scopes.last() {
            _ if !qualifier.is_empty() => {
                let owner = join(&self.scope_path(), &qualifier.join("::"));
                (format!("{}::{owner}", self.file), false)
            }
            Some(Scope::Class(index, _)) => {
                let is_static = (start..name_start).any(|k| self.code.text(k) == "static");
                (self.out.classes[*index].id.clone(), is_static)
            }
            _ => (self.module(), true),
        };
        self.record(&name, &class_id, is_static, (start, body), close);
        next
    }

    /// The function name before the parameter list at `open` and the index
    /// where the name starts: `f`, `~A`, `operator==`, `operator()`.
    fn function_name(&self, open: usize) -> Option<(String, usize)> {
        let code = &self.code;
        let last = open.checked_sub(1)?;
        if let Some(keyword) = (open.saturating_sub(4)..open)
            .rev()
            .find(|&j| code.text(j) == "operator")
        {
            if keyword + 1 == open {
                return None;
            }
            let symbol: Vec<&str> = (keyword + 1..open)
                .map(|j| code.toks[j].text.as_str())
                .collect();
            let sep = if symbol.first().is_some_and(|s| is_identifier(s)) {
                " "
            } else {
                ""
            };
            return Some((format!("operator{sep}{}", symbol.join(" ")), keyword));
        }
        if !code.is_name(last) {
            return None;
        }
        let name = code.text(last);
        if last > 0 && code.text(last - 1) == "~" {
            return Some((format!("~{name}"), last - 1));
        }
        Some((name.to_string(), last))
    }

    /// `A::B::` before the name at `start`, outermost first, and where it
    /// starts; template arguments of a qualifier are dropped.
    fn qualifier(&self, start: usize) -> (Vec<String>, usize) {
        let code = &self.code;
        let mut out = Vec::new();
        let mut k = start;
        while k >= 2 && code.text(k - 1) == "::" {
            let mut name = k - 2;
            if code.text(name) == ">" {
                let mut depth = 0usize;
                while name > 0 {
                    match code.text(name) {
                        ">" => depth += 1,
                        "<" => depth -= 1,
                        _ => {}
                    }
                    if depth == 0 {
                        break;
                    }
                    name -= 1;
                }
                name = name.saturating_sub(1);
            }
            if !code.is_name(name) {
                break;
            }
            out.push(code.text(name).to_string());
            k = name;
        }
        out.reverse();
        (out, k)
    }

    /// The first token of the declaration whose name starts at `name`.
    fn header_start(&self, name: usize) -> usize {
        let code = &self.code;
        let mut k = name;
        while k > 0 {
            let before = code.text(k - 1);
            let access = before == ":"
                && k >= 2
                && matches!(code.text(k - 2), "public" | "private" | "protected");
            if matches!(before, ";" | "{" | "}") || access {
                break;
            }
            k -= 1;
        }
        k
    }

    /// What follows the parameter list that closes before `from`.
    fn tail(&self, from: usize) -> Tail {
        let code = &self.code;
        let mut k = from;
        loop {
            match code.text(k) {
                _ if let Some(next) = code.trailer_end(k) => k = next,
                // A trailing return type or a requires clause.
                "->" | "requires" => k = code.clause_end(k),
                "try" if code.text(k + 1) == "{" => k += 1,
                "{" => return Tail::Body(k, code.close(k)),
                ":" => return self.initializers(k + 1),
                "=" if code.text(k + 1) == "0" && code.text(k + 2) == ";" => {
                    return Tail::Pure(k + 3);
                }
                _ => return Tail::Skip(k),
            }
        }
    }

    /// A constructor's member initializers up to the body brace.
    fn initializers(&self, from: usize) -> Tail {
        let code = &self.code;
        let mut k = from;
        while k < code.toks.len() {
            match code.text(k) {
                "(" | "[" => k = code.close(k),
                // `a{1}` initializes a member; a brace after `)` or `}` is the body.
                "{" if k > from && matches!(code.text(k - 1), ")" | "}") => {
                    return Tail::Body(k, code.close(k));
                }
                "{" => k = code.close(k),
                ";" => return Tail::Skip(k),
                _ => {}
            }
            k += 1;
        }
        Tail::Skip(k)
    }

    /// The pseudo-class of free functions.
    fn module(&mut self) -> String {
        let id = format!("{}::(module)", self.file);
        let class = CallClass {
            name: "(module)".into(),
            kind: "module".into(),
            ..self.class_row(&id)
        };
        self.add_class(class);
        id
    }

    /// Record and measure a function with header from `start` and body braces `body`.
    fn record(
        &mut self,
        name: &str,
        class_id: &str,
        is_static: bool,
        (start, (open, close)): (usize, (usize, usize)),
        params_close: usize,
    ) {
        let toks = self.code.toks;
        let line = toks[start].line;
        let id = format!("{class_id}#{name}@{line}");
        let signature = self.source[toks[start].start..toks[params_close].end]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let measure = Measure::of(&self.code, open + 1, close);
        self.out.methods.push(CallMethod {
            id: id.clone(),
            class_id: class_id.into(),
            name: name.into(),
            signature,
            file: self.file.into(),
            line,
            end_line: Some(toks[close].line),
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

/// `a::b` with empty halves collapsed.
fn join(prefix: &str, name: &str) -> String {
    match (prefix.is_empty(), name.is_empty()) {
        (true, _) => name.to_string(),
        (_, true) => prefix.to_string(),
        _ => format!("{prefix}::{name}"),
    }
}

/// The package of a file: its directory.
fn package(file: &str) -> String {
    file.rsplit_once('/').map_or("", |(dir, _)| dir).to_string()
}

/// Base class names between the `:` at `from` and the body at `open`,
/// sorted; access specifiers and template arguments are dropped.
fn supertypes(code: &Code, from: usize, open: usize) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut angle = 0usize;
    for k in from + 1..open {
        let word = code.text(k);
        let last = out.last_mut().expect("never empty");
        match word {
            "<" => angle += 1,
            ">" | ">>" => angle = angle.saturating_sub(word.len()),
            "," if angle == 0 => out.push(String::new()),
            "::" if angle == 0 => last.push_str(word),
            _ if angle == 0 && code.is_name(k) && (last.is_empty() || last.ends_with("::")) => {
                last.push_str(word)
            }
            _ => {}
        }
    }
    out.retain(|name| !name.is_empty());
    out.sort();
    out
}

/// How a brace inside a measured body affects nesting.
#[derive(Clone, Copy, PartialEq)]
enum Block {
    Plain,
    Nest,
    Do,
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
        let mut walk = Walk {
            code,
            from,
            m: Measure {
                cyclomatic: 1,
                ..Default::default()
            },
            stack: Vec::new(),
            level: 0,
            logic: None,
            closed_do: None,
        };
        for i in from..to {
            walk.step(i);
        }
        walk.m
    }

    /// A decision that counts once and adds cognitive load: with nesting
    /// (`nested`) for a new construct, flat for `else if`.
    fn decision(&mut self, nested: bool) {
        self.cyclomatic += 1;
        self.cognitive += if nested { 1 + self.depth } else { 1 };
        self.max_nesting = self.max_nesting.max(self.depth + 1);
    }

    fn halstead(&mut self, token: &Token) {
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
            || (is_identifier(&token.text) && !KEYWORDS.contains(&token.text.as_str()));
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

/// The measuring pass over a body, token by token.
struct Walk<'a> {
    code: &'a Code<'a>,
    from: usize,
    m: Measure,
    stack: Vec<Block>,
    level: usize,
    /// The current run of logical operators: its level and whether it is `&&`.
    logic: Option<(usize, bool)>,
    closed_do: Option<usize>,
}

impl Walk<'_> {
    fn step(&mut self, i: usize) {
        let token = &self.code.toks[i];
        self.m.halstead(token);
        if token.literal {
            return;
        }
        let word = self.code.text(i);
        if !self.bracket(i, word) {
            self.keyword(i, word);
        }
    }

    fn previous(&self, i: usize) -> &str {
        if i > self.from {
            self.code.text(i - 1)
        } else {
            ""
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
                let block = match self.previous(i) {
                    "do" => Block::Do,
                    ")" | "else" | "]" | "mutable" => Block::Nest,
                    _ => Block::Plain,
                };
                if block != Block::Plain {
                    self.m.depth += 1;
                    self.m.max_nesting = self.m.max_nesting.max(self.m.depth);
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

    fn keyword(&mut self, i: usize, word: &str) {
        match word {
            "if" => self.m.decision(self.previous(i) != "else"),
            "else" if self.code.text(i + 1) != "if" => self.m.cognitive += 1,
            "while" if self.closed_do == Some(i.wrapping_sub(1)) => {}
            "for" | "while" | "do" | "catch" | "?" => self.m.decision(true),
            "switch" => {
                self.m.cognitive += 1 + self.m.depth;
                self.m.max_nesting = self.m.max_nesting.max(self.m.depth + 1);
            }
            "case" => self.m.cyclomatic += 1,
            "goto" => self.m.cognitive += 1,
            "&&" | "and" => self.logical(true),
            "||" | "or" => self.logical(false),
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

    fn scan(source: &str) -> CppStructure {
        scan_cpp_structure("src/shop/order.cpp", "src/shop/order.cpp", source)
    }
    fn method<'a>(found: &'a CppStructure, name: &str) -> &'a CallMethod {
        found
            .methods
            .iter()
            .find(|m| m.name == name)
            .unwrap_or_else(|| panic!("no function {name}: {:?}", found.methods))
    }
    fn metrics<'a>(found: &'a CppStructure, name: &str) -> &'a SourceMethodMetrics {
        let id = &method(found, name).id;
        found.metrics.iter().find(|m| &m.method_id == id).unwrap()
    }
    /// (cyclomatic, max nesting, cognitive) of one function.
    fn complexity(found: &CppStructure, name: &str) -> (u32, u32, u32) {
        let measured = method(found, name).complexity.as_ref().unwrap();
        (
            measured.cyclomatic,
            measured.max_nesting,
            metrics(found, name).cognitive,
        )
    }

    #[test]
    fn counts_follow_the_shared_rules() {
        let found = scan(
            r#"#include "order.h"
int plain() { return 1; }
int branches(bool a, bool b) {
    if (a) {
        if (b) { return 1; } else { return 2; }
    } else if (b) {
        return 3;
    } else {
        return 4;
    }
}
bool logic(bool a, bool b, bool c) { return a && b || c; }
int arms(int x) {
    switch (x) {
    case 0:
    case 1: return 0;
    case 2: { return 2; }
    default: return 3;
    }
}
int guarded(int* items, int n) {
    try {
        for (int i = 0; i < n; ++i) {
            while (items[i] != 0) { break; }
        }
    } catch (const std::exception& e) {
        return 0;
    } catch (...) {
        return 1;
    }
    return 2;
}
int ternary(int a) { return a > 0 ? a : -a; }
int lambdas(std::vector<int>& xs) {
    auto f = [](int y) { if (y > 0) { return y; } return 0; };
    std::vector<int> init{1, 2};
    { int scoped = 0; }
    return f(xs[0]) && init.size() or false;
}
int looped(int n) { do { n--; } while (n > 0); return n; }
int braceless(bool a) { if (a) return 1; return 2; }
void jumps(int n) {
    for (int i = 0; i < n; i++) {
        if (i == 3) goto done;
    }
done:
    return;
}
"#,
        );
        assert_eq!(complexity(&found, "plain"), (1, 0, 0));
        // Same shape and numbers as the Rust, Python, PHP and Kotlin cases.
        assert_eq!(complexity(&found, "branches"), (4, 2, 6));
        assert_eq!(complexity(&found, "logic"), (3, 0, 2));
        // Each `case` decides; `switch` nests, `default` adds nothing.
        assert_eq!(complexity(&found, "arms"), (4, 1, 1));
        // `try` neither counts nor nests; each `catch` does.
        assert_eq!(complexity(&found, "guarded"), (5, 2, 5));
        assert_eq!(complexity(&found, "ternary"), (2, 1, 1));
        // A lambda body nests; initializer braces and plain blocks do not;
        // `&&` then `or` is one run with one change of operator.
        assert_eq!(complexity(&found, "lambdas"), (4, 2, 4));
        // `do … while` is one loop, not two.
        assert_eq!(complexity(&found, "looped"), (2, 1, 1));
        assert_eq!(complexity(&found, "braceless"), (2, 1, 1));
        assert_eq!(complexity(&found, "jumps"), (3, 2, 4));
        // `return 1;`: one operator and one operand.
        let plain = metrics(&found, "plain");
        assert_eq!(
            (plain.operators, plain.operands, plain.source_lines),
            (1, 1, 1)
        );
        assert_eq!(method(&found, "branches").end_line, Some(11));
        assert!(found.diagnostics.is_empty(), "{:?}", found.diagnostics);
    }

    #[test]
    fn preprocessor_lines_are_dropped_and_branches_measured_as_written() {
        let found = scan(
            "#define CHECK(x) if (x) { \\\n  abort(); }\n\
             int checked(int a) {\n\
             #ifdef DEBUG\n\
                 if (a < 0) return -1;\n\
             #else\n\
                 if (a < 0) return 0;\n\
             #endif\n\
                 return a;\n\
             }\n\
             const char* s = \"{ if (\";\n",
        );
        // Both `#ifdef` branches count; the macro body never does.
        assert_eq!(complexity(&found, "checked"), (3, 1, 2));
        assert_eq!(found.methods.len(), 1);
        assert!(found.diagnostics.is_empty(), "{:?}", found.diagnostics);
    }

    #[test]
    fn classes_namespaces_and_methods_come_from_declarations() {
        let found = scan(
            r#"namespace shop {
namespace detail { struct Node { int v; }; }
class Shape {
public:
    virtual ~Shape() = default;
    virtual double area() const = 0;
    static Shape* make();
    int sides() const { return 0; }
};
class EXPORT Circle final : public Shape, private detail::Node, public Base<int, 2> {
public:
    Circle(double r) : r_(r), id_{1} { if (r < 0) r_ = 0; }
    double area() const override { return 3.14 * r_ * r_; }
    struct Iter { int next() { return 1; } };
private:
    double r_;
    int id_;
};
template <class T> T twice(T x) { return x + x; }
Shape* Shape::make() { return new Circle(1.0); }
bool operator==(const Circle& a, const Circle& b) { return a.area() == b.area(); }
}
namespace { int hidden() { return 0; } }
extern "C" { int c_api(void) { return 0; } }
struct Point { int x, y; };
typedef struct { int a; } Anonymous;
enum class Color { Red, Green };
struct Forward;
"#,
        );
        let class = |name: &str| {
            found
                .classes
                .iter()
                .find(|c| c.name == name)
                .unwrap_or_else(|| panic!("no class {name}: {:?}", found.classes))
        };
        let rows: Vec<_> = found
            .classes
            .iter()
            .map(|c| (c.id.as_str(), c.kind.as_str(), c.is_abstract))
            .collect();
        assert_eq!(
            rows,
            [
                (
                    "src/shop/order.cpp::shop::detail::Node",
                    "struct",
                    Some(false)
                ),
                ("src/shop/order.cpp::shop::Shape", "class", Some(true)),
                ("src/shop/order.cpp::shop::Circle", "class", Some(false)),
                (
                    "src/shop/order.cpp::shop::Circle::Iter",
                    "struct",
                    Some(false)
                ),
                ("src/shop/order.cpp::(module)", "module", Some(false)),
                ("src/shop/order.cpp::Point", "struct", Some(false)),
            ]
        );
        let circle = class("Circle");
        assert_eq!(circle.supertypes, ["Base", "Shape", "detail::Node"]);
        assert_eq!(circle.package, "src/shop");
        assert_eq!(circle.language.as_deref(), Some("cpp"));
        assert_eq!(circle.line, 10);
        let names: Vec<_> = found
            .methods
            .iter()
            .map(|m| {
                (
                    m.name.as_str(),
                    m.class_id.rsplit("::").next().unwrap(),
                    m.is_static,
                )
            })
            .collect();
        assert_eq!(
            names,
            [
                ("sides", "Shape", false),
                ("Circle", "Circle", false),
                ("area", "Circle", false),
                ("next", "Iter", false),
                ("twice", "(module)", true),
                // Out of class: the qualifier names the class the engine binds.
                ("make", "Shape", false),
                ("operator==", "(module)", true),
                ("hidden", "(module)", true),
                ("c_api", "(module)", true),
            ]
        );
        assert_eq!(
            method(&found, "make").class_id,
            "src/shop/order.cpp::shop::Shape"
        );
        assert_eq!(complexity(&found, "Circle"), (2, 1, 1));
        assert_eq!(
            method(&found, "twice").signature,
            "template <class T> T twice(T x)"
        );
        assert_eq!(method(&found, "area").signature, "double area()");
        assert!(found.diagnostics.is_empty(), "{:?}", found.diagnostics);
    }

    #[test]
    fn unbalanced_sources_are_reported_not_measured() {
        let found = scan("#if A\nvoid f() {\n#else\nvoid f(int) {\n#endif\n}\n");
        assert!(found.methods.is_empty());
        assert_eq!(
            found.diagnostics,
            ["src/shop/order.cpp: unbalanced brackets; not measured"]
        );
    }
}
