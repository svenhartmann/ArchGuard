//! What the window says about the coverage of each call analyzer: a kind
//! the window translates into a lead sentence, keyed by the language,
//! backend and status the engine reports. The engine's own text stays the
//! detail behind it, with its fixed notes translated ([`diagnostic`]).

use archguard_engine::core::{CallAnalysis, CallAnalyzer};

use crate::tab_support::Texts;

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

/// An engine diagnostic in the window's language: the analyzers' fixed
/// notes (`CallAnalyzer::NOTES`, keys in `ui/texts.slint`) are
/// translated, also after a "<language>: " prefix; any other text (file
/// errors, counts, compiler output) stays as the engine wrote it.
pub fn diagnostic(texts: &Texts, text: &str) -> String {
    let note = |t: &str| CallAnalyzer::NOTES.contains(&t);
    match text.split_once(": ") {
        _ if note(text) => texts.t(text).to_string(),
        Some((language, rest)) if note(rest) => format!("{language}: {}", texts.t(rest)),
        _ => text.to_string(),
    }
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

    /// The analyzers' fixed coverage notes read in the window's
    /// language, also behind the "<language>: " prefix of the
    /// analysis-wide list; other engine text stays as it is.
    #[test]
    fn engine_notes_are_translated_and_other_text_stays() {
        let de = Texts::german();
        for text in CallAnalyzer::NOTES {
            let german = diagnostic(&de, text);
            assert_ne!(german, text, "no German for {text}");
            assert_eq!(
                diagnostic(&de, &format!("rust: {text}")),
                format!("rust: {german}")
            );
        }
        assert_eq!(
            diagnostic(&de, CallAnalyzer::RUST_UNTRUSTED),
            "Rust-Aufrufe und Typreferenzen bleiben unaufgelöst. ARCHGUARD_TRUST_PROJECT_COMPILER=1 außerhalb des Repositorys setzen, um sie mit rust-analyzer aufzulösen; Build-Skripte und Proc-Macros laufen nie."
        );
        for raw in [
            "rust: bad.rs: expected `;`",
            "12 project calls could not be mapped.",
        ] {
            assert_eq!(diagnostic(&de, raw), raw);
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
