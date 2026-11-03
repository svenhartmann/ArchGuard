//! Language scanner interface (ARCHITECTURE.md Kap. 6, step 3): the
//! [`Extractor`] trait, the raw findings a scanner reports and the registry
//! the engine uses to pick the scanner for a [`Lang`].
//!
//! Scanners are pure functions over one file: they receive the file's
//! root-relative path (as produced by [`crate::walk`]) plus its content and
//! return [`RawImport`]s and [`Declaration`]s — no IO, no resolution (that is
//! T07). Every scanner implements [`Extractor`] and is listed in
//! [`registry`], so languages never know about each other: a new language is
//! a new scanner, not a rewrite.

use crate::walk::Lang;

/// What an import refers to, before resolution (T07).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetSpec {
    /// A path-like spec as written in the source — relative specs such as
    /// `./util` or `../api/x`, or a bare module spec (resolved against the
    /// file tree; unknown bare specs end up external).
    Path(String),
    /// A module/package/namespace symbol. `path` is the normalized module
    /// path with dots (or backslashes) replaced by `/` — `"a/b"` for Java
    /// `a.b` — and `stem` is the class/type name when the import names one
    /// (`Some("C")` for `import a.b.C;`), `None` when only the module is
    /// known (`import a.b.*;`, `using A.B;`).
    Symbol { path: String, stem: Option<String> },
}

/// One raw import found by a scanner, still unresolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawImport {
    /// Root-relative, `/`-separated path of the importing file — the
    /// [`crate::walk::SourceFile::rel_path`] that was scanned.
    pub source: String,
    /// What is imported.
    pub target: TargetSpec,
    /// 1-based line number of the import.
    pub line: u32,
    /// The original source line, kept verbatim for reports and warnings.
    pub raw: String,
}

/// A module a source file declares for itself (`package a.b;`,
/// `namespace A.B;`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    /// Normalized module path, dots → slashes: `"a/b"` from `package a.b;`.
    pub module_path: String,
    /// Root-relative, `/`-separated path of the declaring file.
    pub file: String,
}

/// Everything one scanner found in one file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extraction {
    /// Raw imports found in the file.
    pub imports: Vec<RawImport>,
    /// Declarations found in the file.
    pub declarations: Vec<Declaration>,
}

/// A language scanner: turns one source file into raw imports and
/// declarations. Implementations are stateless and perform no IO.
pub trait Extractor {
    /// The language this scanner handles.
    fn lang(&self) -> Lang;
    /// Scan `content` of the file at `rel_path` (root-relative, `/`-joined).
    fn extract(&self, rel_path: &str, content: &str) -> Extraction;
}

/// All extractors available in this build.
///
/// One entry per language scanner as it lands (T08–T12); a new language is
/// a new scanner appended here, not a rewrite.
pub fn registry() -> Vec<Box<dyn Extractor>> {
    vec![
        Box::new(crate::rust::RustExtractor),
        Box::new(crate::ts::TsExtractor::TS),
        Box::new(crate::ts::TsExtractor::JS),
        Box::new(crate::py::PyExtractor),
        Box::new(crate::java::JavaExtractor),
        Box::new(crate::cs::CsExtractor),
        Box::new(crate::php::PhpExtractor),
        Box::new(crate::go::GoExtractor),
    ]
}

/// The extractor registered for `lang`, `None` while no scanner for that
/// language exists.
pub fn extractor_for(lang: Lang) -> Option<Box<dyn Extractor>> {
    pick(registry(), lang)
}

/// The first of `extractors` whose [`Extractor::lang`] equals `lang`.
///
/// The seam that lets [`extractor_for`] be tested with dummy extractors
/// before the real scanners exist.
fn pick(extractors: Vec<Box<dyn Extractor>>, lang: Lang) -> Option<Box<dyn Extractor>> {
    extractors.into_iter().find(|e| e.lang() == lang)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test-only scanner: echoes the scanned file back as one symbolic
    /// import and one declaration.
    struct DummyExtractor;

    impl Extractor for DummyExtractor {
        fn lang(&self) -> Lang {
            Lang::Py
        }
        fn extract(&self, rel_path: &str, content: &str) -> Extraction {
            Extraction {
                imports: vec![RawImport {
                    source: rel_path.to_string(),
                    target: TargetSpec::Symbol {
                        path: "a/b".to_string(),
                        stem: Some("C".to_string()),
                    },
                    line: 1,
                    raw: content.trim().to_string(),
                }],
                declarations: vec![Declaration {
                    module_path: "a/b".to_string(),
                    file: rel_path.to_string(),
                }],
            }
        }
    }

    /// Second test-only scanner for a different language.
    struct OtherDummyExtractor;

    impl Extractor for OtherDummyExtractor {
        fn lang(&self) -> Lang {
            Lang::Rust
        }
        fn extract(&self, _rel_path: &str, _content: &str) -> Extraction {
            Extraction::default()
        }
    }

    fn dummies() -> Vec<Box<dyn Extractor>> {
        vec![Box::new(DummyExtractor), Box::new(OtherDummyExtractor)]
    }

    #[test]
    fn dummy_extractor_reports_imports_and_declarations_through_the_trait() {
        let extractor: Box<dyn Extractor> = Box::new(DummyExtractor);
        assert_eq!(extractor.lang(), Lang::Py);
        assert_eq!(
            extractor.extract("pkg/m.py", "import a.b.C\n"),
            Extraction {
                imports: vec![RawImport {
                    source: "pkg/m.py".to_string(),
                    target: TargetSpec::Symbol {
                        path: "a/b".to_string(),
                        stem: Some("C".to_string()),
                    },
                    line: 1,
                    raw: "import a.b.C".to_string(),
                }],
                declarations: vec![Declaration {
                    module_path: "a/b".to_string(),
                    file: "pkg/m.py".to_string(),
                }],
            }
        );
    }

    #[test]
    fn pick_returns_the_extractor_matching_the_language() {
        let found = pick(dummies(), Lang::Py);
        assert!(matches!(&found, Some(e) if e.lang() == Lang::Py));
        let found = pick(dummies(), Lang::Rust);
        assert!(matches!(&found, Some(e) if e.lang() == Lang::Rust));
    }

    #[test]
    fn pick_returns_none_without_a_matching_extractor() {
        assert!(pick(dummies(), Lang::Php).is_none());
        assert!(pick(Vec::new(), Lang::Rust).is_none());
    }

    #[test]
    fn registry_holds_at_most_one_extractor_per_language() {
        let mut langs: Vec<Lang> = registry().iter().map(|e| e.lang()).collect();
        let total = langs.len();
        langs.sort();
        langs.dedup();
        assert_eq!(langs.len(), total, "duplicate language in registry");
    }

    #[test]
    fn extractor_for_agrees_with_the_registry() {
        let registered: Vec<Lang> = registry().iter().map(|e| e.lang()).collect();
        for lang in [
            Lang::Rust,
            Lang::Ts,
            Lang::Js,
            Lang::Py,
            Lang::Java,
            Lang::CSharp,
            Lang::Php,
            Lang::Go,
        ] {
            let found = extractor_for(lang);
            assert_eq!(found.is_some(), registered.contains(&lang), "lang {lang:?}");
            assert!(found.is_none_or(|e| e.lang() == lang));
        }
    }
}
