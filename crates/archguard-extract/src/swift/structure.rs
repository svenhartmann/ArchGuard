//! Token-based Swift structure and measurements (ADR 0018): types,
//! extensions and functions with per-function complexity, cognitive and
//! Halstead counts, plus the call sites a resolver can name. No Swift
//! compiler is needed; types stay unresolved.
//!
//! Rules mirror the Kotlin, Rust, Python and PHP analyzers
//! (`ast-decisions-v1`, `source-maintainability/v1`):
//! - cyclomatic: `if`, `guard`, `for`, `while`, `repeat`, `catch`, each
//!   `case` label of a `switch` (not `default`), `&&`, `||`, `??` and the
//!   ternary `?`. Optional chaining `?.`, `try?` and the commas of a
//!   condition list (`if let a, let b`) are not decisions.
//! - cognitive: `if`, `guard`, loops, `switch`, `catch` and the ternary add 1
//!   plus nesting; `else if` and `else` add 1, the `else` of a `guard` adds
//!   nothing; a run of logical operators adds 1 plus 1 per change of
//!   operator; `break label` and `continue label` add 1. `do`, `defer` and
//!   `??` add nothing.
//! - nesting follows braced control bodies, `switch` bodies, closures and
//!   local functions; `do` and `defer` blocks do not nest.
//!   Local functions count toward their enclosing function; local types are
//!   not measured.
//! - Halstead: keywords and operators are operators; names and literals are
//!   operands, a string with its interpolations is one operand; brackets,
//!   `;` and `,` are neither.
//!
//! Recorded functions are `func`, `init`, `deinit` and `subscript` with a
//! body; protocol requirements, computed properties, accessors, property
//! observers and top-level statements are not recorded.

use std::collections::{BTreeMap, BTreeSet};

use archguard_core::{CallClass, CallMethod, MethodComplexity, SourceMethodMetrics};

use super::tokens::{Code, KEYWORDS, Tok, merged, pairs};

/// Longest `code` text of a call site.
const CODE_LIMIT: usize = 200;

/// Types, functions, measurements and call sites of one Swift file.
#[derive(Debug, Default)]
pub struct SwiftStructure {
    pub classes: Vec<CallClass>,
    pub methods: Vec<CallMethod>,
    pub metrics: Vec<SourceMethodMetrics>,
    pub diagnostics: Vec<String>,
    pub sites: Vec<SwiftCallSite>,
}

/// One call in a measured function body, with names as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwiftCallSite {
    pub from_class: String,
    pub from_method: String,
    pub target: SwiftTarget,
    pub label: String,
    pub line: u32,
    pub column: u32,
    pub code: String,
}

/// What the source says a call targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwiftTarget {
    /// `Type(…)` or `Type.init(…)`.
    Init(String),
    /// `Type.name(…)`: a static method, or a nested type when `name` is one.
    Static { owner: String, name: String },
    /// Calls on values, `self`, `super`, call results, optional chains and
    /// bare lowercase names (a function, a method of `self` or a closure).
    Unresolved,
}

/// Scan one Swift source. `file` is the source path, `graph_node` the name
/// of its node in the dependency graph, `package` the module (target)
/// path the file belongs to.
pub fn scan_swift_structure(
    file: &str,
    graph_node: &str,
    package: &str,
    source: &str,
) -> SwiftStructure {
    let toks = merged(source);
    let mut out = SwiftStructure::default();
    let Some(pairs) = pairs(&toks) else {
        out.diagnostics
            .push(format!("{file}: unbalanced brackets; not measured"));
        return out;
    };
    let mut scan = Scan {
        file,
        graph_node,
        package,
        source,
        code: Code { toks: &toks, pairs },
        out,
    };
    let mut owners = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        i = scan.declaration(i, &mut owners);
    }
    scan.out
}

/// Attach the methods and conformances of every `extension T` to the one
/// project type `T` of that name (as written, `Outer.Inner` included),
/// across all scanned files. An extension of a type declared nowhere or
/// several times in the project stays a class of kind `extension`.
pub fn attach_extensions(classes: &mut Vec<CallClass>, methods: &mut [CallMethod]) {
    let path = |id: &str| id.split_once("::").map_or("", |(_, p)| p).to_string();
    let mut types: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, class) in classes.iter().enumerate() {
        if !matches!(class.kind.as_str(), "extension" | "module") {
            types.entry(path(&class.id)).or_default().push(index);
        }
    }
    let mut owner_of = BTreeMap::new();
    for class in classes.iter().filter(|c| c.kind == "extension") {
        if let Some([index]) = types.get(&class.name).map(Vec::as_slice) {
            owner_of.insert(class.id.clone(), (*index, class.supertypes.clone()));
        }
    }
    for (index, supertypes) in owner_of.values() {
        let target = &mut classes[*index].supertypes;
        target.extend(supertypes.iter().cloned());
        target.sort();
        target.dedup();
    }
    let ids: BTreeMap<_, _> = owner_of
        .iter()
        .map(|(ext, (index, _))| (ext.clone(), classes[*index].id.clone()))
        .collect();
    for method in methods.iter_mut() {
        if let Some(id) = ids.get(&method.class_id) {
            method.class_id = id.clone();
        }
    }
    classes.retain(|class| !ids.contains_key(&class.id));
}

/// An open type or extension body.
struct Owner {
    /// The `CallClass` id methods belong to.
    id: String,
    /// The type path nested declarations hang under.
    path: String,
}

/// What a function-like keyword declares.
#[derive(Clone, Copy, PartialEq)]
enum Fun {
    Func,
    Init,
    Subscript,
    Deinit,
}

struct Scan<'a> {
    file: &'a str,
    graph_node: &'a str,
    package: &'a str,
    source: &'a str,
    code: Code<'a>,
    out: SwiftStructure,
}

impl Scan<'_> {
    /// Handle the token at `i` outside any function body; returns the next index.
    fn declaration(&mut self, i: usize, owners: &mut Vec<Owner>) -> usize {
        let code = &self.code;
        if code.toks[i].literal || code.after_dot(i) || (i > 0 && code.text(i - 1) == "#") {
            return i + 1;
        }
        if let Some(fun) = fun_at(code, i) {
            return self.function(i, fun, owners.last());
        }
        match code.text(i) {
            "import" => {
                let line = code.toks[i].line;
                (i + 1..code.toks.len())
                    .find(|&j| code.toks[j].line > line)
                    .unwrap_or(code.toks.len())
            }
            // Property initializers, accessors, observers and top-level statements.
            "{" | "(" | "[" => code.pairs[&i] + 1,
            "}" => {
                owners.pop();
                i + 1
            }
            "class" | "struct" | "enum" | "protocol" | "actor" if code.is_identifier(i + 1) => {
                self.class(i, owners)
            }
            "extension" if code.is_name(i + 1) => self.extension(i, owners),
            _ => i + 1,
        }
    }

    /// Record the type declared at `keyword`; returns where to go on.
    fn class(&mut self, keyword: usize, owners: &mut Vec<Owner>) -> usize {
        let code = &self.code;
        let name = code.name(keyword + 1);
        let path = match owners.last() {
            Some(owner) => format!("{}.{name}", owner.path),
            None => name.clone(),
        };
        let (kind, is_abstract) = match code.text(keyword) {
            "protocol" => ("interface", true),
            other => (other, false),
        };
        let id = format!("{}::{path}", self.file);
        self.open(
            keyword,
            keyword + 2,
            (id, name, path, kind, is_abstract),
            owners,
        )
    }

    /// Record `extension T` at `keyword`; [`attach_extensions`] later hands
    /// its members to `T`.
    fn extension(&mut self, keyword: usize, owners: &mut Vec<Owner>) -> usize {
        let code = &self.code;
        let name = code.dotted(keyword + 1);
        let header = keyword + 1 + 2 * name.matches('.').count() + 1;
        let line = code.toks[keyword].line;
        let id = format!("{}::extension {name}@{line}", self.file);
        let entry = (id, name.clone(), name, "extension", false);
        self.open(keyword, header, entry, owners)
    }

    /// Push a class and open its body as the current owner.
    fn open(
        &mut self,
        keyword: usize,
        header: usize,
        (id, name, path, kind, is_abstract): (String, String, String, &str, bool),
        owners: &mut Vec<Owner>,
    ) -> usize {
        let (body, end) = match self.code.body(header) {
            Ok(open) => (Some(open), open + 1),
            Err(end) => (None, end),
        };
        self.out.classes.push(CallClass {
            language: Some("swift".into()),
            id: id.clone(),
            name,
            package: self.package.into(),
            file: self.file.into(),
            graph_node: self.graph_node.into(),
            line: self.code.toks[keyword].line,
            kind: kind.into(),
            is_abstract: Some(is_abstract),
            visibility: None,
            supertypes: self.code.supertypes(header, body.unwrap_or(end)),
            supertype_sites: None,
            declaration_files: None,
        });
        if body.is_some() {
            owners.push(Owner { id, path });
        }
        end
    }

    /// The pseudo-class of top-level functions.
    fn module(&mut self) -> String {
        let id = format!("{}::(module)", self.file);
        if !self.out.classes.iter().any(|c| c.id == id) {
            self.out.classes.push(CallClass {
                language: Some("swift".into()),
                id: id.clone(),
                name: "(module)".into(),
                package: self.package.into(),
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
        }
        id
    }

    /// Record and measure the function declared at `keyword`; returns the
    /// index after its body, or after its header when it has none.
    fn function(&mut self, keyword: usize, fun: Fun, owner: Option<&Owner>) -> usize {
        let code = &self.code;
        let Some((name, header)) = signature_end(code, keyword, fun) else {
            return keyword + 1;
        };
        let Ok(open) = code.body(header) else {
            return header;
        };
        let close = code.pairs[&open];
        let modifiers = code.modifiers(keyword);
        let is_static = owner.is_none()
            || (fun == Fun::Func && modifiers.iter().any(|m| matches!(*m, "static" | "class")));
        let class_id = match owner {
            Some(owner) => owner.id.clone(),
            None => self.module(),
        };
        let toks = self.code.toks;
        let line = toks[keyword].line;
        let id = format!("{class_id}#{name}@{line}");
        let signature = self.source[toks[keyword].start..toks[open - 1].end]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let measure = Measure::of(&self.code, open + 1, close);
        self.out.methods.push(CallMethod {
            id: id.clone(),
            class_id: class_id.clone(),
            name,
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
            method_id: id.clone(),
            definition: "source-maintainability/v1".into(),
            cognitive: measure.cognitive,
            distinct_operators: measure.operators.len() as u32,
            distinct_operands: measure.operands.len() as u32,
            operators: measure.operator_count,
            operands: measure.operand_count,
            source_lines: measure.lines.len() as u32,
        });
        self.sites(open + 1, close, &measure.skip, (&class_id, &id));
        close + 1
    }

    /// The call sites in the body tokens `from..to`, outside local types.
    fn sites(
        &mut self,
        from: usize,
        to: usize,
        skip: &BTreeMap<usize, usize>,
        (class_id, method_id): (&str, &str),
    ) {
        let code = &self.code;
        let mut i = from;
        while i < to {
            if let Some(&end) = skip.get(&i) {
                i = end;
                continue;
            }
            let Some((start, target)) = call_at(code, i) else {
                i += 1;
                continue;
            };
            let first = &code.toks[start];
            let end = &code.toks[code.pairs[&(i + 1)]];
            let line_start = self.source[..first.start].rfind('\n').map_or(0, |n| n + 1);
            self.out.sites.push(SwiftCallSite {
                from_class: class_id.into(),
                from_method: method_id.into(),
                target,
                label: collapse(&self.source[first.start..code.toks[i].end]),
                line: first.line,
                column: (first.start - line_start + 1) as u32,
                code: collapse(&self.source[first.start..end.end])
                    .chars()
                    .take(CODE_LIMIT)
                    .collect(),
            });
            i += 1;
        }
    }
}

/// The function-like declaration a keyword at `i` starts.
fn fun_at(code: &Code, i: usize) -> Option<Fun> {
    let next = code.text(i + 1);
    match code.text(i) {
        "func" => Some(Fun::Func),
        "init" if matches!(next, "(" | "?" | "!" | "<") => Some(Fun::Init),
        "subscript" if matches!(next, "(" | "<") => Some(Fun::Subscript),
        "deinit" if next == "{" => Some(Fun::Deinit),
        _ => None,
    }
}

/// The name of the function declared at `keyword` and the index after its
/// parameter list; `None` for a malformed header.
fn signature_end(code: &Code, keyword: usize, fun: Fun) -> Option<(String, usize)> {
    let (name, mut j) = match fun {
        Fun::Func if keyword + 1 < code.toks.len() => (code.name(keyword + 1), keyword + 2),
        Fun::Func => return None,
        Fun::Init => ("init".to_string(), keyword + 1),
        Fun::Subscript => ("subscript".to_string(), keyword + 1),
        Fun::Deinit => return Some(("deinit".to_string(), keyword + 1)),
    };
    if fun == Fun::Init && matches!(code.text(j), "?" | "!") {
        j += 1;
    }
    if code.text(j) == "<" {
        j = generics_end(code, j);
    }
    (code.text(j) == "(").then(|| (name, code.pairs[&j] + 1))
}

/// The call whose callee name is at `i`: where its expression starts and
/// what it targets; `None` when no call starts there.
fn call_at(code: &Code, i: usize) -> Option<(usize, SwiftTarget)> {
    let called = (code.is_identifier(i) || code.text(i) == "init")
        && code.text(i + 1) == "("
        && code.adjacent(i, i + 1)
        && code.text(i.wrapping_sub(1)) != "func";
    if !called {
        return None;
    }
    let upper = |k: usize| code.toks[k].text.starts_with(|c: char| c.is_uppercase());
    if !code.after_dot(i) {
        let target = if upper(i) {
            SwiftTarget::Init(code.name(i))
        } else {
            SwiftTarget::Unresolved
        };
        return Some((i, target));
    }
    let typed = code.text(i - 1) == "."
        && i >= 2
        && code.is_identifier(i - 2)
        && upper(i - 2)
        && !code.after_dot(i - 2);
    if !typed {
        return Some((i, SwiftTarget::Unresolved));
    }
    let owner = code.name(i - 2);
    let target = match code.text(i) {
        "init" => SwiftTarget::Init(owner),
        _ => SwiftTarget::Static {
            owner,
            name: code.name(i),
        },
    };
    Some((i - 2, target))
}

/// The index after the `>` that closes the generic parameters at `open`.
fn generics_end(code: &Code, open: usize) -> usize {
    let mut depth = 0usize;
    let mut j = open;
    while j < code.toks.len() {
        match code.text(j) {
            "<" => depth += 1,
            ">" => {
                depth -= 1;
                if depth == 0 {
                    return j + 1;
                }
            }
            ">>" => {
                depth = depth.saturating_sub(2);
                if depth == 0 {
                    return j + 1;
                }
            }
            "{" | "(" if depth == 0 => return j,
            _ => {}
        }
        j += 1;
    }
    j
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// How a brace inside a measured body affects nesting.
#[derive(Clone, Copy, PartialEq)]
enum Block {
    Plain,
    Nest,
    Repeat,
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
    /// Local type bodies: start index to the index after them.
    skip: BTreeMap<usize, usize>,
}

impl Measure {
    /// Measure the body tokens `from..to`.
    fn of(code: &Code, from: usize, to: usize) -> Self {
        let mut marks = Marks::default();
        let mut i = from;
        while i < to {
            i = marks.mark(code, i);
        }
        let skip = marks.skip.clone();
        let mut m = Walk {
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
            closed_repeat: None,
        }
        .run(to);
        m.skip = skip;
        m
    }

    /// A decision that counts once and adds cognitive load: with nesting
    /// (`nested`) for a new construct, flat for `else if`.
    fn decision(&mut self, nested: bool) {
        self.cyclomatic += 1;
        self.cognitive += if nested { 1 + self.depth } else { 1 };
        self.max_nesting = self.max_nesting.max(self.depth + 1);
    }

    /// `switch`: cognitive load with nesting, but no decision of its own
    /// (its `case` labels count).
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
/// closure, local types to skip, the counted `case` labels and the `else`
/// of every `guard`.
#[derive(Default)]
struct Marks {
    blocks: BTreeMap<usize, Block>,
    skip: BTreeMap<usize, usize>,
    arms: BTreeSet<usize>,
    guard_else: BTreeSet<usize>,
}

impl Marks {
    /// Mark the token at `i`; returns the next index to look at.
    fn mark(&mut self, code: &Code, i: usize) -> usize {
        if code.after_dot(i) {
            return i + 1;
        }
        let braced = code.text(i + 1) == "{";
        match code.text(i) {
            "do" | "defer" if braced => {
                self.blocks.insert(i + 1, Block::Plain);
            }
            "repeat" if braced => {
                self.blocks.insert(i + 1, Block::Repeat);
            }
            "switch" => self.switch(code, i),
            "guard" => {
                if let Some(at) = scan_to(code, i + 1, "else") {
                    self.guard_else.insert(at);
                }
            }
            "class" | "struct" | "enum" | "protocol" | "actor" if code.is_identifier(i + 1) => {
                if let Ok(open) = code.body(i + 2) {
                    self.skip.insert(i, code.pairs[&open] + 1);
                    return code.pairs[&open] + 1;
                }
            }
            _ => {}
        }
        i + 1
    }

    /// The `{` that opens the body after the header at `i`.
    fn after_header(&self, code: &Code, i: usize) -> Option<usize> {
        scan_to(code, i + 1, "{")
    }

    /// Count the `case` labels of the `switch` at `i`, at its own level.
    fn switch(&mut self, code: &Code, i: usize) {
        let Some(open) = self.after_header(code, i) else {
            return;
        };
        let mut k = open + 1;
        while k < code.pairs[&open] {
            match code.text(k) {
                "case" if !matches!(code.text(k - 1), "if" | "guard" | "while" | "for" | ",") => {
                    self.arms.insert(k);
                }
                "(" | "[" | "{" => k = code.pairs[&k],
                _ => {}
            }
            k += 1;
        }
    }
}

/// The first `word` from `from` on at the same bracket level; `None` when
/// the enclosing scope closes first.
fn scan_to(code: &Code, from: usize, word: &str) -> Option<usize> {
    let mut k = from;
    while k < code.toks.len() {
        match code.text(k) {
            w if w == word => return Some(k),
            "(" | "[" | "{" => k = code.pairs[&k],
            ")" | "]" | "}" => return None,
            _ => {}
        }
        k += 1;
    }
    None
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
    closed_repeat: Option<usize>,
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
        // `#if`, `#else` and `#available` are compiler directives.
        let directive = i > 0 && self.code.text(i - 1) == "#";
        if !self.bracket(i, word) && !directive && !self.keyword(i, word) {
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
                // Unmarked braces are closures, local functions and control bodies.
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
                self.closed_repeat = (block == Block::Repeat).then_some(i);
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
        if code.after_dot(i) {
            return false;
        }
        match word {
            "if" => self
                .m
                .decision(!(i > self.from && code.text(i - 1) == "else")),
            "else" if code.text(i + 1) == "if" || self.marks.guard_else.contains(&i) => {}
            "else" => self.m.cognitive += 1,
            "while" if self.closed_repeat == Some(i.wrapping_sub(1)) => {}
            "guard" | "for" | "while" | "repeat" | "catch" => self.m.decision(true),
            "switch" => self.m.branch(),
            "break" | "continue" if self.labeled(i) => self.m.cognitive += 1,
            _ => return false,
        }
        true
    }

    /// `break label` / `continue label` on one line.
    fn labeled(&self, i: usize) -> bool {
        self.code.is_identifier(i + 1) && !self.code.new_line(i + 1)
    }

    /// `??`, the ternary `?`, logical operators and `case` labels.
    fn operator(&mut self, i: usize, word: &str) {
        let code = self.code;
        match word {
            "??" => self.m.cyclomatic += 1,
            // A ternary `?` stands apart; an optional `?` touches its type
            // or expression (`Int?`, `try?`, `a?.b`).
            "?" if !code.adjacent(i - 1, i) && !code.adjacent(i, i + 1) => self.m.decision(true),
            "&&" => self.logical(true),
            "||" => self.logical(false),
            "case" if self.marks.arms.contains(&i) => self.m.cyclomatic += 1,
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
