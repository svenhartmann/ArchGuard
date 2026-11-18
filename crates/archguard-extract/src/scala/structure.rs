//! Token-based Scala structure and measurements (ADR 0020): classes, case
//! classes, traits, objects, enums, given instances and `def`s with
//! per-def complexity, cognitive and Halstead counts. No Scala compiler is
//! needed; types stay unresolved. Call sites come from [`super::sites`].
//!
//! Bodies end at their closing brace or, without braces (Scala 3
//! indentation syntax, brace-less Scala 2 bodies), at the first line
//! indented no deeper than the declaration ([`Code::region_end`]); `end`
//! markers are skipped. Rules mirror the Kotlin analyzer
//! (`ast-decisions-v1`, `source-maintainability/v1`):
//! - cyclomatic: `if`, `while`, `for`, each `case` clause except a bare
//!   `case _ =>` (match, catch and partial-function cases alike), `&&` and
//!   `||`. `match`, `catch`, `try` and `do` add nothing of their own.
//! - cognitive: `if`, `while`, `for`, `match` and `catch` add 1 plus
//!   nesting; `else if` and `else` add 1; a run of logical operators adds 1
//!   plus 1 per change of operator (a line break ends a run unless an
//!   operator continues it).
//! - nesting is read from lines: a line with `if`, `else`, `while`, `for`,
//!   `match`, `catch` or a lambda `=>` nests the deeper indented lines after
//!   it, in brace and indentation syntax alike. Constructs on one line share
//!   that line's nesting. Local defs and classes count toward their def.
//! - Halstead: keywords and operators are operators; names and literals are
//!   operands, an interpolated string with its `${…}` parts is one operand;
//!   brackets, `;` and `,` are neither.
//!
//! Recorded defs have a body (`= …` or Scala 2 procedure syntax `{ … }`);
//! abstract defs, `val`s, constructors and template statements are not
//! recorded.

use std::collections::BTreeSet;

use archguard_core::{CallClass, CallMethod, MethodComplexity, SourceMethodMetrics};

use super::sites;
use super::symbols::{ScalaClassFacts, ScalaSymbols};
use super::tokens::{Code, KEYWORDS, Tok, merged, pairs};

/// Classes, defs and raw measurements of one Scala file.
#[derive(Debug, Default)]
pub struct ScalaStructure {
    pub classes: Vec<CallClass>,
    pub methods: Vec<CallMethod>,
    pub metrics: Vec<SourceMethodMetrics>,
    pub diagnostics: Vec<String>,
    /// Declarations and call sites for the engine's call resolver.
    pub symbols: ScalaSymbols,
}

/// Scan one Scala source. `file` is the source path, `graph_node` the name
/// of its node in the dependency graph.
pub fn scan_scala_structure(file: &str, graph_node: &str, source: &str) -> ScalaStructure {
    let toks = merged(source);
    let mut out = ScalaStructure::default();
    let Some(pairs) = pairs(&toks) else {
        out.diagnostics
            .push(format!("{file}: unbalanced brackets; not measured"));
        return out;
    };
    out.symbols.measured = true;
    out.symbols.imports = super::clauses::bindings(source);
    let mut scan = Scan {
        file,
        graph_node,
        source,
        code: Code::new(&toks, pairs, source),
        package: Vec::new(),
        out,
        seen: BTreeSet::new(),
    };
    scan.walk();
    scan.out.symbols.package = scan.package.join(".");
    scan.out
}

/// An open template or `extension` block, until the token index `end`.
struct Owner {
    /// The class id; `None` at the top level.
    id: Option<String>,
    object: bool,
    end: usize,
    extension: bool,
}

struct Scan<'a> {
    file: &'a str,
    graph_node: &'a str,
    source: &'a str,
    code: Code<'a>,
    package: Vec<String>,
    out: ScalaStructure,
    seen: BTreeSet<String>,
}

impl Scan<'_> {
    /// Declarations outside def bodies; bodies are measured and skipped.
    fn walk(&mut self) {
        let mut owners: Vec<Owner> = Vec::new();
        let mut i = 0;
        while i < self.code.toks.len() {
            while owners.last().is_some_and(|o| i >= o.end) {
                owners.pop();
            }
            i = self.declaration(i, &mut owners);
        }
    }

    /// Handle the token at `i` outside any def body; returns the next index.
    fn declaration(&mut self, i: usize, owners: &mut Vec<Owner>) -> usize {
        let code = &self.code;
        if code.toks[i].literal || code.after_member(i) {
            return i + 1;
        }
        match code.text(i) {
            // `end Name` closes an indentation region that already ended.
            "end" if code.new_line(i) && !code.new_line(i + 1) => i + 2,
            "package" => self.package_clause(i),
            "def" => self.function(i, owners.last()),
            "extension" if matches!(code.text(i + 1), "(" | "[") => self.extension(i, owners),
            "{" | "(" | "[" => code.pairs[&i] + 1,
            _ => match self.class_at(i) {
                Some((kind, name)) => self.class(i, kind, &name, owners),
                None => i + 1,
            },
        }
    }

    /// `package a.b`, `package a.b {` (its members follow) or `package object`.
    fn package_clause(&mut self, i: usize) -> usize {
        let code = &self.code;
        if code.text(i + 1) == "object" || !code.is_name(i + 1) {
            return i + 1;
        }
        let (names, next) = code.dotted(i + 1);
        self.package.extend(names);
        next + usize::from(code.text(next) == "{")
    }

    /// The kind and name of a class-like declaration at `i`.
    fn class_at(&self, i: usize) -> Option<(&'static str, String)> {
        let code = &self.code;
        let named = code.is_name(i + 1).then(|| code.name(i + 1));
        let skip = |mut k: usize| {
            while matches!(code.text(k), "[" | "(") {
                k = code.pairs[&k] + 1;
            }
            k
        };
        match code.text(i) {
            "class" => Some(("class", named?)),
            "trait" => Some(("trait", named?)),
            "object" => Some(("object", named?)),
            "enum" => Some(("enum", named?)),
            // `given name: T with …`, or anonymous `given T with …` (named
            // `given_T` as the compiler names it).
            "given" => {
                if named.is_some() && code.text(skip(i + 2)) == ":" {
                    return Some(("object", named?));
                }
                let mut k = skip(i + 1);
                k += usize::from(code.text(k) == ":");
                code.is_name(k)
                    .then(|| ("object", format!("given_{}", code.name(k))))
            }
            _ => None,
        }
    }

    /// Record the class-like declaration at `keyword`; returns where to go on.
    fn class(&mut self, keyword: usize, kind: &str, name: &str, owners: &mut Vec<Owner>) -> usize {
        let code = &self.code;
        let given = code.text(keyword) == "given";
        let from = keyword + 1 + usize::from(!given);
        let (body, header_end) = self.template(from, code.indent(keyword), given);
        let modifiers = code.modifiers(keyword);
        let outer = owners.iter().rev().find_map(|o| o.id.clone());
        let id = match &outer {
            Some(outer) => format!("{outer}.{name}"),
            None => format!("{}::{name}", self.file),
        };
        let supertypes = if given {
            Vec::new()
        } else {
            code.supertypes(from, header_end)
        };
        let is_abstract = kind == "trait" || modifiers.iter().any(|m| m == "abstract");
        let line = code.toks[keyword].line;
        self.out.symbols.classes.push(ScalaClassFacts {
            id: id.clone(),
            name: name.into(),
            kind: kind.into(),
            outer,
            supertypes: supertypes.clone(),
            modifiers,
        });
        self.klass(CallClass {
            language: Some("scala".into()),
            id: id.clone(),
            name: name.into(),
            package: self.package.join("."),
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
        match body {
            Some((first, end)) => {
                owners.push(Owner {
                    id: Some(id),
                    object: kind == "object",
                    end,
                    extension: false,
                });
                first
            }
            None => header_end,
        }
    }

    /// The body of the template whose header continues at `from` on a line
    /// indented by `base`: its first token and end, and where the header
    /// ends. A `given` opens its body with `with`; `given … = expr` has none.
    fn template(&self, from: usize, base: u32, given: bool) -> (Option<(usize, usize)>, usize) {
        let code = &self.code;
        let opener = if given { "with" } else { ":" };
        let mut j = from;
        while !code.closes(j, from, base) {
            match code.text(j) {
                "(" | "[" => {
                    j = code.pairs[&j] + 1;
                    continue;
                }
                "=" if given => return (None, code.region_end(j + 1, base)),
                word => {
                    // `{` opens a class body; a `given` has `with {`.
                    let brace = if given { j + 1 } else { j };
                    if (!given || word == "with") && code.text(brace) == "{" {
                        return (Some((brace + 1, code.pairs[&brace])), j);
                    }
                    if word == opener && code.line_ends(j) {
                        return (Some((j + 1, code.region_end(j + 1, base))), j);
                    }
                }
            }
            j += 1;
        }
        (None, j)
    }

    /// An `extension (x: T)` block: its defs belong to the enclosing scope,
    /// and calls in them stay unresolved.
    fn extension(&mut self, keyword: usize, owners: &mut Vec<Owner>) -> usize {
        let code = &self.code;
        let mut j = keyword + 1;
        while matches!(code.text(j), "(" | "[") {
            j = code.pairs[&j] + 1;
        }
        let (first, end) = if code.text(j) == "{" {
            (j + 1, code.pairs[&j])
        } else {
            (j, code.region_end(j, code.indent(keyword)))
        };
        let enclosing = owners.last();
        owners.push(Owner {
            id: enclosing.and_then(|o| o.id.clone()),
            object: enclosing.is_none_or(|o| o.object),
            end,
            extension: true,
        });
        first
    }

    fn klass(&mut self, class: CallClass) {
        if self.seen.insert(class.id.clone()) {
            self.out.classes.push(class);
        }
    }

    /// The pseudo-class of top-level defs.
    fn module(&mut self) -> String {
        let id = format!("{}::(module)", self.file);
        if !self.seen.contains(&id) {
            self.out.symbols.classes.push(ScalaClassFacts {
                id: id.clone(),
                name: "(module)".into(),
                kind: "module".into(),
                outer: None,
                supertypes: Vec::new(),
                modifiers: Vec::new(),
            });
        }
        self.klass(CallClass {
            language: Some("scala".into()),
            id: id.clone(),
            name: "(module)".into(),
            package: self.package.join("."),
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

    /// Record and measure the def at `keyword`; returns the index after its
    /// body, or after its header when it has none.
    fn function(&mut self, keyword: usize, owner: Option<&Owner>) -> usize {
        let body = match self.def_body(keyword) {
            Ok(body) => body,
            Err(next) => return next,
        };
        let class_id = match owner.and_then(|o| o.id.clone()) {
            Some(id) => id,
            None => self.module(),
        };
        let static_member = owner.is_none_or(|o| o.object);
        let extension = owner.is_some_and(|o| o.extension);
        self.record(keyword, &body, class_id, (static_member, extension));
        body.next
    }

    /// The body of the def at `keyword`, or where to go on when it has none.
    fn def_body(&self, keyword: usize) -> Result<Body, usize> {
        let code = &self.code;
        if keyword + 1 >= code.toks.len() || code.text(keyword + 1) == "this" {
            return Err(keyword + 1);
        }
        let base = code.indent(keyword);
        let mut j = keyword + 2;
        let head_end = loop {
            if code.closes(j, keyword, base) {
                return Err(j);
            }
            match code.text(j) {
                "(" | "[" => j = code.pairs[&j],
                "=" | "{" => break j,
                _ => {}
            }
            j += 1;
        };
        let (to, next, end_line) = if code.text(head_end) == "{" {
            let close = code.pairs[&head_end];
            (close, close + 1, code.toks[close].line)
        } else {
            let end = code.region_end(head_end + 1, base);
            (end, end, code.toks[end - 1].end_line)
        };
        if to <= head_end + 1 {
            return Err(next);
        }
        Ok(Body {
            head_end,
            from: head_end + 1,
            to,
            next,
            end_line,
        })
    }

    /// Record the def at `keyword` with its measurements and call sites.
    fn record(&mut self, keyword: usize, body: &Body, class_id: String, kind: (bool, bool)) {
        let (is_static, extension) = kind;
        let code = &self.code;
        let toks = code.toks;
        let name = code.name(keyword + 1);
        let line = toks[keyword].line;
        let id = format!("{class_id}#{name}@{line}");
        let signature = self.source[toks[keyword].start..toks[body.head_end - 1].end]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let measure = Measure::of(code, body.from, body.to);
        let owner = sites::Owner {
            class_id: &class_id,
            method_id: &id,
            extension,
        };
        let found = sites::collect(code, self.source, (body.from, body.to), owner);
        self.out.symbols.sites.extend(found);
        self.out.methods.push(CallMethod {
            id: id.clone(),
            class_id,
            name,
            signature,
            file: self.file.into(),
            line,
            end_line: Some(body.end_line),
            is_static,
            complexity: Some(MethodComplexity {
                cyclomatic: measure.cyclomatic,
                max_nesting: measure.max_nesting,
                definition: "ast-decisions-v1".into(),
            }),
        });
        self.out.metrics.push(measure.metrics(id));
    }
}

/// A def body: the `=` or `{` that ends the header, the measured tokens
/// `from..to`, where the walk goes on and the last line.
struct Body {
    head_end: usize,
    from: usize,
    to: usize,
    next: usize,
    end_line: u32,
}

#[derive(Default)]
struct Measure {
    cyclomatic: u32,
    cognitive: u32,
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
            opened: 0,
            level: 0,
            logic: None,
            pattern: false,
        };
        for i in from..to {
            walk.step(i);
        }
        walk.m
    }

    fn metrics(&self, method_id: String) -> SourceMethodMetrics {
        SourceMethodMetrics {
            method_id,
            definition: "source-maintainability/v1".into(),
            cognitive: self.cognitive,
            distinct_operators: self.operators.len() as u32,
            distinct_operands: self.operands.len() as u32,
            operators: self.operator_count,
            operands: self.operand_count,
            source_lines: self.lines.len() as u32,
        }
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

/// The measuring pass over a body, token by token.
struct Walk<'a> {
    code: &'a Code<'a>,
    from: usize,
    m: Measure,
    /// Indents of the lines that nest the lines below them.
    stack: Vec<u32>,
    /// The last line that opened a nesting.
    opened: u32,
    level: usize,
    /// The current run of logical operators: its level and whether it is `&&`.
    logic: Option<(usize, bool)>,
    /// Inside a `case` pattern, up to its `=>`.
    pattern: bool,
}

impl Walk<'_> {
    fn step(&mut self, i: usize) {
        let code = self.code;
        let word = code.text(i);
        let logical = |w: &str| matches!(w, "&&" | "||");
        if i == self.from || code.new_line(i) {
            let indent = code.indent(i);
            while self.stack.last().is_some_and(|&b| b >= indent) {
                self.stack.pop();
            }
            if i > self.from && !logical(word) && !logical(code.text(i - 1)) {
                self.logic = None;
            }
        }
        self.m.halstead(&code.toks[i]);
        if self.control(i, word) && self.opened != code.toks[i].line {
            self.opened = code.toks[i].line;
            self.stack.push(code.indent(i));
            self.m.max_nesting = self.m.max_nesting.max(self.stack.len() as u32);
        }
    }

    /// Count the word at `i`; true when its line nests the lines below.
    fn control(&mut self, i: usize, word: &str) -> bool {
        let code = self.code;
        let depth = self.stack.len() as u32;
        match word {
            "if" => {
                let flat = i > self.from && code.text(i - 1) == "else";
                self.m.cyclomatic += 1;
                self.m.cognitive += if flat { 1 } else { 1 + depth };
            }
            "else" => self.m.cognitive += u32::from(code.text(i + 1) != "if"),
            "while" | "for" => {
                self.m.cyclomatic += 1;
                self.m.cognitive += 1 + depth;
            }
            "match" | "catch" => self.m.cognitive += 1 + depth,
            // A lambda arrow; the arrow of a case pattern does not nest.
            "=>" if !self.pattern => {}
            _ => {
                self.operator(i, word);
                return false;
            }
        }
        true
    }

    /// Case clauses, logical operators and brackets.
    fn operator(&mut self, i: usize, word: &str) {
        let code = self.code;
        match word {
            "case" if !matches!(code.text(i + 1), "class" | "object") => {
                let default = code.text(i + 1) == "_" && code.text(i + 2) == "=>";
                self.m.cyclomatic += u32::from(!default);
                self.pattern = true;
            }
            "=>" => self.pattern = false,
            "&&" | "||" => {
                let and = word == "&&";
                self.m.cyclomatic += 1;
                self.m.cognitive += match self.logic {
                    Some((level, previous)) if level == self.level => u32::from(previous != and),
                    _ => 1,
                };
                self.logic = Some((self.level, and));
            }
            "(" | "[" => self.level += 1,
            ")" | "]" => {
                self.level = self.level.saturating_sub(1);
                self.logic = self.logic.filter(|(level, _)| *level <= self.level);
            }
            "{" | "}" | ";" | "," => self.logic = None,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scan(source: &str) -> ScalaStructure {
        scan_scala_structure("src/Shop.scala", "com/shop/Shop.scala", source)
    }
    fn method<'a>(found: &'a ScalaStructure, name: &str) -> &'a CallMethod {
        found
            .methods
            .iter()
            .find(|m| m.name == name)
            .unwrap_or_else(|| panic!("no def {name}: {:?} {:?}", found.methods, found.diagnostics))
    }
    fn metrics<'a>(found: &'a ScalaStructure, name: &str) -> &'a SourceMethodMetrics {
        let id = &method(found, name).id;
        found.metrics.iter().find(|m| &m.method_id == id).unwrap()
    }
    /// (cyclomatic, max nesting, cognitive) of one def.
    fn complexity(found: &ScalaStructure, name: &str) -> (u32, u32, u32) {
        let measured = method(found, name).complexity.as_ref().unwrap();
        (
            measured.cyclomatic,
            measured.max_nesting,
            metrics(found, name).cognitive,
        )
    }

    #[test]
    fn counts_follow_the_shared_rules_in_brace_syntax() {
        let found = scan(
            r#"package com.shop

object Rules {
  def plain = 1
  def block(): Int = { return 1 }
  def branches(a: Boolean, b: Boolean): Int = {
    if (a) {
      if (b) { return 1 } else { return 2 }
    } else if (b) {
      return 3
    } else {
      return 4
    }
  }
  def logic(a: Boolean, b: Boolean, c: Boolean) = a && b || c
  def statements(a: Boolean, b: Boolean): Boolean = {
    val x = a && b
    val y = a &&
      b
    x && y
  }
  def arms(x: Int): String = x match {
    case 0 | 1 => "low"
    case 2 => { "mid" }
    case _ => "high"
  }
  def guarded(items: List[Int]): Int = {
    try {
      for (item <- items) {
        while (item > 0) { println(item) }
      }
      0
    } catch {
      case e: IllegalStateException => 1
      case e: Exception => 2
    } finally {
      println("done")
    }
  }
  def lambdas(xs: List[Int]): List[Int] =
    xs.map { x =>
      if (x > 0) x else 0
    }
  def procedure(a: Boolean) { if (a) println(a) }
}
"#,
        );
        assert_eq!(complexity(&found, "plain"), (1, 0, 0));
        assert_eq!(complexity(&found, "block"), (1, 0, 0));
        // Same shape and numbers as the Kotlin, Rust, Python and PHP cases.
        assert_eq!(complexity(&found, "branches"), (4, 2, 6));
        assert_eq!(complexity(&found, "logic"), (3, 0, 2));
        assert_eq!(complexity(&found, "statements"), (4, 0, 3));
        // Each case except the bare `case _`; `0 | 1` is one case.
        assert_eq!(complexity(&found, "arms"), (3, 1, 1));
        // `try` and `finally` neither count nor nest; catch cases count.
        assert_eq!(complexity(&found, "guarded"), (5, 2, 4));
        // The lambda line nests its body: the `if` sits one level deeper.
        assert_eq!(complexity(&found, "lambdas"), (2, 2, 3));
        assert_eq!(complexity(&found, "procedure"), (2, 1, 1));
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
        assert_eq!(method(&found, "arms").end_line, Some(26));
        assert!(method(&found, "plain").is_static);
        assert!(found.diagnostics.is_empty(), "{:?}", found.diagnostics);
    }

    #[test]
    fn scala3_indentation_syntax_gives_the_same_counts() {
        let found = scan(
            r#"package com.shop

object Rules:
  def branches(a: Boolean, b: Boolean): Int =
    if a then
      if b then 1 else 2
    else if b then
      3
    else
      4

  def arms(x: Int): String = x match
    case 0 | 1 => "low"
    case 2 => "mid"
    case _ => "high"

  def loops(xs: List[Int]): Int =
    var n = 0
    while n < 10 do
      for x <- xs do
        n += x
    n
  end loops

  def after(a: Boolean) = if a then 1 else 2
end Rules

def topLevel(s: String): String =
  val t = s"${if (s.isEmpty) "x" else s}"
  t
"#,
        );
        assert_eq!(complexity(&found, "branches"), (4, 2, 6));
        assert_eq!(complexity(&found, "arms"), (3, 1, 1));
        assert_eq!(complexity(&found, "loops"), (3, 2, 3));
        assert_eq!(complexity(&found, "after"), (2, 1, 2));
        assert_eq!(method(&found, "loops").end_line, Some(22));
        // An interpolated string with its `${…}` parts is one operand.
        assert_eq!(complexity(&found, "topLevel"), (1, 0, 0));
        let top = metrics(&found, "topLevel");
        assert_eq!((top.operators, top.operands), (2, 3));
        assert_eq!(
            method(&found, "topLevel").class_id,
            "src/Shop.scala::(module)"
        );
        assert_eq!(method(&found, "after").class_id, "src/Shop.scala::Rules");
        assert_eq!(found.methods.len(), 5, "{:?}", found.methods);
        assert!(found.diagnostics.is_empty(), "{:?}", found.diagnostics);
    }

    #[test]
    fn classes_defs_and_abstractness_come_from_declarations() {
        let found = scan(
            r#"package com.shop
package model

import com.shop.base.Base

trait Port { def send(msg: String): Unit }

sealed trait Shape:
  def area: Double

final case class Circle(r: Double) extends Shape with Serializable:
  def area = 3.14 * r * r

case object Empty extends Shape { def area = 0.0 }

abstract class Repo[T](val name: String) extends Base(name) with Port {
  override def send(msg: String): Unit = println(msg)
  class Cursor { def next(): Int = 1 }
  val size: Int = 0
}

object Repo {
  def apply(name: String): Repo[Int] = null
}

enum Suit:
  case Hearts, Spades
  def short = toString.take(1)

given intOrd: Ordering[Int] with
  def compare(a: Int, b: Int) = a - b

given Ordering[String] with {
  def compare(a: String, b: String) = 0
}

extension (s: String)
  def shout: String = s.toUpperCase + "!"

class Plain
"#,
        );
        let class = |id: &str| {
            found
                .classes
                .iter()
                .find(|c| c.id == format!("src/Shop.scala::{id}"))
                .unwrap_or_else(|| panic!("no class {id}: {:?}", found.classes))
        };
        let kinds: Vec<_> = [
            "Port",
            "Shape",
            "Circle",
            "Empty",
            "Repo",
            "Repo.Cursor",
            "Suit",
            "intOrd",
            "given_Ordering",
            "(module)",
            "Plain",
        ]
        .iter()
        .map(|id| (class(id).kind.as_str(), class(id).is_abstract))
        .collect();
        assert_eq!(
            kinds,
            [
                ("trait", Some(true)),
                ("trait", Some(true)),
                ("class", Some(false)),
                ("object", Some(false)),
                ("class", Some(true)),
                ("class", Some(false)),
                ("enum", Some(false)),
                ("object", Some(false)),
                ("object", Some(false)),
                ("module", Some(false)),
                ("class", Some(false)),
            ]
        );
        // The class `Repo` and its companion object share one id.
        assert_eq!(found.classes.len(), 11, "{:?}", found.classes);
        let repo = class("Repo");
        assert_eq!(repo.supertypes, ["Base", "Port"]);
        assert_eq!(repo.package, "com.shop.model");
        assert_eq!(repo.language.as_deref(), Some("scala"));
        assert_eq!(class("Circle").supertypes, ["Serializable", "Shape"]);
        assert_eq!(found.symbols.package, "com.shop.model");
        let facts = found
            .symbols
            .classes
            .iter()
            .find(|c| c.name == "Circle")
            .unwrap();
        assert_eq!(facts.modifiers, ["case", "final"]);
        // Abstract defs have nothing to measure.
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
                ("area", "Circle", false),
                ("area", "Empty", true),
                ("send", "Repo", false),
                ("next", "Repo.Cursor", false),
                ("apply", "Repo", true),
                ("short", "Suit", false),
                ("compare", "intOrd", true),
                ("compare", "given_Ordering", true),
                ("shout", "(module)", true),
            ]
        );
        assert_eq!(
            method(&found, "send").signature,
            "def send(msg: String): Unit"
        );
        assert!(found.diagnostics.is_empty(), "{:?}", found.diagnostics);
    }

    #[test]
    fn unbalanced_sources_are_reported_not_measured() {
        let found = scan("def broken() = { if (\n");
        assert!(found.methods.is_empty());
        assert!(!found.symbols.measured);
        assert_eq!(
            found.diagnostics,
            ["src/Shop.scala: unbalanced brackets; not measured"]
        );
    }
}
