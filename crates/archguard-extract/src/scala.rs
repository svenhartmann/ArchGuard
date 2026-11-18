//! Scala import scanner with lexical shielding (Scala 2 and 3 syntax, ADR
//! 0020). Scala shares the JVM package model with Java and Kotlin: the
//! `package` clauses of a file, chained or nested, declare one module
//! (`package a.b` + `package c` → `a/b/c`), and imports resolve against
//! Java, Kotlin and Scala files alike. An import ends at a line break outside
//! braces; selectors (`{C, D => E}`, `{C as D}`), wildcards (`_`, `*`) and
//! given imports are read. Comments and string bodies cannot create imports.
//! Relative imports of Scala 2 (`import sibling.X` inside `package a`) and
//! imports of values (`import order._`) stay external or unresolved.

mod clauses;
mod sites;
mod structure;
mod symbols;
mod tokens;

use std::collections::BTreeSet;

pub use structure::{ScalaStructure, scan_scala_structure};
pub use symbols::{ScalaCallSite, ScalaClassFacts, ScalaImport, ScalaSymbols, ScalaTarget};

use crate::extract::{Declaration, Extraction, Extractor, RawImport, TargetSpec};
use crate::lex::Flavor;
use crate::walk::Lang;
use clauses::{clauses, dotted, import_targets};

/// Scala scanner. See the [module documentation](self).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ScalaExtractor;

impl Extractor for ScalaExtractor {
    fn lang(&self) -> Lang {
        Lang::Scala
    }

    fn extract(&self, rel_path: &str, content: &str) -> Extraction {
        let mut extraction = Extraction::default();
        let lines: Vec<&str> = content.lines().collect();
        let raw = |line: u32| {
            lines
                .get(line as usize - 1)
                .copied()
                .unwrap_or("")
                .to_string()
        };
        let mut header = BTreeSet::new();
        let mut package = Vec::new();
        for clause in clauses(content) {
            header.extend(clause.lines.clone());
            let words: Vec<&str> = clause.words.iter().map(String::as_str).collect();
            match words.split_first() {
                Some((&"package", rest)) if rest.first() != Some(&"object") => {
                    package.extend(dotted(rest));
                }
                Some((&"import", rest)) => {
                    for target in import_targets(rest) {
                        extraction.imports.push(RawImport {
                            source: rel_path.to_string(),
                            target,
                            line: *clause.lines.start(),
                            raw: raw(*clause.lines.start()),
                        });
                    }
                }
                _ => {}
            }
        }
        if !package.is_empty() {
            extraction.declarations.push(Declaration {
                module_path: package.join("/"),
                file: rel_path.to_string(),
            });
        }
        extraction.imports.extend(
            crate::lex::dotted_chains(content, Flavor::Kotlin, &[])
                .into_iter()
                .filter(|(line, segments)| {
                    !header.contains(line) && crate::java::is_qualified_type(segments)
                })
                .map(|(line, segments)| RawImport {
                    source: rel_path.to_string(),
                    target: TargetSpec::Qualified(segments),
                    line,
                    raw: raw(line),
                }),
        );
        extraction
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::{extractor_for, registry};
    use clauses::bindings;

    fn imports(content: &str) -> Vec<(u32, TargetSpec)> {
        ScalaExtractor
            .extract("app/Main.scala", content)
            .imports
            .into_iter()
            .map(|import| (import.line, import.target))
            .collect()
    }

    fn symbol(path: &str, stem: Option<&str>) -> TargetSpec {
        TargetSpec::Symbol {
            path: path.into(),
            stem: stem.map(str::to_string),
        }
    }

    fn module(content: &str) -> Vec<String> {
        ScalaExtractor
            .extract("src/Main.scala", content)
            .declarations
            .into_iter()
            .map(|d| d.module_path)
            .collect()
    }

    #[test]
    fn package_clauses_declare_one_module() {
        assert_eq!(
            module("package com.shop.order\n\nclass Main\n"),
            ["com/shop/order"]
        );
        // Chained clauses add up, as do nested packages with braces.
        assert_eq!(
            module("package com.shop\npackage order;\nclass Main\n"),
            ["com/shop/order"]
        );
        assert_eq!(
            module("package com.shop {\n  package order {\n    class Main\n  }\n}\n"),
            ["com/shop/order"]
        );
        // A package object adds nothing; no clause declares nothing.
        assert_eq!(
            module("package com.shop\npackage object order { }\n"),
            ["com/shop"]
        );
        assert!(module("object Main\n").is_empty());
    }

    #[test]
    fn import_forms_map_to_symbols() {
        let source = "package app\n\
            import com.shop.order.Order\n\
            import com.shop.billing._\n\
            import com.shop.billing2.*\n\
            import com.shop.pay.{Payment, Refund => R, Hidden => _, Card as C}\n\
            import com.shop.pay.Payment.Status\n\
            import com.shop.util.formatDate, com.shop.util.Clock\n\
            import com.shop.given\n\
            import com.shop.inst.{given, Codec}\n\
            import com.shop.multi.{\n  Alpha,\n  Beta\n}\n";
        assert_eq!(
            imports(source),
            vec![
                (2, symbol("com/shop/order", Some("Order"))),
                (3, symbol("com/shop/billing", None)),
                (4, symbol("com/shop/billing2", None)),
                (5, symbol("com/shop/pay", Some("Payment"))),
                (5, symbol("com/shop/pay", Some("Refund"))),
                (5, symbol("com/shop/pay", Some("Card"))),
                // As in Java, the pipeline walks a nested type up to the
                // file of its outer class.
                (6, symbol("com/shop/pay/Payment", Some("Status"))),
                (7, symbol("com/shop/util", Some("formatDate"))),
                (7, symbol("com/shop/util", Some("Clock"))),
                (8, symbol("com/shop", None)),
                (9, symbol("com/shop/inst", None)),
                (9, symbol("com/shop/inst", Some("Codec"))),
                (10, symbol("com/shop/multi", Some("Alpha"))),
                (10, symbol("com/shop/multi", Some("Beta"))),
            ]
        );
    }

    #[test]
    fn imports_bind_names_for_the_resolver() {
        let found = bindings(
            "import com.shop.{Order, Refund => R, Card as C, Hidden => _}\n\
             import com.shop.billing._\nimport scala.util.Try\n",
        );
        let bound: Vec<_> = found
            .iter()
            .map(|i| (i.path.as_str(), i.name.as_deref()))
            .collect();
        assert_eq!(
            bound,
            [
                ("com.shop.Order", Some("Order")),
                ("com.shop.Refund", Some("R")),
                ("com.shop.Card", Some("C")),
                ("com.shop.billing", None),
                ("scala.util.Try", Some("Try")),
            ]
        );
    }

    #[test]
    fn platform_imports_are_ignored() {
        let source = "import scala.collection.mutable\nimport scala.concurrent._\n\
            import java.util.UUID\nimport javax.inject.Inject\n";
        assert_eq!(imports(source), vec![]);
    }

    #[test]
    fn comments_and_strings_never_create_imports() {
        let source = "/* outer /* nested */\nimport com.fake.Hidden\n*/\n\
            // import com.fake.Line\n\
            val text = \"\"\"\nimport com.fake.Raw\n\"\"\"\n\
            val s = s\"${\"import com.fake.Template\"}\"\n\
            import com.shop.order.Order\n";
        assert_eq!(
            imports(source),
            vec![(9, symbol("com/shop/order", Some("Order")))]
        );
    }

    #[test]
    fn local_imports_and_qualified_names_count() {
        let source = "package app\n\
            import com.shop.order.Order\n\
            class Checkout(repo: com.shop.store.Repository):\n\
            \x20 def total(order: Order) =\n\
            \x20   import com.shop.tax.Rates\n\
            \x20   order.items.size\n";
        let found = imports(source);
        assert!(found.contains(&(
            3,
            TargetSpec::Qualified(vec![
                "com".into(),
                "shop".into(),
                "store".into(),
                "Repository".into(),
            ])
        )));
        assert!(found.contains(&(5, symbol("com/shop/tax", Some("Rates")))));
        // `order.items.size` is a member chain, not a type.
        assert_eq!(found.len(), 3, "{found:?}");
    }

    #[test]
    fn scala_is_registered_for_scala_files() {
        assert_eq!(Lang::from_extension("scala"), Some(Lang::Scala));
        assert!(registry().iter().any(|e| e.lang() == Lang::Scala));
        assert!(matches!(&extractor_for(Lang::Scala), Some(e) if e.lang() == Lang::Scala));
    }
}
