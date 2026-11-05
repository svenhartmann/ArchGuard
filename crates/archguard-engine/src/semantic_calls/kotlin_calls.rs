//! Token-based Kotlin measurements from `archguard_extract::kotlin`. No
//! Kotlin compiler or Gradle run is needed. Calls and types stay unresolved,
//! so the report is `partial`.
use std::path::Path;

use archguard_core::{CallAnalysis, Config};
use archguard_extract::kotlin::scan_kotlin_structure;
use archguard_extract::{Extractor, KotlinExtractor, Lang};

const LIMITATION: &str = "Token-based analysis: classes, objects, functions, complexity and Halstead counts. Calls and types are not resolved; abstractness is read from interface, abstract class and sealed declarations. Property accessors, constructors and init blocks are not measured.";

pub(crate) fn analyze_kotlin_calls(root: &Path, config: &Config) -> CallAnalysis {
    super::token_calls::token_calls(
        root,
        config,
        (Lang::Kotlin, "kotlin", LIMITATION),
        |path, content| {
            // The graph names a Kotlin file after its package, as the pipeline does.
            let declarations = KotlinExtractor.extract(path, content).declarations;
            let node = crate::pipeline::file_node_name(path, &declarations);
            let found = scan_kotlin_structure(path, &node, content);
            (
                found.classes,
                found.methods,
                found.metrics,
                found.diagnostics,
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (path, source) in files {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, source).unwrap();
        }
        dir
    }

    #[test]
    fn kotlin_classes_are_measured_and_broken_files_reported() {
        let dir = project(&[
            (
                "src/com/shop/User.kt",
                "package com.shop\n\nsealed class User {\n    fun name(a: Boolean) = if (a) \"x\" else \"y\"\n}\n",
            ),
            ("src/broken.kt", "fun broken() { if (\n"),
        ]);
        let report = analyze_kotlin_calls(dir.path(), &Config::default());
        assert_eq!(report.status, "partial");
        assert_eq!(report.files, 2);
        let analyzer = &report.analyzers[0];
        assert_eq!(
            (
                analyzer.language.as_str(),
                analyzer.backend.as_str(),
                analyzer.status.as_str(),
                analyzer.files
            ),
            ("kotlin", "archguard tokens", "partial", 2)
        );
        let user = report.classes.iter().find(|c| c.name == "User").unwrap();
        assert_eq!(user.is_abstract, Some(true));
        assert_eq!(user.graph_node, "com/shop/User.kt");
        let name = report.methods.iter().find(|m| m.name == "name").unwrap();
        assert_eq!(name.complexity.as_ref().unwrap().cyclomatic, 2);
        let details = report.details.as_ref().unwrap();
        assert_eq!(details.enumeration_complete, Some(false));
        assert_eq!(details.method_metrics.len(), 1);
        assert!(
            report.diagnostics.iter().any(|d| d.contains("broken.kt")),
            "{:?}",
            report.diagnostics
        );
    }

    #[test]
    fn projects_without_kotlin_are_not_applicable() {
        let dir = project(&[("a.py", "x = 1\n")]);
        let report = analyze_kotlin_calls(dir.path(), &Config::default());
        assert_eq!(report.status, "not_applicable");
        assert!(report.analyzers.is_empty());
    }
}
