//! Rust visibility scanner with lexical shielding. Rust imports are
//! resolved by the engine from syntax trees and Cargo manifests, so the
//! registered extractor reports no imports or declarations of its own;
//! strings, nested comments and raw-string bodies never create symbols.

use crate::extract::{Extraction, Extractor};
use crate::walk::Lang;

/// Rust scanner (T08), registered so the walk keeps Rust files.
///
/// [`Extractor::extract`] returns an empty [`Extraction`]: the engine reads
/// `use` and `mod` items itself from syntax trees, which a line scan cannot
/// match (multi-line groups, cfg, workspace crates). The Rust hierarchy is
/// path-derived, so there are no [`Extraction::declarations`] either.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RustExtractor;

impl Extractor for RustExtractor {
    fn lang(&self) -> Lang {
        Lang::Rust
    }

    fn extract(&self, _rel_path: &str, _content: &str) -> Extraction {
        Extraction::default()
    }
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

    const FIXTURE: &str = "\
use crate::billing::invoice::total;
use super::x::{a::deep, b};
pub mod api;
mod storage;
pub use crate::reexport::item;
";

    fn extract(content: &str) -> Extraction {
        RustExtractor.extract("src/app/lib.rs", content)
    }

    #[test]
    fn rust_never_reports_declarations() {
        let extraction = extract(FIXTURE);
        assert!(extraction.declarations.is_empty());
    }

    #[test]
    fn rust_extraction_is_empty_because_the_engine_resolves_rust_itself() {
        assert_eq!(extract(FIXTURE), Extraction::default());
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
