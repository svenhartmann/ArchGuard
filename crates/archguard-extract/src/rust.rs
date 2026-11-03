//! Lightweight Rust import and visibility scanner with lexical shielding.
//! The engine additionally uses syntax trees to resolve crate-local imports.
//! This scanner retains its legacy symbolic target format for direct clients;
//! strings, nested comments and raw-string bodies never create declarations.

use crate::extract::{Extraction, Extractor, RawImport, TargetSpec};
use crate::walk::Lang;

/// Rust scanner (T08).
///
/// Everything it emits is symbolic with `stem: None`:
///
/// * `use crate::a::b::c;` and `use super::a::b;` → `Symbol { path }` with
///   the path after the leading `crate::`/`super::`, joined with `/` and
///   capped at the first two segments (`"a/b"`). `pub use` re-exports count
///   as `use`. A leading segment other than `crate`/`super` (`std`,
///   external crates, `self`) means the spec is ignored. Group imports are
///   split into one import per element: `use crate::a::{b, c};` yields
///   `"a/b"` and `"a/c"`.
/// * `mod x;` / `pub mod x;` (also `pub(crate) mod x;`) → one edge to the
///   declared child module: `Symbol { path: "<directory of rel_path>/x" }`,
///   i.e. the child's module path in the same `rel_path` namespace the
///   [`DeclMap`] derives its path-derived entries from. Inline
///   `mod x { … }` is a definition, not a child file, and is not emitted.
///
/// Reading of the T08 wording "detect `pub mod x;`/`mod x;` declarations"
/// vs. "No `Declaration`s (Rust hierarchy is path-derived)":
/// ARCHITECTURE.md Kap. 6.1 lists `mod x;` under Rust's *captured imports*
/// (resolution "Modulpfad-Match"), so a `mod` statement is an import edge,
/// while [`Extraction::declarations`] stays empty — the Rust hierarchy is
/// path-derived and the [`DeclMap`] already registers every file under its
/// directory chain without any declaration (see its own documentation).
///
/// [`DeclMap`]: crate::resolve::DeclMap
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RustExtractor;

impl Extractor for RustExtractor {
    fn lang(&self) -> Lang {
        Lang::Rust
    }

    fn extract(&self, rel_path: &str, content: &str) -> Extraction {
        let mut imports = Vec::new();
        let clean = crate::lex::code_only(content, crate::lex::Flavor::Rust);
        for (index, (line, code)) in content.lines().zip(clean.lines()).enumerate() {
            let code = code.trim();
            if code.is_empty() {
                continue;
            }
            let line_number = index as u32 + 1;
            imports.extend(use_imports(rel_path, line, line_number, code));
            imports.extend(mod_import(rel_path, line, line_number, code));
        }
        Extraction {
            imports,
            declarations: Vec::new(),
        }
    }
}

/// Drops a leading `pub` / `pub(...)` visibility qualifier.
fn strip_visibility(code: &str) -> &str {
    if let Some(rest) = code.strip_prefix("pub ") {
        return rest.trim_start();
    }
    if let Some(rest) = code
        .strip_prefix("pub(")
        .and_then(|qualified| qualified.find(')').map(|end| &qualified[end + 1..]))
    {
        return rest.trim_start();
    }
    code
}

/// The imports of one comment-stripped line, empty unless it is a complete
/// `use …;` statement.
fn use_imports(rel_path: &str, raw: &str, line: u32, code: &str) -> Vec<RawImport> {
    let Some(body) = strip_visibility(code)
        .strip_prefix("use ")
        .map(str::trim)
        .and_then(|body| body.strip_suffix(';'))
    else {
        return Vec::new();
    };
    expand_braces(body)
        .iter()
        .filter_map(|path| use_symbol(path))
        .map(|path| RawImport {
            source: rel_path.to_string(),
            target: TargetSpec::Symbol { path, stem: None },
            line,
            raw: raw.to_string(),
        })
        .collect()
}

/// Splits `a::{b, c::{d, e}}` into fully joined single paths; a path
/// without braces is returned unchanged.
fn expand_braces(path: &str) -> Vec<String> {
    let (Some(open), Some(close)) = (path.find('{'), path.rfind('}')) else {
        return vec![path.to_string()];
    };
    if close <= open {
        return vec![path.to_string()];
    }
    let prefix = path[..open].trim_end().trim_end_matches(':');
    let suffix = path[close + 1..].trim_start().trim_start_matches(':');
    split_top_level(&path[open + 1..close])
        .into_iter()
        .filter(|item| !item.is_empty())
        .flat_map(|item| {
            let mut joined = String::new();
            if !prefix.is_empty() {
                joined.push_str(prefix);
                joined.push_str("::");
            }
            joined.push_str(item);
            if !suffix.is_empty() {
                joined.push_str("::");
                joined.push_str(suffix);
            }
            expand_braces(&joined)
        })
        .collect()
}

/// Splits on commas that sit outside every `{ … }` group, so a nested
/// group's commas stay part of its item.
fn split_top_level(inner: &str) -> Vec<&str> {
    let mut items = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (index, c) in inner.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                items.push(inner[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    items.push(inner[start..].trim());
    items
}

/// Normalized module path of one fully expanded `use` spec, `None` when the
/// spec is external — anything not rooted at `crate::` or `super::` (in
/// particular `std::…`, external crates and `self::…`). A `self` segment
/// inside a brace group (`x::{self, y}`) denotes the module `x` itself and
/// is dropped.
fn use_symbol(path: &str) -> Option<String> {
    let segments: Vec<&str> = path
        .split("::")
        .map(|segment| segment.split_whitespace().next().unwrap_or_default())
        .filter(|segment| !segment.is_empty())
        .collect();
    match segments.first().copied() {
        Some("crate") | Some("super") => {}
        _ => return None,
    }
    let kept: Vec<&str> = segments[1..]
        .iter()
        .copied()
        .filter(|segment| *segment != "self")
        .take(2)
        .collect();
    if kept.is_empty() {
        return None;
    }
    Some(kept.join("/"))
}

/// The module edge of a comment-stripped `mod x;` / `pub mod x;` line,
/// `None` for every other line — in particular for inline `mod x { … }`
/// definitions, which declare no child file. The edge path is the child's
/// module path: the importing file's directory plus the module name.
fn mod_import(rel_path: &str, raw: &str, line: u32, code: &str) -> Option<RawImport> {
    let name = strip_visibility(code)
        .strip_prefix("mod ")
        .map(str::trim)
        .and_then(|body| body.strip_suffix(';'))
        .map(str::trim)?;
    if !is_module_name(name) {
        return None;
    }
    let path = match rel_path.rsplit_once('/') {
        Some((dir, _)) if !dir.is_empty() => format!("{dir}/{name}"),
        _ => name.to_string(),
    };
    Some(RawImport {
        source: rel_path.to_string(),
        target: TargetSpec::Symbol { path, stem: None },
        line,
        raw: raw.to_string(),
    })
}

/// `true` for a plain `mod NAME;` identifier (attributes, paths and inline
/// groups never match).
fn is_module_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
}

/// Exported/internal symbol split of one file (WP-02).
///
/// Rule (R13): `pub` → exported, everything else (including
/// crate-visible) is internal.
pub fn scan_visibility(content: &str) -> archguard_core::Visibility {
    let clean = crate::lex::code_only(content, crate::lex::Flavor::Rust);
    let content = clean.as_str();
    let mut visibility = archguard_core::Visibility::default();
    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with("//") || line.starts_with("/*") {
            continue;
        }
        let mut rest = line;
        let mut exported = false;
        if let Some(after) = strip_word(rest, "pub") {
            if after.starts_with('(') {
                // Restricted `pub(…)` stays crate-visible → internal.
                match after.find(')') {
                    Some(close) => rest = after[close + 1..].trim_start(),
                    None => continue, // malformed line, no claim
                }
            } else {
                exported = true;
                rest = after;
            }
        }
        // Prefix keywords that carry no visibility of their own.
        while let Some(after) = ["unsafe", "default", "async", "extern"]
            .into_iter()
            .find_map(|prefix| strip_word(rest, prefix))
        {
            rest = after;
        }
        if let Some(after) = strip_word(rest, "const")
            && after.starts_with("fn ")
        {
            rest = after;
        }
        for keyword in [
            "fn", "struct", "enum", "trait", "const", "static", "type", "mod", "use", "union",
        ] {
            let Some(after) = strip_word(rest, keyword) else {
                continue;
            };
            let name = if keyword == "use" {
                // A re-export publishes the last path segment or the
                // `as` alias, not the module path it starts from.
                let tail = after.split(';').next().unwrap_or(after);
                let tail = tail
                    .rsplit_once(" as ")
                    .map(|(_, alias)| alias)
                    .unwrap_or(tail.rsplit("::").next().unwrap_or(tail));
                first_identifier(tail)
            } else {
                first_identifier(after)
            };
            if let Some(name) = name {
                if exported {
                    visibility.exported.insert(name);
                } else {
                    visibility.internal.insert(name);
                }
            }
            break;
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
        .is_some_and(|c| c.is_alphanumeric() || c == '_'))
    .then_some(rest.trim_start())
}

/// First identifier token of `text`, `None` when there is none.
fn first_identifier(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut index = 0usize;
    while index < chars.len()
        && !chars[index].is_alphanumeric()
        && chars[index] != '_'
        && chars[index] != '$'
    {
        index += 1;
    }
    let start = index;
    while index < chars.len()
        && (chars[index].is_alphanumeric() || chars[index] == '_' || chars[index] == '$')
    {
        index += 1;
    }
    (index > start).then(|| chars[start..index].iter().collect())
}

#[cfg(test)]
mod visibility_tests {
    use super::*;

    #[test]
    fn pub_declarations_are_exported() {
        let source = "pub fn run() {}\npub struct Config;\npub(crate) fn helper() {}\npub trait Runner {}\nfn secret() {}\nstruct Impl;\n";
        let visibility = scan_visibility(source);
        assert!(visibility.exported.contains("run"));
        assert!(visibility.exported.contains("Config"));
        assert!(visibility.exported.contains("Runner"));
        assert!(
            visibility.internal.contains("helper"),
            "pub(...) is crate-visible"
        );
        assert!(visibility.internal.contains("secret"));
        assert!(visibility.internal.contains("Impl"));
        assert_eq!(visibility.exported.len(), 3, "{:?}", visibility.exported);
    }

    #[test]
    fn unsafe_and_doc_comments_do_not_disturb() {
        let source = "/// docs\n// comment\npub unsafe fn ffi() {}\nunsafe fn raw() {}\n";
        let visibility = scan_visibility(source);
        assert!(visibility.exported.contains("ffi"));
        assert!(visibility.internal.contains("raw"));
    }

    #[test]
    fn use_re_exports_publish_the_last_segment_or_alias() {
        let visibility = scan_visibility(
            "pub use crate::thing::Thing;\npub use crate::a::b as c;\nuse crate::other;\n",
        );
        assert!(visibility.exported.contains("Thing"));
        assert!(
            visibility.exported.contains("c"),
            "the `as` alias is published"
        );
        assert!(visibility.internal.contains("other"));
        assert!(
            !visibility.exported.contains("crate"),
            "the path root is not a symbol: {:?}",
            visibility.exported
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::{extractor_for, registry};

    /// ~20-line fixture covering every T08 case: `use crate::`, `use
    /// super::`, first-two-segment capping, brace groups, `pub use`,
    /// external/`std`/`self` ignores, line and block comments (incl. a
    /// multi-line block), `mod` declarations and a non-statement line.
    const FIXTURE: &str = "\
use crate::billing::invoice::total; // first two segments only
use crate::domain::money;
use super::common;
use crate::util::{fmt, io};
use super::x::{a::deep, b};
use std::collections::HashMap;
use serde::Serialize;
use self::local_helper;
// use crate::commented_out::gone;
/* use crate::trailing_block::gone; */
/*
use crate::inside_block::gone;
*/
use crate::after_block::kept;
pub mod api;
mod storage;
pub(crate) mod internal;
mod inline_helper { pub use crate::no::edge; }
pub use crate::reexport::item;
fn run() { let v = crate::not::an::import(); }
";

    /// The expected import for fixture line `line_no` with module `path`
    /// (raw is taken verbatim from the fixture itself).
    fn expected(line_no: u32, path: &str) -> RawImport {
        RawImport {
            source: "src/app/lib.rs".to_string(),
            target: TargetSpec::Symbol {
                path: path.to_string(),
                stem: None,
            },
            line: line_no,
            raw: FIXTURE
                .lines()
                .nth((line_no - 1) as usize)
                .expect("fixture line exists")
                .to_string(),
        }
    }

    fn extract(content: &str) -> Extraction {
        RustExtractor.extract("src/app/lib.rs", content)
    }

    fn paths(extraction: &Extraction) -> Vec<String> {
        extraction
            .imports
            .iter()
            .map(|import| match &import.target {
                TargetSpec::Symbol { path, .. } => path.clone(),
                TargetSpec::Path(spec) => spec.clone(),
                TargetSpec::Qualified(segments) => format!("q:{}", segments.join(".")),
            })
            .collect()
    }

    #[test]
    fn fixture_yields_exactly_the_expected_imports() {
        let extraction = extract(FIXTURE);
        assert_eq!(
            extraction.imports,
            vec![
                expected(1, "billing/invoice"),
                expected(2, "domain/money"),
                expected(3, "common"),
                expected(4, "util/fmt"),
                expected(4, "util/io"),
                expected(5, "x/a"),
                expected(5, "x/b"),
                expected(14, "after_block/kept"),
                expected(15, "src/app/api"),
                expected(16, "src/app/storage"),
                expected(17, "src/app/internal"),
                expected(19, "reexport/item"),
            ],
            "imports: {:#?}",
            extraction.imports
        );
    }

    #[test]
    fn rust_never_reports_declarations() {
        let extraction = extract(FIXTURE);
        assert!(extraction.declarations.is_empty());
    }

    #[test]
    fn use_statement_is_capped_at_two_segments_after_the_root_keyword() {
        let extraction = extract("use crate::a::b::c::d;\nuse super::x::y;\n");
        assert_eq!(paths(&extraction), ["a/b", "x/y"]);
    }

    #[test]
    fn brace_group_is_split_into_multiple_imports() {
        let extraction = extract("use crate::x::{y, z};\nuse crate::a::{b::{c, d}, e};\n");
        assert_eq!(paths(&extraction), ["x/y", "x/z", "a/b", "a/b", "a/e"]);
    }

    #[test]
    fn self_in_a_brace_group_refers_to_its_module() {
        let extraction = extract("use crate::x::{self, y};\nuse crate::{self};\n");
        assert_eq!(paths(&extraction), ["x", "x/y"]);
    }

    #[test]
    fn alias_is_stripped() {
        let extraction = extract("use crate::foo::bar as baz;\n");
        assert_eq!(paths(&extraction), ["foo/bar"]);
    }

    #[test]
    fn external_std_and_self_specs_are_ignored() {
        let content = "use std::collections::HashMap;\nuse std::{io, fs};\n\
                       use serde::Serialize;\nuse self::local;\nuse crate::kept::one;\n";
        assert_eq!(paths(&extract(content)), ["kept/one"]);
    }

    #[test]
    fn line_without_a_trailing_semicolon_is_not_a_complete_use_statement() {
        // Multi-line `use` groups are a documented v1 limitation.
        let extraction = extract("use crate::a::{\n    b,\n};\n");
        assert!(extraction.imports.is_empty());
    }

    #[test]
    fn line_comments_and_block_comment_state_are_skipped() {
        let content = "\
use crate::before::a;
// use crate::line_comment::gone;
/* use crate::same_line::gone; */ use crate::after_block::b;
/*
use crate::inside_block::gone;
still inside;
*/
use crate::after_multi_line_block::c;
/* unterminated block starts here
use crate::swallowed::gone;
";
        assert_eq!(
            paths(&extract(content)),
            ["before/a", "after_block/b", "after_multi_line_block/c"]
        );
    }

    #[test]
    fn doc_comment_examples_are_not_imports() {
        let content = "/// use crate::doc::example;\n//! use crate::inner_doc::example;\n";
        assert!(extract(content).imports.is_empty());
    }

    #[test]
    fn mod_declarations_become_module_edges_in_the_files_directory() {
        let extraction = extract("pub mod api;\nmod storage;\npub(crate) mod internal;\n");
        assert_eq!(
            paths(&extraction),
            ["src/app/api", "src/app/storage", "src/app/internal"]
        );
        for import in &extraction.imports {
            assert_eq!(import.source, "src/app/lib.rs");
            assert!(matches!(
                import.target,
                TargetSpec::Symbol { stem: None, .. }
            ));
        }
    }

    #[test]
    fn mod_edge_at_the_tree_root_has_no_directory_prefix() {
        let extraction = RustExtractor.extract("lib.rs", "mod api;\n");
        assert_eq!(paths(&extraction), ["api"]);
    }

    #[test]
    fn inline_mod_definition_lines_are_not_module_edges() {
        // `mod name { … }` declares no child file → no module edge (the
        // fixture covers the one-line form). A `use` written *inside* the
        // inline block is still a real import of the scanned file.
        let extraction = extract("mod helper {\n    use crate::inner::thing;\n}\nmod tests {}\n");
        assert_eq!(paths(&extraction), ["inner/thing"]);
    }

    #[test]
    fn path_expressions_in_code_are_not_imports() {
        let content = "fn run() { let v = crate::not::an::import(); }\nextern crate serde;\n";
        assert!(extract(content).imports.is_empty());
    }

    #[test]
    fn line_numbers_are_one_based_and_raw_is_verbatim() {
        let extraction = extract("use crate::a::b;\nuse crate::c::d; // trailing\n");
        assert_eq!(extraction.imports[1].line, 2);
        assert_eq!(extraction.imports[1].raw, "use crate::c::d; // trailing");
    }

    #[test]
    fn extractor_reports_the_rust_language() {
        assert_eq!(RustExtractor.lang(), Lang::Rust);
    }

    #[test]
    fn rust_extractor_is_registered() {
        assert!(registry().iter().any(|e| e.lang() == Lang::Rust));
        let found = extractor_for(Lang::Rust);
        assert!(matches!(&found, Some(e) if e.lang() == Lang::Rust));
    }
}
