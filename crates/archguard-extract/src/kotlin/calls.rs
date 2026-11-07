//! Collects the package, imports, declarations and call sites of one Kotlin
//! file while the structure walk passes them. Names stay as written; the
//! engine binds them across the project.

use super::sites::{Body, Owner};
use super::symbols::{KotlinClassFacts, KotlinFunctionFacts, KotlinImport, KotlinSymbols};
use super::tokens::Code;

/// A class-like declaration as the structure walk found it.
pub(super) struct ClassDecl<'a> {
    pub(super) id: &'a str,
    pub(super) name: &'a str,
    pub(super) kind: &'a str,
    pub(super) outer: Option<&'a str>,
    pub(super) supertypes: Vec<String>,
    pub(super) modifiers: Vec<String>,
    /// The header tokens between the name and the body.
    pub(super) header: (usize, usize),
}

pub(super) struct Calls {
    symbols: KotlinSymbols,
}

impl Calls {
    pub(super) fn new() -> Self {
        Calls {
            symbols: KotlinSymbols {
                measured: true,
                ..Default::default()
            },
        }
    }

    pub(super) fn finish(self) -> KotlinSymbols {
        self.symbols
    }

    pub(super) fn package(&mut self, package: &str) {
        self.symbols.package = package.into();
    }

    /// The `import` statement at `at`; returns the index after it.
    pub(super) fn import(&mut self, code: &Code, at: usize) -> usize {
        let mut segments = Vec::new();
        let (mut alias, mut star) = (None, false);
        let mut j = at + 1;
        while j < code.toks.len() && !code.new_line(j) && code.text(j) != ";" {
            match code.text(j) {
                "*" => star = true,
                "as" if code.is_name(j + 1) => {
                    alias = Some(code.toks[j + 1].text.trim_matches('`').to_string());
                    j += 1;
                }
                "." => {}
                _ if code.is_name(j) => segments.push(code.toks[j].text.trim_matches('`')),
                _ => {}
            }
            j += 1;
        }
        if !segments.is_empty() {
            let name =
                (!star).then(|| alias.unwrap_or_else(|| segments.last().unwrap().to_string()));
            self.symbols.imports.push(KotlinImport {
                path: segments.join("."),
                name,
            });
        }
        j
    }

    /// The property declared by `val`/`var` at `at` in `owner`, or at the
    /// top level.
    pub(super) fn property(&mut self, code: &Code, at: usize, owner: Option<&str>) {
        let mut j = at + 1;
        if code.text(j) == "<" {
            while j < code.toks.len() && code.text(j) != ">" {
                j += 1;
            }
            j += 1;
        }
        if !code.is_name(j) {
            return;
        }
        while code.text(j + 1) == "." && code.is_name(j + 2) {
            j += 2;
        }
        let name = code.toks[j].text.trim_matches('`').to_string();
        self.value(owner, name);
    }

    /// A name whose call is no function the resolver could name.
    pub(super) fn value(&mut self, owner: Option<&str>, name: String) {
        match owner.and_then(|id| self.facts(id)) {
            Some(facts) => facts.values.push(name),
            None if owner.is_none() => self.symbols.values.push(name),
            None => {}
        }
    }

    fn facts(&mut self, class_id: &str) -> Option<&mut KotlinClassFacts> {
        self.symbols
            .classes
            .iter_mut()
            .rev()
            .find(|c| c.id == class_id)
    }

    pub(super) fn declare_class(&mut self, code: &Code, decl: ClassDecl) {
        if self.symbols.classes.iter().any(|c| c.id == decl.id) {
            return;
        }
        self.symbols.classes.push(KotlinClassFacts {
            id: decl.id.into(),
            name: decl.name.into(),
            kind: decl.kind.into(),
            outer: decl.outer.map(Into::into),
            companion: decl.modifiers.iter().any(|m| m == "companion"),
            supertypes: decl.supertypes,
            modifiers: decl.modifiers,
            bodiless: Vec::new(),
            values: constructor_properties(code, decl.header),
        });
    }

    pub(super) fn declare_bodiless(&mut self, class_id: &str, name: &str) {
        if let Some(facts) = self.facts(class_id) {
            facts.bodiless.push(name.into());
        }
    }

    pub(super) fn declare_function(&mut self, name: &str, method_id: &str, extension: bool) {
        self.symbols.functions.push(KotlinFunctionFacts {
            name: name.into(),
            method_id: method_id.into(),
            extension,
        });
    }

    /// Call sites in the body tokens `from..to` of a function whose
    /// parameter list spans `params`.
    pub(super) fn body_calls(
        &mut self,
        code: &Code,
        source: &str,
        (from, to): (usize, usize),
        params: (usize, usize),
        owner: Owner<'_>,
    ) {
        let sites = Body::new(code, source, (from, to), params).sites(owner);
        self.symbols.sites.extend(sites);
    }
}

/// Names after `val`/`var` in the primary constructor of a class header.
fn constructor_properties(code: &Code, (from, to): (usize, usize)) -> Vec<String> {
    let mut angle = 0usize;
    for j in from..to {
        match code.text(j) {
            "<" => angle += 1,
            ">" => angle = angle.saturating_sub(1),
            ":" | "{" if angle == 0 => break,
            "(" if angle == 0 => {
                let close = code.pairs[&j];
                return (j + 1..close)
                    .filter(|&k| matches!(code.text(k), "val" | "var") && code.is_name(k + 1))
                    .map(|k| code.toks[k + 1].text.trim_matches('`').to_string())
                    .collect();
            }
            _ => {}
        }
    }
    Vec::new()
}
