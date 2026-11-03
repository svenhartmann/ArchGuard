//! Kotlin import scanner with lexical shielding. Kotlin shares the JVM package
//! model with Java, so `package a.b` declares the module `a/b` and imports
//! resolve against Java and Kotlin files alike. Statements end at line breaks;
//! comments (nested block comments included) and string bodies cannot create
//! imports. Backtick-quoted names, top-level functions without a file of the
//! same stem and runtime-loaded classes remain unresolved or module-level.

use std::collections::BTreeSet;

use crate::extract::{Declaration, Extraction, Extractor, RawImport, TargetSpec};
use crate::lex::Flavor;
use crate::walk::Lang;

/// Kotlin scanner. See the [module documentation](self).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct KotlinExtractor;

impl Extractor for KotlinExtractor {
    fn lang(&self) -> Lang {
        Lang::Kotlin
    }

    fn extract(&self, rel_path: &str, content: &str) -> Extraction {
        let mut extraction = Extraction::default();
        let source_lines: Vec<&str> = content.lines().collect();
        let raw = |line: u32| {
            source_lines
                .get(line as usize - 1)
                .copied()
                .unwrap_or("")
                .to_string()
        };
        let mut header = BTreeSet::new();
        for (line, words) in code_lines(content) {
            let words: Vec<&str> = words.iter().map(String::as_str).collect();
            match words.split_first() {
                Some((&"package", rest)) => {
                    header.insert(line);
                    if let Some((segments, false)) = dotted(rest) {
                        extraction.declarations.push(Declaration {
                            module_path: segments.join("/"),
                            file: rel_path.to_string(),
                        });
                    }
                }
                Some((&"import", rest)) => {
                    header.insert(line);
                    if let Some(target) = import_target(rest) {
                        extraction.imports.push(RawImport {
                            source: rel_path.to_string(),
                            target,
                            line,
                            raw: raw(line),
                        });
                    }
                }
                _ => {}
            }
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

/// The non-literal tokens of each source line that has any; comments and
/// string bodies (multiline raw strings included) never reach this list.
fn code_lines(content: &str) -> Vec<(u32, Vec<String>)> {
    let tokens = crate::lex::tokens(content, Flavor::Kotlin);
    let mut lines: Vec<(u32, Vec<String>)> = Vec::new();
    let mut fresh = true;
    for token in &tokens {
        if token.text == "\n" {
            fresh = true;
            continue;
        }
        if fresh {
            lines.push((token.line, Vec::new()));
            fresh = false;
        }
        if !token.literal {
            lines
                .last_mut()
                .expect("pushed above")
                .1
                .push(token.text.clone());
        }
    }
    lines
}

/// `a.b.c` or `a.b.*` from statement words, with the wildcard flag. A
/// trailing `as Alias` or `;` ends the name; anything else is malformed.
fn dotted<'a>(words: &[&'a str]) -> Option<(Vec<&'a str>, bool)> {
    let mut segments = Vec::new();
    let mut wildcard = false;
    let mut rest = words.iter();
    loop {
        match rest.next() {
            Some(&"*") if !segments.is_empty() => wildcard = true,
            Some(word) if is_identifier(word) => segments.push(*word),
            _ => return None,
        }
        match rest.next() {
            Some(&".") if !wildcard => {}
            None | Some(&";") => break,
            Some(&"as") if !wildcard => break,
            _ => return None,
        }
    }
    Some((segments, wildcard))
}

/// The target of an `import` statement, `None` for malformed specs and the
/// Kotlin and Java platform packages.
fn import_target(words: &[&str]) -> Option<TargetSpec> {
    let (segments, wildcard) = dotted(words)?;
    if matches!(segments[0], "kotlin" | "kotlinx" | "java" | "javax") {
        return None;
    }
    let (stem, path) = if wildcard {
        (None, &segments[..])
    } else {
        let (stem, path) = segments.split_last()?;
        (Some(stem.to_string()), path)
    };
    if path.is_empty() {
        return None;
    }
    Some(TargetSpec::Symbol {
        path: path.join("/"),
        stem,
    })
}

fn is_identifier(word: &str) -> bool {
    word.starts_with(|c: char| c.is_alphabetic() || c == '_')
        && word.chars().all(|c| c.is_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::{extractor_for, registry};

    fn imports(content: &str) -> Vec<(u32, TargetSpec)> {
        KotlinExtractor
            .extract("app/Main.kt", content)
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

    #[test]
    fn package_without_semicolon_declares_the_module() {
        let extraction =
            KotlinExtractor.extract("src/Main.kt", "package com.shop.order\n\nclass Main\n");
        assert_eq!(
            extraction.declarations,
            vec![Declaration {
                module_path: "com/shop/order".into(),
                file: "src/Main.kt".into(),
            }]
        );
    }

    #[test]
    fn import_forms_map_to_symbols() {
        let source = "package app\n\
            import com.shop.order.Order\n\
            import com.shop.billing.*\n\
            import com.shop.pay.Payment as Pay\n\
            import com.shop.pay.Payment.Status\n\
            import com.shop.util.formatDate;\n";
        assert_eq!(
            imports(source),
            vec![
                (2, symbol("com/shop/order", Some("Order"))),
                (3, symbol("com/shop/billing", None)),
                (4, symbol("com/shop/pay", Some("Payment"))),
                // As in Java, the pipeline walks a nested type up to the
                // file of its outer class.
                (5, symbol("com/shop/pay/Payment", Some("Status"))),
                // A top-level function has no file of its stem; resolution
                // falls back to the package module, never a guessed file.
                (6, symbol("com/shop/util", Some("formatDate"))),
            ]
        );
    }

    #[test]
    fn platform_imports_are_ignored() {
        let source = "import kotlin.math.max\nimport kotlinx.coroutines.flow.Flow\n\
            import java.util.UUID\nimport javax.inject.Inject\n";
        assert_eq!(imports(source), vec![]);
    }

    #[test]
    fn comments_and_strings_never_create_imports() {
        let source = "/* outer /* nested */\nimport com.fake.Hidden\n*/\n\
            // import com.fake.Line\n\
            val text = \"\"\"\nimport com.fake.Raw\n\"\"\"\n\
            import com.shop.order.Order\n";
        assert_eq!(
            imports(source),
            vec![(8, symbol("com/shop/order", Some("Order")))]
        );
    }

    #[test]
    fn qualified_names_in_code_count_outside_the_header() {
        let source = "package app\n\
            import com.shop.order.Order\n\
            class Checkout(val repo: com.shop.store.Repository) {\n\
                fun total(order: Order) = order.items.size\n\
            }\n";
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
        // The import line is not reported a second time as a qualified name,
        // and `order.items.size` is a member chain, not a type.
        assert_eq!(found.len(), 2, "{found:?}");
    }

    #[test]
    fn kotlin_is_registered_for_kt() {
        assert_eq!(Lang::from_extension("kt"), Some(Lang::Kotlin));
        assert!(registry().iter().any(|e| e.lang() == Lang::Kotlin));
        assert!(matches!(&extractor_for(Lang::Kotlin), Some(e) if e.lang() == Lang::Kotlin));
    }
}
