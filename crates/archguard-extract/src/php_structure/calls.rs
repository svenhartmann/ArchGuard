//! Collects the declarations and call sites of one PHP file while the
//! structure walk passes `namespace`, `use`, class and function tokens.
//!
//! `$this`, `self`, `static` and `parent` inside closures and arrow functions
//! stay unresolved: `Closure::bind` can rebind them. Nested named functions,
//! nested classes and anonymous class bodies are not scanned; their names
//! are listed as hidden so the engine never calls them external.

use std::collections::BTreeMap;

use super::names::{Import, Names};
use super::sites::Sites;
use super::symbols::{PhpCallSite, PhpClassFacts, PhpFunctionFacts, PhpSymbols};
use super::tokens::{Tok, View, class_kind};

const CODE_LIMIT: usize = 200;

pub(super) struct Calls<'a> {
    pub(super) view: View<'a>,
    /// The namespace and imports at the current point of the walk.
    pub(super) names: Names,
    symbols: PhpSymbols,
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl<'a> Calls<'a> {
    pub(super) fn new(source: &'a str, toks: &'a [Tok], pairs: &'a BTreeMap<usize, usize>) -> Self {
        Calls {
            view: View::new(source, toks, pairs),
            names: Names::default(),
            symbols: PhpSymbols {
                measured: true,
                ..Default::default()
            },
        }
    }

    /// The collected symbols, call sites in source order.
    pub(super) fn finish(&mut self) -> PhpSymbols {
        let mut symbols = std::mem::take(&mut self.symbols);
        symbols.sites.sort_by_key(|site| (site.line, site.column));
        symbols
    }

    /// A file-level `use` statement at `at`; returns the index after it.
    pub(super) fn import(&mut self, at: usize) -> usize {
        let end = self.view.find(at, &[";"]).unwrap_or(self.view.toks.len());
        let (kind, mut j) = self.import_kind(at + 1, Import::Class);
        while let Some((name, k)) = self.view.qname(j).filter(|_| j < end) {
            j = if self.view.text(k) == "\\" && self.view.text(k + 1) == "{" {
                self.group(&name, k + 1, kind)
            } else {
                let (alias, k) = self.alias(k, &name);
                self.names.bind(kind, &alias, &name);
                k
            };
            if self.view.text(j) != "," {
                break;
            }
            j += 1;
        }
        end + 1
    }

    /// The members of a group `use A\{B, C as D}` whose `{` is at `open`;
    /// returns the index after the group.
    fn group(&mut self, prefix: &str, open: usize, kind: Import) -> usize {
        let close = self.view.pairs[&open];
        let mut e = open + 1;
        while e < close {
            let (member_kind, start) = self.import_kind(e, kind);
            let Some((member, k)) = self.view.qname(start) else {
                break;
            };
            let (alias, k) = self.alias(k, &member);
            self.names
                .bind(member_kind, &alias, &format!("{prefix}\\{member}"));
            e = k + 1;
        }
        close + 1
    }

    fn import_kind(&self, j: usize, default: Import) -> (Import, usize) {
        if self.view.is(j, "function") {
            (Import::Function, j + 1)
        } else if self.view.is(j, "const") {
            (Import::Const, j + 1)
        } else {
            (default, j)
        }
    }

    fn alias(&self, k: usize, name: &str) -> (String, usize) {
        match self.view.word(k + 1) {
            Some(alias) if self.view.is(k, "as") => (alias.text.clone(), k + 2),
            _ => (name.rsplit('\\').next().unwrap_or(name).into(), k),
        }
    }

    /// A `use` of traits directly in the body of `class_id`; returns the
    /// index after it.
    pub(super) fn trait_use(&mut self, at: usize, class_id: &str) -> usize {
        let end = self
            .view
            .find(at + 1, &[";", "{"])
            .unwrap_or(self.view.toks.len());
        let mut traits = Vec::new();
        let mut j = at + 1;
        while j < end {
            match self.view.qname(j) {
                Some((name, k)) => {
                    traits.push(self.names.class_name(&name));
                    j = k;
                }
                None => j += 1,
            }
        }
        let rules = self.view.text(end) == "{";
        if let Some(facts) = self.facts(class_id) {
            facts.traits.extend(traits);
            facts.trait_rules |= rules;
        }
        if rules {
            self.view.pairs[&end] + 1
        } else {
            end + 1
        }
    }

    fn facts(&mut self, class_id: &str) -> Option<&mut PhpClassFacts> {
        self.symbols
            .classes
            .iter_mut()
            .rev()
            .find(|c| c.id == class_id)
    }

    /// Record a class-like declaration; `header` spans the tokens between
    /// its name and its body.
    pub(super) fn declare_class(
        &mut self,
        id: &str,
        name: &str,
        kind: &str,
        header: (usize, usize),
    ) {
        if self.symbols.classes.iter().any(|c| c.id == id) {
            return;
        }
        let parent = (kind == "class")
            .then(|| (header.0..header.1).find(|&k| self.view.is(k, "extends")))
            .flatten()
            .and_then(|k| self.view.qname(k + 1))
            .map(|(parent, _)| self.names.class_name(&parent));
        self.symbols.classes.push(PhpClassFacts {
            id: id.into(),
            name: self.names.qualify(name),
            kind: kind.into(),
            parent,
            traits: Vec::new(),
            trait_rules: false,
            bodiless: Vec::new(),
        });
    }

    pub(super) fn declare_bodiless(&mut self, class_id: &str, name: &str) {
        let name = name.to_ascii_lowercase();
        if let Some(facts) = self.facts(class_id) {
            facts.bodiless.push(name);
        }
    }

    pub(super) fn declare_function(&mut self, name: &str, class_id: &str, method_id: &str) {
        self.symbols.functions.push(PhpFunctionFacts {
            name: self.names.qualify(name),
            class_id: class_id.into(),
            method_id: method_id.into(),
        });
    }

    /// Call sites in the body tokens `from..to` of `method_id`; `in_class`
    /// when `$this`, `self` and `parent` name its class.
    pub(super) fn body_calls(
        &mut self,
        (from, to): (usize, usize),
        owner: (&str, &str),
        in_class: bool,
    ) {
        let mut closures = Vec::new();
        let mut i = from;
        while i < to {
            if let Some(next) = self.nested(i, to, &mut closures) {
                i = next;
                continue;
            }
            let this = in_class && !closures.iter().any(|&(a, b)| a < i && i < b);
            self.site(i, this, owner.0, Some(owner.1));
            i += 1;
        }
    }

    /// The index after a nested scope at `i` that is not scanned; records
    /// closure and arrow function ranges in `closures`.
    fn nested(&mut self, i: usize, to: usize, closures: &mut Vec<(usize, usize)>) -> Option<usize> {
        let v = &self.view;
        let token = &v.toks[i];
        if token.literal || matches!(v.text(i.wrapping_sub(1)), "->" | "?->" | "::") {
            return None;
        }
        if token.is("function") {
            return self.nested_function(i, to, closures);
        }
        if token.is("fn") && v.text(i + 1) == "(" {
            let close = v.pairs[&(i + 1)];
            if let Some(arrow) = (close + 1..to).find(|&k| v.text(k) == "=>") {
                closures.push((i, v.expression_end(arrow + 1, to)));
            }
            return None;
        }
        if token.is("new") && v.is(i + 1, "class") {
            return Some(v.anonymous_end(i).min(to));
        }
        if class_kind(token).is_some() && v.word(i + 1).is_some() {
            let end = v.find(i + 1, &["{"]).map_or(to, |open| v.pairs[&open] + 1);
            let name = self.names.qualify(&v.toks[i + 1].text);
            self.symbols.hidden_classes.push(name);
            return Some(end);
        }
        None
    }

    /// A named function inside a body is skipped; a closure is recorded.
    fn nested_function(
        &mut self,
        i: usize,
        to: usize,
        closures: &mut Vec<(usize, usize)>,
    ) -> Option<usize> {
        let v = &self.view;
        let named = if v.text(i + 1) == "&" { i + 2 } else { i + 1 };
        if v.word(named).is_some() && v.text(named + 1) == "(" {
            let end = v
                .find(v.pairs[&(named + 1)] + 1, &["{", ";"])
                .map_or(to, |k| {
                    if v.text(k) == "{" {
                        v.pairs[&k] + 1
                    } else {
                        k + 1
                    }
                });
            let name = self.names.qualify(&v.toks[named].text);
            self.symbols.hidden_functions.push(name);
            return Some(end);
        }
        if v.text(named) == "(" {
            let mut at = v.pairs[&named] + 1;
            if v.is(at, "use") && v.text(at + 1) == "(" {
                at = v.pairs[&(at + 1)] + 1;
            }
            if let Some(open) = v.find(at, &["{", ";"]).filter(|&k| v.text(k) == "{") {
                closures.push((i, v.pairs[&open]));
            }
        }
        None
    }

    /// Record the call site that starts at `i`, if any.
    pub(super) fn site(
        &mut self,
        i: usize,
        this: bool,
        from_class: &str,
        from_method: Option<&str>,
    ) {
        let v = &self.view;
        if v.toks[i].literal || v.in_attr[i] {
            return;
        }
        let sites = Sites {
            view: v,
            names: &self.names,
        };
        let Some((target, start, callee_end, end)) = sites.at(i, this) else {
            return;
        };
        let first = &v.toks[start];
        let line_start = v.source[..first.start].rfind('\n').map_or(0, |n| n + 1);
        let code = collapse(&v.source[first.start..v.toks[end].end])
            .chars()
            .take(CODE_LIMIT)
            .collect();
        let label = collapse(&v.source[first.start..v.toks[callee_end - 1].end]);
        self.symbols.sites.push(PhpCallSite {
            from_class: from_class.into(),
            from_method: from_method.map(Into::into),
            target,
            label,
            line: first.line,
            column: (first.start - line_start + 1) as u32,
            code,
        });
    }
}
