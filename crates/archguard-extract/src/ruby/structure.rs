//! Token-based Ruby structure and measurements: classes, modules and methods
//! with per-method complexity, cognitive and Halstead counts, plus the
//! constant references and call sites the engine resolves ([`super::sites`]).
//! Nothing is executed and no Ruby installation is needed.
//!
//! Rules, aligned with the PHP and Kotlin scanners (`ast-decisions-v1`,
//! `source-maintainability/v1`):
//! - cyclomatic: `if`, `unless`, `elsif`, `while`, `until`, `for` (block and
//!   modifier forms alike), each `when` and each `in` clause of a `case`,
//!   each `rescue` (clause or modifier), the ternary `?`, `&&`, `||`, `and`,
//!   `or`, `||=`, `&&=` and the safe navigation `&.`.
//! - cognitive: `if`, `unless`, loops, `case`, `rescue`, ternaries and the
//!   modifier forms add 1 plus nesting; `elsif` and the `else` of an `if`
//!   add 1; a run of logical operators adds 1 plus 1 per change of operator
//!   (`and` counts as `&&`, `or` as `||`). `begin`/`ensure` add nothing.
//! - nesting follows `if`/`unless`/loop/`case` bodies and blocks (`do … end`,
//!   `{ … }`); a modifier has no body and does not nest. Rescue clauses do
//!   not nest their body (they have no delimiters of their own).
//! - Halstead: keywords and operators are operators; identifiers, constants,
//!   variables, `self`, `nil`, `true`, `false` and literals are operands;
//!   brackets, `,`, `;` and `end` are neither.
//! - nested `def`, `class` and `module` inside a method are not measured
//!   with it.

use std::collections::{BTreeMap, BTreeSet};

use archguard_core::{CallClass, CallMethod, MethodComplexity, SourceMethodMetrics};

use super::code::{Code, Header, KEYWORDS, Kind, nested_scope_end};
use super::sites::Sites;
use super::symbols::{RubyClassFacts, RubyConst, RubyMethodFacts, RubySymbols, Superclass};
use crate::lex::Token;

/// Classes, methods and raw measurements of one Ruby file.
#[derive(Debug, Default)]
pub struct RubyStructure {
    pub classes: Vec<CallClass>,
    pub methods: Vec<CallMethod>,
    pub metrics: Vec<SourceMethodMetrics>,
    pub diagnostics: Vec<String>,
    /// Declarations, constant references and call sites for the engine.
    pub symbols: RubySymbols,
}

/// Scan one Ruby source. `file` is the source path, `graph_node` the name of
/// its node in the dependency graph.
pub fn scan_ruby_structure(file: &str, graph_node: &str, source: &str) -> RubyStructure {
    let Some(code) = Code::new(source) else {
        return RubyStructure {
            diagnostics: vec![format!("{file}: unbalanced blocks; not measured")],
            ..Default::default()
        };
    };
    let mut scan = Scan {
        file,
        graph_node,
        code: &code,
        package: file.rsplit_once('/').map_or("", |(dir, _)| dir).to_string(),
        out: RubyStructure::default(),
        facts: Vec::new(),
        index: BTreeMap::new(),
    };
    scan.body(
        0,
        code.toks.len(),
        &Scope {
            nesting: Vec::new(),
            class: None,
            singleton: false,
        },
    );
    let Scan { mut out, facts, .. } = scan;
    out.symbols.measured = true;
    out.symbols.classes = facts;
    out
}

/// Where a token stands: its lexical nesting and the class whose body it is in.
#[derive(Clone)]
struct Scope {
    nesting: Vec<String>,
    /// Index into `facts`; `None` at the top level.
    class: Option<usize>,
    /// Inside `class << self`.
    singleton: bool,
}

struct Scan<'a> {
    file: &'a str,
    graph_node: &'a str,
    code: &'a Code<'a>,
    package: String,
    out: RubyStructure,
    facts: Vec<RubyClassFacts>,
    /// Class id → index into `facts`.
    index: BTreeMap<String, usize>,
}

impl Scan<'_> {
    /// Declarations in `from..to`; method bodies are measured and skipped.
    fn body(&mut self, from: usize, to: usize, scope: &Scope) {
        let code = self.code;
        let mut i = from;
        while i < to {
            let kind = code.kinds.get(&i).copied();
            let word = code.text(i);
            match kind {
                Some(Kind::Class | Kind::Module) => {
                    i = self.class(i, kind == Some(Kind::Class), scope);
                    continue;
                }
                Some(Kind::Singleton) => {
                    let close = code.pairs[&i];
                    let inner = Scope {
                        singleton: true,
                        ..scope.clone()
                    };
                    self.body(code.line_end(i), close, &inner);
                    i = close + 1;
                    continue;
                }
                // ActiveSupport::Concern: `class_methods do … end`.
                Some(Kind::Block) if code.text(i.wrapping_sub(1)) == "class_methods" => {
                    let close = code.pairs[&i];
                    let inner = Scope {
                        singleton: true,
                        ..scope.clone()
                    };
                    self.body(i + 1, close, &inner);
                    i = close + 1;
                    continue;
                }
                _ => {}
            }
            if word == "def"
                && !code.after_dot(i)
                && let Some(header) = code.def_header(i)
            {
                self.method(i, &header, scope);
                i = header.end + 1;
                continue;
            }
            if let Some(next) = self.statement(i, scope) {
                i = next;
                continue;
            }
            if let Some((constant, next)) = super::sites::constant_at(code, i, &scope.nesting) {
                self.out
                    .symbols
                    .references
                    .push((code.toks[i].line, constant));
                i = next;
                continue;
            }
            i += 1;
        }
    }

    /// `include`/`extend`/`prepend X, Y` in a class body, or `CONST = …`.
    fn statement(&mut self, i: usize, scope: &Scope) -> Option<usize> {
        let code = self.code;
        if code.after_dot(i) || !code.statement_start(i) {
            return None;
        }
        let word = code.text(i);
        if code.text(i + 1) == "=" && word.starts_with(|c: char| c.is_ascii_uppercase()) {
            let name = match scope.nesting.last() {
                Some(outer) => format!("{outer}::{word}"),
                None => word.to_string(),
            };
            self.out.symbols.constants.push(name);
            return Some(i + 2);
        }
        let class = scope.class?;
        if !matches!(word, "include" | "extend" | "prepend") {
            return None;
        }
        let (found, at) = self.constant_list(i + 1, &scope.nesting);
        let facts = &mut self.facts[class];
        match word {
            "include" => facts.includes.extend(found),
            "prepend" => facts.prepends.extend(found),
            _ if scope.singleton => facts.includes.extend(found),
            _ => facts.extends.extend(found),
        }
        Some(at.max(i + 1))
    }

    /// `A, B::C` (optionally in parentheses) from `at`, recorded as
    /// references; the index after the list.
    fn constant_list(&mut self, mut at: usize, nesting: &[String]) -> (Vec<RubyConst>, usize) {
        let code = self.code;
        if code.text(at) == "(" {
            at += 1;
        }
        let mut found = Vec::new();
        while let Some((constant, next)) = super::sites::constant_at(code, at, nesting) {
            self.out
                .symbols
                .references
                .push((code.toks[at].line, constant.clone()));
            found.push(constant);
            at = next;
            if code.text(at) != "," {
                break;
            }
            at += 1;
            while code.text(at) == "\n" {
                at += 1;
            }
        }
        (found, at)
    }

    /// The facts index of `name`, creating its `CallClass` on first sight.
    fn declare(&mut self, name: &str, kind: &str, line: u32, superclass: Superclass) -> usize {
        let id = format!("{}::{name}", self.file);
        if let Some(&known) = self.index.get(&id) {
            if self.facts[known].superclass == Superclass::None {
                self.facts[known].superclass = superclass;
            }
            return known;
        }
        self.out.classes.push(CallClass {
            language: Some("ruby".into()),
            id: id.clone(),
            name: name.into(),
            package: self.package.clone(),
            file: self.file.into(),
            graph_node: self.graph_node.into(),
            line,
            kind: kind.into(),
            // Ruby has no abstract classes: not applicable, see ADR 0019.
            is_abstract: None,
            visibility: None,
            supertypes: Vec::new(),
            supertype_sites: None,
            declaration_files: None,
        });
        self.facts.push(RubyClassFacts {
            id: id.clone(),
            name: name.into(),
            kind: kind.into(),
            superclass,
            includes: Vec::new(),
            prepends: Vec::new(),
            extends: Vec::new(),
            methods: Vec::new(),
        });
        self.index.insert(id, self.facts.len() - 1);
        self.facts.len() - 1
    }

    /// A `class`/`module` at `i`; returns the index after its `end`.
    fn class(&mut self, i: usize, is_class: bool, scope: &Scope) -> usize {
        let code = self.code;
        let close = code.pairs[&i];
        let Some((written, after)) = super::sites::constant_at(code, i + 1, &[]) else {
            // `Class.new` style or malformed: scan the body in the outer scope.
            self.body(i + 1, close, scope);
            return close + 1;
        };
        let name = match (written.absolute, scope.nesting.last()) {
            (false, Some(outer)) => format!("{outer}::{}", written.path),
            _ => written.path.clone(),
        };
        let mut body_start = after;
        let mut superclass = Superclass::None;
        if is_class && code.text(after) == "<" {
            let end = code.line_end(after + 1);
            superclass = match super::sites::constant_at(code, after + 1, &scope.nesting) {
                Some((constant, next)) if next == end => {
                    self.out
                        .symbols
                        .references
                        .push((code.toks[after + 1].line, constant.clone()));
                    Superclass::Const(constant)
                }
                _ => Superclass::Expression,
            };
            body_start = end;
        }
        let kind = if is_class { "class" } else { "module" };
        let class = self.declare(&name, kind, code.toks[i].line, superclass);
        let mut nesting = scope.nesting.clone();
        nesting.push(name);
        let inner = Scope {
            nesting,
            class: Some(class),
            singleton: false,
        };
        self.body(body_start, close, &inner);
        close + 1
    }

    /// Record and measure the method declared at `keyword`.
    fn method(&mut self, keyword: usize, header: &Header, scope: &Scope) {
        let code = self.code;
        let class = match scope.class {
            Some(class) => class,
            None => self.declare("(module)", "module", 1, Superclass::None),
        };
        let class_id = self.facts[class].id.clone();
        let line = code.toks[keyword].line;
        let id = format!("{class_id}#{}@{line}", header.name);
        let singleton = header.singleton || scope.singleton;
        let signature = code.source
            [code.toks[keyword].start..code.toks[header.signature_end.max(keyword + 1)].end]
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let measure = Measure::of(code, header.body);
        let mut sites = Sites::new(code, &scope.nesting, (&class_id, &id), singleton);
        sites.params(header.params);
        sites.scan(header.body, &header.name);
        let (found, references) = sites.finish();
        self.out.symbols.sites.extend(found);
        self.out.symbols.references.extend(references);
        self.facts[class].methods.push(RubyMethodFacts {
            name: header.name.clone(),
            method_id: id.clone(),
            singleton,
        });
        self.out.methods.push(CallMethod {
            id: id.clone(),
            class_id,
            name: header.name.clone(),
            signature,
            file: self.file.into(),
            line,
            end_line: code.toks.get(header.end).map(|t| t.line),
            is_static: singleton,
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
    /// Open blocks: their closing token and kind.
    open: Vec<(usize, Kind)>,
    /// The current run of logical operators: bracket level and `&&`-ness.
    logic: Option<(usize, bool)>,
    /// Bracket level.
    level: usize,
}

impl Measure {
    /// Measure the body tokens `from..to`.
    fn of(code: &Code, (from, to): (usize, usize)) -> Self {
        let mut m = Measure {
            cyclomatic: 1,
            ..Default::default()
        };
        let mut i = from;
        while i < to {
            if let Some(end) = nested_scope_end(code, i) {
                i = end;
                continue;
            }
            m.close_blocks(i);
            m.halstead(&code.toks[i]);
            if code.after_dot(i) || !m.keyword(code, i) {
                m.operator(code, i);
            }
            if let Some(&kind) = code.kinds.get(&i) {
                if kind.nests() {
                    m.nest();
                }
                m.open.push((code.pairs[&i], kind));
            }
            i += 1;
        }
        m
    }

    /// Leave the blocks that end at `i`.
    fn close_blocks(&mut self, i: usize) {
        while self.open.last().is_some_and(|&(close, _)| close == i) {
            let (_, kind) = self.open.pop().expect("checked");
            if kind.nests() {
                self.depth -= 1;
            }
            self.logic = None;
        }
    }

    /// Control keywords; false for any other word.
    fn keyword(&mut self, code: &Code, i: usize) -> bool {
        let innermost = self.open.last().map(|&(_, kind)| kind);
        match code.text(i) {
            "if" | "unless" | "while" | "until" | "for" | "rescue" => self.decision(true),
            "elsif" => self.decision(false),
            "else" if innermost == Some(Kind::Cond) => self.cognitive += 1,
            "case" => self.branch(),
            "when" => self.cyclomatic += 1,
            "in" if innermost == Some(Kind::Case) => self.cyclomatic += 1,
            _ => return false,
        }
        true
    }

    /// Ternaries, logical operators, brackets and statement ends.
    fn operator(&mut self, code: &Code, i: usize) {
        let continued = matches!(code.text(i.wrapping_sub(1)), "&&" | "||" | "and" | "or");
        match code.text(i) {
            "?" => {
                self.decision(true);
                self.logic = None;
            }
            "&." | "||=" | "&&=" => self.cyclomatic += 1,
            "&&" | "and" => self.logical(true),
            "||" | "or" => self.logical(false),
            "(" | "[" => self.level += 1,
            ")" | "]" => {
                self.level = self.level.saturating_sub(1);
                let level = self.level;
                self.logic = self.logic.filter(|(at, _)| *at <= level);
            }
            "\n" if !continued => self.logic = None,
            ";" | "," => self.logic = None,
            _ => {}
        }
    }

    /// A decision that counts once and adds cognitive load: with nesting for
    /// a new construct, flat for `elsif`.
    fn decision(&mut self, nested: bool) {
        self.cyclomatic += 1;
        self.cognitive += if nested { 1 + self.depth } else { 1 };
        self.max_nesting = self.max_nesting.max(self.depth + 1);
    }

    /// `case`: cognitive load with nesting, but no decision of its own.
    fn branch(&mut self) {
        self.cognitive += 1 + self.depth;
        self.max_nesting = self.max_nesting.max(self.depth + 1);
    }

    fn nest(&mut self) {
        self.depth += 1;
        self.max_nesting = self.max_nesting.max(self.depth);
    }

    fn logical(&mut self, and: bool) {
        self.cyclomatic += 1;
        self.cognitive += match self.logic {
            Some((at, previous)) if at == self.level => u32::from(previous != and),
            _ => 1,
        };
        self.logic = Some((self.level, and));
    }

    fn halstead(&mut self, token: &Token) {
        let text = token.text.as_str();
        if !token.literal
            && matches!(
                text,
                "\n" | "(" | ")" | "[" | "]" | "{" | "}" | ";" | "," | "end"
            )
        {
            return;
        }
        let operand = token.literal
            || matches!(text, "self" | "nil" | "true" | "false")
            || (text.starts_with(|c: char| c.is_alphanumeric() || matches!(c, '_' | '@' | '$'))
                && !KEYWORDS.contains(&text));
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

#[cfg(test)]
mod tests {
    use super::super::symbols::RubyTarget;
    use super::*;

    fn scan(source: &str) -> RubyStructure {
        scan_ruby_structure("app/models/shop.rb", "app/models/shop.rb", source)
    }
    fn method<'a>(found: &'a RubyStructure, name: &str) -> &'a CallMethod {
        found
            .methods
            .iter()
            .find(|m| m.name == name)
            .unwrap_or_else(|| panic!("no method {name}: {:?}", found.methods))
    }
    /// (cyclomatic, max nesting, cognitive) of one method.
    fn complexity(found: &RubyStructure, name: &str) -> (u32, u32, u32) {
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
            r##"class Shop
  def plain
    1
  end

  def branches(a, b)
    if a
      if b then 1 else 2 end
    elsif b
      3
    else
      4
    end
  end

  def logic(a, b, c) = a && b || c

  def arms(x)
    case x
    when 0 then 1
    when 1, 2 then 2
    else 3
    end
  end

  def guarded(items)
    items.each do |item|
      while item
        item -= 1
      end
    end
  rescue ArgumentError
    0
  rescue TypeError => e
    1
  end

  def modifiers(a)
    return 1 if a
    a += 1 unless a.nil?
    a -= 1 while a > 0
    a ? 1 : 2
  end

  def safe(a)
    a&.b || a.c(d: 1)
  end

  def looped(n)
    begin
      n -= 1
    end while n > 0
    for i in 1..n do
      puts i
    end
  end

  def texts
    sql = <<~SQL
      select * from t where a = 1 if b
    SQL
    %w[if unless end].map { |w| "#{w} end" }
  end
end
"##,
        );
        assert!(found.diagnostics.is_empty(), "{:?}", found.diagnostics);
        assert_eq!(complexity(&found, "plain"), (1, 0, 0));
        // Same shape and numbers as the PHP, Rust and Python `branches` cases.
        assert_eq!(complexity(&found, "branches"), (4, 2, 6));
        assert_eq!(complexity(&found, "logic"), (3, 0, 2));
        // Each `when` counts, `else` does not; the case nests once.
        assert_eq!(complexity(&found, "arms"), (3, 1, 1));
        // The block nests the loop; each `rescue` counts like a `catch`.
        assert_eq!(complexity(&found, "guarded"), (4, 2, 4));
        // Modifiers count like their block forms but nest nothing.
        assert_eq!(complexity(&found, "modifiers"), (5, 1, 4));
        // `&.` is a decision without cognitive load.
        assert_eq!(complexity(&found, "safe"), (3, 0, 1));
        // `begin … end while` is one loop; the optional `do` of `for` opens
        // no block.
        assert_eq!(complexity(&found, "looped"), (3, 1, 2));
        // Heredoc and `%w` contents are no code.
        assert_eq!(complexity(&found, "texts"), (1, 1, 0));
        let plain = found
            .metrics
            .iter()
            .find(|m| m.method_id == method(&found, "plain").id)
            .unwrap();
        // `1`: no operator, one operand, one line.
        assert_eq!(
            (plain.operators, plain.operands, plain.source_lines),
            (0, 1, 1)
        );
    }

    #[test]
    fn classes_modules_and_methods_come_from_declarations() {
        let found = scan(
            r#"module Billing
  class Invoice < ApplicationRecord
    include Payable, Auditable
    prepend Tracing
    extend Finder
    TAX = 0.19

    def self.build(attrs) = new(attrs)

    class << self
      def recent; end
    end

    def total=(value)
      @total = value
    end

    def ==(other)
      other.id == id
    end

    def paid?
      !!@paid
    end
  end

  class Payment < Struct.new(:amount)
  end
end

class ::Top
end

def helper
end
"#,
        );
        assert!(found.diagnostics.is_empty(), "{:?}", found.diagnostics);
        let class = |name: &str| {
            found
                .classes
                .iter()
                .find(|c| c.name == name)
                .unwrap_or_else(|| panic!("no class {name}: {:?}", found.classes))
        };
        let kinds: Vec<_> = [
            "Billing",
            "Billing::Invoice",
            "Billing::Payment",
            "Top",
            "(module)",
        ]
        .iter()
        .map(|name| (class(name).kind.as_str(), class(name).is_abstract))
        .collect();
        assert_eq!(
            kinds,
            [
                ("module", None),
                ("class", None),
                ("class", None),
                ("class", None),
                ("module", None),
            ]
        );
        let invoice = class("Billing::Invoice");
        assert_eq!(invoice.id, "app/models/shop.rb::Billing::Invoice");
        assert_eq!(invoice.package, "app/models");
        assert_eq!(invoice.language.as_deref(), Some("ruby"));
        let facts = found
            .symbols
            .classes
            .iter()
            .find(|c| c.name == "Billing::Invoice")
            .unwrap();
        let paths = |list: &[RubyConst]| list.iter().map(|c| c.path.clone()).collect::<Vec<_>>();
        assert_eq!(paths(&facts.includes), ["Payable", "Auditable"]);
        assert_eq!(paths(&facts.prepends), ["Tracing"]);
        assert_eq!(paths(&facts.extends), ["Finder"]);
        match &facts.superclass {
            Superclass::Const(c) => {
                assert_eq!(
                    (c.path.as_str(), c.nesting.clone()),
                    ("ApplicationRecord", vec!["Billing".to_string()])
                );
            }
            other => panic!("{other:?}"),
        }
        let payment = found
            .symbols
            .classes
            .iter()
            .find(|c| c.name == "Billing::Payment")
            .unwrap();
        assert_eq!(payment.superclass, Superclass::Expression);
        let methods: Vec<_> = facts
            .methods
            .iter()
            .map(|m| (m.name.as_str(), m.singleton))
            .collect();
        assert_eq!(
            methods,
            [
                ("build", true),
                ("recent", true),
                ("total=", false),
                ("==", false),
                ("paid?", false),
            ]
        );
        assert_eq!(method(&found, "build").signature, "def self.build(attrs)");
        assert!(method(&found, "build").is_static);
        assert_eq!(method(&found, "total=").signature, "def total=(value)");
        assert_eq!(method(&found, "paid?").end_line, Some(24));
        assert_eq!(
            method(&found, "helper").class_id,
            "app/models/shop.rb::(module)"
        );
        assert_eq!(found.symbols.constants, ["Billing::Invoice::TAX"]);
    }

    #[test]
    fn unbalanced_sources_are_reported_not_measured() {
        let found = scan("class A\n  def x\n    if y\n  end\nend\n");
        assert!(found.methods.is_empty());
        assert_eq!(
            found.diagnostics,
            ["app/models/shop.rb: unbalanced blocks; not measured"]
        );
        assert!(!found.symbols.measured);
    }

    // Call sites and constant references (`super::sites`).

    /// (label, target) of every site in the source.
    fn sites(source: &str) -> Vec<(String, RubyTarget)> {
        let found = scan_ruby_structure("lib/shop.rb", "lib/shop.rb", source);
        assert!(found.diagnostics.is_empty(), "{:?}", found.diagnostics);
        found
            .symbols
            .sites
            .into_iter()
            .map(|site| (site.label, site.target))
            .collect()
    }

    fn constant(path: &str, nesting: &[&str]) -> RubyConst {
        RubyConst {
            path: path.into(),
            absolute: false,
            nesting: nesting.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn call_forms_map_to_targets() {
        let found = sites(
            r#"module Shop
  class Cart
    def checkout(items, coupon: nil)
      total = Billing::Invoice.new(items)
      Tax.rate
      self.audit
      validate(total)
      items.each { |item| item.price }
      super
      send(:hidden)
      repo.public_send(:x)
      coupon
    end
  end
end
"#,
        );
        let nesting = ["Shop", "Shop::Cart"];
        assert_eq!(
            found,
            [
                (
                    "Billing::Invoice.new".to_string(),
                    RubyTarget::Const {
                        constant: constant("Billing::Invoice", &nesting),
                        method: "new".into()
                    }
                ),
                (
                    "Tax.rate".into(),
                    RubyTarget::Const {
                        constant: constant("Tax", &nesting),
                        method: "rate".into()
                    }
                ),
                ("self.audit".into(), RubyTarget::SelfCall("audit".into())),
                ("validate".into(), RubyTarget::SelfCall("validate".into())),
                ("items.each".into(), RubyTarget::Unresolved),
                ("item.price".into(), RubyTarget::Unresolved),
                ("super".into(), RubyTarget::Super("checkout".into())),
                ("send".into(), RubyTarget::Dynamic),
                ("repo".into(), RubyTarget::SelfCall("repo".into())),
                ("repo.public_send".into(), RubyTarget::Dynamic),
            ]
        );
    }

    #[test]
    fn sites_carry_position_and_code() {
        let found = scan_ruby_structure(
            "lib/shop.rb",
            "lib/shop.rb",
            "class Cart\n  def run\n    Billing::Invoice.new(1,\n      2)\n  end\nend\n",
        );
        let site = &found.symbols.sites[0];
        assert_eq!((site.line, site.column), (3, 5));
        assert_eq!(site.code, "Billing::Invoice.new(1, 2)");
        assert_eq!(site.from_class, "lib/shop.rb::Cart");
        assert_eq!(site.from_method.as_deref(), Some("lib/shop.rb::Cart#run@2"));
    }
}
