//! Java import scanner with lexical shielding and multiline statement assembly.
//! Comments and string bodies cannot create imports. This is a bounded static
//! scanner; dynamic loading, preprocessing and runtime resolution remain unsupported.

use crate::extract::{Declaration, Extraction, Extractor, RawImport, TargetSpec};
use crate::walk::Lang;

/// Java scanner (T11). See the [module documentation](self) for the
/// emitted declaration and target shapes.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct JavaExtractor;

impl Extractor for JavaExtractor {
    fn lang(&self) -> Lang {
        Lang::Java
    }

    fn extract(&self, rel_path: &str, content: &str) -> Extraction {
        let mut extraction = Extraction::default();
        for statement in crate::lex::statements(content, crate::lex::Flavor::C) {
            let code = statement.code.trim();
            if let Some(declaration) = package_declaration(rel_path, code) {
                extraction.declarations.push(declaration);
            } else {
                extraction.imports.extend(import_declaration(
                    rel_path,
                    &statement.raw,
                    statement.line,
                    code,
                ));
            }
        }
        extraction
    }
}

/// The [`Declaration`] of a comment-stripped `package a.b.c;` line, `None`
/// for every other line — the dotted package name becomes the module path
/// with dots → slashes.
fn package_declaration(rel_path: &str, code: &str) -> Option<Declaration> {
    let body = statement_body(code, "package")?;
    let segments = dotted_segments(body)?;
    Some(Declaration {
        module_path: segments.join("/"),
        file: rel_path.to_string(),
    })
}

/// The single [`RawImport`] of a comment-stripped `import …;` line, `None`
/// for every other line, for malformed specs and for ignored `java.*` /
/// `javax.*` imports.
fn import_declaration(rel_path: &str, raw: &str, line: u32, code: &str) -> Option<RawImport> {
    let target = import_target(code)?;
    Some(RawImport {
        source: rel_path.to_string(),
        target,
        line,
        raw: raw.to_string(),
    })
}

/// The target of a comment-stripped line, `None` unless the line is a
/// complete `import …;` statement with a well-formed, non-ignored spec.
fn import_target(code: &str) -> Option<TargetSpec> {
    let body = statement_body(code, "import")?;
    let is_static = body
        .strip_prefix("static")
        .is_some_and(|rest| rest.starts_with(char::is_whitespace));
    let body = if is_static {
        body["static".len()..].trim_start()
    } else {
        body
    };
    let segments: Vec<&str> = body.split('.').map(str::trim).collect();
    let (last, init) = segments.split_last()?;
    let wildcard = *last == "*";
    // Everything except a trailing `*` must be a plain identifier.
    let named: &[&str] = if wildcard { init } else { &segments };
    if !named.iter().all(|segment| is_identifier(segment)) {
        return None;
    }
    if matches!(named.first().copied(), Some("java") | Some("javax")) {
        return None;
    }
    // (stem, package path) per form; the path must never be empty (a
    // single-segment or bare-`*` spec names no package).
    let (stem, path): (Option<&str>, &[&str]) = match (wildcard, is_static) {
        // `import a.b.C;` — last segment stems, the rest is the package.
        (false, false) => {
            let (stem, path) = segments.split_last()?;
            (Some(stem), path)
        }
        // `import a.b.*;` — the whole dotted prefix is the module.
        (true, false) => (None, init),
        // `import static a.b.C.m;` / `import static a.b.C.*;` — the
        // trailing member (identifier or `*`) is dropped, `C` stems.
        (_, true) => {
            let (_, class_and_package) = segments.split_last()?;
            let (stem, path) = class_and_package.split_last()?;
            (Some(stem), path)
        }
    };
    if path.is_empty() {
        return None;
    }
    Some(TargetSpec::Symbol {
        path: path.join("/"),
        stem: stem.map(str::to_string),
    })
}

/// The body of a statement line: `Some` only when `code` opens with the
/// whole word `keyword` (followed by whitespace) and a complete
/// `… ;`-terminated statement with non-empty content follows.
fn statement_body<'a>(code: &'a str, keyword: &str) -> Option<&'a str> {
    let rest = code.strip_prefix(keyword)?;
    let body = rest.trim_start();
    if body.is_empty() || body.len() == rest.len() {
        return None;
    }
    let body = body.trim_end().strip_suffix(';')?.trim();
    (!body.is_empty()).then_some(body)
}

/// The segments of a dotted identifier path `a.b.c`, `None` when the
/// string is not a sequence of non-empty identifier segments.
fn dotted_segments(dotted: &str) -> Option<Vec<&str>> {
    if dotted.is_empty() {
        return None;
    }
    dotted
        .split('.')
        .map(|segment| is_identifier(segment).then_some(segment))
        .collect()
}

/// `true` for a plain Java identifier (keywords with punctuation, digits
/// up front and stray characters never match).
fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_alphabetic() || matches!(c, '_' | '$'))
        && chars.all(|c| c.is_alphanumeric() || matches!(c, '_' | '$'))
}

/// Exported/internal symbol split of one file (WP-02).
///
/// Rule (R15): `public` → exported; package-private, `private` and
/// `protected` are internal.
pub fn scan_visibility(content: &str) -> archguard_core::Visibility {
    let clean = crate::lex::code_only(content, crate::lex::Flavor::C);
    let content = clean.as_str();
    let mut visibility = archguard_core::Visibility::default();
    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty()
            || line.starts_with("//")
            || line.starts_with("/*")
            || line.starts_with('*')
            || line.starts_with('@')
        {
            continue;
        }
        // Visibility modifier of the declaration (default: package-private).
        let (body, exported) = if let Some(rest) = strip_word(line, "public") {
            (rest, true)
        } else if let Some(rest) =
            strip_word(line, "private").or_else(|| strip_word(line, "protected"))
        {
            (rest, false)
        } else {
            (line, false)
        };
        // Non-visibility modifiers may precede the type keyword.
        let mut body = body;
        for modifier in [
            "abstract", "final", "static", "sealed", "strictfp", "default",
        ] {
            if let Some(rest) = strip_word(body, modifier) {
                body = rest;
                break;
            }
        }
        for keyword in ["class", "interface", "enum", "record"] {
            if let Some(rest) = strip_word(body, keyword) {
                if let Some(name) = first_identifier(rest) {
                    if exported {
                        visibility.exported.insert(name);
                    } else {
                        visibility.internal.insert(name);
                    }
                }
                break;
            }
        }
    }
    visibility
}

/// The rest of the line after `keyword`, only when it is a whole word.
fn strip_word<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(keyword)?;
    (!rest
        .chars()
        .next()
        .is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '$')))
    .then_some(rest.trim_start())
}

/// First identifier token of `text`, `None` when there is none.
fn first_identifier(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut index = 0usize;
    while index < chars.len() && !(chars[index].is_alphanumeric() || chars[index] == '_') {
        index += 1;
    }
    let start = index;
    while index < chars.len() && (chars[index].is_alphanumeric() || chars[index] == '_') {
        index += 1;
    }
    (index > start).then(|| chars[start..index].iter().collect())
}

#[cfg(test)]
mod visibility_tests {
    use super::*;

    #[test]
    fn public_types_are_exported_package_private_are_internal() {
        let source = "public class Service {}\ninterface Repository {}\npublic final class Final {}\npublic interface Api {}\npublic enum Mode {}\n";
        let visibility = scan_visibility(source);
        assert!(visibility.exported.contains("Service"));
        assert!(visibility.exported.contains("Final"));
        assert!(visibility.exported.contains("Api"));
        assert!(visibility.exported.contains("Mode"));
        assert!(
            visibility.internal.contains("Repository"),
            "package-private"
        );
    }

    #[test]
    fn private_and_protected_types_are_internal() {
        let visibility =
            scan_visibility("private class Helper {}\nprotected abstract class Base {}\n");
        assert!(visibility.internal.contains("Helper"));
        assert!(visibility.internal.contains("Base"));
        assert!(visibility.exported.is_empty());
    }

    #[test]
    fn comments_and_annotations_are_skipped() {
        let visibility = scan_visibility(
            "// public class Ghost {}\n/* class G2 */\n@Override\npublic class Real {}\n",
        );
        assert_eq!(
            visibility
                .exported
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["Real"]
        );
        assert!(visibility.internal.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::{extractor_for, registry};

    /// Fixture covering every T11 case: the `package` declaration, class
    /// imports, static imports (member dropped, class kept), wildcard
    /// imports, ignored `java.*`/`javax.*` forms, line and block comments
    /// (incl. a multi-line block and an unterminated one), a second
    /// package line, keyword lookalikes, incomplete statements, malformed
    /// specs, whitespace around dots and a deep dotted import.
    const FIXTURE: &str = "\
package com.example.app;

import com.example.domain.Order;
import static com.example.util.Calc.sum;
import com.example.util.*;
import java.util.List;
import javax.swing.JFrame;
import static java.lang.Math.PI;
// import com.example.commented.Gone;
/* import com.example.sameline.Gone; */
/*
import com.example.inblock.Gone;
*/
import com.example.after.Block;
import com.example.Deep.Nested.Path.Class;
import javax.annotation.processing.*;
package second.pkg;
imports com.example.lookalike.Gone;
import com.example.incomplete
import static a.b;
import com.example.Ws . Gap ;
public class Main { public static void main(String[] a) {} }
/* unterminated block swallows the tail
import com.example.swallowed.Gone;
";

    /// The expected import for fixture line `line_no` with package `path`
    /// and class `stem` (raw is taken verbatim from the fixture itself).
    fn symbol(line_no: u32, path: &str, stem: Option<&str>) -> RawImport {
        RawImport {
            source: "src/main/java/app/Main.java".to_string(),
            target: TargetSpec::Symbol {
                path: path.to_string(),
                stem: stem.map(str::to_string),
            },
            line: line_no,
            raw: fixture_line(line_no),
        }
    }

    fn fixture_line(line_no: u32) -> String {
        FIXTURE
            .lines()
            .nth((line_no - 1) as usize)
            .expect("fixture line exists")
            .to_string()
    }

    fn extract(content: &str) -> Extraction {
        JavaExtractor.extract("src/main/java/app/Main.java", content)
    }

    fn targets(extraction: &Extraction) -> Vec<String> {
        extraction
            .imports
            .iter()
            .map(|import| match &import.target {
                TargetSpec::Symbol { path, stem } => match stem {
                    Some(stem) => format!("{path}:{stem}"),
                    None => path.clone(),
                },
                TargetSpec::Path(spec) => spec.clone(),
            })
            .collect()
    }

    #[test]
    fn fixture_yields_exactly_the_expected_imports() {
        let extraction = extract(FIXTURE);
        assert_eq!(
            extraction.imports,
            vec![
                symbol(3, "com/example/domain", Some("Order")),
                symbol(4, "com/example/util", Some("Calc")),
                symbol(5, "com/example/util", None),
                symbol(14, "com/example/after", Some("Block")),
                symbol(15, "com/example/Deep/Nested/Path", Some("Class")),
                symbol(21, "com/example/Ws", Some("Gap")),
            ],
            "imports: {:#?}",
            extraction.imports
        );
    }

    #[test]
    fn fixture_yields_exactly_the_expected_declarations() {
        let extraction = extract(FIXTURE);
        assert_eq!(
            extraction.declarations,
            vec![
                Declaration {
                    module_path: "com/example/app".to_string(),
                    file: "src/main/java/app/Main.java".to_string(),
                },
                Declaration {
                    module_path: "second/pkg".to_string(),
                    file: "src/main/java/app/Main.java".to_string(),
                },
            ],
            "declarations: {:#?}",
            extraction.declarations
        );
    }

    /// One T11 import-table row: source line and its expected
    /// `path`/`stem` (or `None` when no import may be emitted).
    type ImportCase = (&'static str, Option<(&'static str, Option<&'static str>)>);

    /// The T11 table: each source line and its expected outcome.
    const IMPORT_TABLE: &[ImportCase] = &[
        ("import a.b.C;", Some(("a/b", Some("C")))),
        ("import a.b.c.D;", Some(("a/b/c", Some("D")))),
        ("import static a.b.C.m;", Some(("a/b", Some("C")))),
        ("import static a.b.C.MY_CONST;", Some(("a/b", Some("C")))),
        ("import a.b.*;", Some(("a/b", None))),
        ("import a.*;", Some(("a", None))),
        ("import static a.b.C.*;", Some(("a/b", Some("C")))),
        ("import java.util.List;", None),
        ("import java.util.*;", None),
        ("import javax.swing.JFrame;", None),
        ("import javax.annotation.processing.*;", None),
        ("import static java.lang.Math.PI;", None),
        ("import javax; ", None),
        ("import javax.a.B;", None),
        ("import java.lang.invoke.MethodHandles;", None),
        ("import com.a.b.C;", Some(("com/a/b", Some("C")))),
        ("import C;", None),
        ("import static a.b;", None),
        ("import static a.b.C;", Some(("a", Some("b")))),
        ("import a.b.C", None),
        ("import a..C;", None),
        ("import a.b.9C;", None),
        ("import a.b.C.;", None),
        ("import a.b.*.;", None),
        ("import *a.b;", None),
        ("import;", None),
        ("import ;", None),
        ("imports a.b.C;", None),
        ("myimport a.b.C;", None),
        ("importx a.b.C;", None),
        ("importcom.example.a.B;", None),
    ];

    #[test]
    fn import_table_yields_exactly_the_expected_targets() {
        for &(source, expected) in IMPORT_TABLE {
            let extraction = extract(&format!("{source}\n"));
            let actual = targets(&extraction);
            let expected: Vec<String> = expected
                .map(|(path, stem)| match stem {
                    Some(stem) => format!("{path}:{stem}"),
                    None => path.to_string(),
                })
                .into_iter()
                .collect();
            assert_eq!(actual, expected, "source {source:?}");
            for import in &extraction.imports {
                assert!(matches!(&import.target, TargetSpec::Symbol { .. }));
                assert_eq!(import.source, "src/main/java/app/Main.java");
            }
        }
    }

    /// Package-declaration table: source line → expected `module_path`
    /// (`None` = no declaration).
    const PACKAGE_TABLE: &[(&str, Option<&str>)] = &[
        ("package a.b.c;", Some("a/b/c")),
        ("package a;", Some("a")),
        ("package com.example.app;", Some("com/example/app")),
        ("package a.b ;", Some("a/b")),
        ("package  a.b ;  ", Some("a/b")),
        ("package a.b", None),
        ("package;", None),
        ("package ;", None),
        ("package a..b;", None),
        ("package a.9b;", None),
        ("package .a;", None),
        ("package a.b.*;", None),
        ("package", None),
        ("package_x a.b;", None),
        ("packagea a.b;", None),
    ];

    #[test]
    fn package_table_yields_exactly_the_expected_declarations() {
        for &(source, expected) in PACKAGE_TABLE {
            let extraction = extract(&format!("{source}\n"));
            let actual: Vec<&str> = extraction
                .declarations
                .iter()
                .map(|declaration| declaration.module_path.as_str())
                .collect();
            let expected: Vec<&str> = expected.into_iter().collect();
            assert_eq!(actual, expected, "source {source:?}");
            for declaration in &extraction.declarations {
                assert_eq!(declaration.file, "src/main/java/app/Main.java");
            }
        }
    }

    #[test]
    fn static_import_drops_the_member_and_keeps_the_class_stem() {
        let extraction = extract("import static a.b.C.m;\n");
        assert_eq!(
            extraction.imports[0].target,
            TargetSpec::Symbol {
                path: "a/b".to_string(),
                stem: Some("C".to_string()),
            }
        );
    }

    #[test]
    fn star_import_targets_the_package_without_a_stem() {
        let extraction = extract("import a.b.*;\n");
        assert_eq!(
            extraction.imports[0].target,
            TargetSpec::Symbol {
                path: "a/b".to_string(),
                stem: None,
            }
        );
    }

    #[test]
    fn package_line_declares_the_files_logical_module() {
        let extraction = extract("package a.b.c;\n");
        assert_eq!(extraction.imports, Vec::new());
        assert_eq!(
            extraction.declarations,
            vec![Declaration {
                module_path: "a/b/c".to_string(),
                file: "src/main/java/app/Main.java".to_string(),
            }]
        );
    }

    #[test]
    fn line_comments_and_block_comment_state_are_skipped() {
        let content = "\
package a.b;
// import a.LineComment;
/* import a.SameLine; */ import a.AfterBlock;
/*
package swallowed.pkg;
import a.InsideBlock;
*/
import a.AfterMultiLineBlock;
import a.BeforeOpen;
/* unterminated block starts here
import a.Swallowed;
";
        let extraction = extract(content);
        assert_eq!(
            targets(&extraction),
            ["a:AfterBlock", "a:AfterMultiLineBlock", "a:BeforeOpen"]
        );
        assert_eq!(extraction.declarations.len(), 1);
        assert_eq!(extraction.declarations[0].module_path, "a/b");
    }

    #[test]
    fn package_and_import_keywords_in_code_are_not_statements() {
        let content = "\
String s = \"package a.b; import a.c.D;\";
int importer = import_like();
";
        let extraction = extract(content);
        assert!(extraction.imports.is_empty());
        assert!(extraction.declarations.is_empty());
    }

    #[test]
    fn empty_and_comment_only_files_yield_nothing() {
        assert_eq!(extract(""), Extraction::default());
        assert_eq!(extract("// nothing\n/* here */\n\n"), Extraction::default());
    }

    #[test]
    fn line_numbers_are_one_based_and_raw_is_verbatim() {
        let extraction = extract("package a.b;\n\nimport a.b.C; // trailing\n");
        assert_eq!(extraction.imports[0].line, 3);
        assert_eq!(extraction.imports[0].raw, "import a.b.C; // trailing");
        assert_eq!(
            extraction.declarations[0].file,
            "src/main/java/app/Main.java"
        );
    }

    #[test]
    fn extractor_reports_the_java_language() {
        assert_eq!(JavaExtractor.lang(), Lang::Java);
    }

    #[test]
    fn java_extractor_is_registered() {
        assert!(registry().iter().any(|e| e.lang() == Lang::Java));
        assert!(matches!(&extractor_for(Lang::Java), Some(e) if e.lang() == Lang::Java));
    }
}
