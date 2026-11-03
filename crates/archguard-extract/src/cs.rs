//! C# import scanner with lexical shielding and multiline statement assembly.
//! Comments and string bodies cannot create imports. This is a bounded static
//! scanner; dynamic loading, preprocessing and runtime resolution remain unsupported.

use crate::extract::{Declaration, Extraction, Extractor, RawImport, TargetSpec};
use crate::walk::Lang;

/// C# scanner (T42). See the [module documentation](self) for the emitted
/// declaration and target shapes.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CsExtractor;

impl Extractor for CsExtractor {
    fn lang(&self) -> Lang {
        Lang::CSharp
    }

    fn extract(&self, rel_path: &str, content: &str) -> Extraction {
        let mut extraction = Extraction::default();
        let mut depth = 0usize;
        let mut namespaces: Vec<(usize, String)> = Vec::new();
        let mut file_namespace = String::new();
        for statement in crate::lex::statements(content, crate::lex::Flavor::C) {
            let code = statement.code.trim();
            if code == "}" {
                depth = depth.saturating_sub(1);
                namespaces.retain(|(at, _)| *at <= depth);
                continue;
            }
            if let Some(mut declaration) = namespace_declaration(rel_path, code) {
                let prefix = namespaces
                    .last()
                    .map(|(_, name)| name.as_str())
                    .unwrap_or(&file_namespace);
                if !prefix.is_empty() {
                    declaration.module_path = format!("{prefix}/{}", declaration.module_path);
                }
                if code.ends_with('{') {
                    depth += 1;
                    namespaces.push((depth, declaration.module_path.clone()));
                } else {
                    file_namespace = declaration.module_path.clone();
                }
                extraction.declarations.push(declaration);
                continue;
            }
            extraction
                .imports
                .extend(using_import(rel_path, &statement.raw, statement.line, code));
            if code.ends_with('{') {
                depth += 1;
            }
        }
        extraction
    }
}

/// The [`Declaration`] of a comment-stripped `namespace A.B;` (file-scoped)
/// or `namespace A.B {` (block-style) line, `None` for every other line —
/// the dotted namespace name becomes the module path with dots → slashes.
fn namespace_declaration(rel_path: &str, code: &str) -> Option<Declaration> {
    let body = strip_word(code, "namespace")?;
    let body = body.strip_suffix(';').or_else(|| body.strip_suffix('{'))?;
    let segments = dotted_segments(body)?;
    Some(Declaration {
        module_path: segments.join("/"),
        file: rel_path.to_string(),
    })
}

/// The single [`RawImport`] of a comment-stripped `using …;` directive
/// line, `None` for every other line and for ignored `System.*` imports.
fn using_import(rel_path: &str, raw: &str, line: u32, code: &str) -> Option<RawImport> {
    let target = using_target(code)?;
    Some(RawImport {
        source: rel_path.to_string(),
        target,
        line,
        raw: raw.to_string(),
    })
}

/// The target of a comment-stripped, trimmed line, `None` unless the line
/// is a complete `using …;` *directive* (no `(`, `;`-terminated — which
/// rules out the `using (var x = …)` resource-block statement) with a
/// well-formed, non-ignored spec.
fn using_target(code: &str) -> Option<TargetSpec> {
    if code.contains('(') {
        return None;
    }
    let body = code.strip_suffix(';')?.trim();
    let body = strip_word(body, "global").unwrap_or(body);
    let body = strip_word(body, "using")?;
    if body.is_empty() {
        return None;
    }
    // `using static A.B.C;` — last segment stems, the rest is the namespace.
    if let Some(spec) = strip_word(body, "static") {
        let segments = dotted_segments(spec)?;
        if matches!(segments.first().copied(), Some("System")) {
            return None;
        }
        let (stem, path) = segments.split_last()?;
        return symbol(path, Some(stem));
    }
    // `using X = A.B.C;` — the alias name is dropped, the spec stems.
    if let Some((alias, spec)) = body.split_once('=') {
        if !is_identifier(alias.trim().trim_start_matches('@')) {
            return None;
        }
        let segments = dotted_segments(spec.trim())?;
        if matches!(segments.first().copied(), Some("System")) {
            return None;
        }
        let (stem, path) = segments.split_last()?;
        return symbol(path, Some(stem));
    }
    // A bare `using static;` names nothing and must not fall through to
    // the plain form.
    if body == "static" {
        return None;
    }
    // `using A.B;` — the whole dotted namespace is the module.
    let segments = dotted_segments(body)?;
    if matches!(segments.first().copied(), Some("System")) {
        return None;
    }
    symbol(&segments, None)
}

/// A symbolic target from namespace `segments` and optional stem, `None`
/// when the path would be empty.
fn symbol(segments: &[&str], stem: Option<&str>) -> Option<TargetSpec> {
    if segments.is_empty() {
        return None;
    }
    Some(TargetSpec::Symbol {
        path: segments.join("/"),
        stem: stem.map(str::to_string),
    })
}

/// The remainder of `code` after the whole word `word` (followed by
/// whitespace) and its trailing whitespace, `None` when `code` does not
/// open with that word plus a non-empty remainder.
fn strip_word<'a>(code: &'a str, word: &str) -> Option<&'a str> {
    let rest = code.strip_prefix(word)?;
    let trimmed = rest.trim_start();
    (!trimmed.is_empty() && trimmed.len() != rest.len()).then_some(trimmed)
}

/// The segments of a dotted namespace path `A.B.C` (whitespace around the
/// dots is tolerated, as C# itself allows), `None` when the string is not a
/// sequence of non-empty identifier segments.
fn dotted_segments(dotted: &str) -> Option<Vec<&str>> {
    let dotted = dotted.strip_prefix("global::").unwrap_or(dotted);
    if dotted.is_empty() {
        return None;
    }
    dotted
        .split('.')
        .map(str::trim)
        .map(|segment| {
            let segment = segment.trim_start_matches('@');
            is_identifier(segment).then_some(segment)
        })
        .collect()
}

/// `true` for a plain C# identifier (keywords with punctuation, digits up
/// front, `::` qualifiers and stray characters never match).
fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// Exported/internal symbol split of one file (WP-02).
///
/// Rule (R16): `public` → exported, everything else is internal.
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
            || line.starts_with('[')
        {
            continue;
        }
        // C# type-level default without a modifier is `internal`.
        let (mut body, exported) = if let Some(rest) = strip_word(line, "public") {
            (rest, true)
        } else {
            (line, false)
        };
        for modifier in [
            "private",
            "internal",
            "protected",
            "static",
            "abstract",
            "sealed",
            "partial",
            "readonly",
            "unsafe",
            "extern",
            "new",
            "file",
        ] {
            while let Some(rest) = strip_word(body, modifier) {
                body = rest;
            }
        }
        for keyword in ["class", "interface", "struct", "enum", "record"] {
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
    fn public_types_are_exported_the_rest_is_internal() {
        let source = "public class Service {}\ninternal class Helper {}\nclass Bare {}\npublic sealed class Locked {}\npublic interface IRepository {}\npublic static class Ext {}\n";
        let visibility = scan_visibility(source);
        assert!(visibility.exported.contains("Service"));
        assert!(visibility.exported.contains("Locked"));
        assert!(visibility.exported.contains("IRepository"));
        assert!(visibility.exported.contains("Ext"));
        assert!(visibility.internal.contains("Helper"));
        assert!(
            visibility.internal.contains("Bare"),
            "default is internal in C#"
        );
    }

    #[test]
    fn record_forms_are_covered() {
        let visibility = scan_visibility("public record Point(int X, int Y);\n");
        assert!(visibility.exported.contains("Point"));
    }

    #[test]
    fn comments_and_attributes_are_skipped() {
        let visibility =
            scan_visibility("// public class Ghost\n[Serializable]\npublic class Real\n");
        assert_eq!(visibility.exported.len(), 1);
        assert!(visibility.exported.contains("Real"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::{extractor_for, registry};

    /// Fixture covering every T42 case: file-scoped and block-style
    /// namespaces, usings outside and inside a namespace block, `global
    /// using`, `using static`, alias `using`, ignored `System.*` forms,
    /// line and block comments (incl. a multi-line block and an
    /// unterminated one), a `using (var …)` resource-block statement and
    /// a keyword lookalike.
    const FIXTURE: &str = "\
using System;
using System.Collections.Generic;
using App.Shared.Utils;
global using App.Global.Thing;
namespace App.Domain.Models;

// using App.Commented;
using App.Domain.Entities;
/* using App.Sameline; */ using App.After.Block;
/*
using App.Inside.Block;
*/
using static App.Domain.Calc;
using Repo = App.Infrastructure.Repository;
using (var scope = new Scope())
{
    var x = scope.Value();
}
using App.Domain.Services;
namespace App.Nested {

    using App.Nested.Inner;

    internal class Widget { }

}
/* unterminated block swallows the tail
using App.Swallowed;
";

    /// The expected import for fixture line `line_no` with namespace `path`
    /// and type `stem` (raw is taken verbatim from the fixture itself).
    fn symbol_import(line_no: u32, path: &str, stem: Option<&str>) -> RawImport {
        RawImport {
            source: "src/App/Domain/Widget.cs".to_string(),
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
        CsExtractor.extract("src/App/Domain/Widget.cs", content)
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
                symbol_import(3, "App/Shared/Utils", None),
                symbol_import(4, "App/Global/Thing", None),
                symbol_import(8, "App/Domain/Entities", None),
                symbol_import(9, "App/After/Block", None),
                symbol_import(13, "App/Domain", Some("Calc")),
                symbol_import(14, "App/Infrastructure", Some("Repository")),
                symbol_import(19, "App/Domain/Services", None),
                symbol_import(22, "App/Nested/Inner", None),
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
                    module_path: "App/Domain/Models".to_string(),
                    file: "src/App/Domain/Widget.cs".to_string(),
                },
                Declaration {
                    module_path: "App/Domain/Models/App/Nested".to_string(),
                    file: "src/App/Domain/Widget.cs".to_string(),
                },
            ],
            "declarations: {:#?}",
            extraction.declarations
        );
    }

    /// One T42 import-table row: source line and its expected
    /// `path`/`stem` (or `None` when no import may be emitted).
    type ImportCase = (&'static str, Option<(&'static str, Option<&'static str>)>);

    /// The T42 table: each source line and its expected outcome.
    const USING_TABLE: &[ImportCase] = &[
        ("using A.B;", Some(("A/B", None))),
        ("using A;", Some(("A", None))),
        ("using A.B.C;", Some(("A/B/C", None))),
        ("using A . B ;", Some(("A/B", None))),
        ("global using A.B;", Some(("A/B", None))),
        ("global  using  A.B ;", Some(("A/B", None))),
        ("using static A.B.C;", Some(("A/B", Some("C")))),
        ("using static A.B;", Some(("A", Some("B")))),
        ("global using static A.B.C;", Some(("A/B", Some("C")))),
        ("using static A.B.C.m;", Some(("A/B/C", Some("m")))),
        ("using static A;", None),
        ("using static;", None),
        ("using X = A.B.C;", Some(("A/B", Some("C")))),
        ("using X = A.B;", Some(("A", Some("B")))),
        ("using X= A.B.C;", Some(("A/B", Some("C")))),
        ("global using X = A.B.C;", Some(("A/B", Some("C")))),
        ("using X = A;", None),
        ("using = A.B.C;", None),
        ("using A.B.C = D.E.F;", None),
        ("using X = A.B = C;", None),
        ("using X == A.B;", None),
        ("using System;", None),
        ("using System.Collections.Generic;", None),
        ("global using System.Threading.Tasks;", None),
        ("using static System.Math;", None),
        ("using X = System.Text.Encoding;", None),
        ("using SystemX.A;", Some(("SystemX/A", None))),
        ("using system.a.B;", Some(("system/a/B", None))),
        ("using A.B", None),
        ("using (var x = new Foo());", None),
        ("using (var db = new Db()) {", None),
        ("using var handle = Open();", None),
        ("using A.B();", None),
        ("using global::A.B;", Some(("A/B", None))),
        ("using A.B.*;", None),
        ("using A..B;", None),
        ("using A.9B;", None),
        ("using @X = A.B.C;", Some(("A/B", Some("C")))),
        ("using ;", None),
        ("using;", None),
        (";", None),
        ("usingx A.B;", None),
        ("globalx using A.B;", None),
        ("usingusing A.B;", None),
        ("globalusing A.B;", None),
    ];

    #[test]
    fn using_table_yields_exactly_the_expected_targets() {
        for &(source, expected) in USING_TABLE {
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
                assert_eq!(import.source, "src/App/Domain/Widget.cs");
            }
        }
    }

    /// Namespace-declaration table: source line → expected `module_path`
    /// (`None` = no declaration).
    const NAMESPACE_TABLE: &[(&str, Option<&str>)] = &[
        ("namespace A.B;", Some("A/B")),
        ("namespace A;", Some("A")),
        ("namespace A.B.C;", Some("A/B/C")),
        ("namespace A.B {", Some("A/B")),
        ("namespace A.B{", Some("A/B")),
        ("namespace A.B.C {", Some("A/B/C")),
        ("namespace  A.B  ;  ", Some("A/B")),
        ("namespace A . B ;", Some("A/B")),
        ("namespace A.B", None),
        ("namespace A.B }", None),
        ("namespace;", None),
        ("namespace ;", None),
        ("namespace A..B;", None),
        ("namespace A.9B;", None),
        ("namespace A.B.*;", None),
        ("namespace", None),
        ("namespace_x A.B;", None),
        ("namespacea A.B;", None),
        ("namespace System.X;", Some("System/X")),
    ];

    #[test]
    fn namespace_table_yields_exactly_the_expected_declarations() {
        for &(source, expected) in NAMESPACE_TABLE {
            let extraction = extract(&format!("{source}\n"));
            let actual: Vec<&str> = extraction
                .declarations
                .iter()
                .map(|declaration| declaration.module_path.as_str())
                .collect();
            let expected: Vec<&str> = expected.into_iter().collect();
            assert_eq!(actual, expected, "source {source:?}");
            for declaration in &extraction.declarations {
                assert_eq!(declaration.file, "src/App/Domain/Widget.cs");
            }
        }
    }

    #[test]
    fn file_scoped_and_block_namespace_declare_the_same_module() {
        assert_eq!(
            extract("namespace A.B;\n").declarations,
            extract("namespace A.B {\n").declarations
        );
        assert_eq!(
            extract("namespace A.B;\n").declarations,
            vec![Declaration {
                module_path: "A/B".to_string(),
                file: "src/App/Domain/Widget.cs".to_string(),
            }]
        );
    }

    #[test]
    fn block_namespace_accepts_opening_brace_on_next_non_comment_line() {
        for content in [
            "namespace Acme.Domain\n{\nusing Acme.Contracts;\n}",
            "namespace Acme.Domain // Jellyfin-style block\n\n// comment\n/* another\ncomment */\n{ // body\nusing Acme.Contracts;\n}",
            "namespace\tAcme.Domain\n{\nusing Acme.Contracts;\n}",
        ] {
            let extraction = extract(content);
            assert_eq!(extraction.declarations.len(), 1, "{content}");
            assert_eq!(extraction.declarations[0].module_path, "Acme/Domain");
            assert_eq!(extraction.imports.len(), 1);
            assert_eq!(extraction.imports[0].raw, "using Acme.Contracts;");
            assert_eq!(
                content.lines().nth(extraction.imports[0].line as usize - 1),
                Some("using Acme.Contracts;")
            );
        }
    }

    #[test]
    fn incomplete_block_namespace_does_not_consume_later_statements() {
        for content in [
            "namespace Acme.MissingBrace",
            "namespace Acme.MissingBrace\nusing Acme.Contracts;\n{",
            "namespace Acme.Invalid Name\n{\nusing Acme.Contracts;",
        ] {
            assert!(extract(content).declarations.is_empty(), "{content}");
        }
        let extraction =
            extract("namespace Acme.MissingBrace\nnamespace Acme.Valid;\nusing Acme.Contracts;");
        assert_eq!(extraction.declarations.len(), 1);
        assert_eq!(extraction.declarations[0].module_path, "Acme/Valid");
        assert_eq!(extraction.imports.len(), 1);
    }

    #[test]
    fn using_inside_a_namespace_block_is_emitted() {
        let content = "\
namespace A.B {
    using C.D;
}
";
        let extraction = extract(content);
        assert_eq!(targets(&extraction), ["C/D"]);
        assert_eq!(extraction.declarations[0].module_path, "A/B");
    }

    #[test]
    fn using_resource_block_statement_is_not_emitted() {
        let content = "\
using (var scope = new Scope())
{
    Do(scope);
}
using var writer = File.CreateText(path);
using (File.Open(p)) ;
using A.B;
";
        let extraction = extract(content);
        assert_eq!(targets(&extraction), ["A/B"]);
    }

    #[test]
    fn file_without_a_namespace_gets_no_declaration() {
        let extraction = extract("using A.B;\n\ninternal class Widget { }\n");
        assert!(extraction.declarations.is_empty());
        assert_eq!(targets(&extraction), ["A/B"]);
    }

    #[test]
    fn line_comments_and_block_comment_state_are_skipped() {
        let content = "\
namespace a.b;
// using a.LineComment;
/* using a.SameLine; */ using a.AfterBlock;
/*
namespace swallowed.pkg;
using a.InsideBlock;
*/
using a.AfterMultiLineBlock;
using a.BeforeOpen;
/* unterminated block starts here
using a.Swallowed;
";
        let extraction = extract(content);
        assert_eq!(
            targets(&extraction),
            ["a/AfterBlock", "a/AfterMultiLineBlock", "a/BeforeOpen"]
        );
        assert_eq!(extraction.declarations.len(), 1);
        assert_eq!(extraction.declarations[0].module_path, "a/b");
    }

    #[test]
    fn namespace_and_using_keywords_in_code_are_not_statements() {
        let content = "\
String s = \"namespace A.B; using C.D;\";
int using_like = Using();
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
        let extraction = extract("namespace A.B;\n\nusing A.B.C; // trailing\n");
        assert_eq!(extraction.imports[0].line, 3);
        assert_eq!(extraction.imports[0].raw, "using A.B.C; // trailing");
        assert_eq!(extraction.declarations[0].file, "src/App/Domain/Widget.cs");
    }

    #[test]
    fn extractor_reports_the_csharp_language() {
        assert_eq!(CsExtractor.lang(), Lang::CSharp);
    }

    #[test]
    fn cs_extractor_is_registered() {
        assert!(registry().iter().any(|e| e.lang() == Lang::CSharp));
        assert!(matches!(&extractor_for(Lang::CSharp), Some(e) if e.lang() == Lang::CSharp));
    }
}
