//! The shared loop of token-scanner backends (PHP, Kotlin).
use std::path::Path;

use archguard_core::{
    CallAnalysis, CallAnalyzer, CallClass, CallMethod, Config, SemanticDetails, SourceMethodMetrics,
};
use archguard_extract::{Lang, SourceFile, strip_root};

/// What a token scanner finds in one file: classes, methods, metrics and
/// diagnostics.
pub(crate) type TokenScan = (
    Vec<CallClass>,
    Vec<CallMethod>,
    Vec<SourceMethodMetrics>,
    Vec<String>,
);

/// A token-scanner backend for languages without a compiler backend (PHP,
/// Kotlin): every source file of `lang` goes through `scan(path, content)`.
/// Calls and types stay unresolved, so the report is `partial` with
/// `limitation` as its coverage note, and the method enumeration is
/// incomplete.
pub(crate) fn token_calls(
    root: &Path,
    config: &Config,
    (lang, language, limitation): (Lang, &str, &str),
    scan: impl Fn(&SourceFile, &str, &str) -> TokenScan,
) -> CallAnalysis {
    const SOURCE_LIMIT: u64 = 8 * 1024 * 1024;
    let mut report = CallAnalysis {
        java_release: 0,
        ..Default::default()
    };
    let files = match crate::pipeline::walk_files(root, config) {
        Ok(files) => files,
        Err(error) => {
            report.status = "unavailable".into();
            report.diagnostics.push(error.to_string());
            return report;
        }
    };
    let mut metrics = Vec::new();
    for file in files.iter().filter(|file| file.lang == lang) {
        report.files += 1;
        let stripped = strip_root(&file.rel_path, &config.project.source_roots);
        let text =
            crate::project_files::read_repo_file(root, Path::new(&file.rel_path), SOURCE_LIMIT)
                .map_err(|error| error.to_string())
                .and_then(|bytes| String::from_utf8(bytes).map_err(|error| error.to_string()));
        match text {
            Ok(content) => {
                let (classes, methods, found, diagnostics) = scan(file, &stripped, &content);
                report.classes.extend(classes);
                report.methods.extend(methods);
                report.diagnostics.extend(diagnostics);
                metrics.extend(found);
            }
            Err(error) => report.diagnostics.push(format!("{stripped}: {error}")),
        }
    }
    if report.files == 0 {
        return report;
    }
    report.status = "partial".into();
    report.diagnostics.push(limitation.into());
    report.analyzers.push(CallAnalyzer {
        language: language.into(),
        backend: "archguard tokens".into(),
        status: "partial".into(),
        files: report.files,
        version: None,
        project: None,
        diagnostics: vec![limitation.into()],
    });
    report.details = Some(SemanticDetails {
        enumeration_complete: Some(false),
        method_metrics: metrics,
        ..Default::default()
    });
    report
}
