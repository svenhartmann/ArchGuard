//! What the window says about the coverage of each call analyzer: a kind
//! the window translates into a lead sentence, keyed by the language,
//! backend and status the engine reports. The engine's own text stays the
//! detail behind it.

use archguard_engine::core::{CallAnalysis, CallAnalyzer};

/// The lead sentence of an analyzer note (`CallsAnalyzer.note` in
/// `ui/calls_data.slint`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Note {
    /// Complete coverage: nothing to explain.
    None = 0,
    NeedsJdk,
    NeedsDotnet,
    NeedsTypeScript,
    NeedsPython,
    NeedsGo,
    /// Rust from syntax only: calls stay unresolved.
    RustSyntax,
    /// Rust calls resolved with rust-analyzer, macros excepted.
    RustAnalyzer,
    /// A token scanner (PHP, Kotlin): calls and types are not resolved.
    Tokens,
    /// Python calls resolved statically from syntax.
    PythonStatic,
    Unavailable,
    Partial,
}

/// The note of an analyzer, from what the engine reports about it.
pub fn note(a: &CallAnalyzer) -> Note {
    match (a.language.as_str(), a.backend.as_str(), a.status.as_str()) {
        (_, _, "ready" | "not_applicable") => Note::None,
        (_, "archguard tokens", _) => Note::Tokens,
        ("rust", "syn", _) => Note::RustSyntax,
        ("rust", _, "partial") => Note::RustAnalyzer,
        ("python", _, "partial") => Note::PythonStatic,
        (_, _, "unavailable") => needs(&a.language),
        _ => Note::Partial,
    }
}

fn needs(language: &str) -> Note {
    match language {
        "java" => Note::NeedsJdk,
        "csharp" | "cs" => Note::NeedsDotnet,
        "typescript" | "javascript" => Note::NeedsTypeScript,
        "python" => Note::NeedsPython,
        "go" => Note::NeedsGo,
        _ => Note::Unavailable,
    }
}

/// The analysis-wide diagnostics that no analyzer already shows: the
/// engine prefixes each analyzer's own diagnostics with its language.
pub fn extra_diagnostics(a: &CallAnalysis) -> Vec<String> {
    let own = |text: &str| {
        a.analyzers
            .iter()
            .any(|x| x.diagnostics.iter().any(|d| d == text))
    };
    a.diagnostics
        .iter()
        .filter(|d| !d.split_once(": ").is_some_and(|(_, text)| own(text)))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn analyzer(language: &str, backend: &str, status: &str) -> CallAnalyzer {
        CallAnalyzer {
            language: language.into(),
            backend: backend.into(),
            status: status.into(),
            files: 1,
            version: None,
            project: None,
            diagnostics: vec!["raw engine text".into()],
        }
    }

    #[test]
    fn notes_follow_language_backend_and_status() {
        let cases = [
            (("java", "javac", "ready"), Note::None),
            (("java", "javac", "unavailable"), Note::NeedsJdk),
            (("java", "javac", "partial"), Note::Partial),
            (("csharp", "roslyn", "unavailable"), Note::NeedsDotnet),
            (
                ("typescript", "typescript", "unavailable"),
                Note::NeedsTypeScript,
            ),
            (
                ("javascript", "typescript", "unavailable"),
                Note::NeedsTypeScript,
            ),
            (("python", "python ast", "unavailable"), Note::NeedsPython),
            (("python", "python ast", "partial"), Note::PythonStatic),
            (("go", "go/parser + go/types", "unavailable"), Note::NeedsGo),
            (("rust", "syn", "partial"), Note::RustSyntax),
            (
                ("rust", "syn + rust-analyzer scip", "partial"),
                Note::RustAnalyzer,
            ),
            (("php", "archguard tokens", "partial"), Note::Tokens),
            (("kotlin", "archguard tokens", "partial"), Note::Tokens),
            (("cobol", "none", "unavailable"), Note::Unavailable),
            (("cobol", "none", "partial"), Note::Partial),
        ];
        for ((language, backend, status), want) in cases {
            let got = note(&analyzer(language, backend, status));
            assert_eq!(got, want, "{language} · {backend} · {status}");
        }
    }

    /// The engine repeats every analyzer diagnostic as "<language>: <text>";
    /// the Analysis panel lists it once, under the analyzer.
    #[test]
    fn analyzer_diagnostics_are_not_repeated() {
        let mut ts = analyzer("javascript", "typescript", "unavailable");
        ts.diagnostics = vec!["No installed TypeScript compiler found.".into()];
        let a = CallAnalysis {
            analyzers: vec![analyzer("rust", "syn", "partial"), ts],
            diagnostics: vec![
                "rust: raw engine text".into(),
                "rust: bad.rs: expected `;`".into(),
                "typescript: No installed TypeScript compiler found.".into(),
                "12 project calls could not be mapped.".into(),
            ],
            ..Default::default()
        };
        assert_eq!(
            extra_diagnostics(&a),
            vec![
                "rust: bad.rs: expected `;`".to_string(),
                "12 project calls could not be mapped.".to_string(),
            ]
        );
    }
}
